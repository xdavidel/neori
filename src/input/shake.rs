use std::collections::VecDeque;

/// Detects a "shake" gesture in relative pointer motion.
///
/// A shake is a rapid sequence of direction reversals, like moving the mouse quickly
/// left-right several times. We detect it by looking at consecutive motion deltas: when
/// the movement direction flips by more than ~120 degrees, that's a reversal. Enough
/// reversals within a short window counts as a shake.
pub struct ShakeTracker {
    /// Reversals older than this are forgotten.
    window_ms: u32,
    /// Minimum number of reversals within the window to count as a shake.
    min_reversals: usize,
    /// Deltas smaller than this (in pixels) are ignored as jitter.
    min_step: f64,
    /// Time of the last significant motion delta, used to reset after inactivity.
    last_time_ms: Option<u32>,
    /// Direction of the last significant motion delta.
    last_dir: Option<(f64, f64)>,
    /// Timestamps (ms) of the reversals currently within the window.
    reversals: VecDeque<u32>,
}

impl ShakeTracker {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            window_ms: 600,
            min_reversals: 4,
            min_step: 12.0,
            last_time_ms: None,
            last_dir: None,
            reversals: VecDeque::new(),
        }
    }

    /// Feeds a relative motion delta at the given time in milliseconds.
    ///
    /// Returns `true` if a shake is currently detected.
    pub fn push(&mut self, time_ms: u32, delta_x: f64, delta_y: f64) -> bool {
        // Reset the tracked direction if we've been idle for a whole window.
        if let Some(last) = self.last_time_ms {
            if time_ms.saturating_sub(last) > self.window_ms {
                self.last_dir = None;
                self.reversals.clear();
            }
        }

        self.prune(time_ms);

        let mag = delta_x.hypot(delta_y);
        if mag < self.min_step {
            return self.is_shaking();
        }

        self.last_time_ms = Some(time_ms);

        if let Some((px, py)) = self.last_dir {
            let prev_mag = px.hypot(py);
            let dot = px * delta_x + py * delta_y;
            let cos = dot / (prev_mag * mag);
            // ~120 degrees or sharper counts as a reversal.
            if cos < -0.5 {
                self.reversals.push_back(time_ms);
            }
        }

        self.last_dir = Some((delta_x, delta_y));
        self.prune(time_ms);

        self.is_shaking()
    }

    /// Clears the tracked state, e.g. after a shake was handled.
    pub fn reset(&mut self) {
        self.last_time_ms = None;
        self.last_dir = None;
        self.reversals.clear();
    }

    fn is_shaking(&self) -> bool {
        self.reversals.len() >= self.min_reversals
    }

    fn prune(&mut self, now_ms: u32) {
        while let Some(&front) = self.reversals.front() {
            if now_ms.saturating_sub(front) > self.window_ms {
                self.reversals.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates a horizontal shake: strong left/right movement with the given number of legs.
    fn feed_shake(tracker: &mut ShakeTracker, legs: usize) -> bool {
        let mut time = 0u32;
        let mut dir = 1.0;
        let mut shaking = false;
        // Each leg consists of a couple of samples in the same direction.
        for _ in 0..legs {
            for _ in 0..3 {
                time += 8;
                shaking |= tracker.push(time, dir * 30.0, 0.0);
            }
            dir = -dir;
        }
        shaking
    }

    #[test]
    fn detects_a_shake() {
        let mut tracker = ShakeTracker::new();
        assert!(feed_shake(&mut tracker, 6));
    }

    #[test]
    fn steady_motion_is_not_a_shake() {
        let mut tracker = ShakeTracker::new();
        let mut time = 0u32;
        for _ in 0..50 {
            time += 8;
            assert!(!tracker.push(time, 30.0, 0.0));
        }
    }

    #[test]
    fn a_single_reversal_is_not_a_shake() {
        let mut tracker = ShakeTracker::new();
        let mut time = 0u32;
        for _ in 0..10 {
            time += 8;
            tracker.push(time, 30.0, 0.0);
        }
        for _ in 0..10 {
            time += 8;
            tracker.push(time, -30.0, 0.0);
        }
        assert!(!tracker.is_shaking());
    }

    #[test]
    fn jitter_below_threshold_is_ignored() {
        let mut tracker = ShakeTracker::new();
        let mut time = 0u32;
        let mut dir = 1.0;
        for _ in 0..100 {
            time += 8;
            // Alternating but tiny movement.
            tracker.push(time, dir * 1.0, 0.0);
            dir = -dir;
        }
        assert!(!tracker.is_shaking());
    }

    #[test]
    fn slow_reversals_do_not_accumulate() {
        let mut tracker = ShakeTracker::new();
        let mut time = 0u32;
        let mut dir = 1.0;
        for _ in 0..10 {
            time += 1000;
            tracker.push(time, dir * 30.0, 0.0);
            dir = -dir;
        }
        assert!(!tracker.is_shaking());
    }

    #[test]
    fn vertical_shake_is_detected() {
        let mut tracker = ShakeTracker::new();
        let mut time = 0u32;
        let mut dir = 1.0;
        let mut shaking = false;
        for _ in 0..6 {
            for _ in 0..3 {
                time += 8;
                shaking |= tracker.push(time, 0.0, dir * 30.0);
            }
            dir = -dir;
        }
        assert!(shaking);
    }
}
