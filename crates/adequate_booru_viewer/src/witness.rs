#[cfg(feature = "egui-test")]
pub use eternalist_apps::witness::anchor;
pub use eternalist_apps::witness::response;

#[cfg(feature = "egui-test")]
pub use active::{Search, Settings, State, Tiles, Viewer};

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
        pub viewer: Option<Viewer>,
        pub search: Search,
        pub tiles: Tiles,
    }

    /// The open viewer's post, and whether its full image stands at rest:
    /// drawn opaque, with no fade, recoil, or drag in flight.
    #[derive(Clone, Copy, Serialize)]
    pub struct Viewer {
        pub post: u32,
        pub painted: bool,
    }

    /// The serial of the newest search the gallery requested, and of the
    /// newest whose answer it installed: equal, the results answer the query.
    #[derive(Clone, Copy, Serialize)]
    pub struct Search {
        pub requested: u64,
        pub answered: u64,
    }

    /// Gallery tiles the submitted pass laid out, and those drawing a ready
    /// thumbnail.
    #[derive(Clone, Copy, Default, Serialize)]
    pub struct Tiles {
        pub visible: u32,
        pub painted: u32,
    }

    #[derive(Default, Serialize)]
    pub struct Settings {
        pub open: bool,
        pub fault: Option<String>,
        pub settled: bool,
    }
}
