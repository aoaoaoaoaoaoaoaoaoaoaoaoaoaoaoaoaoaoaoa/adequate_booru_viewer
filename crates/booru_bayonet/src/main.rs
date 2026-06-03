mod app;
mod booru;
mod clip;
mod config;
mod index;
mod media;
mod model;
mod worker;
mod xdg;

use std::error::Error;

use app::Bayonet;
use eframe::egui::ViewportBuilder;

type DynError = Box<dyn Error + Send + Sync>;

fn main() -> eframe::Result {
    let native = eframe::NativeOptions {
        viewport: ViewportBuilder::default().with_inner_size([1440.0, 920.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Booru Bayonet",
        native,
        Box::new(|cc| match Bayonet::new(cc) {
            Ok(app) => Ok(Box::new(app)),
            Err(err) => Err(DynError::from(err)),
        }),
    )
}
