#![expect(
    unused_crate_dependencies,
    reason = "the GUI binary is module-owned; the sibling retrieval library exists for benchmark tooling"
)]

#[macro_export]
macro_rules! witness_anchor {
    ($ui:expr, $name:expr, $rect:expr) => {{
        #[cfg(feature = "egui-test")]
        {
            let rect = $rect;
            if rect.is_positive()
                && let Some(name) = $name.wire()
            {
                $crate::witness::anchor($ui, name, rect);
            }
        }
    }};
}

mod app;
mod booru;
mod commands;
mod configuration;
mod controls;
mod favorites;
mod filter_bank;
mod host;
mod media;
mod query_ui;
mod saved_filter_ui;
mod tag_chroma;
mod tag_menu;
mod tag_palette;
mod witness;
mod worker;

pub(crate) use adequate_booru_viewer::{
    application_paths, date, filter_expression, index, kin, model, trace,
};

pub(crate) use brass_poolrooms::{chrome, water};

use anyhow::{Context as _, Result, bail};
use app::Launch;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let mut paused = false;
    let mut sealed = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--version" | "-V") => {
                println!("abv {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("--pause-mirror") => paused = true,
            Some("--sealed-index") => {
                sealed = Some(PathBuf::from(
                    arguments
                        .next()
                        .context("--sealed-index requires an index path")?,
                ));
            }
            _ => bail!("unknown argument `{}`", argument.display()),
        }
    }
    let launch = match (sealed, paused) {
        (Some(path), _) => Launch::Sealed(path),
        (None, true) => Launch::Paused,
        (None, false) => Launch::Configured,
    };
    trace::startup("main.enter");
    let ctx = egui::Context::default();
    chrome::install(&ctx);
    let trace = eternalist_apps::TraceGuard::arm()?;
    let result = host::run(eternalist_apps::Ingress::Desktop, ctx, launch);
    trace.flush();
    result
}
