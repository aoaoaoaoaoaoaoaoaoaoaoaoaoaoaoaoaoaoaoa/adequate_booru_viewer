use std::fmt::Display;

#[inline]
#[cfg(feature = "egui-test")]
pub fn anchor(ui: &egui::Ui, name: impl Display, rect: egui::Rect) {
    egui_tester_witness::egui::record(ui, name.to_string(), rect);
}

#[inline]
pub fn response(ui: &egui::Ui, name: impl Display, response: &egui::Response) {
    #[cfg(feature = "egui-test")]
    egui_tester_witness::egui::record_response(ui, name.to_string(), response);
    #[cfg(not(feature = "egui-test"))]
    {
        let _ = (ui, response);
        drop(name);
    }
}

#[inline]
pub fn rect(ctx: &egui::Context, name: impl Display, rect: egui::Rect) {
    #[cfg(feature = "egui-test")]
    egui_tester_witness::egui::record_rect(ctx, name.to_string(), rect);
    #[cfg(not(feature = "egui-test"))]
    {
        let _ = (ctx, rect);
        drop(name);
    }
}

#[cfg(feature = "egui-test")]
pub use active::{Settings, State};

#[cfg(feature = "egui-test")]
mod active {
    use serde::Serialize;

    #[derive(Serialize)]
    #[expect(
        clippy::struct_excessive_bools,
        reason = "the wire observation carries independent UI facts, not a latent state machine"
    )]
    pub struct State {
        pub contract: &'static str,
        pub water: &'static str,
        pub filter: String,
        pub result_posts: usize,
        pub text_edit_focused: bool,
        pub ui_open: bool,
        pub query_open: bool,
        pub active_group: Vec<usize>,
        pub images_per_row: u16,
        pub guide_open: bool,
        pub settings: Settings,
        pub prefetch_on_hover: bool,
        pub mirror_active: bool,
        pub viewer_tags_open: bool,
    }

    #[derive(Default, Serialize)]
    pub struct Settings {
        pub open: bool,
        pub fault: Option<String>,
        pub settled: bool,
    }
}
