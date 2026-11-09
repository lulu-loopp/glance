//! The GPU makers' libraries (see `gpu_power`), worked on a thread of their own, the only one that calls into them:
//! each card looked up through its maker's library there, and read through it there, as the sampler asks, the sampler
//! taking what was last read without waiting for it. Opening a library and finding the cards through it may take
//! seconds while the drivers are still starting (5.5 s at a laptop's sign-in), and a driver's call may not come back
//! at all: neither holds the first look at the hardware, nor any other reading.
//!
//! One thread for all of them, look-ups and readings alike: NVML is documented as safe to call from several threads,
//! but NVAPI, ADL and IGCL say nothing of it, and a thread of its own each would still have a card's look-up and its
//! readings, or two cards' of one maker, meet in one library. Nor is a library called from two threads by two samplers
//! one after the other: a sampler's thread is the process's only one in them while it lasts (see `HELD`), one made
//! while another's is still in a library waiting for it to end.

use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::gpu_power::{GpuPower, More, PciAddress, Reader};
use crate::metrics::PowerNow;
use crate::reading::GpuLimit;

/// What the makers' libraries are asked: `GpuPower`, or what stands for it in a test.
pub trait Libraries {
    /// How to read the card made by `vendor` at `address`, as `GpuPower::reader` says.
    fn reader(&mut self, vendor: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>>;
    fn power(&self, reader: Reader) -> Option<f32>;
    fn more(&self, reader: Reader) -> Option<More>;
    fn limit(&self, reader: Reader) -> Option<GpuLimit>;
    fn intel_integrated(&self, reader: Reader) -> bool;
    /// What Intel's library found, for a report, as `GpuPower::describe_intel` says.
    fn describe_intel(&self, awake: &dyn Fn(PciAddress) -> bool) -> Option<String>;
}

impl Libraries for GpuPower {
    fn reader(&mut self, vendor: u32, address: PciAddress, awake: &mut dyn FnMut() -> bool) -> Option<Option<Reader>> {
        GpuPower::reader(self, vendor, address, awake)
    }

    fn power(&self, reader: Reader) -> Option<f32> {
        self.read(reader)
    }

    fn more(&self, reader: Reader) -> Option<More> {
        GpuPower::more(self, reader)
    }

    fn limit(&self, reader: Reader) -> Option<GpuLimit> {
        GpuPower::limit(self, reader)
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

/// What a card's maker's library read of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Read {
    /// Its power, in watts.
    pub power: Option<f32>,
    pub more: More,
    /// What holds its clock back (only NVIDIA's library says).
    pub limit: Option<GpuLimit>,
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

/// What the libraries' thread works on: the libraries, and the cards.
struct Work<L> {
    libraries: L,
    cards: Vec<Card>,
}

impl<L: Libraries> Work<L> {
    /// Each card not looked up yet looked up through its maker's library (see `GpuPower::reader`): once, and only while
    /// neither it nor any other card of its maker sleeps, as `power` says just before each step of it, its own state
    /// asked again first (the cards before it have been looked up since, which may have taken long). Neither IGCL nor
    /// NVAPI has a way to find one card but asking each of its maker's what it is (IGCL, its properties) or where it
    /// sits (NVAPI, its bus and its slot) till it comes to the one asked for, and IGCL, NVAPI and ADL list every one as
    /// they are opened: one asleep, an Intel or an AMD laptop's discrete GPU beside graphics of the same maker's in its
    /// CPU, would be asked along with the one awake. Its maker's cards are those there now (see
    /// `PowerNow::makers_awake`), not only those found as the sampler was made. One whose place on the bus is not known
    /// is read by none. Each card's library asked, `calling` is told of it first. Returns those whose library was
    /// asked, by their places among the cards, with how long it took.
    fn look_up(&mut self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>)) -> Vec<(usize, Duration)> {
        let mut asked = Vec::new();
        for (at, card) in self.cards.iter_mut().enumerate().filter(|(_, card)| !matches!(card.lookup, Lookup::Done(_))) {
            if power.sleeps(card.device) {
                card.lookup = Lookup::PutOff;
                continue;
            }
            let Some(address) = card.address else {
                card.lookup = Lookup::Done(None);
                continue;
            };
            calling(Some(format!("looking up {} through its maker's library", card.name)));
            let began = Instant::now();
            let vendor = card.vendor;
            card.lookup = self.libraries.reader(vendor, address, &mut || power.makers_awake(vendor)).map_or(Lookup::PutOff, Lookup::Done);
            asked.push((at, began.elapsed()));
        }
        calling(None);
        asked
    }

    /// Each card's readings through its maker's library, by its place among the cards: of a card looked up and awake,
    /// as `power` says just before it is read (after the cards before it were, whose libraries may have taken a
    /// while), and none of any other. Each card's library asked, `calling` is told of it first.
    fn read(&self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>)) -> Vec<Option<Read>> {
        let read = self
            .cards
            .iter()
            .map(|card| {
                let Lookup::Done(Some(reader)) = card.lookup else { return None };
                if power.sleeps(card.device) {
                    return None;
                }
                calling(Some(format!("reading {} through its maker's library", card.name)));
                Some(Read { power: self.libraries.power(reader), more: self.libraries.more(reader).unwrap_or_default(), limit: self.libraries.limit(reader) })
            })
            .collect();
        calling(None);
        read
    }

    /// What the cards' looking up has come to, for a report: each card's line, and Intel's library's, its graphics
    /// asked what they read only where `power` says they are awake just before (found awake, they may have gone to
    /// sleep since), and which are Intel's graphics in the CPU.
    fn found(&self, power: &impl PowerNow, calling: &mut dyn FnMut(Option<String>)) -> Found {
        let mut lines = lines_of(&self.cards);
        let awake = |at| self.cards.iter().find(|card| card.address == Some(at)).is_some_and(|card| !power.sleeps(card.device));
        calling(Some("asking Intel's library what it reads of its graphics".into()));
        if let Some(intel) = self.libraries.describe_intel(&awake) {
            lines.push(format!("Intel's graphics library: {}", if intel.is_empty() { "no graphics of Intel's".to_string() } else { intel }));
        }
        calling(None);
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

/// What the libraries' thread and the sampler share: what was last read, and when, what the cards' looking up came
/// to as it changes (none: taken already), and the call into a library under way.
#[derive(Default)]
struct Shared {
    read: Option<(Instant, Vec<Option<Read>>)>,
    found: Option<Found>,
    calling: Option<Calling>,
}

impl Shared {
    /// What was read, if it was read no longer than `within` before `now`.
    fn read_within(&self, within: Duration, now: Instant) -> Option<Vec<Option<Read>>> {
        self.read.as_ref().filter(|(at, _)| now.saturating_duration_since(*at) <= within).map(|(_, read)| read.clone())
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

/// Held by the libraries' thread for as long as it lasts: one thread of the process in the libraries at a time.
static HELD: Mutex<()> = Mutex::new(());

/// The GPU makers' libraries, worked on a thread of their own, which ends once this is dropped (and any call it is in
/// has come back).
pub struct GpuLibraries {
    shared: Arc<Mutex<Shared>>,
    ask: SyncSender<()>,
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
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (ask, asked) = mpsc::sync_channel::<()>(1);
        std::thread::Builder::new()
            .name("glance: GPU libraries".into())
            .spawn({
                let shared = shared.clone();
                move || {
                    // (Held, poisoned or not, till the libraries are closed as this ends.)
                    let _held = HELD.lock();
                    let mut calling = |what: Option<String>| {
                        let came_back = shared.lock().unwrap().calling(what, crate::sensing::awake());
                        if let Some(line) = came_back {
                            crate::journal::note(line);
                        }
                    };
                    let mut work = Work { libraries: open(), cards };
                    let began = Instant::now();
                    let asked_of = work.look_up(&power, &mut calling);
                    if began.elapsed() >= NOTED_FROM {
                        crate::journal::note(first_look_up_noted(began.elapsed(), &work.cards, &asked_of));
                    }
                    let found = work.found(&power, &mut calling);
                    shared.lock().unwrap().found = Some(found);
                    while asked.recv().is_ok() {
                        // A card put off looked up once it and its maker's other cards are all awake.
                        let mut looked = work.look_up(&power, &mut calling);
                        looked.retain(|(at, _)| matches!(work.cards[*at].lookup, Lookup::Done(_)));
                        for &(at, took) in &looked {
                            let card = &work.cards[at];
                            crate::journal::note(format!("GPU power, {}, looked for as it and its maker's other cards were awake: {} ({} ms)", card.name, card.lookup.describe(), took.as_millis()));
                        }
                        if !looked.is_empty() {
                            let found = work.found(&power, &mut calling);
                            shared.lock().unwrap().found = Some(found);
                        }
                        let read = work.read(&power, &mut calling);
                        shared.lock().unwrap().read = Some((Instant::now(), read));
                    }
                }
            })
            .expect("a thread started");
        GpuLibraries { shared, ask }
    }

    /// What was last read of each card, by its place among the cards, if it was read within `within`, another reading
    /// asked (one asked and not yet begun is not asked twice). Never waited for.
    pub fn take(&self, within: Duration) -> Option<Vec<Option<Read>>> {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
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

    /// Libraries standing in for the makers': each takes its step only as it is told its maker's cards are awake, as
    /// `GpuPower::reader` does, and does `meanwhile` as it does (what goes to sleep as it takes its time), the devices
    /// of the cards it was asked of (looked up, or read) written down in order. Each card is read as one of Intel's
    /// library's, numbered by its device, and drawing as many watts: device 1's the graphics in the CPU.
    struct Standing<'a> {
        meanwhile: Box<dyn Fn(u32) + 'a>,
        asked: RefCell<Vec<(&'static str, u32)>>,
    }

    fn standing<'a>(meanwhile: impl Fn(u32) + 'a) -> Standing<'a> {
        Standing { meanwhile: Box::new(meanwhile), asked: RefCell::new(Vec::new()) }
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

        fn power(&self, reader: Reader) -> Option<f32> {
            let Reader::Intel(device) = reader else { return None };
            self.asked.borrow_mut().push(("read", device as u32));
            (self.meanwhile)(device as u32);
            Some(device as f32)
        }

        fn more(&self, _: Reader) -> Option<More> {
            None
        }

        fn limit(&self, _: Reader) -> Option<GpuLimit> {
            None
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
        let mut work = Work { libraries: standing(meanwhile), cards };
        work.look_up(machine, &mut |_| {});
        let asked = work.libraries.asked.take().into_iter().map(|(_, device)| device).collect();
        (work, asked)
    }

    /// Looked up again, on `machine`: the devices of those asked.
    fn again(work: &mut Work<Standing>, machine: &Machine) -> Vec<u32> {
        work.look_up(machine, &mut |_| {});
        work.libraries.asked.take().into_iter().map(|(_, device)| device).collect()
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

    /// A card looked up is read through its maker's library only where it is awake just before, after the cards before
    /// it were read: an NVIDIA card gone to sleep while Intel's graphics before it were read is asked nothing, and has
    /// no readings. One not looked up has none, and is asked nothing.
    #[test]
    fn a_card_gone_to_sleep_while_another_was_read_is_not_asked() {
        let laptop = Machine::new(&[(INTEL, 1), (NVIDIA, 2), (AMD, 3)], &[3]);
        let (mut work, _) = looked_up(vec![card(INTEL, 1), card(NVIDIA, 2), card(AMD, 3)], &laptop, |_| {});
        assert_eq!(work.cards[2].lookup, Lookup::PutOff);
        work.libraries.meanwhile = Box::new(|device| if device == 1 { laptop.sleep(2) });
        let read = work.read(&laptop, &mut |_| {});
        assert_eq!(work.libraries.asked.take(), [("read", 1)]);
        assert_eq!(read, [Some(Read { power: Some(1.0), ..Read::default() }), None, None]);
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
        let found = work.found(&machine, &mut |_| {});
        assert_eq!(found.lines, ["GPU power, card 1: IGCL", "GPU power, card 2: looked for once it and its maker's other cards are awake (one asleep)", "GPU power, card 4: not available"]);
        assert_eq!(found.integrated, [true, false, false]);
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

    /// What was read is taken only while it is recent: read longer ago than it may be (the libraries slow, or stuck),
    /// none is, rather than what it was then shown as now.
    #[test]
    fn takes_only_what_was_read_lately() {
        let now = Instant::now();
        let read = vec![Some(Read { power: Some(3.0), ..Read::default() })];
        let shared = Shared { read: Some((now, read.clone())), ..Shared::default() };
        assert_eq!(shared.read_within(Duration::from_secs(2), now + Duration::from_secs(2)), Some(read));
        assert_eq!(shared.read_within(Duration::from_secs(2), now + Duration::from_millis(2001)), None);
        assert_eq!(Shared::default().read_within(Duration::from_secs(2), now), None);
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

    /// Libraries standing in for the makers' on a thread of their own: each step (a card's looking up, its reading)
    /// waits for the test to let it go, at most a few seconds, and says it has begun.
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

        fn power(&self, _: Reader) -> Option<f32> {
            self.step("read", self.hold_reads);
            Some(42.0)
        }

        fn more(&self, _: Reader) -> Option<More> {
            None
        }

        fn limit(&self, _: Reader) -> Option<GpuLimit> {
            Some(GpuLimit::Power)
        }

        fn intel_integrated(&self, _: Reader) -> bool {
            false
        }

        fn describe_intel(&self, _: &dyn Fn(PciAddress) -> bool) -> Option<String> {
            None
        }
    }

    /// Libraries held, on a thread of their own, for `hold_reads` (their reads) or not (their looking up): what lets
    /// them go, what says each step has begun, and the libraries.
    fn held(hold_reads: bool) -> (mpsc::Sender<()>, mpsc::Receiver<&'static str>, GpuLibraries) {
        let (let_go, held) = mpsc::channel();
        let (begun, begins) = mpsc::channel();
        let libraries = GpuLibraries::working(vec![card(NVIDIA, 2)], AllAwake, move || Held { let_go: held, begun, hold_reads });
        (let_go, begins, libraries)
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

    /// The libraries' looking up, however long it takes, holds nothing that starts them: they start, are asked to read
    /// and are taken from at once, with nothing read yet. Looked up, what came of it is said, and once read, what they
    /// read is taken.
    #[test]
    fn a_slow_look_up_holds_nothing() {
        let made = Instant::now();
        let (let_go, begins, libraries) = held(false);
        assert_eq!(libraries.take(Duration::from_secs(60)), None);
        assert!(made.elapsed() < Duration::from_secs(1), "not waited for: {:?}", made.elapsed());
        assert!(begins.recv_timeout(Duration::from_secs(30)).is_ok_and(|begun| begun == "looked up"));
        // (The looking up under way, held.)
        let asked = Instant::now();
        assert_eq!(libraries.take(Duration::from_secs(60)), None);
        assert!(libraries.found().is_none());
        assert!(asked.elapsed() < Duration::from_secs(1), "not waited for: {:?}", asked.elapsed());
        let_go.send(()).unwrap();
        assert_eq!(awaited(&libraries, GpuLibraries::found).lines, ["GPU power, card 2: NVML"]);
        let read = awaited(&libraries, |libraries| libraries.take(Duration::from_secs(60)));
        assert_eq!(read, [Some(Read { power: Some(42.0), limit: Some(GpuLimit::Power), ..Read::default() })]);
    }

    /// A reading under way, however long it takes, holds nothing that takes from the libraries: taken from at once,
    /// with nothing read yet. Once read, what they read is taken.
    #[test]
    fn a_slow_reading_holds_nothing() {
        let (let_go, begins, libraries) = held(true);
        assert!(awaited(&libraries, GpuLibraries::found).lines == ["GPU power, card 2: NVML"]);
        assert_eq!(libraries.take(Duration::from_secs(60)), None);
        assert!(begins.recv_timeout(Duration::from_secs(30)).is_ok_and(|begun| begun == "looked up"));
        assert!(begins.recv_timeout(Duration::from_secs(30)).is_ok_and(|begun| begun == "read"));
        // (The reading under way, held.)
        let asked = Instant::now();
        assert_eq!(libraries.take(Duration::from_secs(60)), None);
        assert!(asked.elapsed() < Duration::from_secs(1), "not waited for: {:?}", asked.elapsed());
        let_go.send(()).unwrap();
        assert!(awaited(&libraries, |libraries| libraries.take(Duration::from_secs(60))).len() == 1);
        // (A reading asked since is let go at once, the libraries' thread not left holding the other tests up.)
        drop(libraries);
        let _ = let_go.send(());
    }

    /// One thread of the process is in the libraries at a time: a second sampler's, made while the first's is in a
    /// call, asks its libraries nothing till the first's call has come back and that thread ended, its sampler gone.
    #[test]
    fn one_thread_in_the_libraries_at_a_time() {
        let (let_go, begins, first) = held(false);
        assert!(begins.recv_timeout(Duration::from_secs(30)).is_ok_and(|begun| begun == "looked up"));
        let (second_go, second_begins, second) = held(false);
        drop(first);
        assert!(second_begins.recv_timeout(Duration::from_millis(500)).is_err(), "the second asked nothing meanwhile");
        let_go.send(()).unwrap();
        assert!(second_begins.recv_timeout(Duration::from_secs(30)).is_ok_and(|begun| begun == "looked up"));
        second_go.send(()).unwrap();
        drop(second);
    }
}
