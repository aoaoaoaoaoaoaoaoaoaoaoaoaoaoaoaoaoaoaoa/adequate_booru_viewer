const THUMB_TELEPORT: f32 = 2500.0;
const THUMB_SPEED_CEIL: f32 = 16_000.0;
const THUMB_LOOKAHEAD_S: f32 = 0.45;
const THUMB_BASE_ROWS: usize = 2;
const THUMB_MAX_ROWS: usize = 18;
const THUMB_SPEED_EPSILON: f32 = 60.0;

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
    fn thumb_cruise_prefetches_directionally_without_chasing_teleports() {
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
        assert!(
            cruise
                .wake(3000.0, 1.0, 1.0 / 60.0, 100.0, 100, 10..15)
                .is_none()
        );
    }
}
