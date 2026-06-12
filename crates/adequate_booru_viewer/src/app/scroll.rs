/// A scroll jump larger than this is navigation, not momentum.
const TILT_TELEPORT: f32 = 2500.0;
const TILT_SPEED_CEIL: f32 = 14_000.0;
const FORCE_CEIL: f32 = 48.0;
const FORCE_EPSILON: f32 = 0.015;

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
}
