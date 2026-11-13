//! Things chosen together on a station's lane (on the station, copied lately, come lately), as Explorer chooses files:
//! Ctrl and a press take one in or out, Shift and a press take all from the one pressed last to it (Ctrl and Shift:
//! those, besides what is chosen already). What is chosen is dragged off together, put on the clipboard together, and
//! looked at one after another (see `ui::look`).
//!
//! What is chosen lives in one list: a page of a lane in one window (the panel's, a widget's). A press in another list
//! leaves it, as does the page turned, Esc, a press where nothing is, and its window going. A press alone keeps doing
//! what it did before anything could be chosen: the thing put on the clipboard, nothing chosen after it.
//!
//! What is chosen is the windows' thread's alone (they press, draw and drag on it): kept here, as the things held to
//! be dragged off are (see `ui::view::press_tile`).

use crate::ui::view::{Hit, Tab};

/// The keys held as a press came: Ctrl, Shift.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    pub ctrl: bool,
    pub shift: bool,
}

impl Keys {
    /// As a press's message has them (its `wParam`: `MK_CONTROL`, `MK_SHIFT`).
    pub fn of(wparam: usize) -> Keys {
        Keys { ctrl: wparam & 0x0008 != 0, shift: wparam & 0x0004 != 0 }
    }

    /// Neither held.
    pub fn none(self) -> bool {
        !self.ctrl && !self.shift
    }
}

/// A list things are chosen in: the window it shows in (its handle, as a number) and the page of the lane it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct On {
    pub window: isize,
    pub page: Tab,
}

impl On {
    /// The list `hit` is in, in `window`, its lane showing the page `shown` (a thing on the station shows on its page
    /// whatever page is chosen where the lane is a row of them).
    pub fn of(window: isize, hit: Hit, shown: Tab) -> On {
        let page = match (hit, shown) {
            (Hit::Copied(_), Tab::Copies(_)) | (Hit::Recent(_), Tab::Recent(_)) => shown,
            (Hit::Copied(_), _) => Tab::Copies(Default::default()),
            (Hit::Recent(_), _) => Tab::Recent(Default::default()),
            _ => Tab::Shelf,
        };
        On { window, page }
    }
}

/// What is chosen: the list it is in, the things chosen there, and the one pressed last there (where Shift takes from).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chosen {
    pub on: Option<On>,
    pub hits: Vec<Hit>,
    pub anchor: Option<Hit>,
}

impl Chosen {
    /// What is marked chosen in the window `window`: what is chosen there, none of what is chosen in another window
    /// that shows the same things (a press, a drag or a menu there takes them one by one).
    pub fn marked_in(&self, window: isize) -> Vec<Hit> {
        if self.on.is_some_and(|on| on.window == window) { self.hits.clone() } else { Vec::new() }
    }

    /// Whether `hit` in the list `on` is chosen.
    pub fn holds(&self, on: On, hit: Hit) -> bool {
        self.on == Some(on) && self.hits.contains(&hit)
    }

    /// What is chosen in the list `on`, as it is now (`order`: its things, in their order), in that order: none of
    /// another list, nor of what is no longer in it.
    pub fn in_order(&self, on: On, order: &[Hit]) -> Vec<Hit> {
        if self.on != Some(on) {
            return Vec::new();
        }
        order.iter().copied().filter(|hit| self.hits.contains(hit)).collect()
    }

    /// Nothing chosen, nothing pressed last.
    pub fn clear(&mut self) {
        *self = Chosen::default();
    }

    /// Nothing chosen in `on` any more, where what is chosen is there.
    fn leave_others(&mut self, on: On) {
        if self.on != Some(on) {
            *self = Chosen { on: Some(on), hits: Vec::new(), anchor: None };
        }
    }

    /// `hit` pressed alone: nothing chosen, it pressed last.
    fn alone(&mut self, on: On, hit: Hit) {
        *self = Chosen { on: Some(on), hits: Vec::new(), anchor: Some(hit) };
    }

    /// `hit` taken in or out (Ctrl and a press), and pressed last.
    fn toggle(&mut self, on: On, hit: Hit) {
        self.leave_others(on);
        match self.hits.iter().position(|chosen| *chosen == hit) {
            Some(at) => {
                self.hits.remove(at);
            }
            None => self.hits.push(hit),
        }
        self.anchor = Some(hit);
    }

    /// All from the one pressed last to `hit` chosen (Shift and a press), besides what is chosen already (`add`: Ctrl
    /// too) or alone. With none pressed last in the list (or it gone from it), `hit` alone, and pressed last.
    fn range(&mut self, on: On, hit: Hit, order: &[Hit], add: bool) {
        self.leave_others(on);
        let span = match (self.anchor.and_then(|anchor| order.iter().position(|one| *one == anchor)), order.iter().position(|one| *one == hit)) {
            (Some(from), Some(to)) => &order[from.min(to)..=from.max(to)],
            _ => {
                self.hits = vec![hit];
                self.anchor = Some(hit);
                return;
            }
        };
        let kept: Vec<Hit> = if add { self.hits.iter().copied().filter(|chosen| !span.contains(chosen)).collect() } else { Vec::new() };
        self.hits = kept.into_iter().chain(span.iter().copied()).collect();
    }

    /// All of the list `on` chosen (`order`: its things), the one pressed last kept where it is in it.
    pub fn all(&mut self, on: On, order: &[Hit]) {
        self.leave_others(on);
        self.hits = order.to_vec();
        if self.anchor.is_none_or(|anchor| !order.contains(&anchor)) {
            self.anchor = order.first().copied();
        }
    }

    /// `hit` in the list `on` pressed with `keys` held (`order`: the list's things): Shift chooses at once, as the
    /// press comes, the rest waits for it to be let go or dragged.
    pub fn pressed(&mut self, on: On, hit: Hit, keys: Keys, order: &[Hit]) {
        if keys.shift {
            self.range(on, hit, order, keys.ctrl);
        } else if self.on.is_some_and(|chosen| chosen != on) {
            // (Pressed in another list: what was chosen is left.)
            self.leave_others(on);
        }
    }

    /// `hit` in the list `on`, pressed with `keys` held, let go where it was, never dragged: whether it is a press
    /// alone, done as one ever was (put on the clipboard), nothing chosen after it. With Ctrl, taken in or out, with
    /// Shift, chosen as it was pressed.
    pub fn released(&mut self, on: On, hit: Hit, keys: Keys) -> bool {
        match (keys.ctrl, keys.shift) {
            (_, true) => false,
            (true, false) => {
                self.toggle(on, hit);
                false
            }
            (false, false) => {
                self.alone(on, hit);
                true
            }
        }
    }

    /// `hit` in the list `on` given a right click (`order`: the list's things): what its menu is for, where it is one
    /// of several chosen together, all of them in the list's order, else none (its own menu), and where it is not
    /// chosen, nothing chosen any more.
    pub fn menu_on(&mut self, on: On, hit: Hit, order: &[Hit]) -> Vec<Hit> {
        if !self.holds(on, hit) {
            self.clear();
            return Vec::new();
        }
        let together = self.in_order(on, order);
        if together.len() > 1 { together } else { Vec::new() }
    }

    /// `hit` in the list `on`, pressed with `keys` held, dragged off: what goes, it first and what else is chosen
    /// after it in the list's order (`order`). Chosen, all that is chosen goes. Not chosen, with Ctrl it is taken in
    /// and all goes, else it goes alone and nothing is chosen.
    pub fn dragged(&mut self, on: On, hit: Hit, keys: Keys, order: &[Hit]) -> Vec<Hit> {
        if !self.holds(on, hit) {
            if keys.ctrl {
                self.toggle(on, hit);
            } else {
                self.alone(on, hit);
                return vec![hit];
            }
        }
        std::iter::once(hit).chain(self.in_order(on, order).into_iter().filter(|chosen| *chosen != hit)).collect()
    }
}

/// A thing on a station's lane pressed: it, the list it is in, the keys held as it was, and where the pointer was
/// (screen px). Dragged off once the pointer goes far enough (see `run::drag_begins`), else let go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub hit: Hit,
    pub on: On,
    pub keys: Keys,
    pub from: windows::Win32::Foundation::POINT,
}

/// `hit` pressed with `keys` held in the window `window`, its lane showing the page `shown`, the pointer at `from`:
/// what is chosen as the press comes (see `Chosen::pressed`).
pub fn press(window: isize, hit: Hit, shown: Tab, keys: Keys, from: windows::Win32::Foundation::POINT) -> Press {
    let on = On::of(window, hit, shown);
    change(|chosen| chosen.pressed(on, hit, keys, &crate::ui::view::lane_order(on.page)));
    Press { hit, on, keys, from }
}

/// `press` let go where it was: whether it is done as a press alone (see `Chosen::released`).
pub fn release(press: Press) -> bool {
    change(|chosen| chosen.released(press.on, press.hit, press.keys))
}

/// `press` dragged off: what goes, the thing pressed first (see `Chosen::dragged`).
pub fn drag(press: Press) -> Vec<Hit> {
    change(|chosen| chosen.dragged(press.on, press.hit, press.keys, &crate::ui::view::lane_order(press.on.page)))
}

/// `hit` given a right click in the window `window` showing the page `shown`: what the menu of things chosen together
/// is for, or none (see `Chosen::menu_on`).
pub fn menu_on(window: isize, hit: Hit, shown: Tab) -> Vec<Hit> {
    let on = On::of(window, hit, shown);
    change(|chosen| chosen.menu_on(on, hit, &crate::ui::view::lane_order(on.page)))
}

/// All of the list `hit` is in chosen, in the window `window` showing the page `shown` (Ctrl+A over it).
pub fn all(window: isize, hit: Hit, shown: Tab) {
    let on = On::of(window, hit, shown);
    change(|chosen| chosen.all(on, &crate::ui::view::lane_order(on.page)));
}

/// What is chosen now.
static CHOSEN: std::sync::Mutex<Chosen> = std::sync::Mutex::new(Chosen { on: None, hits: Vec::new(), anchor: None });

/// What is chosen changed by `change`, and what it gives: where the things chosen changed, what shows them is drawn
/// again (see `ui::view::tile_changes`).
pub fn change<R>(change: impl FnOnce(&mut Chosen) -> R) -> R {
    let mut chosen = CHOSEN.lock().unwrap();
    let was = chosen.hits.clone();
    let given = change(&mut chosen);
    if chosen.hits != was {
        crate::ui::view::tiles_chosen();
    }
    given
}

/// What is chosen now.
pub fn now() -> Chosen {
    CHOSEN.lock().unwrap().clone()
}

/// What is marked chosen in the window `window` (see `Chosen::marked_in`).
pub fn marked(window: isize) -> Vec<Hit> {
    now().marked_in(window)
}

/// The window what is chosen is in, where anything is.
pub fn window() -> Option<isize> {
    let chosen = CHOSEN.lock().unwrap();
    chosen.on.filter(|_| !chosen.hits.is_empty()).map(|on| on.window)
}

/// What is chosen in the window `window`, in its list's order: none where nothing is chosen there.
pub fn in_window(window: isize) -> Vec<Hit> {
    let chosen = now();
    chosen.on.filter(|on| on.window == window).map_or_else(Vec::new, |on| chosen.in_order(on, &crate::ui::view::lane_order(on.page)))
}

/// Nothing chosen in the window `window` any more (a press there where nothing is, it gone).
pub fn leave(window: isize) {
    change(|chosen| {
        if chosen.on.is_some_and(|on| on.window == window) {
            chosen.clear();
        }
    });
}

/// The page shown in the window `window` turned to `page`: what was chosen on another page there, no longer.
pub fn turned(window: isize, page: Tab) {
    change(|chosen| {
        if chosen.on.is_some_and(|on| on.window == window && on.page != page) {
            chosen.clear();
        }
    });
}

/// How many files land where `what` is dropped: each of files put on the station together, one of anything else
/// (words, a link, a picture: one file each).
pub fn files_in(what: &super::shelf::What) -> usize {
    match what {
        super::shelf::What::Files { paths, .. } => paths.len(),
        super::shelf::What::Came { files } => super::shelf::entries(files).len(),
        _ => 1,
    }
}

/// How many files land where `hits` are dropped (see `files_in`, files come together, each of them).
pub fn landing(hits: &[Hit]) -> usize {
    let shown = super::run::shown();
    hits.iter()
        .map(|hit| match *hit {
            Hit::Shelf(id) | Hit::Copied(id) => shown.items.iter().find(|item| item.id == id).map_or(0, |item| files_in(&item.what)),
            Hit::Recent(entry) => super::recent::paths_of(entry).map_or(0, |paths| paths.len()),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: On = On { window: 7, page: Tab::Shelf };
    const ORDER: [Hit; 6] = [Hit::Shelf(10), Hit::Shelf(11), Hit::Shelf(12), Hit::Shelf(13), Hit::Shelf(14), Hit::Shelf(15)];
    const CTRL: Keys = Keys { ctrl: true, shift: false };
    const SHIFT: Keys = Keys { ctrl: false, shift: true };
    const BOTH: Keys = Keys { ctrl: true, shift: true };
    const ALONE: Keys = Keys { ctrl: false, shift: false };

    /// What is chosen is marked in its own window alone, not in another showing the same things.
    #[test]
    fn what_is_chosen_is_marked_in_its_own_window_alone() {
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, ORDER[1], CTRL);
        click(&mut chosen, ON, ORDER[3], CTRL);
        assert_eq!(chosen.marked_in(ON.window), [ORDER[1], ORDER[3]]);
        assert_eq!(chosen.marked_in(ON.window + 1), []);
    }

    /// A press and its letting go, never dragged: whether it was a press alone.
    fn click(chosen: &mut Chosen, on: On, hit: Hit, keys: Keys) -> bool {
        chosen.pressed(on, hit, keys, &ORDER);
        chosen.released(on, hit, keys)
    }

    /// The keys a press's message says were held.
    #[test]
    fn the_keys_held_are_read_off_the_press() {
        assert_eq!(Keys::of(0x0001), ALONE);
        assert_eq!(Keys::of(0x0001 | 0x0008), CTRL);
        assert_eq!(Keys::of(0x0004), SHIFT);
        assert_eq!(Keys::of(0x0008 | 0x0004 | 0x0001), BOTH);
    }

    /// Ctrl and a press take a thing in, and out again, each pressed last. None of them is a press alone.
    #[test]
    fn ctrl_takes_one_in_and_out() {
        let mut chosen = Chosen::default();
        assert!(!click(&mut chosen, ON, Hit::Shelf(11), CTRL));
        assert!(!click(&mut chosen, ON, Hit::Shelf(13), CTRL));
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(11), Hit::Shelf(13)]);
        assert_eq!(chosen.anchor, Some(Hit::Shelf(13)));
        assert!(!click(&mut chosen, ON, Hit::Shelf(11), CTRL));
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(13)]);
        assert_eq!(chosen.anchor, Some(Hit::Shelf(11)));
    }

    /// Shift and a press take all from the one pressed last (alone or with Ctrl) to it, either way along the list, in
    /// place of what was chosen, Ctrl and Shift, besides it.
    #[test]
    fn shift_takes_all_from_the_one_pressed_last() {
        let mut chosen = Chosen::default();
        // (Pressed alone first: the press done as ever, and pressed last.)
        assert!(click(&mut chosen, ON, Hit::Shelf(12), ALONE));
        assert!(chosen.hits.is_empty());
        assert!(!click(&mut chosen, ON, Hit::Shelf(14), SHIFT));
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(12), Hit::Shelf(13), Hit::Shelf(14)]);
        // From the same one, the other way: in place of those.
        click(&mut chosen, ON, Hit::Shelf(10), SHIFT);
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(10), Hit::Shelf(11), Hit::Shelf(12)]);
        assert_eq!(chosen.anchor, Some(Hit::Shelf(12)));
        // Ctrl pressed last, Ctrl and Shift: from it, besides what is chosen.
        click(&mut chosen, ON, Hit::Shelf(15), CTRL);
        click(&mut chosen, ON, Hit::Shelf(14), BOTH);
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(10), Hit::Shelf(11), Hit::Shelf(12), Hit::Shelf(14), Hit::Shelf(15)]);
        // Shift with none pressed last: it alone.
        let mut fresh = Chosen::default();
        click(&mut fresh, ON, Hit::Shelf(13), SHIFT);
        assert_eq!((fresh.in_order(ON, &ORDER), fresh.anchor), (vec![Hit::Shelf(13)], Some(Hit::Shelf(13))));
    }

    /// Shift chooses as it is pressed (dragged at once, what it chose goes), Ctrl as it is let go (pressed on one
    /// chosen and dragged, it is not taken out first).
    #[test]
    fn shift_chooses_as_it_is_pressed_and_ctrl_as_it_is_let_go() {
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, Hit::Shelf(11), CTRL);
        chosen.pressed(ON, Hit::Shelf(13), SHIFT, &ORDER);
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(11), Hit::Shelf(12), Hit::Shelf(13)]);
        chosen.pressed(ON, Hit::Shelf(12), CTRL, &ORDER);
        assert_eq!(chosen.dragged(ON, Hit::Shelf(12), CTRL, &ORDER), [Hit::Shelf(12), Hit::Shelf(11), Hit::Shelf(13)]);
        assert_eq!(chosen.in_order(ON, &ORDER).len(), 3);
    }

    /// A press alone is done as ever and leaves nothing chosen, pressed last, on one chosen too.
    #[test]
    fn a_press_alone_chooses_nothing() {
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, Hit::Shelf(10), CTRL);
        click(&mut chosen, ON, Hit::Shelf(12), CTRL);
        assert!(click(&mut chosen, ON, Hit::Shelf(12), ALONE));
        assert_eq!(chosen, Chosen { on: Some(ON), hits: Vec::new(), anchor: Some(Hit::Shelf(12)) });
    }

    /// Dragged: one chosen takes all that is chosen with it, it first, the rest in the list's order. One not chosen goes
    /// alone and leaves nothing chosen. With Ctrl, it is taken in and all go.
    #[test]
    fn what_is_chosen_is_dragged_together() {
        let mut chosen = Chosen::default();
        for hit in [Hit::Shelf(14), Hit::Shelf(10), Hit::Shelf(12)] {
            click(&mut chosen, ON, hit, CTRL);
        }
        chosen.pressed(ON, Hit::Shelf(12), ALONE, &ORDER);
        assert_eq!(chosen.dragged(ON, Hit::Shelf(12), ALONE, &ORDER), [Hit::Shelf(12), Hit::Shelf(10), Hit::Shelf(14)]);
        let mut with_ctrl = chosen.clone();
        assert_eq!(with_ctrl.dragged(ON, Hit::Shelf(11), CTRL, &ORDER), [Hit::Shelf(11), Hit::Shelf(10), Hit::Shelf(12), Hit::Shelf(14)]);
        assert_eq!(chosen.dragged(ON, Hit::Shelf(11), ALONE, &ORDER), [Hit::Shelf(11)]);
        assert!(chosen.hits.is_empty());
        assert_eq!(chosen.anchor, Some(Hit::Shelf(11)));
    }

    /// What is chosen lives in one list: a press in another (another page, another window) leaves it, Ctrl or not.
    #[test]
    fn what_is_chosen_lives_in_one_list() {
        let copies = On { window: 7, page: Tab::Copies(Default::default()) };
        let widget = On { window: 9, page: Tab::Shelf };
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, Hit::Shelf(10), CTRL);
        click(&mut chosen, ON, Hit::Shelf(11), CTRL);
        chosen.pressed(copies, Hit::Copied(3), ALONE, &[Hit::Copied(3)]);
        assert!(chosen.in_order(ON, &ORDER).is_empty());
        click(&mut chosen, ON, Hit::Shelf(10), CTRL);
        click(&mut chosen, widget, Hit::Shelf(11), CTRL);
        assert_eq!(chosen.in_order(widget, &ORDER), [Hit::Shelf(11)]);
        assert!(chosen.in_order(ON, &ORDER).is_empty());
        // Shift in another list: from nothing pressed there, it alone.
        click(&mut chosen, ON, Hit::Shelf(13), SHIFT);
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(13)]);
    }

    /// A right click on one of several chosen: their menu, for all of them in the list's order. On one chosen alone:
    /// its own menu, it kept chosen. On one not chosen: its own menu, nothing chosen any more.
    #[test]
    fn a_right_click_on_what_is_chosen_is_for_all_of_it() {
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, Hit::Shelf(13), CTRL);
        assert!(chosen.menu_on(ON, Hit::Shelf(13), &ORDER).is_empty());
        assert_eq!(chosen.in_order(ON, &ORDER), [Hit::Shelf(13)]);
        click(&mut chosen, ON, Hit::Shelf(11), CTRL);
        assert_eq!(chosen.menu_on(ON, Hit::Shelf(13), &ORDER), [Hit::Shelf(11), Hit::Shelf(13)]);
        assert!(chosen.menu_on(ON, Hit::Shelf(12), &ORDER).is_empty());
        assert!(chosen.hits.is_empty());
    }

    /// All of a list chosen, in its order (what was pressed last kept, where it is still in it), what is gone from the
    /// list is no longer counted chosen.
    #[test]
    fn all_of_a_list_is_chosen_and_what_goes_from_it_is_not() {
        let mut chosen = Chosen::default();
        click(&mut chosen, ON, Hit::Shelf(12), ALONE);
        chosen.all(ON, &ORDER);
        assert_eq!(chosen.in_order(ON, &ORDER), ORDER);
        assert_eq!(chosen.anchor, Some(Hit::Shelf(12)));
        let fewer = [Hit::Shelf(10), Hit::Shelf(15)];
        assert_eq!(chosen.in_order(ON, &fewer), fewer);
    }

    /// The list a thing is in: the page shown, of its kind (a filter of it kept), a thing on the station, its page,
    /// shown as a row of them whatever page is chosen.
    #[test]
    fn a_thing_is_in_the_list_of_its_page() {
        use crate::station::shelf::Filter;
        let words = Tab::Copies(Filter::Words);
        assert_eq!(On::of(3, Hit::Copied(1), words).page, words);
        assert_eq!(On::of(3, Hit::Copied(1), Tab::Shelf).page, Tab::Copies(Filter::All));
        assert_eq!(On::of(3, Hit::Shelf(1), words).page, Tab::Shelf);
        let shots = Tab::Recent(crate::station::recent::Filter::Screenshots);
        let entry = crate::station::recent::Entry::File(crate::station::recent::Key { drive: 'C', file: 1 });
        assert_eq!(On::of(3, Hit::Recent(entry), shots), On { window: 3, page: shots });
    }

    /// Files put on the station together land as so many files, anything else as one.
    #[test]
    fn files_together_land_as_so_many() {
        use crate::station::shelf::What;
        let files = What::Files { paths: vec!["C:\\a".into(), "C:\\b".into(), "C:\\c".into()], kept: Vec::new() };
        assert_eq!(files_in(&files), 3);
        assert_eq!(files_in(&What::Text { text: "a".into() }), 1);
        assert_eq!(files_in(&What::Link { url: "https://a.b".into() }), 1);
        assert_eq!(files_in(&What::Picture { file: "1.png".into() }), 1);
    }
}
