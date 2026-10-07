//! A game's frames: the moments it handed each to the screen, and what they
//! tell. Frames a second over the last second; the 1% low over the last ten
//! (the 99th percentile frame time, as frames a second: what PresentMon and
//! CapFrameX call it); and the longest frame of the last second, the spike
//! a stutter makes.

use std::collections::VecDeque;
use std::time::Duration;

/// How long frames are kept: the span of the 1% low.
pub const KEPT: Duration = Duration::from_secs(10);
/// The span frames a second, and the longest frame, are taken over; a
/// game with no frame in it is not drawing (paused, minimized, loading).
pub const RECENT: Duration = Duration::from_secs(1);
/// Fewer frames than this over `KEPT` say nothing of the slowest 1%.
const LOW_FRAMES: usize = 20;

/// What a game's frames tell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStats {
    pub fps: f32,
    /// None until enough frames have come.
    pub low: Option<f32>,
    /// The longest frame of the last second, in milliseconds.
    pub longest_ms: f32,
}

/// One program's frames, as the moments it presented them (any clock,
/// counted from any origin).
#[derive(Default)]
pub struct Presents {
    times: VecDeque<Duration>,
}

impl Presents {
    /// A frame presented at `at`, no earlier than the last.
    pub fn push(&mut self, at: Duration) {
        self.times.push_back(at);
        while self.times.front().is_some_and(|&first| at.saturating_sub(first) > KEPT) {
            self.times.pop_front();
        }
    }

    /// The frames as of `now`: none if it has presented no two frames in the
    /// last second.
    pub fn stats(&self, now: Duration) -> Option<FrameStats> {
        let recent: Vec<Duration> = self.times.iter().copied().filter(|&t| now.saturating_sub(t) <= RECENT).collect();
        let (first, last) = (*recent.first()?, *recent.last()?);
        if recent.len() < 2 || last <= first {
            return None;
        }
        let fps = (recent.len() - 1) as f64 / (last - first).as_secs_f64();
        let longest_ms = recent.windows(2).map(|pair| (pair[1] - pair[0]).as_secs_f64() * 1000.0).fold(0.0, f64::max);
        // Every frame time kept, the slowest 1% from the 99th percentile on.
        let mut times: Vec<f64> = self.times.iter().collect::<Vec<_>>().windows(2).map(|pair| (*pair[1] - *pair[0]).as_secs_f64()).collect();
        let low = (times.len() >= LOW_FRAMES).then(|| {
            times.sort_by(f64::total_cmp);
            let index = ((times.len() as f64 * 0.99).ceil() as usize).clamp(1, times.len()) - 1;
            (1.0 / times[index]) as f32
        });
        Some(FrameStats { fps: fps as f32, low, longest_ms: longest_ms as f32 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: f64) -> Duration {
        Duration::from_secs_f64(n / 1000.0)
    }

    #[test]
    fn counts_frames_a_second_and_the_slowest_one_percent() {
        let mut presents = Presents::default();
        // Ten seconds at 100 frames a second (10 ms each), one frame in a
        // hundred taking 50 ms.
        let mut t = 0.0;
        for i in 0..1000 {
            t += if i % 100 == 99 { 50.0 } else { 10.0 };
            presents.push(ms(t));
        }
        let stats = presents.stats(ms(t)).unwrap();
        assert!((stats.fps - 100.0).abs() < 5.0, "{stats:?}");
        // The 99th percentile frame time is the 50 ms one: 20 frames a second.
        assert!((stats.low.unwrap() - 20.0).abs() < 0.5, "{stats:?}");
        assert!((stats.longest_ms - 50.0).abs() < 0.5, "{stats:?}");
    }

    #[test]
    fn tells_nothing_of_a_game_not_drawing() {
        let mut presents = Presents::default();
        presents.push(ms(0.0));
        presents.push(ms(16.0));
        // A frame and its follower, then nothing for two seconds.
        assert!(presents.stats(ms(16.0)).is_some());
        assert_eq!(presents.stats(ms(2016.0)), None);
        // Too few frames for a 1% low.
        assert_eq!(presents.stats(ms(16.0)).unwrap().low, None);
    }

    #[test]
    fn keeps_only_the_last_ten_seconds() {
        let mut presents = Presents::default();
        for i in 0..2000 {
            presents.push(ms(i as f64 * 10.0));
        }
        assert!(presents.times.len() <= 1001);
    }
}
