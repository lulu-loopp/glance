//! What a press on the to-dos' lane does (see `view::todo`), in whichever window shows it (the panel's, a widget's); the
//! field an item is written in there, its window taking the keys meanwhile (see `ui::keys`); and the picker of an item's
//! day and time (see `pick`).
//!
//! What is written in the field stays as it was written while its window gives the keys back (another window came in
//! front, the picker opened from it): written in again, it goes on from there. Enter adds it as an item, its time the
//! one set for it, else one its words name (see `todo::words`), and the field is empty for the next.
//!
//! An item not done is written in where it shows, its words pressed (the caret put where they were pressed), in the
//! field's stead: its row as it was, with the caret. Enter, or a press anywhere else, keeps what is written there (a
//! time its words name its new time, the words naming it left out, else its time as it was), Esc leaves it as it was,
//! and so does nothing written: an item is taken out by its button. It keeps its place.

pub mod pick;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{WM_APP, WM_CHAR, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP};

use super::typing::Typing;
use super::view::todo::{self as lane, key_of, Field, TodoHit};
use crate::todo::{Day, Due, Item, List};

/// Posted to the window an item's field was written in as the picker opened from it goes: it is written in again.
pub const RESUME: u32 = WM_APP + 41;

/// How long the caret shows, and is hidden, as it blinks.
const BLINK: Duration = Duration::from_millis(530);

/// The most an item's words hold (UTF-16 units).
const MOST: usize = 500;

/// The to-do a notification of it opened the to-dos for, by its words and its time as kept (see `remind::key`), and
/// since when: lit a moment where it shows (see `lit`).
static LIT: std::sync::Mutex<Option<(String, Instant)>> = std::sync::Mutex::new(None);

/// How long a to-do opened from its notification is lit.
const LIGHT: Duration = Duration::from_millis(2400);

/// The to-do `key` (see `remind::key`) lit where it shows, from now (none: none).
pub fn light(key: Option<String>) {
    *LIT.lock().unwrap() = key.map(|key| (key, Instant::now()));
}

/// How lit `item` is drawn at `now` (0 not at all, 1 most), as the panel opens on it.
pub fn lit(item: &Item, now: Instant) -> f32 {
    let lit = LIT.lock().unwrap();
    let Some((key, since)) = lit.as_ref() else { return 0.0 };
    if crate::remind::key(item).as_ref() != Some(key) {
        return 0.0;
    }
    let t = now.saturating_duration_since(*since).as_secs_f32() / LIGHT.as_secs_f32();
    if t >= 1.0 {
        return 0.0;
    }
    // (Up over the first part of it, held, and gone over the last two fifths.)
    (t / 0.08).min(1.0) * ((1.0 - t) / 0.4).min(1.0)
}

/// What a press on the lane asks of the window it is in, besides what it did itself.
pub enum Act {
    Nothing,
    /// The field written in: the window is to take the keys (see `write_in`).
    Write,
    /// The picker of a time opened, from where on the lane it was pressed (see `pick::open`).
    Pick(PickFor),
    /// The `n`th item, as it is, written in where it shows (see `edit`).
    Edit(usize, Item),
}

/// What the picker sets the time of: the item being written in the field, or the `n`th item, as it is.
#[derive(Clone, PartialEq, Debug)]
pub enum PickFor {
    Field,
    Item(usize, Item),
}

/// The `n`th item where it is still the one keyed `key` (see `key_of`).
fn item_at(list: &List, n: usize, key: u64) -> Option<Item> {
    let items = list.items();
    (n < items.len() && key_of(&items, n) == key).then(|| items[n].clone())
}

/// A press on the lane, on `hit`: done (ticked, unticked, taken out, cleared, those done folded or shown), or asked of
/// its window (see `Act`).
pub fn pressed(hit: TodoHit) -> Act {
    let list = crate::todo::list();
    match hit {
        TodoHit::Tick(n, key) => {
            let (n, key) = (n as usize, key);
            if let Some(item) = item_at(&list, n, key) {
                if item.done {
                    crate::todo::tick(n, &item, false);
                } else {
                    // Ticked where it is, among those not done: drawn there a moment before it sinks.
                    let slot = list.items()[..n].iter().filter(|each| !each.done).count();
                    crate::todo::tick(n, &item, true);
                    lane::ticked(key, slot);
                }
            }
            Act::Nothing
        }
        TodoHit::Remove(n, key) => {
            if let Some(item) = item_at(&list, n as usize, key) {
                crate::todo::change_item(n as usize, &item, |list, n| list.remove(n));
            }
            Act::Nothing
        }
        TodoHit::Due(n, key) => item_at(&list, n as usize, key).map_or(Act::Nothing, |item| Act::Pick(PickFor::Item(n as usize, item))),
        TodoHit::Clear => {
            crate::todo::change(List::clear_done);
            Act::Nothing
        }
        TodoHit::Done => {
            lane::toggle_open();
            Act::Nothing
        }
        TodoHit::Field | TodoHit::Add => Act::Write,
        TodoHit::FieldDue => Act::Pick(PickFor::Field),
        TodoHit::Words(n, key) => item_at(&list, n as usize, key).filter(|item| !item.done).map_or(Act::Nothing, |item| Act::Edit(n as usize, item)),
        TodoHit::Row(..) => Act::Nothing,
    }
}

/// The field being written: in which window, whether that window takes the keys for it now, what is written, the time
/// set for it, since when the caret blinks, and where the field was last drawn on the screen (physical px, and the
/// screen's scale: see `placed`).
struct Writing {
    owner: isize,
    taking: bool,
    typing: Typing,
    due: Option<Due>,
    blink_from: Instant,
    at: Option<(RECT, f32)>,
    /// The window in front before its window took the keys, given the front back as they are given back by Esc.
    before: isize,
    /// Whether the input method is putting words together for it (between its composition's start and end): words on
    /// their way into the field, not in its text yet.
    composing: bool,
    /// The item written in where it shows, in the field's stead (what is written in the field kept meanwhile).
    edit: Option<Edit>,
}

/// An item written in where it shows: the `n`th, keyed `key` (see `key_of`), as it was, and what is written for it.
struct Edit {
    n: usize,
    key: u64,
    item: Item,
    typing: Typing,
}

impl Writing {
    /// What the keys write in: the item written in where it shows, else the field.
    fn typing(&mut self) -> &mut Typing {
        match &mut self.edit {
            Some(edit) => &mut edit.typing,
            None => &mut self.typing,
        }
    }
}

thread_local! {
    static WRITING: RefCell<Option<Writing>> = const { RefCell::new(None) };
}

/// The field written in in the window `owner` (its keys taken for it: see `ui::keys`), what was written in it before
/// there kept (written in another window, carried there); what answers its keys.
pub fn write_in(owner: HWND) -> super::keys::Answer {
    let mut carried = false;
    WRITING.with_borrow_mut(|writing| {
        let now = Instant::now();
        match writing {
            Some(writing) => {
                // (Carried from another window: where it was drawn there is no longer where it is. Words the input
                // method was putting together, there or here before the keys were given back, are over: their end is
                // heard by the window they were put together for, if at all.)
                if writing.owner != owner.0 as isize {
                    writing.at = None;
                    carried = writing.edit.is_some();
                }
                (writing.owner, writing.taking, writing.blink_from, writing.composing) = (owner.0 as isize, true, now, false);
            }
            None => *writing = Some(Writing { owner: owner.0 as isize, taking: true, typing: Typing::most(MOST), due: None, blink_from: now, at: None, before: 0, composing: false, edit: None }),
        }
    });
    // (An item written in where it shows in the window it left: kept, written in there no more.)
    if carried {
        keep_edit();
    }
    Rc::new(answer)
}

/// The field written in in `window` (see `write_in`): it comes to the front with the keys, the caret shown at once. An
/// item written in where it shows kept first.
pub fn begin(window: &super::window::Window) {
    keep_edit();
    take(window);
}

/// `window` taking the keys for what is written on its lane (unless it does), the caret shown at once.
fn take(window: &super::window::Window) {
    let owner = window.hwnd;
    if writing(owner) {
        WRITING.with_borrow_mut(|writing| {
            if let Some(writing) = writing.as_mut() {
                writing.blink_from = Instant::now();
            }
        });
        return;
    }
    let before = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    super::keys::take(owner, write_in(owner));
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut() {
            writing.before = if before == owner { 0 } else { before.0 as isize };
        }
    });
    window.take_keys(true);
}

/// The `n`th item, as `item`, written in where it shows in `window`, its caret `caret` into its words: what is written
/// in another item, or in this one in another window, kept first; what is written in this one there kept, its caret
/// moved.
pub fn edit(window: &super::window::Window, n: usize, item: Item, caret: usize) {
    let owner = window.hwnd.0 as isize;
    let same = WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.owner == owner && writing.edit.as_ref().is_some_and(|edit| edit.n == n && edit.item == item)));
    if !same {
        keep_edit();
    }
    take(window);
    let key = key_of(&crate::todo::list().items(), n);
    WRITING.with_borrow_mut(|writing| {
        let Some(writing) = writing.as_mut() else { return };
        let edit = writing.edit.get_or_insert_with(|| {
            let mut typing = Typing::most(MOST);
            typing.put(&item.text.encode_utf16().collect::<Vec<_>>());
            Edit { n, key, item, typing }
        });
        edit.typing.go(caret, false);
        writing.blink_from = Instant::now();
    });
}

/// The item written in where it shows, where one is: what is written there kept (see `edited`), and written in no more.
fn keep_edit() {
    let Some(edit) = WRITING.with_borrow_mut(|writing| writing.as_mut().and_then(|writing| writing.edit.take())) else { return };
    if let Some(item) = edited(&edit.item, &edit.typing.string(), Day::today()) {
        crate::todo::change_item(edit.n, &edit.item, |list, n| {
            list.set_text(n, item.text);
            list.set_due(n, item.due);
        });
    }
}

/// `item` as it is kept once `written` is written for it on `today`: its words those written, a time they name its time
/// (the words naming it left out: see `todo::words`), else its time as it was. Nothing to change where nothing is
/// written (it is taken out by its button, not by its words cleared), or the same as it was.
fn edited(item: &Item, written: &str, today: Day) -> Option<Item> {
    let written = written.trim();
    if written.is_empty() {
        return None;
    }
    let (words, named) = crate::todo::words(written, today);
    let kept = Item { text: words, due: named.or(item.due), ..item.clone() };
    (kept != *item).then_some(kept)
}

/// Whether a press on `hit` keeps what is written on the lane being written in: the field, its time's button and the
/// head's plus, where the field is written in; the words of the item written in where it shows, where one is. (A press
/// anywhere else gives the keys back: see `end`.)
pub fn keeps(hit: TodoHit) -> bool {
    WRITING.with_borrow(|writing| match writing.as_ref().and_then(|writing| writing.edit.as_ref()) {
        Some(edit) => hit == TodoHit::Words(edit.n as u32, edit.key),
        None => matches!(hit, TodoHit::Field | TodoHit::Add | TodoHit::FieldDue),
    })
}

/// Where on the lane of the window `owner` the caret is drawn: in the words of the item written in where it shows, else
/// in the field (the input method's words put there: see `placed`).
pub fn caret_in(owner: HWND) -> TodoHit {
    WRITING.with_borrow(|writing| match writing.as_ref().filter(|writing| writing.owner == owner.0 as isize).and_then(|writing| writing.edit.as_ref()) {
        Some(edit) => TodoHit::Words(edit.n as u32, edit.key),
        None => TodoHit::Field,
    })
}

/// Where in the words of the `n`th item, as `item`, a press `into` DIPs across them puts the caret, laid out by `gfx` in
/// `font` as its row draws them: from where they are scrolled to, in the one written in where it shows.
pub fn caret_at(gfx: &super::gfx::Gfx, font: super::canvas::Font, n: usize, item: &Item, into: f32) -> usize {
    let (text, shift) = WRITING.with_borrow(|writing| match writing.as_ref().and_then(|writing| writing.edit.as_ref()).filter(|edit| edit.n == n && edit.item == *item) {
        Some(edit) => (edit.typing.string(), lane::edit_shift()),
        None => (item.text.clone(), 0.0),
    });
    super::typing::at_point(&gfx.layout(&text, font, 10_000.0, super::canvas::Align::Start), (into + shift).max(0.0), font.size / 2.0)
}

/// The field written in in `window` given back its keys (Esc, another window in front, a press away from it, its
/// window going): what was written stays; the front given back to the window that had it where `escaped`. What is
/// written in an item where it shows is kept (see `keep_edit`), Esc leaving it as it was.
pub fn end(window: &super::window::Window, escaped: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    let owner = window.hwnd;
    if !writing(owner) {
        return;
    }
    if escaped {
        WRITING.with_borrow_mut(|writing| {
            if let Some(writing) = writing.as_mut() {
                writing.edit = None;
            }
        });
    }
    super::keys::give_back(owner);
    stop(owner);
    window.take_keys(false);
    let before = WRITING.with_borrow(|writing| writing.as_ref().map_or(0, |writing| writing.before));
    let before = HWND(before as *mut _);
    if escaped && !before.is_invalid() && unsafe { IsWindow(Some(before)) }.as_bool() {
        let _ = unsafe { SetForegroundWindow(before) };
    }
}

/// The window `owner` gives back the keys its field took: what was written stays, what was written in an item where it
/// shows kept (see `keep_edit`).
pub fn stop(owner: HWND) {
    let stopped = WRITING.with_borrow_mut(|writing| {
        let Some(writing) = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize) else { return false };
        writing.taking = false;
        true
    });
    if stopped {
        keep_edit();
    }
}

/// The window `owner` gone: its field written in no more (what was written stays), the picker opened from it put away.
pub fn went(owner: HWND) {
    stop(owner);
    if pick::up_for(owner) {
        pick::close(false);
    }
}

/// Whether the window `owner` takes the keys for its field now.
pub fn writing(owner: HWND) -> bool {
    WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.taking && writing.owner == owner.0 as isize))
}

/// Whether the window `owner` is to stay as it is for the to-dos: its field written in and holding what is not kept yet
/// (words, a time set, or words the input method is putting together), or the picker opened from it up. A field taking
/// the keys with nothing in it holds nothing: once Enter has added what was written, the panel goes as the pointer
/// leaves it, as it would have before the field was pressed (going, it gives the keys back: see `went`).
pub fn holds(owner: HWND) -> bool {
    pick::up_for(owner) || WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.owner == owner.0 as isize && unkept(writing)))
}

/// Whether the field takes the keys and holds what would be lost if it went now (an item written in where it shows
/// among that).
fn unkept(writing: &Writing) -> bool {
    writing.taking && (!writing.typing.text.is_empty() || writing.due.is_some() || writing.composing || writing.edit.is_some())
}

/// The input method begins (`true`) or ends putting words together for the field.
fn composing(now: bool) {
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut() {
            writing.composing = now;
        }
    });
}

/// The field as the window `owner` shows it at `now`, and when it is next to be drawn for its caret: what is written
/// there, where it was written there (else as empty).
pub fn field(owner: HWND, now: Instant) -> (Field, Option<Instant>) {
    WRITING.with_borrow(|writing| {
        let Some(writing) = writing.as_ref().filter(|writing| writing.owner == owner.0 as isize) else { return (Field::default(), None) };
        let since = now.duration_since(writing.blink_from);
        let blinks = (since.as_millis() / BLINK.as_millis()) as u32;
        let next = writing.taking.then(|| writing.blink_from + BLINK * (blinks + 1));
        let today = Day::today();
        let caret_on = writing.taking && blinks.is_multiple_of(2);
        let edit = writing.edit.as_ref().map(|edit| lane::Editing { n: edit.n as u32, key: edit.key, text: edit.typing.text.clone(), caret: edit.typing.caret, anchor: edit.typing.anchor, caret_on });
        let field = Field {
            writing: writing.taking && edit.is_none(),
            text: writing.typing.text.clone(),
            caret: writing.typing.caret,
            anchor: writing.typing.anchor,
            caret_on: caret_on && edit.is_none(),
            due: writing.due.map(|due| due.label(today, crate::update::language() == super::text::Lang::Zh)),
            edit,
        };
        (field, next)
    })
}

/// Where the field written in the window `owner` was just drawn on the screen (physical px), at `scale`: the input
/// method's words put at its caret.
pub fn placed(owner: HWND, field: RECT, scale: f32) {
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize) {
            writing.at = Some((field, scale));
        }
    });
}

/// The time set for the item being written (from the picker).
pub fn set_field_due(due: Option<Due>) {
    WRITING.with_borrow_mut(|writing| {
        let writing = writing.get_or_insert_with(|| Writing { owner: 0, taking: false, typing: Typing::most(MOST), due: None, blink_from: Instant::now(), at: None, before: 0, composing: false, edit: None });
        writing.due = due;
    });
}

/// The time set for the item being written.
pub fn field_due() -> Option<Due> {
    WRITING.with_borrow(|writing| writing.as_ref().and_then(|writing| writing.due))
}

/// A message for a window whose field takes its keys (see `ui::keys::Answer`): its keys and the characters they make,
/// the input method's words put at its caret. (Esc is not its: the keys are given back.)
fn answer(hwnd: HWND, message: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<Option<LRESULT>> {
    match message {
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            if key == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE {
                return None;
            }
            key_down(hwnd, key);
            Some(Some(LRESULT(0)))
        }
        WM_CHAR => {
            typed(wparam.0 as u16);
            Some(Some(LRESULT(0)))
        }
        WM_KEYUP => Some(Some(LRESULT(0))),
        WM_IME_STARTCOMPOSITION => {
            composing(true);
            place_ime(hwnd);
            Some(None)
        }
        WM_IME_ENDCOMPOSITION => {
            composing(false);
            Some(None)
        }
        _ => None,
    }
}

/// A key down in the field (or the item written in where it shows): the caret moved (with Shift, the selection), words
/// erased, all chosen, pasted; Enter adds what is written (keeps what is written for the item, its window giving the
/// keys back: see `keys::LEFT`).
fn key_down(hwnd: HWND, key: VIRTUAL_KEY) {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    let (ctrl, shift) = (down(VK_CONTROL), down(VK_SHIFT));
    let pasted = (ctrl && key.0 == u16::from(b'V')).then(|| crate::clipboard::text(hwnd).unwrap_or_default());
    if key == VK_RETURN && WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.edit.is_some())) {
        keep_edit();
        let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(hwnd), super::keys::LEFT, WPARAM(0), LPARAM(0)) };
        return;
    }
    WRITING.with_borrow_mut(|writing| {
        let Some(writing) = writing.as_mut() else { return };
        let typing = writing.typing();
        match key {
            VK_LEFT => typing.go(if typing.caret != typing.anchor && !shift { typing.selection().start } else { typing.step(typing.caret, false) }, shift),
            VK_RIGHT => typing.go(if typing.caret != typing.anchor && !shift { typing.selection().end } else { typing.step(typing.caret, true) }, shift),
            VK_HOME => typing.go(0, shift),
            VK_END => typing.go(typing.text.len(), shift),
            VK_BACK => typing.erase(false),
            VK_DELETE => typing.erase(true),
            VK_RETURN => add(writing),
            _ if ctrl && key.0 == u16::from(b'A') => {
                typing.go(0, false);
                typing.go(typing.text.len(), true);
            }
            // (One line of what was pasted: an item is a line.)
            _ if pasted.is_some() => {
                let line: Vec<u16> = pasted.as_deref().unwrap_or_default().lines().next().unwrap_or_default().encode_utf16().collect();
                typing.put(&line);
            }
            _ => return,
        }
        writing.blink_from = Instant::now();
    });
}

/// A character typed in the field (control characters are keys: see `key_down`).
fn typed(unit: u16) {
    if unit < 0x20 || unit == 0x7F {
        return;
    }
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut() {
            writing.typing().put(&[unit]);
            writing.blink_from = Instant::now();
        }
    });
}

/// What is written in the field added as an item, first among those not done (it drops there from the field), its time
/// the one set for it, else the one its words name; the field emptied.
fn add(writing: &mut Writing) {
    let Some(words) = crate::todo::add(&writing.typing.string(), writing.due) else { return };
    // (It drops into the list from the field: the first of its words among those not done, as it now is.)
    let list = crate::todo::list();
    let items = list.items();
    if let Some(n) = items.iter().position(|item| !item.done && item.text == words) {
        lane::added(key_of(&items, n));
    }
    (writing.typing, writing.due) = (Typing::most(MOST), None);
}

/// The input method's words and choices put at the field's caret in `hwnd`, where it was last drawn, as its words.
fn place_ime(hwnd: HWND) {
    let Some((field, scale)) = WRITING.with_borrow(|writing| writing.as_ref().and_then(|writing| writing.at)) else { return };
    let (x, y, tall, size) = lane::caret();
    let mut corner = POINT { x: field.left, y: field.top };
    let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut corner) };
    let px = |dips: f32| (dips * scale).round() as i32;
    super::typing::place_ime(hwnd, POINT { x: corner.x + px(x), y: corner.y + px(y) }, px(y + tall) - px(y), "Microsoft YaHei UI", px(size));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field holds the panel only while it holds what is not kept: pressed and empty, or emptied by Enter, it holds
    /// nothing (the panel went only after a press elsewhere); words written, a time set, or words the input method is
    /// putting together, it holds; given back its keys, it holds nothing.
    #[test]
    fn the_field_holds_the_panel_only_while_it_holds_what_is_not_kept() {
        let owner = HWND(0x5151 as *mut _);
        WRITING.with_borrow_mut(|writing| *writing = None);
        let _ = write_in(owner);
        assert!(writing(owner));
        assert!(!holds(owner), "a field pressed with nothing in it");
        WRITING.with_borrow_mut(|writing| writing.as_mut().unwrap().typing.put(&"买猫粮".encode_utf16().collect::<Vec<_>>()));
        assert!(holds(owner), "words written");
        WRITING.with_borrow_mut(|writing| writing.as_mut().unwrap().typing = Typing::most(MOST));
        assert!(!holds(owner), "emptied, as Enter leaves it");
        composing(true);
        assert!(holds(owner), "words the input method is putting together");
        composing(false);
        assert!(!holds(owner));
        WRITING.with_borrow_mut(|writing| writing.as_mut().unwrap().typing.put(&"预约体检".encode_utf16().collect::<Vec<_>>()));
        stop(owner);
        assert!(!holds(owner), "its keys given back");
        assert!(!holds(HWND(0x5252 as *mut _)), "another window's");
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// What an item written in where it shows is kept as: the words written, a time they name its new time (the words
    /// naming it left out), else its time as it was. Words naming a time and nothing else are its words, its time as
    /// it was. Nothing written, or the same as it was, changes nothing (it is not taken out by its words cleared).
    #[test]
    fn an_item_written_in_where_it_shows_keeps_its_words_and_a_time_they_name() {
        let today = Day::of(2026, 11, 21).unwrap();
        let due = |day: Day, time: Option<(u32, u32)>| Some(Due { day, time });
        let was = Item { text: "回复客户的邮件".into(), done: false, due: due(today, Some((18, 0))) };
        let (words, named) = crate::todo::words("回复客户的邮件 明天 15:00", today);
        assert_eq!(edited(&was, "回复客户的邮件 明天 15:00", today), Some(Item { text: words, due: named, ..was.clone() }));
        assert_eq!(named.map(|due| due.time), Some(Some((15, 0))), "the time named");
        assert_eq!(edited(&was, "回复王总的邮件", today), Some(Item { text: "回复王总的邮件".into(), ..was.clone() }), "no time named: its own kept");
        assert_eq!(edited(&was, "  回复王总的邮件  ", today).map(|item| item.text), Some("回复王总的邮件".into()));
        assert_eq!(edited(&was, "明天", today), Some(Item { text: "明天".into(), ..was.clone() }), "a time alone: its words, its time kept");
        assert_eq!(edited(&was, "", today), None, "nothing written");
        assert_eq!(edited(&was, "   ", today), None);
        assert_eq!(edited(&was, "回复客户的邮件", today), None, "as it was");
    }

    /// A press on an item's words puts the caret where it was pressed: before its first character at its left, after
    /// a character pressed past its middle, after them all past their end. In the item written in where it shows, it is
    /// the words written there that are pressed (not those it had).
    #[test]
    fn a_press_on_an_item_s_words_puts_the_caret_where_it_was_pressed() {
        let _device = crate::ui::gfx::held().unwrap();
        let gfx = crate::ui::gfx::Gfx::new().expect("graphics");
        let font = crate::ui::theme::Theme::new(crate::ui::theme::Skin::Glass, true).body;
        let item = Item { text: "交房租".into(), done: false, due: None };
        let wide = |text: &str| gfx.measure(text, font);
        WRITING.with_borrow_mut(|writing| *writing = None);
        assert_eq!(caret_at(&gfx, font, 0, &item, 0.0), 0);
        assert_eq!(caret_at(&gfx, font, 0, &item, wide("交") * 0.3), 0);
        assert_eq!(caret_at(&gfx, font, 0, &item, wide("交") * 0.7), 1);
        assert_eq!(caret_at(&gfx, font, 0, &item, wide("交房") + wide("租") * 0.6), 3);
        assert_eq!(caret_at(&gfx, font, 0, &item, 400.0), 3);
        // Written in where it shows: its words as written.
        let _ = write_in(HWND(0x7373 as *mut _));
        WRITING.with_borrow_mut(|writing| {
            let mut typing = Typing::most(MOST);
            typing.put(&"交房租和水电费".encode_utf16().collect::<Vec<_>>());
            writing.as_mut().unwrap().edit = Some(Edit { n: 0, key: 1, item: item.clone(), typing });
        });
        assert_eq!(caret_at(&gfx, font, 0, &item, 400.0), 7);
        assert_eq!(caret_at(&gfx, font, 1, &Item { text: "别的".into(), ..item.clone() }, 400.0), 2, "another item: its own words");
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// An item written in where it shows takes what is typed in the field's stead (what is written in the field kept
    /// meanwhile), holds the panel, is drawn where it shows (the field not written in), and keeps the keys only for a
    /// press on its own words: one on the field, or anywhere else, gives them back (and keeps it). Given back, it is
    /// written in no more and the field's words are as they were.
    #[test]
    fn an_item_written_in_where_it_shows_takes_the_keys_from_the_field() {
        let owner = HWND(0x7171 as *mut _);
        WRITING.with_borrow_mut(|writing| *writing = None);
        let _ = write_in(owner);
        typed(u16::from(b'a'));
        assert!(keeps(TodoHit::Field) && keeps(TodoHit::Add) && keeps(TodoHit::FieldDue));
        assert_eq!(caret_in(owner), TodoHit::Field);
        let item = Item { text: "交房租".into(), done: false, due: None };
        WRITING.with_borrow_mut(|writing| {
            let mut typing = Typing::most(MOST);
            typing.put(&item.text.encode_utf16().collect::<Vec<_>>());
            typing.go(1, false);
            writing.as_mut().unwrap().edit = Some(Edit { n: 2, key: 77, item: item.clone(), typing });
        });
        typed(u16::from(b'x'));
        let (shown, _) = field(owner, Instant::now());
        let edit = shown.edit.clone().expect("drawn where it shows");
        assert_eq!((edit.n, edit.key, String::from_utf16_lossy(&edit.text), edit.caret), (2, 77, "交x房租".to_string(), 2));
        assert_eq!(String::from_utf16_lossy(&shown.text), "a", "the field's words kept");
        assert!(!shown.writing && !shown.caret_on, "the field not written in meanwhile");
        assert!(holds(owner));
        assert_eq!(caret_in(owner), TodoHit::Words(2, 77));
        assert_eq!(caret_in(HWND(0x7272 as *mut _)), TodoHit::Field, "another window's");
        assert!(keeps(TodoHit::Words(2, 77)));
        for away in [TodoHit::Field, TodoHit::Add, TodoHit::Words(3, 78), TodoHit::Row(2, 77), TodoHit::Tick(2, 77), TodoHit::Due(2, 77)] {
            assert!(!keeps(away), "{away:?}");
        }
        stop(owner);
        WRITING.with_borrow(|writing| assert!(writing.as_ref().unwrap().edit.is_none(), "written in no more"));
        let (shown, _) = field(owner, Instant::now());
        assert_eq!(String::from_utf16_lossy(&shown.text), "a");
        assert!(shown.edit.is_none());
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// Carried to another window, the field keeps its words but not where it was drawn in the window it left (the
    /// input method's words are put at its caret where it is next drawn), nor words the input method was putting
    /// together there (that window hears their end, not this one). Taken up again in the same window after its keys
    /// were given back, no words are being put together either.
    #[test]
    fn the_field_carried_to_another_window_leaves_its_place_and_composition_behind() {
        let (a, b) = (HWND(0x6161 as *mut _), HWND(0x6262 as *mut _));
        WRITING.with_borrow_mut(|writing| *writing = None);
        let _ = write_in(a);
        placed(a, RECT { left: 10, top: 20, right: 300, bottom: 52 }, 1.5);
        WRITING.with_borrow_mut(|writing| writing.as_mut().unwrap().typing.put(&"交房租".encode_utf16().collect::<Vec<_>>()));
        composing(true);
        let _ = write_in(b);
        WRITING.with_borrow(|writing| {
            let writing = writing.as_ref().unwrap();
            assert_eq!(writing.typing.string(), "交房租", "its words carried");
            assert!(writing.at.is_none(), "where it was drawn in the window it left");
            assert!(!writing.composing, "words being put together in the window it left");
        });
        WRITING.with_borrow_mut(|writing| writing.as_mut().unwrap().typing = Typing::most(MOST));
        assert!(!holds(b), "empty in its new window, it holds nothing");
        composing(true);
        stop(b);
        let _ = write_in(b);
        assert!(!holds(b), "taken up again with no words being put together");
        WRITING.with_borrow_mut(|writing| *writing = None);
    }
}
