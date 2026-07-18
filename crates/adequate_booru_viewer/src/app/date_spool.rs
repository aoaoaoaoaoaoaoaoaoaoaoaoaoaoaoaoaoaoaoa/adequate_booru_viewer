//! Date bounds as an austere analog tape-transport — a dreamlike, submerged
//! poolroom fitting. Three black recesses are sunk into a dim tiled faceplate;
//! down in each runs a strip of near-black magnetic tape that lies flat across
//! a reading head and vanishes over a conveyor roller at either end (a
//! cassette/conveyor profile, seen through a perspective eye), the labels
//! *printed* on the tape and bending over the rollers exactly as in a 3D scene.
//! Only the rollers' capped, spindled ends show past the ribbon, their welded
//! notch turning in lockstep with the tape; a brass index arrow welded to each
//! recess wall points at the value. One physical model — a lightly underdamped
//! spring on a continuous roll offset — drives wheel, drag, and the elastic
//! recoil when the year reel is shoved past its stops; spinning a reel also
//! drags the surrounding water (see `app::water`).

use crate::{chrome, date::CreatedDay};

const WHEEL_CLAIM: &str = "date-spool-wheel-claim";
const YEAR_MIN: i32 = 2005;
const H: f32 = 104.0;
const RECESS_TILES: f32 = 4.0; // each recess is this many tiles wide

// --- The tape path, as a 3D scene --------------------------------------------
// Not a circular dial: a cassette ribbon seen edge-on. The middle of the strip
// lies on a gentle arc (radius R_FLAT — labels nearly full size, readable),
// and past the surface angle BETA_F it wraps a *tight* roller (radius R_ROLL)
// that races the angle to BETA_MAX and curls out of sight, bunching labels.
// All lengths are in aperture half-heights, so the profile scales with the
// reel; a perspective eye CAM_D back folds width as the tape recedes.
const R_FLAT: f32 = 3.8; // flat-run radius (nearly straight) / aperture half-height
const R_ROLL: f32 = 0.50; // roller radius — a tight pulley the tape wraps 180°
const BETA_F: f32 = 0.19; // surface angle where the flat hands off to the roller
// The tape wraps the roller and disappears: we render the front of the wrap,
// curling to edge-on, then the black recess swallows the far side.
const BETA_MAX: f32 = 1.46;
// The ribbon's *print* dissolves over the last stretch of curl before the apex:
// past readability the perspective fold barely shrinks a glyph (fold ≈ 0.83 even
// at the rim), so without this every label beyond the rim collapses to near-full
// width at the apex and consecutive ones pile into a ghost just below the roller.
const CURL_FADE: f32 = 0.55; // radians of curl over which the print fades to nothing
const CAM_D: f32 = 2.4; // eye distance
const PITCH: f32 = 0.46; // label spacing along the tape
const ROLL_GAIN: f32 = PITCH / R_ROLL; // roller turn per pitch of tape (no slip)

// --- Light (warm key, just above front) + Blinn-Phong gloss ------------------
// Nearly head-on: on the almost-flat run the surface angle barely varies, so a
// strong tilt would throw the whole gloss onto one label. A gentle tilt keeps
// the sheen centred on the value at the reading head and still shades the curl.
const L_Y: f32 = -0.180; // key direction, screen-down component (up is −y)
const L_Z: f32 = 0.984; // ...toward the eye
const HV_Y: f32 = -0.090; // half-vector with V=(0,0,1)
const HV_Z: f32 = 0.996;
const SHINE: f32 = 26.0;
const AMBIENT: f32 = 0.17; // floor the curl rides on — lift it clear of the WELL
const DIFFUSE: f32 = 0.34;
const GLOSS: f32 = 0.15;

// Real magnetic tape: a near-black, near-neutral body, with only the faint
// violet iridescence its sheen flashes back — the dark ribbon in the recess.
const ALBEDO: [f32; 3] = [16.0, 14.0, 18.0];
const SPARK: [f32; 3] = [80.0, 58.0, 112.0];
const WELL: egui::Color32 = egui::Color32::from_rgb(5, 5, 6); // recess void
const GUTTER: egui::Color32 = egui::Color32::from_rgb(8, 8, 10);

// --- The metal: quiet bronze accents — caps, spindles, the index — over a dark
// tiled void. The faceplate itself is the dim poolroom mosaic, not brass.
const BRONZE_LO: [f32; 3] = [34.0, 28.0, 19.0]; // shadowed metal
const BRONZE_MD: [f32; 3] = [104.0, 86.0, 58.0]; // the metal's body
const BRONZE_HI: [f32; 3] = [196.0, 170.0, 124.0]; // a lit edge / glint
const TILE_BASE: [f32; 3] = [34.0, 28.0, 20.0]; // pool-tile body (baseline dark)
const TILE_LIFT: [f32; 3] = [17.0, 14.0, 9.0]; // per-tile brightness scatter
const TILE_CAST: f32 = 5.0; // per-tile warm/cool tint scatter (±, r vs b)
const GROUT: egui::Color32 = egui::Color32::from_rgb(17, 13, 8); // tile seam
const TILE: f32 = 15.6; // pool-tile pitch (px)
const CAP_W: f32 = 3.0; // roller end-cap width — the cylinder end seen edge-on
const TAPE_THK: f32 = 1.0; // tape thickness: gap from the tape's top to the cap's
const RECESS_EDGE: egui::Color32 = egui::Color32::from_rgb(46, 38, 27); // recess border

// --- Spring + stops ----------------------------------------------------------
const SPRING_K: f32 = 560.0; // stiffness (ω≈23.7 rad/s, ~0.26s settle)
const SPRING_C: f32 = 24.0; // damping (ζ≈0.5 → a lively overshoot)
const COMMIT: f32 = 0.5; // roll past this and a new label has reached the head
const WALL_STRETCH: f32 = 0.62; // how far the tape gives past a hard stop (pitches)
const WALL_KICK: f32 = 7.0; // recoil velocity slammed back off the stop
const HAZARD_GAP: f32 = 0.72; // blank tape (pitches) between last year and hatch
// Arm/clear: a snappy spring-tensioned lever — tens of ms, with a little bounce
// on the up-swing; the down-swing just slams to the floor of the void.
const LIFT_K: f32 = 1100.0; // stiffness (ω≈33 rad/s, ~0.15s — fast but legible)
const LIFT_C: f32 = 28.0; // damping (ζ≈0.42 → a small overshoot up)

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

#[derive(Clone, Copy, Debug)]
pub(super) struct DateEdit {
    pub value: Option<CreatedDay>,
    pub changed: bool,
    pub pulse: Option<DatePulse>,
    /// A second reel dragged along by a mechanical coupling this frame (the day
    /// rolling to its clamp when the month/year turns) — its own water shove.
    pub couple: Option<DatePulse>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum DatePulse {
    /// A spun reel: its window, and the signed screen-y its tape head travels
    /// (so the water can be shoved off that edge).
    Tape(egui::Rect, f32),
    /// Armed (+1, rollers rise → water pushed out) or cleared (−1, rollers sink
    /// into the void → water sucked in).
    Lever(egui::Rect, f32),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Reel {
    #[default]
    Year,
    Month,
    Day,
}

impl Reel {
    const ALL: [Reel; 3] = [Reel::Year, Reel::Month, Reel::Day];
}

#[derive(Clone, Copy, Debug)]
struct Parts {
    year: i32,
    month: u32,
    day: u32,
}

/// One reel's living state: the visual roll offset (in label pitches) of the
/// committed value away from the head, and the spring velocity carrying it
/// home. `notch` banks fractional wheel travel between frames; `turns`
/// accumulates committed steps so the rollers can spin in lockstep with the
/// tape (visual tape position = `turns - roll`).
#[derive(Clone, Copy, Debug, Default)]
struct Drum {
    roll: f32,
    vel: f32,
    notch: f32,
    turns: f32,
}

impl Drum {
    /// Relax the roll toward the detent (or toward a clamped stop), returning
    /// whether the reel is still in motion and wants another frame.
    fn relax(&mut self, dt: f32, floor: f32, ceil: f32) -> bool {
        self.vel += (-SPRING_K * self.roll - SPRING_C * self.vel) * dt;
        self.roll += self.vel * dt;
        if self.roll < floor {
            self.roll = floor;
            self.vel = self.vel.max(0.0);
        } else if self.roll > ceil {
            self.roll = ceil;
            self.vel = self.vel.min(0.0);
        }
        self.roll.abs() > 5e-4 || self.vel.abs() > 5e-4
    }
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
    let mut couple = None;
    let _label = ui.label(chrome::muted(label));
    let _row = ui.horizontal(|ui| {
        let width = (ui.available_width() - 28.0).max(158.0);
        let turn = chronometer(ui, id, &mut next, width);
        if turn.changed {
            changed = true;
        }
        pulse = turn.pulse;
        couple = turn.couple;
        let icon = if next.is_some() { "×" } else { "+" };
        let hint = if next.is_some() {
            "clear date bound"
        } else {
            "arm date bound at today"
        };
        let action = chrome::icon_still(ui, icon).on_hover_text(hint);
        crate::probe_anchor!(
            ui,
            format!("date:{}:lever", id.trim_start_matches("date-")),
            action.interact_rect
        );
        if action.clicked() {
            let arming = next.is_none();
            next = arming.then(CreatedDay::today_utc);
            changed = true;
            pulse = Some(DatePulse::Lever(
                action.rect,
                if arming { 1.0 } else { -1.0 },
            ));
        }
    });
    DateEdit {
        value: next,
        changed,
        pulse,
        couple,
    }
}

#[derive(Clone, Copy, Debug)]
struct Turn {
    changed: bool,
    pulse: Option<DatePulse>,
    couple: Option<DatePulse>,
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
    let mut parts: Parts = value.map_or_else(today_parts, |day| day.ymd().into());
    let reels = reel_rects(rect);
    #[cfg(feature = "devtools")]
    {
        let bound = id.trim_start_matches("date-");
        for reel in Reel::ALL {
            let key = match reel {
                Reel::Year => "year",
                Reel::Month => "month",
                Reel::Day => "day",
            };
            crate::probe_anchor!(ui, format!("date:{bound}:{key}"), reel_window(reels, reel));
        }
    }
    let dt = ui
        .input(|input| input.stable_dt)
        .clamp(1.0 / 240.0, 1.0 / 30.0);

    let mut changed = false;
    let mut pulse = None;
    let mut couple = None;

    if active {
        let hovered = ui
            .ctx()
            .pointer_latest_pos()
            .and_then(|pos| reel_at(pos, reels));
        let drag = pump_drag(ui, id, &response, hovered);
        // Apply at most one source of intent this frame: a held drag wins. The
        // pulse direction is the screen-y the tape head travels — the gesture
        // direction — so the water gets shoved off that side. A day reel clamped
        // by the turn rides along as `couple`, carrying its own shove.
        if let Some((reel, slip)) = drag {
            let (chg, day) = turn_reel(ui, id, &mut parts, reel, Slip::Drag(slip), value);
            changed |= chg;
            pulse = Some(DatePulse::Tape(reel_window(reels, reel), slip.signum()));
            couple = day.map(|dir| DatePulse::Tape(reel_window(reels, Reel::Day), dir));
        } else if let Some(reel) = hovered.filter(|_| response.hovered()) {
            let steps = wheel_steps(ui, id, reel);
            // Own every scrap of scroll over a reel — not only the notch frame
            // but egui's whole smoothing tail — so the panel under us never
            // drifts and no stray delta bounces the date back.
            swallow_scroll(ui);
            if steps != 0 {
                let (chg, day) = turn_reel(ui, id, &mut parts, reel, Slip::Notch(steps), value);
                changed |= chg;
                pulse = Some(DatePulse::Tape(
                    reel_window(reels, reel),
                    -(steps.signum() as f32),
                ));
                couple = day.map(|dir| DatePulse::Tape(reel_window(reels, Reel::Day), dir));
            }
        }
    }

    // Relax every reel that is not being actively dragged this frame.
    let held = drag_capture(ui, id);
    let mut moving = false;
    for reel in Reel::ALL {
        if held == Some(reel) {
            continue;
        }
        let (floor, ceil) = stretch_bounds(reel, parts);
        moving |= with_drum(ui, id, reel, |drum| drum.relax(dt, floor, ceil));
    }
    if moving {
        ui.ctx().request_repaint();
    }

    // Arm/clear elevation: 1 = rollers up and threaded, 0 = sunk into the void.
    let lift = lift_spring(ui, id, active, dt);
    paint(ui, id, rect, reels, active, lift, parts);

    Turn {
        changed,
        pulse,
        couple,
    }
}

enum Slip {
    Notch(i32),
    Drag(f32),
}

/// Fold an intent into a reel: spin the committed value across detents,
/// catching the tape up with roll so the new value rolls into the head, and
/// recoiling off a hard stop the value refuses to cross.
fn turn_reel(
    ui: &egui::Ui,
    id: &'static str,
    parts: &mut Parts,
    reel: Reel,
    slip: Slip,
    value: &mut Option<CreatedDay>,
) -> (bool, Option<f32>) {
    let before = *value;
    let day_before = parts.day;
    with_drum(ui, id, reel, |drum| match slip {
        Slip::Notch(steps) => {
            for _ in 0..steps.unsigned_abs() {
                let _moved = commit(parts, drum, reel, steps.signum());
            }
        }
        Slip::Drag(slip) => {
            drum.roll += slip;
            while drum.roll > COMMIT {
                if !commit(parts, drum, reel, -1) {
                    break;
                }
            }
            while drum.roll < -COMMIT {
                if !commit(parts, drum, reel, 1) {
                    break;
                }
            }
        }
    });
    // Mechanical day coupling: a month/year turn that forced the day to clamp
    // rolls the day reel down to its new value (banking turns for its rollers, so
    // it spins too) instead of teleporting. The returned sign is the screen-y its
    // head travels, for the water it shoves.
    let couple = (reel != Reel::Day && parts.day != day_before).then(|| {
        let delta = parts.day as f32 - day_before as f32;
        with_drum(ui, id, Reel::Day, |day| {
            day.roll += delta;
            day.turns += delta;
        });
        -delta.signum()
    });
    *value = Some(parts.day());
    (*value != before, couple)
}

/// One detent of travel in `dir` (+1 = next/larger). Returns whether the value
/// actually moved; a refused move leaves a stretched, recoiling tape instead.
fn commit(parts: &mut Parts, drum: &mut Drum, reel: Reel, dir: i32) -> bool {
    if parts.spin(reel, dir, year_max()) {
        // The new value is at the head; let the old one still sit there and
        // spring the fresh label up into place. The tape advanced one pitch, so
        // bank a step for the rollers to turn through.
        drum.roll += dir as f32;
        drum.turns += dir as f32;
        true
    } else {
        drum.roll = (drum.roll - dir as f32 * WALL_STRETCH).clamp(-WALL_STRETCH, WALL_STRETCH);
        drum.vel = dir as f32 * WALL_KICK;
        false
    }
}

/// The roll interval a reel may occupy: months/days are endless, the year reel
/// hits a rubber wall a fraction past its first/last admissible value.
fn stretch_bounds(reel: Reel, parts: Parts) -> (f32, f32) {
    if reel != Reel::Year {
        return (f32::NEG_INFINITY, f32::INFINITY);
    }
    // roll>0 drags earlier years up; floor caps how far past YEAR_MIN we sag.
    let floor = if parts.year <= YEAR_MIN {
        -WALL_STRETCH
    } else {
        f32::NEG_INFINITY
    };
    let ceil = if parts.year >= year_max() {
        WALL_STRETCH
    } else {
        f32::INFINITY
    };
    (floor, ceil)
}

// --- Input plumbing ----------------------------------------------------------

/// Drains this frame's wheel travel for `reel` into integer detents, banking
/// the fraction across frames. Returns the signed step count (0 if sub-detent).
fn wheel_steps(ui: &egui::Ui, id: &'static str, reel: Reel) -> i32 {
    let Some(delta) = wheel_delta(ui) else {
        return 0;
    };
    with_drum(ui, id, reel, |drum| {
        if drum.notch != 0.0 && drum.notch.signum() != delta.signum() {
            drum.notch = 0.0;
        }
        drum.notch += delta;
        let steps = drum.notch.trunc();
        drum.notch -= steps;
        (steps as i32).clamp(-8, 8)
    })
}

/// Wheel travel this frame, in detents. A frame's worth of line/page wheeling
/// is one deliberate detent however many events claim it — that absorbs the
/// press/release doubling some stacks emit — while a trackpad's finer point
/// stream is metered continuously.
fn wheel_delta(ui: &egui::Ui) -> Option<f32> {
    let (line, point) = ui.input(|input| {
        let mut line = 0.0_f32;
        let mut point = 0.0_f32;
        for event in &input.events {
            if let egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } = event
                && !modifiers.ctrl
                && !modifiers.command
                && !modifiers.alt
            {
                match unit {
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => line += delta.y,
                    egui::MouseWheelUnit::Point => point += delta.y,
                }
            }
        }
        (line, point)
    });
    // Beware `(0.0).signum() == 1.0`: an idle frame must read as zero, not a
    // phantom up-detent, or the reel drifts whenever the pointer rests on it.
    let line = if line.abs() > 1e-4 {
        line.signum()
    } else {
        0.0
    };
    let detents = line + point / 50.0;
    (detents.abs() > 1e-4).then_some(detents)
}

/// Continuous drag of a captured reel: returns the reel and the roll slip (in
/// pitches) to fold in this frame. The capture survives until the drag ends.
fn pump_drag(
    ui: &egui::Ui,
    id: &'static str,
    response: &egui::Response,
    hovered: Option<Reel>,
) -> Option<(Reel, f32)> {
    let key = egui::Id::new((id, "drag"));
    ui.ctx().data_mut(|data| {
        if response.drag_stopped() || (!response.dragged() && !response.is_pointer_button_down_on())
        {
            let _ = data.remove_temp::<Reel>(key);
            return None;
        }
        if !response.dragged() {
            return None;
        }
        // The reel the drag first grabbed stays captured until the button lifts,
        // so a sloppy vertical drag can't hop between adjacent reels.
        let reel = match data.get_temp::<Reel>(key).or(hovered) {
            Some(reel) => reel,
            None => return None,
        };
        let _ = data.insert_temp(key, reel);
        // Drag travels in pixels; one pitch is one center-spaced label.
        let pitch = Spool::new(reel_window(reel_rects(response.rect), reel)).pitch();
        let slip = response.drag_delta().y / pitch.max(1.0);
        Some((reel, slip))
    })
}

fn drag_capture(ui: &egui::Ui, id: &'static str) -> Option<Reel> {
    ui.ctx()
        .data(|data| data.get_temp::<Reel>(egui::Id::new((id, "drag"))))
}

fn with_drum<R>(
    ui: &egui::Ui,
    id: &'static str,
    reel: Reel,
    edit: impl FnOnce(&mut Drum) -> R,
) -> R {
    let key = egui::Id::new((id, reel as u8, "drum"));
    ui.ctx().data_mut(|data| {
        let mut drum = data.get_temp::<Drum>(key).unwrap_or_default();
        let out = edit(&mut drum);
        let _ = data.insert_temp(key, drum);
        out
    })
}

/// Eat all vertical scroll over a reel for this frame: the raw wheel events and
/// egui's smoothed remainder alike. Only claims (and wakes the panel to undo
/// its scroll) when there is actually scroll to eat, so a merely-resting
/// pointer costs nothing.
fn swallow_scroll(ui: &egui::Ui) {
    let scrolling = ui.input(|input| {
        input.smooth_scroll_delta.y != 0.0
            || input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::MouseWheel { modifiers, .. }
                        if !modifiers.ctrl && !modifiers.command && !modifiers.alt
                )
            })
    });
    if !scrolling {
        return;
    }
    ui.ctx().input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::MouseWheel { modifiers, .. }
                    if !modifiers.ctrl && !modifiers.command && !modifiers.alt
            )
        });
        input.smooth_scroll_delta.y = 0.0;
    });
    ui.ctx().data_mut(|data| {
        let _ = data.insert_temp(egui::Id::new(WHEEL_CLAIM), true);
    });
}

// --- The tape path -----------------------------------------------------------

/// One sampled point on the tape: its screen height `y`, the surface-normal
/// angle `beta` (0 = facing the eye) that drives shading, and the perspective
/// `fold` that narrows width as the strip recedes over a roller.
#[derive(Clone, Copy)]
struct Sample {
    y: f32,
    beta: f32,
    fold: f32,
}

/// The tape transport: a gentle flat run that wraps a tight roller at each end.
/// Distances ride in aperture half-heights so the whole profile scales.
#[derive(Clone, Copy)]
struct Spool {
    cx: f32,
    cy: f32,
    a: f32,
}

impl Spool {
    fn new(window: egui::Rect) -> Self {
        Self {
            cx: window.center().x,
            cy: window.center().y,
            a: window.height() * 0.5,
        }
    }

    /// Carry a signed arc-length `t` (px from the head) onto the cassette curve:
    /// the gentle `R_FLAT` arc until the surface tilts to `BETA_F`, then the
    /// tight `R_ROLL` roller racing to `BETA_MAX`. Height and width both fold
    /// through the perspective eye as the tape recedes.
    fn sample(self, t: f32) -> Sample {
        let (flat_r, roll_r, eye) = (R_FLAT * self.a, R_ROLL * self.a, CAM_D * self.a);
        let sgn = if t < 0.0 { -1.0 } else { 1.0 };
        let arc = t.abs();
        let flat_arc = flat_r * BETA_F;
        let (beta, height, depth) = if arc <= flat_arc {
            let beta = arc / flat_r;
            (beta, flat_r * beta.sin(), flat_r * (1.0 - beta.cos()))
        } else {
            let beta = (BETA_F + (arc - flat_arc) / roll_r).min(BETA_MAX);
            let height = flat_r * BETA_F.sin() + roll_r * (beta.sin() - BETA_F.sin());
            let depth = flat_r * (1.0 - BETA_F.cos()) + roll_r * (BETA_F.cos() - beta.cos());
            (beta, height, depth)
        };
        let fold = eye / (eye + depth);
        Sample {
            y: self.cy + sgn * height * fold,
            beta: sgn * beta,
            fold,
        }
    }

    /// Arc-length between adjacent labels (their spacing along the flat run).
    fn pitch(self) -> f32 {
        self.a * PITCH
    }

    /// Arc-length past which the tape has curled edge-on; labels beyond cull.
    fn rim(self) -> f32 {
        self.a * (R_FLAT * BETA_F + R_ROLL * (BETA_MAX - BETA_F))
    }
}

/// Diffuse and specular response of the tape surface at normal angle `beta`.
fn lumen(beta: f32) -> (f32, f32) {
    let (s, c) = (beta.sin(), beta.cos());
    let diff = (s * L_Y + c * L_Z).max(0.0);
    let spec = (s * HV_Y + c * HV_Z).max(0.0).powf(SHINE);
    (diff, spec)
}

fn tape_rgb(beta: f32, gain: f32) -> egui::Color32 {
    let (diff, spec) = lumen(beta);
    let body = AMBIENT + DIFFUSE * diff;
    let glint = GLOSS * spec * (0.35 + 0.65 * gain);
    let chan = |albedo: f32, spark: f32| (albedo * body * gain + spark * glint).min(255.0) as u8;
    egui::Color32::from_rgb(
        chan(ALBEDO[0], SPARK[0]),
        chan(ALBEDO[1], SPARK[1]),
        chan(ALBEDO[2], SPARK[2]),
    )
}

/// Ink shaded by the same light: printed labels dim toward the rollers and
/// catch fire as they cross the gloss band on the flat run.
fn ink_shade(beta: f32, base: egui::Color32) -> egui::Color32 {
    let (diff, spec) = lumen(beta);
    let lit = (0.46 + 0.72 * diff).min(1.25);
    let glow = spec * 0.18;
    let chan = |c: u8, spark: f32| (f32::from(c) * lit + spark * glow).min(255.0) as u8;
    egui::Color32::from_rgb(
        chan(base.r(), SPARK[0]),
        chan(base.g(), SPARK[1]),
        chan(base.b(), SPARK[2]),
    )
}

// --- Painting: one austere brass control, blue tape sunk into it -------------

/// A snappy lever spring driving the arm/clear elevation: it rises to 1 with a
/// small bounce on the up-swing, and sinks to 0 and stops at the floor of the
/// void (no bounce down — there's a wall there).
fn lift_spring(ui: &egui::Ui, id: &'static str, active: bool, dt: f32) -> f32 {
    #[derive(Clone, Copy)]
    struct Lift {
        pos: f32,
        vel: f32,
    }
    let key = egui::Id::new((id, "lift"));
    let (pos, moving) = ui.ctx().data_mut(|data| {
        let mut st = data.get_temp::<Lift>(key).unwrap_or(Lift {
            pos: f32::from(active),
            vel: 0.0,
        });
        let target = f32::from(active);
        st.vel += (-LIFT_K * (st.pos - target) - LIFT_C * st.vel) * dt;
        st.pos += st.vel * dt;
        if st.pos < 0.0 {
            st.pos = 0.0;
            st.vel = st.vel.max(0.0);
        }
        let moving = (st.pos - target).abs() > 1e-3 || st.vel.abs() > 1e-3;
        let _old = data.insert_temp(key, st);
        (st.pos, moving)
    });
    if moving {
        ui.ctx().request_repaint();
    }
    pos
}

/// The bronze ramp shadow→body→glint; `t` is how lit the metal is at a point.
fn bronze(t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: [f32; 3], b: [f32; 3], f: f32| {
        egui::Color32::from_rgb(
            (a[0] + (b[0] - a[0]) * f) as u8,
            (a[1] + (b[1] - a[1]) * f) as u8,
            (a[2] + (b[2] - a[2]) * f) as u8,
        )
    };
    if t < 0.6 {
        mix(BRONZE_LO, BRONZE_MD, t / 0.6)
    } else {
        mix(BRONZE_MD, BRONZE_HI, (t - 0.6) / 0.4)
    }
}

/// Brightness 0..1 of a brass cylinder's end-cap rim at fractional height `v`
/// (−1 top, +1 bottom): a top-lit diffuse with a tight glint near the crown,
/// sinking dark at the lower rim — the cap read edge-on as a thin sliver.
fn cap_tone(v: f32) -> f32 {
    let s = v.clamp(-1.0, 1.0);
    let c = (1.0 - s * s).max(0.0).sqrt();
    let diff = (s * -0.55 + c * 0.835).max(0.0);
    let spec = (s * -0.286 + c * 0.958).max(0.0).powf(14.0);
    (0.16 + 0.5 * diff + 0.8 * spec).clamp(0.0, 1.0)
}

fn paint(
    ui: &egui::Ui,
    id: &'static str,
    rect: egui::Rect,
    reels: [(Reel, egui::Rect); 3],
    active: bool,
    lift: f32,
    parts: Parts,
) {
    let painter = ui.painter();
    facia(painter, rect);
    for (reel, window) in reels {
        let (roll, turns) = with_drum(ui, id, reel, |drum| (drum.roll, drum.turns));
        draw_reel(painter, window, reel, parts, roll, turns, lift, active);
    }
}

/// The faceplate: the dim poolroom tilework the reels are sunk into — baseline
/// dark, scattered tile-to-tile in both brightness and a faint warm/cool cast,
/// set off by darker grout and ringed by the same machined lip as the recesses.
fn facia(painter: &egui::Painter, rect: egui::Rect) {
    let _grout = painter.rect_filled(rect, 3.0, GROUT);
    let clip = painter.with_clip_rect(rect);
    let cols = (rect.width() / TILE).ceil() as i32;
    let rows = (rect.height() / TILE).ceil() as i32;
    for cy in 0..rows {
        for cx in 0..cols {
            let min = egui::pos2(
                rect.left() + cx as f32 * TILE,
                rect.top() + cy as f32 * TILE,
            );
            let tile = egui::Rect::from_min_size(min, egui::Vec2::splat(TILE)).intersect(rect);
            let (n, m) = tile_hash(cx, cy);
            let cast = (m - 0.5) * 2.0 * TILE_CAST; // + = warmer (more r, less b)
            let chan = |base: f32, lift: f32, c: f32| (base + lift * n + c).clamp(0.0, 255.0) as u8;
            let color = egui::Color32::from_rgb(
                chan(TILE_BASE[0], TILE_LIFT[0], cast),
                chan(TILE_BASE[1], TILE_LIFT[1], 0.0),
                chan(TILE_BASE[2], TILE_LIFT[2], -cast),
            );
            let _t = clip.add(egui::Shape::rect_filled(tile.shrink(0.7), 1.0, color));
        }
    }
    let _frame = painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.5_f32, RECESS_EDGE),
        egui::StrokeKind::Inside,
    );
}

fn tile_hash(cx: i32, cy: i32) -> (f32, f32) {
    let h = (cx.wrapping_mul(73_856_093) ^ cy.wrapping_mul(19_349_663)) as u32;
    let n = f32::from((h >> 19) as u16) / 8192.0;
    let m = f32::from((h >> 3) as u16 & 0x1fff) / 8192.0;
    (n, m)
}

#[expect(
    clippy::too_many_arguments,
    reason = "a reel carries its full per-frame render state"
)]
fn draw_reel(
    painter: &egui::Painter,
    window: egui::Rect,
    reel: Reel,
    parts: Parts,
    roll: f32,
    turns: f32,
    lift: f32,
    active: bool,
) {
    socket(painter, window); // the recess (an abyss) is always there

    if lift > 0.02 {
        // Arm/clear recedes the transport in z, not down the wall: it shrinks
        // back toward the recess depth, rising the same way out toward the eye.
        // At rest it fills the recess (capped at 1, so the lever spring's
        // overshoot can't spill past the lip); cleared, it withdraws to a third.
        let scale = (0.2 + 0.8 * lift).min(1.0);
        let content = egui::Rect::from_center_size(window.center(), window.size() * scale);
        let spool = Spool::new(content);
        let gain = if active { 1.0 } else { 0.5 };
        let tape_w = (window.width() - 12.0) * scale;

        let clip = painter.with_clip_rect(window);
        let _tape = clip.add(tape_mesh(spool, tape_w, gain));
        tape_edges(&clip, spool, tape_w, gain);

        // Labels only while the bound holds a value; a clearing reel sinks dark.
        if active {
            let cull = spool.rim() + spool.pitch();
            let (mut top_stop, mut bot_stop) = (None, None);
            for lane in -6..=6 {
                let t = (lane as f32 + roll) * spool.pitch();
                if t.abs() > cull {
                    continue;
                }
                match label(reel, parts, lane) {
                    Some(text) => print_label(&clip, spool, tape_w, t, &text),
                    // Remember the stop nearest the head on each side; the hazard
                    // then fills one band from there to the curl, not a chip a lane.
                    None if t < 0.0 => top_stop = Some(top_stop.map_or(lane, |l: i32| l.max(lane))),
                    None => bot_stop = Some(bot_stop.map_or(lane, |l: i32| l.min(lane))),
                }
            }
            if let Some(lane) = top_stop {
                hazard(&clip, spool, tape_w, lane, roll, -1.0);
            }
            if let Some(lane) = bot_stop {
                hazard(&clip, spool, tape_w, lane, roll, 1.0);
            }
        }

        // The roller grips the tape with no slip, so its surface tracks the
        // ribbon's travel — `roll - turns`, not its negative — or the drum spins
        // backward against the tape it is supposedly carrying.
        let phase = (roll - turns) * ROLL_GAIN;
        roller(&clip, spool, content, window, true, phase);
        roller(&clip, spool, content, window, false, phase);

        // Hold the dark back until the transport has visibly receded, so the
        // last third of the withdrawal swallows it rather than winking it out.
        let veil = (((0.4 - lift) / 0.4).clamp(0.0, 1.0) * 255.0) as u8;
        if veil > 0 {
            let _veil = clip.add(egui::Shape::rect_filled(
                window,
                0.0,
                egui::Color32::from_black_alpha(veil),
            ));
        }
    }

    // The index is welded to the frame, not the transport: it keeps its station
    // on the wall while the rollers rise out of / recede into the void behind it.
    index_arrow(painter, window, active);

    // The thin recess border, always crisp on top.
    let _border = painter.rect_stroke(
        window,
        1.0,
        egui::Stroke::new(1.0_f32, RECESS_EDGE),
        egui::StrokeKind::Inside,
    );
}

/// A reel sunk into the tilework: a black recess with a hard dark inner lip at
/// the top and a sliver of catch-light along the bottom wall.
fn socket(painter: &egui::Painter, window: egui::Rect) {
    let _void = painter.rect_filled(window, 1.0, WELL);
    let _gutter = painter.rect_filled(window.shrink2(egui::vec2(2.0, 0.0)), 0.0, GUTTER);
    let _shadow = painter.add(egui::Shape::line(
        vec![window.left_bottom(), window.left_top(), window.right_top()],
        egui::Stroke::new(1.6_f32, egui::Color32::from_rgb(2, 2, 3)),
    ));
    let _catch = painter.add(egui::Shape::line_segment(
        [window.left_bottom(), window.right_bottom()],
        egui::Stroke::new(1.0_f32, bronze(0.26)),
    ));
}

/// The two ends of one conveyor roller. The roller is a cylinder lying along
/// screen-x with the tape wrapped over its front, so each end reads edge-on as
/// a **thin vertical sliver** — the cap — poking past the ribbon, on a spindle
/// pinned to the recess wall. The cap's top sits one tape-thickness past where
/// the ribbon goes vertical. The welded longitudinal seam projects onto the
/// sliver as a mark that circles with the drum — riding up and down (`r·sinθ`)
/// across the near face and hidden once it rounds to the far side (`cosθ < 0`).
fn roller(
    painter: &egui::Painter,
    spool: Spool,
    content: egui::Rect,
    window: egui::Rect,
    top: bool,
    phase: f32,
) {
    let rim = spool.rim();
    let apex = spool.sample(if top { -rim } else { rim }).y;
    let cap_h = spool.a * 0.30;
    let r = cap_h * 0.5;
    let cap_cy = if top {
        apex + TAPE_THK + r
    } else {
        apex - TAPE_THK - r
    };
    const ROWS: usize = 10;
    for side in [-1.0_f32, 1.0] {
        let edge = if side < 0.0 {
            content.left()
        } else {
            content.right()
        };
        let cap_x = edge - side * (CAP_W * 0.5 + 2.4);
        // The spindle journals into the real recess wall, so it lengthens as the
        // transport rests inset and shortens as the lever pops up to fill.
        let wall_x = if side < 0.0 {
            window.left()
        } else {
            window.right()
        };
        let _spindle = painter.add(egui::Shape::line_segment(
            [
                egui::pos2(wall_x, cap_cy),
                egui::pos2(cap_x + side * CAP_W * 0.5, cap_cy),
            ],
            egui::Stroke::new(1.2_f32, bronze(0.4)),
        ));
        // the edge-on cylinder cap, shaded top-bright to bottom-dark.
        let mut mesh = egui::Mesh::default();
        for row in 0..=ROWS {
            let f = row as f32 / ROWS as f32;
            let y = cap_cy - r + f * cap_h;
            let color = bronze(cap_tone(2.0 * f - 1.0));
            mesh.colored_vertex(egui::pos2(cap_x - CAP_W * 0.5, y), color);
            mesh.colored_vertex(egui::pos2(cap_x + CAP_W * 0.5, y), color);
            if row > 0 {
                let base = (row as u32 - 1) * 2;
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base + 1, base + 3, base + 2);
            }
        }
        let _cap = painter.add(egui::Shape::mesh(mesh));
        // the welded seam, edge-on: a mark riding up and down with the rotation
        // (`r·sinθ`), occluded once it rounds to the drum's far side (`cosθ < 0`)
        // and fading as it grazes the silhouette, so it circles instead of
        // sliding back retrograde.
        let face = phase.cos();
        if face > 0.0 {
            let ny = cap_cy + (r - 0.6) * phase.sin();
            let fade = ((face / 0.3).min(1.0) * 255.0) as u8;
            let _notch = painter.add(egui::Shape::line_segment(
                [
                    egui::pos2(cap_x - CAP_W * 0.5, ny),
                    egui::pos2(cap_x + CAP_W * 0.5, ny),
                ],
                egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(3, 3, 4, fade),
                ),
            ));
        }
    }
}

/// The reading head: a brass index arrow hanging over the black recess at the
/// centre line, pointing in at the value. Its broad base is welded to the right
/// recess wall — anchored to the housing, not floating in the void.
fn index_arrow(painter: &egui::Painter, window: egui::Rect, active: bool) {
    let cy = window.center().y;
    let base_h = 5.4;
    let tip_h = base_h * 0.2; // a snub, sawed-off nose — tip ≈ 0.2× the base
    let base_x = window.right();
    let tip_x = window.right() - 9.0;
    let lit = if active { 0.0 } else { -0.2 };
    // a foot welding the base into the recess wall.
    let _foot = painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(base_x - 0.5, cy - base_h - 1.0),
            egui::pos2(base_x + 1.8, cy + base_h + 1.0),
        ),
        0.5,
        bronze(0.42 + lit),
    );
    let wedge = vec![
        egui::pos2(base_x, cy - base_h),
        egui::pos2(base_x, cy + base_h),
        egui::pos2(tip_x, cy + tip_h),
        egui::pos2(tip_x, cy - tip_h),
    ];
    let _wedge = painter.add(egui::Shape::convex_polygon(
        wedge,
        bronze(0.6 + lit),
        egui::Stroke::NONE,
    ));
    let _bevel_hi = painter.add(egui::Shape::line_segment(
        [
            egui::pos2(base_x, cy - base_h),
            egui::pos2(tip_x, cy - tip_h),
        ],
        egui::Stroke::new(1.0_f32, bronze(0.86 + lit)),
    ));
    let _bevel_lo = painter.add(egui::Shape::line_segment(
        [
            egui::pos2(base_x, cy + base_h),
            egui::pos2(tip_x, cy + tip_h),
        ],
        egui::Stroke::new(1.0_f32, bronze(0.18)),
    ));
}

/// The glossy ribbon, as a per-scanline-shaded mesh whose width and height fold
/// with perspective as the tape lies flat then curls over its rollers.
fn tape_mesh(spool: Spool, tape_w: f32, gain: f32) -> egui::Shape {
    const ROWS: usize = 44;
    let half = tape_w * 0.5;
    let rim = spool.rim();
    let mut mesh = egui::Mesh::default();
    for row in 0..=ROWS {
        let t = rim * (2.0 * row as f32 / ROWS as f32 - 1.0);
        let s = spool.sample(t);
        let color = tape_rgb(s.beta, gain);
        mesh.colored_vertex(egui::pos2(spool.cx - half * s.fold, s.y), color);
        mesh.colored_vertex(egui::pos2(spool.cx + half * s.fold, s.y), color);
        if row > 0 {
            let base = (row as u32 - 1) * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    egui::Shape::mesh(mesh)
}

/// The two recessed tape edges, darkened and curving with the surface.
fn tape_edges(painter: &egui::Painter, spool: Spool, tape_w: f32, gain: f32) {
    const ROWS: usize = 28;
    let half = tape_w * 0.5;
    let rim = spool.rim();
    for side in [-1.0_f32, 1.0] {
        let pts: Vec<egui::Pos2> = (0..=ROWS)
            .map(|row| {
                let s = spool.sample(rim * (2.0 * row as f32 / ROWS as f32 - 1.0));
                egui::pos2(spool.cx + side * half * s.fold, s.y)
            })
            .collect();
        let edge = egui::Color32::from_rgb(
            (16.0 * gain) as u8,
            (22.0 * gain) as u8,
            (34.0 * gain) as u8,
        );
        let _edge = painter.add(egui::Shape::line(pts, egui::Stroke::new(1.0_f32, edge)));
    }
}

/// Lay a label flat at the head, then carry it onto the tape at arc-length
/// `t_center` and bend every glyph along the profile, shaded by the same light
/// — printed-on-tape, not floated-over-it.
fn print_label(painter: &egui::Painter, spool: Spool, tape_w: f32, t_center: f32, text: &str) {
    let Some(mut shape) = lay_out(painter, spool, tape_w, text, chrome::HOT) else {
        return;
    };
    warp(spool, &mut shape, t_center, true);
    let _glyphs = painter.add(shape);
}

fn lay_out(
    painter: &egui::Painter,
    spool: Spool,
    tape_w: f32,
    text: &str,
    color: egui::Color32,
) -> Option<egui::epaint::TextShape> {
    let size = (tape_w * 0.32).clamp(12.0, 18.0);
    painter.fonts_mut(|fonts| {
        match egui::Shape::text(
            fonts,
            egui::pos2(spool.cx, spool.cy),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::new(size, egui::FontFamily::Monospace),
            color,
        ) {
            egui::Shape::Text(text) => Some(text),
            _ => None,
        }
    })
}

/// Project a flat-laid galley onto the tape at arc-length `t_center`: each
/// vertex's flat height becomes an arc offset along the profile, folding
/// through perspective and dimming by the surface light.
fn warp(spool: Spool, shape: &mut egui::epaint::TextShape, t_center: f32, shade: bool) {
    let galley = std::sync::Arc::make_mut(&mut shape.galley);
    galley.mesh_bounds = egui::Rect::NOTHING;
    galley.rect = egui::Rect::NOTHING;
    for placed in &mut galley.rows {
        let row_pos = placed.pos;
        let row = std::sync::Arc::make_mut(&mut placed.row);
        let mut bounds = egui::Rect::NOTHING;
        for vertex in &mut row.visuals.mesh.vertices {
            let flat = shape.pos + row_pos.to_vec2() + vertex.pos.to_vec2();
            let s = spool.sample(t_center + (flat.y - spool.cy));
            let world = egui::pos2(spool.cx + (flat.x - spool.cx) * s.fold, s.y);
            vertex.pos = world - shape.pos.to_vec2() - row_pos.to_vec2();
            if shade {
                // Dissolve the print as it rounds the curl, so a label vanishes
                // into the roller instead of collapsing onto the apex and piling
                // into a ghost beneath it.
                let fade = ((BETA_MAX - s.beta.abs()) / CURL_FADE).clamp(0.0, 1.0);
                let lit = ink_shade(s.beta, vertex.color);
                let dim = |c: u8| (f32::from(c) * fade) as u8;
                vertex.color = egui::Color32::from_rgba_premultiplied(
                    dim(lit.r()),
                    dim(lit.g()),
                    dim(lit.b()),
                    dim(lit.a()),
                );
            }
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

/// The end of tape: hazard hatching *printed on the ribbon* past the last
/// admissible year. The hatch is a flat diagonal field (`across + arc = c`), but
/// each stripe is sampled along the arc and projected through the same
/// perspective fold as the printed labels — so it bows and crowds over the
/// rollers like a true 3-D print, and tapers to the tape's real silhouette
/// instead of standing as a flat screen-space grate. Bounded to the invalid
/// arc-run [gap, curl] so it can't float past the edge-on rim, and slid by the
/// roll so the whole field rides with the tape. `toward_rim` is the screen-y
/// sign of the stop (−1 above the head, +1 below).
fn hazard(
    painter: &egui::Painter,
    spool: Spool,
    tape_w: f32,
    lane: i32,
    roll: f32,
    toward_rim: f32,
) {
    let pitch = spool.pitch();
    let last_valid = lane as f32 - toward_rim; // the printed year the hatch trails
    let t_edge = (last_valid + roll + toward_rim * HAZARD_GAP) * pitch;
    let t_rim = toward_rim * spool.rim();
    let (t_lo, t_hi) = (t_edge.min(t_rim), t_edge.max(t_rim));
    if t_hi - t_lo < 1.0 {
        return;
    }
    let half = tape_w * 0.5;
    let period = (spool.a * 0.175).max(5.0);
    let ink = egui::Color32::from_rgba_unmultiplied(214, 162, 78, 184);
    let c0 = roll * pitch; // rides the field with the tape
    let k_lo = ((t_lo - half - c0) / period).floor() as i32;
    let k_hi = ((t_hi + half - c0) / period).ceil() as i32;
    for k in k_lo..=k_hi {
        let c = c0 + k as f32 * period; // this stripe: across + arc = c
        let a_start = t_lo.max(c - half);
        let a_end = t_hi.min(c + half);
        if a_end - a_start < 1.0 {
            continue;
        }
        let steps = ((a_end - a_start) / 3.0).ceil().max(1.0) as usize;
        let pts: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let a = a_start + (a_end - a_start) * i as f32 / steps as f32;
                let s = spool.sample(a);
                egui::pos2(spool.cx + (c - a) * s.fold, s.y)
            })
            .collect();
        let _stripe = painter.add(egui::Shape::line(pts, egui::Stroke::new(1.4_f32, ink)));
    }
}

// --- Geometry helpers --------------------------------------------------------

fn reel_rects(rect: egui::Rect) -> [(Reel, egui::Rect); 3] {
    // Three *identical* tall recesses, each RECESS_TILES wide with a one-tile
    // gutter between, sat flush on the tile grid and filling the framed height —
    // no padding tiles above or below, so the transport reads big. The recess
    // narrows only to fit a cramped panel.
    let frame = 3.0;
    let inner = rect.shrink(frame);
    let fit = ((inner.width() / TILE - 2.0) / 3.0).floor();
    let tiles = fit.clamp(2.0, RECESS_TILES);
    let recess_w = tiles * TILE;
    let gap = TILE;
    let span = 3.0 * recess_w + 2.0 * gap;
    let raw = inner.left() + (inner.width() - span) * 0.5;
    let x0 = rect.left() + ((raw - rect.left()) / TILE).round() * TILE;
    let mk = |i: f32| {
        egui::Rect::from_min_size(
            egui::pos2(x0 + i * (recess_w + gap), inner.top()),
            egui::vec2(recess_w, inner.height()),
        )
    };
    [
        (Reel::Year, mk(0.0)),
        (Reel::Month, mk(1.0)),
        (Reel::Day, mk(2.0)),
    ]
}

fn reel_window(reels: [(Reel, egui::Rect); 3], needle: Reel) -> egui::Rect {
    reels
        .into_iter()
        .find_map(|(reel, slot)| (reel == needle).then_some(slot))
        .unwrap_or(egui::Rect::NOTHING)
}

fn reel_at(pos: egui::Pos2, reels: [(Reel, egui::Rect); 3]) -> Option<Reel> {
    reels
        .into_iter()
        .find_map(|(reel, rect)| rect.expand(3.0).contains(pos).then_some(reel))
}

// --- Value mapping -----------------------------------------------------------

/// The label printed at `offset` lanes from the head, or `None` where the year
/// reel runs off its admissible range (a stop, not a number).
fn label(reel: Reel, parts: Parts, offset: i32) -> Option<String> {
    match reel {
        Reel::Year => {
            let year = parts.year + offset;
            (YEAR_MIN..=year_max())
                .contains(&year)
                .then(|| format!("{year:04}"))
        }
        Reel::Month => Some(MONTHS[wrap(parts.month as i32 - 1 + offset, 12) as usize].to_owned()),
        Reel::Day => {
            let days = CreatedDay::days_in_month(parts.year, parts.month) as i32;
            Some(format!(
                "{:02}",
                wrap(parts.day as i32 - 1 + offset, days) + 1
            ))
        }
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
        CreatedDay::from_ymd(self.year, self.month, self.day).unwrap_or_else(CreatedDay::today_utc)
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
    fn commit_recoils_off_a_refused_year() {
        let mut parts = Parts {
            year: YEAR_MIN,
            month: 1,
            day: 1,
        };
        let mut drum = Drum::default();
        assert!(!commit(&mut parts, &mut drum, Reel::Year, -1));
        assert!(drum.roll.abs() <= WALL_STRETCH + 1e-6);
        assert!(drum.vel.abs() > 0.0, "a refused shove kicks the tape back");
    }

    #[test]
    fn commit_rolls_a_fresh_label_into_the_head() {
        let mut parts = Parts {
            year: 2020,
            month: 6,
            day: 15,
        };
        let mut drum = Drum::default();
        assert!(commit(&mut parts, &mut drum, Reel::Year, 1));
        assert_eq!(parts.year, 2021);
        assert!((drum.roll - 1.0).abs() < 1e-6);
    }

    #[test]
    fn spring_settles_to_the_detent() {
        let mut drum = Drum {
            roll: 1.0,
            vel: 0.0,
            notch: 0.0,
            turns: 0.0,
        };
        let mut overshot = false;
        for _ in 0..600 {
            let _moving = drum.relax(1.0 / 120.0, f32::NEG_INFINITY, f32::INFINITY);
            overshot |= drum.roll < -1e-3;
        }
        assert!(
            overshot,
            "an underdamped tape should overshoot the detent once"
        );
        assert!(drum.roll.abs() < 1e-2, "and then come to rest at the head");
    }

    #[test]
    fn year_wall_clamps_the_stretch() {
        let parts = Parts {
            year: YEAR_MIN,
            month: 1,
            day: 1,
        };
        let (floor, ceil) = stretch_bounds(Reel::Year, parts);
        assert_eq!(floor, -WALL_STRETCH);
        assert_eq!(ceil, f32::INFINITY);
        let mut drum = Drum {
            roll: -5.0,
            vel: -3.0,
            notch: 0.0,
            turns: 0.0,
        };
        let _ = drum.relax(1.0 / 120.0, floor, ceil);
        assert_eq!(drum.roll, -WALL_STRETCH);
    }
}
