use super::*;

const CARD_W: f32 = 250.0;
const CARD_H: f32 = 150.0;
const DWELL: Duration = Duration::from_millis(180);

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
        self.loading_card(ui, arena);
    }

    fn loading_card(&mut self, ui: &mut egui::Ui, arena: egui::Rect) {
        let size = egui::vec2(
            CARD_W.min((arena.width() - 24.0).max(120.0)),
            CARD_H.min((arena.height() - 24.0).max(96.0)),
        );
        let rect = egui::Rect::from_center_size(arena.center(), size);
        if self.water_ui.wet() {
            self.loading_raft.show(ui.ctx(), rect);
            self.arm_water();
        } else {
            self.loading_raft.hide();
        }

        let painter = ui.painter();
        let _fill = painter.rect_filled(rect, 2.0, chrome::SURFACE);
        let _stroke = painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, chrome::EDGE_STRONG),
            egui::StrokeKind::Inside,
        );
        let title_font = egui::FontId::new(25.0, egui::FontFamily::Proportional);
        let detail_font = egui::FontId::new(38.0, egui::FontFamily::Proportional);
        let LoadingCard { title, detail } = self.loading_state();
        let title_galley =
            painter.layout_no_wrap(title.to_owned(), title_font.clone(), chrome::HOT);
        let detail_galley =
            painter.layout_no_wrap(detail.clone(), detail_font.clone(), chrome::TEXT);
        let title_at = egui::pos2(
            rect.center().x - title_galley.size().x * 0.5,
            rect.top() + 28.0,
        );
        let detail_at = egui::pos2(
            rect.center().x - detail_galley.size().x * 0.5,
            rect.center().y + 7.0,
        );
        let _title = painter.text(
            title_at,
            egui::Align2::LEFT_TOP,
            title,
            title_font,
            chrome::HOT,
        );
        let _detail = painter.text(
            detail_at,
            egui::Align2::LEFT_TOP,
            detail,
            detail_font,
            chrome::TEXT,
        );
    }

    fn loading_state(&self) -> LoadingCard {
        if self.warm_state == WarmState::Exhausted {
            LoadingCard {
                title: "NO MATCHES",
                detail: "0 HITS".to_owned(),
            }
        } else {
            LoadingCard {
                title: "LOADING",
                detail: query_warm_percent(self.warm_state, self.warm_next_page),
            }
        }
    }
}

struct LoadingCard {
    title: &'static str,
    detail: String,
}

fn query_warm_percent(state: WarmState, next_page: u32) -> String {
    if state == WarmState::Exhausted {
        return "100%".to_owned();
    }
    let pages = next_page.saturating_sub(1);
    rough_percent(100.0 * pages as f32 / DANBOORU_SEARCH_PAGE_LIMIT as f32)
}

fn rough_percent(value: f32) -> String {
    let value = value.clamp(0.0, 100.0);
    if value < 1.0 {
        format!("{value:.4}%")
    } else if value < 10.0 {
        format!("{value:.3}%")
    } else {
        format!("{value:.2}%")
    }
}
