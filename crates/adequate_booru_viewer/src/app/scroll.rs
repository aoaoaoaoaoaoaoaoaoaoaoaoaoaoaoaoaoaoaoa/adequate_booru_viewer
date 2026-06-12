/// A scroll jump larger than this is navigation, not momentum.
const TILT_TELEPORT: f32 = 2500.0;
const TILT_SPEED_CEIL: f32 = 14_000.0;
const TILT_CEIL: f32 = 48.0;
const TILT_EPSILON: f32 = 0.015;

/// Linearized tray tilt: scroll velocity tips the water tray, while the
/// persistent solver performs the pile-up, release, reflection, and damping.
#[derive(Default)]
pub(super) enum TrayTilt {
    #[default]
    Virgin,
    Awake {
        offset: f32,
        velocity: f32,
        height: f32,
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
            height,
        } = self
        else {
            *self = Self::Awake {
                offset,
                velocity: 0.0,
                height: 0.0,
            };
            return 0.0;
        };
        let delta = (offset - *last) * pixels_per_point;
        *last = offset;
        if delta.abs() > TILT_TELEPORT {
            *velocity = 0.0;
            *height = 0.0;
            return 0.0;
        }
        let sample = (delta / dt).clamp(-TILT_SPEED_CEIL, TILT_SPEED_CEIL);
        let alpha = 1.0 - (-dt / tau.max(0.02)).exp();
        *velocity += (sample - *velocity) * alpha;
        *velocity = velocity.clamp(-TILT_SPEED_CEIL, TILT_SPEED_CEIL);
        *height += ((*velocity * coupling).clamp(-TILT_CEIL, TILT_CEIL) - *height) * alpha;
        if height.abs() < TILT_EPSILON && sample.abs() < 1.0 {
            *height = 0.0;
        }
        *height
    }
}
