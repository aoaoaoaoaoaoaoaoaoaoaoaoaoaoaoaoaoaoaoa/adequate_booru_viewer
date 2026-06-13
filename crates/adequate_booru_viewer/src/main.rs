#![expect(
    unused_crate_dependencies,
    reason = "the GUI binary is module-owned; the sibling retrieval library exists for benchmark tooling"
)]

mod app;
mod boiler;
mod booru;
mod chrome;
mod config;
mod date;
mod filter_bank;
mod frost;
mod index;
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
