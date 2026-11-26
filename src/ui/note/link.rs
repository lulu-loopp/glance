//! A link written in a note, as a word processor has one written: a small plate under the words it is made of (or the
//! caret), in the look of the note's menus, with a field for the words it shows and one for where it goes. Its words
//! written as Markdown writes a link (see `notes::edit::put_link`) once both are filled in, in place of what was chosen,
//! or of the link that was there (edited). Its fields are text services' boxes of their own (see `Editor::line`), as a
//! table's cell written in is: the keys, the input method's words and the pointer's theirs while it is up.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_A, VK_C, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_INSERT, VK_MENU, VK_RETURN, VK_SHIFT, VK_TAB, VK_UP, VK_V, VK_X, VK_Y, VK_Z};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::super::canvas::{Align, Canvas, Color, Family, Font};
use super::super::editor::Editor;
use super::super::text::Lang;
use super::super::theme::{Skin, Theme};
use super::{HEAD, PAD, FOOT, WORDS};

/// Its measures (DIPs, for words `WORDS` high: larger as they are): as wide as it is at most, the room round what is
/// on it, a row's height (a field's), the room between rows, the room its rows' names take, its button's width, the
/// room inside a field before its words, how far it is from the words it is for and from its page's edges.
const WIDE: f32 = 320.0;
const INSET: f32 = 10.0;
const ROW: f32 = 32.0;
const BETWEEN: f32 = 8.0;
const NAMES: f32 = 44.0;
const BUTTON: f32 = 64.0;
const IN_FIELD: f32 = 8.0;
const GAP: f32 = 6.0;
const EDGE: f32 = 8.0;
/// How high its names, its fields' words and its button's are written (DIPs).
const SMALL: f32 = 13.0;
const TYPED: f32 = 14.0;

/// One of its fields: the words the link shows, or where it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum Field {
    Words,
    To,
}

/// Where it and what is on it are (window pixels): all of it, its fields, its button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::ui) struct Laid {
    pub plate: RECT,
    pub words: RECT,
    pub to: RECT,
    pub button: RECT,
}

fn inside(rect: &RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

/// How it ends: left with nothing written (Esc, a press off it, its note no longer written in), or its link written.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::ui) enum Ending {
    Left,
    Written(crate::notes::edit::Edit),
}

/// A link being written: the box it is written into (the note's words', or a table's cell's written in), what of its
/// words it goes in place of (UTF-16 units: what was chosen, the link that was there), what was chosen there as it opened (chosen again as it is left), the note's words' box (its page round it, which it
/// stays within), its fields, which has the keys, how high a field's
/// line is (pixels), how many pixels a DIP is, whether a press on a field is followed till it is let go, and whether the
/// character the key last down makes is to be eaten.
pub(in crate::ui) struct Linking {
    pub target: Rc<Editor>,
    pub chosen: (usize, usize),
    was: (i32, i32),
    page: Rc<Editor>,
    pub words: Rc<Editor>,
    pub to: Rc<Editor>,
    focused: Cell<Field>,
    line: i32,
    per: f32,
    pressing: Cell<bool>,
    eaten: Cell<bool>,
}

/// The link being written in a note, if one is, shared with its words' box's messages (see `Writing::answer`).
pub(in crate::ui) type Held = Rc<RefCell<Option<Rc<Linking>>>>;

impl Linking {
    /// A link written into `target` in place of `chosen` (a link there, or what is chosen), its fields filled with
    /// `words` and `to`, the note's words written in `page` `pixels` high in `face`, of `ink`: its fields made in its window, the
    /// keys given to the one to fill in first (where it goes, its words given, else its words), all of it chosen.
    pub fn open(target: Rc<Editor>, chosen: (usize, usize), page: Rc<Editor>, (words, to): (&str, &str), face: &str, pixels: f32, ink: windows::Win32::Foundation::COLORREF) -> Option<Linking> {
        let per = pixels / WORDS;
        let hwnd = page.window();
        // (Made wide enough for a line, as tall as one is found, then put where they go.)
        let room = RECT { left: 0, top: 0, right: (WIDE * per) as i32, bottom: (4.0 * ROW * per) as i32 };
        let field = || Editor::line(hwnd, room, face, TYPED * per, ink).map(Rc::new);
        let (words_field, to_field) = (field()?, field()?);
        words_field.set_text("国");
        let line = words_field.character_anywhere(0).map_or((TYPED * per * 1.4).round() as i32, |drawn| drawn.bottom - drawn.top);
        words_field.set_text(words);
        to_field.set_text(to);
        let first = if words.trim().is_empty() { Field::Words } else { Field::To };
        let was = target.selection();
        let linking = Linking { target, chosen, was, page, words: words_field, to: to_field, focused: Cell::new(first), line, per, pressing: Cell::new(false), eaten: Cell::new(false) };
        linking.lay();
        let _ = linking.target.send(WM_KILLFOCUS, WPARAM(0), LPARAM(0));
        linking.focus(first);
        Some(linking)
    }

    /// The field that has the keys.
    pub fn focused(&self) -> Field {
        self.focused.get()
    }

    pub fn field(&self, which: Field) -> &Rc<Editor> {
        match which {
            Field::Words => &self.words,
            Field::To => &self.to,
        }
    }

    /// `which` given the keys (the other told it has them no more), all its words chosen.
    fn focus(&self, which: Field) {
        let was = self.focused.replace(which);
        if was != which {
            let _ = self.field(was).send(WM_KILLFOCUS, WPARAM(0), LPARAM(0));
        }
        let field = self.field(which);
        let _ = field.send(WM_SETFOCUS, WPARAM(0), LPARAM(0));
        field.select((0, field.words().encode_utf16().count() as i32));
    }

    /// Its fields' words, as they are to be written: their spaces at their ends left out.
    pub fn filled(&self) -> (String, String) {
        (self.words.words().trim().to_string(), self.to.words().trim().to_string())
    }

    /// Whether its link can be written: both its fields filled in (no link of no words, nor one going nowhere).
    pub fn ready(&self) -> bool {
        let (words, to) = self.filled();
        !words.is_empty() && !to.is_empty()
    }

    /// Its link, written into its box (see `notes::edit::put_link`): none till it is ready.
    fn written(&self) -> Option<Ending> {
        let (words, to) = self.filled();
        self.ready().then(|| Ending::Written(crate::notes::edit::put_link(&self.target.words(), self.chosen, &words, &to)))
    }

    /// Where it is (see `Laid`): under the words it is for (the last line of them, or the caret's), over them where
    /// there is no room under them on the note's page, as wide as it is or as the page leaves it, within the page.
    pub fn laid(&self) -> Laid {
        let per = self.per;
        let at = |dips: f32| (dips * per).round() as i32;
        let bounds = self.page.bounds();
        let page = RECT { left: bounds.left - at(PAD), top: bounds.top - at(HEAD + 4.0), right: bounds.right + at(PAD), bottom: bounds.bottom + at(FOOT) };
        let (from, to) = self.chosen;
        let first = self.target.character_anywhere(from);
        let last = if to > from { self.target.character_anywhere(to - 1) } else { first };
        let box_of = self.target.bounds();
        let (left, top, bottom) = (first.map_or(box_of.left, |first| first.left), first.map_or(box_of.top, |first| first.top), last.map_or(box_of.top, |last| last.bottom));
        let wide = at(WIDE).min(page.right - page.left - 2 * at(EDGE));
        let tall = at(2.0 * INSET + 2.0 * ROW + BETWEEN);
        let below = bottom + at(GAP);
        let above = top - at(GAP) - tall;
        let top = if below + tall <= page.bottom - at(EDGE) || above < page.top + at(EDGE) { below.min(page.bottom - at(EDGE) - tall) } else { above };
        let left = left.min(page.right - at(EDGE) - wide).max(page.left + at(EDGE));
        let plate = RECT { left, top, right: left + wide, bottom: top + tall };
        let row = |nth: i32| plate.top + at(INSET) + nth * at(ROW + BETWEEN);
        let fields = plate.left + at(INSET + NAMES);
        let words = RECT { left: fields, top: row(0), right: plate.right - at(INSET), bottom: row(0) + at(ROW) };
        let button = RECT { left: plate.right - at(INSET + BUTTON), top: row(1), right: plate.right - at(INSET), bottom: row(1) + at(ROW) };
        let to = RECT { left: fields, top: row(1), right: button.left - at(BETWEEN), bottom: row(1) + at(ROW) };
        Laid { plate, words, to, button }
    }

    /// Where a field's words are written in its field `field` (window pixels): within its room, their line in its
    /// middle.
    fn words_in(&self, field: &RECT) -> RECT {
        let inset = (IN_FIELD * self.per).round() as i32;
        let top = field.top + (field.bottom - field.top - self.line) / 2;
        RECT { left: field.left + inset, top, right: field.right - inset, bottom: top + self.line }
    }

    /// Its fields' boxes put where they are now (see `laid`).
    pub fn lay(&self) -> Laid {
        let laid = self.laid();
        for (field, room) in [(&self.words, &laid.words), (&self.to, &laid.to)] {
            let rect = self.words_in(room);
            if field.bounds() != rect {
                field.place(rect);
            }
        }
        laid
    }

    /// Whether `point` (in the window) is on it.
    pub fn holds(&self, point: POINT) -> bool {
        inside(&self.laid().plate, point)
    }

    /// A window message for the note's words' box while it is up, answered by it (see `Writing::answer`): the keys, the
    /// characters they make and the input method's for the field that has them (Enter writes the link, Esc leaves it,
    /// Tab and the arrows up and down go from field to field, a paste is of one line), the pointer's on it (a field
    /// pressed given the keys, its button pressed writes the link). A press off it leaves it, the press the note's. What
    /// was answered (none: not its, the note's to answer), and how it ends, where it does.
    pub fn answer(&self, hwnd: HWND, (message, wparam, lparam): (u32, WPARAM, LPARAM)) -> (Option<Option<LRESULT>>, Option<Ending>) {
        let point = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
        let field = self.field(self.focused.get()).clone();
        let sent = || Some(Some(field.send(message, wparam, lparam).unwrap_or(LRESULT(0))));
        let done = Some(Some(LRESULT(0)));
        let eaten = |answered| {
            self.eaten.set(true);
            answered
        };
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                self.eaten.set(false);
                if field.composing() {
                    return (sent(), None);
                }
                if message == WM_SYSKEYDOWN {
                    return (done, None);
                }
                let held = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
                let (ctrl, shift, alt) = (held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU));
                let other = match self.focused.get() {
                    Field::Words => Field::To,
                    Field::To => Field::Words,
                };
                match VIRTUAL_KEY(wparam.0 as u16) {
                    VK_ESCAPE => (eaten(done), Some(Ending::Left)),
                    VK_RETURN => (eaten(done), self.written()),
                    VK_TAB => {
                        self.focus(other);
                        (eaten(done), None)
                    }
                    VK_UP if !ctrl && !alt => {
                        self.focus(Field::Words);
                        (done, None)
                    }
                    VK_DOWN if !ctrl && !alt => {
                        self.focus(Field::To);
                        (done, None)
                    }
                    VK_V if ctrl && !alt => (eaten(self.paste(hwnd, &field)), None),
                    VK_INSERT if shift && !ctrl && !alt => (eaten(self.paste(hwnd, &field)), None),
                    // (Of the keys held with Ctrl, choosing all, copying, cutting, undoing and doing again are a field's:
                    // its words are written as they are, dressed in nothing.)
                    // (Ctrl with Alt is AltGr, writing a character on many keyboards: not a shortcut.)
                    VK_A | VK_C | VK_X | VK_Z | VK_Y if ctrl && !alt => (sent(), None),
                    key if ctrl && !alt && (0x30..=0x5A).contains(&key.0) => (eaten(done), None),
                    _ => (sent(), None),
                }
            }
            WM_CHAR => match (self.eaten.replace(false), char::from_u32(wparam.0 as u32)) {
                (true, _) => (done, None),
                // (A line break, a tab, Esc, what Ctrl with a letter makes: none of a field's words. Backspace takes one
                // back.)
                (_, Some(c)) if c < ' ' && c != '\u{8}' => (done, None),
                _ => (sent(), None),
            },
            WM_KEYUP | WM_SYSCHAR | WM_UNICHAR | WM_SETFOCUS | WM_KILLFOCUS => (sent(), None),
            // (The input method's, unanswered by the field, are the system's as well.)
            WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_IME_NOTIFY | WM_IME_SETCONTEXT | WM_IME_CHAR | WM_IME_REQUEST | WM_IME_COMPOSITIONFULL | WM_IME_SELECT => (Some(field.send(message, wparam, lparam)), None),
            // (Its fields' timers theirs, the note's its own.)
            WM_TIMER => {
                let _ = field.send(message, wparam, lparam);
                (None, None)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let laid = self.lay();
                if !inside(&laid.plate, point) {
                    return (None, Some(Ending::Left));
                }
                if inside(&laid.button, point) {
                    return (done, self.written());
                }
                let pressed = [(Field::Words, laid.words), (Field::To, laid.to)].into_iter().find(|(_, room)| inside(room, point)).map(|(which, _)| which);
                let Some(which) = pressed else { return (done, None) };
                if which != self.focused.get() {
                    self.focus(which);
                }
                self.pressing.set(true);
                // (Pressed anywhere in its room, as on its words, as far as they go.)
                let inner = self.field(which).bounds();
                let at = POINT { x: point.x.min(inner.right - 1).max(inner.left), y: point.y.min(inner.bottom - 1).max(inner.top) };
                let at = LPARAM(((at.y as isize) << 16) | (at.x as isize & 0xFFFF));
                (Some(Some(self.field(which).send(message, wparam, at).unwrap_or(LRESULT(0)))), None)
            }
            // (Let go, or moved with the button up: a press whose letting go went elsewhere is over too.)
            WM_LBUTTONUP | WM_MOUSEMOVE if self.pressing.get() => {
                self.pressing.set(message == WM_MOUSEMOVE && wparam.0 & 0x0001 != 0);
                (sent(), None)
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MOUSEWHEEL | WM_MOUSEMOVE | WM_LBUTTONUP => match self.holds(point) || message == WM_MOUSEWHEEL && self.holds(self.pointer(hwnd)) {
                true => (done, None),
                // (The right button pressed off it leaves it, as the left does.)
                false => (None, (message == WM_RBUTTONDOWN).then_some(Ending::Left)),
            },
            WM_SETCURSOR => {
                let at = self.pointer(hwnd);
                let laid = self.lay();
                if !inside(&laid.plate, at) {
                    return (None, None);
                }
                match [(Field::Words, laid.words), (Field::To, laid.to)].into_iter().find(|(_, room)| inside(room, at)) {
                    Some(_) => unsafe {
                        SetCursor(LoadCursorW(None, IDC_IBEAM).ok());
                    },
                    None => unsafe {
                        SetCursor(LoadCursorW(None, if inside(&laid.button, at) && self.ready() { IDC_HAND } else { IDC_ARROW }).ok());
                    },
                }
                (Some(Some(LRESULT(1))), None)
            }
            _ => (None, None),
        }
    }

    /// Where the pointer is (in the window).
    fn pointer(&self, hwnd: HWND) -> POINT {
        let mut at = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut at);
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut at);
        }
        at
    }

    /// The clipboard's words written in `field` in place of what is chosen there, on one line (its line breaks spaces).
    fn paste(&self, hwnd: HWND, field: &Editor) -> Option<Option<LRESULT>> {
        if let Some(text) = super::clipboard_text(hwnd) {
            let one_line = text.split('\n').map(str::trim_end).collect::<Vec<_>>().join(" ");
            let (from, to) = field.selection();
            let (from, to) = (from.min(to).max(0) as usize, from.max(to).max(0) as usize);
            let end = from + one_line.encode_utf16().count();
            field.apply(&crate::notes::edit::Edit { range: from..to, with: one_line, chosen: (end, end) });
        }
        Some(Some(LRESULT(0)))
    }

    /// It drawn on `frame` (DIPs), its window's corner at `window` on it and `scale` pixels a DIP: a plate as the note's
    /// menus are (see `menu::plate`), on it each field's name and its room, the words in it as its box draws them (or,
    /// none, what goes there, faint), the one with the keys ringed in the accent and its caret where it is
    /// (`caret_on`: in its blink, shown), the button that writes the link, faint till it can.
    pub fn paint(&self, frame: &super::super::gfx::Frame, theme: &Theme, lang: Lang, window: (f32, f32), scale: f32, caret_on: bool) {
        let laid = self.lay();
        let in_window = |rect: &RECT| (window.0 + rect.left as f32 / scale, window.1 + rect.top as f32 / scale, (rect.right - rect.left) as f32 / scale, (rect.bottom - rect.top) as f32 / scale);
        let per = self.per / scale;
        let plate = super::super::menu::plate(frame, super::super::menu::Look::of(theme.skin, theme.dark), in_window(&laid.plate));
        let small = Font { size: SMALL * per, ..Font::new(Family::YaHei, SMALL, 400.0) };
        let typed = Font { size: TYPED * per, ..small };
        let accent = match theme.skin {
            Skin::Paper => theme.text,
            _ => super::super::theme::accent(theme.dark),
        };
        // (Written in the middle of `rect`, across from `x` as wide as `wide`.)
        let middle = |text: &str, font: Font, ink: Color, (x, y, wide, tall): (f32, f32, f32, f32), align: Align| {
            let high = frame.gfx.wrapped_height(text, font, wide);
            frame.text(text, font, ink, x, y + (tall - high) / 2.0, wide, align);
        };
        let names = [(Field::Words, laid.words, lang.pick("文字", "Text"), lang.pick("链接显示的文字", "Text to display")), (Field::To, laid.to, lang.pick("链接", "Link"), lang.pick("粘贴或输入链接地址", "Paste or type an address"))];
        for (which, room, name, wanted) in names {
            let (x, y, wide, tall) = in_window(&room);
            let left = in_window(&laid.plate).0 + INSET * per;
            middle(name, small, theme.text2, (left, y, NAMES * per, tall), Align::Start);
            let radius = (plate.radius - INSET * per).clamp(3.0 * per, 8.0 * per);
            frame.fill_rounded(plate.highlight, x, y, wide, tall, radius);
            let focused = self.focused.get() == which;
            let edge = if focused { accent } else { theme.rule };
            frame.stroke_rounded(super::super::canvas::Fill::Solid(edge), x, y, wide, tall, radius, if focused { 1.5 } else { 1.0 });
            let field = self.field(which);
            frame.clip(x + 1.0, y + 1.0, wide - 2.0, tall - 2.0);
            if field.words().is_empty() && !field.composing() {
                let (wx, _, ww, _) = in_window(&field.bounds());
                middle(wanted, typed, theme.text3, (wx, y, ww, tall), Align::Start);
            }
            let mut was = windows_numerics::Matrix3x2::default();
            unsafe {
                frame.dc.GetTransform(&mut was);
                frame.dc.SetTransform(&(windows_numerics::Matrix3x2::translation(window.0, window.1) * was));
            }
            frame.in_pixels(|target| field.draw(target));
            unsafe { frame.dc.SetTransform(&was) };
            if let (true, true, Some((cx, cy, cw, ch))) = (focused, caret_on, field.caret()) {
                frame.fill(theme.text, window.0 + cx as f32 / scale, window.1 + cy as f32 / scale, (cw as f32 / scale).max(1.2), ch as f32 / scale);
            }
            frame.unclip();
        }
        // Its button: in the accent, lit under the pointer, once the link can be written, faint till then.
        let (x, y, wide, tall) = in_window(&laid.button);
        let radius = (plate.radius - INSET * per).clamp(3.0 * per, 8.0 * per);
        let ready = self.ready();
        let lit = ready && inside(&laid.button, self.pointer(self.page.window()));
        let (ground, ink) = match (ready, theme.skin) {
            (false, _) => (plate.highlight, theme.text3),
            (true, Skin::Paper) => (theme.text, Color { a: 1.0, ..theme.paper }),
            (true, _) if theme.dark => (accent, Color::hex(0x000000, 0.9)),
            (true, _) => (accent, Color::hex(0xFFFFFF, 1.0)),
        };
        frame.fill_rounded(if lit { ground.alpha(0.85) } else { ground }, x, y, wide, tall, radius);
        let (said, bold) = (lang.pick("确定", "OK"), Font { weight: 600.0, ..small });
        let across = frame.measure(said, bold);
        middle(said, bold, ink, (x + (wide - across) / 2.0, y, across + 1.0, tall), Align::Start);
    }
}

/// The link being written in `held` ended as `ending` says: its fields let go, the keys given back to the box it was
/// written into, its link written there (one undo) where it was.
pub(in crate::ui) fn end(held: &Held, ending: Ending) {
    let Some(linking) = held.borrow_mut().take() else { return };
    let _ = linking.field(linking.focused.get()).send(WM_KILLFOCUS, WPARAM(0), LPARAM(0));
    let _ = linking.target.send(WM_SETFOCUS, WPARAM(0), LPARAM(0));
    match ending {
        Ending::Written(edit) => linking.target.apply(&edit),
        Ending::Left => linking.target.select(linking.was),
    }
}
