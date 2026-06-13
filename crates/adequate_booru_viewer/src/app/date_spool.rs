use crate::{chrome, date::CreatedDay};

const YEAR_MIN: i32 = 2005;
const STEP: f32 = 19.0;
const H: f32 = 76.0;
const LIP: f32 = 9.0;
const GAP: f32 = 5.0;
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

#[derive(Clone, Copy, Debug)]
pub(super) struct DateEdit {
    pub value: Option<CreatedDay>,
    pub changed: bool,
    pub pulse: Option<egui::Rect>,
}

#[derive(Clone, Copy, Debug)]
enum Reel {
    Year,
    Month,
    Day,
}

#[derive(Clone, Copy, Debug)]
struct Parts {
    year: i32,
    month: u32,
    day: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Motion {
    kick: f32,
    strain: f32,
    frame: u64,
}

pub(super) fn date_bound(
    ui: &mut egui::Ui,
    id: &'static str,
    label: &'static str,
    value: Option<CreatedDay>,
) -> DateEdit {
    let mut next = value;
    let mut changed = false;
    let mut pulse = None;
    let _label = ui.label(chrome::muted(label));
    let _row = ui.horizontal(|ui| {
        let width = (ui.available_width() - 28.0).max(158.0);
        let turn = chronometer(ui, id, &mut next, width);
        if turn.changed {
            changed = true;
            pulse = Some(turn.rect);
        }
        let clear = chrome::icon_still(ui, "×").on_hover_text("clear date bound");
        if clear.clicked() && next.is_some() {
            next = None;
            changed = true;
            pulse = Some(clear.rect);
        }
    });
    DateEdit {
        value: next,
        changed,
        pulse,
    }
}

#[derive(Clone, Copy, Debug)]
struct Turn {
    changed: bool,
    rect: egui::Rect,
}

fn chronometer(
    ui: &mut egui::Ui,
    id: &'static str,
    value: &mut Option<CreatedDay>,
    width: f32,
) -> Turn {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.min(ui.available_width()), H),
        egui::Sense::click(),
    );
    chrome::shallow_tension(ui, &response);
    let active = value.is_some();
    let mut parts = value.map_or_else(today_parts, |day| day.ymd().into());
    let mut changed = false;
    let reels = reel_rects(rect);
    let hovered_reel = ui
        .ctx()
        .pointer_latest_pos()
        .and_then(|pos| reel_at(pos, reels));
    if response.clicked() && value.is_none() {
        *value = Some(parts.day());
        changed = true;
    }
    if response.hovered()
        && let Some(reel) = hovered_reel
        && let Some(delta) = take_wheel(ui)
    {
        *value = Some(parts.day());
        let spin = delta_steps(delta);
        let mut over = false;
        for _ in 0..spin.steps {
            over |= !parts.spin(reel, spin.dir, year_max());
        }
        changed = !over || *value != Some(parts.day());
        *value = Some(parts.day());
        jolt(ui, id, reel, spin.dir, over);
    }
    if active || changed {
        *value = Some(parts.day());
    }
    paint(ui, id, rect, reels, active || changed, parts);
    Turn { changed, rect }
}

fn paint(
    ui: &egui::Ui,
    id: &'static str,
    rect: egui::Rect,
    reels: [(Reel, egui::Rect); 3],
    active: bool,
    parts: Parts,
) {
    let painter = ui.painter();
    let _back = painter.rect_filled(rect, 2.0, chrome::CONTROL);
    let _edge = painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, chrome::EDGE),
        egui::StrokeKind::Inside,
    );
    for (reel, slot) in reels {
        let motion = settle(ui, egui::Id::new((id, reel as u8)));
        draw_reel(ui, slot, reel, parts, motion, active);
    }
}

fn draw_reel(
    ui: &egui::Ui,
    rect: egui::Rect,
    reel: Reel,
    parts: Parts,
    motion: Motion,
    active: bool,
) {
    let painter = ui.painter();
    let hot = if active { chrome::HOT } else { chrome::MUTED };
    let stroke = if active {
        egui::Stroke::new(1.0, chrome::EDGE_STRONG)
    } else {
        egui::Stroke::new(1.0, chrome::EDGE)
    };
    let _socket = painter.rect_filled(rect, 1.0, chrome::SURFACE);
    let _stroke = painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
    if matches!(reel, Reel::Year) {
        hatch(painter, rect, motion.strain);
    }
    let clip = rect.shrink2(egui::vec2(1.0, LIP));
    let tape = painter.with_clip_rect(clip);
    for lane in -3..=3 {
        let y = rect.center().y + lane as f32 * STEP + motion.kick * STEP + motion.strain * 5.0;
        let text = if active {
            label(reel, parts, lane)
        } else if lane == 0 {
            blank_label(reel).to_owned()
        } else {
            String::new()
        };
        let color = if lane == 0 {
            hot
        } else {
            chrome::MUTED.gamma_multiply(0.65)
        };
        let _glyph = tape.text(
            egui::pos2(rect.center().x, y),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::new(
                if matches!(reel, Reel::Year) {
                    12.0
                } else {
                    13.0
                },
                egui::FontFamily::Monospace,
            ),
            color,
        );
    }
    roller(ui, rect, true);
    roller(ui, rect, false);
    pointer(ui, rect, hot);
}

fn roller(ui: &egui::Ui, rect: egui::Rect, top: bool) {
    let painter = ui.painter();
    let band = if top {
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + LIP))
    } else {
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - LIP), rect.max)
    };
    let _lip = painter.rect_filled(band, 1.0, chrome::RAISED);
    let y = if top { band.bottom() } else { band.top() };
    let _line = painter.line_segment(
        [
            egui::pos2(rect.left() + 2.0, y),
            egui::pos2(rect.right() - 2.0, y),
        ],
        egui::Stroke::new(1.0, chrome::EDGE),
    );
}

fn pointer(ui: &egui::Ui, rect: egui::Rect, color: egui::Color32) {
    let painter = ui.painter();
    let cy = rect.center().y;
    let left = vec![
        egui::pos2(rect.left() - 2.0, cy - 7.0),
        egui::pos2(rect.left() + 7.0, cy - 3.5),
        egui::pos2(rect.left() + 7.0, cy + 3.5),
        egui::pos2(rect.left() - 2.0, cy + 7.0),
    ];
    let right = vec![
        egui::pos2(rect.right() + 2.0, cy - 7.0),
        egui::pos2(rect.right() - 7.0, cy - 3.5),
        egui::pos2(rect.right() - 7.0, cy + 3.5),
        egui::pos2(rect.right() + 2.0, cy + 7.0),
    ];
    let _left = painter.add(egui::Shape::convex_polygon(left, color, egui::Stroke::NONE));
    let _right = painter.add(egui::Shape::convex_polygon(
        right,
        color,
        egui::Stroke::NONE,
    ));
}

fn hatch(painter: &egui::Painter, rect: egui::Rect, strain: f32) {
    if strain.abs() < 0.03 {
        return;
    }
    let high = strain > 0.0;
    let band = if high {
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 15.0), rect.max)
    } else {
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 15.0))
    };
    let ink = egui::Color32::from_rgba_unmultiplied(235, 197, 90, 90);
    for n in -2..8 {
        let x = band.left() + n as f32 * 8.0;
        let _hatch = painter.line_segment(
            [
                egui::pos2(x, band.bottom()),
                egui::pos2(x + 13.0, band.top()),
            ],
            egui::Stroke::new(1.0, ink),
        );
    }
}

fn reel_rects(rect: egui::Rect) -> [(Reel, egui::Rect); 3] {
    let w = ((rect.width() - 2.0 * GAP) / 3.0).floor();
    let year_w = (w + 16.0).min(64.0);
    let month_w = (w - 4.0).max(42.0);
    let day_w = (rect.width() - year_w - month_w - 2.0 * GAP).max(36.0);
    let x = rect.left() + 6.0;
    let y = rect.top() + 5.0;
    let h = rect.height() - 10.0;
    let year = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(year_w, h));
    let month =
        egui::Rect::from_min_size(egui::pos2(year.right() + GAP, y), egui::vec2(month_w, h));
    let day = egui::Rect::from_min_size(
        egui::pos2(month.right() + GAP, y),
        egui::vec2(day_w - 6.0, h),
    );
    [(Reel::Year, year), (Reel::Month, month), (Reel::Day, day)]
}

fn reel_at(pos: egui::Pos2, reels: [(Reel, egui::Rect); 3]) -> Option<Reel> {
    reels
        .into_iter()
        .find_map(|(reel, rect)| rect.expand(3.0).contains(pos).then_some(reel))
}

#[derive(Clone, Copy)]
struct Spin {
    dir: i32,
    steps: u32,
}

fn delta_steps(delta: f32) -> Spin {
    Spin {
        dir: if delta < 0.0 { 1 } else { -1 },
        steps: ((delta.abs() / 72.0).round() as u32).clamp(1, 16),
    }
}

fn take_wheel(ui: &mut egui::Ui) -> Option<f32> {
    let delta = ui.input(|input| {
        input
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
                    egui::MouseWheelUnit::Line => delta.y * 72.0,
                    egui::MouseWheelUnit::Page => delta.y * 240.0,
                }),
                _ => None,
            })
            .sum::<f32>()
    });
    if delta == 0.0 {
        return None;
    }
    ui.input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::MouseWheel {
                    modifiers,
                    ..
                } if !modifiers.ctrl && !modifiers.command && !modifiers.alt
            )
        });
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
    Some(delta)
}

fn settle(ui: &egui::Ui, id: egui::Id) -> Motion {
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|data| {
        let mut motion = data.get_temp::<Motion>(id).unwrap_or(Motion {
            frame,
            ..Motion::default()
        });
        let dt = frame.saturating_sub(motion.frame).min(8) as f32;
        if dt > 0.0 {
            motion.kick *= 0.72_f32.powf(dt);
            motion.strain *= 0.78_f32.powf(dt);
            motion.frame = frame;
        }
        if motion.kick.abs() > 0.01 || motion.strain.abs() > 0.01 {
            ui.ctx().request_repaint();
        }
        let _old = data.insert_temp(id, motion);
        motion
    })
}

fn jolt(ui: &egui::Ui, id: &'static str, reel: Reel, dir: i32, over: bool) {
    let key = egui::Id::new((id, reel as u8));
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|data| {
        let mut motion = data.get_temp::<Motion>(key).unwrap_or(Motion {
            frame,
            ..Motion::default()
        });
        if over {
            motion.strain = (motion.strain + dir as f32 * 1.15).clamp(-2.0, 2.0);
        } else {
            motion.kick = (-dir as f32).clamp(-1.0, 1.0);
        }
        motion.frame = frame;
        let _old = data.insert_temp(key, motion);
    });
    ui.ctx().request_repaint();
}

fn label(reel: Reel, parts: Parts, offset: i32) -> String {
    match reel {
        Reel::Year => format!("{:04}", parts.year + offset),
        Reel::Month => MONTHS[wrap(parts.month as i32 - 1 + offset, 12) as usize].to_owned(),
        Reel::Day => {
            let days = CreatedDay::days_in_month(parts.year, parts.month) as i32;
            format!("{:02}", wrap(parts.day as i32 - 1 + offset, days) + 1)
        }
    }
}

fn blank_label(reel: Reel) -> &'static str {
    match reel {
        Reel::Year => "────",
        Reel::Month => "───",
        Reel::Day => "──",
    }
}

impl Parts {
    fn spin(&mut self, reel: Reel, dir: i32, year_max: i32) -> bool {
        match reel {
            Reel::Year => {
                let next = self.year + dir;
                if !(YEAR_MIN..=year_max).contains(&next) {
                    return false;
                }
                self.year = next;
                self.clamp_day();
            }
            Reel::Month => {
                self.month = (wrap(self.month as i32 - 1 + dir, 12) + 1) as u32;
                self.clamp_day();
            }
            Reel::Day => {
                let days = CreatedDay::days_in_month(self.year, self.month) as i32;
                self.day = (wrap(self.day as i32 - 1 + dir, days) + 1) as u32;
            }
        }
        true
    }

    fn clamp_day(&mut self) {
        self.day = self
            .day
            .min(CreatedDay::days_in_month(self.year, self.month));
    }

    fn day(self) -> CreatedDay {
        if let Some(day) = CreatedDay::from_ymd(self.year, self.month, self.day) {
            day
        } else {
            CreatedDay::today_utc()
        }
    }
}

impl From<(i32, u32, u32)> for Parts {
    fn from((year, month, day): (i32, u32, u32)) -> Self {
        Self { year, month, day }
    }
}

fn today_parts() -> Parts {
    CreatedDay::today_utc().ymd().into()
}

fn year_max() -> i32 {
    today_parts().year + 1
}

fn wrap(value: i32, modulus: i32) -> i32 {
    value.rem_euclid(modulus.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_month_clamps_day() {
        let mut parts = Parts {
            year: 2024,
            month: 3,
            day: 31,
        };
        assert!(parts.spin(Reel::Month, -1, 2026));
        assert_eq!((parts.year, parts.month, parts.day), (2024, 2, 29));
    }

    #[test]
    fn circular_day_wraps_inside_month() {
        let mut parts = Parts {
            year: 2025,
            month: 4,
            day: 30,
        };
        assert!(parts.spin(Reel::Day, 1, 2026));
        assert_eq!(parts.day, 1);
    }

    #[test]
    fn year_limits_refuse_motion() {
        let mut parts = Parts {
            year: YEAR_MIN,
            month: 1,
            day: 1,
        };
        assert!(!parts.spin(Reel::Year, -1, 2026));
        assert_eq!(parts.year, YEAR_MIN);
    }
}
