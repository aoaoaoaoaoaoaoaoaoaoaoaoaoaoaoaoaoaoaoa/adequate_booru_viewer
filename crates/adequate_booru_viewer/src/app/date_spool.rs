use crate::{chrome, date::CreatedDay};

const WHEEL_CLAIM: &str = "date-spool-wheel-claim";
const YEAR_MIN: i32 = 2005;
const STEP: f32 = 19.0;
const H: f32 = 76.0;
const LIP: f32 = 9.0;
const PAD: f32 = 12.0;
const REEL_GAP: f32 = 12.0;
const TAPE_GAP: f32 = 3.0;
const TAPE_EDGE: egui::Color32 = egui::Color32::from_rgb(91, 73, 47);
const TAPE_FACE: egui::Color32 = egui::Color32::from_rgb(39, 32, 22);
const TAPE_FACE_DIM: egui::Color32 = egui::Color32::from_rgb(26, 22, 17);
const WELL: egui::Color32 = egui::Color32::from_rgb(8, 7, 6);
const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

#[derive(Clone, Copy, Debug)]
pub(super) struct DateEdit {
    pub value: Option<CreatedDay>,
    pub changed: bool,
    pub pulse: Option<DatePulse>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DatePulse {
    Tape(egui::Rect),
    Button(egui::Rect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default)]
struct DragTape {
    reel: Option<Reel>,
    y: f32,
    carry: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct WheelTape {
    carry: f32,
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
        }
        if turn.impulse {
            pulse = Some(DatePulse::Tape(turn.rect));
        }
        let icon = if next.is_some() { "×" } else { "+" };
        let hint = if next.is_some() {
            "clear date bound"
        } else {
            "arm date bound at today"
        };
        let action = chrome::icon_still(ui, icon).on_hover_text(hint);
        if action.clicked() {
            next = next.is_none().then(CreatedDay::today_utc);
            changed = true;
            pulse = Some(DatePulse::Button(action.rect));
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
    impulse: bool,
    rect: egui::Rect,
}

pub(super) fn take_wheel_claim(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        data.remove_temp::<bool>(egui::Id::new(WHEEL_CLAIM))
            .unwrap_or(false)
    })
}

fn chronometer(
    ui: &mut egui::Ui,
    id: &'static str,
    value: &mut Option<CreatedDay>,
    width: f32,
) -> Turn {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width.min(ui.available_width()), H),
        egui::Sense::click_and_drag(),
    );
    let active = value.is_some();
    let mut parts = value.map_or_else(today_parts, |day| day.ymd().into());
    let mut changed = false;
    let reels = reel_rects(rect);
    let hovered_reel = ui
        .ctx()
        .pointer_latest_pos()
        .and_then(|pos| reel_at(pos, reels));
    let mut impulse = false;
    let mut impulse_rect = rect;
    if active
        && response.hovered()
        && let Some(reel) = hovered_reel
        && let Some(spin) = wheel_spin(ui, id, reel)
    {
        let before = *value;
        let mut over = false;
        for _ in 0..spin.steps {
            over |= !parts.spin(reel, spin.dir, year_max());
        }
        *value = Some(parts.day());
        changed = before != *value;
        impulse = true;
        impulse_rect = reel_rect(reels, reel);
        jolt(ui, id, reel, spin.dir, over);
        swallow_wheel(ui);
    } else if active && let Some((reel, spin)) = drag_spin(ui, id, &response, hovered_reel) {
        let before = *value;
        let mut over = false;
        for _ in 0..spin.steps {
            over |= !parts.spin(reel, spin.dir, year_max());
        }
        *value = Some(parts.day());
        changed = before != *value;
        impulse = true;
        impulse_rect = reel_rect(reels, reel);
        jolt(ui, id, reel, spin.dir, over);
    }
    paint(ui, id, rect, reels, active || changed, parts);
    Turn {
        changed,
        impulse,
        rect: impulse_rect,
    }
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
    let well = well_rect(rect);
    let strip = tape_rect(rect);
    let face = if active { TAPE_FACE } else { TAPE_FACE_DIM };
    let _well = painter.rect_filled(well, 1.0, WELL);
    let _tape = painter.rect_filled(strip, 1.0, face);
    let _left_edge = painter.line_segment(
        [
            egui::pos2(strip.left(), strip.top() + 1.0),
            egui::pos2(strip.left(), strip.bottom() - 1.0),
        ],
        egui::Stroke::new(1.0, TAPE_EDGE),
    );
    let _right_edge = painter.line_segment(
        [
            egui::pos2(strip.right(), strip.top() + 1.0),
            egui::pos2(strip.right(), strip.bottom() - 1.0),
        ],
        egui::Stroke::new(1.0, TAPE_EDGE),
    );
    if matches!(reel, Reel::Year) {
        let hatched = painter.with_clip_rect(strip);
        hatch(&hatched, strip, motion.strain);
    }
    for lane in -4..=4 {
        let Some(view) = LaneView::project(strip, lane, motion) else {
            continue;
        };
        let text = if active {
            label(reel, parts, lane)
        } else if lane == 0 {
            blank_label(reel).to_owned()
        } else {
            String::new()
        };
        let ink = if lane == 0 {
            hot
        } else {
            chrome::MUTED.gamma_multiply(0.65)
        };
        cylindrical_text(
            ui,
            strip,
            egui::pos2(strip.center().x, view.y),
            egui::FontId::new(
                if matches!(reel, Reel::Year) {
                    12.0
                } else {
                    13.0
                } * view.scale,
                egui::FontFamily::Monospace,
            ),
            &text,
            ink,
        );
    }
    roller(ui, well, true);
    roller(ui, well, false);
    let _stroke = painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Inside);
    pointer(ui, rect, hot);
}

#[derive(Clone, Copy, Debug)]
struct LaneView {
    y: f32,
    scale: f32,
}

impl LaneView {
    fn project(rect: egui::Rect, lane: i32, motion: Motion) -> Option<Self> {
        let r = (rect.height() - 2.0 * LIP) * 0.58;
        let theta = (lane as f32 * STEP + motion.kick * STEP + motion.strain * 5.0) / r;
        if theta.abs() > 1.48 {
            return None;
        }
        let face = theta.cos().max(0.0);
        Some(Self {
            y: rect.center().y + theta.sin() * r,
            scale: 0.62 + 0.38 * face,
        })
    }
}

fn cylindrical_text(
    ui: &egui::Ui,
    strip: egui::Rect,
    center: egui::Pos2,
    font: egui::FontId,
    text: &str,
    color: egui::Color32,
) {
    if text.is_empty() {
        return;
    }
    let painter = ui.painter().with_clip_rect(strip);
    let Some(mut shape) = painter.fonts_mut(|fonts| {
        match egui::Shape::text(
            fonts,
            center,
            egui::Align2::CENTER_CENTER,
            text,
            font,
            color,
        ) {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        }
    }) else {
        return;
    };
    bend_text(strip, &mut shape);
    let _glyphs = painter.add(shape);
}

fn bend_text(strip: egui::Rect, shape: &mut egui::epaint::TextShape) {
    let center = strip.center().y;
    let r = strip.height() * 0.62;
    let galley = std::sync::Arc::make_mut(&mut shape.galley);
    galley.mesh_bounds = egui::Rect::NOTHING;
    galley.rect = egui::Rect::NOTHING;
    for row in &mut galley.rows {
        let row_pos = row.pos;
        let row = std::sync::Arc::make_mut(&mut row.row);
        let mut bounds = egui::Rect::NOTHING;
        for vertex in &mut row.visuals.mesh.vertices {
            let local = row_pos + vertex.pos.to_vec2();
            let world = shape.pos + local.to_vec2();
            let theta = ((world.y - center) / r).clamp(-1.43, 1.43);
            let face = theta.cos().max(0.08);
            let bowed = egui::pos2(
                strip.center().x + (world.x - strip.center().x) * (0.72 + 0.28 * face),
                center + theta.sin() * r,
            );
            vertex.pos = (bowed - shape.pos - row_pos.to_vec2()).to_pos2();
            bounds.extend_with(vertex.pos);
        }
        if !bounds.is_positive() {
            continue;
        }
        row.visuals.mesh_bounds = bounds;
        galley
            .mesh_bounds
            .extend_with(row_pos + bounds.min.to_vec2());
        galley
            .mesh_bounds
            .extend_with(row_pos + bounds.max.to_vec2());
    }
    galley.rect = galley.mesh_bounds;
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
    let half_base = 4.9;
    let half_tip = half_base * 0.075;
    let base = rect.right() + 1.0;
    let tip = rect.right() - 8.0;
    let needle = vec![
        egui::pos2(base, cy - half_base),
        egui::pos2(base, cy + half_base),
        egui::pos2(tip, cy + half_tip),
        egui::pos2(tip, cy - half_tip),
    ];
    let _needle = painter.add(egui::Shape::convex_polygon(
        needle,
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
    let inner = (rect.width() - 2.0 * PAD).max(130.0);
    let scale = (inner / 176.0).min(1.0);
    let year_w = 58.0 * scale;
    let month_w = 42.0 * scale;
    let day_w = 38.0 * scale;
    let gap = REEL_GAP * scale;
    let used = year_w + month_w + day_w + 2.0 * gap;
    let x = rect.left() + ((rect.width() - used) * 0.5).max(PAD);
    let y = rect.top() + 5.0;
    let h = rect.height() - 10.0;
    let year = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(year_w, h));
    let month =
        egui::Rect::from_min_size(egui::pos2(year.right() + gap, y), egui::vec2(month_w, h));
    let day = egui::Rect::from_min_size(egui::pos2(month.right() + gap, y), egui::vec2(day_w, h));
    [(Reel::Year, year), (Reel::Month, month), (Reel::Day, day)]
}

fn well_rect(rect: egui::Rect) -> egui::Rect {
    rect.shrink2(egui::vec2(3.0, 4.0))
}

fn aperture_rect(rect: egui::Rect) -> egui::Rect {
    well_rect(rect).shrink2(egui::vec2(1.0, LIP))
}

fn tape_rect(rect: egui::Rect) -> egui::Rect {
    aperture_rect(rect).shrink2(egui::vec2(TAPE_GAP, TAPE_GAP))
}

fn reel_at(pos: egui::Pos2, reels: [(Reel, egui::Rect); 3]) -> Option<Reel> {
    reels
        .into_iter()
        .find_map(|(reel, rect)| rect.expand(3.0).contains(pos).then_some(reel))
}

fn reel_rect(reels: [(Reel, egui::Rect); 3], needle: Reel) -> egui::Rect {
    reels
        .into_iter()
        .find_map(|(reel, rect)| (reel == needle).then_some(tape_rect(rect)))
        .unwrap_or(egui::Rect::NOTHING)
}

#[derive(Clone, Copy)]
struct Spin {
    dir: i32,
    steps: u32,
}

fn wheel_spin(ui: &egui::Ui, id: &'static str, reel: Reel) -> Option<Spin> {
    let delta = wheel_delta(ui)?;
    let key = egui::Id::new((id, reel as u8, "wheel-tape"));
    ui.ctx().data_mut(|data| {
        let mut wheel = data.get_temp::<WheelTape>(key).unwrap_or_default();
        if wheel.carry.signum() != delta.signum() {
            wheel.carry = 0.0;
        }
        wheel.carry += delta.clamp(-1.0, 1.0);
        if wheel.carry.abs() < 1.0 {
            let _old = data.insert_temp(key, wheel);
            return None;
        }
        let dir = if wheel.carry < 0.0 { 1 } else { -1 };
        wheel.carry = 0.0;
        let _old = data.insert_temp(key, wheel);
        Some(Spin { dir, steps: 1 })
    })
}

fn drag_spin(
    ui: &egui::Ui,
    id: &'static str,
    response: &egui::Response,
    hovered_reel: Option<Reel>,
) -> Option<(Reel, Spin)> {
    const QUANTUM: f32 = 11.0;
    let key = egui::Id::new((id, "drag-tape"));
    ui.ctx().data_mut(|data| {
        if response.drag_stopped() {
            let _old = data.remove_temp::<DragTape>(key);
            return None;
        }
        if !response.dragged() {
            return None;
        }
        let y = response.drag_delta().y;
        let mut drag = data.get_temp::<DragTape>(key).unwrap_or(DragTape {
            reel: hovered_reel,
            y,
            carry: 0.0,
        });
        if drag.reel.is_none() {
            drag.reel = hovered_reel;
        }
        let reel = drag.reel?;
        drag.carry += y - drag.y;
        drag.y = y;
        let steps = (drag.carry.abs() / QUANTUM).floor() as u32;
        if steps == 0 {
            let _old = data.insert_temp(key, drag);
            return None;
        }
        let slip = drag.carry.signum() * steps as f32 * QUANTUM;
        drag.carry -= slip;
        let _old = data.insert_temp(key, drag);
        Some((
            reel,
            Spin {
                dir: if slip < 0.0 { 1 } else { -1 },
                steps: steps.min(16),
            },
        ))
    })
}

fn wheel_delta(ui: &egui::Ui) -> Option<f32> {
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
                    egui::MouseWheelUnit::Point => delta.y / 80.0,
                    egui::MouseWheelUnit::Line => delta.y / 3.0,
                    egui::MouseWheelUnit::Page => delta.y.signum(),
                }),
                _ => None,
            })
            .sum::<f32>()
    });
    (delta.abs() > f32::EPSILON).then_some(delta)
}

fn swallow_wheel(ui: &egui::Ui) {
    ui.ctx().input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::MouseWheel {
                    modifiers,
                    ..
                } if !modifiers.ctrl && !modifiers.command && !modifiers.alt
            )
        });
        input.smooth_scroll_delta.y = 0.0;
    });
    ui.ctx().data_mut(|data| {
        let _old = data.insert_temp(egui::Id::new(WHEEL_CLAIM), true);
    });
}

fn settle(ui: &egui::Ui, id: egui::Id) -> Motion {
    let frame = ui.ctx().cumulative_frame_nr();
    let motion = ui.ctx().data_mut(|data| {
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
        let _old = data.insert_temp(id, motion);
        motion
    });
    if motion.kick.abs() > 0.01 || motion.strain.abs() > 0.01 {
        ui.ctx().request_repaint();
    }
    motion
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

    #[test]
    fn settling_motion_does_not_reenter_context_lock() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("armed-spool-motion");
        ctx.data_mut(|data| {
            let _old = data.insert_temp(
                id,
                Motion {
                    kick: 1.0,
                    strain: 0.0,
                    frame: 0,
                },
            );
        });
        let _output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 180.0),
                )),
                ..Default::default()
            },
            |ui| {
                let motion = settle(ui, id);
                assert!(motion.kick > 0.01);
            },
        );
    }
}
