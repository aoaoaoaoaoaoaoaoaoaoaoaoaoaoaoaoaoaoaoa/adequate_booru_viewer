use anyhow::Result;
use eframe::{
    App, CreationContext,
    egui::{self, ColorImage, TextureHandle, TextureOptions},
};
use std::collections::{HashMap, HashSet};

use crate::{
    index::Index,
    media::MediaCache,
    model::{PostId, PostRecord, Query, SearchHit, Sort},
    worker::{Command, Event, Worker},
    xdg::{Lair, compact_path},
};

const RESULT_LIMIT: usize = 360;
const TILE: f32 = 184.0;
const GAP: f32 = 8.0;

pub struct Bayonet {
    lair: Lair,
    index: Index,
    worker: Worker,
    query_text: String,
    sort: Sort,
    hit: SearchHit,
    textures: HashMap<PostId, TextureHandle>,
    inflight: HashSet<PostId>,
    status: String,
}

impl Bayonet {
    pub fn new(_cc: &CreationContext<'_>) -> Result<Self> {
        let lair = Lair::claim()?;
        let index = Index::open(&lair.index_path())?;
        let media = MediaCache::new(lair.media_dir())?;
        let worker = Worker::spawn(index.clone(), media);
        let mut app = Self {
            status: format!("index {}", compact_path(&lair.index_path())),
            lair,
            index,
            worker,
            query_text: String::new(),
            sort: Sort::Newest,
            hit: SearchHit::default(),
            textures: HashMap::new(),
            inflight: HashSet::new(),
        };
        app.reap(false, 0)?;
        Ok(app)
    }

    fn reap(&mut self, warm: bool, pages: u32) -> Result<()> {
        let query = Query::parse(&self.query_text);
        self.hit = self.index.search(&query, self.sort, RESULT_LIMIT)?;
        self.status = format!(
            "{} hits from {} candidates; {}",
            self.hit.posts.len(),
            self.hit.candidates,
            compact_path(&self.lair.data)
        );
        if warm {
            self.worker.send(Command::Warm {
                query,
                sort: self.sort,
                pages,
            })?;
        }
        Ok(())
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
                Event::Blade(blade) => {
                    let image = ColorImage::from_rgba_unmultiplied(blade.size, &blade.rgba);
                    let texture = ctx.load_texture(
                        format!("post-{}", blade.id),
                        image,
                        TextureOptions::LINEAR,
                    );
                    let _old = self.textures.insert(blade.id, texture);
                    let _was_inflight = self.inflight.remove(&blade.id);
                    ctx.request_repaint();
                }
                Event::Fault(fault) => {
                    self.status = fault;
                    ctx.request_repaint();
                }
            }
        }
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        let _bar = ui.horizontal(|ui| {
            let query = ui.text_edit_singleline(&mut self.query_text);
            if query.changed() {
                self.strike(false, 0);
            }
            let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
            if query.lost_focus() && enter {
                self.strike(true, 1);
            }

            for sort in Sort::ALL {
                if ui
                    .selectable_label(self.sort == sort, sort.label())
                    .clicked()
                {
                    self.sort = sort;
                    if let Err(err) = self.reap(false, 0) {
                        self.status = format!("{err:#}");
                    }
                }
            }

            if ui.button("warm +200").clicked() {
                self.strike(true, 1);
            }
            if ui.button("ransack +1000").clicked() {
                self.strike(true, 5);
            }
        });
        let _label = ui.label(&self.status);
    }

    fn grid(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width().max(TILE);
        let cols = ((width + GAP) / (TILE + GAP)).floor().max(1.0) as usize;
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
        let _tile = ui.vertical(|ui| {
            ui.set_width(TILE);
            if let Some(texture) = self.texture(post) {
                let image = egui::Image::new(texture).max_size(egui::vec2(TILE, TILE));
                let _response = ui.add(image);
            } else {
                let _response = ui.allocate_ui(egui::vec2(TILE, TILE), |ui| {
                    let _center = ui.centered_and_justified(|ui| {
                        let _label = ui.label("loading");
                    });
                });
            }
            let _id = ui.label(format!(
                "#{}  score {}  fav {}",
                post.id, post.score, post.favs
            ));
            let _tags = ui.label(post.haystack());
        });
    }

    fn texture(&mut self, post: &PostRecord) -> Option<&TextureHandle> {
        if !self.textures.contains_key(&post.id) && !self.inflight.contains(&post.id) {
            let _now_inflight = self.inflight.insert(post.id);
            if let Err(err) = self.worker.send(Command::Blade {
                id: post.id,
                url: post.blade_url().map(ToOwned::to_owned),
            }) {
                self.status = format!("{err:#}");
            }
        }
        self.textures.get(&post.id)
    }
}

impl App for Bayonet {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let _edge = egui::Panel::top("edge").show_inside(ui, |ui| self.top(ui));
        let _center = egui::CentralPanel::default().show_inside(ui, |ui| self.grid(ui));
    }
}
