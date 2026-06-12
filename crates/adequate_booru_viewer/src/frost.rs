//! The water under the UI: frost veil, lift plates, persistent shallow-water
//! field, control quivers, and the viewer pond.

use egui_wgpu::wgpu;
use std::f32::consts::TAU;

#[cfg(test)]
mod audit;
mod guard;

const LEVELS: usize = 3;
const SIM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SIM_BYTES: u32 = 8;
const SIM_WORKGROUP: u32 = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Definition {
    #[default]
    Sd,
    Hd,
}

#[derive(Clone, Copy)]
struct Spec {
    scale: u32,
    steps: usize,
    dt: f32,
}

impl Definition {
    fn spec(self) -> Spec {
        match self {
            Self::Sd => Spec {
                scale: 2,
                steps: 4,
                dt: 1.0 / 240.0,
            },
            Self::Hd => Spec {
                scale: 1,
                steps: 8,
                dt: 1.0 / 480.0,
            },
        }
    }

    fn slot(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sd => "sd",
            Self::Hd => "hd",
        }
    }
}

pub const LIFT_SLOTS: usize = 4;

pub const SPLASH_SLOTS: usize = 32;
pub const QUIVER_SLOTS: usize = 4;
pub const BULGE_CEIL: f32 = 12.0;
pub const TOUCH_SLOTS: usize = 12;

const MASK_BYTES: u64 = 1616;

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

/// One quivering control: a small plate vibrating at the surface. Drives the
/// chromatic pull toward the pointer, a small pulsing bulge, and a continuous
/// tremor wavetrain. Physical pixels; `grip` is the 0..=1 engagement.
#[derive(Clone, Copy, Debug)]
pub struct Tension {
    pub id: u64,
    pub rect: egui::Rect,
    pub pointer: egui::Pos2,
    pub grip: f32,
    /// Per-control angular pulse rate. Zero means "use the global button
    /// tremor" so old/default seeds stay compact.
    pub omega: f32,
}

/// A raised pane in the water. Surface panes are image tiles hauled up to the
/// air; shallow panes are readable UI glass just below the surface. Physical
/// pixels; `grip` is the 0..=1 engagement.
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

/// One ring radiating from a plate's hull: the wavefront is an expanding
/// iso-contour of the rect's SDF, so the plate's own face is never lensed.
/// Physical pixels; `age` in seconds since the plunge.
#[derive(Clone, Copy, Debug)]
pub struct Splash {
    pub rect: egui::Rect,
    pub age: f32,
    pub amp: f32,
}

/// A fingertip ripple inside the full-image viewer. Unlike `Splash`, this is
/// a point disturbance living inside a bounded pond whose image rect reflects
/// it.
#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub center: egui::Pos2,
    pub age: f32,
    pub amp: f32,
}

/// Everything the composite pass draws in one frame: the viewer veil, the
/// hovered button's tension, the risen lift plates, and the splashes rippling
/// across the water (whose surface is `water`, in physical px — the panel to
/// its left is the shallows).
pub struct Surge<'a> {
    pub dry: bool,
    pub veil: Option<Veil>,
    pub tensions: &'a [Tension],
    pub lifts: &'a [Lift],
    pub water: egui::Rect,
    pub scroll_tilt: f32,
    pub splashes: &'a [Splash],
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
    /// Is the water disturbed at all? When false the frost pass is skipped.
    pub fn live(&self) -> bool {
        !self.dry
            && (self.veil.is_some()
                || !self.tensions.is_empty()
                || !self.lifts.is_empty()
                || !self.splashes.is_empty()
                || !self.touches.is_empty()
                || self.wake)
    }
}

/// Composite parameters, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Veil {
    /// Rounded-rect cutouts kept sharp (viewer window; clicked tile + menu).
    pub cuts: [Cut; 2],
    /// 0..=1 mix toward the frosted backdrop.
    pub strength: f32,
    /// How hard the frosted region is dimmed toward black.
    pub dim: f32,
    /// 0..=1 share of blur in the backdrop; at zero the Kawase chain is skipped
    /// entirely and the veil is a pure dim.
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
    pipes: [Pipes; 2],
    sampler: wgpu::Sampler,
    mask: wgpu::Buffer,
    format: wgpu::TextureFormat,
    definition: Definition,
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
    definition: Definition,
}

struct Water {
    size: wgpu::Extent3d,
    textures: Vec<wgpu::Texture>,
    composite_bind: Vec<wgpu::BindGroup>,
    sim_bind: Vec<wgpu::BindGroup>,
    phase: usize,
}

struct Target {
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
                texture_entry(4),
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
        let pipes = |definition: Definition| {
            let label = definition.label();
            let spec = definition.spec();
            let field = [("FIELD_SCALE", f64::from(spec.scale))];
            let impulse = 4.0 / spec.steps as f64;
            let sim_consts = [
                ("SIM_SCALE", f64::from(spec.scale)),
                ("DT", f64::from(spec.dt)),
                ("IMPULSE_GAIN", impulse),
            ];
            let composite_label = format!("frost-composite-{label}");
            let sim_label = format!("frost-sim-{label}");
            Pipes {
                composite: pipeline(&composite_label, &composite_layout, "composite", &field),
                sim: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(&sim_label),
                    layout: Some(&sim_layout_handle),
                    module: &sim_module,
                    entry_point: Some("step"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &sim_consts,
                        ..Default::default()
                    },
                    cache: None,
                }),
            }
        };
        Self {
            down: pipeline("frost-down", &sample_layout, "kawase_down", &[]),
            up: pipeline("frost-up", &sample_layout, "kawase_up", &[]),
            pipes: [pipes(Definition::Sd), pipes(Definition::Hd)],
            sample_layout,
            composite_layout,
            sim_layout,
            sampler,
            mask,
            format,
            definition: Definition::default(),
            rig: None,
            sentinel: guard::Sentinel::default(),
        }
    }

    pub fn set_definition(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        definition: Definition,
    ) {
        if self.definition == definition {
            return;
        }
        self.definition = definition;
        self.resize(device, queue, width, height);
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
                    | wgpu::TextureUsages::TEXTURE_BINDING,
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
            Target { view, bind }
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
            definition: self.definition,
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
        let pipes = &self.pipes[rig.definition.slot()];
        for _ in 0..rig.definition.spec().steps {
            run_compute(
                encoder,
                &pipes.sim,
                &rig.water.sim_bind[rig.water.phase],
                rig.water.size,
            );
            rig.water.phase ^= 1;
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
            &pipes.composite,
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

    fn water(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        scene: &Target,
        blur: &Target,
    ) -> Water {
        let scale = self.definition.spec().scale;
        let size = wgpu::Extent3d {
            width: width.div_ceil(scale).max(1),
            height: height.div_ceil(scale).max(1),
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
    // splashes @ byte 288 (lane 72): rect, then age + amp + pad.
    for (slot, splash) in surge.splashes.iter().take(SPLASH_SLOTS).enumerate() {
        let at = 72 + slot * 8;
        lanes[at..at + 8].copy_from_slice(&[
            splash.rect.min.x,
            splash.rect.min.y,
            splash.rect.max.x,
            splash.rect.max.y,
            splash.age,
            splash.amp,
            0.0,
            0.0,
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
    let mut bytes = [0_u8; MASK_BYTES as usize];
    for (slot, lane) in lanes.iter().enumerate() {
        bytes[slot * 4..slot * 4 + 4].copy_from_slice(&lane.to_le_bytes());
    }
    bytes
}

const WGSL: &str = r"
struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> VsOut {
    var out: VsOut;
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.pos = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn kawase_down(in: VsOut) -> @location(0) vec4f {
    let half_texel = 0.5 / vec2f(textureDimensions(tex));
    var color = textureSample(tex, samp, in.uv) * 4.0;
    color += textureSample(tex, samp, in.uv - half_texel);
    color += textureSample(tex, samp, in.uv + half_texel);
    color += textureSample(tex, samp, in.uv + vec2f(half_texel.x, -half_texel.y));
    color += textureSample(tex, samp, in.uv - vec2f(half_texel.x, -half_texel.y));
    return color / 8.0;
}

@fragment
fn kawase_up(in: VsOut) -> @location(0) vec4f {
    let t = 1.0 / vec2f(textureDimensions(tex));
    var color = textureSample(tex, samp, in.uv + vec2f(-t.x * 2.0, 0.0));
    color += textureSample(tex, samp, in.uv + vec2f(-t.x, t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(0.0, t.y * 2.0));
    color += textureSample(tex, samp, in.uv + vec2f(t.x, t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(t.x * 2.0, 0.0));
    color += textureSample(tex, samp, in.uv + vec2f(t.x, -t.y)) * 2.0;
    color += textureSample(tex, samp, in.uv + vec2f(0.0, -t.y * 2.0));
    color += textureSample(tex, samp, in.uv + vec2f(-t.x, -t.y)) * 2.0;
    return color / 12.0;
}

struct Mask {
    a_min: vec2f,
    a_max: vec2f,
    b_min: vec2f,
    b_max: vec2f,
    water_min: vec2f,
    water_max: vec2f,
    radius_a: f32,
    radius_b: f32,
    strength: f32,
    dim: f32,
    blur: f32,
    tide: f32,
    scroll_tilt: f32,
    _pad1: f32,
    lift_rects: array<vec4f, 4>,
    lift_grips: vec4f,
    quivers: array<Quiver, 4>,
    splashes: array<Splash, 32>,
    viewer_min: vec2f,
    viewer_max: vec2f,
    touches: array<Touch, 12>,
    reach: f32,
    meniscus_px: f32,
    refract_px: f32,
    ior_spread: f32,
    quiver_bulge: f32,
    quiver_pulse: f32,
    tremor_k: f32,
    tremor_omega: f32,
    tremor_amp: f32,
    tremor_fade: f32,
    tremor_reach: f32,
    bulge_px: f32,
    lift_bright: f32,
    wave_v: f32,
    wave_sigma: f32,
    wave_damp: f32,
    wave_spread: f32,
    source_gain: f32,
    height_retention: f32,
    tilt_gain: f32,
    t_panel: f32,
    r_panel: f32,
    r_wall: f32,
    shore_feather: f32,
}

// touch.xy = pointer, touch.z = grip, touch.w = optional angular pulse rate.
struct Quiver {
    rect: vec4f,
    touch: vec4f,
}

// vitals.x = age seconds, vitals.y = amplitude px, vitals.zw = pad.
struct Splash {
    rect: vec4f,
    vitals: vec4f,
}

// wave.xy = center, wave.z = age seconds, wave.w = amplitude px.
struct Touch {
    wave: vec4f,
}

// The water's chemistry lives in the mask's brine block (runtime-tunable);
// only the rounded-rect corner radius is baked in.
const LIFT_RADIUS: f32 = 3.0;
const PLATE_FEATHER: f32 = 6.0;
const PLATE_LIFT_GAIN: f32 = 2.0;
const PLATE_DRY_GAIN: f32 = 5.0;
const SHALLOW_BULGE_GAIN: f32 = 0.18;
const SHALLOW_BRIGHT_GAIN: f32 = 0.35;
const SHALLOW_MASS_GAIN: f32 = 1.35;
override FIELD_SCALE: f32 = 2.0;
const FIELD_HEIGHT_CEIL: f32 = 48.0;
const FIELD_FLOW_CEIL: f32 = 18.0;

@group(0) @binding(0) var sharp_tex: texture_2d<f32>;
@group(0) @binding(1) var blur_tex: texture_2d<f32>;
@group(0) @binding(2) var comp_samp: sampler;
@group(0) @binding(3) var<uniform> mask: Mask;
@group(0) @binding(4) var water_tex: texture_2d<f32>;

// Signed distance to a rounded rect: negative inside, in pixels.
fn sd_cut(px: vec2f, rect_min: vec2f, rect_max: vec2f, radius: f32) -> f32 {
    let center = (rect_min + rect_max) * 0.5;
    let half_size = (rect_max - rect_min) * 0.5 - radius;
    let q = abs(px - center) - half_size;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

// Shore crossing gain: full strength on the source's side of the panel
// shelf, a faint transmitted remnant across the depth step.
fn crossing(shore_px: f32, src_x: f32) -> f32 {
    let src_water = step(mask.water_min.x, src_x);
    let same = 1.0 - abs(shore_px - src_water);
    return mix(mask.t_panel, 1.0, same);
}

// Split a single physical refraction vector through the booru-fluid's
// wavelength-dependent index of refraction. There is one water surface; RGB
// merely bend through it by slightly different amounts.
fn prism(flow: vec2f) -> mat3x2f {
    let g = flow * mask.refract_px;
    let spread = mask.ior_spread;
    return mat3x2f(g * max(0.0, 1.0 - spread), g, g * (1.0 + spread));
}

fn finite(x: f32) -> bool {
    return x == x && abs(x) < 1e20;
}

// C¹ island membership from a signed distance field: 1 well inside, 0 outside,
// and smooth across the shore. Every plate/wave operation consumes this
// instead of step tests, so field composition is order-independent.
fn island(sd: f32) -> f32 {
    return 1.0 - smoothstep(-PLATE_FEATHER, PLATE_FEATHER, sd);
}

fn field_obstacle(px: vec2f) -> f32 {
    var block = max(
        island(sd_cut(px, mask.a_min, mask.a_max, mask.radius_a)),
        island(sd_cut(px, mask.b_min, mask.b_max, mask.radius_b)),
    );
    for (var i = 0u; i < 4u; i = i + 1u) {
        let g = abs(mask.lift_grips[i]);
        if (g <= 0.0) {
            continue;
        }
        let r = mask.lift_rects[i];
        block = max(block, island(sd_cut(px, r.xy, r.zw, LIFT_RADIUS)) * g);
    }
    return clamp(block, 0.0, 1.0);
}

fn lift_warp(px: vec2f, rect: vec4f, grow: f32) -> vec2f {
    let emin = rect.xy - vec2f(grow);
    let emax = rect.zw + vec2f(grow);
    let away = px - (emin + emax) * 0.5;
    let half_t = (rect.zw - rect.xy) * 0.5;
    let half_b = half_t + vec2f(grow);
    let s = max(half_b.x, half_b.y) / max(max(half_t.x, half_t.y), 1.0);
    return away / s - away;
}

fn quiver_omega(q: Quiver) -> f32 {
    return select(mask.tremor_omega, q.touch.w, q.touch.w > 0.0);
}

// Fingertip point disturbance inside the viewer pond.
fn touch_flow(px: vec2f, center: vec2f, age: f32, amp: f32) -> vec2f {
    let zero = vec2f(0.0);
    let ray = px - center;
    let d = length(ray);
    let travel = mask.wave_v * age;
    if (abs(d - travel) > 4.0 * mask.wave_sigma + 0.05 * travel) {
        return zero;
    }
    let a = amp * exp(-age / mask.wave_damp) / sqrt(1.0 + d / mask.wave_spread);
    let dir = ray / max(d, 1e-3);
    let s = (d - travel) / mask.wave_sigma;
    return dir * (a * s * exp(-s * s * 0.5));
}

fn sane_height(x: f32) -> f32 {
    return clamp(select(0.0, x, finite(x)), -FIELD_HEIGHT_CEIL, FIELD_HEIGHT_CEIL);
}

fn field_uv(px: vec2f) -> vec2f {
    let dims = vec2f(textureDimensions(water_tex));
    return clamp(px / (dims * FIELD_SCALE), vec2f(0.0), vec2f(1.0));
}

fn sample_height(px: vec2f) -> f32 {
    return sane_height(textureSampleLevel(water_tex, comp_samp, field_uv(px), 0.0).x);
}

fn sample_visible_height(px: vec2f, center_h: f32) -> f32 {
    return mix(sample_height(px), center_h, field_obstacle(px));
}

fn field_flow(px: vec2f) -> vec2f {
    let center_h = sample_height(px);
    let hx = sample_visible_height(px + vec2f(FIELD_SCALE, 0.0), center_h)
        - sample_visible_height(px - vec2f(FIELD_SCALE, 0.0), center_h);
    let hy = sample_visible_height(px + vec2f(0.0, FIELD_SCALE), center_h)
        - sample_visible_height(px - vec2f(0.0, FIELD_SCALE), center_h);
    var flow = -vec2f(hx, hy) * (4.5 / FIELD_SCALE);
    let mag = length(flow);
    flow = flow * min(1.0, FIELD_FLOW_CEIL / max(mag, 1e-4));
    return flow;
}

@fragment
fn composite(in: VsOut) -> @location(0) vec4f {
    let size = vec2f(textureDimensions(sharp_tex));
    let px = in.uv * size;
    let shore_px = smoothstep(-mask.shore_feather, mask.shore_feather, px.x - mask.water_min.x);

    // The veil's sharp cutouts are dry land: no wave or pull reaches the
    // viewed image (or the tag menu). 0 inside a cut, 1 in open water.
    let dist = min(
        sd_cut(px, mask.a_min, mask.a_max, mask.radius_a),
        sd_cut(px, mask.b_min, mask.b_max, mask.radius_b),
    );
    let outside = smoothstep(-1.0, 1.0, dist);

    // Plates at the surface: image lifts (big) and button quivers (small,
    // pulsing at the tremor rate). Achromatic — a surfaced plate is dry.
    // They superpose as smooth islands; no step-function holes, no ordering
    // dependence, no wavefront getting guillotined by a tile edge.
    var flow_num = vec2f(0.0);
    var tint_num = 0.0;
    var plate_mass = 0.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let raw_g = mask.lift_grips[i];
        let g = abs(raw_g);
        if (g <= 0.0) {
            continue;
        }
        let shallow = select(0.0, 1.0, raw_g < 0.0);
        let rect = mask.lift_rects[i];
        let grow = mask.bulge_px * g * mix(1.0, SHALLOW_BULGE_GAIN, shallow);
        let emin = rect.xy - vec2f(grow);
        let emax = rect.zw + vec2f(grow);
        let erad = LIFT_RADIUS + grow;
        let bd = sd_cut(px, emin, emax, erad);
        let w = island(bd) * g;
        flow_num = flow_num + lift_warp(px, rect, grow) * w;
        tint_num = tint_num + mask.lift_bright * g * w * mix(1.0, SHALLOW_BRIGHT_GAIN, shallow);
        plate_mass = plate_mass + w * mix(1.0, SHALLOW_MASS_GAIN, shallow);
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        let grow = g * mask.quiver_bulge * (1.0 + mask.quiver_pulse * sin(quiver_omega(q) * mask.tide));
        let emin = q.rect.xy - vec2f(grow);
        let emax = q.rect.zw + vec2f(grow);
        let bd = sd_cut(px, emin, emax, LIFT_RADIUS + grow);
        let w = island(bd) * g;
        flow_num = flow_num + lift_warp(px, q.rect, grow) * w;
        tint_num = tint_num + mask.lift_bright * 0.5 * g * w;
        plate_mass = plate_mass + w;
    }
    let plate_lift = 1.0 - exp(-PLATE_LIFT_GAIN * plate_mass);
    let flow = flow_num / max(plate_mass, 1e-4) * plate_lift;
    let tint = 1.0 + tint_num / max(plate_mass, 1e-4) * plate_lift;
    let dry = outside * exp(-PLATE_DRY_GAIN * plate_mass);

    // Persistent background water: the compute pass owns the height field;
    // composite samples its slope and keeps only local meniscus pulls here.
    var water_flow = field_flow(px) * crossing(shore_px, px.x);
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        // Capillary pull toward the hovering fingertip: still one scalar
        // surface deformation, split chromatically only at the final prism.
        let to_ptr = q.touch.xy - px;
        let span = length(to_ptr);
        let p = exp(-(span * span) / (mask.reach * mask.reach));
        let inside = clamp(-sd_cut(px, q.rect.xy, q.rect.zw, 2.0), 0.0, 1.0);
        let bend = p * inside * g;
        let tdir = to_ptr / max(span, 1.0);
        water_flow = water_flow - tdir * (mask.meniscus_px * bend);
    }
    var viewer_flow = vec2f(0.0);
    let vx0 = mask.viewer_min.x;
    let vx1 = mask.viewer_max.x;
    let vy0 = mask.viewer_min.y;
    let vy1 = mask.viewer_max.y;
    for (var i = 0u; i < 12u; i = i + 1u) {
        let touch = mask.touches[i].wave;
        let amp = touch.w;
        if (amp <= 0.0) {
            continue;
        }
        let c = touch.xy;
        let age = touch.z;
        viewer_flow = viewer_flow
            + touch_flow(px, c, age, amp)
            + touch_flow(px, vec2f(2.0 * vx0 - c.x, c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(2.0 * vx1 - c.x, c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(c.x, 2.0 * vy0 - c.y), age, amp * mask.r_wall)
            + touch_flow(px, vec2f(c.x, 2.0 * vy1 - c.y), age, amp * mask.r_wall);
    }
    let viewer_wet = 1.0 - smoothstep(
        -1.0,
        1.0,
        sd_cut(px, mask.viewer_min, mask.viewer_max, 0.0),
    );

    // One wavelength-split sampler: lift flow achromatic, waves and tension
    // chromatic. Background waves halt inside veil cutouts; viewer touches
    // are a separate bounded pond inside the full image.
    let lift_flow = flow * outside;
    let wet = prism(water_flow) * dry + prism(viewer_flow) * viewer_wet;
    let uv_r = in.uv + (lift_flow + wet[0]) / size;
    let uv_g = in.uv + (lift_flow + wet[1]) / size;
    let uv_b = in.uv + (lift_flow + wet[2]) / size;
    let r = textureSample(sharp_tex, comp_samp, uv_r).r;
    let g = textureSample(sharp_tex, comp_samp, uv_g).g;
    let b = textureSample(sharp_tex, comp_samp, uv_b).b;
    let a = textureSample(sharp_tex, comp_samp, in.uv + lift_flow / size).a;
    let sharp = vec4f(vec3f(r, g, b) * mix(1.0, tint, outside), a);

    let blurred = textureSample(blur_tex, comp_samp, in.uv);
    // Feather inward (fully frosted at the boundary) so the cutout's own
    // border hides the transition and no sharp fringe leaks outside.
    let base = mix(sharp, blurred, mask.blur);
    let frosted = vec4f(base.rgb * mask.dim, base.a);
    let veiled = mix(sharp, frosted, mask.strength);
    return mix(sharp, veiled, outside);
}
";

const SIM_WGSL: &str = r"
struct Mask {
    a_min: vec2f,
    a_max: vec2f,
    b_min: vec2f,
    b_max: vec2f,
    water_min: vec2f,
    water_max: vec2f,
    radius_a: f32,
    radius_b: f32,
    strength: f32,
    dim: f32,
    blur: f32,
    tide: f32,
    scroll_tilt: f32,
    _pad1: f32,
    lift_rects: array<vec4f, 4>,
    lift_grips: vec4f,
    quivers: array<Quiver, 4>,
    splashes: array<Splash, 32>,
    viewer_min: vec2f,
    viewer_max: vec2f,
    touches: array<Touch, 12>,
    reach: f32,
    meniscus_px: f32,
    refract_px: f32,
    ior_spread: f32,
    quiver_bulge: f32,
    quiver_pulse: f32,
    tremor_k: f32,
    tremor_omega: f32,
    tremor_amp: f32,
    tremor_fade: f32,
    tremor_reach: f32,
    bulge_px: f32,
    lift_bright: f32,
    wave_v: f32,
    wave_sigma: f32,
    wave_damp: f32,
    wave_spread: f32,
    source_gain: f32,
    height_retention: f32,
    tilt_gain: f32,
    t_panel: f32,
    r_panel: f32,
    r_wall: f32,
    shore_feather: f32,
}

struct Quiver {
    rect: vec4f,
    touch: vec4f,
}

struct Splash {
    rect: vec4f,
    vitals: vec4f,
}

struct Touch {
    wave: vec4f,
}

override SIM_SCALE: f32 = 2.0;
override DT: f32 = 1.0 / 240.0;
override IMPULSE_GAIN: f32 = 1.0;
const LIFT_RADIUS: f32 = 3.0;
const PLATE_FEATHER: f32 = 6.0;
const SOURCE_LIFE: f32 = 0.22;
const SOURCE_CEIL: f32 = 72.0;
const H_CEIL: f32 = 48.0;
const V_CEIL: f32 = 1440.0;
const TILT_CEIL: f32 = 36.0;

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var dst_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> mask: Mask;

fn cell_px(p: vec2i) -> vec2f {
    return (vec2f(p) + vec2f(0.5)) * SIM_SCALE;
}

fn sd_cut(px: vec2f, rect_min: vec2f, rect_max: vec2f, radius: f32) -> f32 {
    let center = (rect_min + rect_max) * 0.5;
    let half_size = (rect_max - rect_min) * 0.5 - radius;
    let q = abs(px - center) - half_size;
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn island(sd: f32) -> f32 {
    return 1.0 - smoothstep(-PLATE_FEATHER, PLATE_FEATHER, sd);
}

fn finite(x: f32) -> bool {
    return x == x && abs(x) < 1e20;
}

fn sane(x: f32, ceil: f32) -> f32 {
    return clamp(select(0.0, x, finite(x)), -ceil, ceil);
}

fn soft_limiter(x: f32, ceil: f32) -> f32 {
    return ceil * x / (abs(x) + ceil);
}

fn quiver_omega(q: Quiver) -> f32 {
    return select(mask.tremor_omega, q.touch.w, q.touch.w > 0.0);
}

fn obstacle(px: vec2f) -> f32 {
    var block = max(
        island(sd_cut(px, mask.a_min, mask.a_max, mask.radius_a)),
        island(sd_cut(px, mask.b_min, mask.b_max, mask.radius_b)),
    );
    for (var i = 0u; i < 4u; i = i + 1u) {
        let g = abs(mask.lift_grips[i]);
        if (g <= 0.0) {
            continue;
        }
        let r = mask.lift_rects[i];
        block = max(block, island(sd_cut(px, r.xy, r.zw, LIFT_RADIUS)) * g);
    }
    return clamp(block, 0.0, 1.0);
}

fn load_state(p: vec2i, dims: vec2i) -> vec2f {
    let raw = textureLoad(src_tex, clamp(p, vec2i(0), dims - vec2i(1)), 0).xy;
    return vec2f(sane(raw.x, H_CEIL), sane(raw.y, V_CEIL));
}

fn wall_height(p: vec2i, dims: vec2i, h: f32) -> f32 {
    let q = clamp(p, vec2i(0), dims - vec2i(1));
    let b = obstacle(cell_px(q));
    return mix(load_state(q, dims).x, h, b);
}

fn source_shell(px: vec2f, rect: vec4f, age: f32, amp: f32) -> f32 {
    if (amp == 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    let d = sd_cut(px, rect.xy, rect.zw, LIFT_RADIUS);
    if (d < -PLATE_FEATHER) {
        return 0.0;
    }
    let shell = exp(-0.5 * pow(max(d, 0.0) / max(mask.wave_sigma, 1.0), 2.0));
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    return amp * shell * birth;
}

fn source(px: vec2f) -> f32 {
    var drive = 0.0;
    for (var i = 0u; i < 32u; i = i + 1u) {
        let splash = mask.splashes[i];
        drive = drive + source_shell(px, splash.rect, splash.vitals.x, splash.vitals.y);
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        let d = sd_cut(px, q.rect.xy, q.rect.zw, LIFT_RADIUS);
        if (d <= 0.0 || d > mask.tremor_reach) {
            continue;
        }
        let shell = exp(-d / max(mask.tremor_fade, 1.0));
        let phase = mask.tremor_k * d - quiver_omega(q) * mask.tide;
        drive = drive + mask.tremor_amp * g * shell * sin(phase);
    }
    return soft_limiter(drive, SOURCE_CEIL);
}

fn tilt_drive(px: vec2f, h: f32) -> f32 {
    let span = max(mask.water_max.y - mask.water_min.y, 1.0);
    let y = clamp((px.y - mask.water_min.y) / span, 0.0, 1.0);
    let ramp = y * 2.0 - 1.0;
    let lip = max(SIM_SCALE * 2.0, 1.0);
    let y_gate = smoothstep(mask.water_min.y, mask.water_min.y + lip, px.y)
        * (1.0 - smoothstep(mask.water_max.y - lip, mask.water_max.y, px.y));
    let x_gate = smoothstep(
        mask.water_min.x - mask.shore_feather,
        mask.water_min.x + mask.shore_feather,
        px.x,
    );
    let desired = -clamp(mask.scroll_tilt, -TILT_CEIL, TILT_CEIL) * ramp;
    return (desired - h) * max(mask.tilt_gain, 0.0) * x_gate * y_gate;
}

@compute @workgroup_size(8, 8, 1)
fn step(@builtin(global_invocation_id) gid: vec3u) {
    let dims_u = textureDimensions(src_tex);
    if (gid.x >= dims_u.x || gid.y >= dims_u.y) {
        return;
    }
    let dims = vec2i(dims_u);
    let p = vec2i(gid.xy);
    let px = cell_px(p);
    let here = load_state(p, dims);
    let h = here.x;

    let l = wall_height(p + vec2i(-1, 0), dims, h);
    let r = wall_height(p + vec2i(1, 0), dims, h);
    let u = wall_height(p + vec2i(0, -1), dims, h);
    let d = wall_height(p + vec2i(0, 1), dims, h);
    let lap = (l + r + u + d - 4.0 * h) / (SIM_SCALE * SIM_SCALE);

    let shelf = smoothstep(-mask.shore_feather, mask.shore_feather, px.x - mask.water_min.x);
    let shelf_speed = clamp((1.0 - mask.r_panel) / (1.0 + mask.r_panel), 0.2, 1.0);
    let cfl = 0.66 * SIM_SCALE / DT;
    let c = min(mask.wave_v * mix(shelf_speed, 1.0, shelf), cfl);
    var v = here.y
        + c * c * lap * DT
        + source(px) * mask.source_gain * IMPULSE_GAIN
        + tilt_drive(px, h) * DT;
    v = v * exp(-DT / max(mask.wave_damp, 0.08)) * mix(0.985, 1.0, shelf);

    let block = obstacle(px);
    v = mix(v, 0.0, block);
    let keep = clamp(mask.height_retention, 0.95, 1.0);
    var next_h = mix((h + v * DT) * keep, h * keep, block);
    v = sane(v, V_CEIL);
    next_h = sane(next_h, H_CEIL);
    textureStore(dst_tex, p, vec4f(next_h, v, 0.0, 0.0));
}
";
