//! What a press on the to-dos' lane does (see `view::todo`), in whichever window shows it (the panel's, a widget's); the
//! field an item is written in there, its window taking the keys meanwhile (see `ui::keys`); and the picker of an item's
//! day and time (see `pick`).
//!
//! What is written in the field stays as it was written while its window gives the keys back (another window came in
//! front, the picker opened from it): written in again, it goes on from there. Enter adds it as an item, its time the
//! one set for it, else one its words name (see `todo::words`), and the field is empty for the next.

pub mod pick;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{WM_APP, WM_CHAR, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP};

use super::typing::Typing;
use super::view::todo::{self as lane, key_of, Field, TodoHit};
use crate::todo::{Day, Due, Item, List};

/// Posted to the window an item's field was written in as the picker opened from it goes: it is written in again.
pub const RESUME: u32 = WM_APP + 41;

/// How long the caret shows, and is hidden, as it blinks.
const BLINK: Duration = Duration::from_millis(530);

/// The most an item's words hold (UTF-16 units).
const MOST: usize = 500;

/// What a press on the lane asks of the window it is in, besides what it did itself.
pub enum Act {
    Nothing,
    /// The field written in: the window is to take the keys (see `write_in`).
    Write,
    /// The picker of a time opened, from where on the lane it was pressed (see `pick::open`).
    Pick(PickFor),
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
}

thread_local! {
    static WRITING: RefCell<Option<Writing>> = const { RefCell::new(None) };
}

/// The field written in in the window `owner` (its keys taken for it: see `ui::keys`), what was written in it before
/// there kept (written in another window, carried there); what answers its keys.
pub fn write_in(owner: HWND) -> super::keys::Answer {
    WRITING.with_borrow_mut(|writing| {
        let now = Instant::now();
        match writing {
            Some(writing) => {
                (writing.owner, writing.taking, writing.blink_from) = (owner.0 as isize, true, now);
                if writing.owner != owner.0 as isize {
                    writing.at = None;
                }
            }
            None => *writing = Some(Writing { owner: owner.0 as isize, taking: true, typing: Typing::most(MOST), due: None, blink_from: now, at: None, before: 0 }),
        }
    });
    Rc::new(answer)
}

/// The field written in in `window` (see `write_in`): it comes to the front with the keys, the caret shown at once.
pub fn begin(window: &super::window::Window) {
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

/// The field written in in `window` given back its keys (Esc, another window in front, a press away from it, its
/// window going): what was written stays; the front given back to the window that had it where `escaped`.
pub fn end(window: &super::window::Window, escaped: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    let owner = window.hwnd;
    if !writing(owner) {
        return;
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

/// The window `owner` gives back the keys its field took: what was written stays.
pub fn stop(owner: HWND) {
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize) {
            writing.taking = false;
        }
    });
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

/// Whether the window `owner` is to stay as it is for the to-dos: its field written in, or the picker opened from it up.
pub fn holds(owner: HWND) -> bool {
    writing(owner) || pick::up_for(owner)
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
        let zh = crate::update::language() == super::text::Lang::Zh;
        let field = Field {
            writing: writing.taking,
            text: writing.typing.text.clone(),
            caret: writing.typing.caret,
            anchor: writing.typing.anchor,
            caret_on: writing.taking && blinks.is_multiple_of(2),
            due: writing.due.map(|due| due.label(today, zh)),
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
        let writing = writing.get_or_insert_with(|| Writing { owner: 0, taking: false, typing: Typing::most(MOST), due: None, blink_from: Instant::now(), at: None, before: 0 });
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
            place_ime(hwnd);
            Some(None)
        }
        _ => None,
    }
}

/// A key down in the field: the caret moved (with Shift, the selection), words erased, all chosen, pasted; Enter adds
/// what is written.
fn key_down(hwnd: HWND, key: VIRTUAL_KEY) {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
    let (ctrl, shift) = (down(VK_CONTROL), down(VK_SHIFT));
    let pasted = (ctrl && key.0 == u16::from(b'V')).then(|| crate::clipboard::text(hwnd).unwrap_or_default());
    WRITING.with_borrow_mut(|writing| {
        let Some(writing) = writing.as_mut() else { return };
        let typing = &mut writing.typing;
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
            writing.typing.put(&[unit]);
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
