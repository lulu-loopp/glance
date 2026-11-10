//! The GPU makers' libraries (see `gpu_power`), worked on a thread of their own, the only one that calls into them:
//! each card looked up through its maker's library there, and read through it there, as the sampler asks, the sampler
//! taking what was last read without waiting for it. Opening a library and finding the cards through it may take
//! seconds while the drivers are still starting (5.5 s at a laptop's sign-in), and a driver's call may not come back
//! at all: neither holds the first look at the hardware, nor any other reading. What is known is told as it is known:
//! each card's looking up as it is done, each of its readings as it is read (with when its reading began, so that one
//! read before a call that did not come back is not taken for one read since).
//!
//! One thread for all of them, look-ups and readings alike: NVML is documented as safe to call from several threads,
//! but NVAPI, ADL and IGCL say nothing of it, and a thread of its own each would still have a card's look-up and its
//! readings, or two cards' of one maker, meet in one library. Nor is a library called from two threads by two samplers
//! one after the other: a sampler's thread is the process's only one in them while it lasts (see `InTheLibraries`),
//! one made while another's is still in a library waiting for it to end, or for its own sampler to go.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::gpu_power::{GpuPower, More, PciAddress, Reader};
use crate::metrics::PowerNow;
use crate::reading::GpuLimit;

/// What the makers' libraries are asked: `GpuPower`, or what stands for it in a test. Each reading's calls are made
/// only as `awake` says the card is just before each of them (see `GpuPower::more`).
pub trait Libraries {
    /// How to read the card made by `vendor` at `address`, as `GpuPower::reader` says.
    fn reader(&mut self, vendor: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>>;
    fn power(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<f32>;
    fn more(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<More>;
    fn limit(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<GpuLimit>;
    /// Whether it is Intel's graphics in the CPU, as its library said as it found it (nothing asked now).
    fn intel_integrated(&self, reader: Reader) -> bool;
    /// What Intel's library found, for a report, as `GpuPower::describe_intel` says.
    fn describe_intel(&self, awake: &dyn Fn(PciAddress) -> bool) -> Option<String>;
}

impl Libraries for GpuPower {
    fn reader(&mut self, vendor: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>> {
        GpuPower::reader(self, vendor, address, awake)
    }

    fn power(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
        self.read(reader, awake)
    }

    fn more(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<More> {
        GpuPower::more(self, reader, awake)
    }

    fn limit(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<GpuLimit> {
        GpuPower::limit(self, reader, awake)
    }

    fn intel_integrated(&self, reader: Reader) -> bool {
        GpuPower::intel_integrated(self, reader)
    }

    fn describe_intel(&self, awake: &dyn Fn(PciAddress) -> bool) -> Option<String> {
        GpuPower::describe_intel(self, awake)
    }
}

/// How far a card's looking up through its maker's library has come.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lookup {
    /// Not come to yet: the libraries' thread looks the cards up as it starts, after the first look at the hardware
    /// has begun.
    NotYet,
    /// Put off: it, or a card of its maker, slept (see `GpuPower::reader`).
    PutOff,
    /// How its maker's library reads it, if it does.
    Done(Option<Reader>),
}

impl Lookup {
    /// How its power is read, for a report.
    fn describe(self) -> &'static str {
        match self {
            Lookup::NotYet => "not looked for yet (its maker's library is opened apart from the first look at the hardware)",
            Lookup::PutOff => "looked for once it and its maker's other cards are awake (one asleep)",
            Lookup::Done(reader) => reader.map_or("not available", Reader::describe),
        }
    }
}

/// A GPU as the libraries' thread has it: its name (for what is noted and reported of it), its maker (its PCI vendor
/// id) and where it sits on the PCI bus (which of the makers' libraries reads it, and which of that library's cards it
/// is), and its device, as Windows' device manager has it, whose power state says whether it sleeps (none where it is
/// not found).
pub struct Card {
    pub name: String,
    pub vendor: u32,
    pub address: Option<PciAddress>,
    pub device: Option<u32>,
    lookup: Lookup,
}

impl Card {
    /// A card not looked up yet.
    pub fn new(name: String, vendor: u32, address: Option<PciAddress>, device: Option<u32>) -> Self {
        Card { name, vendor, address, device, lookup: Lookup::NotYet }
    }
}

/// What a card's maker's library read of it lately (see `GpuLibraries::take`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Read {
    /// Its power, in watts.
    pub power: Option<f32>,
    pub more: More,
    /// What holds its clock back (only NVIDIA's library says).
    pub limit: Option<GpuLimit>,
}

/// One of a card's readings through its maker's library, with when its reading began: it is as old as that, however
/// long the library took to give it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reading {
    Power(Instant, Option<f32>),
    More(Instant, More),
    Limit(Instant, Option<GpuLimit>),
    /// The card found asleep: none of what was read of it is so any longer.
    Asleep,
}

/// What was last read of a card, each with when its reading began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Readings {
    power: Option<(Instant, Option<f32>)>,
    more: Option<(Instant, More)>,
    limit: Option<(Instant, Option<GpuLimit>)>,
}

impl Readings {
    fn put(&mut self, reading: Reading) {
        match reading {
            Reading::Power(at, watts) => self.power = Some((at, watts)),
            Reading::More(at, more) => self.more = Some((at, more)),
            Reading::Limit(at, limit) => self.limit = Some((at, limit)),
            Reading::Asleep => *self = Readings::default(),
        }
    }

    /// Those whose reading began no longer than `within` before `now`, together (none: none did).
    fn within(&self, within: Duration, now: Instant) -> Option<Read> {
        let fresh = |at: Instant| now.saturating_duration_since(at) <= within;
        let power = self.power.filter(|(at, _)| fresh(*at));
        let more = self.more.filter(|(at, _)| fresh(*at));
        let limit = self.limit.filter(|(at, _)| fresh(*at));
        (power.is_some() || more.is_some() || limit.is_some()).then(|| Read {
            power: power.and_then(|(_, watts)| watts),
            more: more.map_or_else(More::default, |(_, more)| more),
            limit: limit.and_then(|(_, limit)| limit),
        })
    }
}

/// What the cards' looking up came to: each card's line for a report (see `StaticInfo::found`), and whether each is
/// Intel's graphics built into the CPU, by their places among the cards.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub lines: Vec<String>,
    pub integrated: Vec<bool>,
}

/// The cards' lines for a report, as each one's looking up has come (Intel's library's own line apart).
fn lines_of(cards: &[Card]) -> Vec<String> {
    cards.iter().map(|card| format!("GPU power, {}: {}", card.name, card.lookup.describe())).collect()
}

/// What is said for a report of `cards` before the libraries' thread has looked any of them up.
pub fn found_before(cards: &[Card]) -> Found {
    Found { lines: lines_of(cards), integrated: vec![false; cards.len()] }
}

/// What the libraries' thread works on: the libraries, the cards, and Intel's library's line for a report, as last
/// asked (none: not yet, or it found none).
struct Work<L> {
    libraries: L,
    cards: Vec<Card>,
    intel: Option<String>,
}

impl<L: Libraries> Work<L> {
    fn new(libraries: L, cards: Vec<Card>) -> Self {
        Work { libraries, cards, intel: None }
    }

    /// Each card not looked up yet looked up through its maker's library (see `GpuPower::reader`): once, and only while
    /// neither it nor any other card of its maker sleeps, as `power` says just before each step of it, its own state
    /// asked again first (the cards before it have been looked up since, which may have taken long). Neither IGCL nor
    /// NVAPI has a way to find one card but asking each of its maker's what it is (IGCL, its properties) or where it
    /// sits (NVAPI, its bus and its slot) till it comes to the one asked for, and IGCL, NVAPI and ADL list every one as
    /// they are opened: one asleep, an Intel or an AMD laptop's discrete GPU beside graphics of the same maker's in its
    /// CPU, would be asked along with the one awake. Its maker's cards are those there now (see
    /// `PowerNow::makers_awake`), not only those found as the sampler was made. One whose place on the bus is not known
    /// is read by none. Each card's library asked, `calling` is told of it first, and what the looking up has come to
    /// is told to `told` as each card's changes (a call after it may not come back). None is asked once `wanted` says
    /// the looking up is no longer wanted (its sampler gone). Returns those whose library was asked, by their places
    /// among the cards, with how long it took.
    fn look_up(&mut self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>), wanted: &dyn Fn() -> bool, told: &mut dyn FnMut(Found)) -> Vec<(usize, Duration)> {
        let mut asked = Vec::new();
        for at in 0..self.cards.len() {
            if matches!(self.cards[at].lookup, Lookup::Done(_)) {
                continue;
            }
            if !wanted() {
                break;
            }
            let card = &self.cards[at];
            let lookup = if power.sleeps(card.device) {
                Lookup::PutOff
            } else if let Some(address) = card.address {
                calling(Some(format!("looking up {} through its maker's library", card.name)));
                let began = Instant::now();
                let vendor = card.vendor;
                let lookup = self.libraries.reader(vendor, address, &mut || power.makers_awake(vendor)).map_or(Lookup::PutOff, Lookup::Done);
                asked.push((at, began.elapsed()));
                lookup
            } else {
                Lookup::Done(None)
            };
            if std::mem::replace(&mut self.cards[at].lookup, lookup) != lookup {
                told(self.found());
            }
        }
        calling(None);
        asked
    }

    /// Each card's readings through its maker's library, by its place among the cards, put by `put` as each is read,
    /// with when its reading began: of a card looked up, each only as `power` says the card is awake just before each
    /// call of it (after the cards and the calls before it, which may have taken a while), what is not asked unread.
    /// A card found asleep is put so, and asked nothing. Only the card's own state is asked: a reading asks the card
    /// alone, by the handle its library gave as it found it (NVML's device, ADL's adapter, IGCL's device, NVAPI's GPU),
    /// and lists or asks no other, as opening a library and finding a card through it do (see `look_up`). Each card's
    /// library asked, `calling` is told of it first.
    fn read(&self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>), put: &mut dyn FnMut(usize, Reading)) {
        for (at, card) in self.cards.iter().enumerate() {
            let Lookup::Done(Some(reader)) = card.lookup else { continue };
            if power.sleeps(card.device) {
                put(at, Reading::Asleep);
                continue;
            }
            calling(Some(format!("reading {} through its maker's library", card.name)));
            let mut awake = || !power.sleeps(card.device);
            let began = Instant::now();
            put(at, Reading::Power(began, self.libraries.power(reader, &mut awake)));
            let began = Instant::now();
            put(at, Reading::More(began, self.libraries.more(reader, &mut awake).unwrap_or_default()));
            let began = Instant::now();
            put(at, Reading::Limit(began, self.libraries.limit(reader, &mut awake)));
        }
        calling(None);
    }

    /// Intel's library's line for a report asked anew, its graphics asked what they read only where `power` says they
    /// are awake just before (found awake, they may have gone to sleep since). `calling` is told of it first.
    fn describe_intel(&mut self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>)) {
        let awake = |at: PciAddress| self.cards.iter().find(|card| card.address == Some(at)).is_some_and(|card| !power.sleeps(card.device));
        calling(Some("asking Intel's library what it reads of its graphics".into()));
        let intel = self.libraries.describe_intel(&awake);
        calling(None);
        if let Some(intel) = intel {
            self.intel = Some(format!("Intel's graphics library: {}", if intel.is_empty() { "no graphics of Intel's".to_string() } else { intel }));
        }
    }

    /// What the cards' looking up has come to, as it is known (no library asked): each card's line, Intel's library's
    /// as last asked, and which are Intel's graphics in the CPU.
    fn found(&self) -> Found {
        let mut lines = lines_of(&self.cards);
        lines.extend(self.intel.clone());
        let integrated = self.cards.iter().map(|card| matches!(card.lookup, Lookup::Done(Some(reader)) if self.libraries.intel_integrated(reader))).collect();
        Found { lines, integrated }
    }
}

/// A first looking up of the GPUs that took this long or longer is noted, with each card's part in it: it no longer
/// holds the first look at the hardware, but holds back what the makers' libraries read for as long.
const NOTED_FROM: Duration = Duration::from_secs(1);

/// What is noted of the first looking up of `cards` through their makers' libraries, which took `took`: each card,
/// with how long its library took where it was asked (`asked`), and what came of it.
fn first_look_up_noted(took: Duration, cards: &[Card], asked: &[(usize, Duration)]) -> String {
    let each: Vec<String> = cards
        .iter()
        .enumerate()
        .map(|(at, card)| match asked.iter().find(|(card, _)| *card == at) {
            Some((_, took)) => format!("{}: {} ({} ms)", card.name, card.lookup.describe(), took.as_millis()),
            None => format!("{}: {}", card.name, card.lookup.describe()),
        })
        .collect();
    format!("GPU libraries: the GPUs were looked up in {} ms, apart from the first look at the hardware ({})", took.as_millis(), each.join(", "))
}

/// The call into a library under way: since when, in time awake (see `sensing::awake`), what it is, and whether it has
/// been noted as stuck.
struct Calling {
    since: Duration,
    what: String,
    noted: bool,
}

/// What the libraries' thread and the sampler share: what was last read of each card, what the cards' looking up came
/// to as it changes (none: taken already), and the call into a library under way.
#[derive(Default)]
struct Shared {
    read: Vec<Readings>,
    found: Option<Found>,
    calling: Option<Calling>,
}

impl Shared {
    /// What was read of each card whose reading began no longer than `within` before `now`.
    fn read_within(&self, within: Duration, now: Instant) -> Vec<Option<Read>> {
        self.read.iter().map(|readings| readings.within(within, now)).collect()
    }

    /// The call under way, noted as stuck once it has been for longer than a reading may be (see `sensing::STUCK_AFTER`)
    /// by `now`, in time awake: what to note of it, the first time.
    fn stuck(&mut self, now: Duration) -> Option<String> {
        let calling = self.calling.as_mut().filter(|calling| !calling.noted && now.saturating_sub(calling.since) > crate::sensing::STUCK_AFTER)?;
        calling.noted = true;
        Some(format!(
            "GPU libraries: {} has taken over {} s, and what the makers' libraries read is left out till it comes back (the rest read as ever)",
            calling.what,
            crate::sensing::STUCK_AFTER.as_secs()
        ))
    }

    /// The call under way from now (none: none is), as of `now` in time awake. Returns what to note of the one before,
    /// if it was noted stuck (it came back).
    fn calling(&mut self, what: Option<String>, now: Duration) -> Option<String> {
        let before = std::mem::replace(&mut self.calling, what.map(|what| Calling { since: now, what, noted: false }))?;
        before.noted.then(|| format!("GPU libraries: {} came back after {} s", before.what, now.saturating_sub(before.since).as_secs()))
    }
}

/// Whether a libraries' thread of the process is in the libraries now, and what wakes those waiting to be: the one in
/// them gone, or a sampler of theirs gone.
static IN_THE_LIBRARIES: Mutex<bool> = Mutex::new(false);
static FREED: Condvar = Condvar::new();

/// A libraries' thread's place in the libraries, the process's only one there while it is held: given up as it is
/// dropped, after the libraries opened under it are closed (dropped before it).
struct InTheLibraries;

impl InTheLibraries {
    /// Taken once no other thread of the process is in the libraries, waited for till then while `gone` says the
    /// sampler it is for has not gone (none: it has, and nothing is taken, so a thread whose sampler went while it
    /// waited ends without opening any library).
    fn taken(gone: &AtomicBool) -> Option<Self> {
        let mut held = IN_THE_LIBRARIES.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if gone.load(Ordering::Relaxed) {
                return None;
            }
            if !*held {
                *held = true;
                return Some(InTheLibraries);
            }
            held = FREED.wait(held).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for InTheLibraries {
    fn drop(&mut self) {
        *IN_THE_LIBRARIES.lock().unwrap_or_else(PoisonError::into_inner) = false;
        FREED.notify_all();
    }
}

/// The GPU makers' libraries, worked on a thread of their own, which ends once this is dropped (and any call it is in
/// has come back).
pub struct GpuLibraries {
    shared: Arc<Mutex<Shared>>,
    ask: SyncSender<()>,
    /// Whether this has been dropped, for its thread waiting to be in the libraries (see `InTheLibraries::taken`).
    gone: Arc<AtomicBool>,
}

impl GpuLibraries {
    /// The makers' libraries, for `cards`.
    pub fn start(cards: Vec<Card>) -> Self {
        Self::working(cards, crate::metrics::DeviceManager, GpuPower::new)
    }

    /// Its thread started, for `cards`, on `open`'s libraries (opened on it) and `power`'s word on what sleeps: once
    /// it is the process's only thread in the libraries, it looks the cards up, and then reads them each time it is
    /// asked (see `take`). Nothing here waits for it.
    fn working<L: Libraries, P: PowerNow + Send + 'static>(cards: Vec<Card>, power: P, open: impl FnOnce() -> L + Send + 'static) -> Self {
        let shared = Arc::new(Mutex::new(Shared { read: vec![Readings::default(); cards.len()], ..Shared::default() }));
        let gone = Arc::new(AtomicBool::new(false));
        let (ask, asked) = mpsc::sync_channel::<()>(1);
        std::thread::Builder::new()
            .name("glance: GPU libraries".into())
            .spawn({
                let (shared, gone) = (shared.clone(), gone.clone());
                move || {
                    let Some(_in_them) = InTheLibraries::taken(&gone) else { return };
                    let wanted = || !gone.load(Ordering::Relaxed);
                    let mut calling = |what: Option<String>| {
                        let came_back = shared.lock().unwrap().calling(what, crate::sensing::awake());
                        if let Some(line) = came_back {
                            crate::journal::note(line);
                        }
                    };
                    let mut told = |found: Found| shared.lock().unwrap().found = Some(found);
                    let mut put = |at: usize, reading: Reading| shared.lock().unwrap().read[at].put(reading);
                    let mut work = Work::new(open(), cards);
                    let began = Instant::now();
                    let asked_of = work.look_up(&power, &mut calling, &wanted, &mut told);
                    if began.elapsed() >= NOTED_FROM {
                        crate::journal::note(first_look_up_noted(began.elapsed(), &work.cards, &asked_of));
                    }
                    if wanted() {
                        work.describe_intel(&power, &mut calling);
                        told(work.found());
                    }
                    while asked.recv().is_ok() {
                        // A card put off looked up once it and its maker's other cards are all awake.
                        let mut looked = work.look_up(&power, &mut calling, &wanted, &mut told);
                        looked.retain(|(at, _)| matches!(work.cards[*at].lookup, Lookup::Done(_)));
                        for &(at, took) in &looked {
                            let card = &work.cards[at];
                            crate::journal::note(format!("GPU power, {}, looked for as it and its maker's other cards were awake: {} ({} ms)", card.name, card.lookup.describe(), took.as_millis()));
                        }
                        if !looked.is_empty() {
                            work.describe_intel(&power, &mut calling);
                            told(work.found());
                        }
                        work.read(&power, &mut calling, &mut put);
                    }
                }
            })
            .expect("a thread started");
        GpuLibraries { shared, ask, gone }
    }

    /// What was read lately of each card, by its place among the cards: each reading whose reading began within
    /// `within` (one older is not shown as now), another reading asked (one asked and not yet begun is not asked
    /// twice). Never waited for.
    pub fn take(&self, within: Duration) -> Vec<Option<Read>> {
        let _ = self.ask.try_send(());
        self.shared.lock().unwrap().read_within(within, Instant::now())
    }

    /// What the cards' looking up has come to, if it has changed since this was last asked.
    pub fn found(&self) -> Option<Found> {
        self.shared.lock().unwrap().found.take()
    }

    /// What to note of a call into a library stuck, once it is (see `Shared::stuck`).
    pub fn stuck(&self) -> Option<String> {
        self.shared.lock().unwrap().stuck(crate::sensing::awake())
    }
}

impl Drop for GpuLibraries {
    fn drop(&mut self) {
        // (Said under the lock its thread looks at it under, as it waits to be in the libraries: it does not miss it.)
        let held = IN_THE_LIBRARIES.lock().unwrap_or_else(PoisonError::into_inner);
        self.gone.store(true, Ordering::Relaxed);
        drop(held);
        FREED.notify_all();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;

    pub(crate) const INTEL: u32 = 0x8086;
    pub(crate) const NVIDIA: u32 = 0x10DE;
    pub(crate) const AMD: u32 = 0x1002;

    /// A machine's GPUs as its device manager tells them, for a test: each card there, by its maker and its device,
    /// and those of them asleep, put to sleep and woken as the test goes.
    pub(crate) struct Machine {
        cards: Vec<(u32, u32)>,
        sleeping: RefCell<HashSet<u32>>,
    }

    impl Machine {
        pub(crate) fn new(cards: &[(u32, u32)], sleeping: &[u32]) -> Self {
            Machine { cards: cards.to_vec(), sleeping: RefCell::new(sleeping.iter().copied().collect()) }
        }

        pub(crate) fn sleep(&self, device: u32) {
            self.sleeping.borrow_mut().insert(device);
        }

        pub(crate) fn wake(&self, device: u32) {
            self.sleeping.borrow_mut().remove(&device);
        }
    }

    impl PowerNow for Machine {
        fn sleeps(&self, device: Option<u32>) -> bool {
            device.is_some_and(|device| self.sleeping.borrow().contains(&device))
        }

        fn makers_awake(&self, vendor: u32) -> bool {
            self.cards.iter().filter(|(maker, _)| *maker == vendor).all(|(_, device)| !self.sleeping.borrow().contains(device))
        }
    }

    /// A machine whose cards are all awake, all the while.
    struct AllAwake;

    impl PowerNow for AllAwake {
        fn sleeps(&self, _: Option<u32>) -> bool {
            false
        }

        fn makers_awake(&self, _: u32) -> bool {
            true
        }
    }

    /// The card of the maker `vendor` whose device is `device`, as the sampler found it.
    fn card(vendor: u32, device: u32) -> Card {
        Card::new(format!("card {device}"), vendor, Some((device, 0, 0)), Some(device))
    }

    /// Libraries standing in for the makers': each call made only as it is told the card is awake (or, looking one up,
    /// its maker's cards), as `GpuPower`'s are, and doing `meanwhile` as it is made (what goes to sleep as it takes its
    /// time), what each was asked of which card's device written down in order, with when. Each card is read as one of
    /// Intel's library's, numbered by its device, and drawing as many watts: device 1's the graphics in the CPU.
    struct Standing<'a> {
        meanwhile: Box<dyn Fn(u32) + 'a>,
        asked: RefCell<Vec<(&'static str, u32)>>,
        when: RefCell<Vec<Instant>>,
    }

    fn standing<'a>(meanwhile: impl Fn(u32) + 'a) -> Standing<'a> {
        Standing { meanwhile: Box::new(meanwhile), asked: RefCell::new(Vec::new()), when: RefCell::new(Vec::new()) }
    }

    impl Standing<'_> {
        /// `what` asked of the card read by `reader`, if `awake` says it is.
        fn ask(&self, what: &'static str, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<u32> {
            let Reader::Intel(device) = reader else { return None };
            if !awake() {
                return None;
            }
            self.asked.borrow_mut().push((what, device as u32));
            self.when.borrow_mut().push(Instant::now());
            (self.meanwhile)(device as u32);
            Some(device as u32)
        }
    }

    impl Libraries for Standing<'_> {
        fn reader(&mut self, _: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>> {
            if !awake() {
                return None;
            }
            self.asked.borrow_mut().push(("looked up", address.0));
            (self.meanwhile)(address.0);
            Some(Some(Reader::Intel(address.0 as usize)))
        }

        fn power(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<f32> {
            self.ask("power", reader, awake).map(|device| device as f32)
        }

        fn more(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<More> {
            self.ask("more", reader, awake).map(|device| More { fan_rpm: Some(device), ..More::default() })
        }

        fn limit(&self, reader: Reader, awake: &mut dyn FnMut() -> bool) -> Option<GpuLimit> {
            self.ask("limit", reader, awake).map(|_| GpuLimit::Free)
        }

        fn intel_integrated(&self, reader: Reader) -> bool {
            reader == Reader::Intel(1)
        }

        fn describe_intel(&self, _: &dyn Fn(PciAddress) -> bool) -> Option<String> {
            None
        }
    }

    /// `cards` looked up on `machine` through libraries standing in for the makers', which do `meanwhile` as they do:
    /// the devices of those their library was asked of, in order.
    fn looked_up<'a>(cards: Vec<Card>, machine: &Machine, meanwhile: impl Fn(u32) + 'a) -> (Work<Standing<'a>>, Vec<u32>) {
        let mut work = Work::new(standing(meanwhile), cards);
        work.look_up(machine, &mut |_| {}, &|| true, &mut |_| {});
        let asked = work.libraries.asked.take().into_iter().map(|(_, device)| device).collect();
        (work, asked)
    }

    /// Looked up again, on `machine`: the devices of those asked.
    fn again(work: &mut Work<Standing>, machine: &Machine) -> Vec<u32> {
        work.look_up(machine, &mut |_| {}, &|| true, &mut |_| {});
        work.libraries.asked.take().into_iter().map(|(_, device)| device).collect()
    }

    /// `work`'s cards read on `machine`: what was read of each, as it was put (with when each reading began).
    fn read_on<L: Libraries>(work: &Work<L>, machine: &Machine) -> Vec<Readings> {
        let mut read = vec![Readings::default(); work.cards.len()];
        work.read(machine, &mut |_| {}, &mut |at, reading| read[at].put(reading));
        read
    }

    /// A GPU awake is looked up through its maker's library only while every other card of that maker is awake too:
    /// an Intel laptop's graphics in its CPU not while its Arc card sleeps (IGCL would ask the sleeping card what it
    /// is as it looked for the other), an AMD's likewise, a second NVIDIA card's not while the first sleeps (NVAPI would
    /// ask where it sits). Another maker's card asleep stands in the way of none, and one looked up already is not
    /// looked up again. All awake, each is.
    #[test]
    fn no_card_asleep_is_asked_as_another_is_looked_up() {
        let intel_laptop = Machine::new(&[(INTEL, 1), (INTEL, 2)], &[2]);
        let (mut work, asked) = looked_up(vec![card(INTEL, 1), card(INTEL, 2)], &intel_laptop, |_| {});
        assert!(asked.is_empty());
        assert!(work.cards.iter().all(|card| card.lookup == Lookup::PutOff));
        let amd_laptop = Machine::new(&[(AMD, 1), (AMD, 2)], &[2]);
        assert!(looked_up(vec![card(AMD, 1), card(AMD, 2)], &amd_laptop, |_| {}).1.is_empty());
        let two_nvidia = Machine::new(&[(NVIDIA, 1), (NVIDIA, 2)], &[1]);
        assert!(looked_up(vec![card(NVIDIA, 1), card(NVIDIA, 2)], &two_nvidia, |_| {}).1.is_empty());
        // Graphics of Intel's in the CPU, beside an NVIDIA card asleep.
        let hybrid = Machine::new(&[(INTEL, 1), (NVIDIA, 2)], &[2]);
        let (hybrid_work, asked) = looked_up(vec![card(INTEL, 1), card(NVIDIA, 2)], &hybrid, |_| {});
        assert_eq!(asked, [1]);
        assert_eq!(hybrid_work.cards[1].lookup, Lookup::PutOff);
        // The Arc card woken, both of the Intel laptop's are looked up, once.
        intel_laptop.wake(2);
        assert_eq!(again(&mut work, &intel_laptop), [1, 2]);
        assert!(again(&mut work, &intel_laptop).is_empty());
    }

    /// Whether a card sleeps is asked just before it is looked up, after the cards before it were, as are its maker's
    /// other cards: an Intel and NVIDIA laptop's NVIDIA card gone to sleep while Intel's library was opened and asked
    /// is not looked up (NVML's finding it would start it up), nor an NVIDIA card awake while another of NVIDIA's went
    /// to sleep meanwhile (NVAPI would ask the sleeping one where it sits). Once awake, each is.
    #[test]
    fn a_card_gone_to_sleep_while_another_was_looked_up_is_not_asked() {
        let laptop = Machine::new(&[(INTEL, 1), (NVIDIA, 2)], &[]);
        let (mut work, asked) = looked_up(vec![card(INTEL, 1), card(NVIDIA, 2)], &laptop, |_| laptop.sleep(2));
        assert_eq!(asked, [1]);
        assert_eq!(work.cards[1].lookup, Lookup::PutOff);
        laptop.wake(2);
        work.libraries.meanwhile = Box::new(|_| {});
        assert_eq!(again(&mut work, &laptop), [2]);
        let two_nvidia = Machine::new(&[(INTEL, 1), (NVIDIA, 2), (NVIDIA, 3)], &[]);
        let (work, asked) = looked_up(vec![card(INTEL, 1), card(NVIDIA, 2), card(NVIDIA, 3)], &two_nvidia, |device| if device == 1 { two_nvidia.sleep(3) });
        assert_eq!(asked, [1]);
        assert!(work.cards[1].lookup == Lookup::PutOff && work.cards[2].lookup == Lookup::PutOff);
    }

    /// A card's maker's cards are those there now, not only those the sampler found as it was made: an NVIDIA card
    /// come since (a dock's), or one Windows' graphics list no adapter of, asleep, holds back the NVIDIA card the
    /// sampler has, which NVAPI would find only by asking it where it sits. Once it is awake, the card is looked up.
    #[test]
    fn a_card_come_since_holds_back_its_makers() {
        let docked = Machine::new(&[(NVIDIA, 2), (NVIDIA, 3)], &[3]);
        let (mut work, asked) = looked_up(vec![card(NVIDIA, 2)], &docked, |_| {});
        assert!(asked.is_empty());
        assert_eq!(work.cards[0].lookup, Lookup::PutOff);
        docked.wake(3);
        assert_eq!(again(&mut work, &docked), [2]);
    }

    /// A card looked up is read through its maker's library only where it is awake just before each call of it, after
    /// the cards and the calls before it: an NVIDIA card gone to sleep while Intel's graphics before it were read is
    /// asked nothing, and what was read of it before is so no longer, and a card gone to sleep as its power was read
    /// is asked nothing more. One not looked up is asked nothing.
    #[test]
    fn a_card_gone_to_sleep_while_another_was_read_is_not_asked() {
        let laptop = Machine::new(&[(INTEL, 1), (NVIDIA, 2), (AMD, 3)], &[3]);
        let (mut work, _) = looked_up(vec![card(INTEL, 1), card(NVIDIA, 2), card(AMD, 3)], &laptop, |_| {});
        assert_eq!(work.cards[2].lookup, Lookup::PutOff);
        let before = read_on(&work, &laptop);
        assert!(before[1].power.is_some());
        work.libraries.asked.take();
        work.libraries.meanwhile = Box::new(|device| if device == 1 { laptop.sleep(2) });
        let read = read_on(&work, &laptop);
        assert_eq!(work.libraries.asked.take(), [("power", 1), ("more", 1), ("limit", 1)]);
        assert_eq!(read[1], Readings::default());
        assert_eq!(read[0].power.map(|(_, watts)| watts), Some(Some(1.0)));
        laptop.wake(2);
        work.libraries.meanwhile = Box::new(|device| laptop.sleep(device));
        let read = read_on(&work, &laptop);
        assert_eq!(work.libraries.asked.take(), [("power", 1), ("power", 2)]);
        assert_eq!((read[0].power.map(|(_, watts)| watts), read[0].more.map(|(_, more)| more), read[0].limit.map(|(_, limit)| limit)), (Some(Some(1.0)), Some(More::default()), Some(None)));
    }

    /// Each reading is as old as the moment its reading began: one read before a call that took long is not taken for
    /// one read after it.
    #[test]
    fn each_reading_is_as_old_as_its_reading() {
        let machine = Machine::new(&[(INTEL, 1)], &[]);
        let (mut work, _) = looked_up(vec![card(INTEL, 1)], &machine, |_| {});
        work.libraries.meanwhile = Box::new(|_| std::thread::sleep(Duration::from_millis(30)));
        let read = read_on(&work, &machine);
        let when = work.libraries.when.take();
        let (power, more, limit) = (read[0].power.unwrap().0, read[0].more.unwrap().0, read[0].limit.unwrap().0);
        assert!(power <= when[0] && when[0] < more && more <= when[1] && when[1] < limit && limit <= when[2]);
    }

    /// Before the libraries' thread has come to them, the cards are said not looked for yet. Once looked up, how each
    /// is read (or that it is put off, one of its maker's asleep), and whether it is Intel's graphics in the CPU. A
    /// card whose place on the bus is not known is read by none, its library not asked.
    #[test]
    fn says_what_the_looking_up_came_to() {
        let mut nowhere = card(AMD, 4);
        nowhere.address = None;
        let cards = vec![card(INTEL, 1), card(NVIDIA, 2), nowhere];
        let before = found_before(&cards);
        assert_eq!(before.lines[0], "GPU power, card 1: not looked for yet (its maker's library is opened apart from the first look at the hardware)");
        assert_eq!(before.integrated, [false, false, false]);
        let machine = Machine::new(&[(INTEL, 1), (NVIDIA, 2), (AMD, 4)], &[2]);
        let (work, asked) = looked_up(cards, &machine, |_| {});
        assert_eq!(asked, [1]);
        let found = work.found();
        assert_eq!(found.lines, ["GPU power, card 1: IGCL", "GPU power, card 2: looked for once it and its maker's other cards are awake (one asleep)", "GPU power, card 4: not available"]);
        assert_eq!(found.integrated, [true, false, false]);
    }

    /// What each card's looking up came to is told as it is known, before the next card's library is asked (whose
    /// call may not come back): the graphics in the CPU said so, its line said, while the card after it is looked up.
    #[test]
    fn tells_each_card_as_it_is_looked_up() {
        let machine = Machine::new(&[(INTEL, 1), (INTEL, 2)], &[]);
        let told_before_the_second = Cell::new(None);
        let told = RefCell::new(Vec::new());
        let mut work = Work::new(standing(|device| if device == 2 { told_before_the_second.set(told.borrow().last().cloned()) }), vec![card(INTEL, 1), card(INTEL, 2)]);
        work.look_up(&machine, &mut |_| {}, &|| true, &mut |found| told.borrow_mut().push(found));
        let Some(Found { lines, integrated }) = told_before_the_second.take() else { panic!("told before the second card was looked up") };
        assert_eq!(lines, ["GPU power, card 1: IGCL", "GPU power, card 2: not looked for yet (its maker's library is opened apart from the first look at the hardware)"]);
        assert_eq!(integrated, [true, false]);
        assert_eq!(told.borrow().len(), 2);
        // No longer wanted, no card is looked up.
        let mut work = Work::new(standing(|_| {}), vec![card(INTEL, 1)]);
        assert!(work.look_up(&machine, &mut |_| {}, &|| false, &mut |_| {}).is_empty());
        assert_eq!(work.cards[0].lookup, Lookup::NotYet);
    }

    /// A slow first looking up is noted with each card's part in it, in ms, and what came of each (of one asleep, its
    /// library not asked, only what came of it).
    #[test]
    fn notes_a_slow_first_look_up_by_its_cards() {
        let machine = Machine::new(&[(AMD, 1), (NVIDIA, 2)], &[2]);
        let (work, _) = looked_up(vec![card(AMD, 1), card(NVIDIA, 2)], &machine, |_| {});
        assert_eq!(
            first_look_up_noted(Duration::from_millis(5470), &work.cards, &[(0, Duration::from_millis(5459))]),
            "GPU libraries: the GPUs were looked up in 5470 ms, apart from the first look at the hardware (card 1: IGCL (5459 ms), card 2: looked for once it and its maker's other cards are awake (one asleep))"
        );
    }

    /// What was read is taken only while it is recent, each reading by when its own reading began: one begun longer
    /// ago than it may be (the libraries slow, or stuck) is not, rather than what it was then shown as now, while those
    /// read since are. None recent, none of the card's is.
    #[test]
    fn takes_only_what_was_read_lately() {
        let now = Instant::now();
        let before = now - Duration::from_secs(3);
        let more = More { fan_rpm: Some(900), ..More::default() };
        let readings = Readings { power: Some((now, Some(3.0))), more: Some((before, more)), limit: Some((now, Some(GpuLimit::Power))) };
        let shared = Shared { read: vec![readings, Readings { power: Some((before, Some(9.0))), ..Readings::default() }], ..Shared::default() };
        let within = Duration::from_secs(2);
        assert_eq!(shared.read_within(within, now + within), [Some(Read { power: Some(3.0), more: More::default(), limit: Some(GpuLimit::Power) }), None]);
        assert_eq!(shared.read_within(within, now + Duration::from_millis(2001)), [None, None]);
        assert_eq!(shared.read_within(Duration::from_secs(4), now), [Some(Read { power: Some(3.0), more, limit: Some(GpuLimit::Power) }), Some(Read { power: Some(9.0), ..Read::default() })]);
    }

    /// A call into a library under way for longer than a reading may be is noted once, by the time it has taken awake,
    /// and noted again as it comes back. One that comes back in time is not noted at all.
    #[test]
    fn notes_a_call_stuck_and_its_coming_back() {
        let secs = Duration::from_secs;
        let mut shared = Shared::default();
        assert_eq!(shared.stuck(secs(1000)), None);
        assert_eq!(shared.calling(Some("reading card 1 through its maker's library".into()), secs(100)), None);
        assert_eq!(shared.stuck(secs(100) + crate::sensing::STUCK_AFTER), None);
        assert_eq!(
            shared.stuck(secs(101) + crate::sensing::STUCK_AFTER).as_deref(),
            Some("GPU libraries: reading card 1 through its maker's library has taken over 30 s, and what the makers' libraries read is left out till it comes back (the rest read as ever)")
        );
        assert_eq!(shared.stuck(secs(200)), None);
        assert_eq!(shared.calling(None, secs(400)).as_deref(), Some("GPU libraries: reading card 1 through its maker's library came back after 300 s"));
        assert_eq!(shared.calling(Some("looking up card 2 through its maker's library".into()), secs(400)), None);
        assert_eq!(shared.calling(None, secs(402)), None);
    }

    /// Libraries standing in for the makers' on a thread of their own: each step (a card's looking up, Intel's library
    /// asked of its graphics, a card's reading) waits for the test to let it go, at most a few seconds, and says it has begun. That they were opened is said too.
    struct Held {
        let_go: mpsc::Receiver<()>,
        begun: mpsc::Sender<&'static str>,
        hold_reads: bool,
    }

    impl Held {
        fn step(&self, what: &'static str, held: bool) {
            let _ = self.begun.send(what);
            if held {
                let _ = self.let_go.recv_timeout(Duration::from_secs(5));
            }
        }
    }

    impl Libraries for Held {
        fn reader(&mut self, _: u32, _: PciAddress, _: &mut dyn FnMut() -> bool) -> Option<Option<Reader>> {
            self.step("looked up", !self.hold_reads);
            Some(Some(Reader::Nvidia(0)))
        }

        fn power(&self, _: Reader, _: &mut dyn FnMut() -> bool) -> Option<f32> {
            self.step("read", self.hold_reads);
            Some(42.0)
        }

        fn more(&self, _: Reader, _: &mut dyn FnMut() -> bool) -> Option<More> {
            None
        }

        fn limit(&self, _: Reader, _: &mut dyn FnMut() -> bool) -> Option<GpuLimit> {
            Some(GpuLimit::Power)
        }

        fn intel_integrated(&self, _: Reader) -> bool {
            false
        }

        fn describe_intel(&self, _: &dyn Fn(PciAddress) -> bool) -> Option<String> {
            self.step("asked Intel's", !self.hold_reads);
            Some(String::new())
        }
    }

    /// Libraries held, on a thread of their own, for `hold_reads` (their reads) or not (their looking up, and Intel's
    /// library asked of its graphics): what lets
    /// them go, what says each step has begun (and "opened" as they are opened), and the libraries.
    fn held(hold_reads: bool) -> (mpsc::Sender<()>, mpsc::Receiver<&'static str>, GpuLibraries) {
        let (let_go, held) = mpsc::channel();
        let (begun, begins) = mpsc::channel();
        let libraries = GpuLibraries::working(vec![card(NVIDIA, 2)], AllAwake, move || {
            let _ = begun.send("opened");
            Held { let_go: held, begun, hold_reads }
        });
        (let_go, begins, libraries)
    }

    /// The next step begun, waited for up to 30 s (other tests may hold the libraries meanwhile), "opened" passed over.
    fn next_step(begins: &mpsc::Receiver<&'static str>) -> Option<&'static str> {
        std::iter::from_fn(|| begins.recv_timeout(Duration::from_secs(30)).ok()).find(|begun| *begun != "opened")
    }

    /// Waits up to 30 s (other tests may hold the libraries meanwhile) for `what` to come of `libraries`.
    fn awaited<T>(libraries: &GpuLibraries, what: impl Fn(&GpuLibraries) -> Option<T>) -> T {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(came) = what(libraries) {
                return came;
            }
            assert!(Instant::now() < until, "it came");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// What `libraries` read lately, if anything.
    fn read_lately(libraries: &GpuLibraries) -> Option<Vec<Option<Read>>> {
        Some(libraries.take(Duration::from_secs(60))).filter(|read| read.iter().any(Option::is_some))
    }

    /// The libraries' looking up, however long it takes, holds nothing that starts them: they start, are asked to read
    /// and are taken from at once, with nothing read yet. A card looked up, what came of it is said, while Intel's
    /// library is asked what it reads (which may not come back), and that once it is. Once read, what they read is
    /// taken.
    #[test]
    fn a_slow_look_up_holds_nothing() {
        let made = Instant::now();
        let (let_go, begins, libraries) = held(false);
        assert_eq!(read_lately(&libraries), None);
        assert!(made.elapsed() < Duration::from_secs(1), "not waited for: {:?}", made.elapsed());
        assert_eq!(next_step(&begins), Some("looked up"));
        // (The looking up under way, held.)
        let asked = Instant::now();
        assert_eq!(read_lately(&libraries), None);
        assert!(libraries.found().is_none());
        assert!(asked.elapsed() < Duration::from_secs(1), "not waited for: {:?}", asked.elapsed());
        let_go.send(()).unwrap();
        assert_eq!(next_step(&begins), Some("asked Intel's"));
        // (Intel's library asked, held.)
        assert_eq!(awaited(&libraries, GpuLibraries::found).lines, ["GPU power, card 2: NVML"]);
        let_go.send(()).unwrap();
        assert_eq!(awaited(&libraries, GpuLibraries::found).lines, ["GPU power, card 2: NVML", "Intel's graphics library: no graphics of Intel's"]);
        // (Each reading told as it is read, the card's power before what holds its clock back: all of them waited for.)
        let read = [Some(Read { power: Some(42.0), limit: Some(GpuLimit::Power), ..Read::default() })];
        awaited(&libraries, |libraries| read_lately(libraries).filter(|lately| *lately == read));
    }

    /// A reading under way, however long it takes, holds nothing that takes from the libraries: taken from at once,
    /// with nothing read yet. Once read, what they read is taken.
    #[test]
    fn a_slow_reading_holds_nothing() {
        let (let_go, begins, libraries) = held(true);
        assert!(awaited(&libraries, GpuLibraries::found).lines[0] == "GPU power, card 2: NVML");
        assert_eq!(read_lately(&libraries), None);
        assert_eq!(next_step(&begins), Some("looked up"));
        assert_eq!(next_step(&begins), Some("asked Intel's"));
        assert_eq!(next_step(&begins), Some("read"));
        // (The reading under way, held.)
        let asked = Instant::now();
        assert_eq!(read_lately(&libraries), None);
        assert!(asked.elapsed() < Duration::from_secs(1), "not waited for: {:?}", asked.elapsed());
        let_go.send(()).unwrap();
        assert!(awaited(&libraries, read_lately).len() == 1);
        // (A reading asked since is let go at once, the libraries' thread not left holding the other tests up.)
        drop(libraries);
        let _ = let_go.send(());
    }

    /// One thread of the process is in the libraries at a time: a second sampler's, made while the first's is in a
    /// call, asks its libraries nothing till the first's call has come back and that thread ended, its sampler gone.
    #[test]
    fn one_thread_in_the_libraries_at_a_time() {
        let (let_go, begins, first) = held(false);
        assert_eq!(next_step(&begins), Some("looked up"));
        let (second_go, second_begins, second) = held(false);
        drop(first);
        assert!(second_begins.recv_timeout(Duration::from_millis(500)).is_err(), "the second asked nothing meanwhile");
        let_go.send(()).unwrap();
        assert_eq!(next_step(&second_begins), Some("looked up"));
        // (Its looking up let go, and Intel's library asked after it.)
        second_go.send(()).unwrap();
        second_go.send(()).unwrap();
        drop(second);
    }

    /// A sampler gone while its libraries' thread waits for another's to be out of the libraries ends that thread at
    /// once, and its libraries are never opened: samplers made and let go of behind a call that does not come back
    /// leave no thread behind, nor one that goes into the libraries once it does.
    #[test]
    fn a_sampler_gone_while_its_thread_waits_leaves_nothing() {
        let (let_go, begins, first) = held(false);
        assert_eq!(next_step(&begins), Some("looked up"));
        let (_, waiting_begins, waiting) = held(false);
        let thread = Arc::downgrade(&waiting.shared);
        drop(waiting);
        let until = Instant::now() + Duration::from_secs(30);
        while thread.upgrade().is_some() {
            assert!(Instant::now() < until, "its thread ended");
            std::thread::sleep(Duration::from_millis(10));
        }
        let_go.send(()).unwrap();
        drop(first);
        assert!(waiting_begins.recv_timeout(Duration::from_millis(500)).is_err(), "its libraries never opened");
    }
}
