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
    config::{Config, FilterConfig, FilterName, QueryConfig, SavedFilter, SoftConfig, ViewConfig},
    index::{CacheStats, Index, TagSuggestion},
    media::{MediaCache, RgbaBlade},
    model::{
        BoolOp, Embedding, PostId, PostRecord, Query, QueryAtom, SearchHit, Sort, Tag, TagPolarity,
    },
    query_ui::{QueryAction, render_query_tree},
    tag_menu::{
        HEIGHT as TAG_MENU_HEIGHT, TagMenu, WIDTH as TAG_MENU_WIDTH, position as tag_menu_pos,
    },
    trace::startup,
    worker::{BladeEpoch, Command, Event, Worker},
    xdg::{Lair, compact_path},
};

const RESULT_LIMIT: usize = 360;
const SOFT_POOL: usize = 2_400;
const SOFT_BACKLOG: usize = 128;
const SUGGESTIONS: usize = 12;
const AUTO_WARM_PAGES: u32 = 1;
const DANBOORU_SEARCH_PAGE_LIMIT: u32 = 1_000;
const BASE_TILE: f32 = 260.0;
const MIN_TILE_SCALE: f32 = 0.5;
const MAX_TILE_SCALE: f32 = 3.0;
const GAP: f32 = 8.0;
const VIEWER_CHROME: f32 = 40.0;

pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query: Query,
    active_group: Vec<usize>,
    tag_entry: String,
    filter_name_entry: String,
    saved_filters: Vec<SavedFilter>,
    soft_text: String,
    soft_alpha: f32,
    soft_prompt: Option<String>,
    soft_embedding: Option<Embedding>,
    soft_requested: Option<String>,
    sort: Sort,
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
    tile_scale: f32,
    tag_menu: TagMenu,
    tag_menu_rect: Option<egui::Rect>,
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
        let query = config.query.query();
        let sort = config.view.sort;
        let active_group = query.clamp_group_path(&config.query.active_group);
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
            saved_filters: sorted_filters(config.filters.saved.clone()),
            soft_text: config.soft.prompt.clone(),
            soft_alpha: config.soft.alpha.clamp(0.0, 2.0),
            soft_prompt: None,
            soft_embedding: None,
            soft_requested: None,
            sort,
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
            tile_scale: config.view.tile_scale.clamp(MIN_TILE_SCALE, MAX_TILE_SCALE),
            tag_menu: TagMenu::Closed,
            tag_menu_rect: None,
            clip_inflight: HashSet::new(),
            cache_status: "cache measuring".to_owned(),
            warm_status: "query warm idle".to_owned(),
            startup_probe: StartupProbe::from_env(),
        };
        startup("app.state.built");
        app.reap(true, AUTO_WARM_PAGES)?;
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

    fn reap(&mut self, warm: bool, pages: u32) -> Result<()> {
        startup("app.reap.enter");
        let query = self.query();
        let soft = self.soft_needle().cloned();
        if let Some(needle) = soft {
            startup("app.reap.soft.search.enter");
            let hit = self.index.search_soft(
                &query,
                self.sort,
                &needle,
                self.soft_alpha,
                RESULT_LIMIT,
                SOFT_POOL,
                SOFT_BACKLOG,
            )?;
            startup("app.reap.soft.search.done");
            let queued = self.queue_clip(hit.missing);
            let posts = hit.hit.posts.len();
            let candidates = hit.hit.candidates;
            let embedded = hit.embedded;
            let pool = hit.pool;
            self.install_hit(hit.hit);
            self.status = format!(
                "{} hits from {} candidates; clip {}/{} embedded, queued {}; α {:.2}",
                posts, candidates, embedded, pool, queued, self.soft_alpha
            );
        } else {
            startup("app.reap.search.enter");
            let hit = self.index.search(&query, self.sort, RESULT_LIMIT)?;
            startup("app.reap.search.done");
            let soft_armed = self.soft_prompt().is_some();
            let queued = if soft_armed {
                self.queue_clip(hit.posts.clone())
            } else {
                0
            };
            let posts = hit.posts.len();
            let candidates = hit.candidates;
            self.install_hit(hit);
            let requested = self.request_soft_prompt();
            self.status = if soft_armed {
                format!(
                    "{} hits from {} candidates; clip text {}; queued {} visible images",
                    posts,
                    candidates,
                    if requested { "requested" } else { "pending" },
                    queued
                )
            } else {
                format!(
                    "{} hits from {} candidates; {}",
                    posts,
                    candidates,
                    compact_path(&self.lair.data)
                )
            };
        }
        if warm {
            startup("app.reap.warm.enter");
            self.dispatch_warm(query, pages)?;
            startup("app.reap.warm.done");
        }
        startup("app.reap.stats.enter");
        self.update_cache_status();
        startup("app.reap.stats.done");
        Ok(())
    }

    fn query(&self) -> Query {
        self.query.clone()
    }

    fn install_query(&mut self, query: Query) {
        self.install_query_at(query, self.active_group.clone());
    }

    fn install_query_at(&mut self, query: Query, active_group: Vec<usize>) {
        self.active_group = query.clamp_group_path(&active_group);
        self.query = query;
        self.advance_thumb_epoch();
        let query = self.query.clone();
        self.align_warm(&query);
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

    fn save_current_filter(&mut self) {
        let typed = FilterName::forge(&self.filter_name_entry);
        let name = typed
            .clone()
            .unwrap_or_else(|| spare_filter_name(&self.query, &self.saved_filters));
        let filter = SavedFilter::new(name.clone(), self.query.clone(), self.active_group.clone());
        match self
            .saved_filters
            .binary_search_by(|probe| probe.name.cmp(&filter.name))
        {
            Ok(slot) => self.saved_filters[slot] = filter,
            Err(slot) => self.saved_filters.insert(slot, filter),
        }
        self.filter_name_entry.clear();
        self.status = format!("saved filter `{name}`");
        self.save_config();
    }

    fn load_filter(&mut self, filter: SavedFilter) {
        self.filter_name_entry = filter.name.to_string();
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
        self.status = format!("deleted filter `{}`", removed.name);
        self.save_config();
    }

    fn soft_prompt(&self) -> Option<String> {
        let prompt = self.soft_text.trim();
        (self.soft_alpha > 0.0 && !prompt.is_empty()).then(|| prompt.to_owned())
    }

    fn soft_needle(&self) -> Option<&Embedding> {
        let prompt = self.soft_prompt()?;
        (self.soft_prompt.as_deref() == Some(prompt.as_str()))
            .then_some(self.soft_embedding.as_ref())
            .flatten()
    }

    fn request_soft_prompt(&mut self) -> bool {
        let Some(prompt) = self.soft_prompt() else {
            return false;
        };
        if self.soft_prompt.as_deref() == Some(prompt.as_str())
            || self.soft_requested.as_deref() == Some(prompt.as_str())
        {
            return false;
        }
        self.soft_requested = Some(prompt.clone());
        if let Err(err) = self.worker.send(Command::SoftText { prompt }) {
            self.status = format!("{err:#}");
            return false;
        }
        true
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

    fn strike(&mut self, warm: bool, pages: u32) {
        if let Err(err) = self.reap(warm, pages) {
            self.status = format!("{err:#}");
        }
    }

    fn update_cache_status(&mut self) {
        match self.index.stats() {
            Ok(stats) => self.cache_status = cache_status(&stats),
            Err(err) => self.cache_status = format!("cache stats fault: {err:#}"),
        }
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
        let events = self.worker.drain().collect::<Vec<_>>();
        for event in events {
            match event {
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
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
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
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
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
                Event::SoftText { prompt, embedding } => {
                    if self.soft_requested.as_deref() == Some(prompt.as_str()) {
                        self.soft_requested = None;
                    }
                    if self.soft_prompt().as_deref() == Some(prompt.as_str()) {
                        self.soft_prompt = Some(prompt);
                        self.soft_embedding = Some(embedding);
                        if let Err(err) = self.reap(false, 0) {
                            self.status = format!("{err:#}");
                        }
                    }
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
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    } else if faults == 0 {
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
                    self.update_cache_status();
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

    fn top(&mut self, ui: &mut egui::Ui) {
        let _bar = ui.horizontal(|ui| {
            for sort in Sort::ALL {
                if ui
                    .selectable_label(self.sort == sort, sort.label())
                    .clicked()
                {
                    self.sort = sort;
                    self.save_config();
                    self.strike(true, AUTO_WARM_PAGES);
                }
            }

            if ui.button("warm +200").clicked() {
                self.strike(true, 1);
            }
            if ui.button("ransack +1000").clicked() {
                self.strike(true, 5);
            }
            let _zoom = ui.label(format!("thumb {:.0}px", self.tile_edge()));
        });
        let _soft = ui.horizontal(|ui| {
            let _label = ui.label("soft");
            let prompt = ui.text_edit_singleline(&mut self.soft_text);
            if prompt.changed() {
                self.save_config();
                self.strike(false, 0);
            }
            let slider = egui::Slider::new(&mut self.soft_alpha, 0.0..=2.0)
                .text("clip α")
                .fixed_decimals(2);
            if ui.add(slider).changed() {
                self.save_config();
                self.strike(false, 0);
            }
            if ui.button("embed visible").clicked() {
                let queued = self.queue_clip(self.hit.posts.clone());
                self.status = format!("queued {queued} visible images for Jina CLIP");
            }
        });
        let _label = ui.label(format!(
            "{}; {}; {}; {}",
            self.status, self.cache_status, self.warm_status, self.crawl_status
        ));
    }

    fn autocomplete(&mut self, ui: &mut egui::Ui) {
        let Some(prefix) = active_prefix(&self.tag_entry) else {
            return;
        };
        let suggestions = match self.index.tag_suggestions(&prefix.body, SUGGESTIONS) {
            Ok(suggestions) => suggestions,
            Err(err) => {
                self.status = format!("{err:#}");
                return;
            }
        };
        if suggestions.is_empty() {
            return;
        }
        let _row = ui.horizontal_wrapped(|ui| {
            let _label = ui.label("complete");
            for suggestion in suggestions {
                if ui
                    .small_button(format!("{} ({})", suggestion.tag, suggestion.posts))
                    .clicked()
                {
                    self.complete_active(&suggestion, prefix.negative);
                }
            }
        });
    }

    fn complete_active(&mut self, suggestion: &TagSuggestion, negative: bool) {
        let polarity = if negative {
            TagPolarity::Negative
        } else {
            TagPolarity::Positive
        };
        if let Some(tag) = Tag::forge(&suggestion.tag) {
            self.add_atom(QueryAtom::Tag(tag), polarity);
        }
        self.tag_entry.clear();
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let query = self.query.clone();
        let active_group = self.active_group.clone();
        let mut actions = Vec::new();
        let _heading = ui.heading("filter");
        let entry = ui.text_edit_singleline(&mut self.tag_entry);
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter && (entry.has_focus() || entry.lost_focus()) {
            self.commit_tag_entry();
        }
        self.autocomplete(ui);
        let _hint =
            ui.label("enter targets the highlighted group; -foo inserts NOT foo; rating:q works.");
        let _separator = ui.separator();
        if query.is_empty() {
            let _empty = ui.label("neutral");
        }
        render_query_tree(ui, query.root(), &active_group, &mut actions);
        let _separator = ui.separator();
        let _active = ui.horizontal_wrapped(|ui| {
            let _label = ui.label("active");
            for op in BoolOp::ALL {
                let selected = self
                    .query
                    .group(&self.active_group)
                    .is_some_and(|group| group.op == op);
                if ui.selectable_label(selected, op.label()).clicked() {
                    actions.push(QueryAction::SetOp {
                        path: self.active_group.clone(),
                        op,
                    });
                }
            }
            if ui.button("add group").clicked() {
                actions.push(QueryAction::AddGroup { op: BoolOp::And });
            }
            if ui.button("NOT active").clicked() {
                actions.push(QueryAction::ToggleNot {
                    path: self.active_group.clone(),
                });
            }
        });
        self.apply_query_actions(actions);
        let _separator = ui.separator();
        self.saved_filter_panel(ui);
        let _cache = ui.label(&self.cache_status);
    }

    fn saved_filter_panel(&mut self, ui: &mut egui::Ui) {
        let _heading = ui.heading("saved");
        let mut actions = Vec::new();
        let _save = ui.horizontal(|ui| {
            let entry = ui.add(
                egui::TextEdit::singleline(&mut self.filter_name_entry).hint_text("filter name"),
            );
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            if ui.button("save").clicked() || (entry.has_focus() && enter) {
                actions.push(SavedFilterAction::Save);
            }
        });
        if self.saved_filters.is_empty() {
            let _empty = ui.label("none");
        }
        for filter in &self.saved_filters {
            let _row = ui.horizontal_wrapped(|ui| {
                if ui.small_button("×").clicked() {
                    actions.push(SavedFilterAction::Delete(filter.name.clone()));
                }
                if ui.button(filter.name.as_str()).clicked() {
                    actions.push(SavedFilterAction::Load(filter.clone()));
                }
            });
        }
        for action in actions {
            match action {
                SavedFilterAction::Save => self.save_current_filter(),
                SavedFilterAction::Load(filter) => self.load_filter(filter),
                SavedFilterAction::Delete(name) => self.delete_filter(&name),
            }
        }
    }

    fn commit_tag_entry(&mut self) {
        let terms = Query::parse_terms(&self.tag_entry);
        if terms.is_empty() {
            return;
        }
        let mut query = self.query.clone();
        for term in terms {
            let _inserted = query.push_atom(&self.active_group, term.atom, term.polarity);
        }
        self.tag_entry.clear();
        self.install_query(query);
    }

    fn apply_query_actions(&mut self, actions: Vec<QueryAction>) {
        for action in actions {
            self.apply_query_action(action);
        }
    }

    fn apply_query_action(&mut self, action: QueryAction) {
        match action {
            QueryAction::Select { path } => {
                self.active_group = self.query.clamp_group_path(&path);
                self.save_config();
            }
            QueryAction::SetOp { path, op } => {
                let mut query = self.query.clone();
                if query.set_group_op(&path, op) {
                    self.install_query(query);
                }
            }
            QueryAction::ToggleNot { path } => {
                let mut query = self.query.clone();
                if query.toggle_not(&path) {
                    self.install_query(query);
                }
            }
            QueryAction::RemoveChild { parent, child } => {
                let mut query = self.query.clone();
                if query.remove_child(&parent, child) {
                    self.install_query_at(query, parent);
                }
            }
            QueryAction::AddGroup { op } => {
                let mut query = self.query.clone();
                if let Some(path) = query.push_group(&self.active_group, op) {
                    self.install_query_at(query, path);
                }
            }
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> bool {
        let tile = self.tile_edge();
        let width = ui.available_width().max(tile);
        let cols = ((width + GAP) / (tile + GAP)).floor().max(1.0) as usize;
        let posts = self.hit.posts.clone();
        let rows = posts.len().div_ceil(cols);
        let row_height = tile + GAP;
        let mut tag_source = false;
        let _scroll = egui::ScrollArea::vertical().show_rows(ui, row_height, rows, |ui, range| {
            for row in range {
                let start = row * cols;
                let end = (start + cols).min(posts.len());
                let _row = ui.horizontal(|ui| {
                    for post in &posts[start..end] {
                        tag_source |= self.tile(ui, post);
                    }
                });
            }
        });
        tag_source
    }

    fn tile(&mut self, ui: &mut egui::Ui, post: &PostRecord) -> bool {
        let tile = self.tile_edge();
        let mut tag_source = false;
        let _tile = ui.vertical(|ui| {
            ui.set_width(tile);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(tile, tile), egui::Sense::click());
            match self.thumb(post) {
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
            if response.clicked() {
                self.open_full(post);
            }
            if let Some(pos) = response.hover_pos() {
                self.open_tag_menu(post, pos);
                tag_source = true;
            }
        });
        tag_source
    }

    fn open_tag_menu(&mut self, post: &PostRecord, anchor: egui::Pos2) {
        if self.tag_menu.post_id() == Some(post.id) {
            return;
        }
        self.tag_menu = TagMenu::Open {
            post: Box::new(post.clone()),
            anchor,
        };
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
        let _heading = ui.label(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ));
        let _separator = ui.separator();
        let _scroll = egui::ScrollArea::vertical()
            .max_height(TAG_MENU_HEIGHT)
            .show(ui, |ui| {
                for tag in &post.tags {
                    let active = query.polarity(tag);
                    let _row = ui.horizontal(|ui| {
                        if ui.small_button("-").clicked() {
                            self.set_tag(tag.as_str(), TagPolarity::Negative);
                        }
                        if active.is_some() && ui.small_button("×").clicked() {
                            self.remove_tag(tag.as_str());
                        } else if active.is_none() {
                            ui.add_space(18.0);
                        }
                        let _tag = ui.label(tag.as_str());
                        if ui.small_button("+").clicked() {
                            self.set_tag(tag.as_str(), TagPolarity::Positive);
                        }
                    });
                }
            });
    }

    fn absorb_tag_menu_wheel(&mut self, ctx: &egui::Context) {
        if self.pointer_in_tag_menu(ctx) {
            consume_wheel(ctx);
        }
    }

    fn retain_tag_menu(&mut self, ctx: &egui::Context, tag_source: bool) {
        if matches!(self.tag_menu, TagMenu::Closed) {
            return;
        }
        let inside = self.pointer_in_tag_menu(ctx);
        let outside_click = ctx.input(|input| input.pointer.any_click()) && !inside && !tag_source;
        if outside_click || (!inside && !tag_source) {
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

    fn thumb(&mut self, post: &PostRecord) -> Option<ThumbLoad<'_>> {
        let bucket = thumb_bucket(self.tile_edge());
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
            let url = post.thumb_url(self.tile_edge()).map(ToOwned::to_owned)?;
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

    fn tile_edge(&self) -> f32 {
        BASE_TILE * self.tile_scale
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
        self.tile_scale =
            (self.tile_scale * 1.12_f32.powf(steps)).clamp(MIN_TILE_SCALE, MAX_TILE_SCALE);
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
                saved: self.saved_filters.clone(),
            },
            view: ViewConfig {
                sort: self.sort,
                tile_scale: self.tile_scale,
            },
            soft: SoftConfig {
                prompt: self.soft_text.clone(),
                alpha: self.soft_alpha,
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

#[derive(Clone, Debug)]
enum SavedFilterAction {
    Save,
    Load(SavedFilter),
    Delete(FilterName),
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

fn thumb_bucket(edge: f32) -> u8 {
    if edge > 390.0 {
        2
    } else {
        u8::from(edge > 190.0)
    }
}

fn sorted_filters(mut filters: Vec<SavedFilter>) -> Vec<SavedFilter> {
    filters.sort_by(|a, b| a.name.cmp(&b.name));
    filters.dedup_by(|a, b| a.name == b.name);
    filters
}

fn spare_filter_name(query: &Query, filters: &[SavedFilter]) -> FilterName {
    let base = FilterName::forge(&filter_stem(query)).unwrap_or_else(FilterName::neutral);
    if !filter_name_taken(&base, filters) {
        return base;
    }
    let mut suffix = 2_u64;
    loop {
        let raw = format!("{} {suffix}", base.as_str());
        if let Some(candidate) = FilterName::forge(&raw)
            && !filter_name_taken(&candidate, filters)
        {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

fn filter_stem(query: &Query) -> String {
    let text = query.to_text();
    let text = if text.is_empty() {
        "neutral".to_owned()
    } else {
        text
    };
    clip_chars(&text, 48)
}

fn clip_chars(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut out = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn filter_name_taken(name: &FilterName, filters: &[SavedFilter]) -> bool {
    filters
        .binary_search_by(|filter| filter.name.cmp(name))
        .is_ok()
}

fn cache_status(stats: &CacheStats) -> String {
    let ratings = stats
        .ratings
        .iter()
        .map(|(rating, posts)| format!("{}:{posts}", rating.key()))
        .collect::<Vec<_>>()
        .join("/");
    let frontier = match (stats.crawl_before, stats.rough_crawl_percent()) {
        (Some(before), Some(percent)) => format!("crawl≤#{before} ≈{percent:.1}% ID"),
        (Some(before), None) => format!("crawl≤#{before}"),
        (None, _) => "crawl unstarted".to_owned(),
    };
    let newest = stats
        .newest
        .map_or_else(|| "newest unknown".to_owned(), |id| format!("newest #{id}"));
    format!(
        "cache {} posts, {} tag chunks, {} clip, {} pending fact batches, ratings {ratings}, {newest}, {frontier}",
        stats.posts, stats.tag_chunks, stats.embeddings, stats.pending_fact_batches
    )
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
        let _edge = egui::Panel::top("edge").show_inside(ui, |ui| self.top(ui));
        let _left = egui::Panel::left("filter")
            .resizable(true)
            .default_size(220.0)
            .show_inside(ui, |ui| self.left_panel(ui));
        let prior = self.tag_menu.post_id();
        self.tag_menu_rect = None;
        self.tag_palette_overlay(&ctx);
        self.absorb_tag_menu_wheel(&ctx);
        let mut tag_source = false;
        let _center = egui::CentralPanel::default().show_inside(ui, |ui| {
            tag_source = self.grid(ui);
        });
        if self.tag_menu.post_id() != prior {
            self.tag_menu_rect = None;
            self.tag_palette_overlay(&ctx);
            ctx.request_repaint();
        }
        self.retain_tag_menu(&ctx, tag_source);
        self.full_frame(&ctx);
    }
}
