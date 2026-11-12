//! Shared UI animation timing.
//!
//! Every animated UI value (sidebar reveal, mod toggle thumbs, and future
//! toasts, tickers, loaders) uses a [`Tween`]. Values are computed from
//! wall-clock time, so a dropped or late tick never slows an animation
//! down. Callers request animation frames only while
//! [`Tween::is_running`] is true, so nothing ticks when the UI is idle.
//!
//! Reduce motion: pass `reduce_motion = true` to [`Tween::animate_to`]
//! and the value jumps to its target with no frames requested.

use std::time::{Duration, Instant};

/// Frame interval to subscribe to while any tween is running.
pub const FRAME: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    Linear,
    /// Smoothstep: gentle start and stop.
    #[default]
    EaseInOut,
    /// Cubic ease-out: fast start, soft landing. Good for reveals.
    EaseOut,
}

impl Easing {
    /// Maps linear progress `t` in `0..=1` to eased progress.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t),
            Easing::EaseOut => 1.0 - (1.0 - t).powi(3),
        }
    }
}

/// An `f32` that eases from `from` to `to` over `duration` starting at
/// `start`. With no `start` it is settled at `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: Option<Instant>,
    pub duration: Duration,
    pub easing: Easing,
}

impl Default for Tween {
    fn default() -> Self {
        Self::settled(0.0)
    }
}

impl Tween {
    /// A tween resting at `value` that requests no frames.
    pub const fn settled(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: None,
            duration: Duration::ZERO,
            easing: Easing::EaseInOut,
        }
    }

    pub fn with_easing(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Linear progress `0..=1` at `now`.
    fn progress(&self, now: Instant) -> f32 {
        match self.start {
            None => 1.0,
            Some(_) if self.duration.is_zero() => 1.0,
            Some(start) => {
                let elapsed = now.saturating_duration_since(start);
                (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
            }
        }
    }

    /// The eased value at `now`. Exactly `to` once finished.
    pub fn value(&self, now: Instant) -> f32 {
        let t = self.progress(now);
        if t >= 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * self.easing.apply(t)
    }

    pub fn is_running(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    /// Settles the tween once finished so later checks are free.
    /// Returns whether it is still running.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.start.is_some() && !self.is_running(now) {
            self.start = None;
            self.from = self.to;
        }
        self.start.is_some()
    }

    /// Starts easing from the current value toward `to`.
    ///
    /// `full_duration` is the time for a full 0→1 sweep. The actual duration
    /// scales with the distance left, so reversing halfway takes half as
    /// long and the speed stays consistent. Re-targeting to the current
    /// target is a no-op, so this is safe to call on every hover event.
    pub fn animate_to(
        &mut self,
        to: f32,
        now: Instant,
        full_duration: Duration,
        reduce_motion: bool,
    ) {
        if to == self.to && (self.start.is_some() || self.from == to) {
            return;
        }
        if reduce_motion || full_duration.is_zero() {
            *self = Self { easing: self.easing, ..Self::settled(to) };
            return;
        }
        let from = self.value(now);
        let distance = (to - from).abs().min(1.0);
        self.from = from;
        self.to = to;
        self.start = Some(now);
        self.duration = full_duration.mul_f32(distance.max(0.0));
        if self.duration.is_zero() {
            self.start = None;
            self.from = to;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn settled_tween_is_idle() {
        let now = Instant::now();
        let tween = Tween::settled(1.0);
        assert_eq!(tween.value(now), 1.0);
        assert!(!tween.is_running(now));
    }

    #[test]
    fn eases_between_endpoints_and_lands_exactly() {
        let t0 = Instant::now();
        let mut tween = Tween::settled(0.0);
        tween.animate_to(1.0, t0, MS * 100, false);
        assert_eq!(tween.value(t0), 0.0);
        let mid = tween.value(t0 + MS * 50);
        assert!(mid > 0.0 && mid < 1.0);
        assert!(tween.is_running(t0 + MS * 50));
        assert_eq!(tween.value(t0 + MS * 100), 1.0);
        assert!(!tween.tick(t0 + MS * 100));
        assert_eq!(tween.value(t0 + MS * 500), 1.0);
    }

    #[test]
    fn reversing_midway_starts_from_current_value_and_scales_duration() {
        let t0 = Instant::now();
        let mut tween = Tween::settled(0.0).with_easing(Easing::Linear);
        tween.animate_to(1.0, t0, MS * 100, false);
        let t1 = t0 + MS * 40;
        tween.animate_to(0.0, t1, MS * 100, false);
        assert!((tween.value(t1) - 0.4).abs() < 1e-4);
        assert_eq!(tween.duration, MS * 40);
        assert_eq!(tween.value(t1 + MS * 40), 0.0);
    }

    #[test]
    fn same_target_does_not_restart() {
        let t0 = Instant::now();
        let mut tween = Tween::settled(0.0);
        tween.animate_to(1.0, t0, MS * 100, false);
        tween.animate_to(1.0, t0 + MS * 50, MS * 100, false);
        assert_eq!(tween.start, Some(t0));
    }

    #[test]
    fn reduce_motion_jumps_without_frames() {
        let t0 = Instant::now();
        let mut tween = Tween::settled(0.0);
        tween.animate_to(1.0, t0, MS * 100, true);
        assert_eq!(tween.value(t0), 1.0);
        assert!(!tween.is_running(t0));
    }

    #[test]
    fn easing_curves_hit_endpoints() {
        for easing in [Easing::Linear, Easing::EaseInOut, Easing::EaseOut] {
            assert_eq!(easing.apply(0.0), 0.0);
            assert_eq!(easing.apply(1.0), 1.0);
        }
    }
}
