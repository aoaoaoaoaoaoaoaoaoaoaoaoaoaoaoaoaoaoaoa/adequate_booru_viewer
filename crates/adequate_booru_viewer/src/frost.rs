//! The water under the UI: frost veil, lift plates, persistent shallow-water
//! field, control quivers, and the viewer pond.

use egui_wgpu::wgpu;
use std::f32::consts::TAU;

#[cfg(test)]
mod audit;
mod dump;
mod guard;

const LEVELS: usize = 3;
const SIM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const SIM_BYTES: u32 = 8;
const SIM_WORKGROUP: u32 = 8;
const FIELD_SCALE: u32 = 2;
const SIM_STEPS: usize = 4;
const SIM_DT: f32 = 1.0 / 240.0;

pub const LIFT_SLOTS: usize = 4;

pub const SPLASH_SLOTS: usize = 32;
pub const QUIVER_SLOTS: usize = 4;
pub const BULGE_CEIL: f32 = 12.0;
pub const TOUCH_SLOTS: usize = 12;

const MASK_BYTES: u64 = 1664;

#[derive(Clone, Copy, Debug)]
pub struct Brine {
    pub reach: f32,
    pub meniscus_px: f32,
    pub refract_px: f32,
    pub ior_spread: f32,
    pub quiver_bulge: f32,
    pub quiver_pulse: f32,
    pub tremor_k: f32,
    pub tremor_omega: f32,
    pub tremor_amp: f32,
    pub tremor_fade: f32,
    pub tremor_reach: f32,
    pub bulge_px: f32,
    pub lift_bright: f32,
    pub wave_v: f32,
    pub wave_sigma: f32,
    pub wave_damp: f32,
    pub wave_spread: f32,
    pub source_gain: f32,
    pub height_retention: f32,
    pub tilt_gain: f32,
    pub t_panel: f32,
    pub r_panel: f32,
    pub r_wall: f32,
    pub shore_feather: f32,
}

impl Default for Brine {
    fn default() -> Self {
        Self {
            reach: 34.0,
            meniscus_px: 1.4,
            refract_px: 1.0,
            ior_spread: 0.34,
            quiver_bulge: 3.0,
            quiver_pulse: 0.2,
            tremor_k: 0.2417,
            tremor_omega: 0.9 * TAU,
            tremor_amp: 0.18,
            tremor_fade: 55.0,
            tremor_reach: 150.0,
            bulge_px: 10.0,
            lift_bright: 0.08,
            wave_v: 320.0,
            wave_sigma: 14.0,
            wave_damp: 2.4,
            wave_spread: 480.0,
            source_gain: 44.0,
            height_retention: 0.99965,
            tilt_gain: 120.0,
            t_panel: 0.12,
            r_panel: 0.35,
            r_wall: 0.6,
            shore_feather: 12.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tension {
    pub id: u64,
    pub rect: egui::Rect,
    pub pointer: egui::Pos2,
    pub grip: f32,
    pub omega: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Lift {
    pub rect: egui::Rect,
    pub grip: f32,
    pub depth: LiftDepth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiftDepth {
    Surface,
    Shallow,
}

impl Lift {
    pub fn surface(rect: egui::Rect, grip: f32) -> Self {
        Self {
            rect,
            grip,
            depth: LiftDepth::Surface,
        }
    }

    pub fn shallow(rect: egui::Rect, grip: f32) -> Self {
        Self {
            rect,
            grip,
            depth: LiftDepth::Shallow,
        }
    }

    fn packed_grip(self) -> f32 {
        let grip = self.grip.clamp(0.0, 1.0);
        match self.depth {
            LiftDepth::Surface => grip,
            LiftDepth::Shallow => -grip,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Splash {
    pub rect: egui::Rect,
    pub age: f32,
    pub amp: f32,
    pub shape: SplashShape,
    /// Signed screen-y a dragged surface (a spun tape) travels, 0 for a plain
    /// splash. Non-zero turns the source into a directional velocity dipole.
    pub drag: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SplashShape {
    #[default]
    Ring,
    Basin,
    /// A broadband velocity-noise impulse over the whole rect — the sheet
    /// "thwacked" from beneath. The solver's KO term shreds the high-k content
    /// into a quick shimmer; the sparse low-k residue rides out.
    Jitter,
}

impl SplashShape {
    fn code(self) -> f32 {
        self as u8 as f32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Raft {
    pub rect: egui::Rect,
    pub corners: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub center: egui::Pos2,
    pub age: f32,
    pub amp: f32,
}

pub struct Surge<'a> {
    pub dry: bool,
    pub veil: Option<Veil>,
    pub tensions: &'a [Tension],
    pub lifts: &'a [Lift],
    pub water: egui::Rect,
    pub scroll_tilt: f32,
    pub splashes: &'a [Splash],
    pub raft: Option<Raft>,
    pub floor: egui::Rect,
    pub viewer: egui::Rect,
    pub touches: &'a [Touch],
    /// Keep the persistent solver ticking while old energy decays, even after
    /// its one-frame exciters have fallen out of the CPU source lists.
    pub wake: bool,
    /// Wall-clock seconds (wrapped) driving the tremor wavetrains.
    pub tide: f32,
    pub brine: Brine,
    pub guard: bool,
}

impl Surge<'_> {
    pub fn live(&self) -> bool {
        !self.dry
    }

    fn sim_live(&self) -> bool {
        !self.dry
            && (!self.tensions.is_empty()
                || !self.lifts.is_empty()
                || !self.splashes.is_empty()
                || self.raft.is_some()
                || self.wake)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Veil {
    pub cuts: [Cut; 2],
    pub strength: f32,
    pub dim: f32,
    pub blur: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Cut {
    pub rect: egui::Rect,
    pub radius: f32,
}

impl Cut {
    /// A cutout that never matches any pixel: uniform blur.
    pub const NONE: Self = Self {
        rect: egui::Rect {
            min: egui::Pos2 { x: -4e6, y: -4e6 },
            max: egui::Pos2 { x: -4e6, y: -4e6 },
        },
        radius: 0.0,
    };
}

pub struct Frost {
    sample_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    sim_layout: wgpu::BindGroupLayout,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    pipes: Pipes,
    sampler: wgpu::Sampler,
    mask: wgpu::Buffer,
    format: wgpu::TextureFormat,
    rig: Option<Rig>,
    sentinel: guard::Sentinel,
}

struct Pipes {
    composite: wgpu::RenderPipeline,
    sim: wgpu::ComputePipeline,
}

/// The size-dependent resources, rebuilt on resize.
struct Rig {
    scene: Target,
    chain: Vec<Target>,
    water: Water,
}

struct Water {
    size: wgpu::Extent3d,
    textures: Vec<wgpu::Texture>,
    composite_bind: Vec<wgpu::BindGroup>,
    sim_bind: Vec<wgpu::BindGroup>,
    phase: usize,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Bind group for passes that sample this target.
    bind: wgpu::BindGroup,
}

impl Frost {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frost"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let sim_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("frost-sim"),
            source: wgpu::ShaderSource::Wgsl(SIM_WGSL.into()),
        });
        let sample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frost-sample"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frost-composite"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                sampler_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(MASK_BYTES),
                    },
                    count: None,
                },
                unfilterable_texture_entry(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frost-sim"),
            entries: &[
                unfilterable_texture_entry(0, wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: SIM_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(MASK_BYTES),
                    },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("frost-linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mask = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frost-mask"),
            size: MASK_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline =
            |label: &str, layout: &wgpu::BindGroupLayout, entry, constants: &[(&str, f64)]| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[Some(layout)],
                        immediate_size: 0,
                    });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("fullscreen"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants,
                            ..Default::default()
                        },
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let sim_layout_handle = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("frost-sim"),
            bind_group_layouts: &[Some(&sim_layout)],
            immediate_size: 0,
        });
        let field = [("FIELD_SCALE", f64::from(FIELD_SCALE))];
        let sim_consts = [
            ("SIM_SCALE", f64::from(FIELD_SCALE)),
            ("DT", f64::from(SIM_DT)),
            ("IMPULSE_GAIN", 4.0 / SIM_STEPS as f64),
        ];
        let pipes = Pipes {
            composite: pipeline("frost-composite", &composite_layout, "composite", &field),
            sim: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("frost-sim"),
                layout: Some(&sim_layout_handle),
                module: &sim_module,
                entry_point: Some("step"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &sim_consts,
                    ..Default::default()
                },
                cache: None,
            }),
        };
        Self {
            down: pipeline("frost-down", &sample_layout, "kawase_down", &[]),
            up: pipeline("frost-up", &sample_layout, "kawase_up", &[]),
            pipes,
            sample_layout,
            composite_layout,
            sim_layout,
            sampler,
            mask,
            format,
            rig: None,
            sentinel: guard::Sentinel::default(),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        if width == 0 || height == 0 {
            self.rig = None;
            return;
        }
        let target = |label: &str, w: u32, h: u32| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[self.format],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.sample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            Target {
                texture,
                view,
                bind,
            }
        };
        let scene = target("frost-scene", width, height);
        let chain = (1..=LEVELS as u32)
            .map(|level| target("frost-chain", width >> level, height >> level))
            .collect::<Vec<_>>();
        let water = self.water(device, queue, width, height, &scene, &chain[0]);
        self.rig = Some(Rig {
            scene,
            chain,
            water,
        });
    }

    /// Render target for the egui pass when a veil is up.
    pub fn scene_view(&self) -> Option<&wgpu::TextureView> {
        self.rig.as_ref().map(|rig| &rig.scene.view)
    }

    /// Composites the offscreen scene to `surface` with everything in `surge`.
    pub fn compose(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface: &wgpu::TextureView,
        surge: &Surge<'_>,
    ) {
        let Some(rig) = &mut self.rig else {
            return;
        };
        queue.write_buffer(&self.mask, 0, &mask_bytes(surge));
        if surge.sim_live() {
            for _ in 0..SIM_STEPS {
                run_compute(
                    encoder,
                    &self.pipes.sim,
                    &rig.water.sim_bind[rig.water.phase],
                    rig.water.size,
                );
                rig.water.phase ^= 1;
            }
        }
        if surge.veil.is_some_and(|veil| veil.blur > 0.0) {
            let mut blur = |pipeline, source: &Target, sink: &wgpu::TextureView| {
                run_pass(encoder, pipeline, &source.bind, sink);
            };
            blur(&self.down, &rig.scene, &rig.chain[0].view);
            for level in 1..LEVELS {
                blur(&self.down, &rig.chain[level - 1], &rig.chain[level].view);
            }
            for level in (1..LEVELS).rev() {
                blur(&self.up, &rig.chain[level], &rig.chain[level - 1].view);
            }
        }
        run_pass(
            encoder,
            &self.pipes.composite,
            &rig.water.composite_bind[rig.water.phase],
            surface,
        );
        if surge.guard {
            self.sentinel.encode(device, encoder, &rig.water);
        }
    }

    pub fn after_submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        guard: bool,
    ) -> bool {
        let Some(rig) = &self.rig else {
            return false;
        };
        if guard {
            self.sentinel.after_submit(device, queue, &rig.water)
        } else {
            self.sentinel.disarm();
            false
        }
    }

    pub fn clear_water(&mut self, queue: &wgpu::Queue) {
        self.sentinel.disarm();
        if let Some(rig) = &self.rig {
            rig.water.clear(queue);
        }
    }

    fn water(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        scene: &Target,
        blur: &Target,
    ) -> Water {
        let size = wgpu::Extent3d {
            width: width.div_ceil(FIELD_SCALE).max(1),
            height: height.div_ceil(FIELD_SCALE).max(1),
            depth_or_array_layers: 1,
        };
        let textures = (0..2)
            .map(|slot| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("frost-water"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: SIM_FORMAT,
                    usage: wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::STORAGE_BINDING,
                    view_formats: &[SIM_FORMAT],
                });
                let zeros = vec![0_u8; (size.width * size.height * SIM_BYTES) as usize];
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &zeros,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size.width * SIM_BYTES),
                        rows_per_image: Some(size.height),
                    },
                    size,
                );
                (slot, texture)
            })
            .collect::<Vec<_>>();
        let views = textures
            .iter()
            .map(|(_, texture)| texture.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect::<Vec<_>>();
        let composite_bind = views
            .iter()
            .map(|view| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("frost-composite"),
                    layout: &self.composite_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&scene.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&blur.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: self.mask.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(view),
                        },
                    ],
                })
            })
            .collect::<Vec<_>>();
        let sim_bind = [(0, 1), (1, 0)]
            .into_iter()
            .map(|(src, dst)| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("frost-sim"),
                    layout: &self.sim_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&views[src]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&views[dst]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.mask.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect::<Vec<_>>();
        Water {
            size,
            textures: textures.into_iter().map(|(_, texture)| texture).collect(),
            composite_bind,
            sim_bind,
            phase: 0,
        }
    }
}

fn run_pass(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    sink: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("frost"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: sink,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

fn run_compute(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    bind: &wgpu::BindGroup,
    size: wgpu::Extent3d,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("frost-sim"),
        timestamp_writes: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.dispatch_workgroups(
        size.width.div_ceil(SIM_WORKGROUP),
        size.height.div_ceil(SIM_WORKGROUP),
        1,
    );
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn unfilterable_texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn mask_bytes(surge: &Surge<'_>) -> [u8; MASK_BYTES as usize] {
    const NO_VEIL: Veil = Veil {
        cuts: [Cut::NONE, Cut::NONE],
        strength: 0.0,
        dim: 1.0,
        blur: 0.0,
    };
    let veil = surge.veil.unwrap_or(NO_VEIL);
    let [a, b] = &veil.cuts;
    let mut lanes = [0.0_f32; (MASK_BYTES / 4) as usize];
    // vec2f block (bytes 0..48): cuts a/b, water.
    lanes[0..2].copy_from_slice(&[a.rect.min.x, a.rect.min.y]);
    lanes[2..4].copy_from_slice(&[a.rect.max.x, a.rect.max.y]);
    lanes[4..6].copy_from_slice(&[b.rect.min.x, b.rect.min.y]);
    lanes[6..8].copy_from_slice(&[b.rect.max.x, b.rect.max.y]);
    lanes[8..10].copy_from_slice(&[surge.water.min.x, surge.water.min.y]);
    lanes[10..12].copy_from_slice(&[surge.water.max.x, surge.water.max.y]);
    // scalar block (bytes 48..80), one pad lane to reach the arrays.
    lanes[12] = a.radius;
    lanes[13] = b.radius;
    lanes[14] = veil.strength.clamp(0.0, 1.0);
    lanes[15] = veil.dim;
    lanes[16] = veil.blur.clamp(0.0, 1.0);
    lanes[17] = surge.tide;
    lanes[18] = surge.scroll_tilt;
    // lift_rects @ byte 80 (lane 20); grips @ 144 (lane 36).
    for (slot, lift) in surge.lifts.iter().take(LIFT_SLOTS).enumerate() {
        let at = 20 + slot * 4;
        lanes[at..at + 4].copy_from_slice(&[
            lift.rect.min.x,
            lift.rect.min.y,
            lift.rect.max.x,
            lift.rect.max.y,
        ]);
        lanes[36 + slot] = lift.packed_grip();
    }
    // quivers @ byte 160 (lane 40): rect, then pointer + grip + omega.
    for (slot, quiver) in surge.tensions.iter().take(QUIVER_SLOTS).enumerate() {
        let at = 40 + slot * 8;
        lanes[at..at + 8].copy_from_slice(&[
            quiver.rect.min.x,
            quiver.rect.min.y,
            quiver.rect.max.x,
            quiver.rect.max.y,
            quiver.pointer.x,
            quiver.pointer.y,
            quiver.grip.clamp(0.0, 1.0),
            quiver.omega.max(0.0),
        ]);
    }
    // splashes @ byte 288 (lane 72): rect, then age + amp + shape + pad.
    for (slot, splash) in surge.splashes.iter().take(SPLASH_SLOTS).enumerate() {
        let at = 72 + slot * 8;
        lanes[at..at + 8].copy_from_slice(&[
            splash.rect.min.x,
            splash.rect.min.y,
            splash.rect.max.x,
            splash.rect.max.y,
            splash.age,
            splash.amp,
            splash.shape.code(),
            splash.drag,
        ]);
    }
    // viewer rect @ byte 1312 (lane 328), touches @ byte 1328 (lane 332).
    lanes[328..330].copy_from_slice(&[surge.viewer.min.x, surge.viewer.min.y]);
    lanes[330..332].copy_from_slice(&[surge.viewer.max.x, surge.viewer.max.y]);
    for (slot, touch) in surge.touches.iter().take(TOUCH_SLOTS).enumerate() {
        let at = 332 + slot * 4;
        lanes[at..at + 4].copy_from_slice(&[touch.center.x, touch.center.y, touch.age, touch.amp]);
    }
    // brine @ byte 1520 (lane 380): the runtime-tunable water chemistry.
    let brine = &surge.brine;
    lanes[380..404].copy_from_slice(&[
        brine.reach,
        brine.meniscus_px,
        brine.refract_px,
        brine.ior_spread,
        brine.quiver_bulge,
        brine.quiver_pulse,
        brine.tremor_k,
        brine.tremor_omega,
        brine.tremor_amp,
        brine.tremor_fade,
        brine.tremor_reach,
        brine.bulge_px.min(BULGE_CEIL),
        brine.lift_bright,
        brine.wave_v,
        brine.wave_sigma,
        brine.wave_damp,
        brine.wave_spread,
        brine.source_gain,
        brine.height_retention,
        brine.tilt_gain,
        brine.t_panel,
        brine.r_panel,
        brine.r_wall,
        brine.shore_feather,
    ]);
    if let Some(raft) = surge.raft {
        lanes[404..408].copy_from_slice(&[
            raft.rect.min.x,
            raft.rect.min.y,
            raft.rect.max.x,
            raft.rect.max.y,
        ]);
        lanes[408..412].copy_from_slice(&raft.corners);
    }
    lanes[412..416].copy_from_slice(&[
        surge.floor.min.x,
        surge.floor.min.y,
        surge.floor.max.x,
        surge.floor.max.y,
    ]);
    let mut bytes = [0_u8; MASK_BYTES as usize];
    for (slot, lane) in lanes.iter().enumerate() {
        bytes[slot * 4..slot * 4 + 4].copy_from_slice(&lane.to_le_bytes());
    }
    bytes
}

const WGSL: &str = include_str!("frost/composite.wgsl");
const SIM_WGSL: &str = include_str!("frost/sim.wgsl");
