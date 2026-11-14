//! What is on the station sent to devices nearby (see `nearby`, `docs/nearby.md`): while something is dragged off a
//! station, the devices seen nearby rise at its foot (the strip, drawn by `view` from `shown`), each a place to drop on
//! (a window catching drops over it on the station's thread, see `catch` and `ToDevice`). Let go on one, what is dragged
//! is sent to it, the station's own files of it held till the sending is over (see `run::drag_off`), and the device
//! shows how it goes: waiting on its user, a ring of how much has gone, a tick. The strip goes down a moment after all
//! it sent is over, the drag over.
//!
//! What may not leave this PC (see `shelf::Item::stays_here`) is sent nowhere: the devices are dimmed as it is dragged,
//! and the plate under the pointer says why over one.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{implement, Ref, Result};
use windows::Win32::Foundation::{POINT, POINTL, RECT};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DROPIMAGE_COPY, DROPIMAGE_INVALID, DROPIMAGE_NONE};

use super::clipboard::Content;
use crate::nearby::{Device, Sending};

/// How long the strip takes to rise, and to go down.
const RISE: Duration = Duration::from_millis(260);
const FALL: Duration = Duration::from_millis(220);
/// How long a device takes to grow as the pointer comes over it, and to shrink back as it leaves; how much larger it
/// grows.
const GROW: Duration = Duration::from_millis(220);
const SHRINK: Duration = Duration::from_millis(180);
pub const GROWN: f32 = 0.18;
/// How long a device's tick takes to come, sent.
const TICK: Duration = Duration::from_millis(300);
/// How long what became of a sending is shown after it is over: sent, and not.
const STAYS_SENT: Duration = Duration::from_millis(1100);
const STAYS_NOT: Duration = Duration::from_millis(2400);
/// When the devices nearby are asked to say themselves after the strip rises (as LocalSend asks: one asking may be
/// lost on the network).
const ANNOUNCE: [Duration; 3] = [Duration::from_millis(100), Duration::from_millis(500), Duration::from_millis(2000)];

/// A device on the strip: the device, and the sending to it shown (its number, and when it was over).
#[derive(Clone, Debug)]
struct Chip {
    device: Device,
    job: Option<(u64, Option<Instant>)>,
}

/// The strip: the drag it rose for (see `drag::announce`), when it rose (as far up as it was then, rising again as it
/// went down), the devices on it, whether what is dragged may not leave this PC, the device the pointer is over and
/// the one it left (each by its key, see `key_of`, and since when), when it began to go down, when the devices were
/// asked to say themselves and how many times since, where on the screen what was let go on a device went (the drag,
/// its mark's middle), and what it showed as it was last drawn.
struct Strip {
    drag: u64,
    rose: Instant,
    chips: Vec<Chip>,
    stays_here: bool,
    over: Option<(u64, Instant)>,
    left: Option<(u64, Instant)>,
    falls: Option<Instant>,
    asked: Instant,
    announced: usize,
    into: Option<(u64, POINT)>,
    drawn: Option<Shown>,
}

/// What was let go on a device, for the station's thread to send (see `taken`): the drag, the sending's number, and
/// the device's fingerprint.
struct Dropped {
    drag: u64,
    job: u64,
    to: String,
}

static STRIP: Mutex<Option<Strip>> = Mutex::new(None);
static DROPPED: Mutex<Option<Dropped>> = Mutex::new(None);

/// The strip as it is drawn: how far up it is (0 down, 1 up), whether what is dragged now may be let go on its
/// devices (a drag under way, what is dragged free to leave), whether it may not (dimmed), and its devices.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub up: f32,
    pub open: bool,
    pub stays_here: bool,
    pub chips: Vec<ChipShown>,
}

/// A device as the strip draws it: what it is known by (see `key_of`), what it calls itself, its kind (as LocalSend
/// names kinds), how far grown (0 to 1, past 1 as it springs), whether lit (the pointer over it, what is dragged to be
/// let go there), and how a sending to it goes, with how far its tick has come.
#[derive(Clone, Debug, PartialEq)]
pub struct ChipShown {
    pub key: u64,
    pub alias: String,
    pub kind: Option<String>,
    pub grown: f32,
    pub lit: bool,
    pub sending: Option<(Sending, f32)>,
}

/// What a device is known by on the strip, the same as long as it is the same device: its fingerprint's, told without
/// case.
pub fn key_of(fingerprint: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    fingerprint.to_ascii_lowercase().hash(&mut hasher);
    hasher.finish()
}

/// How far `t` (0 to 1) of the way is eased out: fast, then slow.
fn eased(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// How far `t` (0 to 1) of the way is sprung: past where it goes, and back (as the mock-up's spring).
fn sprung(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// How far into `span` `since` is, at `now` (0 to 1).
fn into(since: Instant, span: Duration, now: Instant) -> f32 {
    (now.saturating_duration_since(since).as_secs_f32() / span.as_secs_f32()).clamp(0.0, 1.0)
}

/// How long what became of a sending is shown after it is over.
fn stays(state: &Sending) -> Duration {
    if *state == Sending::Sent {
        STAYS_SENT
    } else {
        STAYS_NOT
    }
}

impl Strip {
    fn new(drag: u64, now: Instant) -> Strip {
        Strip { drag, rose: now, chips: Vec::new(), stays_here: false, over: None, left: None, falls: None, asked: now, announced: 0, into: None, drawn: None }
    }

    /// How far up it is at `now`.
    fn up(&self, now: Instant) -> f32 {
        match self.falls {
            Some(falls) => 1.0 - eased(into(falls, FALL, now)),
            None => eased(into(self.rose, RISE, now)),
        }
    }

    /// How a sending to `chip` goes, as `sendings` say (one not told of yet is waiting).
    fn sending(chip: &Chip, sendings: &std::collections::HashMap<u64, Sending>) -> Option<Sending> {
        chip.job.map(|(job, _)| sendings.get(&job).cloned().unwrap_or(Sending::Waiting))
    }

    /// What it shows at `now`, a drag of it under way or not (`open`), sendings going as `sendings` say.
    fn shown(&self, now: Instant, open: bool, sendings: &std::collections::HashMap<u64, Sending>) -> Shown {
        let chips = self
            .chips
            .iter()
            .map(|chip| {
                let key = key_of(&chip.device.fingerprint);
                let grown = match (self.over, self.left) {
                    (Some((over, since)), _) if over == key => sprung(into(since, GROW, now)),
                    (_, Some((left, since))) if left == key => 1.0 - eased(into(since, SHRINK, now)),
                    _ => 0.0,
                };
                let sending = Self::sending(chip, sendings).map(|state| {
                    let ticked = chip.job.and_then(|(_, ended)| ended).filter(|_| state == Sending::Sent).map_or(0.0, |ended| sprung(into(ended, TICK, now)));
                    (state, ticked)
                });
                ChipShown { key, alias: chip.device.alias.clone(), kind: chip.device.kind.clone(), grown, lit: open && !self.stays_here && self.over.is_some_and(|(over, _)| over == key), sending }
            })
            .collect();
        Shown { up: self.up(now), open: open && !self.stays_here, stays_here: open && self.stays_here, chips }
    }
}

/// The strip a step on to `now` (on the panel's thread, each frame it ticks): risen as something is dragged off a
/// station where the helper is there to send, its devices those seen nearby (and those sent to, gone or not), each
/// sending's end marked, gone down once the drag and all it sent are over. Whether what it shows changed (to be drawn
/// again), and when it is to be looked at next at the latest (none: when told of something).
pub fn tick(now: Instant) -> (bool, Option<Duration>) {
    let going = super::drag::going();
    let nearby = crate::nearby::shown();
    let under_way = going.ended.is_none();
    let mut held = STRIP.lock().unwrap();
    if under_way && nearby.there && held.as_ref().is_none_or(|strip| strip.drag != going.drag) {
        match held.as_mut() {
            // (Up still, or going down: up again, from as far up as it is, for this drag too.)
            Some(strip) => {
                // (Risen `t` of its time, it is `1 - (1 - t)³` up: see `eased`.)
                let up = strip.up(now).clamp(0.0, 1.0);
                strip.rose = now.checked_sub(RISE.mul_f32(1.0 - (1.0 - up).cbrt())).unwrap_or(now);
                (strip.drag, strip.falls, strip.asked, strip.announced, strip.into) = (going.drag, None, now, 0, None);
            }
            None => *held = Some(Strip::new(going.drag, now)),
        }
    }
    let Some(strip) = held.as_mut() else { return (false, None) };
    let open = under_way && going.drag == strip.drag;
    if open {
        strip.stays_here = going.stays_here;
    }
    // Its devices: those seen nearby, in the order they were first seen; one gone kept while it is sent to.
    strip.chips.retain(|chip| chip.job.is_some() || nearby.devices.iter().any(|device| device.fingerprint == chip.device.fingerprint));
    for device in &nearby.devices {
        match strip.chips.iter_mut().find(|chip| chip.device.fingerprint == device.fingerprint) {
            Some(chip) => chip.device = device.clone(),
            None => strip.chips.push(Chip { device: device.clone(), job: None }),
        }
    }
    // Each sending's end, as it is told: shown so a while.
    let mut due: Option<Duration> = None;
    let mut sooner = |after: Duration| due = Some(due.map_or(after, |due| due.min(after)));
    let mut lingered = true;
    for chip in &mut strip.chips {
        let Some(state) = Strip::sending(chip, &nearby.sendings) else { continue };
        let Some((job, ended)) = chip.job.as_mut() else { continue };
        if !state.over() {
            lingered = false;
            continue;
        }
        let ended = *ended.get_or_insert(now);
        let left = stays(&state).saturating_sub(now.saturating_duration_since(ended));
        if left.is_zero() {
            // (Over and shown long enough: while something is dragged still, the device as it is again.)
            if open {
                crate::nearby::forget(*job);
                chip.job = None;
            }
        } else {
            lingered = false;
            sooner(left);
        }
    }
    if !open && strip.falls.is_none() && lingered {
        strip.falls = Some(now);
    }
    if let Some(falls) = strip.falls {
        if now.saturating_duration_since(falls) >= FALL {
            for (job, _) in strip.chips.iter().filter_map(|chip| chip.job) {
                crate::nearby::forget(job);
            }
            *held = None;
            return (true, None);
        }
    }
    // (Asked as it rises, while it is dragged.)
    while open && strip.announced < ANNOUNCE.len() && now.saturating_duration_since(strip.asked) >= ANNOUNCE[strip.announced] {
        crate::nearby::announce();
        strip.announced += 1;
    }
    if open && strip.announced < ANNOUNCE.len() {
        sooner(ANNOUNCE[strip.announced].saturating_sub(now.saturating_duration_since(strip.asked)));
    }
    let shown = strip.shown(now, open, &nearby.sendings);
    let changed = strip.drawn.as_ref() != Some(&shown);
    strip.drawn = Some(shown);
    // (Moving: drawn again with the screen.)
    if changed {
        sooner(Duration::ZERO);
    }
    (changed, due)
}

/// What the strip shows now: none where it is down.
pub fn shown() -> Option<Shown> {
    #[cfg(any(test, feature = "studio"))]
    if let Some(shown) = PRETENDED.with_borrow(Clone::clone) {
        return Some(shown);
    }
    STRIP.lock().unwrap().as_ref().and_then(|strip| strip.drawn.clone())
}

#[cfg(any(test, feature = "studio"))]
thread_local! {
    /// What this thread draws the strip as, pretended (see `pretend`).
    static PRETENDED: RefCell<Option<Shown>> = const { RefCell::new(None) };
}

/// The strip drawn on this thread as `shown` (none: as it is), to picture it.
#[cfg(any(test, feature = "studio"))]
pub fn pretend(shown: Option<Shown>) {
    PRETENDED.with_borrow_mut(|pretended| *pretended = shown);
}

/// The pointer over the device `key` on the strip as something is dragged (none: off it, where it was over `key`).
fn over(key: u64, on: bool, now: Instant) {
    let mut held = STRIP.lock().unwrap();
    let Some(strip) = held.as_mut() else { return };
    let was = strip.over.map(|(over, _)| over);
    if on && was != Some(key) {
        strip.left = was.map(|was| (was, now));
        strip.over = Some((key, now));
    } else if !on && was == Some(key) {
        strip.left = Some((key, now));
        strip.over = None;
    }
    drop(held);
    crate::app().controller.wake();
}

/// What is dragged let go on the device `key` on the strip (its mark's middle at `at` on the screen): sent to it as a
/// sending numbered now, shown on it at once; the station's thread sends it once the drag is over (see `taken`). None
/// where that device is no longer there.
fn dropped(key: u64, at: POINT) -> bool {
    let drag = super::drag::going().drag;
    let mut held = STRIP.lock().unwrap();
    let Some(strip) = held.as_mut() else { return false };
    let Some(chip) = strip.chips.iter_mut().find(|chip| key_of(&chip.device.fingerprint) == key) else { return false };
    let job = crate::nearby::job();
    chip.job = Some((job, None));
    strip.into = Some((drag, at));
    *DROPPED.lock().unwrap() = Some(Dropped { drag, job, to: chip.device.fingerprint.clone() });
    true
}

/// The sending to the device `key` on the strip cancelled, where one goes on (pressed there): whether one did.
pub fn cancel(key: u64) -> bool {
    let sendings = crate::nearby::shown().sendings;
    let held = STRIP.lock().unwrap();
    let job = held.as_ref().and_then(|strip| strip.chips.iter().find(|chip| key_of(&chip.device.fingerprint) == key)).and_then(|chip| chip.job).map(|(job, _)| job);
    drop(held);
    match job.filter(|job| sendings.get(job).is_none_or(|state| !state.over())) {
        Some(job) => crate::nearby::cancel(job),
        None => false,
    }
}

/// Where on the screen what drag `drag` let go on a device went (its mark's middle), where it was let go on one.
pub fn into_device(drag: u64) -> Option<POINT> {
    STRIP.lock().unwrap().as_ref().and_then(|strip| strip.into.filter(|(dropped, _)| *dropped == drag).map(|(_, at)| at))
}

/// What the drag just over was let go on a device to be sent as (on the station's thread, once): the sending's number
/// and the device's fingerprint. None where it was not.
pub fn taken() -> Option<(u64, String)> {
    let drag = super::drag::going().drag;
    DROPPED.lock().unwrap().take().filter(|dropped| dropped.drag == drag).map(|dropped| (dropped.job, dropped.to))
}

/// Where what is sent but is not a file yet (words, a link, a picture with no file of its own) is written as one, a
/// folder for each sending: among Windows' temporary ones (in a test, its own).
pub fn folder() -> PathBuf {
    #[cfg(not(test))]
    return std::env::temp_dir().join("Glance").join("sending");
    #[cfg(test)]
    return crate::settings::local_dir().join("sending");
}

/// The folder sending `job` writes in (see `folder`).
pub fn folder_of(job: u64) -> PathBuf {
    folder().join(job.to_string())
}

/// What `given` sends: each file where it is, and the name the device is told; a folder as each file in it (named by
/// its place there, "folder/inner/file", as LocalSend names them; what in it only points elsewhere left out: a link
/// may point back up into it); what is not a file yet written as one in `folder` first (as it is dragged off, see
/// `drag::as_paths`). None of it where something of it could not be written.
pub fn outgoing(given: &[Content], folder: &Path) -> Vec<(PathBuf, String)> {
    let Some(paths) = super::drag::as_paths(given, folder) else { return Vec::new() };
    let mut files = Vec::new();
    for path in paths {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => within(&path, &name, &mut files),
            Ok(_) => files.push((path, name)),
            Err(_) => {}
        }
    }
    files
}

/// Each file within the folder `folder` (named `named`) added to `files`, named by its place there.
fn within(folder: &Path, named: &str, files: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let Ok(kind) = entry.file_type() else { continue };
        let name = format!("{named}/{}", entry.file_name().to_string_lossy());
        if kind.is_symlink() {
            continue;
        } else if kind.is_dir() {
            within(&entry.path(), &name, files);
        } else if kind.is_file() {
            files.push((entry.path(), name));
        }
    }
}

/// A device on the strip as a place to drop on, by its key (see `key_of`), its mark's middle on the screen at `at`:
/// what is dragged off the station is let go there to be sent to it (what may not leave this PC, refused, saying why).
/// It says so on what is dragged (`drag::describe`), as Explorer says where a drop goes.
#[implement(IDropTarget)]
pub struct ToDevice {
    key: u64,
    at: POINT,
    takes: Cell<bool>,
    data: RefCell<Option<IDataObject>>,
}

impl ToDevice {
    /// One over `rect` on the screen.
    pub fn over(key: u64, rect: RECT) -> IDropTarget {
        let at = POINT { x: (rect.left + rect.right) / 2, y: (rect.top + rect.bottom) / 2 };
        ToDevice { key, at, takes: Cell::new(false), data: RefCell::new(None) }.into()
    }
}

/// What the device `key` on the strip calls itself, where it is on it.
fn alias_of(key: u64) -> Option<String> {
    STRIP.lock().unwrap().as_ref()?.chips.iter().find(|chip| key_of(&chip.device.fingerprint) == key).map(|chip| chip.device.alias.clone())
}

impl IDropTarget_Impl for ToDevice_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let going = super::drag::going();
        let under_way = going.ended.is_none();
        let alias = alias_of(self.key);
        self.takes.set(under_way && !going.stays_here && alias.is_some());
        over(self.key, true, std::time::Instant::now());
        if let Some(data) = data.as_ref() {
            let lang = crate::update::language_in(&crate::app().settings.lock().unwrap().view);
            match alias {
                Some(alias) if self.takes.get() => super::drag::describe(data, DROPIMAGE_COPY, lang.pick("发送到 %1", "Send to %1"), &alias),
                _ if under_way && going.stays_here => super::drag::describe(data, DROPIMAGE_NONE, lang.pick("此内容不可发送到其他设备", "This may not leave this PC"), ""),
                _ => {}
            }
            *self.data.borrow_mut() = Some(data.clone());
        }
        self.answer(effect);
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.answer(effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.gone();
        Ok(())
    }

    fn Drop(&self, _: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let offered = unsafe { *effect };
        let sent = self.takes.get() && offered.0 & DROPEFFECT_COPY.0 != 0 && dropped(self.key, self.at);
        self.gone();
        unsafe { *effect = if sent { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }
}

impl ToDevice_Impl {
    /// A copy where it takes what is dragged and the station lets it be copied; else nothing.
    fn answer(&self, effect: *mut DROPEFFECT) {
        let offered = unsafe { *effect };
        let copies = self.takes.get() && offered.0 & DROPEFFECT_COPY.0 != 0;
        unsafe { *effect = if copies { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
    }

    /// The pointer gone off it, or what is dragged let go: off it, and what it said taken back.
    fn gone(&self) {
        over(self.key, false, std::time::Instant::now());
        if let Some(data) = self.data.borrow_mut().take() {
            super::drag::describe(&data, DROPIMAGE_INVALID, "", "");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder is sent as its files, each named by its place in it, what in it only points elsewhere left out; a file
    /// as itself; words as a text file named by them.
    #[test]
    fn what_is_sent_is_each_file_named_by_its_place() {
        let root = crate::settings::local_dir().join("send-outgoing");
        let _ = std::fs::remove_dir_all(&root);
        let trip = root.join("trip");
        std::fs::create_dir_all(trip.join("day 2")).unwrap();
        std::fs::write(trip.join("a.jpg"), b"a").unwrap();
        std::fs::write(trip.join("day 2").join("b.jpg"), b"b").unwrap();
        let alone = root.join("alone.pdf");
        std::fs::write(&alone, b"c").unwrap();
        let given = [Content::Files(vec![trip.clone(), alone.clone()]), Content::Text("见面地点\n东门".into())];
        let files = outgoing(&given, &root.join("written"));
        let names: Vec<&str> = files.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, ["trip/a.jpg", "trip/day 2/b.jpg", "alone.pdf", "见面地点.txt"]);
        assert_eq!(files[1].0, trip.join("day 2").join("b.jpg"));
        assert!(files[3].0.starts_with(root.join("written")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A device grows as the pointer comes over it (past its size, and back), and shrinks back as it leaves; it is
    /// lit only where what is dragged may be let go there.
    #[test]
    fn a_device_grows_under_the_pointer() {
        let now = Instant::now();
        let device = Device { fingerprint: "AB".into(), alias: "Galaxy Z Fold6".into(), model: None, kind: Some("mobile".into()) };
        let mut strip = Strip::new(1, now);
        strip.chips.push(Chip { device, job: None });
        let key = key_of("ab");
        strip.over = Some((key, now));
        let sendings = std::collections::HashMap::new();
        let at = |after: u64, open: bool, strip: &Strip| strip.shown(now + Duration::from_millis(after), open, &sendings).chips[0].clone();
        assert_eq!(at(0, true, &strip).grown, 0.0);
        assert!(at(150, true, &strip).grown > 1.0, "past its size as it springs");
        assert!((at(400, true, &strip).grown - 1.0).abs() < 1e-4);
        assert!(at(400, true, &strip).lit);
        strip.stays_here = true;
        assert!(!at(400, true, &strip).lit, "dimmed: what is dragged may not leave");
        assert!(strip.shown(now, true, &sendings).stays_here);
        (strip.over, strip.left, strip.stays_here) = (None, Some((key, now)), false);
        assert!(at(90, true, &strip).grown > 0.0 && at(90, true, &strip).grown < 1.0);
        assert_eq!(at(200, true, &strip).grown, 0.0);
        assert!(!at(200, false, &strip).lit);
    }

    /// The strip rises over its time and goes down over its; a sending to a device is shown waiting till it is told
    /// of, and its tick comes as it is sent.
    #[test]
    fn the_strip_rises_and_shows_how_a_sending_goes() {
        let now = Instant::now();
        let mut strip = Strip::new(1, now);
        let up = |after: u64, strip: &Strip| strip.up(now + Duration::from_millis(after));
        assert_eq!(up(0, &strip), 0.0);
        assert!(up(130, &strip) > 0.5);
        assert_eq!(up(260, &strip), 1.0);
        strip.falls = Some(now + Duration::from_millis(300));
        assert!(up(400, &strip) < 0.5);
        assert_eq!(up(520, &strip), 0.0);
        strip.falls = None;
        let device = Device { fingerprint: "AB".into(), alias: "Fold".into(), model: None, kind: None };
        strip.chips.push(Chip { device, job: Some((9, None)) });
        let mut sendings = std::collections::HashMap::new();
        assert_eq!(strip.shown(now, true, &sendings).chips[0].sending, Some((Sending::Waiting, 0.0)));
        sendings.insert(9, Sending::Going { done: 5, of: 10 });
        assert_eq!(strip.shown(now, true, &sendings).chips[0].sending, Some((Sending::Going { done: 5, of: 10 }, 0.0)));
        sendings.insert(9, Sending::Sent);
        strip.chips[0].job = Some((9, Some(now)));
        assert_eq!(strip.shown(now + TICK, true, &sendings).chips[0].sending.as_ref().map(|(_, ticked)| (ticked * 1e4).round()), Some(1e4));
    }
}
