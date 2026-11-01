//! Searching this PC's files by name, as quickly as Everything does, with the user's own rights: every drive's
//! folders listed once (kept on disk after, and read from there: see `store`), then kept current as its change
//! journal says which folders changed (or, on a drive with none, as Windows tells of names changed while it is
//! watched). The user finds what they may list, and nothing else: no drive is opened whole, nothing is asked of the
//! sensors.
//!
//! Each drive's index is changed by one thread alone, its own (see `follow`), which keeps it anew on disk between
//! its rounds of listing (never in one, whose entries' places it holds), as what changed since it was last kept
//! grows, now and then, and as Glance quits.

pub mod drive;
pub mod index;
pub mod journal;
pub mod query;
mod readings;
mod store;
pub mod walk;
mod watch;

use drive::Drive;
use index::{flag, Index, ROOT};
pub use query::{Hit, Only, Query};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
/// How much memory what changed since an index was kept may hold before it is kept anew.
const HELD_MOST: usize = 16 << 20;
/// How long Glance quitting waits for the indexes to be kept.
const QUIT_WAIT: Duration = Duration::from_secs(3);

/// A drive being searched.
pub struct Volume {
    pub drive: Drive,
    index: RwLock<Index>,
    ready: AtomicBool,
    gone: AtomicBool,
    /// What its index is kept with (its journal and where it has come to, where it has one).
    kept: Mutex<Option<store::Kept>>,
    /// Asked to be kept (Glance quitting), and how many times it was kept or found to have nothing to keep since.
    keep_asked: AtomicBool,
    kept_times: AtomicU64,
}

impl Volume {
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

/// Starts finding the names on every drive, and keeping them current, on threads of its own (once; again does
/// nothing).
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("glance: search drives".into()).spawn(drives);
    });
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
            come.push(Arc::new(Volume { drive, index: RwLock::new(index), ready: AtomicBool::new(false), gone: AtomicBool::new(false), kept: Mutex::new(None), keep_asked: AtomicBool::new(false), kept_times: AtomicU64::new(0) }));
        }
        *VOLUMES.lock().unwrap() = staying.into_iter().chain(come.iter().cloned()).collect();
        for volume in come {
            let _ = std::thread::Builder::new().name(format!("glance: search {}", &volume.drive.path[..1])).spawn(move || follow(&volume));
        }
        std::thread::sleep(DRIVES_EVERY);
    }
}

/// A drive's names found (or read as kept), then kept current till it is gone, or let go of.
fn follow(volume: &Arc<Volume>) {
    let drive = &volume.drive;
    let threads = walk::threads();
    let journal = if drive.by_id() { journal::query(drive).ok() } else { None };
    let kept = store::kept(drive.serial);
    let mut keeping = Keeping { at: Instant::now(), changed: false };
    if let Some(journal) = journal {
        let fits = |kept: &store::Kept| kept.journal.is_some_and(|cursor| cursor.journal == journal.id && cursor.next >= journal.first && cursor.next <= journal.next);
        if let Some((index, kept)) = kept.filter(|(index, kept)| index.by_id() && fits(kept)) {
            *volume.index.write().unwrap() = index;
            *volume.kept.lock().unwrap() = Some(kept);
        } else {
            // (Where the journal was before listing: what changes while it lists is read after.)
            *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: Some(store::Cursor { journal: journal.id, next: journal.next }) });
            walk::round(&volume.index, drive, &[ROOT], threads);
            keeping.changed = true;
            keeping.keep(volume, true);
        }
        volume.ready.store(true, Ordering::Relaxed);
        follow_journal(volume, threads, keeping);
    } else {
        let (tell, told) = mpsc::channel();
        let watched = volume.clone();
        let _ = std::thread::Builder::new().name(format!("glance: search watch {}", &drive.path[..1])).spawn(move || watch::watch(&watched.drive, tell));
        *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: None });
        // What was kept found at once, and listed again (nothing kept says what changed while Glance was not
        // running); else all of it listed.
        match kept.filter(|(index, kept)| index.by_id() == drive.by_id() && kept.journal.is_none()) {
            Some((index, _)) => {
                *volume.index.write().unwrap() = index;
                volume.ready.store(true, Ordering::Relaxed);
                relist(volume, threads);
            }
            None => {
                walk::round(&volume.index, drive, &[ROOT], threads);
                volume.ready.store(true, Ordering::Relaxed);
            }
        }
        keeping.changed = true;
        keeping.keep(volume, true);
        follow_watch(volume, &told, threads, keeping);
    }
}

/// When a drive's index was last kept, and whether it changed since.
struct Keeping {
    at: Instant,
    changed: bool,
}

impl Keeping {
    /// The index kept anew (it is between rounds of listing), where it changed and that is asked for (`now`, or by
    /// Glance quitting), it is a while since, or what changed since holds much memory or many dead.
    fn keep(&mut self, volume: &Volume, now: bool) {
        // (A drive let go of is not kept anew: what was listed as it went may be partly listed. What was kept before
        // stands, and what changed since is read from its journal as it comes back.)
        if volume.drive.left() {
            return;
        }
        let asked = volume.keep_asked.swap(false, Ordering::Relaxed);
        let due = {
            let index = volume.index.read().unwrap();
            now || asked || self.at.elapsed() >= KEEP_EVERY || index.held() > HELD_MOST || index.untidy()
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
                        self.changed = false;
                    }
                    Err(error) => crate::journal::note(format!("search: {} not kept: {error}", volume.drive.path)),
                }
                self.at = Instant::now();
            }
        }
        if asked {
            volume.kept_times.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A drive kept current as its journal says which folders changed.
fn follow_journal(volume: &Volume, threads: usize, mut keeping: Keeping) {
    let drive = &volume.drive;
    // Folders that could not be listed (another program had them, say), by their ids: listed again next time.
    let mut again: Vec<u64> = Vec::new();
    while !volume.away() {
        std::thread::sleep(JOURNAL_EVERY);
        let Some(cursor) = volume.kept.lock().unwrap().and_then(|kept| kept.journal) else { return };
        match journal::read(drive, cursor.journal, cursor.next) {
            Ok(journal::Read::Changed { mut folders, next }) => {
                folders.append(&mut again);
                folders.sort_unstable();
                folders.dedup();
                if !folders.is_empty() {
                    let places: Vec<u32> = {
                        let index = volume.index.read().unwrap();
                        folders.iter().filter_map(|&id| index.folder(id)).collect()
                    };
                    again = walk::round(&volume.index, drive, &places, threads).failed;
                    keeping.changed = true;
                }
                // (Only once what changed is in the index: kept now, it holds all before `next`.)
                if let Some(kept) = volume.kept.lock().unwrap().as_mut() {
                    kept.journal = Some(store::Cursor { next, ..cursor });
                }
            }
            Ok(journal::Read::Lost) => {
                // Changes missed: everything listed again, from where the journal is now.
                if let Ok(journal) = journal::query(drive) {
                    *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: Some(store::Cursor { journal: journal.id, next: journal.next }) });
                    relist(volume, threads);
                    again.clear();
                    keeping.changed = true;
                }
            }
            // (The drive going, or its journal stopped: looked at again next time.)
            Err(_) => {}
        }
        keeping.keep(volume, false);
    }
}

/// A drive with no journal kept current as Windows tells of names changed in its folders.
fn follow_watch(volume: &Volume, told: &mpsc::Receiver<watch::Seen>, threads: usize, mut keeping: Keeping) {
    let mut again: Vec<u64> = Vec::new();
    while !volume.away() {
        match told.recv_timeout(JOURNAL_EVERY) {
            Ok(watch::Seen::Folders(paths)) => {
                let mut places: Vec<u32> = {
                    let index = volume.index.read().unwrap();
                    paths.iter().map(|path| index.folder_at(path)).chain(again.drain(..).filter_map(|id| index.folder(id))).collect()
                };
                places.sort_unstable();
                places.dedup();
                again = walk::round(&volume.index, &volume.drive, &places, threads).failed;
                keeping.changed = true;
            }
            Ok(watch::Seen::Lost) => {
                relist(volume, threads);
                keeping.changed = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        keeping.keep(volume, false);
    }
}

/// Every folder of the drive listed again.
fn relist(volume: &Volume, threads: usize) {
    let folders: Vec<u32> = {
        let index = volume.index.read().unwrap();
        (0..index.len()).filter(|&place| index.entry(place).alive() && index.entry(place).flags & (flag::FOLDER | flag::LINK) == flag::FOLDER).collect()
    };
    walk::round(&volume.index, &volume.drive, &folders, threads);
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

/// How each drive's search stands.
pub fn standing() -> Vec<Standing> {
    let volumes = VOLUMES.lock().unwrap().clone();
    volumes.iter().map(|volume| Standing { root: volume.drive.path.clone(), ready: volume.ready.load(Ordering::Relaxed), names: volume.index.read().unwrap().names() }).collect()
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
