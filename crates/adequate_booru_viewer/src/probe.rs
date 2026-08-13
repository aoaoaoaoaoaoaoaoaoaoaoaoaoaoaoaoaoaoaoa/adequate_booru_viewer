//! Debug-only widget anchor probe (Cargo feature `devtools`). Each egui pass,
//! interactive widgets register a `name -> rect` anchor; once the frame settles
//! the registry plus a little live app state is dumped — in physical pixels,
//! stamped with the egui frame serial — to the file named by `ABV_ANCHOR_PROBE`.
//! The demo xtask reads it to resolve named click targets and run closed-loop
//! navigation instead of baking coordinates that rot when the UI moves.
//!
//! The whole module compiles only under `devtools`; the `probe_anchor!` /
//! `probe_reset!` macros (defined in `main.rs`) expand to nothing otherwise, so
//! shipped builds carry zero footprint.

use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use egui::{Context, Id, Rect, Ui};
use serde::Serialize;

use crate::{config::WaterMode, date::DateRange, model::Sort};

const STORE: &str = "probe-anchors";
static ON: AtomicBool = AtomicBool::new(false);
static PATH: OnceLock<PathBuf> = OnceLock::new();

/// The per-pass anchor accumulator, stashed in egui temp-data exactly like
/// `chrome`'s tension field.
#[derive(Default, Clone)]
struct Anchors(Vec<(String, Rect)>);

/// One-time arm from the environment, called in `Bayonet::open`.
pub fn arm() {
    if let Some(path) = std::env::var_os("ABV_ANCHOR_PROBE") {
        let _set = PATH.set(PathBuf::from(path));
        ON.store(true, Ordering::Relaxed);
    }
}

#[inline]
pub fn probing() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Drop the prior pass's anchors at the top of each pass, so a discarded/re-run
/// pass cannot leak stale rects; the frame's final pass wins.
pub fn reset(ctx: &Context) {
    ctx.data_mut(|data| {
        let _dropped = data.remove_temp::<Anchors>(Id::new(STORE));
    });
}

/// Record one interactive widget's hit-test rect under a semantic name.
pub fn record(ui: &Ui, name: impl Into<String>, rect: Rect) {
    ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<Anchors>(Id::new(STORE))
            .0
            .push((name.into(), rect));
    });
}

/// Live app state the choreography reads for closed-loop steps. Visible tiles
/// are not duplicated here — the `tile:<id>` anchors already enumerate them.
#[derive(Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "debug telemetry reports orthogonal live predicates, not one state machine"
)]
pub struct State {
    pub active_group: Vec<usize>,
    pub text_edit_focused: bool,
    pub water: WaterMode,
    pub sort: Sort,
    pub dates: DateRange,
    pub images_per_row: u16,
    pub result_posts: usize,
    pub result_candidates: u64,
    pub result_horizon: usize,
    pub result_tail_open: bool,
    pub requested_horizon: usize,
    pub horizon_pending: bool,
    pub grid_rows: usize,
    pub grid_visible_end: usize,
    pub grid_scroll_offset: f32,
    pub refresh_in_flight: bool,
    pub status: String,
    pub warm_status: String,
    pub zoom_post: Option<u32>,
    pub tag_menu_post: Option<u32>,
}

#[derive(Serialize)]
struct Anchor {
    name: String,
    /// `[min.x, min.y, max.x, max.y]` in physical pixels — the space
    /// `xdotool mousemove --window` clicks in.
    rect: [f32; 4],
}

#[derive(Serialize)]
struct Frame {
    frame: u64,
    ppp: f32,
    anchors: Vec<Anchor>,
    state: State,
}

/// Dump the accumulated anchors + state. `ppp` must be the frame's
/// `FullOutput::pixels_per_point`, not an incidental later read.
pub fn dump(ctx: &Context, ppp: f32, state: State) {
    let Some(path) = PATH.get() else {
        return;
    };
    let anchors = ctx
        .data_mut(|data| data.get_temp::<Anchors>(Id::new(STORE)))
        .unwrap_or_default();
    let poolroom_anchors = brass_poolrooms::instrumentation::take(ctx);
    let frame = Frame {
        frame: ctx.cumulative_frame_nr(),
        ppp,
        anchors: anchors
            .0
            .iter()
            .map(|(name, rect)| (name.clone(), *rect))
            .chain(
                poolroom_anchors
                    .into_iter()
                    .map(|anchor| (anchor.name, anchor.rect)),
            )
            .map(|(name, rect)| Anchor {
                name,
                rect: [
                    rect.min.x * ppp,
                    rect.min.y * ppp,
                    rect.max.x * ppp,
                    rect.max.y * ppp,
                ],
            })
            .collect(),
        state,
    };
    if let Ok(bytes) = serde_json::to_vec(&frame) {
        write_atomic(path, &bytes);
    }
}

/// Write-then-rename so the reader never sees a torn file.
fn write_atomic(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() {
        let _rename = std::fs::rename(&tmp, path);
    }
}
