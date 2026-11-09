use std::time::Duration;

use neori_config::Cursor;

use crate::animation::{Animation, Clock, Curve};
use crate::input::shake::{ShakeSettings, ShakeTracker};

fn settings_from(shake: &neori_config::ShakeToFind) -> ShakeSettings {
    ShakeSettings {
        window_ms: shake.window_ms,
        min_reversals: shake.min_reversals as usize,
        min_step: shake.min_step,
        reversal_angle_deg: shake.reversal_angle_deg,
    }
}

/// Enlarges the cursor when the user shakes the mouse, to help find it.
///
/// The cursor grows, stays enlarged for a while after the last detected shake, then
/// shrinks back. Sizes are expressed as a multiplier of the configured cursor size.
pub struct CursorShake {
    tracker: ShakeTracker,
    phase: Phase,
    enabled: bool,
    /// Multiplier to grow to.
    target_multiplier: f64,
    /// How long to stay enlarged after the last shake.
    timeout: Duration,
    /// Duration of the grow/shrink animations.
    animation_ms: u64,
    /// Current multiplier, always in sync with the cursor manager.
    multiplier: f64,
}

#[derive(Debug)]
enum Phase {
    Idle,
    Growing(Animation),
    Holding { until: Duration },
    Shrinking(Animation),
}

impl CursorShake {
    pub fn new(cursor: &Cursor) -> Self {
        let mut rv = Self {
            tracker: ShakeTracker::new(),
            phase: Phase::Idle,
            enabled: false,
            target_multiplier: 1.,
            timeout: Duration::ZERO,
            animation_ms: 0,
            multiplier: 1.,
        };
        rv.update_config(cursor);
        rv
    }

    /// Re-reads the cursor config, e.g. on config reload.
    pub fn update_config(&mut self, cursor: &Cursor) {
        let shake = &cursor.shake_to_find;
        self.enabled = shake.enable;
        self.timeout = Duration::from_millis(shake.timeout_ms as u64);
        self.animation_ms = shake.animation_ms as u64;
        self.tracker.set_settings(settings_from(shake));

        self.target_multiplier = match shake.size {
            Some(size) if cursor.xcursor_size > 0 => {
                f64::from(size) / f64::from(cursor.xcursor_size)
            }
            _ => shake.size_multiplier,
        }
        .max(1.);

        if !self.enabled {
            self.reset();
        }
    }

    /// Feeds a relative pointer motion delta. `time_ms` is the event timestamp.
    pub fn pointer_motion(&mut self, time_ms: u32, delta_x: f64, delta_y: f64, clock: &Clock) {
        if !self.enabled {
            return;
        }

        if self.tracker.push(time_ms, delta_x, delta_y) {
            self.trigger(clock);
        }
    }

    /// Convenience for triggering the enlargement from an action (bind or IPC).
    pub fn trigger_action(&mut self, clock: &Clock) {
        if self.enabled {
            self.trigger(clock);
        }
    }

    fn trigger(&mut self, clock: &Clock) {
        match &mut self.phase {
            Phase::Idle => {
                // Start growing from the normal size.
                self.multiplier = 1.;
                self.start_growing(clock, 1.);
            }
            Phase::Holding { until } => {
                // Keep it enlarged for longer.
                *until = clock.now() + self.timeout;
            }
            Phase::Growing(_) => {
                // Already growing; the hold timeout is set once the grow completes.
            }
            Phase::Shrinking(_) => {
                // Interrupt the shrink and grow again from the current size.
                let from = self.multiplier;
                self.start_growing(clock, from);
            }
        }
    }

    fn start_growing(&mut self, clock: &Clock, from: f64) {
        let animation = Animation::ease(
            clock.clone(),
            from,
            self.target_multiplier,
            0.,
            self.animation_ms,
            Curve::EaseOutCubic,
        );
        self.phase = Phase::Growing(animation);
    }

    /// Advances the animation and returns the current multiplier.
    pub fn advance(&mut self, clock: &Clock) -> f64 {
        match &mut self.phase {
            Phase::Idle => {
                self.multiplier = 1.;
            }
            Phase::Growing(animation) => {
                self.multiplier = animation.value();
                if animation.is_done() {
                    self.multiplier = self.target_multiplier;
                    self.phase = Phase::Holding {
                        until: clock.now() + self.timeout,
                    };
                }
            }
            Phase::Holding { until } => {
                self.multiplier = self.target_multiplier;
                if clock.now() >= *until {
                    let from = self.multiplier;
                    let animation = Animation::ease(
                        clock.clone(),
                        from,
                        1.,
                        0.,
                        self.animation_ms,
                        Curve::EaseOutCubic,
                    );
                    self.phase = Phase::Shrinking(animation);
                }
            }
            Phase::Shrinking(animation) => {
                self.multiplier = animation.value();
                if animation.is_done() {
                    self.reset();
                }
            }
        }

        self.multiplier
    }

    /// Whether any grow/hold/shrink is in progress, i.e. whether redraws are needed.
    pub fn are_animations_ongoing(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// The current size multiplier.
    pub fn multiplier(&self) -> f64 {
        self.multiplier
    }

    fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.multiplier = 1.;
        self.tracker.reset();
    }
}

#[cfg(test)]
mod tests {
    use neori_config::ShakeToFind;

    use super::*;

    fn cursor_with(enable: bool) -> Cursor {
        Cursor {
            shake_to_find: ShakeToFind {
                enable,
                size_multiplier: 4.,
                size: None,
                timeout_ms: 1000,
                animation_ms: 100,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn disabled_does_nothing() {
        let clock = Clock::with_time(Duration::ZERO);
        let mut shake = CursorShake::new(&cursor_with(false));

        let mut time = 0u32;
        for _ in 0..10 {
            time += 8;
            shake.pointer_motion(time, 30., 0., &clock);
            shake.pointer_motion(time + 4, -30., 0., &clock);
        }
        assert!(!shake.are_animations_ongoing());
        assert_eq!(shake.advance(&clock), 1.);
    }

    #[test]
    fn shake_grows_then_shrinks() {
        let mut clock = Clock::with_time(Duration::ZERO);
        let mut shake = CursorShake::new(&cursor_with(true));

        // Shake: alternating strong deltas.
        let mut time = 0u32;
        let mut dir = 1.0;
        for _ in 0..6 {
            for _ in 0..3 {
                time += 8;
                clock.set_unadjusted(Duration::from_millis(time as u64));
                shake.pointer_motion(time, dir * 30., 0., &clock);
            }
            dir = -dir;
        }
        assert!(shake.are_animations_ongoing());

        // Advance the clock past the grow animation.
        clock.set_unadjusted(Duration::from_millis(time as u64 + 200));
        let value = shake.advance(&clock);
        assert!((value - 4.).abs() < 0.001, "grew to {value}");

        // Still holding.
        assert!(shake.are_animations_ongoing());
        clock.set_unadjusted(Duration::from_millis(time as u64 + 500));
        shake.advance(&clock);
        assert!(shake.are_animations_ongoing());

        // Past the hold timeout: starts shrinking.
        clock.set_unadjusted(Duration::from_millis(time as u64 + 2000));
        shake.advance(&clock);
        assert!(shake.are_animations_ongoing());

        // After the shrink completes, back to normal.
        clock.set_unadjusted(Duration::from_millis(time as u64 + 2500));
        let value = shake.advance(&clock);
        assert!((value - 1.).abs() < 0.001, "shrank to {value}");
        assert!(!shake.are_animations_ongoing());
    }

    #[test]
    fn absolute_size_overrides_multiplier() {
        let mut cursor = cursor_with(true);
        cursor.xcursor_size = 24;
        cursor.shake_to_find.size = Some(96);
        cursor.shake_to_find.size_multiplier = 2.;
        let shake = CursorShake::new(&cursor);
        assert!((shake.target_multiplier - 4.).abs() < 0.001);
    }
}
