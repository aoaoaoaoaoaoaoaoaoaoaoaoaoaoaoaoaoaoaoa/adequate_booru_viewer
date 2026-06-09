mod app;
mod booru;
mod clip;
mod config;
mod filter_bank;
mod index;
mod media;
mod model;
mod posting;
mod query_ui;
mod tag_menu;
mod trace;
mod wire;
mod worker;
mod xdg;

use std::error::Error;

use app::Bayonet;
use eframe::egui::ViewportBuilder;
use trace::startup;

type DynError = Box<dyn Error + Send + Sync>;

fn main() -> eframe::Result {
    startup("main.enter");
    if std::env::var_os("BOORU_BAYONET_STARTUP_PROBE_HEADLESS").is_some() {
        let mut app =
            Bayonet::open().map_err(|err| eframe::Error::AppCreation(DynError::from(err)))?;
        app.draw_startup_probe_frame();
        startup("main.headless.exit");
        std::process::exit(0);
    }

    startup("main.native_options");
    let native = eframe::NativeOptions {
        viewport: ViewportBuilder::default().with_inner_size([1440.0, 920.0]),
        ..Default::default()
    };

    startup("main.run_native");
    eframe::run_native(
        "Booru Bayonet",
        native,
        Box::new(|cc| match Bayonet::new(cc) {
            Ok(app) => Ok(Box::new(app)),
            Err(err) => Err(DynError::from(err)),
        }),
    )
}
