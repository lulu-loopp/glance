//! The desktops the settings read from the wallpaper, on a thread of their own (see `Ui::read_wanted`): each asked
//! once till the screens or the wallpaper change, those wanted together read together (a live wallpaper printed once
//! for them all: see `wallpaper::desktops`), one lot at a time, and a lot that does not come back given up.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::RECT;

/// How long a lot of desktops may be read for before it is given up: they are read in well under a second.
pub(super) const GIVEN_UP_AFTER: Duration = Duration::from_secs(15);

/// A desktop the settings show: a screen's work area as the preview shows it (the screen by its name, its work area
/// in physical px, its size in DIPs, which it is read at), or a screen whole as the layout's map shows it (by its
/// name, where it is, and the picture's size in physical px).
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Ask {
    Stage { screen: String, work: RECT, size: (f32, f32) },
    Map { id: String, monitor: RECT, size: (u32, u32) },
}

impl Ask {
    /// What of the desktop is read, and the picture's width and height (as `wallpaper::desktops` is asked).
    pub(super) fn area(&self) -> (RECT, u32, u32) {
        match self {
            Ask::Stage { work, size, .. } => (*work, size.0 as u32, size.1 as u32),
            Ask::Map { monitor, size, .. } => (*monitor, size.0, size.1),
        }
    }
}

/// Where a desktop asked for is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    /// To go with the next lot.
    Wanted,
    /// Being read, in the lot of that number.
    Out(u64),
    /// Given up (see `GIVEN_UP_AFTER`): shown as a desktop not read is, and not asked again.
    Failed,
}

/// The desktops asked for since the screens or the wallpaper last changed and not yet come (a desktop that has come is
/// what asked for it holds: asked again only once that no longer holds it), and the lot being read, if one is: its
/// number and when it went.
#[derive(Default)]
pub(super) struct Reads {
    asked: Vec<(Ask, State)>,
    out: Option<(u64, Instant)>,
    sent: u64,
}

impl Reads {
    /// `ask` wanted: it goes with the next lot, unless it is being read or was given up.
    pub(super) fn want(&mut self, ask: Ask) {
        if !self.asked.iter().any(|(asked, _)| *asked == ask) {
            self.asked.push((ask, State::Wanted));
        }
    }

    /// The lot to read now, and its number: every desktop wanted, while no other lot is being read.
    pub(super) fn lot(&mut self, now: Instant) -> Option<(u64, Vec<Ask>)> {
        if self.out.is_some() {
            return None;
        }
        let number = self.sent;
        let lot: Vec<Ask> = self
            .asked
            .iter_mut()
            .filter(|(_, state)| *state == State::Wanted)
            .map(|(ask, state)| {
                *state = State::Out(number);
                ask.clone()
            })
            .collect();
        if lot.is_empty() {
            return None;
        }
        self.sent += 1;
        self.out = Some((number, now));
        Some((number, lot))
    }

    /// Whether `ask`, read in lot `lot`, is taken: it is while it is still being read in it (not if that lot was given
    /// up, or the screens or the wallpaper changed since it went).
    pub(super) fn came(&mut self, lot: u64, ask: &Ask) -> bool {
        let at = self.asked.iter().position(|(asked, state)| asked == ask && *state == State::Out(lot));
        if let Some(at) = at {
            self.asked.remove(at);
        }
        at.is_some()
    }

    /// Lot `lot` has all come: the next may go.
    pub(super) fn done(&mut self, lot: u64) {
        if self.out.is_some_and(|(out, _)| out == lot) {
            self.out = None;
        }
    }

    /// The lot being read given up, if it has been for `GIVEN_UP_AFTER`: what it has yet to bring failed, and the next
    /// lot free to go. Whether it was.
    pub(super) fn give_up(&mut self, now: Instant) -> bool {
        let Some((lot, _)) = self.out.filter(|(_, went)| now.duration_since(*went) >= GIVEN_UP_AFTER) else { return false };
        for (_, state) in &mut self.asked {
            if *state == State::Out(lot) {
                *state = State::Failed;
            }
        }
        self.out = None;
        true
    }

    /// The screens or the wallpaper changed: every desktop may be asked again, what was failed too. A lot being read
    /// meanwhile is still waited for (or given up) before the next goes, and what it brings is not taken.
    pub(super) fn forget(&mut self) {
        self.asked.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(id: &str, size: u32) -> Ask {
        Ask::Map { id: id.into(), monitor: RECT { left: 0, top: 0, right: 3840, bottom: 2160 }, size: (size, size / 2) }
    }

    /// A desktop is read once however often it is wanted, those wanted meanwhile together once the lot before has
    /// come, and one wanted again once it has come (what wanted it no longer holding it) is read again.
    #[test]
    fn a_desktop_is_read_once_however_often_it_is_wanted() {
        let mut reads = Reads::default();
        let now = Instant::now();
        assert_eq!(reads.lot(now), None);
        for _ in 0..3 {
            reads.want(map("a", 400));
            reads.want(map("b", 400));
        }
        assert_eq!(reads.lot(now), Some((0, vec![map("a", 400), map("b", 400)])));
        // Being read: wanted again (every frame that draws it), not asked again.
        reads.want(map("a", 400));
        reads.want(map("c", 400));
        assert_eq!(reads.lot(now), None);
        assert!(reads.came(0, &map("a", 400)));
        assert!(!reads.came(0, &map("a", 400)));
        assert!(reads.came(0, &map("b", 400)));
        assert_eq!(reads.lot(now), None);
        reads.done(0);
        assert_eq!(reads.lot(now), Some((1, vec![map("c", 400)])));
        reads.done(1);
        reads.want(map("a", 400));
        assert_eq!(reads.lot(now), Some((2, vec![map("a", 400)])));
    }

    /// A lot that does not come back is given up after a while: what it had yet to bring is failed and not asked
    /// again, and taken no more if it comes after all, and the next lot goes. Once the screens or the wallpaper
    /// change, a failed one is asked again, and what a lot read before brings is not taken.
    #[test]
    fn a_read_that_does_not_come_back_is_given_up() {
        let mut reads = Reads::default();
        let now = Instant::now();
        reads.want(map("a", 400));
        reads.want(map("b", 400));
        assert_eq!(reads.lot(now).map(|(lot, _)| lot), Some(0));
        assert!(reads.came(0, &map("a", 400)));
        reads.want(map("c", 400));
        assert!(!reads.give_up(now + GIVEN_UP_AFTER / 2));
        assert_eq!(reads.lot(now + GIVEN_UP_AFTER / 2), None);
        assert!(reads.give_up(now + GIVEN_UP_AFTER));
        assert!(!reads.give_up(now + GIVEN_UP_AFTER));
        assert_eq!(reads.lot(now + GIVEN_UP_AFTER), Some((1, vec![map("c", 400)])));
        reads.want(map("b", 400));
        assert!(!reads.came(0, &map("b", 400)));
        reads.done(0);
        assert!(reads.came(1, &map("c", 400)));
        reads.done(1);
        assert_eq!(reads.lot(now + GIVEN_UP_AFTER), None);
        // The wallpaper changed while a lot was out: it is waited for, and what it brings is not taken.
        reads.want(map("d", 400));
        assert_eq!(reads.lot(now), Some((2, vec![map("d", 400)])));
        reads.forget();
        reads.want(map("b", 400));
        reads.want(map("d", 400));
        assert_eq!(reads.lot(now), None);
        assert!(!reads.came(2, &map("d", 400)));
        reads.done(2);
        assert_eq!(reads.lot(now), Some((3, vec![map("b", 400), map("d", 400)])));
    }
}
