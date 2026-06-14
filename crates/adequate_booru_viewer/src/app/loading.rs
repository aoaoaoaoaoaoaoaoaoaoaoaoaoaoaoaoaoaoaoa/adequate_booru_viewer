use super::*;

const CARD_W: f32 = 250.0;
const CARD_H: f32 = 150.0;
const DWELL: Duration = Duration::from_millis(180);
const TILE: f32 = 31.0;

#[derive(Clone, Copy)]
enum EmptyState {
    Loading,
    Warming,
    Settled,
}

impl EmptyState {
    fn label(self) -> &'static str {
        match self {
            Self::Loading => "LOADING",
            Self::Warming => "WARMING",
            Self::Settled => "EMPTY",
        }
    }

    fn raised(self) -> bool {
        matches!(self, Self::Loading | Self::Warming)
    }

    fn draining(self) -> bool {
        matches!(self, Self::Settled)
    }
}

impl Bayonet {
    pub(super) fn empty_gallery(&mut self, ui: &mut egui::Ui, arena: egui::Rect) {
        let now = Instant::now();
        let empty_since = *self.empty_since.get_or_insert(now);
        let age = now.saturating_duration_since(empty_since);
        if age < DWELL {
            self.loading_raft.hide();
            ui.ctx().request_repaint_after(DWELL.saturating_sub(age));
            return;
        }
        self.loading_card(ui, arena, self.empty_state());
    }

    fn empty_state(&self) -> EmptyState {
        if self.refresh_pulse.inflight_serial().is_some() {
            EmptyState::Loading
        } else if !self.date_range.active() && self.warm_state == WarmState::InFlight {
            EmptyState::Warming
        } else {
            EmptyState::Settled
        }
    }

    fn loading_card(&mut self, ui: &mut egui::Ui, arena: egui::Rect, state: EmptyState) {
        let size = egui::vec2(
            CARD_W.min((arena.width() - 24.0).max(120.0)),
            CARD_H.min((arena.height() - 24.0).max(96.0)),
        );
        let rect = egui::Rect::from_center_size(arena.center(), size);
        if state.raised() && self.water_mode.wet() {
            self.loading_raft.show(ui.ctx(), rect);
            self.arm_water();
        } else {
            self.loading_raft.hide();
        }
        if state.draining() && self.water_mode.wet() {
            for drain in self.empty_drain.show(ui.ctx(), rect) {
                self.drain_plunge(drain.rect, drain.amp);
            }
        } else {
            self.empty_drain.hide();
        }

        let painter = ui.painter();
        let _fill = painter.rect_filled(rect, 2.0, chrome::SURFACE);
        pool_tiles(painter, rect.shrink(9.0), state);
        let _stroke = painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, chrome::EDGE_STRONG),
            egui::StrokeKind::Inside,
        );
        let font = egui::FontId::new(36.0, egui::FontFamily::Proportional);
        let text = state.label();
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), chrome::HOT);
        let at = rect.center() - galley.size() * 0.5;
        let _text = painter.text(at, egui::Align2::LEFT_TOP, text, font, chrome::HOT);
    }
}

fn pool_tiles(painter: &egui::Painter, rect: egui::Rect, state: EmptyState) {
    let glow = if state.draining() { 70 } else { 42 };
    let grout = egui::Color32::from_rgba_unmultiplied(235, 197, 151, glow);
    let shade = egui::Color32::from_rgba_unmultiplied(235, 197, 151, glow / 3);
    let clip = painter.with_clip_rect(rect);
    let mut x = rect.left() + TILE;
    while x < rect.right() {
        let _line = clip.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, grout),
        );
        x += TILE;
    }
    let mut y = rect.top() + TILE;
    while y < rect.bottom() {
        let _line = clip.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            egui::Stroke::new(1.0, grout),
        );
        y += TILE;
    }
    let _inner = clip.rect_stroke(
        rect,
        1.0,
        egui::Stroke::new(1.0, shade),
        egui::StrokeKind::Inside,
    );
}
