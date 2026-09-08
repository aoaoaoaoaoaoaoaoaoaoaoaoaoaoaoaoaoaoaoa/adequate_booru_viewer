#[cfg(feature = "egui-test")]
pub use eternalist_apps::witness::anchor;
pub use eternalist_apps::witness::response;

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
        pub ui_panel_open: bool,
        pub query_open: bool,
        pub active_group: Vec<usize>,
        pub images_per_row: u16,
        pub guide_open: bool,
        pub settings: Settings,
        pub prefetch_on_hover: bool,
        pub mirror_active: bool,
        pub viewer_tags_open: bool,
        pub tag_push_ready: bool,
        pub viewer_post: Option<u32>,
    }

    #[derive(Default, Serialize)]
    pub struct Settings {
        pub open: bool,
        pub fault: Option<String>,
        pub settled: bool,
    }
}
