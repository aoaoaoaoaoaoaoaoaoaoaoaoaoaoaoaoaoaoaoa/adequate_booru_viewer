use super::*;

const GUTTER: f32 = 8.0;
const FULL_FADE: Duration = Duration::from_millis(50);
const NETWORK_TEXT_DWELL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FullWait {
    LocalDecode,
    NetworkFetch { born: Instant },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ViewerAction {
    Copy,
    Save,
    Tags,
    Close,
}

pub(super) fn viewer_title_bar(
    ui: &mut egui::Ui,
    post: &PostRecord,
    tags_open: bool,
) -> Vec<ViewerAction> {
    let mut actions = Vec::new();
    let _bar = egui::Frame::new()
        .fill(chrome::RAISED)
        .stroke(egui::Stroke::new(1.0, chrome::EDGE))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let _row = ui.horizontal(|ui| {
                let _link = ui
                    .hyperlink_to(
                        egui::RichText::new(viewer_id_date(post))
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
                            actions.push(ViewerAction::Close);
                        }
                        if chrome::glyph_enabled(ui, post.full_url().is_some(), "save", false)
                            .clicked()
                        {
                            actions.push(ViewerAction::Save);
                        }
                        if chrome::glyph(ui, "copy", false).clicked() {
                            actions.push(ViewerAction::Copy);
                        }
                        if chrome::glyph(ui, "tags", tags_open)
                            .on_hover_text("toggle tags (Tab)")
                            .clicked()
                        {
                            actions.push(ViewerAction::Tags);
                        }
                    });
            });
        });
    actions
}

fn viewer_id_date(post: &PostRecord) -> String {
    let day = post.created_at.get(..10).unwrap_or("????-??-??");
    format!("#{}  {}", post.id, day)
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
        self.viewer_tag_groups = None;
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
        self.full_loaded_at.retain(|id, _| *id == post.id);
        self.full_wait.retain(|id, _| *id == post.id);
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
        let media_dir = self.lair.media_dir();
        let born = Instant::now();
        let wait = if crate::media::cached(&media_dir, post.id, &url) {
            FullWait::LocalDecode
        } else {
            FullWait::NetworkFetch { born }
        };
        let _old = self.full_wait.insert(post.id, wait);
        let _now_inflight = self.full_inflight.insert(post.id);
        if let Err(err) = self.worker.send(Command::FullBlade {
            id: post.id,
            url: Some(url),
        }) {
            let _was_inflight = self.full_inflight.remove(&post.id);
            let _was_waiting = self.full_wait.remove(&post.id);
            let _faulted = self.full_faults.insert(post.id);
            self.status = format!("{err:#}");
        }
    }

    pub(super) fn full_frame(&mut self, ctx: &egui::Context) {
        if self.zoom.is_some() && tab_pressed(ctx) {
            self.toggle_viewer_tags();
        }
        if self.zoom.is_some() && !ctx.text_edit_focused() {
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
        let tags = self.viewer_tags_open;
        let gutter = if tags { GUTTER } else { 0.0 };
        let drawer = if tags { TAG_MENU_WIDTH + gutter } else { 0.0 };
        let image_box = full_image_box(&post, self.full.get(&post.id), screen.size(), drawer);
        let body = egui::vec2(image_box.x + drawer, image_box.y + VIEWER_CHROME);
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
                for action in viewer_title_bar(ui, &post, self.viewer_tags_open) {
                    match action {
                        ViewerAction::Copy => self.copy_full(post.id),
                        ViewerAction::Save => self.save_full(&post),
                        ViewerAction::Tags => self.toggle_viewer_tags(),
                        ViewerAction::Close => close = true,
                    }
                }
                let _row = ui.allocate_ui_with_layout(
                    egui::vec2(body.x, image_box.y),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = gutter;
                        if let Some(texture) = self.full.get(&post.id) {
                            let alpha = self.full_alpha(ctx, post.id);
                            let response = ui.add(
                                egui::Image::new(texture)
                                    .fit_to_exact_size(image_box)
                                    .tint(egui::Color32::from_white_alpha(
                                        (alpha * 255.0).round() as u8
                                    ))
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
                        } else if let Some(wait) = self.full_wait.get(&post.id).copied() {
                            wait_box(ui, image_box, wait.opacity());
                        } else {
                            wait_box(ui, image_box, 0.0);
                        }
                        if self.viewer_tags_open {
                            self.viewer_tag_drawer(ui, &post, image_box.y);
                        }
                    },
                );
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
            self.full_loaded_at.clear();
            self.full_wait.clear();
            self.full_faults.clear();
            self.viewer_tag_groups = None;
            self.viewer_touches.clear();
            self.viewer_pond = egui::Rect::ZERO;
        } else {
            self.zoom_gate = ZoomGate::Armed;
        }
    }

    fn full_alpha(&self, ctx: &egui::Context, id: PostId) -> f32 {
        let Some(born) = self.full_loaded_at.get(&id) else {
            return 1.0;
        };
        let t = (born.elapsed().as_secs_f32() / FULL_FADE.as_secs_f32()).clamp(0.0, 1.0);
        if t < 1.0 {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        smooth(t)
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

    fn toggle_viewer_tags(&mut self) {
        self.viewer_tags_open = !self.viewer_tags_open;
        self.save_config();
    }
}

impl FullWait {
    fn opacity(self) -> f32 {
        let Self::NetworkFetch { born } = self else {
            return 0.0;
        };
        let age = born.elapsed();
        if age <= NETWORK_TEXT_DWELL {
            return 0.0;
        }
        let t = (age.saturating_sub(NETWORK_TEXT_DWELL).as_secs_f32() / FULL_FADE.as_secs_f32())
            .clamp(0.0, 1.0);
        smooth(t)
    }
}

fn full_image_box(
    post: &PostRecord,
    texture: Option<&TextureHandle>,
    screen: egui::Vec2,
    reserved_width: f32,
) -> egui::Vec2 {
    let image = texture.map_or_else(|| post_image_size(post), TextureHandle::size_vec2);
    let bounds = egui::vec2(
        (screen.x * 0.9 - reserved_width).max(64.0),
        (screen.y * 0.9 - VIEWER_CHROME).max(64.0),
    );
    fit(image, bounds)
}

fn tab_pressed(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        })
    })
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

fn wait_box(ui: &mut egui::Ui, size: egui::Vec2, opacity: f32) {
    let _box = ui.allocate_ui_with_layout(
        size,
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        |ui| {
            let alpha = (opacity * 255.0).round() as u8;
            if alpha > 0 {
                let text = egui::RichText::new("loading full image")
                    .color(egui::Color32::from_white_alpha(alpha));
                let _label = ui.label(text);
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
        },
    );
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
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
