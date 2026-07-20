//! The boiler room: bespoke winit + wgpu + egui integration.
//!
//! We own the event loop and the render graph outright (no eframe) so the
//! frame can route through arbitrary GPU passes — today the water veil, later
//! whatever else. The common path stays bare-metal: with no veil up, egui
//! rasterizes straight into the swapchain exactly as eframe would, and the
//! loop sleeps until input, a worker klaxon, or an egui-scheduled deadline.

use anyhow::{Context as _, Result};
use egui_wgpu::{RenderState, RendererOptions, ScreenDescriptor, WgpuConfiguration, wgpu};
use egui_winit::winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::{ElementState, StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{Window, WindowAttributes},
};
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use crate::{app::Bayonet, trace::startup, water::Engine};

const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1440.0, 920.0);

/// User event that wakes the loop; the alarm carries the actual deadline.
#[derive(Clone, Copy, Debug)]
struct Spark;

/// Earliest pending repaint deadline, shared with egui's repaint callback.
type Alarm = Arc<Mutex<Option<Instant>>>;

pub fn run(ctx: egui::Context, app: Bayonet) -> Result<()> {
    let event_loop = EventLoop::<Spark>::with_user_event()
        .build()
        .context("build event loop")?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let alarm = Alarm::default();
    arm_repaints(&ctx, alarm.clone(), event_loop.create_proxy());
    let mut boiler = Boiler {
        ctx,
        app,
        alarm,
        rig: None,
        modifiers: ModifiersState::empty(),
        dump_next: false,
    };
    startup("boiler.loop.enter");
    event_loop.run_app(&mut boiler).context("run event loop")
}

/// Routes egui repaint requests (from frames and worker threads alike) into
/// the alarm + a loop wake-up.
fn arm_repaints(ctx: &egui::Context, alarm: Alarm, proxy: EventLoopProxy<Spark>) {
    ctx.set_request_repaint_callback(move |info| {
        let when = Instant::now() + info.delay;
        advance_alarm(&alarm, when);
        let _woken = proxy.send_event(Spark);
    });
}

fn advance_alarm(alarm: &Alarm, when: Instant) {
    let mut alarm = lock_alarm(alarm);
    if alarm.is_none_or(|set| when < set) {
        *alarm = Some(when);
    }
}

fn lock_alarm(alarm: &Alarm) -> MutexGuard<'_, Option<Instant>> {
    match alarm.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct Boiler {
    ctx: egui::Context,
    app: Bayonet,
    alarm: Alarm,
    rig: Option<Rig>,
    modifiers: ModifiersState,
    dump_next: bool,
}

impl Boiler {
    fn paint(&mut self) {
        if self.rig.is_none() {
            return;
        }
        let dump_path = self.take_dump_path();
        let Some(rig) = self.rig.as_mut() else {
            return;
        };
        let raw_input = rig.input.take_egui_input(&rig.window);
        let app = &mut self.app;
        let output = self.ctx.run_ui(raw_input, |ui| app.pulse(ui));
        #[cfg(feature = "devtools")]
        self.app.probe_dump(&self.ctx, output.pixels_per_point);
        rig.input
            .handle_platform_output(&rig.window, output.platform_output);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let tooltip_rects = tooltip_rects(&self.ctx);
        let water = self
            .app
            .water_frame(&self.ctx, output.pixels_per_point, &tooltip_rects);
        if water.wants_repaint() {
            rig.window.request_redraw();
        }
        let dump = rig.render(
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
            &water,
            dump_path.as_deref(),
        );
        if let (Some(path), Some(result)) = (dump_path.as_deref(), dump) {
            self.app.report_debug_dump(result.map(|()| path));
        }
        if let Some(viewport) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            if viewport.repaint_delay.is_zero() {
                rig.window.request_redraw();
            } else if let Some(when) = Instant::now().checked_add(viewport.repaint_delay) {
                advance_alarm(&self.alarm, when);
            }
        }
    }

    fn take_dump_path(&mut self) -> Option<std::path::PathBuf> {
        if !self.dump_next {
            return None;
        }
        self.dump_next = false;
        match self.app.debug_dump_path() {
            Ok(path) => Some(path),
            Err(err) => {
                self.app.report_debug_dump(Err(err));
                None
            }
        }
    }

    /// Fires a redraw if the alarm deadline has arrived.
    fn tend_alarm(&self) {
        let Some(rig) = &self.rig else {
            return;
        };
        let mut alarm = lock_alarm(&self.alarm);
        if alarm.is_some_and(|when| when <= Instant::now()) {
            *alarm = None;
            rig.window.request_redraw();
        }
    }
}

fn tooltip_rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.memory(|mem| {
        mem.layer_ids()
            .filter(|layer| layer.order == egui::Order::Tooltip && mem.areas().is_visible(layer))
            .filter_map(|layer| mem.area_rect(layer.id))
            .filter(|rect| rect.is_positive())
            .collect()
    })
}

impl ApplicationHandler<Spark> for Boiler {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.rig.is_some() {
            return;
        }
        match Rig::raise(event_loop, &self.ctx) {
            Ok(rig) => self.rig = Some(rig),
            Err(err) => {
                eprintln!("boiler failed to raise the window: {err:#}");
                event_loop.exit();
            }
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.tend_alarm();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _spark: Spark) {
        self.tend_alarm();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: egui_winit::winit::window::WindowId,
        event: WindowEvent,
    ) {
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && matches!(&event.logical_key, Key::Named(NamedKey::F10)) =>
            {
                if self.modifiers.shift_key() {
                    self.app.purge_debug_dumps();
                } else {
                    self.dump_next = true;
                }
                if let Some(rig) = &self.rig {
                    rig.window.request_redraw();
                }
                return;
            }
            WindowEvent::RedrawRequested => {
                self.paint();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(rig) = &mut self.rig {
                    rig.resize(*size);
                }
            }
            _ => {}
        }
        let Some(rig) = &mut self.rig else {
            return;
        };
        let response = rig.input.on_window_event(&rig.window, &event);
        if response.repaint {
            rig.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tend_alarm();
        let deadline = *lock_alarm(&self.alarm);
        event_loop.set_control_flow(match deadline {
            Some(when) => ControlFlow::WaitUntil(when),
            None => ControlFlow::Wait,
        });
    }
}

/// Everything that exists only once the window is up.
struct Rig {
    window: Arc<Window>,
    input: egui_winit::State,
    surface: wgpu::Surface<'static>,
    gpu: RenderState,
    config: wgpu::SurfaceConfiguration,
    water_engine: Engine,
}

impl Rig {
    fn raise(event_loop: &ActiveEventLoop, ctx: &egui::Context) -> Result<Self> {
        startup("boiler.rig.raise.enter");
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("adequate booru viewer")
                        .with_inner_size(WINDOW_SIZE),
                )
                .context("create window")?,
        );
        startup("boiler.window.created");
        let input = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        let configuration = WgpuConfiguration::default();
        let instance = pollster::block_on(configuration.wgpu_setup.new_instance());
        let surface = instance
            .create_surface(window.clone())
            .context("create surface")?;
        startup("boiler.surface.created");
        let gpu = pollster::block_on(RenderState::create(
            &configuration,
            &instance,
            Some(&surface),
            RendererOptions::default(),
        ))
        .context("create wgpu render state")?;
        startup("boiler.gpu.ready");
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&gpu.adapter, size.width.max(1), size.height.max(1))
            .context("surface is unsupported by the adapter")?;
        config.format = gpu.target_format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.view_formats = vec![gpu.target_format];
        surface.configure(&gpu.device, &config);
        let mut water_engine = Engine::new(&gpu.device, gpu.target_format);
        water_engine.resize(&gpu.device, config.width, config.height);
        startup("boiler.rig.raised");
        Ok(Self {
            window,
            input,
            surface,
            gpu,
            config,
            water_engine,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.gpu.device, &self.config);
        self.water_engine
            .resize(&self.gpu.device, size.width, size.height);
    }

    fn render(
        &mut self,
        primitives: &[egui::ClippedPrimitive],
        delta: &egui::TexturesDelta,
        pixels_per_point: f32,
        water: &crate::water::Frame,
        dump_path: Option<&Path>,
    ) -> Option<Result<()>> {
        let screen = ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point,
        };
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("boiler"),
            });
        let user_cmds = {
            let mut renderer = self.gpu.renderer.write();
            for (id, image_delta) in &delta.set {
                renderer.update_texture(&self.gpu.device, &self.gpu.queue, *id, image_delta);
            }
            renderer.update_buffers(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                primitives,
                &screen,
            )
        };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return None;
            }
            // Minimized / fully covered: skip; the next window event repaints.
            wgpu::CurrentSurfaceTexture::Occluded => return None,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                self.window.request_redraw();
                return None;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("boiler: surface texture validation failure");
                return None;
            }
        };
        let surface_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        if water.dry() {
            self.water_engine.becalm(&self.gpu.queue);
        }
        let wet_pass = water.live() && self.water_engine.scene_view().is_some();
        {
            let target = if wet_pass {
                self.water_engine.scene_view().unwrap_or(&surface_view)
            } else {
                &surface_view
            };
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
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
                })
                .forget_lifetime();
            self.gpu
                .renderer
                .read()
                .render(&mut pass, primitives, &screen);
        }
        if wet_pass {
            self.water_engine.compose(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &surface_view,
                water,
            );
        }
        let _submission = self
            .gpu
            .queue
            .submit(user_cmds.into_iter().chain([encoder.finish()]));
        if self
            .water_engine
            .after_submit(&self.gpu.device, &self.gpu.queue, water)
        {
            self.window.request_redraw();
        }
        let dump = dump_path.map(|path| {
            self.water_engine.dump(
                &self.gpu.device,
                &self.gpu.queue,
                path,
                water,
                screen.size_in_pixels,
                pixels_per_point,
            )
        });
        self.window.pre_present_notify();
        frame.present();
        // Free only after submit: destroying a texture the just-recorded
        // command buffer still references is a validation error.
        {
            let mut renderer = self.gpu.renderer.write();
            for id in &delta.free {
                renderer.free_texture(id);
            }
        }
        dump
    }
}
