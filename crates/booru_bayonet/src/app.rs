use anyhow::{Context as _, Result};
use arboard::{Clipboard, ImageData};
use eframe::{
    App, CreationContext,
    egui::{self, ColorImage, TextureHandle, TextureOptions},
};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use crate::{
    config::{Config, QueryConfig, SoftConfig, ViewConfig},
    index::{CacheStats, Index, TagSuggestion},
    media::{MediaCache, RgbaBlade},
    model::{
        BoolGroup, BoolOp, Embedding, PostId, PostRecord, Query, QueryAtom, QueryExpr, SearchHit,
        Sort, Tag, TagPolarity,
    },
    worker::{Command, Event, Worker},
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

pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query: Query,
    active_group: Vec<usize>,
    tag_entry: String,
    soft_text: String,
    soft_alpha: f32,
    soft_prompt: Option<String>,
    soft_embedding: Option<Embedding>,
    soft_requested: Option<String>,
    sort: Sort,
    hit: SearchHit,
    thumbs: HashMap<ThumbKey, TextureHandle>,
    thumb_inflight: HashSet<ThumbKey>,
    warm_key: WarmKey,
    warm_next_page: u32,
    warm_stride: u32,
    warm_inflight: bool,
    warm_exhausted: bool,
    full: HashMap<PostId, TextureHandle>,
    full_rgba: HashMap<PostId, RgbaBlade>,
    full_inflight: HashSet<PostId>,
    zoom: Option<PostRecord>,
    tile_scale: f32,
    tag_menu_open: bool,
    clip_inflight: HashSet<PostId>,
    cache_status: String,
    warm_status: String,
    crawl_status: String,
    status: String,
}

impl Bayonet {
    pub fn new(_cc: &CreationContext<'_>) -> Result<Self> {
        let lair = Lair::claim()?;
        let config = Config::load(&lair.config_path())?;
        let index = Index::open(&lair.index_path())?;
        let media = MediaCache::new(lair.media_dir())?;
        let worker = Worker::spawn(index.clone(), media, lair.model_dir());
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
            soft_text: config.soft.prompt.clone(),
            soft_alpha: config.soft.alpha.clamp(0.0, 2.0),
            soft_prompt: None,
            soft_embedding: None,
            soft_requested: None,
            sort,
            hit: SearchHit::default(),
            thumbs: HashMap::new(),
            thumb_inflight: HashSet::new(),
            warm_key: WarmKey::new(&query, sort),
            warm_next_page: 1,
            warm_stride: AUTO_WARM_PAGES,
            warm_inflight: false,
            warm_exhausted: false,
            full: HashMap::new(),
            full_rgba: HashMap::new(),
            full_inflight: HashSet::new(),
            zoom: None,
            tile_scale: config.view.tile_scale.clamp(MIN_TILE_SCALE, MAX_TILE_SCALE),
            tag_menu_open: false,
            clip_inflight: HashSet::new(),
            cache_status: "cache measuring".to_owned(),
            warm_status: "query warm idle".to_owned(),
        };
        app.reap(true, AUTO_WARM_PAGES)?;
        app.worker.backfill_ratings(app.index.clone());
        Ok(app)
    }

    fn reap(&mut self, warm: bool, pages: u32) -> Result<()> {
        let query = self.query();
        let soft = self.soft_needle().cloned();
        if let Some(needle) = soft {
            let hit = self.index.search_soft(
                &query,
                self.sort,
                &needle,
                self.soft_alpha,
                RESULT_LIMIT,
                SOFT_POOL,
                SOFT_BACKLOG,
            )?;
            let queued = self.queue_clip(hit.missing);
            self.status = format!(
                "{} hits from {} candidates; clip {}/{} embedded, queued {}; α {:.2}",
                hit.hit.posts.len(),
                hit.hit.candidates,
                hit.embedded,
                hit.pool,
                queued,
                self.soft_alpha
            );
            self.hit = hit.hit;
        } else {
            self.hit = self.index.search(&query, self.sort, RESULT_LIMIT)?;
            let soft_armed = self.soft_prompt().is_some();
            let queued = if soft_armed {
                self.queue_clip(self.hit.posts.clone())
            } else {
                0
            };
            let requested = self.request_soft_prompt();
            self.status = if soft_armed {
                format!(
                    "{} hits from {} candidates; clip text {}; queued {} visible images",
                    self.hit.posts.len(),
                    self.hit.candidates,
                    if requested { "requested" } else { "pending" },
                    queued
                )
            } else {
                format!(
                    "{} hits from {} candidates; {}",
                    self.hit.posts.len(),
                    self.hit.candidates,
                    compact_path(&self.lair.data)
                )
            };
        }
        if warm {
            self.dispatch_warm(query, pages)?;
        }
        self.update_cache_status();
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
        let query = self.query.clone();
        self.align_warm(&query);
        self.save_config();
        self.strike(true, AUTO_WARM_PAGES);
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
                Event::FullBlade(blade) => {
                    self.install_blade(ctx, blade, BladeKind::Full);
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
                Event::RatingBackfilled { posts } => {
                    self.update_cache_status();
                    self.status = format!("backfilled rating lane over {posts} cached posts");
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
                    ctx.request_repaint();
                }
                Event::UnindexablePurged { tag, posts } => {
                    self.update_cache_status();
                    self.status = format!("purged {posts} `{tag}` posts from index");
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
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
            }
            BladeKind::Full => {
                let _old_texture = self.full.insert(blade.id, texture);
                let _old_rgba = self.full_rgba.insert(blade.id, blade.clone());
                let _was_inflight = self.full_inflight.remove(&blade.id);
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
        let _cache = ui.label(&self.cache_status);
        self.apply_query_actions(actions);
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

    fn grid(&mut self, ui: &mut egui::Ui) {
        let tile = self.tile_edge();
        let width = ui.available_width().max(tile);
        let cols = ((width + GAP) / (tile + GAP)).floor().max(1.0) as usize;
        let posts = self.hit.posts.clone();
        let _scroll = egui::ScrollArea::vertical().show(ui, |ui| {
            for row in posts.chunks(cols) {
                let _row = ui.horizontal(|ui| {
                    for post in row {
                        self.tile(ui, post);
                    }
                });
            }
        });
    }

    fn tile(&mut self, ui: &mut egui::Ui, post: &PostRecord) {
        let tile = self.tile_edge();
        let _tile = ui.vertical(|ui| {
            ui.set_width(tile);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(tile, tile), egui::Sense::click());
            if let Some(texture) = self.thumb(post) {
                let size = fit(texture.size_vec2(), rect.size());
                let image = egui::Rect::from_center_size(rect.center(), size);
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                let _image = ui
                    .painter()
                    .image(texture.id(), image, uv, egui::Color32::WHITE);
            } else {
                let _loading = ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "loading",
                    egui::TextStyle::Body.resolve(ui.style()),
                    ui.visuals().text_color(),
                );
            }
            if response.clicked() {
                self.open_full(post);
            }
            let _hover = response.on_hover_ui(|ui| self.tag_palette(ui, post));
        });
    }

    fn tag_palette(&mut self, ui: &mut egui::Ui, post: &PostRecord) {
        self.tag_menu_open = true;
        let query = self.query();
        let _heading = ui.label(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ));
        let _separator = ui.separator();
        let _scroll = egui::ScrollArea::vertical()
            .max_height(360.0)
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
        consume_wheel(ui.ctx());
    }

    fn open_full(&mut self, post: &PostRecord) {
        self.zoom = Some(post.clone());
        self.request_full(post);
    }

    fn request_full(&mut self, post: &PostRecord) {
        if self.full.contains_key(&post.id) || self.full_inflight.contains(&post.id) {
            return;
        }
        let _now_inflight = self.full_inflight.insert(post.id);
        if let Err(err) = self.worker.send(Command::FullBlade {
            id: post.id,
            url: post.full_url().map(ToOwned::to_owned),
        }) {
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
        let _window = egui::Window::new(format!(
            "#{}  score {}  fav {}",
            post.id, post.score, post.favs
        ))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_size(screen.size() * 0.9)
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
                let size = fit(texture.size_vec2(), ui.available_size());
                let response = ui.add(
                    egui::Image::new(texture)
                        .fit_to_exact_size(size)
                        .sense(egui::Sense::click()),
                );
                if response.secondary_clicked() {
                    close = true;
                }
            } else {
                let _loading = ui.centered_and_justified(|ui| {
                    let _label = ui.label("loading full image");
                });
            }
        });
        if close {
            self.zoom = None;
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

    fn thumb(&mut self, post: &PostRecord) -> Option<&TextureHandle> {
        let bucket = thumb_bucket(self.tile_edge());
        let key = ThumbKey {
            id: post.id,
            bucket,
        };
        if !self.thumbs.contains_key(&key) && !self.thumb_inflight.contains(&key) {
            let _now_inflight = self.thumb_inflight.insert(key);
            if let Err(err) = self.worker.send(Command::Blade {
                id: post.id,
                bucket,
                url: post.thumb_url(self.tile_edge()).map(ToOwned::to_owned),
            }) {
                self.status = format!("{err:#}");
            }
        }
        self.thumbs.get(&key)
    }

    fn tile_edge(&self) -> f32 {
        BASE_TILE * self.tile_scale
    }

    fn zoom_tiles(&mut self, ctx: &egui::Context) {
        if self.tag_menu_open {
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
        self.save_config();
        ctx.request_repaint();
    }

    fn save_config(&mut self) {
        let config = Config {
            query: QueryConfig {
                tree: self.query.clone(),
                active_group: self.active_group.clone(),
                include: Vec::new(),
                exclude: Vec::new(),
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
}

#[derive(Clone, Copy)]
enum BladeKind {
    Thumb(u8),
    Full,
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

#[derive(Clone, Debug)]
enum QueryAction {
    Select { path: Vec<usize> },
    SetOp { path: Vec<usize>, op: BoolOp },
    ToggleNot { path: Vec<usize> },
    RemoveChild { parent: Vec<usize>, child: usize },
    AddGroup { op: BoolOp },
}

fn render_query_tree(
    ui: &mut egui::Ui,
    root: &QueryExpr,
    active: &[usize],
    actions: &mut Vec<QueryAction>,
) {
    let mut path = Vec::new();
    render_query_expr(ui, root, &mut path, None, active, 0, actions);
}

fn render_query_expr(
    ui: &mut egui::Ui,
    expr: &QueryExpr,
    path: &mut Vec<usize>,
    parent: Option<(Vec<usize>, usize)>,
    active: &[usize],
    depth: usize,
    actions: &mut Vec<QueryAction>,
) {
    let (negated, core) = expr.denote();
    match core {
        QueryExpr::Atom { atom } => render_atom(ui, atom, negated, parent, actions),
        QueryExpr::Group { group } => {
            render_group(ui, group, negated, path, parent, active, depth, actions);
        }
        QueryExpr::Not { child } => {
            render_query_expr(ui, child, path, parent, active, depth, actions);
        }
    }
}

fn render_group(
    ui: &mut egui::Ui,
    group: &BoolGroup,
    negated: bool,
    path: &mut Vec<usize>,
    parent: Option<(Vec<usize>, usize)>,
    active: &[usize],
    depth: usize,
    actions: &mut Vec<QueryAction>,
) {
    let active_here = path.as_slice() == active;
    let frame = egui::Frame::group(ui.style())
        .fill(group_fill(depth))
        .stroke(group_stroke(depth, active_here));
    let _frame = frame.show(ui, |ui| {
        let _header = ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(active_here, group_title(path))
                .clicked()
            {
                actions.push(QueryAction::Select { path: path.clone() });
            }
            if ui.selectable_label(negated, "NOT").clicked() {
                actions.push(QueryAction::ToggleNot { path: path.clone() });
            }
            for op in BoolOp::ALL {
                if ui.selectable_label(group.op == op, op.label()).clicked() {
                    actions.push(QueryAction::SetOp {
                        path: path.clone(),
                        op,
                    });
                }
            }
            if let Some((parent, child)) = parent.as_ref()
                && ui.small_button("×").clicked()
            {
                actions.push(QueryAction::RemoveChild {
                    parent: parent.clone(),
                    child: *child,
                });
            }
        });
        if group.children.is_empty() {
            let _empty = ui.label("empty");
        }
        for (child, expr) in group.children.iter().enumerate() {
            let parent_path = path.clone();
            path.push(child);
            render_query_expr(
                ui,
                expr,
                path,
                Some((parent_path, child)),
                active,
                depth + 1,
                actions,
            );
            let _old = path.pop();
        }
    });
}

fn render_atom(
    ui: &mut egui::Ui,
    atom: &QueryAtom,
    negated: bool,
    parent: Option<(Vec<usize>, usize)>,
    actions: &mut Vec<QueryAction>,
) {
    let text = if negated {
        format!("¬ {atom}")
    } else {
        format!("+ {atom}")
    };
    let color = if negated {
        egui::Color32::from_rgb(218, 150, 146)
    } else {
        egui::Color32::from_rgb(156, 204, 176)
    };
    let _row = ui.horizontal(|ui| {
        if let Some((parent, child)) = parent
            && ui.small_button("×").clicked()
        {
            actions.push(QueryAction::RemoveChild { parent, child });
        }
        let _label = ui.label(egui::RichText::new(text).color(color));
    });
}

fn group_title(path: &[usize]) -> String {
    if path.is_empty() {
        "root".to_owned()
    } else {
        format!(
            "g{}",
            path.iter()
                .map(|slot| (slot + 1).to_string())
                .collect::<Vec<_>>()
                .join(".")
        )
    }
}

fn group_fill(depth: usize) -> egui::Color32 {
    const PALETTE: [(u8, u8, u8); 6] = [
        (86, 105, 143),
        (107, 128, 104),
        (135, 111, 83),
        (116, 93, 132),
        (84, 128, 133),
        (137, 91, 98),
    ];
    let (r, g, b) = PALETTE[depth % PALETTE.len()];
    egui::Color32::from_rgba_unmultiplied(r, g, b, 28)
}

fn group_stroke(depth: usize, active: bool) -> egui::Stroke {
    const PALETTE: [(u8, u8, u8); 6] = [
        (135, 161, 214),
        (155, 188, 148),
        (198, 164, 118),
        (174, 145, 202),
        (125, 190, 198),
        (203, 136, 145),
    ];
    let (r, g, b) = PALETTE[depth % PALETTE.len()];
    if active {
        egui::Stroke::new(2.0, egui::Color32::from_rgb(r, g, b))
    } else {
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(r, g, b, 96))
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

fn cache_status(stats: &CacheStats) -> String {
    let ratings = stats
        .ratings
        .iter()
        .map(|(rating, posts)| format!("{}:{posts}", rating.key()))
        .collect::<Vec<_>>()
        .join("/");
    let rating_state = if stats.rating_indexed {
        "ratings ready"
    } else {
        "ratings indexing"
    };
    let frontier = match (stats.crawl_before, stats.rough_crawl_percent()) {
        (Some(before), Some(percent)) => format!("crawl≤#{before} ≈{percent:.1}% ID"),
        (Some(before), None) => format!("crawl≤#{before}"),
        (None, _) => "crawl unstarted".to_owned(),
    };
    let newest = stats
        .newest
        .map_or_else(|| "newest unknown".to_owned(), |id| format!("newest #{id}"));
    format!(
        "cache {} posts, {} tags, {} clip, {rating_state} {ratings}, {newest}, {frontier}",
        stats.posts, stats.tags, stats.embeddings
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
        self.tag_menu_open = false;
        let _edge = egui::Panel::top("edge").show_inside(ui, |ui| self.top(ui));
        let _left = egui::Panel::left("filter")
            .resizable(true)
            .default_size(220.0)
            .show_inside(ui, |ui| self.left_panel(ui));
        let _center = egui::CentralPanel::default().show_inside(ui, |ui| self.grid(ui));
        self.full_frame(ui.ctx());
    }
}
