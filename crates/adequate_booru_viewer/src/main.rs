#![expect(
    unused_crate_dependencies,
    reason = "the GUI binary is module-owned; the sibling retrieval library exists for benchmark tooling"
)]

/// Register a widget's hit-test rect under a semantic name for the `devtools`
/// anchor probe. Expands to nothing without the feature; even the name
/// expression is left unevaluated, so the call site is free in shipped builds.
#[macro_export]
macro_rules! probe_anchor {
    ($ui:expr, $name:expr, $rect:expr) => {{
        #[cfg(feature = "devtools")]
        if $crate::probe::probing() {
            $crate::probe::record($ui, $name, $rect);
        }
    }};
}

/// Clear the anchor accumulator at the start of an egui pass (the final pass of
/// the frame wins). No-op without `devtools`.
#[macro_export]
macro_rules! probe_reset {
    ($ctx:expr) => {{
        #[cfg(feature = "devtools")]
        if $crate::probe::probing() {
            $crate::probe::reset($ctx);
        }
    }};
}

#[cfg(feature = "devtools")]
mod probe;

mod app;
mod boiler;
mod booru;
mod config;
mod date;
mod filter_bank;
mod index;
mod kin;
mod media;
mod model;
mod posting;
mod query_ui;
mod saved_filter_ui;
mod tag_chroma;
mod tag_menu;
mod tag_palette;
mod trace;
mod wire;
mod worker;
mod xdg;

pub(crate) use dwemer_poolrooms::{chrome, water};

use anyhow::Result;

use app::Bayonet;
use trace::startup;

fn main() -> Result<()> {
    startup("main.enter");
    let ctx = egui::Context::default();
    chrome::install(&ctx);
    startup("main.chrome.installed");
    let mut app = Bayonet::open(&ctx)?;
    startup("main.app.opened");

    if std::env::var_os("ADEQUATE_BOORU_VIEWER_STARTUP_PROBE_HEADLESS").is_some() {
        app.draw_startup_probe_frame(&ctx);
        startup("main.headless.exit");
        std::process::exit(0);
    }

    boiler::run(ctx, app)
}
