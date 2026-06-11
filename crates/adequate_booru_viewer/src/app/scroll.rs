/// A scroll jump larger than this is navigation, not momentum: no surge.
/// Generous — egui smooth-scrolling animates flicks, so honest per-frame
/// deltas stay well below it.
const SURGE_TELEPORT: f32 = 2500.0;

/// Linearized scroll slosh state: a trapped shallow sheet remembers the last
/// plate offset, its damped velocity, and how much shear distance has accrued
/// toward the next boundary wave.
#[derive(Default)]
pub(super) enum ScrollSea {
    #[default]
    Virgin,
    Awake {
        offset: f32,
        velocity: f32,
        wake: f32,
    },
}

#[derive(Clone, Copy)]
pub(super) enum SurgeEdge {
    Top,
    Bottom,
}

impl ScrollSea {
    pub(super) fn shear(
        &mut self,
        offset: f32,
        pixels_per_point: f32,
        dt: f32,
        quantum: f32,
        base_amp: f32,
        tau: f32,
    ) -> Option<(SurgeEdge, f32, u8)> {
        let Self::Awake {
            offset: last,
            velocity,
            wake,
        } = self
        else {
            *self = Self::Awake {
                offset,
                velocity: 0.0,
                wake: 0.0,
            };
            return None;
        };
        let delta = (offset - *last) * pixels_per_point;
        *last = offset;
        if delta.abs() > SURGE_TELEPORT {
            *velocity = 0.0;
            *wake = 0.0;
            return None;
        }
        let sample = delta / dt;
        let old = *velocity;
        let alpha = 1.0 - (-dt / tau.max(0.01)).exp();
        *velocity += (sample - *velocity) * alpha;
        let speed = velocity.abs();
        let shove = delta.abs() + 0.18 * (*velocity - old).abs() * dt;
        *wake += shove;
        if speed < 8.0 {
            *wake *= (-dt / tau.max(0.01)).exp();
            return None;
        }
        if *wake < quantum {
            return None;
        }
        let count = (*wake / quantum).floor().min(4.0) as u8;
        *wake -= quantum * f32::from(count);
        *wake = (*wake).min(quantum);
        let acceleration = (*velocity - old).abs() / dt;
        let violence = (0.45 + speed / 900.0 + acceleration / 42_000.0).clamp(0.8, 5.5);
        let edge = if *velocity >= 0.0 {
            SurgeEdge::Top
        } else {
            SurgeEdge::Bottom
        };
        Some((edge, base_amp * violence, count))
    }
}
