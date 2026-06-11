use super::{scroll::SurgeEdge, *};

const PLUNGE_SOURCE_LIFE: f32 = 0.24;
const WATER_WAKE: Duration = Duration::from_secs(14);

impl Bayonet {
    /// Hover-lift for the grid, modelled as bang-bang force plates over a
    /// relaxing membrane: the hovered tile's plate targets full press, every
    /// other targets rest, and each integrates toward its target independently
    /// so several rise and sink at once as the pointer sweeps.
    pub fn frost_lift(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
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
        self.lift_plates
            .iter()
            .filter(|plate| plate.grip > 0.0)
            .map(|plate| crate::frost::Lift {
                rect: egui::Rect::from_min_max(
                    (plate.rect.min.to_vec2() * pixels_per_point).to_pos2(),
                    (plate.rect.max.to_vec2() * pixels_per_point).to_pos2(),
                ),
                grip: plate.grip,
            })
            .collect()
    }

    /// Scroll inertia: the scrolled plate shears a trapped shallow fluid sheet.
    /// Motion accumulates wake distance, while speed and acceleration set the
    /// violence. The viewport edges are tight slits, so waves are born from
    /// the edge the water piles against and then reflect through the shader's
    /// wall images.
    pub(super) fn heave(&mut self, ctx: &egui::Context, offset: f32, pixels_per_point: f32) {
        if !self.water_rect.is_positive() {
            return;
        }
        let dt = ctx.input(|input| input.stable_dt).clamp(1.0 / 240.0, 0.08);
        let Some((edge, amp, count)) = self.scroll.shear(
            offset,
            pixels_per_point,
            dt,
            self.surf.surge_quantum,
            self.surf.surge_amp,
            self.surf.surge_tau,
        ) else {
            return;
        };
        let water = self.water_rect;
        let strip = |top: f32, bottom: f32| {
            egui::Rect::from_min_max(
                egui::pos2(water.left() + 6.0, top),
                egui::pos2(water.right() - 6.0, bottom),
            )
        };
        let rect = match edge {
            SurgeEdge::Top => strip(water.top() - 48.0, water.top() - 6.0),
            SurgeEdge::Bottom => strip(water.bottom() + 6.0, water.bottom() + 48.0),
        };
        self.plunge_with_walls(rect, amp * f32::from(count).sqrt(), WallSet::Vertical);
    }

    /// Drops a plate into the water: one ring, radiating from `rect`'s hull.
    pub(super) fn plunge(&mut self, rect: egui::Rect, amp: f32) {
        self.plunge_with_walls(rect, amp, WallSet::All);
    }

    fn plunge_with_walls(&mut self, rect: egui::Rect, amp: f32, walls: WallSet) {
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
            walls,
        });
        self.arm_water();
    }

    pub(super) fn text_plunge(&mut self, rect: egui::Rect, weight: f32) {
        self.plunge(rect, (TEXT_WAKE_AMP * weight).clamp(0.75, 6.6));
    }

    pub(super) fn touch_viewer(&mut self, center: egui::Pos2) {
        if self.viewer_touches.len() >= crate::frost::TOUCH_SLOTS {
            let _oldest = self.viewer_touches.remove(0);
        }
        self.viewer_touches.push(TouchPlunge {
            center,
            born: Instant::now(),
            amp: VIEWER_TOUCH_AMP,
        });
        self.arm_water();
    }

    fn arm_water(&mut self) {
        self.water_until = Some(Instant::now() + WATER_WAKE);
    }

    /// Birth exciters for the persistent water solver (physical px), plus the
    /// water rect (the grid viewport; the panel to its left is the shallows).
    pub fn frost_splashes(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
    ) -> (egui::Rect, Vec<crate::frost::Splash>) {
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
                    walls: plunge.walls.into(),
                }
            })
            .collect();
        (scale(surface), splashes)
    }

    pub fn frost_touches(
        &mut self,
        ctx: &egui::Context,
        pixels_per_point: f32,
    ) -> (egui::Rect, Vec<crate::frost::Touch>) {
        let life = self.surf.viewer_life;
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

/// A plate dropped into the water: the source of one expanding splash ring.
pub(super) struct Plunge {
    pub rect: egui::Rect,
    pub born: Instant,
    pub amp: f32,
    pub walls: WallSet,
}

#[derive(Clone, Copy)]
pub(super) enum WallSet {
    All,
    Vertical,
}

impl From<WallSet> for egui::Vec2 {
    fn from(walls: WallSet) -> Self {
        match walls {
            WallSet::All => egui::vec2(1.0, 1.0),
            WallSet::Vertical => egui::vec2(0.0, 1.0),
        }
    }
}

/// Fingertip ripple inside the full-image viewer pond.
pub(super) struct TouchPlunge {
    pub center: egui::Pos2,
    pub born: Instant,
    pub amp: f32,
}
