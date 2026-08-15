use crate::{app::Bayonet, xdg::Lair};
use anyhow::Result;
use eternalist_apps::{CrashProduct, CrashReportSpec, NativeApp, WindowSpec};
use std::time::Instant;

pub fn run(ctx: egui::Context, pause_mirror: bool) -> Result<()> {
    eternalist_apps::run_with(ctx, |ctx| Bayonet::open(ctx, pause_mirror))
}

impl NativeApp for Bayonet {
    const WINDOW: WindowSpec = WindowSpec::new("adequate booru viewer", [1_440.0, 920.0]);

    fn crash_reports() -> Option<CrashReportSpec> {
        Lair::claim().ok().map(|lair| {
            CrashReportSpec::new(
                CrashProduct::BooruViewer,
                env!("CARGO_PKG_VERSION"),
                lair.state,
            )
        })
    }

    fn draw(&mut self, ui: &mut egui::Ui) {
        self.pulse(ui);
    }

    fn service_deadline(&self, now: Instant) -> Option<Instant> {
        Bayonet::service_deadline(self, now)
    }

    fn service_deadline_reached(&mut self, now: Instant) -> bool {
        Bayonet::service_deadline_reached(self, now)
    }

    fn after_present(&mut self) -> bool {
        false
    }

    fn water(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
        tooltip_rects: &[egui::Rect],
    ) -> brass_poolrooms::water::Frame {
        self.water_frame(ctx, pixels_per_point, tooltip_rects)
    }

    fn register_gpu(
        _renderer: &mut egui_wgpu::Renderer,
        _device: &egui_wgpu::wgpu::Device,
        _format: egui_wgpu::wgpu::TextureFormat,
    ) {
    }

    #[cfg(feature = "egui-test")]
    type Observation = crate::witness::State;

    #[cfg(feature = "egui-test")]
    fn observe(&self, text_edit_focused: bool) -> Self::Observation {
        self.witness_state(text_edit_focused)
    }
}
