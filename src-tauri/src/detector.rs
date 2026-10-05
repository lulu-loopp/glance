//! Decides when mouse motion against a screen edge means "open the panel".
//!
//! Reaching the edge is not enough: scroll bars and window borders live there.
//! The pointer has to keep pushing into the edge after it has stopped moving.
//! Relative devices (mice, touchpads) report that push as raw counts the
//! cursor can no longer follow; absolute devices (tablets, remote sessions)
//! cannot push past the edge at all, so for them the signal is resting on it.

use std::time::{Duration, Instant};

/// A pause this long between pushes starts the count again.
const PRESSURE_WINDOW: Duration = Duration::from_millis(250);
/// How long an absolute pointer has to rest on the edge.
const DWELL: Duration = Duration::from_millis(300);

/// One mouse motion reported while the cursor is on the trigger edge.
pub enum Motion {
    /// `outward` is positive when moving into the edge; `along` is the motion parallel to it.
    Relative { outward: i32, along: i32 },
    Absolute,
}

#[derive(Default)]
pub struct Detector {
    pressure: i32,
    last_push: Option<Instant>,
    dwell_since: Option<Instant>,
}

impl Detector {
    /// The cursor left the edge, or the gesture was cancelled.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Feeds one motion; returns true when the panel should open. `pressure`
    /// is the outward travel, in raw counts, that opens it.
    pub fn motion(&mut self, motion: Motion, now: Instant, pressure: i32) -> bool {
        match motion {
            Motion::Relative { outward, along } => {
                self.dwell_since = None;
                if self.last_push.is_some_and(|at| now.duration_since(at) > PRESSURE_WINDOW) {
                    self.pressure = 0;
                }
                if outward > 0 && outward >= along.abs() {
                    self.pressure += outward;
                    self.last_push = Some(now);
                } else if outward < 0 {
                    self.pressure = 0;
                }
                self.pressure >= pressure
            }
            Motion::Absolute => {
                self.pressure = 0;
                self.dwell_since.get_or_insert(now);
                self.dwell_elapsed(now)
            }
        }
    }

    /// True while an absolute pointer is resting on the edge and the clock needs watching.
    pub fn dwelling(&self) -> bool {
        self.dwell_since.is_some()
    }

    pub fn dwell_elapsed(&self, now: Instant) -> bool {
        self.dwell_since.is_some_and(|since| now.duration_since(since) >= DWELL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(outward: i32, along: i32) -> Motion {
        Motion::Relative { outward, along }
    }

    #[test]
    fn sustained_push_opens() {
        let mut detector = Detector::default();
        let start = Instant::now();
        assert!(!detector.motion(push(50, 0), start, 120));
        assert!(!detector.motion(push(50, 3), start + Duration::from_millis(8), 120));
        assert!(detector.motion(push(50, -2), start + Duration::from_millis(16), 120));
    }

    #[test]
    fn brushing_the_edge_does_not_open() {
        let mut detector = Detector::default();
        let start = Instant::now();
        assert!(!detector.motion(push(30, 0), start, 120));
        // Sliding along the edge, as when following a scroll bar.
        for i in 1..200 {
            assert!(!detector.motion(push(1, 12), start + Duration::from_millis(i * 4), 120));
        }
    }

    #[test]
    fn pulling_back_or_pausing_starts_over() {
        let mut detector = Detector::default();
        let start = Instant::now();
        assert!(!detector.motion(push(100, 0), start, 120));
        assert!(!detector.motion(push(-1, 0), start + Duration::from_millis(5), 120));
        assert!(!detector.motion(push(100, 0), start + Duration::from_millis(10), 120));
        assert!(!detector.motion(push(100, 0), start + Duration::from_millis(400), 120));
        assert!(detector.motion(push(20, 0), start + Duration::from_millis(410), 120));
    }

    #[test]
    fn absolute_pointer_opens_after_resting() {
        let mut detector = Detector::default();
        let start = Instant::now();
        assert!(!detector.motion(Motion::Absolute, start, 120));
        assert!(detector.dwelling());
        assert!(!detector.dwell_elapsed(start + Duration::from_millis(299)));
        assert!(detector.dwell_elapsed(start + Duration::from_millis(300)));
        detector.reset();
        assert!(!detector.dwelling());
    }
}
