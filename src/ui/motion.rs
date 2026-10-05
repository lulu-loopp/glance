//! Transitions as CSS times them: a value moving from where it is to a
//! target over a duration, along a cubic Bézier easing curve.

use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Easing(pub f32, pub f32, pub f32, pub f32);

pub const LINEAR: Easing = Easing(0.0, 0.0, 1.0, 1.0);

impl Easing {
    /// The curve's progress at time fraction `x`: the y of the point whose x is `x`.
    pub fn at(self, x: f32) -> f32 {
        let Easing(x1, y1, x2, y2) = self;
        let x = x.clamp(0.0, 1.0);
        let bezier = |t: f32, p1: f32, p2: f32| {
            let u = 1.0 - t;
            3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
        };
        let slope = |t: f32, p1: f32, p2: f32| {
            let u = 1.0 - t;
            3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
        };
        // x(t) is monotonic for control points inside [0, 1]: Newton's method
        // from t = x, falling back to bisection where the slope is flat.
        let mut t = x;
        for _ in 0..8 {
            let error = bezier(t, x1, x2) - x;
            if error.abs() < 1e-5 {
                return bezier(t, y1, y2);
            }
            let d = slope(t, x1, x2);
            if d.abs() < 1e-6 {
                break;
            }
            t = (t - error / d).clamp(0.0, 1.0);
        }
        let (mut low, mut high) = (0.0f32, 1.0f32);
        t = x;
        for _ in 0..32 {
            let value = bezier(t, x1, x2);
            if (value - x).abs() < 1e-5 {
                break;
            }
            if value < x { low = t } else { high = t }
            t = (low + high) / 2.0;
        }
        bezier(t, y1, y2)
    }
}

/// A value under transition.
pub struct Transition {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    easing: Easing,
}

impl Transition {
    pub fn settled(value: f32) -> Self {
        Transition { from: value, to: value, start: Instant::now(), duration: Duration::ZERO, easing: LINEAR }
    }

    pub fn value(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let x = now.saturating_duration_since(self.start).as_secs_f32() / self.duration.as_secs_f32();
        self.from + (self.to - self.from) * self.easing.at(x)
    }

    /// Where the value is heading.
    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.duration
    }

    /// Starts towards `to` from wherever the value is now.
    pub fn retarget(&mut self, to: f32, duration: Duration, easing: Easing, now: Instant) {
        self.from = self.value(now);
        self.to = to;
        self.start = now;
        self.duration = duration;
        self.easing = easing;
    }

    pub fn jump(&mut self, value: f32) {
        *self = Transition::settled(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases() {
        let ease_out = Easing(0.16, 1.0, 0.3, 1.0);
        assert_eq!(ease_out.at(0.0), 0.0);
        assert!((ease_out.at(1.0) - 1.0).abs() < 1e-4);
        assert!(ease_out.at(0.25) > 0.7);
        assert!((LINEAR.at(0.3) - 0.3).abs() < 1e-4);
        let ease_in = Easing(0.4, 0.0, 1.0, 1.0);
        assert!(ease_in.at(0.5) < 0.5);
    }
}
