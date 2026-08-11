use super::*;

const GUTTER: f32 = 8.0;
const FULL_FADE: Duration = Duration::from_millis(50);
const NETWORK_TEXT_DWELL: Duration = Duration::from_millis(50);
const TREE_TILE: f32 = 124.0;
const TREE_GAP_X: f32 = 22.0;
const TREE_GAP_Y: f32 = 46.0;
pub(super) const TREE_ZOOM_DEFAULT: f32 = 2.0;
const TREE_ZOOM_MIN: f32 = 0.75;
const TREE_ZOOM_MAX: f32 = 4.0;
const TREE_ZOOM_RATE: f32 = 0.0025;
const KIN_DRAG_COMMIT: f32 = 72.0;
const KIN_DRAG_VALID_CEIL: f32 = 120.0;
const KIN_DRAG_BLOCKED_CEIL: f32 = 22.0;
const KIN_SPRING_OMEGA: f32 = 24.0;
const KIN_SPRING_ZETA: f32 = 0.58;
const VIEWER_RECOIL: f32 = 0.03;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct KinDrag {
    travel: egui::Vec2,
    offset: egui::Vec2,
    recoil: Option<Recoil>,
}

#[derive(Clone, Copy, Debug)]
struct Recoil {
    born: Instant,
    origin: egui::Vec2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FullWait {
    LocalDecode,
    NetworkFetch { born: Instant },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ViewerAction {
    Copy,
    Save,
    Favorite,
    Tags,
    Kin(KinStep),
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum KinStep {
    Previous,
    Parent,
    Children,
    Next,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct KinNav {
    passages: [bool; 4],
    present: bool,
}

impl KinNav {
    fn allows(self, step: KinStep) -> bool {
        self.passages[step as usize]
    }
}

pub(super) fn viewer_title_bar(
    ui: &mut egui::Ui,
    water: &mut dwemer_poolrooms::water::Surface,
    post: &PostRecord,
    favorite: bool,
    tags_open: bool,
    surface: ViewerSurface,
    kin: KinNav,
) -> Vec<ViewerAction> {
    let mut actions = Vec::new();
    let _bar = egui::Frame::new()
        .fill(chrome::RAISED)
        .stroke(egui::Stroke::new(1.0_f32, chrome::EDGE))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let _row = ui.horizontal(|ui| {
                let link = ui.hyperlink_to(
                    egui::RichText::new(viewer_id_date(post))
                        .size(13.0)
                        .strong(),
                    crate::booru::post_url(post.id),
                );
                crate::probe_anchor!(ui, format!("danbooru:{}", post.id.0), link.interact_rect);
                let _link = link.on_hover_text("open on Danbooru");
                let _meta = ui.label(
                    egui::RichText::new(format!("score {}  fav {}", post.score, post.favs))
                        .size(13.0)
                        .strong()
                        .color(chrome::TEXT),
                );
                if kin.present {
                    for (step, help) in [
                        (KinStep::Previous, "previous"),
                        (KinStep::Parent, "parent"),
                        (KinStep::Children, "children"),
                        (KinStep::Next, "next"),
                    ] {
                        if kin_button(ui, step, kin.allows(step))
                            .on_hover_text(help)
                            .clicked()
                        {
                            actions.push(ViewerAction::Kin(step));
                        }
                    }
                }
                let _actions =
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if controls::symbol(ui, water, chrome::Symbol::Remove)
                            .on_hover_text("close")
                            .clicked()
                        {
                            actions.push(ViewerAction::Close);
                        }
                        if controls::plate(ui, if favorite { "♥" } else { "♡" }, favorite)
                            .on_hover_text(if favorite {
                                "remove local favorite"
                            } else {
                                "add local favorite"
                            })
                            .clicked()
                        {
                            actions.push(ViewerAction::Favorite);
                        }
                        if controls::plate_enabled(ui, post.full_url().is_some(), "save", false)
                            .clicked()
                        {
                            actions.push(ViewerAction::Save);
                        }
                        if controls::plate(ui, "copy", false).clicked() {
                            actions.push(ViewerAction::Copy);
                        }
                        if surface == ViewerSurface::Image {
                            let spec = commands::canon().spec(Edict::ToggleViewerTags);
                            let response = ui.add(
                                egui::Button::new(spec.widget_text(ui))
                                    .selected(tags_open)
                                    .min_size(egui::vec2(24.0, 20.0)),
                            );
                            chrome::tension(ui, &response);
                            let response = response.on_hover_text(format!(
                                "toggle tags ({})",
                                commands::canon().shortcuts(Edict::ToggleViewerTags)[0]
                                    .label(ui.ctx())
                            ));
                            if chrome::exact_activation(ui, &response) {
                                actions.push(ViewerAction::Tags);
                            }
                        }
                    });
            });
        });
    actions
}

/// Four congruent chevrons cut from one geometric die. Font arrows vary in
/// weight, baseline, and aperture; a painted primitive cannot drift apart.
fn kin_button(ui: &mut egui::Ui, step: KinStep, enabled: bool) -> egui::Response {
    let button = egui::Button::new("").min_size(egui::vec2(24.0, 20.0));
    let response = ui.add_enabled(enabled, button);
    chrome::tension(ui, &response);
    let direction = match step {
        KinStep::Previous => egui::vec2(-1.0, 0.0),
        KinStep::Parent => egui::vec2(0.0, -1.0),
        KinStep::Children => egui::vec2(0.0, 1.0),
        KinStep::Next => egui::vec2(1.0, 0.0),
    };
    let normal = egui::vec2(-direction.y, direction.x);
    let tip = response.rect.center() + direction * 3.8;
    let heel = response.rect.center() - direction * 2.8;
    let color = if !enabled {
        chrome::MUTED.gamma_multiply(0.42)
    } else if response.hovered() {
        chrome::HOT
    } else {
        chrome::TEXT
    };
    let stroke = egui::Stroke::new(1.35_f32, color);
    let _upper = ui
        .painter()
        .line_segment([heel + normal * 4.0, tip], stroke);
    let _lower = ui
        .painter()
        .line_segment([tip, heel - normal * 4.0], stroke);
    response
}

fn viewer_id_date(post: &PostRecord) -> String {
    let day = post.created_at.get(..10).unwrap_or("????-??-??");
    format!("#{}  {}", post.id, day)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ZoomGate {
    Fresh,
    Settling,
    Armed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ViewerSurface {
    #[default]
    Image,
    Family,
}

impl Bayonet {
    pub(super) fn open_full(&mut self, post: &PostRecord) {
        self.zoom = Some(post.clone());
        self.viewer_gallery_anchor = Some(post.id);
        self.zoom_gate = ZoomGate::Fresh;
        self.viewer_surface = ViewerSurface::Image;
        self.viewer_drag = KinDrag::default();
        self.viewer_recoil = None;
        self.viewer_tree_zoom = TREE_ZOOM_DEFAULT;
        self.viewer_tree_pan = egui::Vec2::ZERO;
        self.viewer_tree_fresh = true;
        self.viewer_family = None;
        self.viewer_tag_groups = None;
        self.family_water.reset();
        self.water.close_pond();
        let _old_fault = self.full_faults.remove(&post.id);
        self.request_full(post);
        self.request_family(post.id);
    }

    fn request_family(&mut self, id: PostId) {
        self.family_serial = self.family_serial.saturating_add(1);
        if let Err(err) = self.worker.send(Command::Family {
            serial: self.family_serial,
            id,
        }) {
            self.status = format!("{err:#}");
        }
    }

    /// Steps the viewer through the current result sequence, keeping
    /// full-image memory O(1) by evicting everything but the new post.
    fn step_zoom(&mut self, step: i32) {
        let Some(slot) = self.gallery_slot() else {
            return;
        };
        let target = slot
            .saturating_add_signed(step as isize)
            .min(self.hit.posts.len().saturating_sub(1));
        let post = self.hit.posts[target].clone();
        if self.zoom.as_ref().is_some_and(|zoom| post.id == zoom.id) {
            return;
        }
        self.full.retain(|id, _| *id == post.id);
        self.full_rgba.retain(|id, _| *id == post.id);
        self.full_loaded_at.retain(|id, _| *id == post.id);
        self.full_wait.retain(|id, _| *id == post.id);
        self.open_full(&post);
    }

    fn gallery_slot(&self) -> Option<usize> {
        let current = self.zoom.as_ref()?.id;
        gallery_slot(&self.hit.posts, current, self.viewer_gallery_anchor)
    }

    fn can_step_zoom(&self, step: i32) -> bool {
        let Some(slot) = self.gallery_slot() else {
            return false;
        };
        match step.cmp(&0) {
            std::cmp::Ordering::Less => slot > 0,
            std::cmp::Ordering::Equal => false,
            std::cmp::Ordering::Greater => slot + 1 < self.hit.posts.len(),
        }
    }

    fn kin_nav(&self, id: PostId) -> KinNav {
        let Some(tree) = &self.viewer_family else {
            return KinNav::default();
        };
        let Some(node) = tree.node(id) else {
            return KinNav::default();
        };
        let peers = level_posts(tree, id);
        let lateral = peers.len() > 1;
        let root = id == tree.root;
        KinNav {
            passages: [
                lateral || root && self.can_step_zoom(-1),
                node.parent
                    .is_some_and(|parent| tree.post(parent).is_some()),
                node.children
                    .iter()
                    .any(|child| tree.post(*child).is_some()),
                lateral || root && self.can_step_zoom(1),
            ],
            present: tree.badge().is_some(),
        }
    }

    fn open_family_tree(&mut self) -> bool {
        if self
            .viewer_family
            .as_ref()
            .and_then(FamilyTree::badge)
            .is_none()
        {
            return false;
        }
        if self.viewer_surface != ViewerSurface::Family {
            self.family_water.reset();
        }
        self.viewer_surface = ViewerSurface::Family;
        self.viewer_tree_fresh = true;
        self.viewer_recoil = None;
        self.water.close_pond();
        true
    }

    fn rebuff_family_pullback(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        self.viewer_recoil = Some(Instant::now());
        // The rejected pullback loads the image-wall spring and releases a
        // low-energy hull wave from the entire pond boundary.
        self.water.poke(rect, crate::water::Poke::ring(0.42));
        ctx.request_repaint();
    }

    fn viewer_recoil_scale(&mut self, ctx: &egui::Context) -> f32 {
        let Some(born) = self.viewer_recoil else {
            return 1.0;
        };
        let t = born.elapsed().as_secs_f32();
        if t >= 0.65 {
            self.viewer_recoil = None;
            return 1.0;
        }
        ctx.request_repaint_after(Duration::from_millis(8));
        1.0 - VIEWER_RECOIL * spring_recoil(t)
    }

    fn navigate_kin(&mut self, step: KinStep) {
        let Some(id) = self.zoom.as_ref().map(|post| post.id) else {
            return;
        };
        let Some(tree) = self.viewer_family.as_ref() else {
            return;
        };
        let root = id == tree.root;
        let target = direct_kin_target(tree, id, step);
        if let Some(post) = target {
            self.focus_family_post(post);
        } else if root {
            match step {
                KinStep::Previous => self.step_zoom(-1),
                KinStep::Next => self.step_zoom(1),
                KinStep::Parent | KinStep::Children => {}
            }
        }
    }

    fn focus_family_post(&mut self, post: PostRecord) {
        self.full.retain(|id, _| *id == post.id);
        self.full_rgba.retain(|id, _| *id == post.id);
        self.full_loaded_at.retain(|id, _| *id == post.id);
        self.full_wait.retain(|id, _| *id == post.id);
        self.full_faults.retain(|id| *id == post.id);
        self.zoom = Some(post.clone());
        self.viewer_surface = ViewerSurface::Image;
        self.viewer_drag = KinDrag::default();
        self.viewer_recoil = None;
        self.viewer_tag_groups = None;
        if let Some(tree) = &mut self.viewer_family {
            tree.focus = post.id;
        }
        self.water.close_pond();
        self.request_full(&post);
    }

    fn select_tree_kin(&mut self, step: KinStep) {
        let Some(tree) = &mut self.viewer_family else {
            return;
        };
        let Some(target) = direct_kin_target(tree, tree.focus, step) else {
            return;
        };
        tree.focus = target.id;
        self.viewer_tree_fresh = true;
    }

    fn promote_tree_focus(&mut self) {
        let post = self
            .viewer_family
            .as_ref()
            .and_then(|tree| tree.post(tree.focus))
            .cloned();
        if let Some(post) = post {
            self.focus_family_post(post);
        }
    }

    /// The full image is a plate held in a stiff cardinal gate. A valid drag
    /// follows the hand far enough to commit; an impossible drag loads a much
    /// stiffer spring and recoils instead of pretending the passage exists.
    fn drag_kin(
        &mut self,
        ctx: &egui::Context,
        response: &egui::Response,
        nav: KinNav,
    ) -> (egui::Vec2, Option<KinStep>) {
        if response.drag_started() {
            self.viewer_drag = KinDrag::default();
        }
        let ending = response.drag_stopped();
        if response.dragged() || ending {
            if response.dragged() {
                self.viewer_drag.travel += response.drag_delta();
            }
            let raw = self.viewer_drag.travel;
            let step = drag_step(raw);
            let allowed = step.is_some_and(|step| nav.allows(step));
            let ceiling = if allowed {
                KIN_DRAG_VALID_CEIL
            } else {
                KIN_DRAG_BLOCKED_CEIL
            };
            self.viewer_drag.offset = rubber_vec(raw, ceiling);
            if ending {
                let commit = step.filter(|step| {
                    nav.allows(*step) && cardinal_extent(raw, *step) >= KIN_DRAG_COMMIT
                });
                if commit.is_some() {
                    self.viewer_drag = KinDrag::default();
                    return (egui::Vec2::ZERO, commit);
                }
                self.viewer_drag.recoil = Some(Recoil {
                    born: Instant::now(),
                    origin: self.viewer_drag.offset,
                });
                self.water
                    .touch(response.rect.center() + self.viewer_drag.offset);
            }
        } else if let Some(recoil) = self.viewer_drag.recoil {
            let t = recoil.born.elapsed().as_secs_f32();
            self.viewer_drag.offset = recoil.origin * spring_recoil(t);
            if self.viewer_drag.offset.length_sq() <= 0.01 || t >= 0.65 {
                self.viewer_drag = KinDrag::default();
            } else {
                ctx.request_repaint_after(Duration::from_millis(8));
            }
        }
        (self.viewer_drag.offset, None)
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
        self.water
            .begin_pond(self.zoom.is_some() && self.viewer_surface == ViewerSurface::Image);
        let inputs_blocked =
            self.guide.is_open() || ctx.memory(|memory| memory.top_modal_layer().is_some());
        if !inputs_blocked && self.zoom.is_some() && !ctx.text_edit_focused() {
            let key = ctx.input(|input| {
                [
                    (egui::Key::ArrowLeft, KinStep::Previous),
                    (egui::Key::ArrowUp, KinStep::Parent),
                    (egui::Key::ArrowDown, KinStep::Children),
                    (egui::Key::ArrowRight, KinStep::Next),
                ]
                .into_iter()
                .find_map(|(key, step)| {
                    input.key_pressed(key).then_some((
                        step,
                        input.modifiers.shift && matches!(step, KinStep::Previous | KinStep::Next),
                    ))
                })
            });
            if let Some((step, true)) = key {
                self.step_zoom(if step == KinStep::Previous { -1 } else { 1 });
            } else if self.viewer_surface == ViewerSurface::Family {
                if let Some((step, false)) = key {
                    self.select_tree_kin(step);
                } else if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
                    self.promote_tree_focus();
                }
            } else if let Some((step, false)) = key {
                let nav = self
                    .zoom
                    .as_ref()
                    .map_or_else(KinNav::default, |post| self.kin_nav(post.id));
                if nav.allows(step) {
                    self.navigate_kin(step);
                } else if matches!(step, KinStep::Previous | KinStep::Next) {
                    self.step_zoom(if step == KinStep::Previous { -1 } else { 1 });
                }
            }
        }
        let Some(post) = self.zoom.clone() else {
            return;
        };
        self.request_full(&post);
        let mut close = false;
        let screen = ctx.content_rect();
        let surface = self.viewer_surface;
        let tags = self.viewer_tags_open && surface == ViewerSurface::Image;
        let gutter = if tags { GUTTER } else { 0.0 };
        let drawer = if tags { TAG_MENU_WIDTH + gutter } else { 0.0 };
        let image_box = match surface {
            ViewerSurface::Image => {
                full_image_box(&post, self.full.get(&post.id), screen.size(), drawer)
            }
            ViewerSurface::Family => egui::vec2(screen.width() * 0.88, screen.height() * 0.82),
        };
        let body = egui::vec2(image_box.x + drawer, image_box.y + VIEWER_CHROME);
        let kin_nav = self.kin_nav(post.id);
        let recoil = if surface == ViewerSurface::Image {
            self.viewer_recoil_scale(ctx)
        } else {
            1.0
        };
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
                for action in viewer_title_bar(
                    ui,
                    &mut self.water,
                    &post,
                    self.local_favorites.contains(post.id),
                    self.viewer_tags_open,
                    surface,
                    kin_nav,
                ) {
                    match action {
                        ViewerAction::Copy => self.copy_full(post.id),
                        ViewerAction::Save => self.save_full(&post),
                        ViewerAction::Favorite => self.toggle_local_favorite(post.id),
                        ViewerAction::Tags => self.toggle_viewer_tags(ctx),
                        ViewerAction::Kin(step) => self.navigate_kin(step),
                        ViewerAction::Close => close = true,
                    }
                }
                match surface {
                    ViewerSurface::Image => {
                        let _row = ui.allocate_ui_with_layout(
                            egui::vec2(body.x, image_box.y),
                            egui::Layout::left_to_right(egui::Align::Min),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = gutter;
                                crate::probe_anchor!(
                                    ui,
                                    abv_contract::Target::ViewerSurface,
                                    egui::Rect::from_min_size(ui.cursor().left_top(), image_box)
                                );
                                if let Some(texture) = self.full.get(&post.id) {
                                    let texture_id = texture.id();
                                    let alpha = self.full_alpha(ctx, post.id);
                                    let (rect, response) = ui.allocate_exact_size(
                                        image_box,
                                        egui::Sense::click_and_drag(),
                                    );
                                    let (drag, drag_step) = self.drag_kin(ctx, &response, kin_nav);
                                    let uv = egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    );
                                    let image = egui::Rect::from_center_size(
                                        rect.center(),
                                        rect.size() * recoil,
                                    )
                                    .translate(drag);
                                    let _image = ui.painter().with_clip_rect(rect).image(
                                        texture_id,
                                        image,
                                        uv,
                                        egui::Color32::from_white_alpha(
                                            (alpha * 255.0).round() as u8
                                        ),
                                    );
                                    self.water.pond_surface(response.rect);
                                    crate::probe_anchor!(
                                        ui,
                                        "viewer:image",
                                        response.interact_rect
                                    );
                                    if response.clicked_by(egui::PointerButton::Primary)
                                        && let Some(pos) = response.interact_pointer_pos()
                                    {
                                        self.water.touch(pos);
                                    }
                                    if !inputs_blocked
                                        && response.secondary_clicked()
                                        && !self.open_family_tree()
                                    {
                                        self.rebuff_family_pullback(ctx, response.rect);
                                    }
                                    if !inputs_blocked
                                        && response.hovered()
                                        && !response.dragged()
                                        && wheel(ctx) < 0.0
                                    {
                                        let _opened = self.open_family_tree();
                                    }
                                    if let Some(step) = drag_step {
                                        self.navigate_kin(step);
                                    }
                                } else if self.full_faults.contains(&post.id) {
                                    centered_box(ui, image_box, "full image failed");
                                } else if let Some(wait) = self.full_wait.get(&post.id).copied() {
                                    wait_box(ui, image_box, wait.opacity());
                                } else {
                                    wait_box(ui, image_box, 0.0);
                                }
                                if tags {
                                    self.viewer_tag_drawer(ui, &post, image_box.y);
                                }
                            },
                        );
                    }
                    ViewerSurface::Family => {
                        if let Some(post) = self.family_tree_view(ui, image_box) {
                            self.focus_family_post(post);
                        }
                    }
                }
            });
        if let Some(window) = &window {
            self.zoom_rect = Some(window.response.rect);
        }
        let clicked_outside = !inputs_blocked
            && window
                .as_ref()
                .is_some_and(|window| outside_click(ctx, window.response.rect));
        close |= !inputs_blocked
            && !self.tag_menu.is_open()
            && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        if close || (self.zoom_gate == ZoomGate::Armed && clicked_outside) {
            self.zoom = None;
            self.viewer_gallery_anchor = None;
            self.zoom_gate = ZoomGate::Fresh;
            self.full.clear();
            self.full_rgba.clear();
            self.full_loaded_at.clear();
            self.full_wait.clear();
            self.full_faults.clear();
            self.viewer_tag_groups = None;
            self.viewer_family = None;
            self.viewer_surface = ViewerSurface::Image;
            self.viewer_drag = KinDrag::default();
            self.viewer_recoil = None;
            self.viewer_tree_zoom = TREE_ZOOM_DEFAULT;
            self.viewer_tree_pan = egui::Vec2::ZERO;
            self.viewer_tree_fresh = true;
            self.water.close_pond();
        } else {
            // A newly materialized fixed-size egui window settles its size and
            // centered position on successive passes. Drive both passes now;
            // otherwise its final placement waits for the user's next input.
            let settling = self.zoom_gate != ZoomGate::Armed;
            self.zoom_gate = match self.zoom_gate {
                ZoomGate::Fresh => ZoomGate::Settling,
                ZoomGate::Settling | ZoomGate::Armed => ZoomGate::Armed,
            };
            if settling {
                ctx.request_repaint();
            }
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

    fn family_tree_view(&mut self, ui: &mut egui::Ui, size: egui::Vec2) -> Option<PostRecord> {
        let Some(tree) = self.viewer_family.clone() else {
            centered_box(ui, size, "loading family");
            return None;
        };
        let (pond, camera) = ui.allocate_exact_size(size, egui::Sense::drag());
        let _background = ui.painter().rect_filled(pond, 0.0, chrome::PAGE);
        self.family_water.begin(crate::water::Domain::basin(pond));
        self.family_water
            .set_floor(Some(crate::water::Floor::deep(pond)));
        let wheel = if !self.guide.is_open()
            && ui.ctx().memory(|memory| memory.top_modal_layer().is_none())
            && pond.contains(ui.ctx().pointer_latest_pos().unwrap_or_default())
        {
            take_wheel(ui.ctx())
        } else {
            0.0
        };
        let zoom = self.viewer_tree_zoom;
        let layout = TreeLayout::forge(&tree, zoom);
        let tile = TREE_TILE * zoom;
        if self.viewer_tree_fresh {
            if let Some(focus) = layout.centers.get(&tree.focus) {
                self.viewer_tree_pan = layout.size * 0.5 - *focus;
            }
            self.viewer_tree_fresh = false;
        }
        let origin = pond.center() - layout.size * 0.5 + self.viewer_tree_pan;
        let mut promoted = None;
        let mut drag = camera.drag_delta();
        let old_clip = ui.clip_rect();
        ui.set_clip_rect(pond);
        for node in tree.nodes.values() {
            let Some(&child_center) = layout.centers.get(&node.id) else {
                continue;
            };
            let Some(parent) = node.parent else {
                continue;
            };
            let Some(&parent_center) = layout.centers.get(&parent) else {
                continue;
            };
            let from = origin + parent_center + egui::vec2(0.0, tile * 0.5);
            let to = origin + child_center - egui::vec2(0.0, tile * 0.5);
            let elbow = (from.y + to.y) * 0.5;
            for segment in [
                [from, egui::pos2(from.x, elbow)],
                [egui::pos2(from.x, elbow), egui::pos2(to.x, elbow)],
                [egui::pos2(to.x, elbow), to],
            ] {
                let _line = ui
                    .painter()
                    .line_segment(segment, egui::Stroke::new(1.0_f32, chrome::EDGE_STRONG));
            }
        }
        for node in tree.nodes.values() {
            let Some(&center) = layout.centers.get(&node.id) else {
                continue;
            };
            let rect = egui::Rect::from_center_size(origin + center, egui::Vec2::splat(tile));
            let response = ui.interact(
                rect,
                ui.make_persistent_id(("family-tile", node.id)),
                egui::Sense::click_and_drag(),
            );
            if response.dragged() {
                drag = response.drag_delta();
            }
            crate::probe_anchor!(ui, format!("family-node:{}", node.id.0), rect);
            let plate = Plate::flat(rect);
            plate.paint(ui, response.hovered());
            if let Some(post) = &node.post {
                match self.thumb(post, tile) {
                    Some(ThumbLoad::Ready(texture)) => plate.paint_image(ui, post, &texture),
                    Some(ThumbLoad::Loading) => paint_tile_text(ui, plate.rect, "loading"),
                    Some(ThumbLoad::Fault) => paint_tile_text(ui, plate.rect, "fault"),
                    None => paint_tile_text(ui, plate.rect, "no image"),
                }
                if self.local_favorites.contains(post.id) {
                    paint_favorite_badge(ui, plate.rect);
                }
                if response.hovered() {
                    self.family_water.hover(("family", post.id), plate.rect);
                }
                if response.clicked() {
                    self.family_water.click(plate.rect);
                    promoted = Some(post.clone());
                }
            } else {
                paint_tile_text(ui, plate.rect, &format!("#{}\nunavailable", node.id));
            }
            if node.id == tree.focus {
                let _focus = ui.painter().rect_stroke(
                    plate.rect.shrink(1.0),
                    egui::CornerRadius::same(TILE_RADIUS),
                    egui::Stroke::new(2.0_f32, chrome::HOT),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ui.set_clip_rect(old_clip);
        if drag != egui::Vec2::ZERO {
            self.viewer_tree_pan += drag;
            self.viewer_tree_fresh = false;
            ui.ctx().request_repaint();
        }
        if promoted.is_none() && wheel != 0.0 {
            let next = (zoom * (wheel * TREE_ZOOM_RATE).exp()).clamp(TREE_ZOOM_MIN, TREE_ZOOM_MAX);
            if (next - zoom).abs() > f32::EPSILON {
                let pivot = ui
                    .ctx()
                    .pointer_latest_pos()
                    .unwrap_or_else(|| pond.center());
                let arm = pivot - pond.center();
                let dilation = next / zoom;
                self.viewer_tree_pan = arm - (arm - self.viewer_tree_pan) * dilation;
                self.viewer_tree_zoom = next;
                ui.ctx().request_repaint();
            }
        }
        promoted
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

    pub(super) fn toggle_viewer_tags(&mut self, ctx: &egui::Context) {
        self.viewer_tags_open = !self.viewer_tags_open;
        self.save_config();
        ctx.request_repaint();
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
    let image = stable_image_size(post, texture);
    let bounds = egui::vec2(
        (screen.x * 0.9 - reserved_width).max(64.0),
        (screen.y * 0.9 - VIEWER_CHROME).max(64.0),
    );
    contain_native(image, bounds)
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

fn gallery_slot(posts: &[PostRecord], current: PostId, anchor: Option<PostId>) -> Option<usize> {
    posts
        .iter()
        .position(|post| post.id == current)
        .or_else(|| anchor.and_then(|id| posts.iter().position(|post| post.id == id)))
}

fn level_posts(tree: &FamilyTree, focus: PostId) -> Vec<PostRecord> {
    let Some(depth) = kin_depth(tree, focus) else {
        return Vec::new();
    };
    let mut posts = Vec::new();
    let mut seen = HashSet::new();
    collect_level(tree, tree.root, 0, depth, &mut seen, &mut posts);
    posts
}

fn collect_level(
    tree: &FamilyTree,
    id: PostId,
    depth: usize,
    target: usize,
    seen: &mut HashSet<PostId>,
    posts: &mut Vec<PostRecord>,
) {
    if !seen.insert(id) {
        return;
    }
    if depth == target {
        if let Some(post) = tree.post(id) {
            posts.push(post.clone());
        }
        return;
    }
    if let Some(node) = tree.node(id) {
        for child in &node.children {
            collect_level(tree, *child, depth + 1, target, seen, posts);
        }
    }
}

fn kin_depth(tree: &FamilyTree, mut id: PostId) -> Option<usize> {
    let mut seen = HashSet::new();
    for depth in 0..=tree.nodes.len() {
        if !seen.insert(id) {
            return None;
        }
        let node = tree.node(id)?;
        let Some(parent) = node.parent else {
            return (id == tree.root).then_some(depth);
        };
        id = parent;
    }
    None
}

/// Direct topology shared by title buttons, key presses, gestures, and the
/// tree cursor. Down means the first available child everywhere.
fn direct_kin_target(tree: &FamilyTree, id: PostId, step: KinStep) -> Option<PostRecord> {
    let node = tree.node(id)?;
    match step {
        KinStep::Parent => node.parent.and_then(|parent| tree.post(parent)).cloned(),
        KinStep::Children => node
            .children
            .iter()
            .find_map(|child| tree.post(*child))
            .cloned(),
        KinStep::Previous | KinStep::Next => {
            let peers = level_posts(tree, id);
            let slot = peers.iter().position(|peer| peer.id == id)?;
            (peers.len() > 1).then(|| match step {
                KinStep::Previous => peers[(slot + peers.len() - 1) % peers.len()].clone(),
                KinStep::Next => peers[(slot + 1) % peers.len()].clone(),
                KinStep::Parent | KinStep::Children => unreachable!(),
            })
        }
    }
}

fn drag_step(delta: egui::Vec2) -> Option<KinStep> {
    (delta.length_sq() > 1.0).then(|| {
        if delta.x.abs() >= delta.y.abs() {
            if delta.x > 0.0 {
                KinStep::Previous
            } else {
                KinStep::Next
            }
        } else if delta.y > 0.0 {
            KinStep::Parent
        } else {
            KinStep::Children
        }
    })
}

fn cardinal_extent(delta: egui::Vec2, step: KinStep) -> f32 {
    match step {
        KinStep::Previous => delta.x,
        KinStep::Parent => delta.y,
        KinStep::Children => -delta.y,
        KinStep::Next => -delta.x,
    }
    .max(0.0)
}

fn rubber_vec(delta: egui::Vec2, ceiling: f32) -> egui::Vec2 {
    let length = delta.length();
    if length <= f32::EPSILON {
        return egui::Vec2::ZERO;
    }
    delta * (ceiling * (length / ceiling).tanh() / length)
}

fn spring_recoil(t: f32) -> f32 {
    let root = (1.0 - KIN_SPRING_ZETA * KIN_SPRING_ZETA).sqrt();
    let phase = KIN_SPRING_OMEGA * root * t;
    (-KIN_SPRING_ZETA * KIN_SPRING_OMEGA * t).exp()
        * (phase.cos() + KIN_SPRING_ZETA / root * phase.sin())
}

struct TreeLayout {
    centers: BTreeMap<PostId, egui::Vec2>,
    size: egui::Vec2,
}

impl TreeLayout {
    fn forge(tree: &FamilyTree, zoom: f32) -> Self {
        let tile = TREE_TILE * zoom;
        let gap_x = TREE_GAP_X * zoom;
        let gap_y = TREE_GAP_Y * zoom;
        let mut centers = BTreeMap::new();
        let mut visited = HashSet::new();
        let mut leaf = 0_u32;
        let mut depth = 0_usize;
        let _root_x = place_branch(
            tree,
            tree.root,
            0,
            &mut leaf,
            &mut depth,
            &mut visited,
            &mut centers,
            tile,
            gap_x,
            gap_y,
        );
        let size = egui::vec2(
            (leaf.max(1) as f32 * (tile + gap_x) - gap_x).max(tile),
            (depth + 1) as f32 * tile + depth as f32 * gap_y,
        );
        Self { centers, size }
    }
}

fn place_branch(
    tree: &FamilyTree,
    id: PostId,
    level: usize,
    leaf: &mut u32,
    depth: &mut usize,
    visited: &mut HashSet<PostId>,
    centers: &mut BTreeMap<PostId, egui::Vec2>,
    tile: f32,
    gap_x: f32,
    gap_y: f32,
) -> f32 {
    *depth = (*depth).max(level);
    if !visited.insert(id) {
        return centers.get(&id).map_or(0.0, |center| center.x);
    }
    let children = tree
        .node(id)
        .map(|node| node.children.as_slice())
        .unwrap_or_default();
    let x = if children.is_empty() {
        let x = *leaf as f32 * (tile + gap_x) + tile * 0.5;
        *leaf += 1;
        x
    } else {
        let mut span = children.iter().map(|child| {
            place_branch(
                tree,
                *child,
                level + 1,
                leaf,
                depth,
                visited,
                centers,
                tile,
                gap_x,
                gap_y,
            )
        });
        let first = span.next().unwrap_or(0.0);
        let last = span.last().unwrap_or(first);
        (first + last) * 0.5
    };
    let y = level as f32 * (tile + gap_y) + tile * 0.5;
    let _old = centers.insert(id, egui::vec2(x, y));
    x
}

fn wheel(ctx: &egui::Context) -> f32 {
    ctx.input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Line => delta.y * 24.0,
                    egui::MouseWheelUnit::Page => delta.y * 240.0,
                }),
                _ => None,
            })
            .sum()
    })
}

/// Claim an unmodified wheel gesture before the nested scroll area can turn
/// semantic tree zoom into accidental panning.
fn take_wheel(ctx: &egui::Context) -> f32 {
    let (raw, smooth) = ctx.input(|input| {
        let raw = input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } if !modifiers.ctrl && !modifiers.command && !modifiers.alt => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Line => delta.y * 24.0,
                    egui::MouseWheelUnit::Page => delta.y * 240.0,
                }),
                _ => None,
            })
            .sum::<f32>();
        (raw, input.smooth_scroll_delta.y)
    });
    if raw == 0.0 && smooth == 0.0 {
        return 0.0;
    }
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::MouseWheel { modifiers, .. }
                    if !modifiers.ctrl && !modifiers.command && !modifiers.alt
            )
        });
        input.smooth_scroll_delta.y = 0.0;
    });
    if raw == 0.0 { smooth } else { raw }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: u32) -> PostRecord {
        PostRecord {
            id: PostId(id),
            rating: crate::model::Rating::General,
            score: 0,
            favs: 0,
            width: 1,
            height: 1,
            created_at: String::new(),
            tags: Vec::new(),
            tag_hints: Vec::new(),
            preview_url: None,
            thumb_360_url: None,
            thumb_720_url: None,
            large_url: None,
            file_url: None,
        }
    }

    #[test]
    fn down_passage_chooses_leftmost_child() {
        let root = PostId(1);
        let left = PostId(2);
        let right = PostId(3);
        let tree = FamilyTree {
            root,
            focus: root,
            nodes: BTreeMap::from([
                (
                    root,
                    crate::model::FamilyNode {
                        id: root,
                        parent: None,
                        children: vec![left, right],
                        post: Some(post(root.0)),
                        incomplete: false,
                    },
                ),
                (
                    left,
                    crate::model::FamilyNode {
                        id: left,
                        parent: Some(root),
                        children: Vec::new(),
                        post: Some(post(left.0)),
                        incomplete: false,
                    },
                ),
                (
                    right,
                    crate::model::FamilyNode {
                        id: right,
                        parent: Some(root),
                        children: Vec::new(),
                        post: Some(post(right.0)),
                        incomplete: false,
                    },
                ),
            ]),
        };
        assert_eq!(
            direct_kin_target(&tree, root, KinStep::Children).map(|post| post.id),
            Some(left)
        );
    }

    #[test]
    fn global_navigation_falls_back_to_family_entry_tile() {
        let posts = [post(20), post(30), post(40)];
        assert_eq!(gallery_slot(&posts, PostId(10), Some(PostId(30))), Some(1));
        assert_eq!(gallery_slot(&posts, PostId(40), Some(PostId(30))), Some(2));
    }
}
