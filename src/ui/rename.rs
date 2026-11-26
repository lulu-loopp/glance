//! The box a name is written in: a thing kept at hand renamed (its menu's "重命名"), a tag of what was copied made or
//! renamed. A plate in the look of what it was opened from, where that menu's row (or button) was, holding one line to
//! write the name in (the name it has now, all of it chosen) and what the keys do. Enter keeps what is written (see
//! `Kept`): for a thing kept at hand, as the name it is shown by in quick launch alone, its file, folder or address left
//! as it is (written empty, its own name again). Esc, a press away from it or another window in front leaves it as it
//! was. Its keys are taken as a note's are (see `ui::keys`), the input method's words
//! put at its caret. Run by the loop of the thread that opened it, as a menu is (see `menu::Menu`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetCapture, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, PostMessageW, SetForegroundWindow, MSG, WM_CHAR, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP};

use crate::station::apps::App;
use crate::ui::canvas::{Align, Canvas, Fill, Font};
use crate::ui::gfx;
use crate::ui::menu::{self, Look};
use crate::ui::text::Lang;
use crate::ui::theme::{Skin, Theme};
use crate::ui::typing::Typing;
use crate::ui::window::{Window, CAPTURE_LOST};

/// Its measures (DIPs), as the agreed design has them.
const PAD: f32 = 10.0;
const FIELD_W: f32 = 150.0;
/// A file's name's field: wider, a file's names being longer.
const FILE_FIELD_W: f32 = 240.0;
const FIELD_H: f32 = 28.0;
const GAP: f32 = 8.0;
/// The words' inset in the field.
const INSET: f32 = 8.0;
/// The most a name holds (UTF-16 units), and a tag's.
const MOST: usize = 64;
const TAG_MOST: usize = 24;
/// The most a file's name holds (Windows' own most).
const NAME_MOST: usize = 255;

/// What is done with what is written, as it is kept.
pub type Kept = Box<dyn FnOnce(&str)>;

/// What is done with what is written where that may be refused (see `Naming::file`): tried on a thread of its own (a
/// file renamed on a drive slow to answer never holds the panel), the box up till it answers: done, or why not.
pub type Tried = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What is done with what is written as Enter is pressed: done as the box goes, or tried first (see `Tried`).
enum Keeping {
    Now(Kept),
    Tried(Tried),
}

/// What a name is written for: the name it has now, what is said where nothing is written, the most it holds (UTF-16
/// units), how much of it is chosen as the box opens (UTF-16 units from its start), how wide its field is (DIPs), and
/// what is done with what is written.
pub struct Naming {
    shown: String,
    empty: &'static str,
    most: usize,
    chosen: usize,
    field: f32,
    keep: Keeping,
}

impl Naming {
    /// `app`, kept at hand, shown by `shown` now: renamed in quick launch alone (see `kept`).
    pub fn app(app: App, shown: String) -> Naming {
        let was = shown.clone();
        let keep = move |written: &str| {
            if let Some(alias) = kept(&app, &was, written) {
                crate::station::apps::rename(app, alias);
            }
        };
        Naming { chosen: units(&shown), shown, empty: "", most: MOST, field: FIELD_W, keep: Keeping::Now(Box::new(keep)) }
    }

    /// A tag of what was copied, named `shown` now (nothing: a new one): what is written given to `keep`.
    pub fn tag(shown: String, lang: Lang, keep: impl FnOnce(&str) + 'static) -> Naming {
        Naming { chosen: units(&shown), shown, empty: lang.pick("标签名称", "Tag name"), most: TAG_MOST, field: FIELD_W, keep: Keeping::Now(Box::new(keep)) }
    }

    /// A file's name, or (`folder`) a folder's, `shown` now: written whole, its kind's ending not chosen (as Explorer
    /// has it). What is written taken as a file's name is (see `os::name_written`) given to `rename` where it is
    /// another: done, or why not (said in the box, which stays). Written empty, or as it was, it is left as it was; one
    /// a name may not be is refused, said why.
    pub fn file(shown: String, folder: bool, lang: Lang, rename: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Naming {
        let was = shown.clone();
        let tried = move |written: &str| match crate::os::name_written(written) {
            Ok(None) => Ok(()),
            Ok(Some(name)) if name == was => Ok(()),
            Ok(Some(name)) => rename(&name),
            Err(crate::os::NameRefused::Holds) => Err(lang.pick("名称不能包含 \\ / : * ? \" < > |", "A name can't contain \\ / : * ? \" < > |").into()),
            Err(crate::os::NameRefused::Device) => Err(lang.pick("此名称由系统保留", "This name is reserved by Windows").into()),
        };
        let chosen = if folder { units(&shown) } else { units(stem(&shown)) };
        Naming { shown, empty: lang.pick("名称", "Name"), most: NAME_MOST, chosen, field: FILE_FIELD_W, keep: Keeping::Tried(Arc::new(tried)) }
    }
}

/// How long `text` is in UTF-16 units.
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A file's name but its kind's ending (".jpg"): all of a name with none, or only one (".gitignore").
fn stem(name: &str) -> &str {
    name.rfind('.').filter(|at| *at > 0).map_or(name, |at| &name[..at])
}

/// What `keys::LEFT` posted to its window says where Enter was pressed: what is written kept. (Esc posts 1, another
/// window in front 0: left as it was.)
const KEEP: usize = 2;

/// Posted to its window where what was written, tried (see `Tried`), was refused: why, in `Renaming::answer`.
const REFUSED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 60;

/// What a message to it came to.
pub enum Handled {
    /// None of its own.
    Not,
    /// Taken, it still up.
    Taken,
    /// Left as it was (a press away from it, the mouse taken from it).
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
    lang: Lang,
    scale: f32,
    rect: RECT,
    /// The window its menu was opened from (the panel's, a widget's).
    owner: isize,
    /// What is said where nothing is written, and what is done with what is (taken as it goes).
    empty: &'static str,
    keep: Option<Keeping>,
    /// How wide its field is (DIPs).
    field: f32,
    /// Where it was asked (its corner there, as near as its screen lets it), and that screen's room for it.
    at: POINT,
    work: RECT,
    /// What is written being tried (see `Tried`): the keys write nothing meanwhile.
    trying: bool,
    /// Why what was written last was refused, as it answered (see `REFUSED`), and said in place of what the keys do till
    /// what is written changes.
    answer: Arc<Mutex<Option<String>>>,
    said: Option<String>,
    typing: Typing,
    /// How far the words are turned left in the field, their caret kept in it (DIPs).
    scroll: f32,
    before: HWND,
    _showing: crate::ui::life::Showing,
}

thread_local! {
    static RENAME: RefCell<Option<Renaming>> = const { RefCell::new(None) };
}

/// The box opened for `naming`, from the window `owner` (its menu, a button of it), its corner at `at` (on the screen:
/// where the menu's row, or the button, was), in `look`: any up before left first. Whether it is up.
pub fn open(owner: HWND, naming: Naming, at: POINT, look: Look, lang: Lang) -> bool {
    close(false);
    let Some(renaming) = Renaming::new(owner, naming, at, look, lang) else { return false };
    let hwnd = renaming.window.hwnd;
    RENAME.with_borrow_mut(|rename| *rename = Some(renaming));
    // In front, with the keyboard and the mouse, as a menu is: its keys its own (see `answer`).
    crate::ui::keys::take(hwnd, Rc::new(answer));
    let before = unsafe { GetForegroundWindow() };
    if !unsafe { SetForegroundWindow(hwnd) }.as_bool() {
        crate::journal::note("rename: not let to the front; not shown");
        close(false);
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

/// Whether `keys::LEFT`, posted to its window with `wparam`, keeps what is written (Enter), else leaves it as it was.
pub fn keeps(wparam: WPARAM) -> bool {
    wparam.0 == KEEP
}

/// A message for its window but its keys' (the pointer's).
pub fn handle(msg: &MSG) -> Handled {
    RENAME.with_borrow_mut(|rename| match rename.as_mut() {
        Some(renaming) if renaming.window.hwnd == msg.hwnd => renaming.handle(msg),
        _ => Handled::Not,
    })
}

/// Put away, if it is up, what is written kept where `keep` (see `kept`): the front given back to what had it, where it
/// still has it.
pub fn close(keep: bool) {
    let Some(renaming) = RENAME.with_borrow_mut(Option::take) else { return };
    let hwnd = renaming.window.hwnd;
    let held = unsafe { GetForegroundWindow() } == hwnd;
    crate::ui::keys::give_back(hwnd);
    crate::ui::window::let_go_of_capture(hwnd);
    let rect = renaming.rect;
    let before = renaming.before;
    renaming.window.destroy();
    // (What was tried is done as it answered.)
    if let (true, Some(Keeping::Now(kept))) = (keep, renaming.keep) {
        kept(&renaming.typing.string());
    }
    crate::widget::own_window_changed(rect);
    if held && !before.is_invalid() {
        let _ = unsafe { SetForegroundWindow(before) };
    }
}

/// What `written` in the box makes of the name `app` is shown by in quick launch (it opened with `was`): nothing,
/// where it is what it opened with; its own name again (none of its own), where it is written empty or as its own
/// name; else what is written, its spaces at its ends left out.
fn kept(app: &App, was: &str, written: &str) -> Option<Option<String>> {
    let written = written.trim();
    if written == was.trim() {
        return None;
    }
    Some((!written.is_empty() && written != app.name).then(|| written.to_string()))
}

/// A message for its window while it takes the keys (see `ui::keys::Answer`): Enter keeps what is written (told as
/// `keys::LEFT` with `KEEP`), Esc is not its (the keys given back leave it), the rest write in its field: the caret
/// moved (with Shift, the selection), words erased, all chosen, a line pasted, what is typed put in.
fn answer(hwnd: HWND, message: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<Option<LRESULT>> {
    match message {
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            if key == VK_ESCAPE {
                return None;
            }
            if key == VK_RETURN {
                if RENAME.with_borrow_mut(|rename| rename.as_mut().is_some_and(|renaming| renaming.enter(hwnd))) {
                    let _ = unsafe { PostMessageW(Some(hwnd), crate::ui::keys::LEFT, WPARAM(KEEP), LPARAM(0)) };
                }
                return Some(Some(LRESULT(0)));
            }
            let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(key.0)) } < 0;
            let (ctrl, shift) = (down(VK_CONTROL), down(VK_SHIFT));
            let pasted = (ctrl && key.0 == u16::from(b'V')).then(|| crate::clipboard::text(hwnd).unwrap_or_default());
            written(|typing| match key {
                VK_LEFT => typing.go(if typing.caret != typing.anchor && !shift { typing.selection().start } else { typing.step(typing.caret, false) }, shift),
                VK_RIGHT => typing.go(if typing.caret != typing.anchor && !shift { typing.selection().end } else { typing.step(typing.caret, true) }, shift),
                VK_HOME => typing.go(0, shift),
                VK_END => typing.go(typing.text.len(), shift),
                VK_BACK => typing.erase(false),
                VK_DELETE => typing.erase(true),
                _ if ctrl && key.0 == u16::from(b'A') => {
                    typing.go(0, false);
                    typing.go(typing.text.len(), true);
                }
                // (One line of what was pasted: a name is a line.)
                _ if pasted.is_some() => {
                    let line: Vec<u16> = pasted.as_deref().unwrap_or_default().lines().next().unwrap_or_default().encode_utf16().collect();
                    typing.put(&line);
                }
                _ => {}
            });
            Some(Some(LRESULT(0)))
        }
        WM_CHAR => {
            let unit = wparam.0 as u16;
            if unit >= 0x20 && unit != 0x7F {
                written(|typing| typing.put(&[unit]));
            }
            Some(Some(LRESULT(0)))
        }
        WM_KEYUP => Some(Some(LRESULT(0))),
        WM_IME_STARTCOMPOSITION => {
            // (Where its caret is found first, the box let go of: the input method asks the window again as it is
            // placed.)
            if let Some((at, tall, size)) = RENAME.with_borrow(|rename| rename.as_ref().map(Renaming::caret)) {
                crate::ui::typing::place_ime(hwnd, at, tall, "Microsoft YaHei UI", size);
            }
            Some(None)
        }
        _ => None,
    }
}

/// `change` made to what is written in it (none while it is tried), drawn so: why it was refused no longer said.
fn written(change: impl FnOnce(&mut Typing)) {
    RENAME.with_borrow_mut(|rename| {
        if let Some(renaming) = rename.as_mut().filter(|renaming| !renaming.trying) {
            let was = renaming.typing.text.clone();
            change(&mut renaming.typing);
            renaming.follow_caret();
            if renaming.said.is_some() && renaming.typing.text != was {
                renaming.said = None;
                renaming.place();
            }
            let _ = renaming.draw();
        }
    });
}

impl Renaming {
    /// Enter pressed in its window `hwnd`: whether what is written is kept as it goes (see `KEEP`). Where it is to be
    /// tried first, tried (once at a time): its answer posted to it, kept where it was done (`KEEP`), else why not
    /// (`REFUSED`).
    fn enter(&mut self, hwnd: HWND) -> bool {
        let Some(Keeping::Tried(tried)) = &self.keep else { return true };
        if !self.trying {
            self.trying = true;
            let (tried, written, answer, window) = (tried.clone(), self.typing.string(), self.answer.clone(), hwnd.0 as isize);
            std::thread::spawn(move || {
                let done = tried(&written);
                let (message, wparam) = if done.is_ok() { (crate::ui::keys::LEFT, KEEP) } else { (REFUSED, 0) };
                *answer.lock().unwrap() = done.err();
                let _ = unsafe { PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(wparam), LPARAM(0)) };
            });
        }
        false
    }

    /// What is said beside its field: why what was written was refused, else what the keys do.
    fn aside(&self) -> &str {
        self.said.as_deref().unwrap_or_else(|| self.hint())
    }

    /// Its window where it was asked, as wide as it is now (see `width`), its corner as near there as its screen lets
    /// it.
    fn place(&mut self) {
        let room = self.room;
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let (w, h) = (px(self.width() + 2.0 * room), px(HEIGHT + 2.0 * room));
        let (at, work) = (self.at, self.work);
        let left = (at.x - px(room)).clamp(work.left - px(room), (work.right - w + px(room)).max(work.left - px(room)));
        let top = (at.y - px(room)).clamp(work.top - px(room), (work.bottom - h + px(room)).max(work.top - px(room)));
        let was = self.rect;
        self.rect = RECT { left, top, right: left + w, bottom: top + h };
        if was != RECT::default() {
            self.window.place(self.rect);
            crate::widget::own_window_changed(was);
            crate::widget::own_window_changed(self.rect);
        }
    }

    fn new(owner: HWND, naming: Naming, at: POINT, look: Look, lang: Lang) -> Option<Renaming> {
        let contact = crate::mouse::monitor_at(at)?;
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
        let mut typing = Typing::most(naming.most);
        typing.put(&naming.shown.encode_utf16().collect::<Vec<_>>());
        typing.go(0, false);
        typing.go(naming.chosen.min(typing.text.len()), true);
        let mut renaming = Renaming {
            gfx,
            window,
            room: surface.room(look),
            surface: Some(surface),
            words: gfx::Layer::default(),
            drawn: 0,
            theme,
            look,
            lang,
            scale: look.px(contact.scale),
            rect: RECT::default(),
            owner: owner.0 as isize,
            empty: naming.empty,
            keep: Some(naming.keep),
            field: naming.field,
            at,
            work: contact.work,
            trying: false,
            answer: Arc::default(),
            said: None,
            typing,
            scroll: 0.0,
            before: HWND::default(),
            _showing: showing,
        };
        // Its plate's corner where it was asked, as near as its screen lets it.
        renaming.place();
        renaming.follow_caret();
        if !renaming.draw() {
            renaming.window.destroy();
            return None;
        }
        renaming.window.place(renaming.rect);
        renaming.window.show();
        crate::widget::own_window_changed(renaming.rect);
        Some(renaming)
    }

    fn font(&self) -> Font {
        Font { size: 13.0, weight: 400.0, ..self.theme.body }
    }

    fn hint(&self) -> &'static str {
        self.lang.pick("回车确定，Esc 取消", "Enter to save, Esc to cancel")
    }

    fn hint_font(&self) -> Font {
        Font { size: 12.0, ..self.font() }
    }

    /// How wide its plate is (DIPs): its field, then what is said beside it.
    fn width(&self) -> f32 {
        PAD + self.field + GAP + self.gfx.measure(self.aside(), self.hint_font()).ceil() + PAD
    }

    /// How far into its words (DIPs) the unit `at` of them is.
    fn into_words(&self, at: usize) -> f32 {
        self.gfx.measure(&String::from_utf16_lossy(&self.typing.text[..at]), self.font())
    }

    /// Its words turned so that the caret stays in its field.
    fn follow_caret(&mut self) {
        let room = self.field - 2.0 * INSET;
        let caret = self.into_words(self.typing.caret);
        let whole = self.into_words(self.typing.text.len());
        self.scroll = self.scroll.min(caret).max(caret - room).min((whole - room).max(0.0)).max(0.0);
    }

    /// Where its caret is in its window (px), how tall, and its words' size (px): the input method's words put there.
    fn caret(&self) -> (POINT, i32, i32) {
        let font = self.font();
        let px = |dips: f32| (dips * self.scale).round() as i32;
        let x = self.room + PAD + INSET + self.into_words(self.typing.caret) - self.scroll;
        let top = self.room + PAD + 6.0;
        (POINT { x: px(x), y: px(top) }, px(FIELD_H - 12.0), px(font.size))
    }

    fn handle(&mut self, msg: &MSG) -> Handled {
        let (x, y) = ((msg.lParam.0 & 0xFFFF) as i16 as i32, ((msg.lParam.0 >> 16) & 0xFFFF) as i16 as i32);
        let (x, y) = (x as f32 / self.scale - self.room, y as f32 / self.scale - self.room);
        let on_plate = (0.0..=self.width()).contains(&x) && (0.0..=HEIGHT).contains(&y);
        match msg.message {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !on_plate => Handled::Left,
            // A press in its field: the caret there (with Shift, the selection to there).
            WM_LBUTTONDOWN if (PAD..=PAD + self.field).contains(&x) => {
                let at = self.unit_at(x - PAD - INSET + self.scroll);
                let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
                self.typing.go(at, shift);
                self.follow_caret();
                let _ = self.draw();
                Handled::Taken
            }
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN => Handled::Taken,
            CAPTURE_LOST => Handled::Left,
            // What was written refused, as it was tried: why said beside it, the keys its again.
            REFUSED => {
                self.trying = false;
                self.said = self.answer.lock().unwrap().take();
                self.place();
                let _ = self.draw();
                Handled::Taken
            }
            _ => Handled::Not,
        }
    }

    /// The place between its words' characters nearest `along` (DIPs into them).
    fn unit_at(&self, along: f32) -> usize {
        let mut at = 0;
        loop {
            let next = self.typing.step(at, true);
            if next == at {
                return at;
            }
            let (here, there) = (self.into_words(at), self.into_words(next));
            if along < (here + there) / 2.0 {
                return at;
            }
            at = next;
        }
    }

    fn draw(&mut self) -> bool {
        let size = ((self.rect.right - self.rect.left).max(1) as u32, (self.rect.bottom - self.rect.top).max(1) as u32);
        let Some(mut surface) = self.surface.take() else { return false };
        let mut words = std::mem::take(&mut self.words);
        self.drawn += 1;
        let width = self.width();
        let drawn = surface.draw(&self.gfx.clone(), size, self.scale, self.look, self.room, (width, HEIGHT), menu::sheet_radius(self.look, true, HEIGHT), &mut words, self.drawn, |_, _| {}, |frame, _| self.paint(frame));
        (self.surface, self.words) = (Some(surface), words);
        if let Err(error) = &drawn {
            gfx::failed("rename", &self.gfx, Some(error));
        }
        drawn.is_ok()
    }

    /// Its field (what is written, what of it is chosen, its caret), then what the keys do.
    fn paint(&self, frame: &dyn Canvas) {
        let theme = &self.theme;
        let font = self.font();
        let (accent, _) = crate::ui::view::todo::mark(theme);
        let round = match theme.skin {
            Skin::Fluent => 4.0,
            _ => 7.0,
        };
        let (x, y, w, h) = (PAD, PAD, self.field, FIELD_H);
        frame.fill_rounded(theme.track, x, y, w, h, round);
        frame.stroke_rounded(Fill::Solid(if theme.skin == Skin::Glass { theme.text2 } else { accent }), x - 0.75, y - 0.75, w + 1.5, h + 1.5, round + 0.75, 1.5);
        let line = frame.line_in(font, y, h);
        let left = x + INSET - self.scroll;
        frame.clip(x + 2.0, y, w - 4.0, h);
        let chosen = self.typing.selection();
        if !chosen.is_empty() {
            let (from, to) = (self.into_words(chosen.start), self.into_words(chosen.end));
            frame.fill(accent.alpha(0.32), left + from, y + 5.0, to - from, h - 10.0);
        }
        let text = self.typing.string();
        if text.is_empty() {
            frame.text(self.empty, font, theme.text3, left, line, w, Align::Start);
        }
        frame.text(&text, font, theme.text, left, line, self.into_words(self.typing.text.len()) + 2.0, Align::Start);
        frame.fill(theme.text, left + self.into_words(self.typing.caret), y + 6.0, 1.2, h - 12.0);
        frame.unclip();
        let hint = self.hint_font();
        let aside = if self.said.is_some() { theme.signal } else { theme.text3 };
        frame.text(self.aside(), hint, aside, x + w + GAP, frame.line_in(hint, y, h), self.width(), Align::Start);
    }
}

/// Its plate's height (DIPs).
const HEIGHT: f32 = 2.0 * PAD + FIELD_H;

#[cfg(test)]
mod tests {
    use super::*;

    /// Enter keeps a name written as the one it is shown by in quick launch alone: nothing changed where it is what the
    /// box opened with (a name told by its folder not made its own), its own name again where it is written empty or
    /// as its own, its spaces at its ends left out.
    #[test]
    fn what_is_written_is_kept_as_its_name() {
        let app = App { target: r"Q:\工作\Docs".into(), name: "Docs".into(), alias: None };
        assert_eq!(kept(&app, "工作", "工作"), None);
        assert_eq!(kept(&app, "工作", " 周报 "), Some(Some("周报".into())));
        assert_eq!(kept(&app, "周报", ""), Some(None));
        assert_eq!(kept(&app, "周报", "Docs"), Some(None));
    }

    /// A file's name opens with all but its kind's ending chosen (a folder's whole, a name with no ending, or only
    /// one, whole). What is written is tried as Explorer takes a name: written empty, or as it was, nothing is done;
    /// what a name may not be refused, said why; else the name given to be done, and its answer told.
    #[test]
    fn a_file_is_renamed_as_explorer_renames_it() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let naming = |shown: &str, folder: bool| {
            let asked = asked.clone();
            Naming::file(shown.into(), folder, Lang::Zh, move |name| {
                asked.lock().unwrap().push(name.to_string());
                if name == "在用.txt" { Err("在用".into()) } else { Ok(()) }
            })
        };
        assert_eq!(naming("报告.final.docx", false).chosen, units("报告.final"));
        assert_eq!(naming("v1.2", true).chosen, units("v1.2"));
        assert_eq!(naming(".gitignore", false).chosen, units(".gitignore"));
        assert_eq!(naming("README", false).chosen, units("README"));
        let Keeping::Tried(tried) = naming("a.txt", false).keep else { panic!("a file's name is tried") };
        assert_eq!(tried("  "), Ok(()));
        assert_eq!(tried("a.txt. "), Ok(()));
        assert!(tried("a?.txt").is_err());
        assert!(tried("con.txt").is_err());
        assert_eq!(tried(" b.txt "), Ok(()));
        assert_eq!(tried("在用.txt"), Err("在用".into()));
        assert_eq!(*asked.lock().unwrap(), ["b.txt", "在用.txt"]);
    }
}
