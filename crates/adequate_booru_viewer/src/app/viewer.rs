use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ViewerBar {
    pub(super) copy: bool,
    pub(super) save: bool,
    pub(super) close: bool,
}

pub(super) fn viewer_title_bar(ui: &mut egui::Ui, post: &PostRecord) -> ViewerBar {
    let mut bar = ViewerBar::default();
    let _bar = egui::Frame::new()
        .fill(chrome::RAISED)
        .stroke(egui::Stroke::new(1.0, chrome::EDGE))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let _row = ui.horizontal(|ui| {
                let _link = ui
                    .hyperlink_to(
                        egui::RichText::new(format!("#{}", post.id))
                            .size(13.0)
                            .strong(),
                        crate::booru::post_url(post.id),
                    )
                    .on_hover_text("open on Danbooru");
                let _meta = ui.label(
                    egui::RichText::new(format!("score {}  fav {}", post.score, post.favs))
                        .size(13.0)
                        .strong()
                        .color(chrome::TEXT),
                );
                let _actions =
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if chrome::icon(ui, "×").on_hover_text("close").clicked() {
                            bar.close = true;
                        }
                        if chrome::glyph_enabled(ui, post.full_url().is_some(), "save", false)
                            .clicked()
                        {
                            bar.save = true;
                        }
                        if chrome::glyph(ui, "copy", false).clicked() {
                            bar.copy = true;
                        }
                    });
            });
        });
    bar
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ZoomGate {
    Fresh,
    Armed,
}

impl Bayonet {
    pub(super) fn open_full(&mut self, post: &PostRecord) {
        self.zoom = Some(post.clone());
        self.zoom_gate = ZoomGate::Fresh;
        self.viewer_touches.clear();
        self.viewer_pond = egui::Rect::ZERO;
        let _old_fault = self.full_faults.remove(&post.id);
        self.request_full(post);
    }

    /// Steps the viewer through the current result sequence, keeping
    /// full-image memory O(1) by evicting everything but the new post.
    fn step_zoom(&mut self, step: i32) {
        let Some(zoom) = &self.zoom else {
            return;
        };
        let Some(slot) = self.hit.posts.iter().position(|post| post.id == zoom.id) else {
            return;
        };
        let target = slot
            .saturating_add_signed(step as isize)
            .min(self.hit.posts.len().saturating_sub(1));
        let post = self.hit.posts[target].clone();
        if post.id == zoom.id {
            return;
        }
        self.full.retain(|id, _| *id == post.id);
        self.full_rgba.retain(|id, _| *id == post.id);
        self.open_full(&post);
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

    pub(super) fn full_frame(&mut self, ctx: &egui::Context) {
        if self.zoom.is_some() && !ctx.egui_wants_keyboard_input() {
            let step = ctx.input(|input| {
                i32::from(input.key_pressed(egui::Key::ArrowRight))
                    - i32::from(input.key_pressed(egui::Key::ArrowLeft))
            });
            if step != 0 {
                self.step_zoom(step);
            }
        }
        let Some(post) = self.zoom.clone() else {
            return;
        };
        self.request_full(&post);
        let mut close = false;
        self.viewer_pond = egui::Rect::ZERO;
        let screen = ctx.content_rect();
        let image_box = full_image_box(&post, self.full.get(&post.id), screen.size());
        let body = egui::vec2(image_box.x, image_box.y + VIEWER_CHROME);
        // fixed_size is re-asserted every frame: egui persists window sizes by Id,
        // and a remembered size would wedge every later image into a stale frame.
        let window = egui::Window::new("full-viewer")
            .id(egui::Id::new("full-viewer"))
            .title_bar(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size(body)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let bar = viewer_title_bar(ui, &post);
                close |= bar.close;
                if bar.copy {
                    self.copy_full(post.id);
                }
                if bar.save {
                    self.save_full(&post);
                }
                if let Some(texture) = self.full.get(&post.id) {
                    let response = ui.add(
                        egui::Image::new(texture)
                            .fit_to_exact_size(image_box)
                            .sense(egui::Sense::click()),
                    );
                    self.viewer_pond = response.rect;
                    if response.clicked_by(egui::PointerButton::Primary)
                        && let Some(pos) = response.interact_pointer_pos()
                    {
                        self.touch_viewer(pos);
                    }
                    if response.secondary_clicked() {
                        close = true;
                    }
                } else if self.full_faults.contains(&post.id) {
                    centered_box(ui, image_box, "full image failed");
                } else {
                    centered_box(ui, image_box, "loading full image");
                }
            });
        if let Some(window) = &window {
            self.zoom_rect = Some(window.response.rect);
        }
        let clicked_outside = window
            .as_ref()
            .is_some_and(|window| outside_click(ctx, window.response.rect));
        close |=
            !self.tag_menu.is_open() && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if close || (self.zoom_gate == ZoomGate::Armed && clicked_outside) {
            self.zoom = None;
            self.zoom_gate = ZoomGate::Fresh;
            self.full.clear();
            self.full_rgba.clear();
            self.full_faults.clear();
            self.viewer_touches.clear();
            self.viewer_pond = egui::Rect::ZERO;
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
        // The X11 clipboard transfer of a full-size image takes long enough
        // to hitch a frame; hand it to a throwaway thread and toast back.
        let blade = blade.clone();
        let crier = self.worker.crier();
        self.status = format!("copying #{id}…");
        let _hand = std::thread::spawn(move || {
            let result = Clipboard::new()
                .context("open clipboard")
                .and_then(|mut clipboard| {
                    clipboard
                        .set_image(ImageData {
                            width: blade.size[0],
                            height: blade.size[1],
                            bytes: Cow::Owned(blade.rgba),
                        })
                        .context("copy image")
                });
            crier.toast(match result {
                Ok(()) => format!("copied #{id}"),
                Err(err) => format!("{err:#}"),
            });
        });
    }
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
