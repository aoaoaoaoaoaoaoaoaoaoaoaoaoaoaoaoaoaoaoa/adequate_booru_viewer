use super::*;

const PLUNGE_SOURCE_LIFE: f32 = 0.24;
const TOOLTIP_GRIP: f32 = 0.72;
const HOVER_BUMP_AMP: f32 = 0.18;
const TAPE_WAKE_AMP: f32 = 0.16;
const LEVER_WAKE_AMP: f32 = 0.5;
const GROUP_SELECT_AMP: f32 = 0.45;
const FOLD_OPEN_AMP: f32 = -0.72;
const FOLD_CLOSE_AMP: f32 = 0.92;
const WATER_WAKE: Duration = Duration::from_secs(14);
const RAFT_RATE: f32 = 1.7;
const RAFT_RISE: Duration = Duration::from_millis(70);
const RAFT_SINK_TAU: f32 = 0.5;
const RAFT_PEAK_MIN: f32 = 13.0;
const RAFT_PEAK_SPAN: f32 = 10.0;
const DRAIN_COLS: usize = 3;
const DRAIN_ROWS: usize = 2;
const DRAIN_CELLS: usize = DRAIN_COLS * DRAIN_ROWS;
const DRAIN_RATE: f32 = 1.15;
const DRAIN_AMP_MIN: f32 = -0.52;
const DRAIN_AMP_SPAN: f32 = -0.48;

impl Bayonet {
    /// Hover-lift for the grid, modelled as bang-bang force plates over a
    /// relaxing membrane: the hovered tile's plate targets full press, every
    /// other targets rest, and each integrates toward its target independently
    /// so several rise and sink at once as the pointer sweeps.
    pub fn frost_lift(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
        tooltip_rects: &[egui::Rect],
    ) -> Vec<crate::frost::Lift> {
        let dt = ctx.input(|input| input.stable_dt).clamp(0.0, 0.1);
        let hovered_id = self.hover_tile.map(|(id, _)| id);
        if hovered_id != self.splash_memo.map(|(id, _)| id) {
            if let Some((_, rect)) = self.splash_memo {
                self.plunge(rect, self.surf.exit_amp);
            }
            if let Some((_, rect)) = self.hover_tile {
                self.plunge(rect, self.surf.enter_amp);
            }
            self.splash_memo = self.hover_tile;
        }
        if let Some((id, rect)) = self.hover_tile {
            match self.lift_plates.iter_mut().find(|plate| plate.id == id) {
                Some(plate) => plate.rect = rect,
                None => self.lift_plates.push(LiftPlate {
                    id,
                    rect,
                    grip: 0.0,
                }),
            }
        }
        let mut animating = false;
        for plate in &mut self.lift_plates {
            let target = f32::from(hovered_id == Some(plate.id));
            let tau = if target > plate.grip {
                self.surf.tau_rise
            } else {
                self.surf.tau_fall
            };
            plate.grip += (target - plate.grip) * (1.0 - (-dt / tau).exp());
            if (plate.grip - target).abs() > 0.002 {
                animating = true;
            }
        }
        self.lift_plates
            .retain(|plate| plate.grip > 0.002 || hovered_id == Some(plate.id));
        if self.lift_plates.len() > crate::frost::LIFT_SLOTS {
            self.lift_plates.sort_by(|a, b| b.grip.total_cmp(&a.grip));
            self.lift_plates.truncate(crate::frost::LIFT_SLOTS);
        }
        if animating {
            ctx.request_repaint();
        }
        let tooltip_slots = tooltip_rects.len().min(1);
        let image_slots = crate::frost::LIFT_SLOTS.saturating_sub(tooltip_slots);
        let mut image_plates = self
            .lift_plates
            .iter()
            .filter(|plate| plate.grip > 0.0)
            .collect::<Vec<_>>();
        image_plates.sort_by(|a, b| b.grip.total_cmp(&a.grip));

        let scale = |rect: egui::Rect| {
            egui::Rect::from_min_max(
                (rect.min.to_vec2() * pixels_per_point).to_pos2(),
                (rect.max.to_vec2() * pixels_per_point).to_pos2(),
            )
        };
        let mut lifts = image_plates
            .into_iter()
            .take(image_slots)
            .map(|plate| crate::frost::Lift::surface(scale(plate.rect), plate.grip))
            .collect::<Vec<_>>();
        lifts.extend(
            tooltip_rects
                .iter()
                .take(tooltip_slots)
                .copied()
                .map(|rect| crate::frost::Lift::shallow(scale(rect), TOOLTIP_GRIP)),
        );
        lifts
    }

    /// Scroll inertia: the gallery tray shoves the water under filtered scroll
    /// velocity. The shader sees a zero-mean body force; pile-up and release
    /// are left to the persistent wave field instead of injected as strips.
    pub(super) fn heave(&mut self, ctx: &egui::Context, offset: f32, pixels_per_point: f32) {
        if !self.water_rect.is_positive() {
            self.scroll_tilt = 0.0;
            return;
        }
        let dt = ctx.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.08);
        self.scroll_tilt = self.scroll.sway(
            offset,
            pixels_per_point,
            dt,
            self.surf.scroll_coupling * self.water_amp(),
            self.surf.scroll_tau,
        );
        if self.scroll_tilt.abs() > 0.015 {
            self.arm_water();
            ctx.request_repaint();
        }
    }

    /// Drops a plate into the water: one ring, radiating from `rect`'s hull.
    pub(super) fn plunge(&mut self, rect: egui::Rect, amp: f32) {
        self.plunge_as(rect, amp, crate::frost::SplashShape::Ring);
    }

    fn plunge_as(&mut self, rect: egui::Rect, amp: f32, shape: crate::frost::SplashShape) {
        self.plunge_scaled(rect, amp * self.water_amp(), shape, 0.0);
    }

    fn plunge_scaled(
        &mut self,
        rect: egui::Rect,
        amp: f32,
        shape: crate::frost::SplashShape,
        drag: f32,
    ) {
        if amp.abs() <= f32::EPSILON {
            return;
        }
        if self.plunges.len() >= crate::frost::SPLASH_SLOTS {
            let victim = self
                .plunges
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.amp.abs().total_cmp(&b.amp.abs()))
                .map_or(0, |(slot, _)| slot);
            let _weakest = self.plunges.remove(victim);
        }
        self.plunges.push(Plunge {
            rect,
            born: Instant::now(),
            amp,
            shape,
            drag,
        });
        self.arm_water();
    }

    pub(super) fn bump_plunge(&mut self, rect: egui::Rect) {
        self.plunge(rect, HOVER_BUMP_AMP);
    }

    /// Arming a date bound raises its rollers and shoulders water out (a source,
    /// `sign > 0`); clearing it drops them into the void and draws water in (a
    /// sink). A piston the size of the reel — a basin either way.
    pub(super) fn lever_plunge(&mut self, rect: egui::Rect, sign: f32) {
        self.plunge_as(
            rect,
            sign * LEVER_WAKE_AMP,
            crate::frost::SplashShape::Basin,
        );
    }

    /// A spun date reel drags the water in contact with it: a directional
    /// velocity dipole, shoving the bulk off the edge the tape head travels
    /// toward (`dir` is the signed screen-y of that travel).
    pub(super) fn tape_plunge(&mut self, rect: egui::Rect, dir: f32) {
        self.plunge_scaled(
            rect,
            TAPE_WAKE_AMP * self.water_amp(),
            crate::frost::SplashShape::Ring,
            dir,
        );
    }

    pub(super) fn group_plunge(&mut self, rect: egui::Rect) {
        self.plunge(rect, GROUP_SELECT_AMP);
    }

    /// A tileset swap thwacks the whole pool from beneath — a broadband
    /// velocity-noise impulse the solver's KO dissipation shreds into a quick
    /// high-k shimmer. `energy` (0..1, the fraction of tiles actually replaced)
    /// scales it, so an unchanged result makes no wave.
    pub(super) fn pool_thwack(&mut self, rect: egui::Rect, energy: f32) {
        self.plunge_as(
            rect,
            self.surf.thwack_amp * energy,
            crate::frost::SplashShape::Jitter,
        );
    }

    pub(super) fn fold_plunge(&mut self, wake: Option<chrome::FoldWake>) {
        let Some(wake) = wake else {
            return;
        };
        let amp = match wake.flux {
            chrome::FoldFlux::Open => FOLD_OPEN_AMP,
            chrome::FoldFlux::Close => FOLD_CLOSE_AMP,
        };
        self.plunge_as(wake.rect, amp, crate::frost::SplashShape::Basin);
    }

    pub(super) fn drain_plunge(&mut self, rect: egui::Rect, amp: f32) {
        self.plunge_as(rect, amp, crate::frost::SplashShape::Basin);
    }

    pub(super) fn text_plunge(&mut self, wake: chrome::TextWake) {
        let raw = wake.amp(self.surf.text_amp).clamp(0.25, 6.6);
        if raw >= 0.25 {
            self.plunge_scaled(
                wake.rect,
                raw * self.glyph_amp(),
                crate::frost::SplashShape::Ring,
                0.0,
            );
        }
    }

    pub(super) fn touch_viewer(&mut self, center: egui::Pos2) {
        if self.viewer_touches.len() >= crate::frost::TOUCH_SLOTS {
            let _oldest = self.viewer_touches.remove(0);
        }
        self.viewer_touches.push(TouchPlunge {
            center,
            born: Instant::now(),
            amp: self.surf.viewer_amp * self.water_amp(),
        });
        self.arm_water();
    }

    pub(super) fn arm_water(&mut self) {
        self.water_until = Some(Instant::now() + WATER_WAKE);
    }

    /// Birth exciters for the persistent water solver (physical px), plus the
    /// water rect (the grid viewport; the panel to its left is the shallows).
    pub fn frost_splashes(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
    ) -> (
        egui::Rect,
        f32,
        Vec<crate::frost::Splash>,
        Option<crate::frost::Raft>,
        egui::Rect,
    ) {
        self.plunges
            .retain(|plunge| plunge.born.elapsed().as_secs_f32() <= PLUNGE_SOURCE_LIFE);
        if !self.plunges.is_empty() {
            ctx.request_repaint();
        }
        let surface = if self.water_rect.is_positive() {
            self.water_rect
        } else {
            ctx.content_rect()
        };
        let scale = |rect: egui::Rect| {
            egui::Rect::from_min_max(
                (rect.min.to_vec2() * pixels_per_point).to_pos2(),
                (rect.max.to_vec2() * pixels_per_point).to_pos2(),
            )
        };
        let splashes = self
            .plunges
            .iter()
            .map(|plunge| {
                let age = plunge.born.elapsed().as_secs_f32();
                crate::frost::Splash {
                    rect: scale(plunge.rect),
                    age,
                    amp: plunge.amp,
                    shape: plunge.shape,
                    drag: plunge.drag,
                }
            })
            .collect();
        let raft = self
            .loading_raft
            .source(ctx, pixels_per_point)
            .map(|mut raft| {
                for corner in &mut raft.corners {
                    *corner *= self.water_amp();
                }
                raft
            });
        (
            scale(surface),
            self.scroll_tilt,
            splashes,
            raft,
            scale(self.floor_rect),
        )
    }

    pub fn frost_touches(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
    ) -> (egui::Rect, Vec<crate::frost::Touch>) {
        let life = self.viewer_life();
        self.viewer_touches.retain(|touch| {
            self.zoom.is_some() && retire(touch.born.elapsed().as_secs_f32(), life) > 0.0
        });
        if !self.viewer_touches.is_empty() {
            ctx.request_repaint();
        }
        let pond = if self.viewer_pond.is_positive() {
            self.viewer_pond
        } else {
            egui::Rect::from_min_size(egui::pos2(-4e6, -4e6), egui::Vec2::ZERO)
        };
        let touches = self
            .viewer_touches
            .iter()
            .map(|touch| {
                let age = touch.born.elapsed().as_secs_f32();
                crate::frost::Touch {
                    center: (touch.center.to_vec2() * pixels_per_point).to_pos2(),
                    age,
                    amp: touch.amp * retire(age, life),
                }
            })
            .collect();
        (
            egui::Rect::from_min_max(
                (pond.min.to_vec2() * pixels_per_point).to_pos2(),
                (pond.max.to_vec2() * pixels_per_point).to_pos2(),
            ),
            touches,
        )
    }

    pub fn frost_wake(&mut self, ctx: &egui::Context) -> bool {
        if self.water_until.is_some_and(|until| until > Instant::now()) {
            ctx.request_repaint();
            return true;
        }
        self.water_until = None;
        false
    }
}

fn retire(age: f32, life: f32) -> f32 {
    const TAIL: f32 = 2.5;
    let t = ((age - life) / TAIL).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    1.0 - smooth
}

/// One grid tile's membrane plate: its grip relaxes toward 1 while hovered,
/// toward 0 once released. Keyed by post so it survives the tile's redraw.
pub(super) struct LiftPlate {
    pub id: PostId,
    pub rect: egui::Rect,
    pub grip: f32,
}

/// A plate dropped into the water: the source of one expanding splash ring,
/// or — when `drag` is non-zero — a directional shove off a moving surface.
pub(super) struct Plunge {
    pub rect: egui::Rect,
    pub born: Instant,
    pub amp: f32,
    pub shape: crate::frost::SplashShape,
    pub drag: f32,
}

/// Fingertip ripple inside the full-image viewer pond.
pub(super) struct TouchPlunge {
    pub center: egui::Pos2,
    pub born: Instant,
    pub amp: f32,
}

pub(super) struct DrainPulse {
    pub rect: egui::Rect,
    pub amp: f32,
}

/// The empty-gallery loading plate: a bilinear high-tension membrane pulled by
/// four independent corner pistons. Each corner fires as a Poisson clock,
/// ramps up quickly, then sinks exponentially with a 500 ms time constant.
pub(super) struct LoadingRaft {
    rect: egui::Rect,
    corners: [RaftPiston; 4],
    rng: u64,
    visible: bool,
}

impl LoadingRaft {
    pub fn new() -> Self {
        let now = Instant::now();
        let mut raft = Self {
            rect: egui::Rect::NOTHING,
            corners: [RaftPiston::new(now); 4],
            rng: 0x2b99_2751_d6e8_4d31,
            visible: false,
        };
        for slot in 0..raft.corners.len() {
            let wait = raft.wait();
            raft.corners[slot].next = now + Duration::from_secs_f32(wait);
        }
        raft
    }

    pub fn show(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        self.visible = true;
        self.rect = rect;
        self.tick(Instant::now());
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    fn source(&mut self, ctx: &egui::Context, pixels_per_point: f32) -> Option<crate::frost::Raft> {
        if !self.visible {
            return None;
        }
        self.tick(Instant::now());
        ctx.request_repaint_after(Duration::from_millis(16));
        Some(crate::frost::Raft {
            rect: egui::Rect::from_min_max(
                (self.rect.min.to_vec2() * pixels_per_point).to_pos2(),
                (self.rect.max.to_vec2() * pixels_per_point).to_pos2(),
            ),
            corners: self
                .corners
                .map(|corner| corner.height() * pixels_per_point),
        })
    }

    fn tick(&mut self, now: Instant) {
        for slot in 0..self.corners.len() {
            if now < self.corners[slot].next {
                continue;
            }
            let peak = RAFT_PEAK_MIN + self.unit() * RAFT_PEAK_SPAN;
            self.corners[slot].fire(now, peak);
            let wait = self.wait();
            self.corners[slot].next = now + Duration::from_secs_f32(wait);
        }
    }

    fn wait(&mut self) -> f32 {
        -(1.0 - self.unit()).ln() / RAFT_RATE
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 7;
        self.rng ^= self.rng >> 9;
        self.rng ^= self.rng << 8;
        ((self.rng >> 40) as u32 as f32 + 0.5) / 16_777_216.0
    }
}

#[derive(Clone, Copy)]
struct RaftPiston {
    fired: Instant,
    next: Instant,
    base: f32,
    peak: f32,
}

impl RaftPiston {
    fn new(now: Instant) -> Self {
        Self {
            fired: now,
            next: now,
            base: 0.0,
            peak: 0.0,
        }
    }

    fn fire(&mut self, now: Instant, peak: f32) {
        self.base = self.height_at(now);
        self.peak = peak;
        self.fired = now;
    }

    fn height(self) -> f32 {
        self.height_at(Instant::now())
    }

    fn height_at(self, now: Instant) -> f32 {
        let age = now.saturating_duration_since(self.fired);
        if age <= RAFT_RISE {
            let t = age.as_secs_f32() / RAFT_RISE.as_secs_f32();
            return self.base + (self.peak - self.base) * t;
        }
        let sink = age.saturating_sub(RAFT_RISE).as_secs_f32();
        self.peak * (-sink / RAFT_SINK_TAU).exp()
    }
}

/// The settled empty-gallery card: a quiet six-cell drain field. Each cell is
/// an independent Poisson clock; a fire withdraws a random small volume from
/// its basin, letting the persistent solver do the funneling.
pub(super) struct EmptyDrain {
    cells: [Instant; DRAIN_CELLS],
    rng: u64,
    visible: bool,
}

impl EmptyDrain {
    pub fn new() -> Self {
        let now = Instant::now();
        let mut drain = Self {
            cells: [now; DRAIN_CELLS],
            rng: 0x83f6_1d05_a04d_e357,
            visible: false,
        };
        for slot in 0..DRAIN_CELLS {
            drain.cells[slot] = now + Duration::from_secs_f32(drain.wait());
        }
        drain
    }

    pub fn show(&mut self, ctx: &egui::Context, rect: egui::Rect) -> Vec<DrainPulse> {
        self.visible = true;
        let now = Instant::now();
        let mut pulses = Vec::new();
        for slot in 0..DRAIN_CELLS {
            if now < self.cells[slot] {
                continue;
            }
            pulses.push(DrainPulse {
                rect: drain_cell(rect, slot),
                amp: DRAIN_AMP_MIN + self.unit() * DRAIN_AMP_SPAN,
            });
            self.cells[slot] = now + Duration::from_secs_f32(self.wait());
        }
        ctx.request_repaint_after(Duration::from_millis(33));
        pulses
    }

    pub fn hide(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        let now = Instant::now();
        for slot in 0..DRAIN_CELLS {
            self.cells[slot] = now + Duration::from_secs_f32(self.wait());
        }
    }

    fn wait(&mut self) -> f32 {
        -(1.0 - self.unit()).ln() / DRAIN_RATE
    }

    fn unit(&mut self) -> f32 {
        self.rng ^= self.rng << 7;
        self.rng ^= self.rng >> 9;
        self.rng ^= self.rng << 8;
        ((self.rng >> 40) as u32 as f32 + 0.5) / 16_777_216.0
    }
}

fn drain_cell(rect: egui::Rect, slot: usize) -> egui::Rect {
    let col = slot % DRAIN_COLS;
    let row = slot / DRAIN_COLS;
    let cell = egui::vec2(
        rect.width() / DRAIN_COLS as f32,
        rect.height() / DRAIN_ROWS as f32,
    );
    let min = rect.min + egui::vec2(col as f32 * cell.x, row as f32 * cell.y);
    egui::Rect::from_min_size(min, cell).shrink(6.0)
}
