//! The picker of a to-do's day and time: a plate in the look of what it is opened from (the panel's, a widget's), by the
//! button it is opened by. Days near at hand at its top (today, tomorrow, next Monday, none), the month's days below
//! them, then the times: the whole day, the times the user keeps (added by "+", taken out by the mark on one under the
//! pointer), and one written for this once. What is chosen is set at once (the item, or the field's time, shows it
//! while the picker is up); "完成" puts it away, "清除时间" takes the time off and puts it away. Run by the loop of the
//! thread that opened it, as a menu is (see `menu::Menu`): in front, the mouse its own, left as anything else comes in
//! front or a press comes away from it.

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VIRTUAL_KEY, VK_BACK, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, SetForegroundWindow, TranslateMessage, MSG, WM_CHAR, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
};

use super::PickFor;
use crate::todo::{clock, Day, Due};
use crate::ui::canvas::{Align, Canvas, Color, Fill, Font, Point};
use crate::ui::gfx;
use crate::ui::icons::Icon;
use crate::ui::menu::{self, Look};
use crate::ui::text::Lang;
use crate::ui::theme::{Skin, Theme};
use crate::ui::view::todo::{mark, soon};
use crate::ui::window::{Window, CAPTURE_LOST};

/// Its measures (DIPs), as the agreed design has them.
const WIDTH: f32 = 286.0;
const PAD: f32 = 12.0;
const BUTTON: f32 = 28.0;
const BUTTON_PAD: f32 = 9.0;
const GAP: f32 = 6.0;
const HEAD: f32 = 28.0;
const WEEKDAYS: f32 = 22.0;
const CELL: f32 = 32.0;
const CELL_GAP: f32 = 2.0;
const LABEL: f32 = 24.0;
const INPUT: f32 = 64.0;
const REMOVE: f32 = 16.0;
/// How far from the button it is opened by.
const APART: f32 = 6.0;

/// A box on it: across, down, wide, tall (DIPs from its plate's corner).
type Area = (f32, f32, f32, f32);

/// What a place on it does.
#[derive(Clone, PartialEq, Debug)]
enum PickHit {
    /// A day near at hand (none: no day).
    Quick(Option<Day>),
    Month(i32),
    Day(Day),
    /// The whole day (no time).
    AllDay,
    Preset((u32, u32)),
    Remove((u32, u32)),
    Plus,
    /// The field a time is written in: a new one kept (`true`), or one for this once.
    Input(bool),
    Clear,
    Done,
}

/// What a message to it came to.
pub enum Handled {
    /// None of its own.
    Not,
    /// Taken, nothing chosen.
    Taken,
    /// A day or a time chosen: set already, to be shown so where it was set.
    Chosen,
    /// Done with: put away (`asked`: by its own buttons or Esc, not left).
    Done { asked: bool },
}

/// A time being written in it: for a new one to keep, or for this once; what is written; whether it was not a time.
struct Writing {
    keep: bool,
    text: String,
    wrong: bool,
}

/// The picker up.
pub struct Picker {
    gfx: Rc<gfx::Gfx>,
    window: Window,
    /// What its window shows, as a menu's (frosted over the screen on glass and Windows 11's look: see
    /// `menu::Drawn`), taken out while it is drawn on.
    surface: Option<menu::Drawn>,
    /// The room round its plate for its shadows (DIPs).
    room: f32,
    /// What is on its plate, kept as drawn (ringed on glass by the glass's halo, as the lanes' words are), and how
    /// often it was drawn (each time anew: what it shows is drawn only as it changes).
    words: gfx::Layer,
    drawn: u64,
    theme: Theme,
    look: Look,
    lang: Lang,
    scale: f32,
    rect: RECT,
    /// Whether it is above the button it was opened by.
    above: bool,
    owner: isize,
    of: PickFor,
    day: Option<Day>,
    time: Option<(u32, u32)>,
    /// The month shown (year, month).
    month: (i32, u32),
    writing: Option<Writing>,
    hover: Option<PickHit>,
    pressed: Option<PickHit>,
    hits: Vec<(f32, f32, f32, f32, PickHit)>,
    before: HWND,
    _showing: crate::ui::life::Showing,
}

thread_local! {
    static PICK: RefCell<Option<Picker>> = const { RefCell::new(None) };
}

/// The picker opened for `of` from the window `owner`, by the button at `anchor` (on the screen), in `look`: any up
/// before put away first. Whether it is up.
pub fn open(owner: HWND, of: PickFor, anchor: RECT, look: Look, lang: Lang) -> bool {
    close(false);
    let due = match &of {
        PickFor::Field => super::field_due(),
        PickFor::Item(_, item) => item.due,
    };
    let Some(picker) = Picker::new(owner, of, due, anchor, look, lang) else { return false };
    PICK.with_borrow_mut(|pick| *pick = Some(picker));
    true
}

/// Whether it is up.
pub fn is_up() -> bool {
    PICK.with_borrow(Option::is_some)
}

/// Whether it is up, opened from the window `owner`.
pub fn up_for(owner: HWND) -> bool {
    PICK.with_borrow(|pick| pick.as_ref().is_some_and(|picker| picker.owner == owner.0 as isize))
}

/// Whether `hwnd` is its window.
pub fn owns(hwnd: HWND) -> bool {
    PICK.with_borrow(|pick| pick.as_ref().is_some_and(|picker| picker.window.hwnd == hwnd))
}

/// Whether it was left by something else coming in front.
pub fn left() -> bool {
    PICK.with_borrow(|pick| pick.as_ref().is_some_and(|picker| unsafe { GetForegroundWindow() } != picker.window.hwnd))
}

/// A message for its window.
pub fn handle(msg: &MSG) -> Handled {
    PICK.with_borrow_mut(|pick| match pick.as_mut() {
        Some(picker) if picker.window.hwnd == msg.hwnd => picker.handle(msg),
        _ => Handled::Not,
    })
}

/// Put away, if it is up: the front given back to what had it, where it still has it; the field it was opened for
/// written in again, where it was put away by its own buttons or Esc (`asked`; see `RESUME`).
pub fn close(asked: bool) {
    let Some(picker) = PICK.with_borrow_mut(Option::take) else { return };
    let held = unsafe { GetForegroundWindow() } == picker.window.hwnd;
    unsafe {
        let _ = ReleaseCapture();
    }
    let rect = picker.rect;
    let (owner, of, before) = (picker.owner, picker.of.clone(), picker.before);
    picker.window.destroy();
    crate::widget::own_window_changed(rect);
    let owner = HWND(owner as *mut _);
    if asked && of == PickFor::Field {
        let _ = unsafe { PostMessageW(Some(owner), super::RESUME, WPARAM(0), LPARAM(0)) };
    } else if held && !before.is_invalid() {
        let _ = unsafe { SetForegroundWindow(before) };
    }
}

impl Picker {
    fn new(owner: HWND, of: PickFor, due: Option<Due>, anchor: RECT, look: Look, lang: Lang) -> Option<Picker> {
        let contact = crate::mouse::monitor_at(POINT { x: anchor.left, y: anchor.top })?;
        let gfx = gfx::current().ok()?;
        let showing = crate::ui::life::Showing::of(&gfx);
        let window = Window::activating().ok()?;
        // (As frosted as glass is at its most, whatever the glass it is opened from is drawn as: it lies over that
        // glass's own words, the lane it sets the time of, not over the desktop that glass is frosted for.)
        let look = Look { frost: 1.0, ..look };
        let surface = match menu::Drawn::new(&gfx, window.hwnd, look) {
            Ok(surface) => surface,
            Err(error) => {
                window.destroy();
                gfx::failed("to-do picker", Some(&error));
                return None;
            }
        };
        let today = Day::today();
        let shown = due.map_or(today, |due| due.day);
        let theme = Theme::new(look.skin.unwrap_or(Skin::Fluent), look.dark);
        let mut picker = Picker {
            gfx,
            window,
            room: surface.room(look),
            surface: Some(surface),
            words: gfx::Layer::default(),
            drawn: 0,
            theme,
            look,
            lang,
            scale: contact.scale,
            rect: RECT::default(),
            above: false,
            owner: owner.0 as isize,
            of,
            day: due.map(|due| due.day),
            time: due.and_then(|due| due.time),
            month: (shown.year, shown.month),
            writing: None,
            hover: None,
            pressed: None,
            hits: Vec::new(),
            before: HWND::default(),
            _showing: showing,
        };
        // Below the button where it fits, else above it, else as near as its screen lets it; its right edge with the
        // button's.
        let (tall, room) = (picker.layout().height, picker.room);
        let px = |dips: f32| (dips * picker.scale).round() as i32;
        let (w, h) = (px(WIDTH + 2.0 * room), px(tall + 2.0 * room));
        let work = contact.work;
        let below = anchor.bottom + px(APART) - px(room);
        let above = anchor.top - px(APART) + px(room) - h;
        picker.above = below + h - px(room) > work.bottom && above + px(room) >= work.top;
        let top = if picker.above { above } else { below.min(work.bottom - h + px(room)) };
        let left = (anchor.right - w + px(room)).clamp(work.left - px(room), (work.right - w + px(room)).max(work.left - px(room)));
        picker.rect = RECT { left, top, right: left + w, bottom: top + h };
        if !picker.draw() {
            picker.window.destroy();
            return None;
        }
        picker.window.place(picker.rect);
        picker.window.show();
        crate::widget::own_window_changed(picker.rect);
        // In front, with the keyboard and the mouse, as a menu is.
        picker.before = unsafe { GetForegroundWindow() };
        if !unsafe { SetForegroundWindow(picker.window.hwnd) }.as_bool() {
            crate::journal::note("to-do picker: not let to the front; not shown");
            picker.window.destroy();
            return None;
        }
        unsafe { SetCapture(picker.window.hwnd) };
        Some(picker)
    }

    fn zh(&self) -> bool {
        self.lang == Lang::Zh
    }

    fn font(&self) -> Font {
        Font { size: 13.0, weight: 400.0, ..self.theme.body }
    }

    fn handle(&mut self, msg: &MSG) -> Handled {
        let (x, y) = ((msg.lParam.0 & 0xFFFF) as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32);
        match msg.message {
            WM_MOUSEMOVE => {
                let hover = self.hit_at(x, y);
                if hover != self.hover {
                    self.hover = hover;
                    self.redraw();
                }
                Handled::Taken
            }
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !self.inside(x, y) => Handled::Done { asked: false },
            WM_LBUTTONDOWN => {
                self.pressed = self.hit_at(x, y);
                Handled::Taken
            }
            WM_LBUTTONUP => match self.pressed.take().filter(|pressed| self.hit_at(x, y).as_ref() == Some(pressed)) {
                Some(hit) => self.press(hit),
                None => Handled::Taken,
            },
            WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONUP => Handled::Taken,
            WM_KEYDOWN => {
                let key = VIRTUAL_KEY(msg.wParam.0 as u16);
                match key {
                    VK_ESCAPE if self.writing.as_ref().is_some_and(|writing| writing.keep) => {
                        self.writing = None;
                        self.redraw();
                        Handled::Taken
                    }
                    VK_ESCAPE => Handled::Done { asked: true },
                    VK_RETURN => self.written(),
                    VK_BACK => {
                        if let Some(writing) = self.writing.as_mut() {
                            writing.text.pop();
                            writing.wrong = false;
                            self.redraw();
                        }
                        Handled::Taken
                    }
                    _ => {
                        // (The characters it makes come as their own messages.)
                        unsafe {
                            let _ = TranslateMessage(msg);
                        }
                        Handled::Taken
                    }
                }
            }
            WM_CHAR => {
                let typed = char::from_u32(msg.wParam.0 as u32).filter(|c| c.is_ascii_digit() || *c == ':' || *c == '：');
                if let (Some(typed), Some(writing)) = (typed, self.writing.as_mut()) {
                    if writing.text.chars().count() < 5 {
                        writing.text.push(typed);
                        writing.wrong = false;
                        self.redraw();
                    }
                }
                Handled::Taken
            }
            // Alt, as with a menu, or the mouse taken from it.
            WM_SYSKEYDOWN | CAPTURE_LOST => Handled::Done { asked: false },
            _ => Handled::Not,
        }
    }

    /// A press on `hit`, let go on it.
    fn press(&mut self, hit: PickHit) -> Handled {
        let today = Day::today();
        match hit {
            PickHit::Quick(day) => {
                self.day = day;
                if day.is_none() {
                    self.time = None;
                }
                if let Some(day) = day {
                    self.month = (day.year, day.month);
                }
            }
            PickHit::Month(by) => {
                let months = self.month.0 * 12 + self.month.1 as i32 - 1 + by;
                self.month = (months.div_euclid(12), months.rem_euclid(12) as u32 + 1);
                self.redraw();
                return Handled::Taken;
            }
            PickHit::Day(day) => self.day = Some(day),
            PickHit::AllDay => {
                self.time = None;
                self.day.get_or_insert(today);
            }
            PickHit::Preset(time) => {
                self.time = Some(time);
                self.day.get_or_insert(today);
            }
            PickHit::Remove(time) => {
                let written = written_time(time);
                crate::app().change(|settings| settings.todo_times.retain(|kept| *kept != written));
                self.fit();
                return Handled::Taken;
            }
            PickHit::Plus => {
                self.writing = Some(Writing { keep: true, text: String::new(), wrong: false });
                self.fit();
                return Handled::Taken;
            }
            PickHit::Input(keep) => {
                if self.writing.as_ref().is_none_or(|writing| writing.keep != keep) {
                    let text = if keep { String::new() } else { self.once().map(written_time).unwrap_or_default() };
                    self.writing = Some(Writing { keep, text, wrong: false });
                    self.fit();
                }
                return Handled::Taken;
            }
            PickHit::Clear => {
                (self.day, self.time) = (None, None);
                self.apply();
                return Handled::Done { asked: true };
            }
            PickHit::Done => return Handled::Done { asked: true },
        }
        self.apply();
        self.fit();
        Handled::Chosen
    }

    /// The time written in it (Enter): chosen, and kept among the user's where it was written as a new one; not a
    /// time, said so.
    fn written(&mut self) -> Handled {
        let Some(writing) = self.writing.as_mut() else { return Handled::Taken };
        let Some(time) = clock(writing.text.trim()) else {
            writing.wrong = true;
            self.redraw();
            return Handled::Taken;
        };
        if writing.keep {
            let written = written_time(time);
            crate::app().change(|settings| {
                if !settings.todo_times.contains(&written) {
                    settings.todo_times.push(written);
                    settings.todo_times.sort_by_key(|kept| clock(kept));
                }
            });
        }
        self.writing = None;
        self.time = Some(time);
        self.day.get_or_insert(Day::today());
        self.apply();
        self.fit();
        Handled::Chosen
    }

    /// What is chosen set where it is for: the item's time, or the field's.
    fn apply(&mut self) {
        let due = self.day.map(|day| Due { day, time: self.time });
        match &mut self.of {
            PickFor::Field => super::set_field_due(due),
            PickFor::Item(n, item) => {
                crate::todo::change_item(*n, item, |list, n| list.set_due(n, due));
                item.due = due;
            }
        }
    }

    /// The time chosen, where it is none of the user's kept ones (written in for this once).
    fn once(&self) -> Option<(u32, u32)> {
        self.time.filter(|time| !self.presets().contains(time))
    }

    /// The times the user keeps, in order.
    fn presets(&self) -> Vec<(u32, u32)> {
        crate::app().settings.lock().unwrap().todo_times.iter().filter_map(|kept| clock(kept)).collect()
    }

    /// Drawn again as it is now, its window as tall as it is (its top kept, or its bottom, opened above its button).
    fn fit(&mut self) {
        let (tall, room) = (self.layout().height, self.room);
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let h = px(tall + 2.0 * room);
        if self.rect.bottom - self.rect.top != h {
            if self.above {
                self.rect.top = self.rect.bottom - h;
            } else {
                self.rect.bottom = self.rect.top + h;
            }
            let _ = self.draw();
            self.window.place(self.rect);
            crate::widget::own_window_changed(self.rect);
        } else {
            self.redraw();
        }
    }

    fn redraw(&mut self) {
        let _ = self.draw();
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        let (x, y) = (x as f32 / self.scale - self.room, y as f32 / self.scale - self.room);
        let height = (self.rect.bottom - self.rect.top) as f32 / self.scale - 2.0 * self.room;
        (0.0..=WIDTH).contains(&x) && (0.0..=height).contains(&y)
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<PickHit> {
        let (x, y) = (x as f32 / self.scale - self.room, y as f32 / self.scale - self.room);
        self.hits.iter().find(|(hx, hy, w, h, _)| x >= *hx && x < hx + w && y >= *hy && y < hy + h).map(|hit| hit.4.clone())
    }

    /// Where everything on it goes.
    fn layout(&self) -> Layout {
        let zh = self.zh();
        let font = self.font();
        let measure = |text: &str| self.gfx.measure(text, font);
        let today = Day::today();
        let inner = WIDTH - 2.0 * PAD;
        // Buttons in a row, wrapped onto the next as they reach its end.
        let flow = |y: &mut f32, x: &mut f32, wide: f32| {
            if *x > PAD && *x + wide > PAD + inner {
                *x = PAD;
                *y += BUTTON + GAP;
            }
            let at = (*x, *y, wide, BUTTON);
            *x += wide + GAP;
            at
        };
        let mut y = PAD;
        let mut x = PAD;
        let mut quick = Vec::new();
        let next_monday = today.plus(7 - i64::from(today.weekday()));
        for (label, day) in [(lang(zh, "今天", "Today"), Some(today)), (lang(zh, "明天", "Tomorrow"), Some(today.plus(1))), (lang(zh, "下周一", "Next Mon"), Some(next_monday)), (lang(zh, "不设日期", "No date"), None)] {
            quick.push((flow(&mut y, &mut x, measure(label) + 2.0 * BUTTON_PAD), label.to_string(), day));
        }
        y += BUTTON + 12.0;
        let head = y;
        y += HEAD + 4.0;
        let weekdays = y;
        y += WEEKDAYS;
        let (year, month) = self.month;
        let first = Day { year, month, day: 1 };
        let lead = first.weekday();
        let column = inner / 7.0;
        let mut cells = Vec::new();
        for day in 1..=Day::month_days(year, month) {
            let slot = lead + day - 1;
            let (col, row) = (slot % 7, slot / 7);
            let cx = PAD + column * (col as f32 + 0.5);
            let cy = y + row as f32 * (CELL + CELL_GAP) + CELL / 2.0;
            cells.push(((cx - CELL / 2.0, cy - CELL / 2.0, CELL, CELL), Day { year, month, day }));
        }
        let rows = (lead + Day::month_days(year, month)).div_ceil(7);
        y += rows as f32 * (CELL + CELL_GAP) - CELL_GAP + 12.0;
        let times_label = y;
        y += LABEL;
        let mut x = PAD;
        let all_day_label = lang(zh, "全天", "All day");
        let all_day = flow(&mut y, &mut x, measure(all_day_label) + 2.0 * BUTTON_PAD);
        let mut presets = Vec::new();
        for time in self.presets() {
            let text = written_time(time);
            let at = flow(&mut y, &mut x, measure(&text) + 2.0 * BUTTON_PAD);
            presets.push((at, text, time));
        }
        let plus = match &self.writing {
            Some(writing) if writing.keep => Err(flow(&mut y, &mut x, INPUT)),
            _ => Ok(flow(&mut y, &mut x, 30.0)),
        };
        y += BUTTON + 8.0;
        let once_label = lang(zh, "其他时间", "Other time");
        let once = (PAD + measure(once_label) + 10.0, y, INPUT, BUTTON);
        let once_label_y = y;
        y += BUTTON + 12.0;
        let rule = y;
        y += 10.0;
        let clear_label = lang(zh, "清除时间", "Clear time");
        let done_label = lang(zh, "完成", "Done");
        let clear = (PAD, y, measure(clear_label) + 2.0 * BUTTON_PAD, BUTTON);
        let done_w = measure(done_label) + 2.0 * BUTTON_PAD + 6.0;
        let done = (PAD + inner - done_w, y, done_w, BUTTON);
        y += BUTTON + PAD;
        Layout { quick, head, weekdays, cells, times_label, all_day: (all_day, all_day_label.to_string()), presets, plus, once_label: (once_label.to_string(), once_label_y), once, rule, clear: (clear, clear_label.to_string()), done: (done, done_label.to_string()), height: y }
    }

    fn draw(&mut self) -> bool {
        let layout = self.layout();
        let mut hits = Vec::new();
        let size = ((self.rect.right - self.rect.left).max(1) as u32, (self.rect.bottom - self.rect.top).max(1) as u32);
        let Some(mut surface) = self.surface.take() else { return false };
        let mut words = std::mem::take(&mut self.words);
        self.drawn += 1;
        let drawn = surface.draw(&self.gfx.clone(), size, self.scale, self.look, self.room, (WIDTH, layout.height), &mut words, self.drawn, |_, _| {}, |frame, frosted| self.paint(frame, &layout, frosted, &mut hits));
        (self.surface, self.words) = (Some(surface), words);
        self.hits = hits;
        if let Err(error) = &drawn {
            gfx::failed("to-do picker", Some(error));
        }
        drawn.is_ok()
    }

    fn paint(&self, frame: &dyn Canvas, layout: &Layout, frosted: bool, hits: &mut Vec<(f32, f32, f32, f32, PickHit)>) {
        let theme = &self.theme;
        let zh = self.zh();
        let font = self.font();
        let bold = Font { weight: 600.0, ..font };
        let small = Font { size: 11.5, ..font };
        let today = Day::today();
        // (On its plate, drawn as a menu's is: see `draw`.)
        let highlight = menu::Colours::new(self.look, frosted).highlight;
        let (accent, on_accent) = mark(theme);
        let round = match theme.skin {
            Skin::Fluent => 4.0,
            _ => 7.0,
        };
        let hover = self.hover.as_ref();
        // A button: its ground (chosen, lit, or plain on the track), its words.
        let button = |at: Area, text: &str, chosen: bool, plain: bool, hit: PickHit, radius: f32, hits: &mut Vec<(f32, f32, f32, f32, PickHit)>| {
            let (x, y, w, h) = at;
            let lit = hover == Some(&hit);
            let ink = if chosen {
                if theme.skin == Skin::Glass {
                    crate::ui::view::glass_pill(frame, theme, (x, y, w, h), radius, lit);
                    theme.text
                } else {
                    frame.fill_rounded(accent, x, y, w, h, radius);
                    on_accent
                }
            } else {
                if !plain {
                    frame.fill_rounded(theme.track, x, y, w, h, radius);
                }
                if lit {
                    frame.fill_rounded(highlight, x, y, w, h, radius);
                }
                if plain { theme.text2 } else { theme.text }
            };
            let (top, bottom) = frame.ink(text, font);
            centred(frame, text, font, ink, x, y + h / 2.0 - (top + bottom) / 2.0, w);
            hits.push((x, y, w, h, hit));
        };
        for (at, label, day) in &layout.quick {
            button(*at, label, self.day == *day, false, PickHit::Quick(*day), round, hits);
        }
        // The month, and the buttons to the one before and after.
        let (year, month) = self.month;
        let title = if zh { format!("{year}年{month}月") } else { format!("{} {year}", MONTHS[month as usize - 1]) };
        let (top, bottom) = frame.ink(&title, bold);
        frame.text(&title, bold, theme.text, PAD, layout.head + HEAD / 2.0 - (top + bottom) / 2.0, WIDTH, Align::Start);
        for (by, icon, x) in [(-1, Icon::ChevronLeft, WIDTH - PAD - 2.0 * HEAD - 2.0), (1, Icon::ChevronRight, WIDTH - PAD - HEAD)] {
            let hit = PickHit::Month(by);
            if hover == Some(&hit) {
                frame.fill_rounded(highlight, x, layout.head, HEAD, HEAD, round);
            }
            frame.icon(icon, Point { x: x + HEAD / 2.0, y: layout.head + HEAD / 2.0 }, 16.0, theme.text2);
            hits.push((x, layout.head, HEAD, HEAD, hit));
        }
        let column = (WIDTH - 2.0 * PAD) / 7.0;
        let names: [&str; 7] = if zh { ["一", "二", "三", "四", "五", "六", "日"] } else { ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] };
        for (n, name) in names.iter().enumerate() {
            let (top, bottom) = frame.ink(name, small);
            centred(frame, name, small, theme.text3, PAD + column * n as f32, layout.weekdays + WEEKDAYS / 2.0 - (top + bottom) / 2.0 - 2.0, column);
        }
        // The month's days: chosen, today (in the accent, bold), gone by (faint); round as glass's marks are, else as
        // the skin rounds its controls.
        let cell_round = match theme.skin {
            Skin::Glass => CELL / 2.0,
            _ => 6.0,
        };
        for ((x, y, w, h), day) in &layout.cells {
            let hit = PickHit::Day(*day);
            let chosen = self.day == Some(*day);
            let ink = if chosen {
                if theme.skin == Skin::Glass {
                    crate::ui::view::glass_pill(frame, theme, (*x, *y, *w, *h), cell_round, hover == Some(&hit));
                    theme.text
                } else {
                    frame.fill_rounded(accent, *x, *y, *w, *h, cell_round);
                    on_accent
                }
            } else {
                if hover == Some(&hit) {
                    frame.fill_rounded(highlight, *x, *y, *w, *h, cell_round);
                }
                if *day == today {
                    soon(theme)
                } else if *day < today {
                    theme.text3
                } else {
                    theme.text
                }
            };
            let text = day.day.to_string();
            let face = if *day == today && !chosen { Font { weight: 700.0, ..font } } else { font };
            let (top, bottom) = frame.ink(&text, face);
            centred(frame, &text, face, ink, *x, y + h / 2.0 - (top + bottom) / 2.0, *w);
            hits.push((*x, *y, *w, *h, hit));
        }
        let label = lang(zh, "时间", "Time");
        let (top, bottom) = frame.ink(label, bold);
        frame.text(label, bold, theme.text, PAD, layout.times_label + LABEL / 2.0 - (top + bottom) / 2.0 - 3.0, WIDTH, Align::Start);
        let (at, text) = &layout.all_day;
        button(*at, text, self.day.is_some() && self.time.is_none(), false, PickHit::AllDay, round, hits);
        // The times kept: under the pointer, the mark that takes one out (before it, for its press to be its own).
        for ((x, y, w, h), text, time) in &layout.presets {
            let remove = PickHit::Remove(*time);
            let lit = hover == Some(&PickHit::Preset(*time)) || hover == Some(&remove);
            if lit {
                hits.push((x + w - REMOVE + 6.0 - 4.0, y - 6.0 - 4.0, REMOVE + 8.0, REMOVE + 8.0, remove.clone()));
            }
            button((*x, *y, *w, *h), text, self.time == Some(*time), false, PickHit::Preset(*time), round, hits);
            if lit {
                let centre = Point { x: x + w - REMOVE / 2.0 + 6.0, y: y - 6.0 + REMOVE / 2.0 };
                frame.fill_circle(if hover == Some(&remove) { theme.text2 } else { theme.text3 }, centre, REMOVE / 2.0);
                frame.icon(Icon::Close, centre, 10.0, on_accent_of(theme));
            }
        }
        match layout.plus {
            Ok((x, y, w, h)) => {
                let hit = PickHit::Plus;
                if hover == Some(&hit) {
                    frame.fill_rounded(highlight, x, y, w, h, round);
                }
                frame.stroke_rounded(Fill::Solid(theme.rule), x + 0.5, y + 0.5, w - 1.0, h - 1.0, round, 1.0);
                frame.icon(Icon::Add, Point { x: x + w / 2.0, y: y + h / 2.0 }, 14.0, theme.text2);
                hits.push((x, y, w, h, hit));
            }
            Err(at) => self.paint_input(frame, at, true, lang(zh, "如 7:30", "e.g. 7:30"), round, hits),
        }
        let (label, y) = &layout.once_label;
        let (top, bottom) = frame.ink(label, Font { size: 12.5, ..font });
        frame.text(label, Font { size: 12.5, ..font }, theme.text3, PAD, y + BUTTON / 2.0 - (top + bottom) / 2.0, WIDTH, Align::Start);
        self.paint_input(frame, layout.once, false, lang(zh, "如 14:30", "e.g. 14:30"), round, hits);
        frame.fill(theme.rule, PAD, layout.rule, WIDTH - 2.0 * PAD, 1.0);
        let (at, text) = &layout.clear;
        button(*at, text, false, true, PickHit::Clear, round, hits);
        let (at, text) = &layout.done;
        button(*at, text, true, false, PickHit::Done, round, hits);
    }

    /// A field a time is written in: for a new one to keep (`keep`), or for this once (showing the time chosen, where
    /// it is none of those kept); its caret where it is written in, ringed in red where what was written is no time.
    fn paint_input(&self, frame: &dyn Canvas, (x, y, w, h): Area, keep: bool, hint: &str, round: f32, hits: &mut Vec<(f32, f32, f32, f32, PickHit)>) {
        let theme = &self.theme;
        let font = self.font();
        let writing = self.writing.as_ref().filter(|writing| writing.keep == keep);
        frame.fill_rounded(theme.track, x, y, w, h, round);
        let (accent, _) = mark(theme);
        match writing {
            Some(writing) if writing.wrong => frame.stroke_rounded(Fill::Solid(Color::hex(0xFF5A5A, 1.0)), x - 0.75, y - 0.75, w + 1.5, h + 1.5, round + 0.75, 1.5),
            Some(_) => frame.stroke_rounded(Fill::Solid(if theme.skin == Skin::Glass { theme.text2 } else { accent }), x - 0.75, y - 0.75, w + 1.5, h + 1.5, round + 0.75, 1.5),
            None => {}
        }
        let text = match writing {
            Some(writing) => writing.text.clone(),
            None if !keep => self.once().map(written_time).unwrap_or_default(),
            None => String::new(),
        };
        let (top, bottom) = frame.ink("0:", font);
        let line = y + h / 2.0 - (top + bottom) / 2.0;
        if text.is_empty() {
            frame.text(hint, font, theme.text3, x + 8.0, line, w - 10.0, Align::Start);
        } else {
            frame.text(&text, font, theme.text, x + 8.0, line, w - 10.0, Align::Start);
        }
        if writing.is_some() {
            let caret = x + 8.0 + if text.is_empty() { 0.0 } else { frame.measure(&text, font) };
            frame.fill(theme.text, caret, y + 6.0, 1.2, h - 12.0);
        }
        hits.push((x, y, w, h, PickHit::Input(keep)));
    }
}

/// Where everything on the picker goes (DIPs from its plate's corner; boxes as x, y, width, height).
struct Layout {
    quick: Vec<(Area, String, Option<Day>)>,
    head: f32,
    weekdays: f32,
    cells: Vec<(Area, Day)>,
    times_label: f32,
    all_day: (Area, String),
    presets: Vec<(Area, String, (u32, u32))>,
    /// The "+" button, or the field a new time is written in.
    plus: Result<Area, Area>,
    once_label: (String, f32),
    once: Area,
    rule: f32,
    clear: (Area, String),
    done: (Area, String),
    height: f32,
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn lang<'a>(zh: bool, chinese: &'a str, english: &'a str) -> &'a str {
    if zh {
        chinese
    } else {
        english
    }
}

/// `text` drawn in the middle of the `width` from `x`.
#[allow(clippy::too_many_arguments)]
fn centred(frame: &dyn Canvas, text: &str, font: Font, ink: Color, x: f32, y: f32, width: f32) {
    let wide = frame.measure(text, font);
    frame.text(text, font, ink, x + (width - wide) / 2.0, y, wide + 1.0, Align::Start);
}

/// A time as it is kept and shown: `09:00`.
fn written_time((hour, minute): (u32, u32)) -> String {
    format!("{hour:02}:{minute:02}")
}

/// The ink of the mark on a dot of faint ink (the one that takes a kept time out): the sheet's.
fn on_accent_of(theme: &Theme) -> Color {
    if theme.dark {
        Color::hex(0x1B1B22, 1.0)
    } else {
        Color::hex(0xFFFFFF, 1.0)
    }
}
