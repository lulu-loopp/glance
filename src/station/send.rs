//! What is on the station sent to devices nearby (see `nearby`, `docs/nearby.md`): while something is dragged off a
//! station, or from another program over one, the devices seen nearby rise at its foot (the strip, drawn by `view` from
//! `shown`), each a place to drop on (a window catching drops over it on the station's thread, see `catch` and
//! `ToDevice`). Let go on one, what is dragged is sent to it, the station's own files of it held till the sending is
//! over (see `run::drag_off`), and the device shows how it goes: waiting on its user, a ring of how much has gone, a
//! tick. The strip goes down a moment after all it sent is over, the drag over. What is sent from a menu's "发送到"
//! (see `menu_row`) raises it as well, to show how that goes.
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
use crate::nearby::{Device, Part, Sending, Target};
use crate::ui::icons::Icon;
use crate::ui::menu::{Item, Mark};
use crate::ui::text::Lang;

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
/// How many times what the strip shows changed (see `changes`).
static CHANGES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times what the strip shows (see `shown`) changed so far: for a drawing kept while what it shows is the
/// same (a station's lane, see `render::PanelLayers::draw_on`) to tell that it is not, as a device is lit, the strip
/// rises or falls. Not told, it was drawn again only as something else in it changed (a sample, once a second), and
/// a device stayed lit as long after the pointer had left it.
pub fn changes() -> u64 {
    CHANGES.load(std::sync::atomic::Ordering::Relaxed)
}

fn changed() {
    CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What the strip rises for besides a drag off a station (whose numbers count up from 1, see `drag::announce`): what
/// was sent from a menu, and (with a number of their own after it) drags from other programs over a station.
const FROM_MENU: u64 = 1 << 63;
/// The drag from another program over a station now (its number, see `FROM_MENU`), while it goes on.
static FOREIGN: Mutex<Option<u64>> = Mutex::new(None);

/// A drag from another program over a station (`over`), or no drag from one any more: the strip rises for it, as
/// for one off a station, and the devices nearby are looked for (see `nearby::want_to_send`).
pub fn foreign(over: bool) {
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut held = FOREIGN.lock().unwrap();
    if over == held.is_some() {
        return;
    }
    *held = over.then(|| FROM_MENU | COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    drop(held);
    if over {
        crate::nearby::want_to_send();
    }
    crate::app().controller.wake();
}

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

    /// Rising again at `now`, from as far up as it is (going down: no more).
    fn rise_again(&mut self, now: Instant) {
        // (Risen `t` of its time, it is `1 - (1 - t)³` up: see `eased`.)
        let up = self.up(now).clamp(0.0, 1.0);
        self.rose = now.checked_sub(RISE.mul_f32(1.0 - (1.0 - up).cbrt())).unwrap_or(now);
        self.falls = None;
    }

    /// Whether `chip` is being sent to (a sending of its not over: one not told of yet is waiting), as `sendings` say.
    fn busy(chip: &Chip, sendings: &std::collections::HashMap<u64, Sending>) -> bool {
        Self::sending(chip, sendings).is_some_and(|state| !state.over())
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
    let foreign = *FOREIGN.lock().unwrap();
    let nearby = crate::nearby::shown();
    // The drag under way: off a station, or else from another program over one (what may leave this PC, all of it).
    let (drag, under_way, stays_here) = match (going.ended, foreign) {
        (None, _) => (going.drag, true, going.stays_here),
        (Some(_), Some(foreign)) => (foreign, true, false),
        (Some(_), None) => (going.drag, false, false),
    };
    let mut held = STRIP.lock().unwrap();
    if under_way && nearby.there && held.as_ref().is_none_or(|strip| strip.drag != drag) {
        match held.as_mut() {
            // (Up still, or going down: up again, from as far up as it is, for this drag too.)
            Some(strip) => {
                strip.rise_again(now);
                (strip.drag, strip.asked, strip.announced, strip.into) = (drag, now, 0, None);
            }
            None => *held = Some(Strip::new(drag, now)),
        }
    }
    let Some(strip) = held.as_mut() else { return (false, None) };
    let open = under_way && drag == strip.drag;
    if open {
        strip.stays_here = stays_here;
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
            changed();
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
    let moved = strip.drawn.as_ref() != Some(&shown);
    strip.drawn = Some(shown);
    // (Moving: drawn again with the screen.)
    if moved {
        changed();
        sooner(Duration::ZERO);
    }
    (moved, due)
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
    changed();
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

/// The sending `job` shown on `chip` (what it showed before, over, forgotten).
fn shows_job(chip: &mut Chip, job: u64) {
    if let Some((was, _)) = chip.job.replace((job, None)) {
        crate::nearby::forget(was);
    }
}

/// What is dragged off the station let go on the device `key` on the strip (its mark's middle at `at` on the screen):
/// sent to it as a sending numbered now, shown on it at once; the station's thread sends it once the drag is over (see
/// `taken`). None where that device is no longer there.
fn dropped(key: u64, at: POINT) -> bool {
    let drag = super::drag::going().drag;
    let mut held = STRIP.lock().unwrap();
    let Some(strip) = held.as_mut() else { return false };
    let Some(chip) = strip.chips.iter_mut().find(|chip| key_of(&chip.device.fingerprint) == key) else { return false };
    let job = crate::nearby::job();
    shows_job(chip, job);
    let to = chip.device.fingerprint.clone();
    strip.into = Some((drag, at));
    *DROPPED.lock().unwrap() = Some(Dropped { drag, job, to });
    true
}

/// What another program's drag gave (`given`) let go on the device `key` on the strip: sent to it as a sending
/// numbered now, shown on it at once (its files told apart on a thread of their own). None where it gave nothing, or
/// that device is no longer there.
fn dropped_from_elsewhere(key: u64, given: Vec<Content>) -> bool {
    if given.is_empty() {
        return false;
    }
    let mut held = STRIP.lock().unwrap();
    let Some(chip) = held.as_mut().and_then(|strip| strip.chips.iter_mut().find(|chip| key_of(&chip.device.fingerprint) == key)) else { return false };
    let job = crate::nearby::job();
    shows_job(chip, job);
    let to = chip.device.fingerprint.clone();
    drop(held);
    std::thread::spawn(move || crate::nearby::send(job, &to, &outgoing(&given, &folder_of(job))));
    true
}

/// The sending `job` to the device `to` (its fingerprint), sent from a menu (see `menu_row`): shown on the strip,
/// risen for it (from as far up as it is), and going down a moment after it is over, as one let go there.
pub fn sent_from_menu(job: u64, to: &str) {
    let now = Instant::now();
    let Some(device) = crate::nearby::shown().devices.into_iter().find(|device| device.fingerprint.eq_ignore_ascii_case(to)) else { return };
    let mut held = STRIP.lock().unwrap();
    let strip = held.get_or_insert_with(|| Strip::new(FROM_MENU, now));
    if strip.falls.is_some() {
        strip.rise_again(now);
    }
    let at = match strip.chips.iter().position(|chip| chip.device.fingerprint == device.fingerprint) {
        Some(at) => at,
        None => {
            strip.chips.push(Chip { device, job: None });
            strip.chips.len() - 1
        }
    };
    shows_job(&mut strip.chips[at], job);
    drop(held);
    crate::app().controller.wake();
}

/// The devices sent to now, by their fingerprints: from the settings' pane (as `shown` says) and on the strip.
fn busy_now(shown: &crate::nearby::Shown) -> Vec<String> {
    let mut busy = crate::nearby::sending_to(shown);
    if let Some(strip) = STRIP.lock().unwrap().as_ref() {
        busy.extend(strip.chips.iter().filter(|chip| Strip::busy(chip, &shown.sendings)).map(|chip| chip.device.fingerprint.to_ascii_lowercase()));
    }
    for to in &mut busy {
        to.make_ascii_lowercase();
    }
    busy.sort();
    busy.dedup();
    busy
}

/// What the "发送到" of the menu up lists (see `menu_row`): the phones paired and the certificates trusted as it
/// opened, and the devices and those sent to as it last listed them.
struct Listed {
    phones: Vec<crate::phone::Phone>,
    trusted: Vec<crate::phone::Trusted>,
    targets: Vec<Target>,
    busy: Vec<String>,
}

static LISTED: Mutex<Option<Listed>> = Mutex::new(None);

/// A menu's "发送到" (of what is on the station, copied lately, or come lately: `stays_here`, what may not leave this
/// PC is among it): the row, opening the devices it may be sent to (see `nearby::targets`), and those devices' keys
/// (see `key_of`) in their order there, what each of them is chosen as. The devices are looked for as it opens (see
/// `nearby::want_to_send`), and listed again as they are found (see `menu_row_again`). What may not leave this PC:
/// the row dimmed, saying so, opening none.
pub fn menu_row(stays_here: bool) -> (Item<'static>, Vec<u64>) {
    let lang = crate::update::language();
    let mut row = Item::new(lang.pick("发送到", "Send to"), Some(Mark::Icon(Icon::Send)), false, false);
    if stays_here {
        (row.detail, row.dimmed) = (Some(lang.pick("此内容不可发送到其他设备", "This may not leave this PC").into()), true);
        *LISTED.lock().unwrap() = None;
        return (row, Vec::new());
    }
    crate::nearby::want_to_send();
    crate::nearby::announce();
    let (phones, trusted) = (crate::phone::phones(), crate::phone::trusted_kept());
    let shown = crate::nearby::shown();
    let targets = crate::nearby::targets(&shown.devices, &phones, &trusted);
    let busy = busy_now(&shown);
    let keys;
    (row.more, keys) = device_rows(&targets, &busy, lang);
    *LISTED.lock().unwrap() = Some(Listed { phones, trusted, targets, busy });
    (row, keys)
}

/// The devices the "发送到" of the menu up lists (see `menu_row`), where they changed since it last listed them (found,
/// gone, sent to, a sending over): what it opens now, and their keys. None where they did not.
pub fn menu_row_again() -> Option<(Vec<Item<'static>>, Vec<u64>)> {
    let mut held = LISTED.lock().unwrap();
    let listed = held.as_mut()?;
    let shown = crate::nearby::shown();
    let targets = crate::nearby::targets(&shown.devices, &listed.phones, &listed.trusted);
    let busy = busy_now(&shown);
    if targets == listed.targets && busy == listed.busy {
        return None;
    }
    (listed.targets, listed.busy) = (targets, busy);
    Some(device_rows(&listed.targets, &listed.busy, crate::update::language()))
}

/// The rows of `targets` in a menu's "发送到", and their keys: each with its kind's mark, its name and, under it, what it
/// is (a paired phone, Glance's or LocalSend's, its model); one being sent to (`busy`, by lowercase fingerprints), or
/// not reached, dimmed, saying so. None yet: a dimmed row saying they are looked for.
fn device_rows(targets: &[Target], busy: &[String], lang: Lang) -> (Vec<Item<'static>>, Vec<u64>) {
    if targets.is_empty() {
        let mut looking = Item::new(lang.pick("正在查找附近的设备", "Looking for devices nearby"), Some(Mark::Icon(Icon::Search)), false, false);
        looking.dimmed = true;
        return (vec![looking], Vec::new());
    }
    targets
        .iter()
        .map(|target| {
            let name = if target.name.is_empty() { lang.pick("未命名的设备", "Unnamed device").to_string() } else { target.name.clone() };
            let mut row = Item::new(name, Some(Mark::Icon(Icon::of_device(target.kind.as_deref()))), false, false);
            let maker = if target.glance { "Glance" } else { "LocalSend" };
            let (detail, dimmed) = if busy.contains(&target.fingerprint.to_ascii_lowercase()) {
                (lang.pick("正在发送", "Sending").to_string(), true)
            } else if !target.found {
                (lang.pick("未开启接收", "Not receiving").to_string(), true)
            } else if target.paired {
                (lang.pick("已配对的手机", "Paired phone").to_string(), false)
            } else {
                (target.model.as_ref().map_or_else(|| maker.to_string(), |model| format!("{maker} · {model}")), false)
            };
            (row.detail, row.dimmed) = (Some(detail.into()), dimmed);
            (row, key_of(&target.fingerprint))
        })
        .unzip()
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

/// What `given` sends (see `nearby::Part`): each file where it is, and the name the device is told; a folder as each
/// file in it (named by its place there, "folder/inner/file", as LocalSend names them; what in it only points elsewhere
/// left out: a link may point back up into it); words and links as words (named as they would be kept as a file, see
/// `drag::text_name`), after the files; a picture or what came with no path yet written as a file in `folder` first
/// (as it is dragged off, see `drag::as_paths`). None of it where something of it could not be written.
pub fn outgoing(given: &[Content], folder: &Path) -> Vec<Part> {
    let not_words: Vec<Content> = given.iter().filter(|content| !matches!(content, Content::Text(_))).cloned().collect();
    let Some(paths) = super::drag::as_paths(&not_words, folder) else { return Vec::new() };
    let mut files = Vec::new();
    for path in paths {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => within(&path, &name, &mut files),
            Ok(_) => files.push(Part::File(path, name)),
            Err(_) => {}
        }
    }
    files.extend(given.iter().filter_map(|content| match content {
        Content::Text(words) => Some(Part::Words(words.clone(), super::drag::text_name(words))),
        _ => None,
    }));
    files
}

/// Each file within the folder `folder` (named `named`) added to `files`, named by its place there.
fn within(folder: &Path, named: &str, files: &mut Vec<Part>) {
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
            files.push(Part::File(entry.path(), name));
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

/// What the device `key` on the strip calls itself, where it is on it, and whether it is being sent to.
fn alias_of(key: u64) -> Option<(String, bool)> {
    let sendings = crate::nearby::shown().sendings;
    STRIP.lock().unwrap().as_ref()?.chips.iter().find(|chip| key_of(&chip.device.fingerprint) == key).map(|chip| (chip.device.alias.clone(), Strip::busy(chip, &sendings)))
}

impl IDropTarget_Impl for ToDevice_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let going = super::drag::going();
        // Off a station (what may leave this PC), or from another program, what the station would take.
        let own = going.ended.is_none();
        let foreign = !own && FOREIGN.lock().unwrap().is_some() && data.as_ref().is_some_and(super::drop::takes);
        let alias = alias_of(self.key);
        // (Being sent to: given nothing more till it is over.)
        self.takes.set((own && !going.stays_here || foreign) && alias.as_ref().is_some_and(|(_, busy)| !busy));
        over(self.key, true, std::time::Instant::now());
        if let Some(data) = data.as_ref() {
            let lang = crate::update::language_in(&crate::app().settings.lock().unwrap().view);
            match alias {
                Some((alias, _)) if self.takes.get() => super::drag::describe(data, DROPIMAGE_COPY, lang.pick("发送到 %1", "Send to %1"), &alias),
                Some(_) if own && going.stays_here => super::drag::describe(data, DROPIMAGE_NONE, lang.pick("此内容不可发送到其他设备", "This may not leave this PC"), ""),
                Some((_, true)) => super::drag::describe(data, DROPIMAGE_NONE, lang.pick("正在发送，请稍候", "Sending, a moment please"), ""),
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

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let offered = unsafe { *effect };
        let copies = self.takes.get() && offered.0 & DROPEFFECT_COPY.0 != 0;
        // (Off a station: sent once its drag is over, by the station's thread. From another program: read now.)
        let sent = copies
            && match super::drag::going().ended {
                None => dropped(self.key, self.at),
                Some(_) => data.as_ref().is_some_and(|data| dropped_from_elsewhere(self.key, super::drop::read(data))),
            };
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
    /// as itself; words and a link as words (nothing written for them), named as their text file would be, after the
    /// files.
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
        let given = [Content::Text("见面地点\n东门".into()), Content::Files(vec![trip.clone(), alone.clone()]), Content::Text("https://glancepc.com".into())];
        let files = outgoing(&given, &root.join("written"));
        let names: Vec<&str> = files.iter().map(Part::name).collect();
        assert_eq!(names, ["trip/a.jpg", "trip/day 2/b.jpg", "alone.pdf", "见面地点.txt", "httpsglancepc.com.txt"]);
        assert_eq!(files[1], Part::File(trip.join("day 2").join("b.jpg"), "trip/day 2/b.jpg".into()));
        assert_eq!(files[3], Part::Words("见面地点\n东门".into(), "见面地点.txt".into()));
        assert!(matches!(&files[4], Part::Words(words, _) if words == "https://glancepc.com"));
        assert!(!root.join("written").exists(), "nothing written for words");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A device grows as the pointer comes over it (past its size, and back), and shrinks back as it leaves; it is
    /// lit only where what is dragged may be let go there.
    #[test]
    fn a_device_grows_under_the_pointer() {
        let now = Instant::now();
        let device = Device { fingerprint: "AB".into(), alias: "Galaxy Z Fold6".into(), model: None, kind: Some("mobile".into()), glance: false };
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

    /// A menu's "发送到" lists each device with what it is under its name; one not reached, or being sent to, dimmed and
    /// saying so; none yet, a dimmed row saying they are looked for. Each is chosen by its key.
    #[test]
    fn a_menu_lists_the_devices_to_send_to() {
        let target = |fingerprint: &str, name: &str, paired: bool, glance: bool, found: bool, model: Option<&str>| Target { fingerprint: fingerprint.into(), name: name.into(), kind: Some(if paired { "mobile" } else { "desktop" }.into()), model: model.map(str::to_string), paired, glance, found };
        let targets = [target("AA", "Fold", true, true, true, None), target("BB", "Pixel", true, true, false, None), target("CC", "Study", false, true, true, Some("Windows")), target("DD", "", false, false, true, None)];
        let (rows, keys) = device_rows(&targets, &["cc".to_string()], Lang::Zh);
        let said: Vec<(String, String, bool)> = rows.iter().map(|row| (row.label.to_string(), row.detail.as_deref().unwrap_or_default().to_string(), row.dimmed)).collect();
        assert_eq!(
            said,
            [("Fold".into(), "已配对的手机".into(), false), ("Pixel".into(), "未开启接收".into(), true), ("Study".into(), "正在发送".into(), true), ("未命名的设备".into(), "LocalSend".into(), false)]
        );
        assert_eq!(keys, [key_of("aa"), key_of("bb"), key_of("cc"), key_of("dd")]);
        let (rows, keys) = device_rows(&targets[2..3], &[], Lang::Zh);
        assert_eq!(rows[0].detail.as_deref(), Some("Glance · Windows"));
        assert!(!rows[0].dimmed && keys.len() == 1);
        let (rows, keys) = device_rows(&[], &[], Lang::Zh);
        assert!(rows.len() == 1 && rows[0].dimmed && keys.is_empty());
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
        let device = Device { fingerprint: "AB".into(), alias: "Fold".into(), model: None, kind: None, glance: false };
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
