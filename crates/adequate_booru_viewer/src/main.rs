#![expect(
    unused_crate_dependencies,
    reason = "the GUI binary is module-owned; the sibling retrieval library exists for benchmark tooling"
)]

#[macro_export]
macro_rules! probe_anchor {
    ($ui:expr, $name:expr, $rect:expr) => {{
        #[cfg(any(feature = "devtools", feature = "egui-test"))]
        {
            let rect = $rect;
            if rect.is_positive() {
                let name = $name;
                #[cfg(feature = "egui-test")]
                $crate::witness::anchor($ui, &name, rect);
                #[cfg(feature = "devtools")]
                if $crate::probe::probing() {
                    $crate::probe::record($ui, name.to_string(), rect);
                }
            }
        }
    }};
}

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
mod booru;
mod commands;
mod config;
mod controls;
mod date;
mod favorites;
mod filter_bank;
mod host;
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
mod witness;
mod worker;
mod xdg;

pub(crate) use dwemer_poolrooms::{chrome, water};

use anyhow::Result;

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--version" || argument == "-V")
    {
        println!("abv {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let pause_mirror = arguments
        .iter()
        .any(|argument| argument == "--pause-mirror");
    trace::startup("main.enter");
    let ctx = egui::Context::default();
    chrome::install(&ctx);
    let trace = eternalist_apps::TraceGuard::arm()?;
    let result = host::run(ctx, pause_mirror);
    trace.flush();
    result
}
