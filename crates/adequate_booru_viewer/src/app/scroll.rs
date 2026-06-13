/// A scroll jump larger than this is navigation, not momentum.
const TILT_TELEPORT: f32 = 2500.0;
const TILT_SPEED_CEIL: f32 = 14_000.0;
const FORCE_CEIL: f32 = 48.0;
const FORCE_EPSILON: f32 = 0.015;
const THUMB_TELEPORT: f32 = 2500.0;
const THUMB_SPEED_CEIL: f32 = 16_000.0;
const THUMB_LOOKAHEAD_S: f32 = 0.45;
const THUMB_BASE_ROWS: usize = 2;
const THUMB_MAX_ROWS: usize = 18;
const THUMB_SPEED_EPSILON: f32 = 60.0;

/// Linearized tray shove: scroll velocity becomes a bounded body force over
/// the water. The persistent solver, not this CPU filter, performs pile-up,
/// release, reflection, and damping.
#[derive(Default)]
pub(super) enum TrayTilt {
    #[default]
    Virgin,
    Awake {
        offset: f32,
        velocity: f32,
    },
}

impl TrayTilt {
    pub(super) fn sway(
        &mut self,
        offset: f32,
        pixels_per_point: f32,
        dt: f32,
        coupling: f32,
        tau: f32,
    ) -> f32 {
        let Self::Awake {
            offset: last,
            velocity,
        } = self
        else {
            *self = Self::Awake {
                offset,
                velocity: 0.0,
            };
            return 0.0;
        };
        let delta = (offset - *last) * pixels_per_point;
        *last = offset;
        if delta.abs() > TILT_TELEPORT {
            *velocity = 0.0;
            return 0.0;
        }
        let sample = (delta / dt).clamp(-TILT_SPEED_CEIL, TILT_SPEED_CEIL);
        let alpha = 1.0 - (-dt / tau.max(0.02)).exp();
        *velocity += (sample - *velocity) * alpha;
        *velocity = velocity.clamp(-TILT_SPEED_CEIL, TILT_SPEED_CEIL);
        if velocity.abs() < FORCE_EPSILON / coupling.max(1.0e-6) && sample.abs() < 1.0 {
            *velocity = 0.0;
        }
        (*velocity * coupling).clamp(-FORCE_CEIL, FORCE_CEIL)
    }
}

/// Thumbnail cruise control: raw-ish scroll velocity becomes a narrow fetch
/// band beyond the visible rows. It deliberately ignores teleport jumps so a
/// query flip or scrollbar drag cannot spray hundreds of dead thumbnail jobs.
#[derive(Default)]
pub(super) enum ThumbCruise {
    #[default]
    Virgin,
    Awake {
        offset: f32,
        velocity: f32,
    },
}

impl ThumbCruise {
    pub(super) fn wake(
        &mut self,
        offset: f32,
        pixels_per_point: f32,
        dt: f32,
        row_height: f32,
        rows: usize,
        visible: std::ops::Range<usize>,
    ) -> Option<std::ops::Range<usize>> {
        let Self::Awake {
            offset: last,
            velocity,
        } = self
        else {
            *self = Self::Awake {
                offset,
                velocity: 0.0,
            };
            return None;
        };
        let delta = (offset - *last) * pixels_per_point;
        *last = offset;
        if delta.abs() > THUMB_TELEPORT {
            *velocity = 0.0;
            return None;
        }
        let sample = (delta / dt).clamp(-THUMB_SPEED_CEIL, THUMB_SPEED_CEIL);
        *velocity = sample.mul_add(0.55, *velocity * 0.45);
        if velocity.abs() < THUMB_SPEED_EPSILON || visible.is_empty() || rows == 0 {
            return None;
        }
        let row_px = (row_height * pixels_per_point).max(1.0);
        let ahead =
            THUMB_BASE_ROWS + ((*velocity).abs() * THUMB_LOOKAHEAD_S / row_px).ceil() as usize;
        let ahead = ahead.min(THUMB_MAX_ROWS);
        if *velocity > 0.0 {
            let start = visible.end.min(rows);
            Some(start..(start + ahead).min(rows))
        } else {
            let end = visible.start.min(rows);
            Some(end.saturating_sub(ahead)..end)
        }
        .filter(|band| !band.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_scroll_frame_hits_force_ceiling_without_second_lowpass() {
        let mut tray = TrayTilt::default();
        let _virgin = tray.sway(0.0, 1.0, 1.0 / 60.0, 0.08, 0.11);
        let force = tray.sway(100.0, 1.0, 1.0 / 60.0, 0.08, 0.11);
        assert!(force > 40.0, "force was {force}");
    }

    #[test]
    fn teleport_is_navigation_not_force() {
        let mut tray = TrayTilt::default();
        let _virgin = tray.sway(0.0, 1.0, 1.0 / 60.0, 0.08, 0.11);
        assert!(tray.sway(3000.0, 1.0, 1.0 / 60.0, 0.08, 0.11).abs() <= f32::EPSILON);
    }

    #[test]
    fn thumb_cruise_prefetches_in_scroll_direction() {
        let mut cruise = ThumbCruise::default();
        assert!(
            cruise
                .wake(0.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15)
                .is_none()
        );
        let down = cruise
            .wake(120.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15)
            .expect("down band");
        assert!(down.start >= 15, "{down:?}");
        let up = cruise
            .wake(20.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15)
            .expect("up band");
        assert!(up.end <= 10, "{up:?}");
    }

    #[test]
    fn thumb_cruise_ignores_teleports() {
        let mut cruise = ThumbCruise::default();
        let _virgin = cruise.wake(0.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15);
        assert!(
            cruise
                .wake(3000.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15)
                .is_none()
        );
    }
}
