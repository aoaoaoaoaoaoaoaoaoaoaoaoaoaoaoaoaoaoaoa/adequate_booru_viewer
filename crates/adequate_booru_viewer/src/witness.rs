#[cfg(feature = "egui-test")]
use std::fmt::Display;

#[cfg(feature = "egui-test")]
#[inline]
pub fn anchor(ui: &egui::Ui, name: impl Display, rect: egui::Rect) {
    egui_tester_witness::egui::record(ui, name.to_string(), rect);
}

#[cfg(feature = "egui-test")]
pub use active::State;

#[cfg(feature = "egui-test")]
mod active {
    use serde::Serialize;

    #[derive(Serialize)]
    pub struct State {
        pub contract: &'static str,
        pub water: &'static str,
        pub filter: String,
        pub result_posts: usize,
        pub text_edit_focused: bool,
        pub ui_open: bool,
    }
}
