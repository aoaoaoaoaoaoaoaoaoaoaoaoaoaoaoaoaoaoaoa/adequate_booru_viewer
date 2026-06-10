use anyhow::{Context as _, Result};
use arboard::{Clipboard, ImageData};
use eframe::{
    App, CreationContext,
    egui::{self, ColorImage, TextureHandle, TextureOptions},
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    env, fs,
    path::PathBuf,
};

use crate::{
    chrome,
    config::{
        Config, EmbeddingConfig, FilterConfig, FilterName, PinConfig, QueryConfig, SavedFilter,
        ViewConfig,
    },
    filter_bank,
    index::{CacheStats, Index, TagSuggestion},
    media::{MediaCache, RgbaBlade, extension},
    model::{
        BoolOp, Embedding, PostId, PostRecord, Query, QueryAtom, SearchHit, Sort, Tag, TagKind,
        TagPolarity,
    },
    query_ui::{QueryAction, render_query_tree},
    saved_filter_ui::{self, Action as SavedFilterAction},
    tag_chroma,
    tag_menu::{
        HEIGHT as TAG_MENU_HEIGHT, TagMenu, WIDTH as TAG_MENU_WIDTH, position as tag_menu_pos,
    },
    tag_palette,
    trace::startup,
    worker::{BladeEpoch, Command, Event, RefreshHit, SoftRefresh, Worker},
    xdg::{Lair, compact_path},
};

mod panels;
mod refresh;

use refresh::AsyncPulse;

const RESULT_LIMIT: usize = 360;
const SOFT_POOL: usize = 2_400;
const SOFT_BACKLOG: usize = 128;
const SUGGESTIONS: usize = 12;
const EVENT_BUDGET: usize = 12;
const AUTO_WARM_PAGES: u32 = 1;
const DANBOORU_SEARCH_PAGE_LIMIT: u32 = 1_000;
const MIN_IMAGES_PER_ROW: u16 = 1;
const MAX_IMAGES_PER_ROW: u16 = 12;
const MIN_TILE_EDGE: f32 = 72.0;
const GAP: f32 = 8.0;
const VIEWER_CHROME: f32 = 40.0;
const MAX_PINS: usize = 6;

pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query: Query,
    active_group: Vec<usize>,
    tag_entry: String,
    filter_name_entry: String,
    active_filter: Option<FilterName>,
    saved_filters: Vec<SavedFilter>,
    rank_alpha: f32,
    rank_pins: Vec<RankPin>,
    sort: Sort,
    refresh_serial: u64,
    refresh_pulse: AsyncPulse,
    stats_serial: u64,
    stats_pulse: AsyncPulse,
    hit: SearchHit,
    thumbs: HashMap<ThumbKey, TextureHandle>,
    thumb_inflight: HashSet<ThumbKey>,
    thumb_faults: HashSet<ThumbKey>,
    thumb_epoch: BladeEpoch,
    warm_key: WarmKey,
    warm_next_page: u32,
    warm_stride: u32,
    warm_inflight: bool,
    warm_exhausted: bool,
    full: HashMap<PostId, TextureHandle>,
    full_rgba: HashMap<PostId, RgbaBlade>,
    full_inflight: HashSet<PostId>,
    full_faults: HashSet<PostId>,
    zoom: Option<PostRecord>,
    zoom_gate: ZoomGate,
    images_per_row: u16,
    tag_menu: TagMenu,
    tag_menu_rect: Option<egui::Rect>,
    tag_kinds: HashMap<Tag, TagKind>,
    clip_inflight: HashSet<PostId>,
    cache_status: String,
    warm_status: String,
    crawl_status: String,
    status: String,
    startup_probe: Option<StartupProbe>,
}

impl Bayonet {
    pub fn new(_cc: &CreationContext<'_>) -> Result<Self> {
        Self::open()
    }

    pub fn open() -> Result<Self> {
        startup("app.open.enter");
        let lair = Lair::claim()?;
        startup("app.lair.claimed");
        let config = Config::load(&lair.config_path())?;
        startup("app.config.loaded");
        let index = Index::open(&lair.index_path())?;
        startup("app.index.opened");
        let media = MediaCache::new(lair.media_dir())?;
        startup("app.media.opened");
        let worker = Worker::spawn(index.clone(), media, lair.model_dir());
        startup("app.worker.spawned");
        let saved_filters = filter_bank::sorted(config.filters.saved.clone());
        let active_filter = filter_bank::active(config.filters.active.clone(), &saved_filters);
        let mut query = active_filter
            .as_ref()
            .and_then(|active| filter_bank::get(active, &saved_filters))
            .map_or_else(|| config.query.query(), |filter| filter.tree.clone());
        query.sort_atoms();
        let sort = config.view.sort;
        let active_group = active_filter
            .as_ref()
            .and_then(|active| filter_bank::get(active, &saved_filters))
            .map_or_else(
                || query.clamp_group_path(&config.query.active_group),
                |filter| query.clamp_group_path(&filter.active_group),
            );
        let rank_pins = restore_rank_pins(&index, &config.embedding.pins)?;
        let mut app = Self {
            status: format!("index {}", compact_path(&lair.index_path())),
            crawl_status: "crawl waking".to_owned(),
            lair,
            index,
            worker,
            query: query.clone(),
            active_group,
            tag_entry: String::new(),
            filter_name_entry: String::new(),
            active_filter,
            saved_filters,
            rank_alpha: config.embedding.alpha.clamp(0.0, 2.0),
            rank_pins,
            sort,
            refresh_serial: 0,
            refresh_pulse: AsyncPulse::Idle,
            stats_serial: 0,
            stats_pulse: AsyncPulse::Idle,
            hit: SearchHit::default(),
            thumbs: HashMap::new(),
            thumb_inflight: HashSet::new(),
            thumb_faults: HashSet::new(),
            thumb_epoch: BladeEpoch::ROOT,
            warm_key: WarmKey::new(&query, sort),
            warm_next_page: 1,
            warm_stride: AUTO_WARM_PAGES,
            warm_inflight: false,
            warm_exhausted: false,
            full: HashMap::new(),
            full_rgba: HashMap::new(),
            full_inflight: HashSet::new(),
            full_faults: HashSet::new(),
            zoom: None,
            zoom_gate: ZoomGate::Fresh,
            images_per_row: config
                .view
                .images_per_row
                .clamp(MIN_IMAGES_PER_ROW, MAX_IMAGES_PER_ROW),
            tag_menu: TagMenu::Closed,
            tag_menu_rect: None,
            tag_kinds: HashMap::new(),
            clip_inflight: HashSet::new(),
            cache_status: "cache measuring".to_owned(),
            warm_status: "query warm idle".to_owned(),
            startup_probe: StartupProbe::from_env(),
        };
        startup("app.state.built");
        app.strike(true, AUTO_WARM_PAGES);
        startup("app.initial.reap.done");
        Ok(app)
    }

    pub fn draw_startup_probe_frame(&mut self) {
        startup("app.draw.enter");
        let ctx = egui::Context::default();
        startup("app.draw.ctx");
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 920.0),
                )),
                ..Default::default()
            },
            |ui| {
                self.zoom_tiles(ui.ctx());
                self.drain(ui.ctx());
                self.paint(ui);
            },
        );
        startup("app.draw.ui.done");
        let _primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        startup("app.draw.tessellated");
        self.report_startup_probe();
        startup("app.draw.probe.reported");
    }

    fn query(&self) -> Query {
        self.query.clone()
    }

    fn install_query(&mut self, query: Query) {
        self.install_query_at(query, self.active_group.clone());
    }

    fn install_query_at(&mut self, query: Query, active_group: Vec<usize>) {
        let mut query = query;
        query.sort_atoms();
        self.active_group = query.clamp_group_path(&active_group);
        self.query = query;
        self.advance_thumb_epoch();
        let query = self.query.clone();
        self.align_warm(&query);
        self.sync_active_filter();
        self.save_config();
        self.strike(true, AUTO_WARM_PAGES);
    }

    fn install_hit(&mut self, hit: SearchHit) {
        if posts_changed(&self.hit.posts, &hit.posts) {
            self.advance_thumb_epoch();
        }
        self.hit = hit;
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

    fn set_tag(&mut self, raw: &str, polarity: TagPolarity) {
        let Some(tag) = Tag::forge(raw) else {
            return;
        };
        self.add_atom(QueryAtom::Tag(tag), polarity);
    }

    fn add_atom(&mut self, atom: QueryAtom, polarity: TagPolarity) {
        let mut query = self.query.clone();
        if query.push_atom(&self.active_group, atom, polarity) {
            self.install_query(query);
        }
    }

    fn remove_tag(&mut self, raw: &str) {
        let Some(tag) = Tag::forge(raw) else {
            return;
        };
        let mut query = self.query.clone();
        query.remove_atom(&QueryAtom::Tag(tag));
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
                .unwrap_or_else(|| filter_bank::spare(&self.query, &self.saved_filters))
        });
        self.upsert_filter(name.clone(), self.query.clone(), self.active_group.clone());
        self.active_filter = Some(name.clone());
        self.filter_name_entry.clear();
        self.status = format!("saved filter `{name}`");
        self.save_config();
    }

    fn load_filter(&mut self, filter: SavedFilter) {
        self.active_filter = Some(filter.name.clone());
        self.filter_name_entry.clear();
        self.status = format!("active filter `{}`", filter.name);
        self.install_query_at(filter.tree, filter.active_group);
    }

    fn new_filter(&mut self) {
        self.active_filter = None;
        self.filter_name_entry.clear();
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
            return;
        }
        if filter_bank::get(&new, &self.saved_filters).is_some() {
            self.status = format!("filter `{new}` already exists");
            return;
        }
        self.delete_filter(&old);
        self.upsert_filter(new.clone(), self.query.clone(), self.active_group.clone());
        self.active_filter = Some(new.clone());
        self.filter_name_entry.clear();
        self.status = format!("renamed filter `{old}` → `{new}`");
        self.save_config();
    }

    fn clone_filter(&mut self, name: &FilterName) {
        let Some(filter) = filter_bank::get(name, &self.saved_filters).cloned() else {
            return;
        };
        let name = filter_bank::spare_named(&filter.name, &self.saved_filters);
        self.upsert_filter(
            name.clone(),
            filter.tree.clone(),
            filter.active_group.clone(),
        );
        self.active_filter = Some(name.clone());
        self.filter_name_entry.clear();
        self.status = format!("cloned filter `{name}`");
        self.install_query_at(filter.tree, filter.active_group);
    }

    fn delete_filter(&mut self, name: &FilterName) {
        let Ok(slot) = self
            .saved_filters
            .binary_search_by(|probe| probe.name.cmp(name))
        else {
            return;
        };
        let removed = self.saved_filters.remove(slot);
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
        let filter = SavedFilter::new(name, tree, active_group);
        match self
            .saved_filters
            .binary_search_by(|probe| probe.name.cmp(&filter.name))
        {
            Ok(slot) => self.saved_filters[slot] = filter,
            Err(slot) => self.saved_filters.insert(slot, filter),
        }
    }

    fn queue_clip(&mut self, posts: Vec<PostRecord>) -> usize {
        let posts = posts
            .into_iter()
            .filter(|post| self.clip_inflight.insert(post.id))
            .collect::<Vec<_>>();
        let queued = posts.len();
        if queued == 0 {
            return 0;
        }
        if let Err(err) = self.worker.send(Command::EmbedPosts { posts }) {
            self.status = format!("{err:#}");
        }
        queued
    }

    fn rank_needle(&mut self) -> Option<Embedding> {
        if self.rank_alpha <= 0.0 || self.rank_pins.is_empty() {
            return None;
        }
        let mut embeddings = Vec::<(f32, Embedding)>::new();
        let mut missing = Vec::new();
        for pin in &self.rank_pins {
            match self.index.embedding(pin.post.id) {
                Ok(Some(embedding)) => embeddings.push((f32::from(pin.weight), embedding)),
                Ok(None) => missing.push(pin.post.clone()),
                Err(err) => {
                    self.status = format!("{err:#}");
                    return None;
                }
            }
        }
        if !missing.is_empty() {
            let queued = self.queue_clip(missing);
            if queued > 0 {
                self.status = format!("queued {queued} pinned images for embedding");
            }
        }
        if embeddings.is_empty() {
            return None;
        }
        Embedding::weighted(
            embeddings
                .iter()
                .map(|(weight, embedding)| (*weight, embedding)),
        )
        .map_err(|err| {
            self.status = format!("{err:#}");
        })
        .ok()
    }

    fn pin_weight(&self, id: PostId) -> Option<u8> {
        self.rank_pins
            .iter()
            .find(|pin| pin.post.id == id)
            .map(|pin| pin.weight)
    }

    fn add_pin(&mut self, post: &PostRecord) {
        if let Some(pin) = self.rank_pins.iter_mut().find(|pin| pin.post.id == post.id) {
            pin.weight = pin.weight.saturating_add(1).min(PinConfig::MAX_WEIGHT);
        } else if self.rank_pins.len() < MAX_PINS {
            self.rank_pins.push(RankPin::new(post.clone(), 1));
            if self.rank_alpha <= f32::EPSILON {
                self.rank_alpha = 0.10;
            }
        } else {
            self.status = format!("pin heap is full ({MAX_PINS})");
            return;
        }
        self.save_config();
        self.request_refresh();
    }

    fn weaken_pin(&mut self, id: PostId) {
        let Some(slot) = self.rank_pins.iter().position(|pin| pin.post.id == id) else {
            return;
        };
        if self.rank_pins[slot].weight > PinConfig::MIN_WEIGHT {
            self.rank_pins[slot].weight -= 1;
        } else {
            let _removed = self.rank_pins.remove(slot);
        }
        self.save_config();
        self.request_refresh();
    }

    fn remove_pin(&mut self, id: PostId) {
        self.rank_pins.retain(|pin| pin.post.id != id);
        self.save_config();
        self.request_refresh();
    }

    fn align_warm(&mut self, query: &Query) {
        let key = WarmKey::new(query, self.sort);
        if self.warm_key == key {
            return;
        }
        self.warm_key = key;
        self.warm_next_page = 1;
        self.warm_stride = AUTO_WARM_PAGES;
        self.warm_inflight = false;
        self.warm_exhausted = false;
    }

    fn dispatch_warm(&mut self, query: Query, pages: u32) -> Result<()> {
        self.align_warm(&query);
        if pages == 0 {
            return Ok(());
        }
        self.warm_stride = self.warm_stride.max(pages);
        if self.warm_inflight || self.warm_exhausted {
            return Ok(());
        }
        let first_page = self.warm_next_page;
        if first_page > DANBOORU_SEARCH_PAGE_LIMIT {
            self.warm_exhausted = true;
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
        self.warm_inflight = true;
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
            self.warm_inflight = false;
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
                        self.warm_inflight = false;
                        self.warm_next_page =
                            self.warm_next_page.max(first_page.saturating_add(pages));
                        self.warm_exhausted = exhausted;
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
                    self.request_refresh();
                    self.request_stats();
                    if self.warm_key == event_key && !self.warm_exhausted {
                        let query = self.query();
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
                    self.request_refresh();
                    self.request_stats();
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
                Event::ClipIndexed {
                    ids,
                    stored,
                    faults,
                } => {
                    for id in ids {
                        let _was_inflight = self.clip_inflight.remove(&id);
                    }
                    self.request_refresh();
                    self.request_stats();
                    if faults == 0 {
                        self.status = format!("embedded {stored} Jina CLIP images");
                    } else {
                        self.status =
                            format!("embedded {stored} Jina CLIP images; {faults} faults");
                    }
                    ctx.request_repaint();
                }
                Event::FactsMerged {
                    batches,
                    bytes,
                    groups,
                } => {
                    self.request_stats();
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
        let image = ColorImage::from_rgba_unmultiplied(blade.size, &blade.rgba);
        let texture = ctx.load_texture(
            format!("{}-{}", kind.texture_prefix(), blade.id),
            image,
            TextureOptions::LINEAR,
        );
        match kind {
            BladeKind::Thumb(bucket) => {
                let key = ThumbKey {
                    id: blade.id,
                    bucket,
                };
                let _old = self.thumbs.insert(key, texture);
                let _was_inflight = self.thumb_inflight.remove(&key);
                let _was_faulted = self.thumb_faults.remove(&key);
            }
            BladeKind::Full => {
                let _old_texture = self.full.insert(blade.id, texture);
                let _old_rgba = self.full_rgba.insert(blade.id, blade.clone());
                let _was_inflight = self.full_inflight.remove(&blade.id);
                let _was_faulted = self.full_faults.remove(&blade.id);
            }
        }
        ctx.request_repaint();
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> bool {
        let width = ui.available_width().max(MIN_TILE_EDGE);
        let cols = usize::from(self.images_per_row.max(1));
        let tile = tile_edge(width, cols);
        let posts = self.hit.posts.clone();
        let rows = posts.len().div_ceil(cols);
        let row_height = tile + GAP;
        let mut menu_opened = false;
        let _scroll = egui::ScrollArea::vertical().show_rows(ui, row_height, rows, |ui, range| {
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
        menu_opened
    }

    fn tile(&mut self, ui: &mut egui::Ui, post: &PostRecord, tile: f32) -> bool {
        let mut menu_opened = false;
        let _tile = ui.vertical(|ui| {
            ui.set_width(tile);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(tile, tile), egui::Sense::click());
            match self.thumb(post, tile) {
                Some(ThumbLoad::Ready(texture)) => {
                    let size = fit(texture.size_vec2(), rect.size());
                    let image = egui::Rect::from_center_size(rect.center(), size);
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    let _image = ui
                        .painter()
                        .image(texture.id(), image, uv, egui::Color32::WHITE);
                }
                Some(ThumbLoad::Loading) => paint_tile_text(ui, rect, "loading"),
                Some(ThumbLoad::Fault) => paint_tile_text(ui, rect, "fault"),
                None => paint_tile_text(ui, rect, "no image"),
            }
            if let Some(weight) = self.pin_weight(post.id) {
                paint_pin_badge(ui, rect, weight);
            }
            let pin_clicked = self.pin_hover(ui, post, rect, &response);
            if response.clicked() && !pin_clicked && !self.tag_menu.is_open() {
                self.open_full(post);
            }
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                self.open_tag_menu(post, pos);
                menu_opened = true;
            }
        });
        menu_opened
    }

    fn open_tag_menu(&mut self, post: &PostRecord, anchor: egui::Pos2) {
        self.tag_menu = TagMenu::Open {
            post: Box::new(post.clone()),
            anchor,
        };
    }

    fn pin_hover(
        &mut self,
        ui: &mut egui::Ui,
        post: &PostRecord,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        let pinned = self.pin_weight(post.id).is_some();
        if !pinned && !response.hovered() {
            return false;
        }
        let pin_rect = egui::Rect::from_min_size(
            rect.right_top() + egui::vec2(-34.0, 6.0),
            egui::vec2(28.0, 24.0),
        );
        let pin = ui.interact(
            pin_rect,
            egui::Id::new(("image-pin", post.id.0)),
            egui::Sense::click(),
        );
        let fill = if pinned {
            chrome::RAISED
        } else {
            chrome::CONTROL
        };
        let stroke = if pinned {
            chrome::HOT
        } else {
            chrome::EDGE_STRONG
        };
        let _fill = ui.painter().rect_filled(pin_rect, 0.0, fill);
        let _stroke = ui.painter().rect_stroke(
            pin_rect,
            0.0,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        let _glyph = ui.painter().text(
            pin_rect.center(),
            egui::Align2::CENTER_CENTER,
            "📌",
            egui::TextStyle::Button.resolve(ui.style()),
            chrome::HOT,
        );
        if pin.clicked() {
            self.add_pin(post);
            return true;
        }
        false
    }

    fn tag_palette_overlay(&mut self, ctx: &egui::Context) {
        let Some((post, anchor)) = self.tag_menu.view() else {
            self.tag_menu_rect = None;
            return;
        };
        let post = post.clone();
        let pos = tag_menu_pos(anchor, ctx.content_rect());
        let area = egui::Area::new(egui::Id::new("tag-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let _frame = egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(TAG_MENU_WIDTH);
                    self.tag_palette(ui, &post);
                });
            });
        self.tag_menu_rect = Some(area.response.rect);
    }

    fn tag_palette(&mut self, ui: &mut egui::Ui, post: &PostRecord) {
        let query = self.query();
        let groups = tag_palette::grouped(post, |tag| self.tag_kind(tag));
        let _heading = ui.label(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ));
        let _scroll = egui::ScrollArea::vertical()
            .max_height(TAG_MENU_HEIGHT)
            .show(ui, |ui| {
                for (kind, tags) in groups {
                    let _kind = ui.label(tag_chroma::text(kind.label(), kind).strong());
                    for tag in tags {
                        let active = query.polarity(&tag);
                        let _row = ui.horizontal(|ui| {
                            if ui.small_button("-").clicked() {
                                self.set_tag(tag.as_str(), TagPolarity::Negative);
                            }
                            if active.is_some() && ui.small_button("×").clicked() {
                                self.remove_tag(tag.as_str());
                            } else if active.is_none() {
                                ui.add_space(18.0);
                            }
                            let _tag = ui.label(tag_chroma::text(tag.as_str(), kind));
                            if ui.small_button("+").clicked() {
                                self.set_tag(tag.as_str(), TagPolarity::Positive);
                            }
                        });
                    }
                }
            });
    }

    fn absorb_tag_menu_wheel(&mut self, ctx: &egui::Context) {
        if self.pointer_in_tag_menu(ctx) {
            consume_wheel(ctx);
        }
    }

    fn retain_tag_menu(&mut self, ctx: &egui::Context, menu_opened: bool) {
        if matches!(self.tag_menu, TagMenu::Closed) {
            return;
        }
        let inside = self.pointer_in_tag_menu(ctx);
        let outside_click =
            ctx.input(|input| input.pointer.primary_clicked()) && !inside && !menu_opened;
        if outside_click {
            self.tag_menu = TagMenu::Closed;
            self.tag_menu_rect = None;
        }
    }

    fn pointer_in_tag_menu(&self, ctx: &egui::Context) -> bool {
        let Some(rect) = self.tag_menu_rect else {
            return false;
        };
        ctx.pointer_latest_pos()
            .is_some_and(|pos| rect.expand(2.0).contains(pos))
    }

    fn open_full(&mut self, post: &PostRecord) {
        self.zoom = Some(post.clone());
        self.zoom_gate = ZoomGate::Fresh;
        let _old_fault = self.full_faults.remove(&post.id);
        self.request_full(post);
    }

    fn request_full(&mut self, post: &PostRecord) {
        if self.full.contains_key(&post.id)
            || self.full_inflight.contains(&post.id)
            || self.full_faults.contains(&post.id)
        {
            return;
        }
        let Some(url) = post.full_url().map(ToOwned::to_owned) else {
            let _faulted = self.full_faults.insert(post.id);
            self.status = format!("#{id} has no full image URL", id = post.id);
            return;
        };
        let _now_inflight = self.full_inflight.insert(post.id);
        if let Err(err) = self.worker.send(Command::FullBlade {
            id: post.id,
            url: Some(url),
        }) {
            let _was_inflight = self.full_inflight.remove(&post.id);
            let _faulted = self.full_faults.insert(post.id);
            self.status = format!("{err:#}");
        }
    }

    fn full_frame(&mut self, ctx: &egui::Context) {
        let Some(post) = self.zoom.clone() else {
            return;
        };
        self.request_full(&post);
        let mut close = false;
        let screen = ctx.content_rect();
        let image_box = full_image_box(&post, self.full.get(&post.id), screen.size());
        let body = egui::vec2(image_box.x, image_box.y + VIEWER_CHROME);
        let window = egui::Window::new(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_size(body)
        .collapsible(false)
        .resizable(true)
        .show(ctx, |ui| {
            let _buttons = ui.horizontal(|ui| {
                if ui.button("copy").clicked() {
                    self.copy_full(post.id);
                }
                if ui
                    .add_enabled(post.full_url().is_some(), egui::Button::new("save"))
                    .clicked()
                {
                    self.save_full(&post);
                }
                if ui.button("close").clicked() {
                    close = true;
                }
            });
            if let Some(texture) = self.full.get(&post.id) {
                let response = ui.add(
                    egui::Image::new(texture)
                        .fit_to_exact_size(image_box)
                        .sense(egui::Sense::click()),
                );
                if response.secondary_clicked() {
                    close = true;
                }
            } else if self.full_faults.contains(&post.id) {
                centered_box(ui, image_box, "full image failed");
            } else {
                centered_box(ui, image_box, "loading full image");
            }
        });
        let clicked_outside = window
            .as_ref()
            .is_some_and(|window| outside_click(ctx, window.response.rect));
        if close || (self.zoom_gate == ZoomGate::Armed && clicked_outside) {
            self.zoom = None;
            self.zoom_gate = ZoomGate::Fresh;
        } else {
            self.zoom_gate = ZoomGate::Armed;
        }
    }

    fn save_full(&mut self, post: &PostRecord) {
        let Some(url) = post.full_url().map(ToOwned::to_owned) else {
            self.status = format!("#{id} has no full image URL", id = post.id);
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(save_filename(post, &url))
            .save_file()
        else {
            return;
        };
        if let Err(err) = self.worker.send(Command::SaveMedia {
            id: post.id,
            url: Some(url),
            path,
        }) {
            self.status = format!("{err:#}");
        } else {
            self.status = format!("saving #{id}", id = post.id);
        }
    }

    fn copy_full(&mut self, id: PostId) {
        let Some(blade) = self.full_rgba.get(&id) else {
            "full image is not loaded yet".clone_into(&mut self.status);
            return;
        };
        let result = Clipboard::new()
            .context("open clipboard")
            .and_then(|mut clipboard| {
                clipboard
                    .set_image(ImageData {
                        width: blade.size[0],
                        height: blade.size[1],
                        bytes: Cow::Owned(blade.rgba.clone()),
                    })
                    .context("copy image")
            });
        match result {
            Ok(()) => self.status = format!("copied #{id}"),
            Err(err) => self.status = format!("{err:#}"),
        }
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

    fn save_config(&mut self) {
        let config = Config {
            query: QueryConfig {
                tree: self.query.clone(),
                active_group: self.active_group.clone(),
            },
            filters: FilterConfig {
                active: self.active_filter.clone(),
                saved: self.saved_filters.clone(),
            },
            view: ViewConfig {
                sort: self.sort,
                images_per_row: self.images_per_row,
                tile_scale: None,
            },
            embedding: EmbeddingConfig {
                alpha: self.rank_alpha,
                pins: self.rank_pins.iter().map(RankPin::config).collect(),
                legacy_prompt: String::new(),
            },
        };
        if let Err(err) = config.save(&self.lair.config_path()) {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZoomGate {
    Fresh,
    Armed,
}

#[derive(Clone, Debug)]
struct RankPin {
    post: PostRecord,
    weight: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PinAction {
    Changed,
    Weaken(PostId),
    Remove(PostId),
    Clear,
}

impl RankPin {
    fn new(post: PostRecord, weight: u8) -> Self {
        Self {
            post,
            weight: weight.clamp(PinConfig::MIN_WEIGHT, PinConfig::MAX_WEIGHT),
        }
    }

    fn config(&self) -> PinConfig {
        PinConfig::new(self.post.id, self.weight)
    }
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
            query: query.key(),
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
        env::var_os("BOORU_BAYONET_STARTUP_PROBE").map(|path| Self {
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

fn restore_rank_pins(index: &Index, pins: &[PinConfig]) -> Result<Vec<RankPin>> {
    let mut restored = Vec::new();
    for pin in pins.iter().take(MAX_PINS) {
        if let Some(post) = index.post(pin.id)? {
            restored.push(RankPin::new(post, pin.weight));
        }
    }
    Ok(restored)
}

fn posts_changed(old: &[PostRecord], new: &[PostRecord]) -> bool {
    old.len() != new.len()
        || old
            .iter()
            .zip(new)
            .any(|(old, new)| old.id != new.id || old.thumb_url(360.0) != new.thumb_url(360.0))
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

fn paint_pin_badge(ui: &egui::Ui, rect: egui::Rect, weight: u8) {
    let badge = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 6.0), egui::vec2(34.0, 20.0));
    let _fill = ui.painter().rect_filled(badge, 0.0, chrome::RAISED);
    let _stroke = ui.painter().rect_stroke(
        badge,
        0.0,
        egui::Stroke::new(1.0, chrome::HOT),
        egui::StrokeKind::Inside,
    );
    let _text = ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        format!("📌{weight}"),
        egui::TextStyle::Button.resolve(ui.style()),
        chrome::HOT,
    );
}

fn full_image_box(
    post: &PostRecord,
    texture: Option<&TextureHandle>,
    screen: egui::Vec2,
) -> egui::Vec2 {
    let image = texture.map_or_else(|| post_image_size(post), TextureHandle::size_vec2);
    let bounds = egui::vec2(
        (screen.x * 0.9).max(64.0),
        (screen.y * 0.9 - VIEWER_CHROME).max(64.0),
    );
    fit(image, bounds)
}

fn post_image_size(post: &PostRecord) -> egui::Vec2 {
    if post.width > 0 && post.height > 0 {
        egui::vec2(post.width as f32, post.height as f32)
    } else {
        egui::vec2(720.0, 720.0)
    }
}

fn save_filename(post: &PostRecord, url: &str) -> String {
    format!("danbooru-{}.{}", post.id, extension(url))
}

fn centered_box(ui: &mut egui::Ui, size: egui::Vec2, text: &str) {
    let _box = ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            let _label = ui.label(text);
        },
    );
}

fn outside_click(ctx: &egui::Context, rect: egui::Rect) -> bool {
    ctx.input(|input| {
        input.pointer.any_click()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !rect.contains(pos))
    })
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

impl App for Bayonet {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.zoom_tiles(ctx);
        self.drain(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.paint(ui);
        self.report_startup_probe();
    }
}

impl Bayonet {
    fn paint(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        chrome::install(&ctx);
        let _left = egui::Panel::left("filter")
            .resizable(false)
            .exact_size(chrome::INSPECTOR_WIDTH)
            .show_inside(ui, |ui| self.left_panel(ui));
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
