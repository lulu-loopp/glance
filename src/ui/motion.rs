//! Transitions as CSS times them: a value moving from where it is to a
//! target over a duration, along a cubic Bézier easing curve. And springs: a
//! value pulled toward its target, as fast as it is going kept as the target
//! changes (a thing in hand, followed as the hand changes its mind).

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

/// A value on a spring: pulled toward its target as hard as `stiff` times how far off it is, held back as hard as
/// `damp` times how fast it goes (per second). Damped less than critically (`damp` under 2√`stiff`), it goes a little
/// past before it rests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub value: f32,
    pub speed: f32,
    pub target: f32,
    stiff: f32,
    damp: f32,
}

impl Spring {
    /// At rest at `value`.
    pub const fn at(value: f32, stiff: f32, damp: f32) -> Spring {
        Spring { value, speed: 0.0, target: value, stiff, damp }
    }

    /// `seconds` on: in steps short enough to hold it steady, however long since the last.
    pub fn step(&mut self, seconds: f32) {
        const STEP: f32 = 0.004;
        let steps = (seconds / STEP).ceil().max(1.0) as usize;
        let dt = seconds / steps as f32;
        for _ in 0..steps {
            self.speed += (self.stiff * (self.target - self.value) - self.damp * self.speed) * dt;
            self.value += self.speed * dt;
        }
    }

    /// Whether it is at rest: within `near` of its target, going slower than `near` a tenth of a second.
    pub fn rests(&self, near: f32) -> bool {
        (self.target - self.value).abs() <= near && self.speed.abs() * 0.1 <= near
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spring let go reaches its target and rests there; damped under critically, it goes past it first, a little
    /// (under a tenth of the way); critically or more, never. However finely or coarsely it is stepped, alike.
    #[test]
    fn a_spring_rests_at_its_target() {
        for frame in [0.004, 1.0 / 60.0, 0.05] {
            let mut under = Spring::at(0.0, 260.0, 26.0);
            let mut critical = Spring::at(0.0, 400.0, 40.0);
            under.target = 1.0;
            critical.target = 1.0;
            let (mut most_under, mut most_critical) = (0.0f32, 0.0f32);
            for _ in 0..(1.5 / frame) as usize {
                under.step(frame);
                critical.step(frame);
                most_under = most_under.max(under.value);
                most_critical = most_critical.max(critical.value);
            }
            assert!(under.rests(0.001) && critical.rests(0.001), "{frame}: {under:?} {critical:?}");
            assert!(most_under > 1.0 && most_under < 1.1, "{frame}: {most_under}");
            assert!(most_critical <= 1.0 + 1e-4, "{frame}: {most_critical}");
        }
    }

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
