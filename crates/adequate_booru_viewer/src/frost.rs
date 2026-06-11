//! The water under the UI: one composite pass that owns every shader effect.
//!
//! The boiler renders the whole UI into an offscreen `scene` texture; this
//! module composites it to the swapchain with (a) the frosted veil for the
//! viewer (dual-Kawase blur over a small mip chain, SDF rounded-rect cutouts
//! kept sharp), (b) the button tension refraction, (c) the grid's lift
//! plates, (d) a persistent damped shallow-water height field excited by
//! splashes, text, scroll shocks, and button tremors, and (e) the viewer's
//! separate bounded pond for click ripples. While nothing is live the boiler
//! bypasses all of this entirely.

use egui_wgpu::wgpu;
use std::f32::consts::TAU;

/// Mip levels in the blur chain: /2, /4, /8 of the surface.
const LEVELS: usize = 3;
/// Water simulation decimation. Solver cells are square, in physical pixels.
const SIM_SCALE: u32 = 2;
const SIM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SIM_BYTES: u32 = 8;
const SIM_STEPS: usize = 4;
const SIM_WORKGROUP: u32 = 8;
/// How many grid tiles may rise/relax at once — the depth of the slosh trail
/// as the pointer sweeps across the grid.
pub const LIFT_SLOTS: usize = 4;

/// How many splashes ride the water at once; the weakest old ring dies first.
pub const SPLASH_SLOTS: usize = 32;
/// How many button quivers (hovered + still fading) the mask carries.
pub const QUIVER_SLOTS: usize = 4;
/// Hard ceiling on the lift bulge. The shader also smooth-blends plate islands,
/// so this is a taste bound rather than a brittle overlap guard.
pub const BULGE_CEIL: f32 = 12.0;
/// Fingertip ripples inside the full-image viewer.
pub const TOUCH_SLOTS: usize = 12;

/// Mask uniform layout (all offsets 16-aligned where arrays demand it):
/// vec2f block @0..48 (cuts a/b, water), scalars @48..80 (radii, strength,
/// dim, blur, tide, 2 pad), `lift_rects` @80, `lift_grips` @144, quivers @160
/// (rect + pointer/grip per slot), splashes @288, viewer rect @1312,
/// touches @1328, brine @1520. 404 f32 lanes = 1616 bytes, matching WGSL.
const MASK_BYTES: u64 = 1616;

/// The water's chemistry: every shader-side tunable, runtime-adjustable via
/// the tide bench (F12). Defaults are the shipped feel. Physical px at 1x.
#[derive(Clone, Copy, Debug)]
pub struct Brine {
    /// Gaussian reach of the chromatic pull around the pointer.
    pub reach: f32,
    /// Capillary meniscus pull at full grip; one scalar field, later split by
    /// the refractive-index spread below.
    pub meniscus_px: f32,
    /// Global multiplier turning the height-field gradient into sample-space
    /// displacement.
    pub refract_px: f32,
    /// Magic booru-fluid differential refractive index. Red bends by
    /// `1 - spread`, blue by `1 + spread`; green is the reference ray.
    pub ior_spread: f32,
    /// The quiver's small bulge and how hard it pulses with the tremor.
    pub quiver_bulge: f32,
    pub quiver_pulse: f32,
    /// Tremor wavetrain: wavenumber k = 2π/λ, angular rate ω, amplitude,
    /// exponential fade, and hard range cutoff.
    pub tremor_k: f32,
    pub tremor_omega: f32,
    pub tremor_amp: f32,
    pub tremor_fade: f32,
    pub tremor_reach: f32,
    /// Lift plate: footprint growth at full grip and surfacing brightness.
    pub bulge_px: f32,
    pub lift_bright: f32,
    /// Persistent gallery solver + viewer pond waves: crest speed, source
    /// width, viscous decay seconds, viewer-only geometric spreading scale.
    pub wave_v: f32,
    pub wave_sigma: f32,
    pub wave_damp: f32,
    pub wave_spread: f32,
    /// Shore: chromatic transmission into the shallows, shelf impedance,
    /// viewer-wall reflectivity, boundary feather.
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
            tremor_amp: 0.55,
            tremor_fade: 55.0,
            tremor_reach: 150.0,
            bulge_px: 10.0,
            lift_bright: 0.08,
            wave_v: 320.0,
            wave_sigma: 14.0,
            wave_damp: 2.4,
            wave_spread: 480.0,
            t_panel: 0.12,
            r_panel: 0.35,
            r_wall: 0.6,
            shore_feather: 12.0,
        }
    }
}

/// One quivering button: a small plate vibrating at the surface. Drives the
/// chromatic pull toward the pointer, a small pulsing bulge, and a continuous
/// tremor wavetrain. Physical pixels; `grip` is the 0..=1 engagement.
#[derive(Clone, Copy, Debug)]
pub struct Tension {
    pub rect: egui::Rect,
    pub pointer: egui::Pos2,
    pub grip: f32,
}

/// Hover lift: a grid tile hauled to the surface — its footprint bulges out
/// over the neighbours and brightens as it nears the eye. Physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Lift {
    pub rect: egui::Rect,
    pub grip: f32,
}

/// One ring radiating from a plate's hull: the wavefront is an expanding
/// iso-contour of the rect's SDF, so the plate's own face is never lensed.
/// Physical pixels; `age` in seconds since the plunge.
#[derive(Clone, Copy, Debug)]
pub struct Splash {
    pub rect: egui::Rect,
    pub age: f32,
    pub amp: f32,
    /// x/y wall reflection enable. Scroll sheet waves set x=0 so ideal planar
    /// shocks do not invent side-wall corner waves.
    pub walls: egui::Vec2,
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
    pub veil: Option<Veil>,
    pub tensions: &'a [Tension],
    pub lifts: &'a [Lift],
    pub water: egui::Rect,
    pub splashes: &'a [Splash],
    pub viewer: egui::Rect,
    pub touches: &'a [Touch],
    /// Keep the persistent solver ticking while old energy decays, even after
    /// its one-frame exciters have fallen out of the CPU source lists.
    pub wake: bool,
    /// Wall-clock seconds (wrapped) driving the tremor wavetrains.
    pub tide: f32,
    pub brine: Brine,
}

impl Surge<'_> {
    /// Is the water disturbed at all? When false the frost pass is skipped.
    pub fn live(&self) -> bool {
        self.veil.is_some()
            || !self.tensions.is_empty()
            || !self.lifts.is_empty()
            || !self.splashes.is_empty()
            || !self.touches.is_empty()
            || self.wake
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
    composite: wgpu::RenderPipeline,
    sim: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    mask: wgpu::Buffer,
    format: wgpu::TextureFormat,
    rig: Option<Rig>,
}

/// The size-dependent resources, rebuilt on resize.
struct Rig {
    scene: Target,
    chain: Vec<Target>,
    water: Water,
}

struct Water {
    size: wgpu::Extent3d,
    _textures: Vec<wgpu::Texture>,
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
        let pipeline = |label, layout: &wgpu::BindGroupLayout, entry| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
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
        let sim = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("frost-sim"),
            layout: Some(&sim_layout_handle),
            module: &sim_module,
            entry_point: Some("step"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        Self {
            down: pipeline("frost-down", &sample_layout, "kawase_down"),
            up: pipeline("frost-up", &sample_layout, "kawase_up"),
            composite: pipeline("frost-composite", &composite_layout, "composite"),
            sim,
            sample_layout,
            composite_layout,
            sim_layout,
            sampler,
            mask,
            format,
            rig: None,
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
        });
    }

    /// Render target for the egui pass when a veil is up.
    pub fn scene_view(&self) -> Option<&wgpu::TextureView> {
        self.rig.as_ref().map(|rig| &rig.scene.view)
    }

    /// Composites the offscreen scene to `surface` with everything in `surge`.
    pub fn compose(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface: &wgpu::TextureView,
        surge: &Surge<'_>,
    ) {
        let Some(rig) = &mut self.rig else {
            return;
        };
        queue.write_buffer(&self.mask, 0, &mask_bytes(surge));
        for _ in 0..SIM_STEPS {
            run_compute(
                encoder,
                &self.sim,
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
            &self.composite,
            &rig.water.composite_bind[rig.water.phase],
            surface,
        );
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
            width: width.div_ceil(SIM_SCALE).max(1),
            height: height.div_ceil(SIM_SCALE).max(1),
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
            _textures: textures.into_iter().map(|(_, texture)| texture).collect(),
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
    // scalar block (bytes 48..80), two pad lanes to reach the arrays.
    lanes[12] = a.radius;
    lanes[13] = b.radius;
    lanes[14] = veil.strength.clamp(0.0, 1.0);
    lanes[15] = veil.dim;
    lanes[16] = veil.blur.clamp(0.0, 1.0);
    lanes[17] = surge.tide;
    // lift_rects @ byte 80 (lane 20); grips @ 144 (lane 36).
    for (slot, lift) in surge.lifts.iter().take(LIFT_SLOTS).enumerate() {
        let at = 20 + slot * 4;
        lanes[at..at + 4].copy_from_slice(&[
            lift.rect.min.x,
            lift.rect.min.y,
            lift.rect.max.x,
            lift.rect.max.y,
        ]);
        lanes[36 + slot] = lift.grip.clamp(0.0, 1.0);
    }
    // quivers @ byte 160 (lane 40): rect, then pointer + grip + pad.
    for (slot, quiver) in surge.tensions.iter().take(QUIVER_SLOTS).enumerate() {
        let at = 40 + slot * 8;
        lanes[at..at + 7].copy_from_slice(&[
            quiver.rect.min.x,
            quiver.rect.min.y,
            quiver.rect.max.x,
            quiver.rect.max.y,
            quiver.pointer.x,
            quiver.pointer.y,
            quiver.grip.clamp(0.0, 1.0),
        ]);
    }
    // splashes @ byte 288 (lane 72): rect, then age + amp + x/y wall mask.
    for (slot, splash) in surge.splashes.iter().take(SPLASH_SLOTS).enumerate() {
        let at = 72 + slot * 8;
        lanes[at..at + 8].copy_from_slice(&[
            splash.rect.min.x,
            splash.rect.min.y,
            splash.rect.max.x,
            splash.rect.max.y,
            splash.age,
            splash.amp,
            splash.walls.x,
            splash.walls.y,
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
    lanes[380..401].copy_from_slice(&[
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
    _pad0: f32,
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
    t_panel: f32,
    r_panel: f32,
    r_wall: f32,
    shore_feather: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
}

// touch.xy = pointer, touch.z = grip; .w pad.
struct Quiver {
    rect: vec4f,
    touch: vec4f,
}

// vitals.x = age seconds, vitals.y = amplitude px, vitals.zw = x/y wall mask.
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

// C¹ island membership from a signed distance field: 1 well inside, 0 outside,
// and smooth across the shore. Every plate/wave operation consumes this
// instead of step tests, so field composition is order-independent.
fn island(sd: f32) -> f32 {
    return 1.0 - smoothstep(-PLATE_FEATHER, PLATE_FEATHER, sd);
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

fn sample_height(coord: vec2i, dims: vec2i) -> f32 {
    let p = clamp(coord, vec2i(0), dims - vec2i(1));
    return textureLoad(water_tex, p, 0).x;
}

fn field_flow(px: vec2f) -> vec2f {
    let dims = vec2i(textureDimensions(water_tex));
    let p = clamp(vec2i(floor(px / 2.0)), vec2i(0), dims - vec2i(1));
    let dx = 2.0;
    let hx = sample_height(p + vec2i(1, 0), dims) - sample_height(p - vec2i(1, 0), dims);
    let hy = sample_height(p + vec2i(0, 1), dims) - sample_height(p - vec2i(0, 1), dims);
    return -vec2f(hx, hy) * (4.5 / dx);
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
        let g = mask.lift_grips[i];
        if (g <= 0.0) {
            continue;
        }
        let rect = mask.lift_rects[i];
        let grow = mask.bulge_px * g;
        let emin = rect.xy - vec2f(grow);
        let emax = rect.zw + vec2f(grow);
        let erad = LIFT_RADIUS + grow;
        let bd = sd_cut(px, emin, emax, erad);
        let w = island(bd) * g;
        flow_num = flow_num + lift_warp(px, rect, grow) * w;
        tint_num = tint_num + mask.lift_bright * g * w;
        plate_mass = plate_mass + w;
    }
    for (var i = 0u; i < 4u; i = i + 1u) {
        let q = mask.quivers[i];
        let g = q.touch.z;
        if (g <= 0.0) {
            continue;
        }
        let grow = g * mask.quiver_bulge * (1.0 + mask.quiver_pulse * sin(mask.tremor_omega * mask.tide));
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
    _pad0: f32,
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
    t_panel: f32,
    r_panel: f32,
    r_wall: f32,
    shore_feather: f32,
    _pad2: f32,
    _pad3: f32,
    _pad4: f32,
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

const SIM_SCALE: f32 = 2.0;
const DT: f32 = 1.0 / 240.0;
const LIFT_RADIUS: f32 = 3.0;
const PLATE_FEATHER: f32 = 6.0;
const SOURCE_GAIN: f32 = 38.0;
const SOURCE_SIGMA: f32 = 12.0;
const SOURCE_LIFE: f32 = 0.20;
const HEIGHT_BLEED: f32 = 0.9993;

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

fn obstacle(px: vec2f) -> f32 {
    var block = max(
        island(sd_cut(px, mask.a_min, mask.a_max, mask.radius_a)),
        island(sd_cut(px, mask.b_min, mask.b_max, mask.radius_b)),
    );
    for (var i = 0u; i < 4u; i = i + 1u) {
        let g = mask.lift_grips[i];
        if (g <= 0.0) {
            continue;
        }
        let r = mask.lift_rects[i];
        block = max(block, island(sd_cut(px, r.xy, r.zw, LIFT_RADIUS)) * g);
    }
    return clamp(block, 0.0, 1.0);
}

fn load_state(p: vec2i, dims: vec2i) -> vec2f {
    return textureLoad(src_tex, clamp(p, vec2i(0), dims - vec2i(1)), 0).xy;
}

fn wall_height(p: vec2i, dims: vec2i, h: f32) -> f32 {
    let q = clamp(p, vec2i(0), dims - vec2i(1));
    let b = obstacle(cell_px(q));
    return mix(textureLoad(src_tex, q, 0).x, h, b);
}

fn plateau(x: f32, lo: f32, hi: f32) -> f32 {
    return smoothstep(lo - SOURCE_SIGMA, lo + SOURCE_SIGMA, x)
        * (1.0 - smoothstep(hi - SOURCE_SIGMA, hi + SOURCE_SIGMA, x));
}

fn source_shell(px: vec2f, rect: vec4f, age: f32, amp: f32, walls: vec2f) -> f32 {
    if (amp <= 0.0 || age > SOURCE_LIFE) {
        return 0.0;
    }
    var shell = 0.0;
    if (walls.x > 0.5 && walls.y > 0.5) {
        let d = sd_cut(px, rect.xy, rect.zw, LIFT_RADIUS);
        if (d < -PLATE_FEATHER) {
            return 0.0;
        }
        shell = exp(-0.5 * pow(max(d, 0.0) / max(mask.wave_sigma, 1.0), 2.0));
    } else if (walls.y > 0.5) {
        let dy = max(max(rect.y - px.y, px.y - rect.w), 0.0);
        shell = exp(-0.5 * pow(dy / max(mask.wave_sigma, 1.0), 2.0))
            * plateau(px.x, rect.x, rect.z);
    } else if (walls.x > 0.5) {
        let dx = max(max(rect.x - px.x, px.x - rect.z), 0.0);
        shell = exp(-0.5 * pow(dx / max(mask.wave_sigma, 1.0), 2.0))
            * plateau(px.y, rect.y, rect.w);
    }
    let birth = 1.0 - smoothstep(0.0, SOURCE_LIFE, age);
    return amp * shell * birth;
}

fn source(px: vec2f) -> f32 {
    var drive = 0.0;
    for (var i = 0u; i < 32u; i = i + 1u) {
        let splash = mask.splashes[i];
        drive = drive
            + source_shell(px, splash.rect, splash.vitals.x, splash.vitals.y, splash.vitals.zw);
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
        let phase = mask.tremor_k * d - mask.tremor_omega * mask.tide;
        drive = drive + mask.tremor_amp * g * shell * sin(phase);
    }
    return drive;
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
    var v = here.y + c * c * lap * DT + source(px) * SOURCE_GAIN;
    v = v * exp(-DT / max(mask.wave_damp, 0.08)) * mix(0.985, 1.0, shelf);

    var next_h = (h + v * DT) * HEIGHT_BLEED;
    let wet = 1.0 - obstacle(px);
    next_h = next_h * wet;
    v = v * wet;
    textureStore(dst_tex, p, vec4f(next_h, v, 0.0, 0.0));
}
";
