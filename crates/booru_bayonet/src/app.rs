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
    index::{Index, TagSuggestion},
    media::{MediaCache, RgbaBlade},
    model::{Embedding, PostId, PostRecord, Query, SearchHit, Sort, Tag, TagPolarity},
    worker::{Command, Event, Worker},
    xdg::{Lair, compact_path},
};

const RESULT_LIMIT: usize = 360;
const SOFT_POOL: usize = 2_400;
const SOFT_BACKLOG: usize = 128;
const SUGGESTIONS: usize = 12;
const BASE_TILE: f32 = 184.0;
const MIN_TILE_SCALE: f32 = 0.5;
const MAX_TILE_SCALE: f32 = 3.0;
const GAP: f32 = 8.0;

pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query_text: String,
    tag_entry: String,
    soft_text: String,
    soft_alpha: f32,
    soft_prompt: Option<String>,
    soft_embedding: Option<Embedding>,
    soft_requested: Option<String>,
    sort: Sort,
    hit: SearchHit,
    thumbs: HashMap<PostId, TextureHandle>,
    thumb_inflight: HashSet<PostId>,
    full: HashMap<PostId, TextureHandle>,
    full_rgba: HashMap<PostId, RgbaBlade>,
    full_inflight: HashSet<PostId>,
    zoom: Option<PostRecord>,
    tile_scale: f32,
    tag_menu_open: bool,
    clip_inflight: HashSet<PostId>,
    crawl_status: String,
    status: String,
}

impl Bayonet {
    pub fn new(_cc: &CreationContext<'_>) -> Result<Self> {
        let lair = Lair::claim()?;
        let index = Index::open(&lair.index_path())?;
        let media = MediaCache::new(lair.media_dir())?;
        let worker = Worker::spawn(index.clone(), media, lair.model_dir());
        let mut app = Self {
            status: format!("index {}", compact_path(&lair.index_path())),
            crawl_status: "crawl waking".to_owned(),
            lair,
            index,
            worker,
            query_text: String::new(),
            tag_entry: String::new(),
            soft_text: String::new(),
            soft_alpha: 0.0,
            soft_prompt: None,
            soft_embedding: None,
            soft_requested: None,
            sort: Sort::Score,
            hit: SearchHit::default(),
            thumbs: HashMap::new(),
            thumb_inflight: HashSet::new(),
            full: HashMap::new(),
            full_rgba: HashMap::new(),
            full_inflight: HashSet::new(),
            zoom: None,
            tile_scale: 1.0,
            tag_menu_open: false,
            clip_inflight: HashSet::new(),
        };
        app.reap(false, 0)?;
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
            self.status = format!(
                "{} hits from {} candidates; {}",
                self.hit.posts.len(),
                self.hit.candidates,
                compact_path(&self.lair.data)
            );
            self.request_soft_prompt();
        }
        if warm {
            self.worker.send(Command::Warm {
                query,
                sort: self.sort,
                pages,
            })?;
        }
        Ok(())
    }

    fn query(&self) -> Query {
        Query::parse(&self.query_text)
    }

    fn install_query(&mut self, query: Query) {
        self.query_text = query.to_text();
        self.strike(false, 0);
    }

    fn set_tag(&mut self, raw: &str, polarity: TagPolarity) {
        let Some(tag) = Tag::forge(raw) else {
            return;
        };
        let mut query = self.query();
        query.set(tag, polarity);
        self.install_query(query);
    }

    fn remove_tag(&mut self, raw: &str) {
        let Some(tag) = Tag::forge(raw) else {
            return;
        };
        let mut query = self.query();
        query.remove(&tag);
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

    fn request_soft_prompt(&mut self) {
        let Some(prompt) = self.soft_prompt() else {
            return;
        };
        if self.soft_prompt.as_deref() == Some(prompt.as_str())
            || self.soft_requested.as_deref() == Some(prompt.as_str())
        {
            return;
        }
        self.soft_requested = Some(prompt.clone());
        self.status = format!("embedding soft prompt `{prompt}`");
        if let Err(err) = self.worker.send(Command::SoftText { prompt }) {
            self.status = format!("{err:#}");
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

    fn strike(&mut self, warm: bool, pages: u32) {
        if let Err(err) = self.reap(warm, pages) {
            self.status = format!("{err:#}");
        }
    }

    fn drain(&mut self, ctx: &egui::Context) {
        let events = self.worker.drain().collect::<Vec<_>>();
        for event in events {
            match event {
                Event::Warmed { query_key, posts } => {
                    self.status = format!("absorbed {posts} posts for [{query_key}]");
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
                    ctx.request_repaint();
                }
                Event::Crawled { posts, before } => {
                    self.crawl_status = before.map_or_else(
                        || "crawl reached empty page".to_owned(),
                        |before| format!("crawl +{posts}; before #{before}"),
                    );
                    ctx.request_repaint();
                }
                Event::Blade(blade) => {
                    self.install_blade(ctx, blade, BladeKind::Thumb);
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
            BladeKind::Thumb => {
                let _old = self.thumbs.insert(blade.id, texture);
                let _was_inflight = self.thumb_inflight.remove(&blade.id);
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
                    self.strike(false, 0);
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
                self.strike(false, 0);
            }
            let slider = egui::Slider::new(&mut self.soft_alpha, 0.0..=2.0)
                .text("clip α")
                .fixed_decimals(2);
            if ui.add(slider).changed() {
                self.strike(false, 0);
            }
            if ui.button("embed visible").clicked() {
                let queued = self.queue_clip(self.hit.posts.clone());
                self.status = format!("queued {queued} visible images for Jina CLIP");
            }
        });
        let _label = ui.label(format!("{}; {}", self.status, self.crawl_status));
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
        self.set_tag(&suggestion.tag, polarity);
        self.tag_entry.clear();
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let query = self.query();
        let _heading = ui.heading("filter");
        let entry = ui.text_edit_singleline(&mut self.tag_entry);
        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if enter && (entry.has_focus() || entry.lost_focus()) {
            self.commit_tag_entry();
        }
        self.autocomplete(ui);
        let _hint = ui.label("enter adds; prefix - to ban; × removes.");
        let _separator = ui.separator();
        if query.tags().is_empty() && query.excluded_tags().is_empty() {
            let _empty = ui.label("neutral");
        }
        for tag in query.tags() {
            self.chip(ui, tag, TagPolarity::Positive);
        }
        for tag in query.excluded_tags() {
            self.chip(ui, tag, TagPolarity::Negative);
        }
    }

    fn chip(&mut self, ui: &mut egui::Ui, tag: &Tag, polarity: TagPolarity) {
        let label = match polarity {
            TagPolarity::Positive => format!("+ {tag}"),
            TagPolarity::Negative => format!("- {tag}"),
        };
        let _chip = ui.horizontal(|ui| {
            if ui.small_button("×").clicked() {
                self.remove_tag(tag.as_str());
            }
            let _label = ui.label(label);
        });
    }

    fn commit_tag_entry(&mut self) {
        let entry = Query::parse(&self.tag_entry);
        if entry.tags().is_empty() && entry.excluded_tags().is_empty() {
            return;
        }
        let mut query = self.query();
        for tag in entry.tags() {
            query.set(tag.clone(), TagPolarity::Positive);
        }
        for tag in entry.excluded_tags() {
            query.set(tag.clone(), TagPolarity::Negative);
        }
        self.tag_entry.clear();
        self.install_query(query);
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
            let response = if let Some(texture) = self.thumb(post) {
                let image = egui::Image::new(texture)
                    .max_size(egui::vec2(tile, tile))
                    .sense(egui::Sense::click());
                ui.add(image)
            } else {
                ui.allocate_ui(egui::vec2(tile, tile), |ui| {
                    let _center = ui.centered_and_justified(|ui| {
                        let _label = ui.label("loading");
                    });
                })
                .response
            };
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
        if !self.thumbs.contains_key(&post.id) && !self.thumb_inflight.contains(&post.id) {
            let _now_inflight = self.thumb_inflight.insert(post.id);
            if let Err(err) = self.worker.send(Command::Blade {
                id: post.id,
                url: post.blade_url().map(ToOwned::to_owned),
            }) {
                self.status = format!("{err:#}");
            }
        }
        self.thumbs.get(&post.id)
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
        ctx.request_repaint();
    }
}

#[derive(Clone, Copy)]
enum BladeKind {
    Thumb,
    Full,
}

impl BladeKind {
    fn texture_prefix(self) -> &'static str {
        match self {
            Self::Thumb => "post",
            Self::Full => "full",
        }
    }
}

struct ActivePrefix {
    body: String,
    negative: bool,
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
