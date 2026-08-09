use super::*;

const CARD_W: f32 = 250.0;
const CARD_H: f32 = 150.0;
const DWELL: Duration = Duration::from_millis(180);

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
            self.water.set_floor(None);
            ui.ctx().request_repaint_after(DWELL.saturating_sub(age));
            return;
        }
        self.loading_card(ui, arena, self.empty_state());
    }

    fn empty_state(&self) -> EmptyState {
        if self.refresh_pulse.inflight_serial().is_some() {
            EmptyState::Loading
        } else if !self.date_range.active() && self.warm.active().state == WarmState::InFlight {
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
        self.water
            .set_floor(Some(crate::water::Floor::shallow(arena)));
        if state.raised() {
            self.water.hide_drain();
            let _rect = self.living_wait.bouncer_with(ui, arena, state.label());
            return;
        }
        if state.draining() && self.water_mode.wet() {
            self.water.show_drain(ui.ctx(), rect);
        } else {
            self.water.hide_drain();
        }

        let painter = ui.painter();
        let _fill = painter.rect_filled(rect, 2.0, chrome::SURFACE);
        let _stroke = painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0_f32, chrome::EDGE_STRONG),
            egui::StrokeKind::Inside,
        );
        let font = egui::FontId::new(36.0, egui::FontFamily::Proportional);
        let text = state.label();
        let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), chrome::HOT);
        let at = rect.center() - galley.size() * 0.5;
        let _text = painter.text(at, egui::Align2::LEFT_TOP, text, font, chrome::HOT);
    }
}
