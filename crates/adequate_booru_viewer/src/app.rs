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
    config::{Config, FilterConfig, FilterName, QueryConfig, SavedFilter, Slate},
    filter_bank::Bank,
    frost::{Cut, Definition, Veil},
    index::{CacheStats, Index, TagSuggestion},
    media::{MediaCache, RgbaBlade, extension},
    model::{
        BoolOp, PostId, PostRecord, Query, QueryAtom, SearchHit, Sort, Tag, TagKind, TagPolarity,
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
mod palette;
mod panels;
mod refresh;
mod scroll;
mod viewer;
mod water;

use refresh::{AsyncPulse, PulseGate};
use scroll::TrayTilt;
use viewer::ZoomGate;
use water::{LiftPlate, LoadingRaft, Plunge, TouchPlunge};

const RESULT_LIMIT: usize = 360;
const EVENT_BUDGET: usize = 12;
const AUTO_WARM_PAGES: u32 = 1;
const DANBOORU_SEARCH_PAGE_LIMIT: u32 = 1_000;
const MIN_IMAGES_PER_ROW: u16 = 1;
const MAX_IMAGES_PER_ROW: u16 = 12;
const MIN_TILE_EDGE: f32 = 72.0;
const GAP: f32 = 12.0;
const LOADING_CARD_W: f32 = 250.0;
const LOADING_CARD_H: f32 = 150.0;
const VIEWER_CHROME: f32 = 40.0;
const MAX_GROUP_DEPTH: usize = 8;
const PLATE_PAD: f32 = 4.0;
const PREFETCH_DWELL: Duration = Duration::from_millis(120);
const CONFIG_SETTLE: Duration = Duration::from_millis(400);
const VEIL_RADIUS: f32 = 2.0;
const VEIL_RISE: f32 = 0.12;
const VEIL_FALL: f32 = 0.06;
const ZOOM_DIM: f32 = 0.78;
const MENU_DIM: f32 = 0.62;
/// The CPU-side water tunables, sibling to `frost::Brine` (the shader side);
/// both adjustable live via the water bench (F12). Defaults are the shipped
/// feel.
struct Surf {
    /// Splash amplitudes: surfacing throws a wave, sinking sheds a softer
    /// ring, a click (plate leaving the water) makes the biggest splash.
    enter_amp: f32,
    exit_amp: f32,
    click_amp: f32,
    /// The viewer pond still uses analytic point ripples; this is their fade.
    text_amp: f32,
    viewer_amp: f32,
    viewer_life: f32,
    /// Button plates ring down in the boiler after pointer contact leaves.
    quiver_release: f32,
    /// Scroll inertia: tray velocity maps to a bounded surface tilt; the
    /// persistent solver performs the ensuing slosh.
    scroll_coupling: f32,
    scroll_tau: f32,
    /// Debug guard: periodically read the water field and zero it if poisoned.
    poison_sweep: bool,
    /// First-order relaxation of the lift plates: rise a little faster than
    /// sink, so the slosh settles slowly.
    tau_rise: f32,
    tau_fall: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaterUi {
    Dry,
    Wet(Definition),
}

impl WaterUi {
    fn from_slate(wet: bool, hd: bool) -> Self {
        if wet {
            Self::Wet(if hd { Definition::Hd } else { Definition::Sd })
        } else {
            Self::Dry
        }
    }

    fn wet(self) -> bool {
        matches!(self, Self::Wet(_))
    }

    fn definition(self) -> Definition {
        match self {
            Self::Dry | Self::Wet(Definition::Sd) => Definition::Sd,
            Self::Wet(Definition::Hd) => Definition::Hd,
        }
    }

    fn is(self, definition: Definition) -> bool {
        matches!(self, Self::Wet(active) if active == definition)
    }
}

impl Default for Surf {
    fn default() -> Self {
        Self {
            enter_amp: 0.9,
            exit_amp: 0.42,
            click_amp: 2.5,
            text_amp: 1.02,
            viewer_amp: 1.6,
            viewer_life: 8.0,
            quiver_release: 0.48,
            scroll_coupling: 0.0028,
            scroll_tau: 0.11,
            poison_sweep: true,
            tau_rise: 0.09,
            tau_fall: 0.24,
        }
    }
}

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
    refresh_serial: u64,
    refresh_pulse: AsyncPulse,
    refresh_gate: PulseGate,
    stats_serial: u64,
    stats_pulse: AsyncPulse,
    stats_gate: PulseGate,
    hit: SearchHit,
    parked_hit: Option<SearchHit>,
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
    full_inflight: HashSet<PostId>,
    full_faults: HashSet<PostId>,
    zoom: Option<PostRecord>,
    zoom_gate: ZoomGate,
    zoom_rect: Option<egui::Rect>,
    viewer_tags_open: bool,
    viewer_tag_groups: Option<(PostId, TagGroups)>,
    images_per_row: u16,
    tag_menu: TagMenu,
    tag_menu_rect: Option<egui::Rect>,
    menu_cuts: Option<(egui::Rect, egui::Rect)>,
    hover_tile: Option<(PostId, egui::Rect)>,
    lift_plates: Vec<LiftPlate>,
    splash_memo: Option<(PostId, egui::Rect)>,
    plunges: Vec<Plunge>,
    viewer_touches: Vec<TouchPlunge>,
    loading_raft: LoadingRaft,
    water_until: Option<Instant>,
    viewer_pond: egui::Rect,
    water_rect: egui::Rect,
    water_ui: WaterUi,
    scroll: TrayTilt,
    scroll_tilt: f32,
    brine: crate::frost::Brine,
    surf: Surf,
    bench_open: bool,
    tag_kinds: HashMap<Tag, TagKind>,
    suggest_memo: Option<(String, Vec<TagSuggestion>)>,
    suggest_serial: u64,
    refetch_inflight: HashSet<PostId>,
    prefetch_on_hover: bool,
    prefetched: HashSet<PostId>,
    hover_arm: Option<(PostId, Instant)>,
    config_dirty: Option<Instant>,
    cache_stats: CacheStats,
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
        let active_filter = filters.active(slate.active_filter.clone());
        let mut query = active_filter
            .as_ref()
            .and_then(|active| filters.get(active))
            .map_or_else(|| slate.query.tree.clone(), |filter| filter.tree.clone());
        query.sort_atoms();
        let sort = slate.sort;
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
            refresh_serial: 0,
            refresh_pulse: AsyncPulse::Idle,
            refresh_gate: PulseGate::refresh(),
            stats_serial: 0,
            stats_pulse: AsyncPulse::Idle,
            stats_gate: PulseGate::stats(),
            hit: SearchHit::default(),
            parked_hit: None,
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
            tag_menu: TagMenu::Closed,
            tag_menu_rect: None,
            menu_cuts: None,
            hover_tile: None,
            lift_plates: Vec::new(),
            splash_memo: None,
            plunges: Vec::new(),
            viewer_touches: Vec::new(),
            loading_raft: LoadingRaft::new(),
            water_until: None,
            viewer_pond: egui::Rect::ZERO,
            water_rect: egui::Rect::ZERO,
            water_ui: WaterUi::from_slate(slate.water_wet, slate.water_hd),
            scroll: TrayTilt::default(),
            scroll_tilt: 0.0,
            brine: crate::frost::Brine::default(),
            surf: Surf::default(),
            bench_open: false,
            tag_kinds: HashMap::new(),
            suggest_memo: None,
            suggest_serial: 0,
            refetch_inflight: HashSet::new(),
            prefetch_on_hover: config.prefetch_on_hover,
            prefetched: HashSet::new(),
            hover_arm: None,
            config_dirty: None,
            cache_stats: CacheStats::default(),
            cache_status: "cache measuring".to_owned(),
            warm_status: "query warm idle".to_owned(),
            startup_probe: StartupProbe::from_env(),
        };
        startup("app.state.built");
        app.strike(true, AUTO_WARM_PAGES);
        startup("app.initial.reap.done");
        Ok(app)
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
        let ctx = ui.ctx().clone();
        self.zoom_tiles(&ctx);
        self.drain(&ctx);
        self.flush_pulse_gates(&ctx);
        self.flush_config(&ctx);
        self.paint(ui);
        // Quivering buttons shed continuous wavetrains; while any seed lives
        // the water moves, so keep painting.
        let quivering = ctx.data(|data| {
            data.get_temp::<Vec<crate::frost::Tension>>(egui::Id::new("tension-field"))
                .is_some_and(|seeds| !seeds.is_empty())
        });
        if quivering {
            ctx.request_repaint();
        }
        self.bench(&ctx);
        self.report_startup_probe();
    }

    /// The water chemistry for the compose pass.
    pub fn brine(&self) -> crate::frost::Brine {
        self.brine
    }

    pub fn water_wet(&self) -> bool {
        self.water_ui.wet()
    }

    pub fn water_definition(&self) -> Definition {
        self.water_ui.definition()
    }

    pub fn quiver_release(&self) -> f32 {
        self.surf.quiver_release
    }

    pub fn water_guard(&self) -> bool {
        self.surf.poison_sweep
    }

    /// Frost parameters for the boiler's blur pass, in physical pixels.
    /// `None` while no veil is showing (the common case — zero GPU cost).
    ///
    /// While a veil is fading out its cutouts are dropped, so the blur turns
    /// uniform and recedes evenly instead of leaving sharp negative space.
    pub fn frost_veil(&self, ctx: &egui::Context, pixels_per_point: f32) -> Option<Veil> {
        let cut = |rect: egui::Rect, radius: f32| Cut {
            rect: egui::Rect::from_min_max(
                (rect.min.to_vec2() * pixels_per_point).to_pos2(),
                (rect.max.to_vec2() * pixels_per_point).to_pos2(),
            ),
            radius: radius * pixels_per_point,
        };
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

    fn install_query_at(&mut self, query: Query, active_group: Vec<usize>) {
        let mut query = query;
        query.sort_atoms();
        self.active_group = query.clamp_group_path(&active_group);
        self.query = query;
        self.clear_hit();
        let query = self.query.clone();
        self.align_warm(&query);
        self.save_config();
        self.strike(true, AUTO_WARM_PAGES);
    }

    fn clear_hit(&mut self) {
        self.parked_hit = None;
        self.commit_hit(SearchHit::default());
    }

    fn install_hit(&mut self, hit: SearchHit) {
        if self.tag_menu.is_open() {
            self.parked_hit = Some(hit);
            return;
        }
        self.commit_hit(hit);
    }

    fn commit_hit(&mut self, hit: SearchHit) {
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
                    if self.warm_key == event_key {
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
                let _was_faulted = self.full_faults.remove(&blade.id);
                // A blade landing after its viewer closed would pin GPU memory forever.
                if self.zoom.as_ref().is_none_or(|post| post.id != blade.id) {
                    return;
                }
                let _old_texture = self.full.insert(blade.id, blade_texture(ctx, &blade, kind));
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
        self.hover_tile = None;
        let arena = ui.max_rect();
        self.water_rect = arena;
        let scroll = egui::ScrollArea::vertical().show_rows(ui, row_height, rows, |ui, range| {
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
            self.loading_card(ui, arena);
        } else {
            self.loading_raft.hide();
        }
        self.heave(ui.ctx(), scroll.state.offset.y, ui.ctx().pixels_per_point());
        self.hit.posts = posts;
        menu_opened
    }

    fn loading_card(&mut self, ui: &mut egui::Ui, arena: egui::Rect) {
        let size = egui::vec2(
            LOADING_CARD_W.min((arena.width() - 24.0).max(120.0)),
            LOADING_CARD_H.min((arena.height() - 24.0).max(96.0)),
        );
        let rect = egui::Rect::from_center_size(arena.center(), size);
        if self.water_ui.wet() {
            self.loading_raft.show(ui.ctx(), rect);
            self.arm_water();
        } else {
            self.loading_raft.hide();
        }

        let painter = ui.painter();
        let _fill = painter.rect_filled(rect, 2.0, chrome::SURFACE);
        let _stroke = painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, chrome::EDGE_STRONG),
            egui::StrokeKind::Inside,
        );
        let title_font = egui::FontId::new(25.0, egui::FontFamily::Proportional);
        let percent_font = egui::FontId::new(38.0, egui::FontFamily::Proportional);
        let title = "LOADING";
        let percent = self.loading_percent();
        let title_galley =
            painter.layout_no_wrap(title.to_owned(), title_font.clone(), chrome::HOT);
        let percent_galley =
            painter.layout_no_wrap(percent.clone(), percent_font.clone(), chrome::TEXT);
        let title_at = egui::pos2(
            rect.center().x - title_galley.size().x * 0.5,
            rect.top() + 28.0,
        );
        let percent_at = egui::pos2(
            rect.center().x - percent_galley.size().x * 0.5,
            rect.center().y + 7.0,
        );
        let _title = painter.text(
            title_at,
            egui::Align2::LEFT_TOP,
            title,
            title_font,
            chrome::HOT,
        );
        let _percent = painter.text(
            percent_at,
            egui::Align2::LEFT_TOP,
            percent,
            percent_font,
            chrome::TEXT,
        );
    }

    fn loading_percent(&self) -> String {
        self.cache_stats
            .rough_crawl_percent()
            .map_or_else(|| "—".to_owned(), rough_percent)
    }

    fn tile(&mut self, ui: &mut egui::Ui, post: &PostRecord, tile: f32) -> bool {
        let mut menu_opened = false;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(tile, tile), egui::Sense::click());
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
                self.hover_tile = Some((post.id, rect));
            }
        }
        // With the tag menu up, a click anywhere only dismisses it; opening
        // the viewer underneath would make the menu feel clingy.
        if response.clicked() && !self.tag_menu.is_open() && self.zoom.is_none() {
            self.plunge(rect, self.surf.click_amp);
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

    fn thumb(&mut self, post: &PostRecord, edge: f32) -> Option<ThumbLoad<'_>> {
        let bucket = thumb_bucket(edge);
        let key = ThumbKey {
            id: post.id,
            bucket,
        };
        if let Some(texture) = self.thumbs.get(&key) {
            return Some(ThumbLoad::Ready(texture));
        }
        if self.thumb_faults.contains(&key) {
            return Some(ThumbLoad::Fault);
        }
        if !self.thumb_inflight.contains(&key) {
            let url = post.thumb_url(edge).map(ToOwned::to_owned)?;
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
                return Some(ThumbLoad::Fault);
            }
        }
        Some(ThumbLoad::Loading)
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
            active_filter: self.active_filter.clone(),
            query: QueryConfig {
                tree: self.query.clone(),
                active_group: self.active_group.clone(),
            },
            sort: self.sort,
            images_per_row: self.images_per_row,
            water_wet: self.water_ui.wet(),
            water_hd: self.water_ui.is(Definition::Hd),
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

enum ThumbLoad<'a> {
    Ready(&'a TextureHandle),
    Loading,
    Fault,
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
        egui::Stroke::new(1.0, edge),
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

fn rough_percent(value: f32) -> String {
    let value = value.clamp(0.0, 100.0);
    if value < 10.0 {
        format!("{value:.3}%")
    } else {
        format!("{value:.2}%")
    }
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
                let _scroll = egui::ScrollArea::vertical()
                    .id_salt("filter-scroll")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(ui.spacing().item_spacing.x);
                        self.left_panel(ui);
                    });
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
