//! A name written in where it shows (its menu's "重命名"): a thing on the station, among what was copied or the files
//! come lately, by its line's or card's name; a tag by its choice over the page; what is kept at hand by the name under
//! its picture (where its picture alone shows, in a box beside it: see `rename`). A tag made (its menu's "新建标签") is
//! made at once, named as Windows names a new folder, its page turned to and its name written in so (see `new_tag`).
//!
//! Its window takes the keys meanwhile (see `ui::keys`): the name drawn where it shows, on a field's ground, all of it
//! chosen but a file's kind's ending (see `view::naming`). Enter, a press anywhere else on its window, or another window
//! in front keeps what is written; Esc leaves it as it was. A file's name is tried on a thread of its own (a drive slow
//! to answer never holds the panel), the keys writing nothing meanwhile: refused, why is said under it in the signal's
//! colour, the name still written there, its keys taken still where Enter tried it; where it was tried as the keys
//! were given back, a press on it takes them again, one anywhere else leaves it as it was.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow, PostMessageW, SetForegroundWindow, WM_APP, WM_CHAR, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP};

use super::typing::Typing;
use super::view::naming::{NameWritten, Named};
use super::window::Window;
use crate::station::apps::App;

/// Posted to the window a name is written in where what was written, tried (see `Tried`), was answered.
pub const ANSWERED: u32 = WM_APP + 60;

/// What `keys::LEFT` posted to its window says where Enter kept what is written (Esc posts 1, another window in front
/// 0: kept so too).
const KEEP: usize = 2;

/// How long the caret shows, and is hidden, as it blinks.
const BLINK: Duration = Duration::from_millis(530);

/// The most a name holds (UTF-16 units): what is kept at hand's, a tag's, a file's (Windows' own most).
const APP_MOST: usize = 64;
const TAG_MOST: usize = 24;
pub const FILE_MOST: usize = 255;

/// What is done with what is written, as it is kept.
pub type Kept = Box<dyn FnOnce(&str)>;

/// What is done with what is written where that may be refused (see `Naming::file`): tried on a thread of its own, the
/// name written in still till it answers: done, or why not.
pub type Tried = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What is done with what is written as it is kept: done then, or tried first (see `Tried`).
enum Keeping {
    Now(Kept),
    Tried(Tried),
}

/// What a name is written for: what it names (a tag just asked to be made by the name it is to have, its number not
/// known yet), the name it has now, the most it holds (UTF-16 units), how much of it is chosen as it is written in
/// (UTF-16 units from its start), and what is done with what is written.
pub struct Naming {
    of: Of,
    shown: String,
    most: usize,
    chosen: usize,
    keep: Keeping,
}

#[derive(Clone, PartialEq, Debug)]
enum Of {
    Named(Named),
    NewTag(String),
}

impl Naming {
    /// What it names, where that is drawn (a tag still to be made is not yet).
    pub fn named(&self) -> Option<Named> {
        match &self.of {
            Of::Named(of) => Some(*of),
            Of::NewTag(_) => None,
        }
    }

    /// `app`, kept at hand, shown by `shown` now: renamed in quick launch alone (see `app_kept`).
    pub fn app(app: App, shown: String) -> Naming {
        let of = Of::Named(Named::App(crate::station::apps::number_of(&app.target)));
        Naming { of, chosen: units(&shown), keep: Keeping::Now(app_kept(app, shown.clone())), shown, most: APP_MOST }
    }

    /// The tag `id`, named `shown` now: what is written its name (left as it is where that is nothing, or another's).
    pub fn tag(id: u32, shown: String) -> Naming {
        let keep = move |written: &str| crate::station::run::ask(crate::station::run::Ask::RenameTag(id, written.to_string()));
        Naming { of: Of::Named(Named::Tag(id)), chosen: units(&shown), shown, most: TAG_MOST, keep: Keeping::Now(Box::new(keep)) }
    }

    /// A tag just asked to be made named `name`, these files come lately of it (see `station::new_tag`): what is written
    /// its name, or where another tag has that name, what is of it of that one instead.
    pub fn new_tag(name: String, files: Vec<crate::station::recent::Key>) -> Naming {
        let was = name.clone();
        let keep = move |written: &str| crate::station::run::ask(crate::station::run::Ask::RenameNewTag(was, written.to_string(), files));
        Naming { of: Of::NewTag(name.clone()), chosen: units(&name), shown: name, most: TAG_MOST, keep: Keeping::Now(Box::new(keep)) }
    }

    /// The name of `of`, a file, or (`folder`) a folder, `shown` now: written whole, its kind's ending not chosen (as
    /// Explorer has it). What is written taken as a file's name is (see `os::name_written`) given to `rename` where it
    /// is another: done, or why not. Written empty, or as it was, it is left as it was; one a name may not be is
    /// refused, said why.
    /// (At most `most` UTF-16 units: a file's where it is, `FILE_MOST`.)
    pub fn file(of: Named, shown: String, folder: bool, most: usize, lang: super::text::Lang, rename: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Naming {
        let was = shown.clone();
        let tried = move |written: &str| match crate::os::name_written(written) {
            Ok(None) => Ok(()),
            Ok(Some(name)) if name == was => Ok(()),
            Ok(Some(name)) => rename(&name),
            Err(crate::os::NameRefused::Holds) => Err(lang.pick("名称不能包含 \\ / : * ? \" < > |", "A name can't contain \\ / : * ? \" < > |").into()),
            Err(crate::os::NameRefused::Device) => Err(lang.pick("此名称由系统保留", "This name is reserved by Windows").into()),
        };
        let chosen = if folder { units(&shown) } else { units(stem(&shown)) };
        Naming { of: Of::Named(of), shown, most, chosen, keep: Keeping::Tried(Arc::new(tried)) }
    }
}

impl Naming {
    /// The most what is written holds (UTF-16 units).
    #[cfg(test)]
    pub fn most(&self) -> usize {
        self.most
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

/// What is done with what is written for `app`, shown by `was` as it was written in: the name it is shown by in quick
/// launch made so (see `kept_as`).
pub fn app_kept(app: App, was: String) -> Kept {
    Box::new(move |written: &str| {
        if let Some(alias) = kept_as(&app, &was, written) {
            crate::station::apps::rename(app, alias);
        }
    })
}

/// What `written` makes of the name `app` is shown by in quick launch (it was written in as `was`): nothing, where it
/// is that; its own name again (none of its own), where it is written empty or as its own name; else what is written,
/// its spaces at its ends left out.
fn kept_as(app: &App, was: &str, written: &str) -> Option<Option<String>> {
    let written = written.trim();
    if written == was.trim() {
        return None;
    }
    Some((!written.is_empty() && written != app.name).then(|| written.to_string()))
}

/// The name being written: in which window, of what, what is written, what is done with it (taken as it is kept),
/// whether its window takes the keys for it, whether what is written is being tried (and its answer as it comes), why
/// it was last refused, since when the caret blinks, where it was last drawn on the screen (physical px, and the
/// screen's scale), the window in front before its window took the keys, and the tag just made found (its page to be
/// turned to: see `turn`).
struct Writing {
    owner: isize,
    of: Of,
    typing: Typing,
    keep: Option<Keeping>,
    taking: bool,
    trying: bool,
    answer: Arc<Mutex<Option<Result<(), String>>>>,
    said: Option<String>,
    blink_from: Instant,
    at: Option<(RECT, f32)>,
    before: isize,
    turn: Option<u32>,
}

thread_local! {
    static WRITING: RefCell<Option<Writing>> = const { RefCell::new(None) };
}

/// `naming` written in where it shows in `window`, which takes the keys for it: one written in before left first (kept
/// where its window still takes the keys for it, else as it was).
pub fn begin(window: &Window, naming: Naming) {
    let before = unsafe { GetForegroundWindow() };
    if let Some(was) = start(window.hwnd, naming, before) {
        super::keys::give_back(was);
    }
    super::keys::take(window.hwnd, Rc::new(answer));
    window.take_keys(true);
}

/// `naming` written in in the window `owner`, `before` in front as it takes the keys for it; one written in before left
/// first (kept where its window took the keys for it, else as it was): that window, where it took them.
fn start(owner: HWND, naming: Naming, before: HWND) -> Option<HWND> {
    let was = WRITING.with_borrow_mut(Option::take).filter(|was| was.taking).map(|was| {
        if let Some(Keeping::Now(kept)) = was.keep {
            kept(&was.typing.string());
        }
        HWND(was.owner as *mut _)
    });
    let mut typing = Typing::most(naming.most);
    typing.put(&naming.shown.encode_utf16().collect::<Vec<_>>());
    typing.go(0, false);
    typing.go(naming.chosen.min(typing.text.len()), true);
    WRITING.with_borrow_mut(|writing| {
        *writing = Some(Writing {
            owner: owner.0 as isize,
            of: naming.of,
            typing,
            keep: Some(naming.keep),
            taking: true,
            trying: false,
            answer: Arc::default(),
            said: None,
            blink_from: Instant::now(),
            at: None,
            before: if before == owner { 0 } else { before.0 as isize },
            turn: None,
        })
    });
    was
}

/// The keys given back by the window a name is written in (Esc, where `escaped`: the front given back to the window
/// that had it; else Enter, a press away from it, another window in front): what is written kept (tried, where it is a
/// file's), or, escaped, left as it was. Where its window took no keys for it (it was refused as it was tried so), it
/// is left as it was.
pub fn end(window: &Window, escaped: bool) {
    let Some(left) = leave(window.hwnd, escaped) else { return };
    if left.gave_back {
        super::keys::give_back(window.hwnd);
        window.take_keys(false);
    }
    if let Some(front) = left.front.filter(|front| unsafe { IsWindow(Some(*front)) }.as_bool()) {
        let _ = unsafe { SetForegroundWindow(front) };
    }
}

/// What the window a name was written in is to do as it is left (see `leave`): give back the keys it took for it, and
/// the front to the window that had it.
#[derive(PartialEq, Debug)]
struct Left {
    gave_back: bool,
    front: Option<HWND>,
}

/// The name written in in the window `owner` left (see `end`): what that window is to do, where one is written there.
fn leave(owner: HWND, escaped: bool) -> Option<Left> {
    let (taking, trying, before) = WRITING.with_borrow(|writing| writing.as_ref().filter(|writing| writing.owner == owner.0 as isize).map(|writing| (writing.taking, writing.trying, writing.before)))?;
    let front = HWND(before as *mut _);
    let front = (escaped && taking && !front.is_invalid()).then_some(front);
    if escaped || !taking && !trying {
        WRITING.with_borrow_mut(|writing| *writing = None);
        return Some(Left { gave_back: taking, front });
    }
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut() {
            writing.taking = false;
        }
    });
    if !trying {
        keep(owner);
    }
    Some(Left { gave_back: taking, front })
}

/// What is written in the window `owner` kept: done, where it is done at once (written in no more); else tried (see
/// `Tried`), its answer posted to it (see `answered`).
fn keep(owner: HWND) {
    let tried = WRITING.with_borrow_mut(|writing| {
        let writing = writing.as_mut()?;
        let Some(Keeping::Tried(tried)) = writing.keep.as_ref() else { return None };
        writing.trying = true;
        Some((tried.clone(), writing.typing.string(), writing.answer.clone()))
    });
    match tried {
        Some((tried, written, answer)) => {
            let window = owner.0 as isize;
            std::thread::spawn(move || {
                *answer.lock().unwrap() = Some(tried(&written));
                let _ = unsafe { PostMessageW(Some(HWND(window as *mut _)), ANSWERED, WPARAM(0), LPARAM(0)) };
            });
        }
        None => {
            if let Some(Writing { keep: Some(Keeping::Now(kept)), typing, .. }) = WRITING.with_borrow_mut(Option::take) {
                kept(&typing.string());
            }
        }
    }
}

/// What was written in `window`, tried, answered (see `ANSWERED`): done, written in no more (its keys given back where
/// it took them); refused, why said, written in still.
pub fn answered(window: &Window) {
    if heard(window.hwnd) == Some(true) {
        super::keys::give_back(window.hwnd);
        window.take_keys(false);
    }
}

/// The answer to what was tried in the window `owner` heard (see `answered`): done, whether it took the keys for it.
fn heard(owner: HWND) -> Option<bool> {
    let done = WRITING.with_borrow_mut(|writing| {
        let writing = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize && writing.trying)?;
        let answer = writing.answer.lock().unwrap().take()?;
        writing.trying = false;
        match answer {
            Ok(()) => Some(writing.taking),
            Err(why) => {
                writing.said = Some(why);
                writing.blink_from = Instant::now();
                None
            }
        }
    });
    if done.is_some() {
        WRITING.with_borrow_mut(|writing| *writing = None);
    }
    done
}

/// A press on the name written in where it shows in `window`, `into` DIPs across its field (with Shift, the selection
/// to there): the caret put there, the keys taken again where they were given back.
pub fn press(window: &Window, gfx: &super::gfx::Gfx, into: f32) {
    let owner = window.hwnd;
    let Some(drawn) = super::view::naming::drawn() else { return };
    let Some(text) = WRITING.with_borrow(|writing| writing.as_ref().filter(|writing| writing.owner == owner.0 as isize).map(|writing| writing.typing.string())) else { return };
    let layout = gfx.layout(&text, drawn.font, 10_000.0, super::canvas::Align::Start);
    let at = super::typing::at_point(&layout, (into - drawn.words).max(0.0), drawn.font.size / 2.0);
    let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
    if put_caret(owner, at, shift, unsafe { GetForegroundWindow() }) {
        super::keys::take(owner, Rc::new(answer));
        window.take_keys(true);
    }
}

/// The caret of the name written in in the window `owner` put at `at` (with `shift`, the selection to there), unless
/// what is written is being tried: whether that window is to take the keys for it again (`before` in front as it does).
fn put_caret(owner: HWND, at: usize, shift: bool, before: HWND) -> bool {
    WRITING.with_borrow_mut(|writing| {
        let Some(writing) = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize && !writing.trying) else { return false };
        writing.typing.go(at, shift);
        writing.blink_from = Instant::now();
        if writing.taking {
            return false;
        }
        writing.taking = true;
        writing.before = if before == owner { 0 } else { before.0 as isize };
        true
    })
}

/// The window `owner` gone: what is written in it kept (tried, where it is a file's, its answer heard by none).
pub fn went(owner: HWND) {
    let Some(writing) = WRITING.with_borrow_mut(|writing| writing.take_if(|writing| writing.owner == owner.0 as isize)) else { return };
    if !writing.taking || writing.trying {
        return;
    }
    match writing.keep {
        Some(Keeping::Now(kept)) => kept(&writing.typing.string()),
        Some(Keeping::Tried(tried)) => {
            let written = writing.typing.string();
            std::thread::spawn(move || {
                let _ = tried(&written);
            });
        }
        None => {}
    }
}

/// Whether the window `owner` takes the keys for a name written in it now.
pub fn writing(owner: HWND) -> bool {
    WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.taking && writing.owner == owner.0 as isize))
}

/// Whether the window `owner` is to stay as it is for a name written in it (taking the keys, being tried, or refused
/// and said so).
pub fn holds(owner: HWND) -> bool {
    WRITING.with_borrow(|writing| writing.as_ref().is_some_and(|writing| writing.owner == owner.0 as isize))
}

/// The tag just made whose name is written in in the window `owner`, as the station now shows it among `tags`: found
/// at last, its number, its page to be turned to there (once).
pub fn turn(owner: HWND, tags: &[crate::station::shelf::Tag]) -> Option<u32> {
    WRITING.with_borrow_mut(|writing| {
        let writing = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize)?;
        if let Of::NewTag(name) = &writing.of {
            let id = tags.iter().find(|tag| tag.name == *name)?.id;
            writing.of = Of::Named(Named::Tag(id));
            writing.turn = Some(id);
        }
        writing.turn.take()
    })
}

/// The name written in where it shows in the window `owner` as it shows at `now` (none where its tag is yet to be
/// found), and when it is next to be drawn for its caret.
pub fn shown(owner: HWND, now: Instant) -> (Option<NameWritten>, Option<Instant>) {
    WRITING.with_borrow(|writing| {
        let Some(writing) = writing.as_ref().filter(|writing| writing.owner == owner.0 as isize) else { return (None, None) };
        let Of::Named(of) = writing.of.clone() else { return (None, None) };
        let blinks = (now.duration_since(writing.blink_from).as_millis() / BLINK.as_millis()) as u32;
        let blinking = writing.taking && !writing.trying;
        let next = blinking.then(|| writing.blink_from + BLINK * (blinks + 1));
        let typing = &writing.typing;
        let shown = NameWritten { of, text: typing.text.clone(), caret: typing.caret, anchor: typing.anchor, caret_on: blinking && blinks.is_multiple_of(2), said: writing.said.clone() };
        (Some(shown), next)
    })
}

/// Where the name written in where it shows in the window `owner` was just drawn on the screen (physical px), at
/// `scale`: the input method's words put at its caret.
pub fn placed(owner: HWND, field: RECT, scale: f32) {
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut().filter(|writing| writing.owner == owner.0 as isize) {
            writing.at = Some((field, scale));
        }
    });
}

/// A message for the window a name is written in while it takes the keys (see `ui::keys::Answer`): Enter keeps what is
/// written (told as `keys::LEFT` with `KEEP`; one to be tried, tried while the keys stay), Esc is not its (the keys
/// given back leave it), the rest write in it: the caret moved (with Shift, the selection), words erased, all chosen,
/// a line pasted, what is typed put in. Nothing is written while what was is tried.
fn answer(hwnd: HWND, message: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<Option<LRESULT>> {
    match message {
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            if key == VK_ESCAPE {
                return None;
            }
            if key == VK_RETURN {
                enter(hwnd);
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
            place_ime(hwnd);
            Some(None)
        }
        _ => None,
    }
}

/// Enter pressed in the window `hwnd`: what is written kept as its keys are given back (`KEEP`), or, one to be tried,
/// tried while it takes them still (once at a time).
fn enter(hwnd: HWND) {
    let tried = WRITING.with_borrow(|writing| writing.as_ref().map(|writing| (matches!(writing.keep, Some(Keeping::Tried(_))), writing.trying)));
    match tried {
        Some((true, false)) => keep(hwnd),
        Some((false, _)) => {
            let _ = unsafe { PostMessageW(Some(hwnd), super::keys::LEFT, WPARAM(KEEP), LPARAM(0)) };
        }
        _ => {}
    }
}

/// `change` made to what is written (none while it is tried), the caret shown at once: why it was refused no longer
/// said once it changes.
fn written(change: impl FnOnce(&mut Typing)) {
    WRITING.with_borrow_mut(|writing| {
        if let Some(writing) = writing.as_mut().filter(|writing| !writing.trying) {
            let was = writing.typing.text.clone();
            change(&mut writing.typing);
            if writing.typing.text != was {
                writing.said = None;
            }
            writing.blink_from = Instant::now();
        }
    });
}

/// The input method's words and choices put at the caret of the name written in `hwnd`, where it was last drawn, as
/// its words.
fn place_ime(hwnd: HWND) {
    let Some((field, scale)) = WRITING.with_borrow(|writing| writing.as_ref().and_then(|writing| writing.at)) else { return };
    let Some(drawn) = super::view::naming::drawn() else { return };
    let (x, y, tall) = drawn.caret;
    let mut corner = POINT { x: field.left, y: field.top };
    let _ = unsafe { windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut corner) };
    let px = |dips: f32| (dips * scale).round() as i32;
    super::typing::place_ime(hwnd, POINT { x: corner.x + px(x), y: corner.y + px(y) }, px(y + tall) - px(y), "Microsoft YaHei UI", px(drawn.font.size));
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::text::Lang;
    use std::cell::RefCell;

    /// A tag's name, `shown` now, its first `chosen` units chosen as it is written in, what is written kept in `kept`.
    fn naming_now(shown: &str, chosen: usize, kept: &Rc<RefCell<Vec<String>>>) -> Naming {
        let kept = kept.clone();
        Naming { of: Of::Named(Named::Tag(7)), shown: shown.into(), most: TAG_MOST, chosen, keep: Keeping::Now(Box::new(move |written: &str| kept.borrow_mut().push(written.into()))) }
    }

    fn typed(words: &str) {
        written(|typing| typing.put(&words.encode_utf16().collect::<Vec<_>>()));
    }

    /// The answer to what was tried, come (its thread done), waited for.
    fn answer_come() {
        let answer = WRITING.with_borrow(|writing| writing.as_ref().unwrap().answer.clone());
        let until = Instant::now() + Duration::from_secs(5);
        while answer.lock().unwrap().is_none() {
            assert!(Instant::now() < until, "no answer");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A name written in where it shows opens chosen as asked, typed over; Enter, a press away from it or another window
    /// in front keeps what is written, its window giving back the keys; Esc leaves it as it was, the front given back
    /// to the window that had it. Written in another window, the one before is kept, that window giving back its keys.
    #[test]
    fn a_name_written_is_kept_as_its_keys_go_and_left_by_esc() {
        let (owner, before, other) = (HWND(0x8181 as *mut _), HWND(0x8282 as *mut _), HWND(0x8383 as *mut _));
        WRITING.with_borrow_mut(|writing| *writing = None);
        let kept = Rc::new(RefCell::new(Vec::new()));
        assert_eq!(start(owner, naming_now("新建标签", 4, &kept), before), None);
        let (shown, next) = shown(owner, Instant::now());
        let shown = shown.expect("drawn where it shows");
        assert_eq!((shown.of, shown.caret, shown.anchor, shown.caret_on), (Named::Tag(7), 4, 0, true));
        assert!(next.is_some(), "its caret blinks");
        assert_eq!(super::shown(other, Instant::now()).0, None, "another window's");
        typed("工作");
        assert!(writing(owner) && holds(owner) && !writing(other));
        assert_eq!(leave(other, false), None, "another window's");
        assert_eq!(leave(owner, false), Some(Left { gave_back: true, front: None }));
        assert_eq!(*kept.borrow(), ["工作"]);
        assert!(!holds(owner));
        // Esc.
        start(owner, naming_now("工作", 2, &kept), before);
        typed("旅行");
        assert_eq!(leave(owner, true), Some(Left { gave_back: true, front: Some(before) }));
        assert_eq!(kept.borrow().len(), 1);
        assert!(!holds(owner));
        // Written in another window meanwhile.
        start(owner, naming_now("工作", 2, &kept), before);
        typed("资料");
        assert_eq!(start(other, naming_now("旅行", 2, &kept), before), Some(owner));
        assert_eq!(*kept.borrow(), ["工作", "资料"]);
        assert!(holds(other) && !holds(owner));
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// A file's name is tried as Enter is pressed, the keys writing nothing meanwhile: refused, it stays written with
    /// why (till what is written changes), its window taking the keys still. Tried as the keys are given back and
    /// refused, it stays written with why, its window taking none: a press on it takes them again, a press away leaves
    /// it as it was. Done, it is written in no more, its window giving back the keys it took.
    #[test]
    fn a_file_s_name_refused_stays_written_with_why() {
        let (owner, before) = (HWND(0x9191 as *mut _), HWND(0x9292 as *mut _));
        WRITING.with_borrow_mut(|writing| *writing = None);
        let file = || {
            let tried = |written: &str| if written == "在用.txt" { Err("此项目正在使用".to_string()) } else { Ok(()) };
            Naming { of: Of::Named(Named::Item(3)), shown: "a.txt".into(), most: FILE_MOST, chosen: 1, keep: Keeping::Tried(Arc::new(tried)) }
        };
        let said = || super::shown(owner, Instant::now()).0.and_then(|shown| shown.said);
        let words = || super::shown(owner, Instant::now()).0.map(|shown| String::from_utf16_lossy(&shown.text));
        start(owner, file(), before);
        typed("在用");
        enter(owner);
        typed("x");
        assert_eq!(words().as_deref(), Some("在用.txt"), "nothing written while it is tried");
        answer_come();
        assert_eq!(heard(owner), None, "refused");
        assert_eq!(said().as_deref(), Some("此项目正在使用"));
        assert!(writing(owner), "its keys taken still");
        typed("");
        assert_eq!(said().as_deref(), Some("此项目正在使用"), "nothing changed");
        written(|typing| typing.go(0, false));
        typed("x");
        assert_eq!(said(), None, "what is written changed");
        written(|typing| typing.erase(false));
        // Tried as its keys are given back.
        assert_eq!(leave(owner, false), Some(Left { gave_back: true, front: None }));
        answer_come();
        assert_eq!(heard(owner), None);
        assert!(said().is_some() && !writing(owner) && holds(owner));
        assert!(put_caret(owner, 1, false, before), "a press on it takes the keys again");
        assert!(writing(owner));
        assert!(!put_caret(owner, 2, false, before), "taken already");
        assert_eq!(leave(owner, false), Some(Left { gave_back: true, front: None }));
        answer_come();
        assert_eq!(heard(owner), None);
        assert_eq!(leave(owner, false), Some(Left { gave_back: false, front: None }), "a press away: left as it was");
        assert!(!holds(owner));
        // Done.
        start(owner, file(), before);
        typed("b");
        enter(owner);
        answer_come();
        assert_eq!(heard(owner), Some(true), "its keys given back");
        assert!(!holds(owner));
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// A tag just made is drawn where it shows once the station shows it, its page turned to then (once): till then it
    /// is written in (what is typed kept) but shown nowhere.
    #[test]
    fn a_tag_just_made_is_turned_to_as_it_is_found() {
        let owner = HWND(0xA1A1 as *mut _);
        WRITING.with_borrow_mut(|writing| *writing = None);
        start(owner, Naming::new_tag("新建标签".into(), Vec::new()), HWND::default());
        typed("工");
        let tag = |id: u32, name: &str| crate::station::shelf::Tag { id, name: name.into() };
        assert_eq!(turn(owner, &[tag(1, "工作")]), None);
        assert_eq!(super::shown(owner, Instant::now()).0, None);
        assert_eq!(turn(owner, &[tag(1, "工作"), tag(4, "新建标签")]), Some(4));
        assert_eq!(turn(owner, &[tag(1, "工作"), tag(4, "新建标签")]), None, "once");
        let shown = super::shown(owner, Instant::now()).0.unwrap();
        assert_eq!((shown.of, String::from_utf16_lossy(&shown.text)), (Named::Tag(4), "工".to_string()));
        WRITING.with_borrow_mut(|writing| *writing = None);
    }

    /// What is kept at hand keeps a name written as the one it is shown by in quick launch alone: nothing changed where
    /// it is what it was written in as (a name told by its folder not made its own), its own name again where it is
    /// written empty or as its own, its spaces at its ends left out.
    #[test]
    fn what_is_written_is_kept_as_its_name() {
        let app = App { target: r"Q:\工作\Docs".into(), name: "Docs".into(), alias: None };
        assert_eq!(kept_as(&app, "工作", "工作"), None);
        assert_eq!(kept_as(&app, "工作", " 周报 "), Some(Some("周报".into())));
        assert_eq!(kept_as(&app, "周报", ""), Some(None));
        assert_eq!(kept_as(&app, "周报", "Docs"), Some(None));
    }

    /// A file's name is written in with all but its kind's ending chosen (a folder's whole, a name with no ending, or
    /// only one, whole). What is written is tried as Explorer takes a name: written empty, or as it was, nothing is
    /// done; what a name may not be refused, said why; else the name given to be done, and its answer told.
    #[test]
    fn a_file_is_renamed_as_explorer_renames_it() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let naming = |shown: &str, folder: bool| {
            let asked = asked.clone();
            Naming::file(Named::Item(1), shown.into(), folder, FILE_MOST, Lang::Zh, move |name| {
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
