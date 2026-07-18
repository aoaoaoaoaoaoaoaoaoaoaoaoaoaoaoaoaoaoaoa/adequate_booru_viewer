use anyhow::{Context as _, Result};
use arboard::{Clipboard, ImageData};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    chrome,
    config::{Config, FilterConfig, FilterName, QueryConfig, SavedFilter, Slate, WaterMode},
    date::{CreatedDay, DateRange},
    filter_bank::Bank,
    frost::{Cut, Veil},
    index::{CacheStats, Index, TagSuggestion},
    media::{MediaCache, RgbaBlade, extension},
    model::{
        BoolOp, GroupCycle, PostId, PostRecord, Query, QueryAtom, SearchHit, Sort, Tag, TagKind,
        TagPolarity,
    },
    query_ui::{QueryAction, render_query_tree},
    saved_filter_ui::{self, Action as SavedFilterAction, NameEdit, ShelfEdit},
    tag_chroma,
    tag_menu::{
        HEIGHT as TAG_MENU_HEIGHT, TagGroups, TagMenu, WIDTH as TAG_MENU_WIDTH,
        position as tag_menu_pos,
    },
    tag_palette,
    trace::startup,
    worker::{BladeEpoch, Command, Event, Worker},
    xdg::Lair,
};

mod bench;
mod date_spool;
mod debug;
mod loading;
mod palette;
mod panels;
mod refresh;
mod scroll;
mod viewer;
mod water;

use refresh::{AsyncPulse, PulseGate};
use scroll::ThumbCruise;
use viewer::{FullWait, ZoomGate};

const RESULT_LIMIT: usize = 360;
const HIT_CACHE_LIMIT: usize = 24;
const EVENT_BUDGET: usize = 12;
const AUTO_WARM_PAGES: u32 = 1;
const DANBOORU_SEARCH_PAGE_LIMIT: u32 = 1_000;
const MIN_IMAGES_PER_ROW: u16 = 1;
const MAX_IMAGES_PER_ROW: u16 = 12;
const MIN_TILE_EDGE: f32 = 72.0;
const GAP: f32 = 12.0;
const VIEWER_CHROME: f32 = 40.0;
const MAX_GROUP_DEPTH: usize = 8;
const PLATE_PAD: f32 = 4.0;
const PREFETCH_DWELL: Duration = Duration::from_millis(120);
const THUMB_PREFETCH_BUDGET: usize = 48;
const CONFIG_SETTLE: Duration = Duration::from_millis(400);
const VEIL_RADIUS: f32 = 2.0;
const VEIL_RISE: f32 = 0.12;
const VEIL_FALL: f32 = 0.06;
const ZOOM_DIM: f32 = 0.78;
const MENU_DIM: f32 = 0.62;

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent app-state flags (UI toggles + a one-shot pending), not a state machine"
)]
pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query: Query,
    active_group: Vec<usize>,
    tag_entry: String,
    filter_name_entry: String,
    name_edit: NameEdit,
    active_filter: Option<FilterName>,
    filters: Bank,
    shelf_edit: Option<ShelfEdit>,
    sort: Sort,
    date_range: DateRange,
    refresh_serial: u64,
    refresh_pulse: AsyncPulse,
    refresh_gate: PulseGate,
    stats_serial: u64,
    stats_pulse: AsyncPulse,
    stats_gate: PulseGate,
    hit: SearchHit,
    hit_cache: HitCache,
    parked_hit: Option<SearchHit>,
    /// A user filter/sort change is in flight; the next `commit_hit` thwacks the
    /// pool with energy set by how many tiles the new result actually replaces.
    thwack_pending: bool,
    thumbs: HashMap<ThumbKey, TextureHandle>,
    thumb_inflight: HashSet<ThumbKey>,
    thumb_faults: HashSet<ThumbKey>,
    thumb_epoch: BladeEpoch,
    warm_key: WarmKey,
    warm_next_page: u32,
    warm_stride: u32,
    warm_state: WarmState,
    full: HashMap<PostId, TextureHandle>,
    full_rgba: HashMap<PostId, RgbaBlade>,
    full_loaded_at: HashMap<PostId, Instant>,
    full_wait: HashMap<PostId, FullWait>,
    full_inflight: HashSet<PostId>,
    full_faults: HashSet<PostId>,
    zoom: Option<PostRecord>,
    zoom_gate: ZoomGate,
    zoom_rect: Option<egui::Rect>,
    viewer_tags_open: bool,
    viewer_tag_groups: Option<(PostId, TagGroups)>,
    images_per_row: u16,
    /// Left-rail recess fold state, the running truth written back to the
    /// slate; keyed by section id, `true` ⇒ open. Seeded from the slate and
    /// updated whenever a recess is thrown.
    shutters: BTreeMap<String, bool>,
    tag_menu: TagMenu,
    tag_menu_rect: Option<egui::Rect>,
    menu_cuts: Option<(egui::Rect, egui::Rect)>,
    water: crate::frost::WaterTable,
    water_mode: WaterMode,
    thumb_cruise: ThumbCruise,
    bench_open: bool,
    tag_kinds: HashMap<Tag, TagKind>,
    suggest_memo: Option<(String, Vec<TagSuggestion>)>,
    suggest_pick: usize,
    suggest_serial: u64,
    refetch_inflight: HashSet<PostId>,
    prefetch_on_hover: bool,
    prefetched: HashSet<PostId>,
    hover_arm: Option<(PostId, Instant)>,
    empty_since: Option<Instant>,
    config_dirty: Option<Instant>,
    cache_status: String,
    warm_status: String,
    crawl_status: String,
    status: String,
    startup_probe: Option<StartupProbe>,
}

impl Bayonet {
    pub fn open(ctx: &egui::Context) -> Result<Self> {
        startup("app.open.enter");
        let lair = Lair::claim()?;
        startup("app.lair.claimed");
        // First-run-ever is the absence of the config file (not an empty
        // library): the seed below is written on first launch, so the file then
        // persists and deleting the seed never brings it back.
        let first_run = !lair.config_path().exists();
        let config = Config::load(&lair.config_path())?;
        startup("app.config.loaded");
        let index = Index::open(&lair.index_path())?;
        startup("app.index.opened");
        let media = MediaCache::new(lair.media_dir())?;
        startup("app.media.opened");
        let worker = Worker::spawn(index.clone(), media, ctx.clone());
        startup("app.worker.spawned");
        let mut filters = Bank::forge(config.filters.saved.clone(), config.filters.shelves.clone());
        let slate = Slate::load(&lair.slate_path());
        for shelf in &mut filters.shelves {
            shelf.open = !slate.closed_folders.contains(&shelf.name);
        }
        let active_filter = filters.active(slate.active_filter.clone().or_else(|| {
            first_run
                .then(|| FilterName::forge(crate::config::SAFE_DEFAULT_FILTER))
                .flatten()
        }));
        let mut query = active_filter
            .as_ref()
            .and_then(|active| filters.get(active))
            .map_or_else(|| slate.query.tree.clone(), |filter| filter.tree.clone());
        query.sort_atoms();
        let sort = slate.sort;
        let date_range = clean_dates(slate.dates);
        let scrubbed_dates = date_range != slate.dates.normalized();
        let active_group = active_filter
            .as_ref()
            .and_then(|active| filters.get(active))
            .map_or_else(
                || query.clamp_group_path(&slate.query.active_group),
                |filter| query.clamp_group_path(&filter.active_group),
            );
        let mut app = Self {
            status: format!("index {}", lair.index_path().display()),
            crawl_status: "crawl waking".to_owned(),
            lair,
            index,
            worker,
            query: query.clone(),
            active_group,
            tag_entry: String::new(),
            filter_name_entry: String::new(),
            name_edit: NameEdit::Idle,
            active_filter,
            filters,
            shelf_edit: None,
            sort,
            date_range,
            refresh_serial: 0,
            refresh_pulse: AsyncPulse::Idle,
            refresh_gate: PulseGate::refresh(),
            stats_serial: 0,
            stats_pulse: AsyncPulse::Idle,
            stats_gate: PulseGate::stats(),
            hit: SearchHit::default(),
            hit_cache: HitCache::default(),
            parked_hit: None,
            thwack_pending: false,
            thumbs: HashMap::new(),
            thumb_inflight: HashSet::new(),
            thumb_faults: HashSet::new(),
            thumb_epoch: BladeEpoch::ROOT,
            warm_key: WarmKey::new(&query, sort),
            warm_next_page: 1,
            warm_stride: AUTO_WARM_PAGES,
            warm_state: WarmState::Idle,
            full: HashMap::new(),
            full_rgba: HashMap::new(),
            full_loaded_at: HashMap::new(),
            full_wait: HashMap::new(),
            full_inflight: HashSet::new(),
            full_faults: HashSet::new(),
            zoom: None,
            zoom_gate: ZoomGate::Fresh,
            zoom_rect: None,
            viewer_tags_open: slate.viewer_tags_open,
            viewer_tag_groups: None,
            images_per_row: slate
                .images_per_row
                .clamp(MIN_IMAGES_PER_ROW, MAX_IMAGES_PER_ROW),
            shutters: slate.shutters,
            tag_menu: TagMenu::Closed,
            tag_menu_rect: None,
            menu_cuts: None,
            water: crate::frost::WaterTable::new(match slate.water {
                WaterMode::Dry => crate::frost::Wetness::Dry,
                WaterMode::Wet => crate::frost::Wetness::Wet,
                WaterMode::ReallyWet => crate::frost::Wetness::Deluge,
            }),
            water_mode: slate.water,
            thumb_cruise: ThumbCruise::default(),
            bench_open: false,
            tag_kinds: HashMap::new(),
            suggest_memo: None,
            suggest_pick: 0,
            suggest_serial: 0,
            refetch_inflight: HashSet::new(),
            prefetch_on_hover: config.prefetch_on_hover,
            prefetched: HashSet::new(),
            hover_arm: None,
            empty_since: None,
            config_dirty: None,
            cache_status: "cache measuring".to_owned(),
            warm_status: "query warm idle".to_owned(),
            startup_probe: StartupProbe::from_env(),
        };
        startup("app.state.built");
        #[cfg(feature = "devtools")]
        crate::probe::arm();
        // Persist the first-run seed synchronously so the config file exists
        // from now on — that file's presence is the first-run-ever marker.
        if first_run {
            app.write_config();
        }
        if scrubbed_dates {
            app.save_config();
        }
        app.strike(true, AUTO_WARM_PAGES);
        startup("app.initial.reap.done");
        Ok(app)
    }

    /// Snapshot the frame's anchors + live state for the `devtools` probe. Built
    /// only with the feature and skipped entirely unless `ABV_ANCHOR_PROBE` armed
    /// it, so no state is cloned in the common case.
    #[cfg(feature = "devtools")]
    pub fn probe_dump(&self, ctx: &egui::Context, pixels_per_point: f32) {
        if !crate::probe::probing() {
            return;
        }
        crate::probe::dump(
            ctx,
            pixels_per_point,
            crate::probe::State {
                active_group: self.active_group.clone(),
                text_edit_focused: ctx.text_edit_focused(),
                water: self.water_mode,
                sort: self.sort,
                dates: self.date_range,
                images_per_row: self.images_per_row,
                zoom_post: self.zoom.as_ref().map(|post| post.id.0),
                tag_menu_post: self.tag_menu.post_id().map(|id| id.0),
            },
        );
    }

    pub fn draw_startup_probe_frame(&mut self, ctx: &egui::Context) {
        startup("app.draw.enter");
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 920.0),
                )),
                ..Default::default()
            },
            |ui| self.pulse(ui),
        );
        startup("app.draw.ui.done");
        let _primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        startup("app.draw.tessellated");
        startup("app.draw.probe.reported");
    }

    /// One full application frame: drain workers, settle gates, paint.
    pub fn pulse(&mut self, ui: &mut egui::Ui) {
        crate::probe_reset!(ui.ctx());
        let ctx = ui.ctx().clone();
        self.zoom_tiles(&ctx);
        self.drain(&ctx);
        self.flush_pulse_gates(&ctx);
        self.flush_config(&ctx);
        self.cycle_query_group(&ctx);
        self.paint(ui);
        self.bench(&ctx);
        self.report_startup_probe();
    }

    fn cycle_query_group(&mut self, ctx: &egui::Context) {
        if self.zoom.is_some()
            || self.tag_menu.is_open()
            || ctx.text_edit_focused()
            || self.tag_entry_arms_completion()
        {
            return;
        }
        let cycle = ctx.input_mut(take_tab_cycle);
        let Some(cycle) = cycle else {
            return;
        };
        if let Some(focus) = ctx.memory(|mem| mem.focused()) {
            ctx.memory_mut(|mem| mem.surrender_focus(focus));
        }
        let active = self.query.cycle_group_path(&self.active_group, cycle);
        if self.active_group != active {
            self.active_group = active;
            self.sync_active_filter();
            self.save_config();
            ctx.request_repaint();
        }
    }

    fn tag_entry_arms_completion(&self) -> bool {
        active_prefix(&self.tag_entry).is_some()
    }

    /// Frost parameters for the boiler's blur pass, in logical points. The
    /// water table performs the sole logical-to-physical conversion.
    /// `None` while no veil is showing (the common case — zero GPU cost).
    ///
    /// While a veil is fading out its cutouts are dropped, so the blur turns
    /// uniform and recedes evenly instead of leaving sharp negative space.
    pub fn frost_veil(&self, ctx: &egui::Context) -> Option<Veil> {
        let cut = |rect: egui::Rect, radius: f32| Cut { rect, radius };
        let zoom_open = self.zoom.is_some();
        let zoom_strength = veil_strength(ctx, "frost-zoom", zoom_open);
        if zoom_strength > 0.0 {
            let cuts = if zoom_open && let Some(rect) = self.zoom_rect {
                [cut(rect, VEIL_RADIUS), Cut::NONE]
            } else {
                [Cut::NONE, Cut::NONE]
            };
            return Some(Veil {
                cuts,
                strength: zoom_strength,
                dim: ZOOM_DIM,
                blur: 1.0,
            });
        }
        let menu_open = self.tag_menu.is_open();
        let menu_strength = veil_strength(ctx, "frost-menu", menu_open);
        if menu_strength > 0.0 {
            let cuts = if menu_open && let Some((tile, menu)) = self.menu_cuts {
                [cut(tile, 0.0), cut(menu, VEIL_RADIUS)]
            } else {
                [Cut::NONE, Cut::NONE]
            };
            // Pure dim: blur-glow from neighboring tiles fights the isolation.
            return Some(Veil {
                cuts,
                strength: menu_strength,
                dim: MENU_DIM,
                blur: 0.0,
            });
        }
        None
    }

    fn install_query(&mut self, query: Query) {
        self.install_query_at(query, self.active_group.clone());
    }

    fn install_dates(&mut self, dates: DateRange) {
        let dates = clean_dates(dates);
        if self.date_range == dates {
            return;
        }
        self.remember_hit();
        self.date_range = dates;
        // A `strike` is about to replace the tiles, so never blank to a loading
        // card in the meantime: restore a cached hit if we have one, else leave
        // the current tiles up and let the async result swap in. `commit_hit`
        // retains unchanged thumbnails by id, so the grid updates without a flash,
        // and thwacks the pool by how much actually changed (set before the
        // restore, so a cache-hit commit reads it too).
        self.thwack_pending = true;
        let _ = self.restore_hit();
        self.save_config();
        if dates.active() {
            self.warm_state = WarmState::Idle;
            "date range active; upstream warm suspended".clone_into(&mut self.warm_status);
        }
        self.strike(false, 0);
    }

    fn install_query_at(&mut self, query: Query, active_group: Vec<usize>) {
        let mut query = query;
        query.sort_atoms();
        self.remember_hit();
        self.active_group = query.clamp_group_path(&active_group);
        self.query = query;
        // Keep the current tiles up until the async `strike` swaps in the new
        // ones (or restore a cached hit) — no loading-card flash on a re-query;
        // `commit_hit` thwacks by how much actually changed.
        self.thwack_pending = true;
        let _ = self.restore_hit();
        let query = self.query.clone();
        self.align_warm(&query);
        self.save_config();
        self.strike(true, AUTO_WARM_PAGES);
    }

    fn remember_hit(&mut self) {
        self.hit_cache.put(
            HitKey::new(&self.query, self.sort, self.date_range),
            self.hit.clone(),
        );
    }

    fn restore_hit(&mut self) -> bool {
        // The key just changed, so any result deferred behind an open tag menu
        // was for the old key — drop it lest it commit stale when the menu closes.
        self.parked_hit = None;
        let key = HitKey::new(&self.query, self.sort, self.date_range);
        let Some(hit) = self.hit_cache.get(&key) else {
            return false;
        };
        self.commit_hit(hit);
        true
    }

    fn install_hit(&mut self, hit: SearchHit) {
        if self.tag_menu.is_open() {
            self.parked_hit = Some(hit);
            return;
        }
        self.commit_hit(hit);
    }

    fn commit_hit(&mut self, hit: SearchHit) {
        // A user filter/sort change thwacks the pool by how much it actually
        // replaced — zero for the appends/tail-trims a date scroll makes, full
        // for a reorder or re-query. Read before `self.hit` is overwritten.
        if std::mem::take(&mut self.thwack_pending) {
            let energy = swap_fraction(&self.hit.posts, &hit.posts);
            if energy > 0.0 {
                self.pool_thwack(self.water.domain(), energy);
            }
        }
        if posts_changed(&self.hit.posts, &hit.posts) {
            self.advance_thumb_epoch();
        }
        self.hit = hit;
        let live = self
            .hit
            .posts
            .iter()
            .map(|post| post.id)
            .collect::<HashSet<_>>();
        self.thumbs.retain(|key, _| live.contains(&key.id));
        self.thumb_faults.retain(|key| live.contains(&key.id));
        self.prefetched.retain(|id| live.contains(id));
    }

    fn close_tag_menu(&mut self) {
        self.tag_menu = TagMenu::Closed;
        self.tag_menu_rect = None;
        if let Some(hit) = self.parked_hit.take() {
            self.commit_hit(hit);
        }
    }

    fn advance_thumb_epoch(&mut self) {
        self.thumb_epoch = self.thumb_epoch.advance();
        self.thumb_inflight.clear();
        self.thumb_cruise = ThumbCruise::default();
        if let Err(err) = self.worker.send(Command::CullBlades {
            epoch: self.thumb_epoch,
        }) {
            self.status = format!("{err:#}");
        }
    }

    fn add_atom(&mut self, atom: QueryAtom, polarity: TagPolarity) {
        let mut query = self.query.clone();
        if query.push_atom(&self.active_group, atom, polarity) {
            self.install_query(query);
        }
    }

    fn remove_atom(&mut self, atom: &QueryAtom) {
        let mut query = self.query.clone();
        query.remove_atom(atom);
        self.install_query(query);
    }

    fn tag_kind(&mut self, tag: &Tag) -> TagKind {
        if let Some(kind) = self.tag_kinds.get(tag) {
            return *kind;
        }
        let kind = match self.index.tag_kind(tag) {
            Ok(kind) => kind,
            Err(err) => {
                self.status = format!("{err:#}");
                TagKind::General
            }
        };
        let _old = self.tag_kinds.insert(tag.clone(), kind);
        kind
    }

    fn atom_kind(&mut self, atom: &QueryAtom) -> TagKind {
        match atom {
            QueryAtom::Tag(tag) => self.tag_kind(tag),
            QueryAtom::Rating(_) => TagKind::Meta,
        }
    }

    fn save_current_filter(&mut self) {
        let typed = FilterName::forge(&self.filter_name_entry);
        let name = typed.unwrap_or_else(|| {
            self.active_filter
                .clone()
                .unwrap_or_else(|| self.filters.spare(&self.query))
        });
        self.upsert_filter(name.clone(), self.query.clone(), self.active_group.clone());
        self.active_filter = Some(name.clone());
        self.filter_name_entry.clear();
        self.name_edit = NameEdit::Idle;
        self.status = format!("saved filter `{name}`");
        self.save_config();
    }

    fn load_filter(&mut self, filter: SavedFilter) {
        self.active_filter = Some(filter.name.clone());
        self.filter_name_entry.clear();
        self.name_edit = NameEdit::Idle;
        self.status = format!("active filter `{}`", filter.name);
        self.install_query_at(filter.tree, filter.active_group);
    }

    fn new_filter(&mut self) {
        self.active_filter = None;
        self.filter_name_entry.clear();
        self.name_edit = NameEdit::Idle;
        "new unsaved filter".clone_into(&mut self.status);
        self.install_query_at(Query::default(), Vec::new());
    }

    fn rename_filter(&mut self) {
        let Some(old) = self.active_filter.clone() else {
            "no active filter to rename".clone_into(&mut self.status);
            return;
        };
        let Some(new) = FilterName::forge(&self.filter_name_entry) else {
            "rename needs a nonempty filter name".clone_into(&mut self.status);
            return;
        };
        if old == new {
            self.filter_name_entry.clear();
            self.name_edit = NameEdit::Idle;
            return;
        }
        if self.filters.taken(&new) {
            self.status = format!("filter `{new}` already exists");
            return;
        }
        self.filters.rename(&old, new.clone());
        self.upsert_filter(new.clone(), self.query.clone(), self.active_group.clone());
        self.active_filter = Some(new.clone());
        self.filter_name_entry.clear();
        self.name_edit = NameEdit::Idle;
        self.status = format!("renamed filter `{old}` → `{new}`");
        self.save_config();
    }

    fn begin_name_edit(&mut self) {
        self.filter_name_entry = self
            .active_filter
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        self.name_edit = NameEdit::Arming;
    }

    fn clone_filter(&mut self, name: &FilterName) {
        let Some(filter) = self.filters.get(name).cloned() else {
            return;
        };
        let source = filter.name.clone();
        let name = self.filters.spare_named(&source);
        self.filters.adopt_beside(
            &source,
            SavedFilter::new(
                name.clone(),
                filter.tree.clone(),
                filter.active_group.clone(),
            ),
        );
        self.active_filter = Some(name.clone());
        self.filter_name_entry.clear();
        self.status = format!("cloned filter `{name}`");
        self.install_query_at(filter.tree, filter.active_group);
    }

    fn delete_filter(&mut self, name: &FilterName) {
        let Some(removed) = self.filters.remove(name) else {
            return;
        };
        if self.active_filter.as_ref() == Some(&removed.name) {
            self.active_filter = None;
        }
        self.status = format!("deleted filter `{}`", removed.name);
        self.save_config();
    }

    fn sync_active_filter(&mut self) {
        let Some(name) = self.active_filter.clone() else {
            return;
        };
        self.upsert_filter(name, self.query.clone(), self.active_group.clone());
    }

    fn upsert_filter(&mut self, name: FilterName, tree: Query, active_group: Vec<usize>) {
        self.filters
            .upsert(SavedFilter::new(name, tree, active_group));
    }

    fn align_warm(&mut self, query: &Query) {
        let key = WarmKey::new(query, self.sort);
        if self.warm_key == key {
            return;
        }
        self.warm_key = key;
        self.warm_next_page = 1;
        self.warm_stride = AUTO_WARM_PAGES;
        self.warm_state = WarmState::Idle;
    }

    fn dispatch_warm(&mut self, query: Query, pages: u32) -> Result<()> {
        if self.date_range.active() {
            self.warm_state = WarmState::Idle;
            "date range active; upstream warm suspended".clone_into(&mut self.warm_status);
            return Ok(());
        }
        self.align_warm(&query);
        if pages == 0 {
            return Ok(());
        }
        self.warm_stride = self.warm_stride.max(pages);
        if self.warm_state != WarmState::Idle {
            return Ok(());
        }
        let first_page = self.warm_next_page;
        if first_page > DANBOORU_SEARCH_PAGE_LIMIT {
            self.warm_state = WarmState::Exhausted;
            self.warm_status = format!(
                "query warm hit Danbooru page cap after {} p{}",
                self.warm_key.label(),
                DANBOORU_SEARCH_PAGE_LIMIT
            );
            return Ok(());
        }
        let pages = self
            .warm_stride
            .max(1)
            .min(DANBOORU_SEARCH_PAGE_LIMIT - first_page + 1);
        self.warm_state = WarmState::InFlight;
        let last_page = first_page.saturating_add(pages.saturating_sub(1));
        self.warm_status = format!(
            "query warm {} p{}..p{}",
            self.warm_key.label(),
            first_page,
            last_page
        );
        let send = self.worker.send(Command::Warm {
            query,
            sort: self.sort,
            first_page,
            pages,
        });
        if let Err(err) = send {
            self.warm_state = WarmState::Idle;
            "query warm fault".clone_into(&mut self.warm_status);
            return Err(err);
        }
        Ok(())
    }

    fn drain(&mut self, ctx: &egui::Context) {
        let mut saturated = false;
        let events = self.worker.drain().take(EVENT_BUDGET).collect::<Vec<_>>();
        for (slot, event) in events.into_iter().enumerate() {
            saturated |= slot + 1 == EVENT_BUDGET;
            match event {
                Event::Refreshed { serial, hit } => {
                    self.finish_refresh(serial, Some(hit), ctx);
                }
                Event::RefreshFault { serial, fault } => {
                    if self.refresh_pulse.inflight_serial() == Some(serial) {
                        self.status = fault;
                    }
                    self.finish_refresh(serial, None, ctx);
                }
                Event::Stats { serial, stats } => {
                    self.finish_stats(serial, Some(stats), ctx);
                }
                Event::StatsFault { serial, fault } => {
                    if self.stats_pulse.inflight_serial() == Some(serial) {
                        self.cache_status = format!("cache stats fault: {fault}");
                    }
                    self.finish_stats(serial, None, ctx);
                }
                Event::Warmed {
                    query_key,
                    sort,
                    first_page,
                    pages,
                    posts,
                    exhausted,
                } => {
                    let event_key = WarmKey {
                        query: query_key,
                        sort,
                    };
                    if self.date_range.active() {
                        self.warm_state = WarmState::Idle;
                        "date range active; upstream warm suspended"
                            .clone_into(&mut self.warm_status);
                    } else if self.warm_key == event_key {
                        self.warm_state = if exhausted {
                            WarmState::Exhausted
                        } else {
                            WarmState::Idle
                        };
                        self.warm_next_page =
                            self.warm_next_page.max(first_page.saturating_add(pages));
                        self.warm_status = if exhausted {
                            let last_page = first_page.saturating_add(pages.saturating_sub(1));
                            format!(
                                "query warm exhausted after {} p{}",
                                event_key.label(),
                                last_page
                            )
                        } else {
                            format!(
                                "query warm +{posts} {}; next p{}",
                                event_key.label(),
                                self.warm_next_page
                            )
                        };
                    }
                    self.nudge_refresh();
                    self.nudge_stats();
                    if self.warm_key == event_key && self.warm_state != WarmState::Exhausted {
                        let query = self.query.clone();
                        if let Err(err) = self.dispatch_warm(query, self.warm_stride) {
                            self.status = format!("{err:#}");
                        }
                    }
                    ctx.request_repaint();
                }
                Event::Crawled { posts, before } => {
                    self.crawl_status = before.map_or_else(
                        || "crawl reached empty page".to_owned(),
                        |before| format!("crawl +{posts}; before #{before}"),
                    );
                    self.nudge_refresh();
                    self.nudge_stats();
                    ctx.request_repaint();
                }
                Event::Suggested { serial, hits } => {
                    if serial == self.suggest_serial
                        && let Some((_, memo)) = &mut self.suggest_memo
                    {
                        *memo = hits;
                        self.suggest_pick = self.suggest_pick.min(memo.len().saturating_sub(1));
                        ctx.request_repaint();
                    }
                }
                Event::Toast(text) => {
                    self.status = text;
                    ctx.request_repaint();
                }
                Event::Refetched { post } => {
                    if let Some(post) = post {
                        let _was_inflight = self.refetch_inflight.remove(&post.id);
                        if self.zoom.as_ref().is_some_and(|zoom| zoom.id == post.id) {
                            self.zoom = Some(*post.clone());
                            self.viewer_tag_groups = None;
                        }
                        // A menu open on this post re-derives its tag groups
                        // in place from the healed record.
                        if self.tag_menu.post_id() == Some(post.id)
                            && let Some((_, anchor, _)) = self.tag_menu.view()
                            && let Some((tile, _)) = self.menu_cuts
                        {
                            self.open_tag_menu(&post, anchor, tile);
                        }
                    }
                    self.nudge_refresh();
                    ctx.request_repaint();
                }
                Event::Blade { bucket, blade } => {
                    self.install_blade(ctx, blade, BladeKind::Thumb(bucket));
                }
                Event::BladeFault { id, bucket, fault } => {
                    let key = ThumbKey { id, bucket };
                    let _was_inflight = self.thumb_inflight.remove(&key);
                    let _faulted = self.thumb_faults.insert(key);
                    self.status = fault;
                    ctx.request_repaint();
                }
                Event::FullBlade(blade) => {
                    self.install_blade(ctx, blade, BladeKind::Full);
                }
                Event::FullBladeFault { id, fault } => {
                    let _was_inflight = self.full_inflight.remove(&id);
                    let _was_waiting = self.full_wait.remove(&id);
                    let _faulted = self.full_faults.insert(id);
                    self.status = fault;
                    ctx.request_repaint();
                }
                Event::MediaSaved { id, path } => {
                    self.status = format!("saved #{id} to {}", path.display());
                    ctx.request_repaint();
                }
                Event::MediaSaveFault { id, fault } => {
                    self.status = format!("save #{id} failed: {fault}");
                    ctx.request_repaint();
                }
                Event::FactsMerged {
                    batches,
                    bytes,
                    groups,
                } => {
                    self.nudge_stats();
                    self.warm_status = format!(
                        "posting merge {batches} batches, {} KiB across {groups} predicates",
                        bytes / 1024
                    );
                    ctx.request_repaint();
                }
                Event::Fault(fault) => {
                    self.status = fault;
                    ctx.request_repaint();
                }
            }
        }
        if saturated {
            ctx.request_repaint();
        }
    }

    fn install_blade(&mut self, ctx: &egui::Context, blade: RgbaBlade, kind: BladeKind) {
        match kind {
            BladeKind::Thumb(bucket) => {
                let key = ThumbKey {
                    id: blade.id,
                    bucket,
                };
                let _was_inflight = self.thumb_inflight.remove(&key);
                let _was_faulted = self.thumb_faults.remove(&key);
                let _old = self.thumbs.insert(key, blade_texture(ctx, &blade, kind));
            }
            BladeKind::Full => {
                let _was_inflight = self.full_inflight.remove(&blade.id);
                let _was_waiting = self.full_wait.remove(&blade.id);
                let _was_faulted = self.full_faults.remove(&blade.id);
                // A blade landing after its viewer closed would pin GPU memory forever.
                if self.zoom.as_ref().is_none_or(|post| post.id != blade.id) {
                    return;
                }
                let _old_texture = self.full.insert(blade.id, blade_texture(ctx, &blade, kind));
                let _old_born = self.full_loaded_at.insert(blade.id, Instant::now());
                let _old_rgba = self.full_rgba.insert(blade.id, blade);
            }
        }
        ctx.request_repaint();
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> bool {
        let width = ui.available_width().max(MIN_TILE_EDGE);
        let max_cols = (((width + GAP) / (MIN_TILE_EDGE + GAP)) as usize).max(1);
        let cols = usize::from(self.images_per_row.max(1)).min(max_cols);
        let tile = tile_edge(width, cols);
        let posts = std::mem::take(&mut self.hit.posts);
        let rows = posts.len().div_ceil(cols);
        let row_height = tile + GAP;
        let mut menu_opened = false;
        let mut visible_rows = 0..0;
        let arena = ui.max_rect();
        self.water.begin_surface(arena);
        let scroll = egui::ScrollArea::vertical().show_rows(ui, row_height, rows, |ui, range| {
            visible_rows = range.clone();
            ui.spacing_mut().item_spacing.x = GAP;
            for row in range {
                let start = row * cols;
                let end = (start + cols).min(posts.len());
                let _row = ui.horizontal(|ui| {
                    for post in &posts[start..end] {
                        menu_opened |= self.tile(ui, post, tile);
                    }
                });
            }
        });
        if posts.is_empty() {
            self.empty_gallery(ui, arena);
        } else {
            self.empty_since = None;
            self.water.set_floor(None);
            self.water.hide_loading();
            self.water.hide_drain();
        }
        self.prefetch_scroll_thumbs(
            ui.ctx(),
            &posts,
            GridScan {
                cols,
                tile,
                row_height,
                rows,
                visible_rows,
                offset: scroll.state.offset.y,
            },
        );
        self.heave(ui.ctx(), scroll.state.offset.y);
        self.hit.posts = posts;
        menu_opened
    }

    fn tile(&mut self, ui: &mut egui::Ui, post: &PostRecord, tile: f32) -> bool {
        let mut menu_opened = false;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(tile, tile), egui::Sense::click());
        crate::probe_anchor!(ui, format!("tile:{}", post.id.0), rect);
        paint_plate(ui, rect, response.hovered());
        let well = rect.shrink(PLATE_PAD);
        match self.thumb(post, tile) {
            Some(ThumbLoad::Ready(texture)) => {
                let size = fit(texture.size_vec2(), well.size());
                let image = egui::Rect::from_center_size(well.center(), size);
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let _image = ui
                    .painter()
                    .image(texture.id(), image, uv, egui::Color32::WHITE);
            }
            Some(ThumbLoad::Loading) => paint_tile_text(ui, rect, "loading"),
            Some(ThumbLoad::Fault) => paint_tile_text(ui, rect, "fault"),
            None => paint_tile_text(ui, rect, "no image"),
        }
        if response.hovered() {
            self.arm_prefetch(ui.ctx(), post);
            // The lift follows the cursor; the menu's own dim owns the grid
            // while it's open, so don't fight it.
            if !self.tag_menu.is_open() && self.zoom.is_none() {
                self.water.hover(post.id, rect);
            }
        }
        // With the tag menu up, a click anywhere only dismisses it; opening
        // the viewer underneath would make the menu feel clingy.
        if response.clicked() && !self.tag_menu.is_open() && self.zoom.is_none() {
            self.water.click(rect);
            self.open_full(post);
        }
        if response.secondary_clicked() && self.zoom.is_none() {
            if self.tag_menu.post_id() == Some(post.id) {
                // Right-click on the same image toggles its menu away.
                self.close_tag_menu();
            } else if let Some(pos) = response.interact_pointer_pos() {
                self.open_tag_menu(post, pos, rect);
                menu_opened = true;
            }
        }
        menu_opened
    }

    /// Warms the disk cache with the full image after a short hover dwell, so
    /// a click lands on bytes that are already local. The dwell keeps casual
    /// mouse sweeps from spraying multi-megabyte downloads.
    fn arm_prefetch(&mut self, ctx: &egui::Context, post: &PostRecord) {
        if !self.prefetch_on_hover
            || self.prefetched.contains(&post.id)
            || self.full_inflight.contains(&post.id)
        {
            return;
        }
        match self.hover_arm {
            Some((armed, since)) if armed == post.id => {
                if since.elapsed() < PREFETCH_DWELL {
                    ctx.request_repaint_after(PREFETCH_DWELL.saturating_sub(since.elapsed()));
                    return;
                }
                let _marked = self.prefetched.insert(post.id);
                let Some(url) = post.full_url().map(ToOwned::to_owned) else {
                    return;
                };
                if let Err(err) = self.worker.send(Command::Prefetch {
                    id: post.id,
                    url: Some(url),
                }) {
                    self.status = format!("{err:#}");
                }
            }
            _ => {
                self.hover_arm = Some((post.id, Instant::now()));
                ctx.request_repaint_after(PREFETCH_DWELL);
            }
        }
    }

    fn open_tag_menu(&mut self, post: &PostRecord, anchor: egui::Pos2, tile: egui::Rect) {
        let groups = self.learn_tag_groups(post);
        self.tag_menu = TagMenu::Open {
            post: Box::new(post.clone()),
            anchor,
            groups,
        };
        // Menu half starts at the tile; the overlay overwrites it once painted.
        self.menu_cuts = Some((tile, tile));
    }

    fn learn_tag_groups(&mut self, post: &PostRecord) -> TagGroups {
        let learned = match self.index.tag_kinds(&post.tags) {
            Ok(learned) => learned,
            Err(err) => {
                self.status = format!("{err:#}");
                BTreeMap::new()
            }
        };
        for (tag, kind) in &learned {
            if *kind != TagKind::General {
                let _old = self.tag_kinds.insert(tag.clone(), *kind);
            }
        }
        let groups = tag_palette::grouped(post, |tag| {
            learned
                .get(tag)
                .copied()
                .or_else(|| self.tag_kinds.get(tag).copied())
                .unwrap_or_default()
        });
        // Records absorbed before tag hints existed carry no kinds; one
        // rate-gated refetch heals them, and the open menu updates live.
        if post.tag_hints.is_empty()
            && self.refetch_inflight.insert(post.id)
            && let Err(err) = self.worker.send(Command::Refetch { id: post.id })
        {
            let _was_inflight = self.refetch_inflight.remove(&post.id);
            self.status = format!("{err:#}");
        }
        groups
    }

    fn thumb(&mut self, post: &PostRecord, edge: f32) -> Option<ThumbLoad> {
        let bucket = thumb_bucket(edge);
        let key = ThumbKey {
            id: post.id,
            bucket,
        };
        if let Some(texture) = self.thumbs.get(&key).cloned() {
            return Some(ThumbLoad::Ready(texture));
        }
        if self.thumb_faults.contains(&key) {
            return Some(ThumbLoad::Fault);
        }
        match self.arm_thumb(post, edge) {
            ThumbArm::Missing => None,
            ThumbArm::Fault => Some(ThumbLoad::Fault),
            ThumbArm::Armed | ThumbArm::Pending => Some(ThumbLoad::Loading),
        }
    }

    fn prefetch_scroll_thumbs(
        &mut self,
        ctx: &egui::Context,
        posts: &[PostRecord],
        scan: GridScan,
    ) {
        if posts.is_empty() {
            return;
        }
        let dt = ctx.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.08);
        let Some(band) = self.thumb_cruise.wake(
            scan.offset,
            ctx.pixels_per_point(),
            dt,
            scan.row_height,
            scan.rows,
            scan.visible_rows,
        ) else {
            return;
        };
        let mut armed = 0;
        for row in band {
            let start = row * scan.cols;
            let end = (start + scan.cols).min(posts.len());
            for post in &posts[start..end] {
                if armed >= THUMB_PREFETCH_BUDGET {
                    return;
                }
                armed += usize::from(self.arm_thumb(post, scan.tile) == ThumbArm::Armed);
            }
        }
    }

    fn arm_thumb(&mut self, post: &PostRecord, edge: f32) -> ThumbArm {
        let bucket = thumb_bucket(edge);
        let key = ThumbKey {
            id: post.id,
            bucket,
        };
        if self.thumbs.contains_key(&key) {
            return ThumbArm::Pending;
        }
        if self.thumb_faults.contains(&key) {
            return ThumbArm::Fault;
        }
        if self.thumb_inflight.contains(&key) {
            return ThumbArm::Pending;
        }
        let Some(url) = post.thumb_url(edge).map(ToOwned::to_owned) else {
            return ThumbArm::Missing;
        };
        let _now_inflight = self.thumb_inflight.insert(key);
        if let Err(err) = self.worker.send(Command::Blade {
            epoch: self.thumb_epoch,
            id: post.id,
            bucket,
            url: Some(url),
        }) {
            let _was_inflight = self.thumb_inflight.remove(&key);
            let _faulted = self.thumb_faults.insert(key);
            self.status = format!("{err:#}");
            return ThumbArm::Fault;
        }
        ThumbArm::Armed
    }

    fn zoom_tiles(&mut self, ctx: &egui::Context) {
        if self.tag_menu.is_open() {
            return;
        }
        let steps = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } if modifiers.ctrl => Some(match unit {
                        egui::MouseWheelUnit::Point => delta.y / 120.0,
                        egui::MouseWheelUnit::Line => delta.y,
                        egui::MouseWheelUnit::Page => delta.y * 4.0,
                    }),
                    _ => None,
                })
                .sum::<f32>()
        });
        if steps == 0.0 {
            return;
        }
        let delta = -steps.round() as i32;
        self.images_per_row = (i32::from(self.images_per_row) + delta)
            .clamp(i32::from(MIN_IMAGES_PER_ROW), i32::from(MAX_IMAGES_PER_ROW))
            as u16;
        self.advance_thumb_epoch();
        self.save_config();
        ctx.request_repaint();
    }

    /// Syncs the active filter's mirror and marks persistence dirty; the write
    /// itself is debounced so wheel ticks and rail drags do not thrash the disk.
    fn save_config(&mut self) {
        self.sync_active_filter();
        self.config_dirty = Some(Instant::now());
    }

    fn flush_config(&mut self, ctx: &egui::Context) {
        let Some(dirty_at) = self.config_dirty else {
            return;
        };
        let settled = dirty_at.elapsed();
        if settled < CONFIG_SETTLE {
            ctx.request_repaint_after(CONFIG_SETTLE.saturating_sub(settled));
            return;
        }
        self.config_dirty = None;
        self.write_config();
    }

    /// Writes both halves of persistence: config (user intent) and slate
    /// (workbench state). Both are tiny and atomic; one dirty flag covers them.
    fn write_config(&mut self) {
        let config = Config {
            prefetch_on_hover: self.prefetch_on_hover,
            filters: FilterConfig {
                saved: self.filters.root.clone(),
                shelves: self.filters.shelves.clone(),
            },
        };
        let slate = Slate {
            closed_folders: self
                .filters
                .shelves
                .iter()
                .filter(|shelf| !shelf.open)
                .map(|shelf| shelf.name.clone())
                .collect(),
            shutters: self.shutters.clone(),
            active_filter: self.active_filter.clone(),
            query: QueryConfig {
                tree: self.query.clone(),
                active_group: self.active_group.clone(),
            },
            sort: self.sort,
            dates: self.date_range.normalized(),
            images_per_row: self.images_per_row,
            water: self.water_mode,
            viewer_tags_open: self.viewer_tags_open,
        };
        let written = config
            .save(&self.lair.config_path())
            .and_then(|()| slate.save(&self.lair.slate_path()));
        if let Err(err) = written {
            self.status = format!("{err:#}");
        }
    }

    fn report_startup_probe(&mut self) {
        let Some(probe) = &mut self.startup_probe else {
            return;
        };
        if probe.reported {
            return;
        }
        match probe.report() {
            Ok(()) => {}
            Err(err) => self.status = format!("{err:#}"),
        }
    }
}

/// The active-query warmer's lifecycle for the current warm key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WarmState {
    Idle,
    InFlight,
    Exhausted,
}

#[derive(Clone, Copy)]
enum BladeKind {
    Thumb(u8),
    Full,
}

enum ThumbLoad {
    Ready(TextureHandle),
    Loading,
    Fault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbArm {
    Armed,
    Pending,
    Missing,
    Fault,
}

struct GridScan {
    cols: usize,
    tile: f32,
    row_height: f32,
    rows: usize,
    visible_rows: std::ops::Range<usize>,
    offset: f32,
}

impl BladeKind {
    fn texture_prefix(self) -> &'static str {
        match self {
            Self::Thumb(bucket) => match bucket {
                0 => "post-180",
                1 => "post-360",
                _ => "post-720",
            },
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ThumbKey {
    id: PostId,
    bucket: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WarmKey {
    query: String,
    sort: Sort,
}

impl WarmKey {
    fn new(query: &Query, sort: Sort) -> Self {
        Self {
            query: query.to_text(),
            sort,
        }
    }

    fn label(&self) -> String {
        if self.query.is_empty() {
            format!("{} ∅", self.sort.label())
        } else {
            format!("{} {}", self.sort.label(), self.query)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HitKey {
    query: String,
    sort: Sort,
    dates: DateRange,
}

impl HitKey {
    fn new(query: &Query, sort: Sort, dates: DateRange) -> Self {
        Self {
            query: query.to_text(),
            sort,
            dates: dates.normalized(),
        }
    }
}

#[derive(Default)]
struct HitCache {
    slots: Vec<(HitKey, SearchHit)>,
}

impl HitCache {
    fn get(&mut self, key: &HitKey) -> Option<SearchHit> {
        let slot = self.slots.iter().position(|(found, _)| found == key)?;
        let pair = self.slots.remove(slot);
        let hit = pair.1.clone();
        self.slots.insert(0, pair);
        Some(hit)
    }

    fn put(&mut self, key: HitKey, hit: SearchHit) {
        if hit.posts.is_empty() {
            return;
        }
        if let Some(slot) = self.slots.iter().position(|(found, _)| found == &key) {
            let _old = self.slots.remove(slot);
        }
        self.slots.insert(0, (key, hit));
        self.slots.truncate(HIT_CACHE_LIMIT);
    }
}

struct ActivePrefix {
    body: String,
    negative: bool,
}

struct StartupProbe {
    path: PathBuf,
    reported: bool,
}

impl StartupProbe {
    fn from_env() -> Option<Self> {
        env::var_os("ADEQUATE_BOORU_VIEWER_STARTUP_PROBE").map(|path| Self {
            path: PathBuf::from(path),
            reported: false,
        })
    }

    fn report(&mut self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&self.path, b"gui-ready\n")
            .with_context(|| format!("write {}", self.path.display()))?;
        self.reported = true;
        Ok(())
    }
}

fn active_prefix(text: &str) -> Option<ActivePrefix> {
    if text.ends_with(char::is_whitespace) {
        return None;
    }
    let token = text.split_whitespace().next_back()?;
    let (negative, body) = match token.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, token.strip_prefix('+').unwrap_or(token)),
    };
    let body = body.trim();
    (!body.is_empty()).then(|| ActivePrefix {
        body: body.to_owned(),
        negative,
    })
}

fn take_tab_cycle(input: &mut egui::InputState) -> Option<GroupCycle> {
    if input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
        Some(GroupCycle::Backward)
    } else if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
        Some(GroupCycle::Forward)
    } else {
        None
    }
}

/// Veil opacity for an open/closed source: rises gently, falls twice as fast.
fn veil_strength(ctx: &egui::Context, id: &'static str, open: bool) -> f32 {
    ctx.animate_bool_with_time(
        egui::Id::new(id),
        open,
        if open { VEIL_RISE } else { VEIL_FALL },
    )
}

fn posts_changed(old: &[PostRecord], new: &[PostRecord]) -> bool {
    old.len() != new.len()
        || old
            .iter()
            .zip(new)
            .any(|(old, new)| old.id != new.id || old.thumb_url(360.0) != new.thumb_url(360.0))
}

/// Fraction of the shared grid positions whose post actually changed — the
/// pool-thwack energy. Compared over the common prefix only, so growing or
/// trimming the tail (every date scroll, Newest-sorted) reads as zero, while a
/// reorder or re-query that *replaces* tiles reads near one.
fn swap_fraction(old: &[PostRecord], new: &[PostRecord]) -> f32 {
    let common = old.len().min(new.len());
    if common == 0 {
        return 0.0;
    }
    let changed = old
        .iter()
        .zip(new)
        .filter(|(old, new)| old.id != new.id)
        .count();
    changed as f32 / common as f32
}

fn blade_texture(ctx: &egui::Context, blade: &RgbaBlade, kind: BladeKind) -> TextureHandle {
    let image = ColorImage::from_rgba_unmultiplied(blade.size, &blade.rgba);
    ctx.load_texture(
        format!("{}-{}", kind.texture_prefix(), blade.id),
        image,
        TextureOptions::LINEAR,
    )
}

/// The mat under every thumbnail: a faintly raised plate that separates
/// neighbors across the gutter and gives badges a surface to anchor to.
fn paint_plate(ui: &egui::Ui, rect: egui::Rect, hovered: bool) {
    let _fill = ui.painter().rect_filled(rect, 2.0, chrome::SURFACE);
    let edge = if hovered {
        chrome::EDGE_STRONG
    } else {
        chrome::EDGE.gamma_multiply(0.55)
    };
    let _stroke = ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0_f32, edge),
        egui::StrokeKind::Inside,
    );
}

fn paint_tile_text(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let _text = ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

fn fit(image: egui::Vec2, bounds: egui::Vec2) -> egui::Vec2 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return bounds;
    }
    let scale = (bounds.x / image.x).min(bounds.y / image.y).min(1.0);
    image * scale
}

fn tile_edge(width: f32, columns: usize) -> f32 {
    let columns = columns.max(1);
    let gaps = GAP * columns.saturating_sub(1) as f32;
    ((width - gaps) / columns as f32).max(MIN_TILE_EDGE)
}

fn thumb_bucket(edge: f32) -> u8 {
    if edge > 390.0 {
        2
    } else {
        u8::from(edge > 190.0)
    }
}

fn consume_wheel(ctx: &egui::Context) {
    ctx.input_mut(|input| {
        input
            .events
            .retain(|event| !matches!(event, egui::Event::MouseWheel { .. }));
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
}

fn clean_dates(dates: DateRange) -> DateRange {
    dates.normalized().scrub_before(CreatedDay::booru_floor())
}

impl Drop for Bayonet {
    fn drop(&mut self) {
        if self.config_dirty.is_some() {
            self.write_config();
        }
    }
}

impl Bayonet {
    fn paint(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let _left = egui::Panel::left("filter")
            .resizable(false)
            .exact_size(chrome::INSPECTOR_WIDTH)
            .show_inside(ui, |ui| {
                let scroll_id = ui.make_persistent_id(egui::Id::new("filter-scroll"));
                let scroll_before = egui::scroll_area::State::load(ui.ctx(), scroll_id);
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("filter-scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(ui.spacing().item_spacing.x);
                        self.left_panel(ui);
                    });
                if date_spool::take_wheel_claim(&ctx) {
                    scroll_before.unwrap_or_default().store(&ctx, scroll.id);
                    ctx.request_repaint();
                }
            });
        let prior = self.tag_menu.post_id();
        self.tag_menu_rect = None;
        self.tag_palette_overlay(&ctx);
        self.absorb_tag_menu_wheel(&ctx);
        let mut menu_opened = false;
        let _center = egui::CentralPanel::default().show_inside(ui, |ui| {
            menu_opened = self.grid(ui);
        });
        if self.tag_menu.post_id() != prior {
            self.tag_menu_rect = None;
            self.tag_palette_overlay(&ctx);
            ctx.request_repaint();
        }
        self.retain_tag_menu(&ctx, menu_opened);
        self.full_frame(&ctx);
    }
}
