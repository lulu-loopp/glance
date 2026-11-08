//! Searching this PC's files by name, as quickly as Everything does, with the user's own rights: every drive's
//! folders listed once (kept on disk after, and read from there: see `store`), then kept current as its change
//! journal says which folders changed (or, on a drive with none, as Windows tells of names changed while it is
//! watched). The user finds what they may list, and nothing else: no drive is opened whole, nothing is asked of the
//! sensors.
//!
//! Each drive's index is changed by one thread alone, its own (see `follow`), which keeps it anew on disk between
//! its rounds of listing (never in one, whose entries' places it holds), as what changed since it was last kept
//! grows, now and then, and as Glance quits.
//!
//! Not begun till the machine has settled after it started (see `settle`), or till search is called up, all of it on
//! threads in the background, listing through the machine's one pool (see `walk::Pool`).
//!
//! A drive listed whole (its first listing, or again, its changes lost) is listed in rounds (see `sweep`), its index
//! kept between them with where its journal was before the listing began (taken before it began, and kept with each
//! round's index: every folder listed since that place, or marked to be), and every folder not yet listed marked to be
//! (`flag::STALE`, as each folder found is till it is listed). The PC turned off as it is listed, Glance started again
//! reads the index kept on from there while the journal still holds all since that place (else it is listed whole
//! again, from where the journal is then): the folders marked are listed, those listed are not again, and what
//! changed in those meanwhile is read from the journal after, from that place. A change in a folder the index does not
//! know is passed over (a path is never made through a folder it does not know): the folder is in one not yet listed,
//! or was made since in one listed, whose change marks it, and is found whole as that one is listed.

pub mod drive;
pub mod index;
pub mod journal;
pub mod query;
mod readings;
mod store;
pub mod walk;
mod watch;

use drive::Drive;
use index::{flag, Index};
pub use query::{Hit, Only, Query};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once, RwLock};
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

/// GetDriveTypeW's removable and fixed drives (others, the network's and CDs, are not searched).
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// How often the drives there are looked at again, the journal read (or what was told of a drive with none), and an
/// index kept that changed.
const DRIVES_EVERY: Duration = Duration::from_secs(5);
const JOURNAL_EVERY: Duration = Duration::from_secs(1);
const KEEP_EVERY: Duration = Duration::from_secs(10 * 60);
/// How much of a journal's records is read at once (the rest at the next turn, a second after).
const JOURNAL_BATCH: usize = 4 << 20;
/// How much memory what changed since an index was kept may hold before it is kept anew.
const HELD_MOST: usize = 16 << 20;
/// How long an index not kept (the disk full, or failing) waits to be tried again: twice as long after each time it
/// fails, up to `KEEP_EVERY`.
const KEEP_RETRY: Duration = Duration::from_secs(60);
/// The shortest round of a drive's listing whole (see `sweep`).
const ROUND_LEAST: Duration = Duration::from_secs(30);
/// How long Glance quitting waits for the indexes to be kept.
const QUIT_WAIT: Duration = Duration::from_secs(3);

/// A drive being searched.
pub struct Volume {
    pub drive: Drive,
    index: RwLock<Index>,
    /// How many names it held as it was last asked (told while its index is being changed: see `standing`).
    names: AtomicUsize,
    ready: AtomicBool,
    gone: AtomicBool,
    /// What its index is kept with (its journal and where it has come to, where it has one).
    kept: Mutex<Option<store::Kept>>,
    /// Asked to be kept (Glance quitting), and how many times it was kept or found to have nothing to keep since.
    keep_asked: AtomicBool,
    kept_times: AtomicU64,
}

impl Volume {
    fn new(drive: Drive, index: Index) -> Volume {
        Volume { drive, index: RwLock::new(index), names: AtomicUsize::new(0), ready: AtomicBool::new(false), gone: AtomicBool::new(false), kept: Mutex::new(None), keep_asked: AtomicBool::new(false), kept_times: AtomicU64::new(0) }
    }

    /// Whether it is no longer followed: gone, or let go of as Windows asked to take it out.
    fn away(&self) -> bool {
        self.gone.load(Ordering::Relaxed) || self.drive.left()
    }
}

/// How a drive's search stands.
#[derive(Clone, Debug)]
pub struct Standing {
    pub root: String,
    /// Whether it is listed whole (else names are still being found).
    pub ready: bool,
    pub names: usize,
}

/// What was found.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// How many names meet what was typed, and how many of them are folders' (the best of them in `hits`).
    pub total: usize,
    pub folders: usize,
    pub hits: Vec<Hit>,
}

static VOLUMES: Mutex<Vec<Arc<Volume>>> = Mutex::new(Vec::new());

/// Starts finding the names on every drive, and keeping them current, on threads of its own, once the machine has
/// settled (once, and again it does nothing).
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("glance: search drives".into()).spawn(|| {
            crate::settle::background("search: drives");
            crate::settle::wait(crate::settle::Work::Search);
            drives();
        });
    });
}

/// Search called up: its drives found now, settled or not.
pub fn asked() {
    crate::settle::want(crate::settle::Work::Search);
    start();
}

/// The drives there are, looked at now and then: a drive come is searched, one gone is not any more.
fn drives() {
    loop {
        let present = unsafe { GetLogicalDrives() };
        let roots: Vec<String> = (0..26u8)
            .filter(|bit| present & (1 << bit) != 0)
            .map(|bit| format!("{}:\\", (b'A' + bit) as char))
            .filter(|root| matches!(unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) }, DRIVE_FIXED | DRIVE_REMOVABLE))
            .collect();
        // Each drive asked without the list held (this thread alone changes it): a drive slow to answer holds none
        // searching meanwhile.
        let followed = VOLUMES.lock().unwrap().clone();
        let staying: Vec<Arc<Volume>> = followed
            .into_iter()
            .filter(|volume| {
                // (One let go of as Windows asked to take it out is not opened again while it is there: the asking
                // failed, it is followed anew, and taken out, it is forgotten, to be followed anew as it comes back.)
                let there = if volume.drive.left() {
                    !volume.drive.stays() && !volume.drive.removed() && roots.contains(&volume.drive.path)
                } else {
                    // (Asked through the root it holds, not opened anew: what letting it go closes is all it holds.)
                    roots.contains(&volume.drive.path) && volume.drive.with_root(drive::serial) == Some(Some(volume.drive.serial))
                };
                if !there {
                    volume.gone.store(true, Ordering::Relaxed);
                }
                there
            })
            .collect();
        let mut come = Vec::new();
        for root in roots {
            if staying.iter().any(|volume| volume.drive.path == root) {
                continue;
            }
            let Ok(mut drive) = Drive::open(&root) else { continue };
            if !drive.hear_removal() {
                crate::journal::note(format!("search: {root} followed, its removal not heard (it may not be taken out safely while Glance runs)"));
            }
            let index = Index::new(drive.root_id, drive.by_id());
            come.push(Arc::new(Volume::new(drive, index)));
        }
        *VOLUMES.lock().unwrap() = staying.into_iter().chain(come.iter().cloned()).collect();
        for volume in come {
            let _ = std::thread::Builder::new().name(format!("glance: search {}", &volume.drive.path[..1])).spawn(move || follow(&volume));
        }
        std::thread::sleep(DRIVES_EVERY);
    }
}

/// A drive's names found (or read as kept, a listing cut short gone on with), then kept current till it is gone, or
/// let go of.
fn follow(volume: &Arc<Volume>) {
    crate::settle::background("search: following a drive");
    let drive = &volume.drive;
    let pool = walk::pool();
    let journal = if drive.by_id() { journal::query(drive).ok() } else { None };
    let kept = store::kept(drive.serial);
    let mut keeping = Keeping::new();
    if let Some(journal) = journal {
        // (Where the journal is now, for an index not read on from where it was kept: what changes while its folders
        // are listed is read after.)
        let from_now = store::Kept { serial: drive.serial, journal: Some(store::Cursor { journal: journal.id, next: journal.next }), whole: false };
        let kept = kept.filter(|(index, _)| index.by_id());
        let whole = match (begun(kept.as_ref().map(|(_, kept)| kept), &journal), kept) {
            // (Its folders marked to be listed listed first: a first listing cut short goes on.)
            (Begun::ReadOn, Some((index, kept))) => {
                *volume.index.write().unwrap() = index;
                *volume.kept.lock().unwrap() = Some(kept);
                volume.ready.store(kept.whole, Ordering::Relaxed);
                !kept.whole
            }
            // Searched at once as it was kept, every folder listed again behind it.
            (Begun::Relisted, Some((index, kept))) => {
                *volume.index.write().unwrap() = index;
                *volume.kept.lock().unwrap() = Some(store::Kept { whole: kept.whole, ..from_now });
                volume.ready.store(kept.whole, Ordering::Relaxed);
                mark_all(volume);
                true
            }
            _ => {
                *volume.kept.lock().unwrap() = Some(from_now);
                true
            }
        };
        sweep(volume, pool, &mut keeping, whole);
        follow_journal(volume, pool, keeping);
    } else {
        let (tell, told) = mpsc::channel();
        let watched = volume.clone();
        let _ = std::thread::Builder::new().name(format!("glance: search watch {}", &drive.path[..1])).spawn(move || {
            crate::settle::background("search: watching a drive");
            watch::watch(&watched.drive, tell);
        });
        // What was kept found at once, and listed again (nothing kept says what changed while Glance was not
        // running); else all of it listed.
        let whole = match kept.filter(|(index, kept)| index.by_id() == drive.by_id() && kept.journal.is_none()) {
            Some((index, kept)) => {
                *volume.index.write().unwrap() = index;
                volume.ready.store(kept.whole, Ordering::Relaxed);
                mark_all(volume);
                kept.whole
            }
            None => false,
        };
        *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: None, whole });
        sweep(volume, pool, &mut keeping, true);
        follow_watch(volume, &told, pool, keeping);
    }
}

/// How a drive with a journal is begun on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Begun {
    /// Its kept index read on from where it was kept, its journal holding all that changed since.
    ReadOn,
    /// Its kept index searched at once and every folder listed again: the journal no longer holds all that changed
    /// since it was kept (Glance not running for long, or the journal made anew).
    Relisted,
    /// Listed whole: nothing of it kept.
    Listed,
}

/// How a drive is begun on, by what was kept of its index (one kept as the drive lists by id) and its journal now.
fn begun(kept: Option<&store::Kept>, journal: &journal::Journal) -> Begun {
    match kept {
        None => Begun::Listed,
        Some(kept) if kept.journal.is_some_and(|cursor| cursor.journal == journal.id && cursor.next >= journal.first && cursor.next <= journal.next) => Begun::ReadOn,
        Some(_) => Begun::Relisted,
    }
}

/// The live folders (followed: no links, each known by its id) marked to be listed, by their places, but those whose
/// listing `failed`.
fn marked(index: &Index, failed: &HashSet<u64>) -> Vec<u32> {
    (0..index.len())
        .filter(|&place| index.entry(place).alive() && index.entry(place).flags & (flag::FOLDER | flag::LINK | flag::STALE) == flag::FOLDER | flag::STALE)
        .filter(|&place| index.id(place).is_some_and(|id| !failed.contains(&id)))
        .collect()
}

/// Every live folder of the drive (followed: no links) marked to be listed: its changes lost, it is listed whole
/// again (see `sweep`).
fn mark_all(volume: &Volume) {
    let mut index = volume.index.write().unwrap();
    let folders: Vec<u32> = (0..index.len()).filter(|&place| index.entry(place).alive() && index.entry(place).flags & (flag::FOLDER | flag::LINK) == flag::FOLDER).collect();
    for place in folders {
        index.set_flag(place, flag::STALE, true);
    }
}

/// Every folder marked to be listed listed (`whole`: the drive's first listing, or its listing again whole, or else
/// those left marked as the index was kept), and what is found in them in turn, in rounds: each as long as all before
/// it together (`ROUND_LEAST` at least, `KEEP_EVERY` at most), so that keeping the index between them costs at most about
/// as much again as keeping it once at the end, and a listing cut short loses at most about half of what it had done.
/// The index is kept between rounds on a drive with a journal (see the module's note: a drive without one is listed
/// whole again as Glance starts, whatever was kept), then whole, and kept so where it was listed whole (else as it is
/// due). The journal is read after, not between rounds: a folder changing every second, listed again in each, would
/// never let it end. Folders whose listing failed are not tried again in it (they are after: see `Pacing`). How many
/// folders it listed.
fn sweep(volume: &Volume, pool: &walk::Pool, keeping: &mut Keeping, whole: bool) -> usize {
    let mut failed: HashSet<u64> = HashSet::new();
    let (mut spent, mut listed) = (Duration::ZERO, 0);
    loop {
        let folders = marked(&volume.index.read().unwrap(), &failed);
        if folders.is_empty() {
            break;
        }
        let begun = Instant::now();
        let round = walk::round(&volume.index, &volume.drive, &folders, pool, Some(begun + spent.clamp(ROUND_LEAST, KEEP_EVERY)));
        spent += begun.elapsed();
        listed += round.folders;
        failed.extend(round.failed);
        keeping.changed = true;
        if volume.away() {
            return listed;
        }
        let journaled = volume.kept.lock().unwrap().is_some_and(|kept| kept.journal.is_some());
        if journaled && !marked(&volume.index.read().unwrap(), &failed).is_empty() {
            keeping.keep(volume, true);
        }
    }
    if let Some(kept) = volume.kept.lock().unwrap().as_mut() {
        kept.whole = true;
    }
    volume.ready.store(true, Ordering::Relaxed);
    keeping.keep(volume, whole);
    listed
}

/// When a drive's index was last kept, whether it changed since, and, where keeping it failed, when it is tried
/// again and how long that waited.
struct Keeping {
    at: Instant,
    changed: bool,
    retry: Option<(Instant, Duration)>,
}

impl Keeping {
    fn new() -> Keeping {
        Keeping { at: Instant::now(), changed: false, retry: None }
    }

    /// Whether it is to be kept `now`: Glance quitting (`asked`), whatever failed before, or else, not before it is
    /// tried again after failing, where that is asked for (`forced`), it is a while since, or what changed since
    /// holds much memory (`held`) or many dead.
    fn due(&self, now: Instant, forced: bool, asked: bool, held: usize, untidy: bool) -> bool {
        if asked {
            return true;
        }
        if self.retry.is_some_and(|(after, _)| now < after) {
            return false;
        }
        forced || now.duration_since(self.at) >= KEEP_EVERY || held > HELD_MOST || untidy
    }

    /// Keeping it failed `now`: tried again after `KEEP_RETRY`, twice as long each time it failed since it was last
    /// kept, `KEEP_EVERY` at most.
    fn failed(&mut self, now: Instant) {
        let wait = self.retry.map_or(KEEP_RETRY, |(_, wait)| (wait * 2).min(KEEP_EVERY));
        self.retry = Some((now + wait, wait));
    }

    fn kept(&mut self, now: Instant) {
        (self.at, self.changed, self.retry) = (now, false, None);
    }

    /// The index kept anew (it is between rounds of listing), where it changed and that is due (see `due`, `now`
    /// asked for).
    fn keep(&mut self, volume: &Volume, now: bool) {
        // (A drive let go of is not kept anew: what was listed as it went may be partly listed. What was kept before
        // stands, and what changed since is read from its journal as it comes back.)
        if volume.drive.left() {
            return;
        }
        let asked = volume.keep_asked.swap(false, Ordering::Relaxed);
        let due = {
            let index = volume.index.read().unwrap();
            self.due(Instant::now(), now, asked, index.held(), index.untidy())
        };
        if self.changed && due {
            if let Some(kept) = *volume.kept.lock().unwrap() {
                // (Written while it is searched, as nothing else changes it: in its place once read back.)
                let written = {
                    let index = volume.index.read().unwrap();
                    store::keep(&index, kept)
                };
                match written {
                    Ok(index) => {
                        *volume.index.write().unwrap() = index;
                        store::let_go(volume.drive.serial);
                        self.kept(Instant::now());
                    }
                    Err(error) => {
                        crate::journal::note(format!("search: {} not kept: {error}", volume.drive.path));
                        self.failed(Instant::now());
                    }
                }
            }
        }
        if asked {
            volume.kept_times.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A drive kept current as its journal says which folders changed.
fn follow_journal(volume: &Volume, pool: &walk::Pool, mut keeping: Keeping) {
    let drive = &volume.drive;
    let mut pacing = Pacing::new(volume);
    while !volume.away() {
        std::thread::sleep(JOURNAL_EVERY);
        let Some(cursor) = volume.kept.lock().unwrap().and_then(|kept| kept.journal) else { return };
        match journal::read(drive, cursor.journal, cursor.next, JOURNAL_BATCH) {
            Ok(journal::Read::Changed { folders, next, .. }) => {
                if pacing.list(volume, folders, pool) {
                    keeping.changed = true;
                }
                // (Only once what changed is in the index, or marked in it to be listed again: kept now, it holds all
                // before `next`.)
                if let Some(kept) = volume.kept.lock().unwrap().as_mut() {
                    kept.journal = Some(store::Cursor { next, ..cursor });
                }
            }
            Ok(journal::Read::Lost) => {
                // Changes missed: everything listed again, from where the journal is now.
                if let Ok(journal) = journal::query(drive) {
                    if let Some(kept) = volume.kept.lock().unwrap().as_mut() {
                        kept.journal = Some(store::Cursor { journal: journal.id, next: journal.next });
                    }
                    mark_all(volume);
                    sweep(volume, pool, &mut keeping, true);
                    pacing = Pacing::new(volume);
                }
            }
            // (The drive going, or its journal stopped: looked at again next time.)
            Err(_) => {}
        }
        keeping.keep(volume, false);
    }
}

/// What was seen, with all told since that waits: the names changed in all their folders, or, where more changed
/// than Windows kept in any, the drive to be listed whole again (once, for all of them: what was told before it is
/// found as it is listed).
fn coalesce(seen: watch::Seen, told: &mpsc::Receiver<watch::Seen>) -> watch::Seen {
    let mut seen = seen;
    while let Ok(more) = told.try_recv() {
        seen = match (seen, more) {
            (watch::Seen::Folders(mut paths), watch::Seen::Folders(more)) => {
                paths.extend(more);
                watch::Seen::Folders(paths)
            }
            _ => watch::Seen::Lost,
        };
    }
    if let watch::Seen::Folders(paths) = &mut seen {
        paths.sort();
        paths.dedup();
    }
    seen
}

/// A drive with no journal kept current as Windows tells of names changed in its folders.
fn follow_watch(volume: &Volume, told: &mpsc::Receiver<watch::Seen>, pool: &walk::Pool, mut keeping: Keeping) {
    let mut pacing = Pacing::new(volume);
    while !volume.away() {
        let changed = match told.recv_timeout(JOURNAL_EVERY).map(|seen| coalesce(seen, told)) {
            Ok(watch::Seen::Folders(paths)) => {
                let folders: Vec<u64> = {
                    let index = volume.index.read().unwrap();
                    paths.iter().filter_map(|path| index.id(index.folder_at(path))).collect()
                };
                pacing.list(volume, folders, pool)
            }
            // (Kept as it is listed.)
            Ok(watch::Seen::Lost) => {
                mark_all(volume);
                sweep(volume, pool, &mut keeping, true);
                pacing = Pacing::new(volume);
                false
            }
            Err(mpsc::RecvTimeoutError::Timeout) => pacing.list(volume, Vec::new(), pool),
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        if changed {
            keeping.changed = true;
        }
        keeping.keep(volume, false);
    }
}

/// How many names a second a large folder that keeps changing is listed again at, at most: one of 110,000 names
/// here (the user's temporary files, written to every second) took a tenth of a core listed every second.
const NAMES_A_SECOND: u64 = 10_000;
/// The longest a folder that changed waits to be listed again.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// Folders that changed, each listed again as soon as it may be: a large one only a while after it was last listed
/// (the longer, the more names it holds), what changes in it meanwhile listed with it then.
#[derive(Default)]
struct Pacing {
    /// The large folders listed lately, by their ids, and when each may be listed again.
    not_before: HashMap<u64, Instant>,
    /// Those waiting (put off, or their listing failed), by their ids.
    waiting: HashSet<u64>,
}

impl Pacing {
    /// Begun with the folders that were waiting as the index was kept (marked in it).
    fn new(volume: &Volume) -> Self {
        let index = volume.index.read().unwrap();
        let waiting = (0..index.len()).filter(|&place| index.entry(place).alive() && index.entry(place).flags & flag::STALE != 0).filter_map(|place| index.id(place)).collect();
        Pacing { not_before: HashMap::new(), waiting }
    }

    /// How long a folder of `names` names waits to be listed again.
    fn wait(names: usize) -> Duration {
        Duration::from_millis(names as u64 * 1000 / NAMES_A_SECOND).min(LONGEST_WAIT)
    }

    /// The folders that `changed` (by their ids) put with those waiting: those that may be listed `now`, and those
    /// left waiting.
    fn take(&mut self, changed: Vec<u64>, now: Instant) -> (Vec<u64>, Vec<u64>) {
        self.not_before.retain(|_, at| *at > now);
        self.waiting.extend(changed);
        let (later, due): (Vec<u64>, Vec<u64>) = self.waiting.drain().partition(|id| self.not_before.contains_key(id));
        self.waiting.extend(&later);
        (due, later)
    }

    /// What a round of listing begun `now` found: the large folders it listed wait, those that failed are tried again.
    fn listed(&mut self, now: Instant, listed: walk::Listed) {
        for (id, names) in listed.large {
            self.not_before.insert(id, now + Pacing::wait(names));
        }
        self.waiting.extend(listed.failed);
    }

    /// The folders that `changed` (by their ids) and those waiting listed, those that may be now: the rest left
    /// waiting, marked in the index (kept so, they are listed as Glance starts again). Whether the index changed.
    fn list(&mut self, volume: &Volume, changed: Vec<u64>, pool: &walk::Pool) -> bool {
        let now = Instant::now();
        let (due, later) = self.take(changed, now);
        let mut marked = false;
        {
            let mut index = volume.index.write().unwrap();
            let places: Vec<u32> = later.iter().filter_map(|&id| index.folder(id)).collect();
            for place in places {
                if index.entry(place).flags & flag::STALE == 0 {
                    index.set_flag(place, flag::STALE, true);
                    marked = true;
                }
            }
        }
        if due.is_empty() {
            return marked;
        }
        let places: Vec<u32> = {
            let index = volume.index.read().unwrap();
            due.iter().filter_map(|&id| index.folder(id)).collect()
        };
        let listed = walk::round(&volume.index, &volume.drive, &places, pool, None);
        self.listed(now, listed);
        true
    }
}

/// Every index that changed kept on disk (as Glance quits): asked of each drive's own thread, and waited for a
/// while (one in a round of listing keeps it after it; one not kept in time has its journal read again from where it
/// was last kept).
pub fn keep_all() {
    let volumes: Vec<(Arc<Volume>, u64)> = VOLUMES.lock().unwrap().iter().filter(|volume| volume.ready.load(Ordering::Relaxed)).map(|volume| (volume.clone(), volume.kept_times.load(Ordering::Relaxed))).collect();
    for (volume, _) in &volumes {
        volume.keep_asked.store(true, Ordering::Relaxed);
    }
    let until = Instant::now() + QUIT_WAIT;
    while Instant::now() < until && volumes.iter().any(|(volume, times)| volume.kept_times.load(Ordering::Relaxed) == *times && !volume.away()) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// How each drive's search stands: its names counted as it is now, or, while its index is being changed (its thread
/// in the background, maybe waiting for the processors: the window asking is not held for it), as last counted.
pub fn standing() -> Vec<Standing> {
    let volumes = VOLUMES.lock().unwrap().clone();
    volumes
        .iter()
        .map(|volume| {
            let names = match volume.index.try_read() {
                Ok(index) => {
                    let names = index.names();
                    volume.names.store(names, Ordering::Relaxed);
                    names
                }
                Err(_) => volume.names.load(Ordering::Relaxed),
            };
            Standing { root: volume.drive.path.clone(), ready: volume.ready.load(Ordering::Relaxed), names }
        })
        .collect()
}

/// The names meeting `query` on every drive: how many there are, how many are folders', and the best `most` folders
/// and the best `most` files, best first.
pub fn find(query: &Query, most: usize) -> Found {
    if query.is_empty() {
        return Found::default();
    }
    let volumes = VOLUMES.lock().unwrap().clone();
    let mut found = Found::default();
    for volume in &volumes {
        let index = volume.index.read().unwrap();
        let (total, folders, hits) = query.find(&index, &volume.drive.path, most);
        found.total += total;
        found.folders += folders;
        found.hits.extend(hits);
    }
    query::sort(&mut found.hits);
    let (mut folders, mut files) = (0, 0);
    found.hits.retain(|hit| {
        let shown = if hit.folder { &mut folders } else { &mut files };
        *shown += 1;
        *shown <= most
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use index::ROOT;
    use journal::Journal;
    use std::path::PathBuf;
    use walk::tests::{indexed, on_disk, tree};

    /// An index kept is read on from where it was kept while the journal still holds all since. Kept before what the
    /// journal holds now (Glance not running while it went on), or with another journal, it is searched as it was kept
    /// and listed again behind it, not listed whole from nothing: two drives here were listed from nothing as search
    /// was called up after Glance had run hours without it.
    #[test]
    fn a_kept_index_the_journal_has_gone_past_is_searched_as_it_was_and_listed_again() {
        let journal = Journal { id: 7, first: 1000, next: 5000 };
        let kept = |id: u64, next: i64| store::Kept { serial: 1, journal: Some(store::Cursor { journal: id, next }), whole: true };
        assert_eq!(begun(Some(&kept(7, 1000)), &journal), Begun::ReadOn);
        assert_eq!(begun(Some(&kept(7, 5000)), &journal), Begun::ReadOn);
        assert_eq!(begun(Some(&kept(7, 999)), &journal), Begun::Relisted);
        assert_eq!(begun(Some(&kept(8, 3000)), &journal), Begun::Relisted);
        assert_eq!(begun(Some(&store::Kept { serial: 1, journal: None, whole: true }), &journal), Begun::Relisted);
        assert_eq!(begun(None, &journal), Begun::Listed);
    }

    /// A large folder that keeps changing (the user's temporary files: 110,000 names, written to every second) is
    /// listed again only a while after it was last listed, the longer the larger, and then with all that changed in it
    /// meanwhile. A small one is listed as it changes, and one whose listing failed is tried again.
    #[test]
    fn a_large_folder_that_keeps_changing_waits_to_be_listed_again() {
        let mut pacing = Pacing::default();
        let start = Instant::now();
        let second = |n: u64| start + Duration::from_secs(n);
        assert_eq!(pacing.take(vec![1, 2], start).0.len(), 2);
        pacing.listed(start, walk::Listed { large: vec![(1, 110_000)], failed: vec![3], ..walk::Listed::default() });
        assert_eq!(Pacing::wait(110_000), Duration::from_secs(11));
        assert_eq!(Pacing::wait(5_000_000), LONGEST_WAIT);

        let (mut due, later) = pacing.take(vec![1, 2], second(1));
        due.sort_unstable();
        assert_eq!((due, later), (vec![2, 3], vec![1]), "the large one waits, the small one and the failed one are listed");
        pacing.listed(second(1), walk::Listed::default());
        assert_eq!(pacing.take(vec![], second(5)), (vec![], vec![1]), "still waiting, though nothing else changed");
        pacing.listed(second(5), walk::Listed::default());
        assert_eq!(pacing.take(vec![], second(12)), (vec![1], vec![]), "listed once its wait is over");
    }

    /// A drive's first listing cut short (the PC turned off after its first round) goes on as Glance starts again: the
    /// index kept between its rounds is read on, its journal holding all since, searched at once for what was listed.
    /// The folders not yet listed are listed, those listed are not again, and it is whole as if listed at one go.
    #[test]
    fn a_first_listing_cut_short_goes_on_from_where_it_was_kept() {
        let root = tree("resume", 3, 2, 3);
        let kept_in = std::env::temp_dir().join(format!("glance-resume-kept-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&kept_in);
        let mut drive = Drive::open(&format!("{}\\", root.display())).unwrap();
        // (A serial of its own: kept apart from any other test's.)
        drive.serial = 0x5E5E_0000 | (std::process::id() & 0xFFFF);
        let pool = walk::Pool::new(2, 1 << 20);
        let cursor = store::Cursor { journal: 7, next: 1000 };

        // Begun: one round, cut short at once (its time over as it began), kept with where the journal was before it.
        let index = RwLock::new(Index::new(drive.root_id, true));
        let first = walk::round(&index, &drive, &[ROOT], &pool, Some(Instant::now()));
        assert_eq!(first.folders, 1, "the root alone listed");
        let kept = store::Kept { serial: drive.serial, journal: Some(cursor), whole: false };
        drop(store::keep_in(&kept_in, &index.read().unwrap(), kept).unwrap());
        drop(index);

        // Started again.
        let (index, kept) = store::kept_in(&kept_in, drive.serial).unwrap();
        assert_eq!((kept.journal, kept.whole), (Some(cursor), false));
        assert_eq!(begun(Some(&kept), &Journal { id: 7, first: 0, next: 2000 }), Begun::ReadOn);
        assert_eq!(indexed(&index), ["file 0.txt", "file 1.txt", "folder 0", "folder 1", "folder 2"], "what was listed searched at once");
        let volume = Volume::new(drive, index);
        // (Kept with no journal: not read here, nor kept between rounds.)
        *volume.kept.lock().unwrap() = Some(store::Kept { journal: None, ..kept });
        let listed = sweep(&volume, &pool, &mut Keeping::new(), true);
        assert_eq!(listed, 3 + 9 + 27, "the folders not yet listed, and none listed before");
        assert_eq!(indexed(&volume.index.read().unwrap()), on_disk(&root));
        assert!(volume.ready.load(Ordering::Relaxed) && volume.kept.lock().unwrap().is_some_and(|kept| kept.whole));
        assert!(marked(&volume.index.read().unwrap(), &HashSet::new()).is_empty());

        drop(volume);
        let _ = std::fs::remove_dir_all(&kept_in);
        let _ = std::fs::remove_dir_all(crate::settings::local_dir().join("Search"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A drive whose changes were lost (more changed than Windows kept) is listed again whole once for all the losses
    /// told meanwhile, and the names changed told with them are found as it is listed: each told was a listing whole.
    #[test]
    fn losses_told_together_are_one_listing_again() {
        let (tell, told) = mpsc::channel();
        for seen in [watch::Seen::Lost, watch::Seen::Folders(vec![PathBuf::from("a")]), watch::Seen::Lost] {
            tell.send(seen).unwrap();
        }
        let first = told.recv().unwrap();
        assert!(matches!(coalesce(first, &told), watch::Seen::Lost));
        assert!(told.try_recv().is_err(), "none left to list it again");

        for seen in [watch::Seen::Folders(vec![PathBuf::from("b"), PathBuf::from("a")]), watch::Seen::Folders(vec![PathBuf::from("a")])] {
            tell.send(seen).unwrap();
        }
        let first = told.recv().unwrap();
        assert!(matches!(coalesce(first, &told), watch::Seen::Folders(paths) if paths == [PathBuf::from("a"), PathBuf::from("b")]));
    }

    /// An index not kept (the disk full) is tried again a minute after, then twice as long each time, up to the
    /// longest it waits between keepings: not each second, as what changed since held much memory still. As Glance
    /// quits it is tried whatever failed before. Once kept, as before.
    #[test]
    fn an_index_not_kept_is_tried_again_later_and_later() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut keeping = Keeping { at: start, changed: true, retry: None };
        let much = HELD_MOST + 1;
        assert!(keeping.due(at(0), false, false, much, false));
        keeping.failed(at(0));
        assert!(!keeping.due(at(1), false, false, much, false), "not at the next turn");
        assert!(!keeping.due(at(1), true, false, much, false), "nor as a round of listing ends");
        assert!(keeping.due(at(1), false, true, much, false), "as Glance quits");
        assert!(keeping.due(at(60), false, false, much, false));
        keeping.failed(at(60));
        assert!(!keeping.due(at(179), false, false, much, false));
        assert!(keeping.due(at(180), false, false, much, false));
        for _ in 0..10 {
            keeping.failed(at(180));
        }
        assert_eq!(keeping.retry.map(|(_, wait)| wait), Some(KEEP_EVERY));
        keeping.kept(at(200));
        assert!(keeping.due(at(201), false, false, much, false), "kept, as before");
    }
}
