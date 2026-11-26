//! The box a name is written in where it does not show: a thing drawn by its picture alone (what is kept at hand on a
//! rail of pictures, the station's things or quick launch's in a workbench grid with no names under them nor at its
//! foot), renamed from its menu (its name written where it shows elsewhere: see `naming`). A plate in the look of what
//! it is on, beside its picture, holding the field the name is written in, written as one where it shows is (see
//! `naming`: its own keys, Enter, Esc, a file's name tried and why it was refused said under it). Enter, a press away
//! from it or another window in front keeps what is written, Esc leaves it as it was. Run by the loop of the thread
//! that opened it, as a menu is (see `menu::Menu`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::SetCapture;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow, MSG, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP};

use crate::ui::canvas::Font;
use crate::ui::gfx;
use crate::ui::menu::{self, Look};
use crate::ui::naming::{self, Naming};
use crate::ui::theme::{Skin, Theme};
use crate::ui::window::{Window, CAPTURE_LOST};

/// Its measures (DIPs): its field, as far in from the plate's edge all round as it is round, and the gap between it and
/// the picture it is beside.
const PAD: f32 = 6.0;
const FIELD_W: f32 = 150.0;
const FIELD_H: f32 = 28.0;
const BESIDE: f32 = 6.0;
/// The words' inset in the field.
const INSET: f32 = 8.0;
/// The line under the field why what was written was refused is said on.
const SAID_H: f32 = 18.0;

/// What a message to it came to.
pub enum Handled {
    /// None of its own.
    Not,
    /// Taken, it still up.
    Taken,
    /// Its keys left as a press away from it leaves them (what is written kept), the mouse taken from it.
    Left,
}

/// The box up.
struct Renaming {
    gfx: Rc<gfx::Gfx>,
    window: Window,
    /// What its window shows, as a menu's, taken out while it is drawn on.
    surface: Option<menu::Drawn>,
    room: f32,
    words: gfx::Layer,
    drawn: u64,
    theme: Theme,
    look: Look,
    scale: f32,
    /// Where it is (physical px), and what it is placed by: the picture it is beside, the room on its screen.
    rect: RECT,
    picture: RECT,
    work: RECT,
    /// The window its menu was opened from (the panel's, a widget's).
    owner: isize,
    /// Whether it says why what was written was refused (a line taller).
    says: bool,
    /// When its caret blinks next.
    due: Option<Instant>,
    before: HWND,
    _showing: crate::ui::life::Showing,
}

thread_local! {
    static RENAME: RefCell<Option<Renaming>> = const { RefCell::new(None) };
}

/// The box opened from the window `owner` (its menu) for `naming`, beside `picture` (on the screen: what it names,
/// drawn there by its picture alone), in `look`: any up before put away first (what was written in it kept). Whether
/// it is up.
pub fn open(owner: HWND, naming: Naming, picture: RECT, look: Look) -> bool {
    close(false);
    let Some(renaming) = Renaming::new(owner, picture, look) else { return false };
    let before = unsafe { GetForegroundWindow() };
    // (Written in it from now: its keys its own, as one written where it shows.)
    naming::begin(&renaming.window, naming);
    let hwnd = renaming.window.hwnd;
    RENAME.with_borrow_mut(|rename| *rename = Some(renaming));
    if !draw() {
        close(true);
        return false;
    }
    RENAME.with_borrow(|rename| {
        if let Some(renaming) = rename.as_ref() {
            renaming.window.show();
            crate::widget::own_window_changed(renaming.rect);
        }
    });
    // In front, with the keyboard and the mouse, as a menu is.
    if !unsafe { SetForegroundWindow(hwnd) }.as_bool() {
        crate::journal::note("rename: not let to the front; not shown");
        close(true);
        return false;
    }
    unsafe { SetCapture(hwnd) };
    RENAME.with_borrow_mut(|rename| {
        if let Some(renaming) = rename.as_mut() {
            renaming.before = before;
        }
    });
    true
}

/// Whether it is up.
pub fn is_up() -> bool {
    RENAME.with_borrow(Option::is_some)
}

/// Whether it is up, opened from the menu of the window `owner`.
pub fn up_for(owner: HWND) -> bool {
    RENAME.with_borrow(|rename| rename.as_ref().is_some_and(|renaming| renaming.owner == owner.0 as isize))
}

/// Whether `hwnd` is its window.
pub fn owns(hwnd: HWND) -> bool {
    RENAME.with_borrow(|rename| rename.as_ref().is_some_and(|renaming| renaming.window.hwnd == hwnd))
}

/// Whether `keys::LEFT`, posted to its window with `wparam`, was Esc (what is written left as it was), else Enter or
/// another window in front (kept).
pub fn escaped(wparam: WPARAM) -> bool {
    wparam.0 == 1
}

/// A message for its window but its keys' (the pointer's).
pub fn handle(msg: &MSG) -> Handled {
    let Some((hwnd, scale, room, says)) = RENAME.with_borrow(|rename| rename.as_ref().filter(|renaming| renaming.window.hwnd == msg.hwnd).map(|renaming| (renaming.window.hwnd, renaming.scale, renaming.room, renaming.says))) else { return Handled::Not };
    let (x, y) = ((msg.lParam.0 & 0xFFFF) as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32);
    let (x, y) = (x as f32 / scale - room, y as f32 / scale - room);
    let on_plate = (0.0..=WIDTH).contains(&x) && (0.0..=height(says)).contains(&y);
    match msg.message {
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !on_plate => Handled::Left,
        // A press in its field: the caret there (with Shift, the selection to there), its keys taken again where they
        // were left (what was written refused as it was tried so).
        WM_LBUTTONDOWN if (PAD..=PAD + FIELD_W).contains(&x) && (PAD..=PAD + FIELD_H).contains(&y) => {
            let pressed = RENAME.with_borrow(|rename| rename.as_ref().map(|renaming| naming::press(&renaming.window, &renaming.gfx, x - PAD)));
            if pressed.is_some() && naming::holds(hwnd) {
                draw();
            }
            Handled::Taken
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN => Handled::Taken,
        CAPTURE_LOST => Handled::Left,
        _ => Handled::Not,
    }
}

/// Its keys left (see `naming::end`): Esc where `escaped` (what is written left as it was), else what is written kept
/// (a file's name tried, it up till that is answered). Whether it is put away.
pub fn left(escaped: bool) -> bool {
    let Some(hwnd) = RENAME.with_borrow(|rename| rename.as_ref().map(|renaming| renaming.window.hwnd)) else { return false };
    RENAME.with_borrow(|rename| {
        if let Some(renaming) = rename.as_ref() {
            naming::end(&renaming.window, escaped);
        }
    });
    if naming::holds(hwnd) {
        draw();
        return false;
    }
    close(false);
    true
}

/// What was written in it, tried, answered (see `naming::ANSWERED`): done, it put away; refused, why said under it.
/// Whether it is put away.
pub fn answered() -> bool {
    let Some(hwnd) = RENAME.with_borrow(|rename| rename.as_ref().map(|renaming| renaming.window.hwnd)) else { return false };
    RENAME.with_borrow(|rename| {
        if let Some(renaming) = rename.as_ref() {
            naming::answered(&renaming.window);
        }
    });
    if naming::holds(hwnd) {
        draw();
        return false;
    }
    close(false);
    true
}

/// What its keys wrote in it (see `ui::keys::took`), drawn, where `hwnd` is its window.
pub fn written(hwnd: HWND) {
    if owns(hwnd) {
        draw();
    }
}

/// Its caret blinked as it is due by `now`: when it is due next (none where it is not up, or its caret does not blink).
pub fn tick(now: Instant) -> Option<std::time::Duration> {
    let due = RENAME.with_borrow(|rename| rename.as_ref().and_then(|renaming| renaming.due))?;
    if due <= now {
        draw();
    }
    let due = RENAME.with_borrow(|rename| rename.as_ref().and_then(|renaming| renaming.due))?;
    Some(due.saturating_duration_since(now))
}

/// Put away, if it is up, what is written in it kept where `keep` (else left as it was): the front given back to what
/// had it, where it still has it.
pub fn close(keep: bool) {
    let Some(renaming) = RENAME.with_borrow_mut(Option::take) else { return };
    let hwnd = renaming.window.hwnd;
    let held = unsafe { GetForegroundWindow() } == hwnd;
    if naming::holds(hwnd) {
        naming::end(&renaming.window, !keep);
    }
    // (Its window gone, what is written in it is seen to as one written where it shows is: see `naming::went`.)
    crate::ui::window::let_go_of_capture(hwnd);
    let rect = renaming.rect;
    let before = renaming.before;
    renaming.window.destroy();
    crate::widget::own_window_changed(rect);
    if held && !before.is_invalid() {
        let _ = unsafe { SetForegroundWindow(before) };
    }
}

/// It drawn as what is written in it is now (placed again where it says why that was refused, or no longer does).
/// Whether it was.
fn draw() -> bool {
    RENAME.with_borrow_mut(|rename| rename.as_mut().is_some_and(Renaming::draw))
}

/// Its plate's height (DIPs): a line taller where it `says` why what was written was refused.
fn height(says: bool) -> f32 {
    2.0 * PAD + FIELD_H + if says { SAID_H } else { 0.0 }
}

impl Renaming {
    fn new(owner: HWND, picture: RECT, look: Look) -> Option<Renaming> {
        let middle = POINT { x: (picture.left + picture.right) / 2, y: (picture.top + picture.bottom) / 2 };
        let contact = crate::mouse::monitor_at(middle)?;
        // (On the device for its screen: see `gfx::on_screen`.)
        let gfx = gfx::for_area(contact.monitor).ok()?;
        let showing = crate::ui::life::Showing::of(&gfx);
        let window = Window::activating().ok()?;
        // (As frosted as glass is at its most: it lies over the glass its menu was opened from, not the desktop.)
        let look = Look { frost: 1.0, ..look };
        let surface = match menu::Drawn::new(&gfx, window.hwnd, look) {
            Ok(surface) => surface,
            Err(error) => {
                window.destroy();
                gfx::failed("rename", &gfx, Some(&error));
                return None;
            }
        };
        let theme = Theme::new(look.skin.unwrap_or(Skin::Fluent), look.dark);
        let room = surface.room(look);
        let scale = look.px(contact.scale);
        let mut renaming = Renaming {
            gfx,
            window,
            room,
            surface: Some(surface),
            words: gfx::Layer::default(),
            drawn: 0,
            theme,
            look,
            scale,
            rect: RECT::default(),
            picture,
            work: contact.work,
            owner: owner.0 as isize,
            says: false,
            due: None,
            before: HWND::default(),
            _showing: showing,
        };
        renaming.place();
        Some(renaming)
    }

    /// Placed beside its picture, its middle level with the picture's: on the side of it with room for it on its
    /// screen, else on the other (a rail at the screen's right edge); as tall as it is now.
    fn place(&mut self) {
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let (picture, work, room) = (self.picture, self.work, self.room);
        let (w, h) = (px(WIDTH + 2.0 * room), px(height(self.says) + 2.0 * room));
        let middle = (picture.top + picture.bottom) / 2;
        let right = picture.right + px(BESIDE) - px(room);
        let left = if right + w - px(room) <= work.right { right } else { picture.left - px(BESIDE) - w + px(room) };
        let left = left.clamp(work.left - px(room), (work.right - w + px(room)).max(work.left - px(room)));
        // (Its field level with the picture's middle, a line said under it growing it downwards.)
        let top = middle - px(room + PAD + FIELD_H / 2.0);
        let top = top.clamp(work.top - px(room), (work.bottom - h + px(room)).max(work.top - px(room)));
        self.rect = RECT { left, top, right: left + w, bottom: top + h };
    }

    fn font(&self) -> Font {
        Font { size: 13.0, weight: 400.0, ..self.theme.body }
    }

    fn draw(&mut self) -> bool {
        let (written, due) = naming::shown(self.window.hwnd, Instant::now());
        let Some(written) = written else { return false };
        self.due = due;
        let says = written.said.is_some();
        if says != self.says || self.drawn == 0 {
            self.says = says;
            self.place();
            self.window.place(self.rect);
        }
        let size = ((self.rect.right - self.rect.left).max(1) as u32, (self.rect.bottom - self.rect.top).max(1) as u32);
        let Some(mut surface) = self.surface.take() else { return false };
        let mut words = std::mem::take(&mut self.words);
        self.drawn += 1;
        let (theme, font) = (&self.theme, self.font());
        let tall = height(says);
        let drawn = surface.draw(&self.gfx.clone(), size, self.scale, self.look, self.room, (WIDTH, tall), menu::sheet_radius(self.look, true, tall), &mut words, self.drawn, |_, _| {}, |frame, _| {
            let field = (PAD, PAD, FIELD_W, FIELD_H);
            crate::ui::view::naming::paint_written(frame, theme, &written, font, field, INSET, false, false, (0.0, 0.0, WIDTH, tall), &mut Vec::new());
            if let Some(said) = written.said.as_deref() {
                crate::ui::view::naming::paint_said(frame, theme, said, PAD + INSET, PAD + FIELD_H + 3.0, FIELD_W - INSET);
            }
        });
        (self.surface, self.words) = (Some(surface), words);
        if let Err(error) = &drawn {
            gfx::failed("rename", &self.gfx, Some(error));
            return false;
        }
        // (Its field on the screen: the input method's words put at its caret.)
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let (left, top) = (self.rect.left + px(self.room + PAD), self.rect.top + px(self.room + PAD));
        naming::placed(self.window.hwnd, RECT { left, top, right: left + px(FIELD_W), bottom: top + px(FIELD_H) }, self.scale);
        true
    }
}

/// Its plate's width (DIPs).
const WIDTH: f32 = 2.0 * PAD + FIELD_W;
