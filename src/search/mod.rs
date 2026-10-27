//! Searching this PC's files by name, as quickly as Everything does, with the user's own rights: every drive's
//! folders listed once (kept on disk after), then kept current as its change journal says which folders changed
//! (or, on a drive with none, as Windows tells of names changed while it is watched). The user finds what they may
//! list, and nothing else: no drive is opened whole, nothing is asked of the sensors.

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
pub use query::{Hit, Query};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once, RwLock};
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

/// GetDriveTypeW's removable and fixed drives (others, the network's and CDs, are not searched).
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

/// How often the drives there are looked at again, the journal read, and an index kept that changed.
const DRIVES_EVERY: Duration = Duration::from_secs(5);
const JOURNAL_EVERY: Duration = Duration::from_secs(1);
const KEEP_EVERY: Duration = Duration::from_secs(10 * 60);

/// A drive being searched.
pub struct Volume {
    pub drive: Drive,
    index: RwLock<Index>,
    ready: AtomicBool,
    gone: AtomicBool,
    /// Its journal and where it has come to, where it has one (as kept with its index).
    kept: Mutex<Option<store::Kept>>,
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
    /// How many names meet what was typed (the best of them in `hits`).
    pub total: usize,
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
        let mut volumes = VOLUMES.lock().unwrap();
        volumes.retain(|volume| {
            let there = roots.contains(&volume.drive.path) && Drive::open(&volume.drive.path).is_ok_and(|now| now.serial == volume.drive.serial);
            if !there {
                volume.gone.store(true, Ordering::Relaxed);
            }
            there
        });
        for root in roots {
            if volumes.iter().any(|volume| volume.drive.path == root) {
                continue;
            }
            let Ok(drive) = Drive::open(&root) else { continue };
            let index = Index::new(drive.root_id, drive.by_id());
            let volume = Arc::new(Volume { drive, index: RwLock::new(index), ready: AtomicBool::new(false), gone: AtomicBool::new(false), kept: Mutex::new(None) });
            volumes.push(volume.clone());
            let _ = std::thread::Builder::new().name(format!("glance: search {}", &root[..1])).spawn(move || follow(&volume));
        }
        drop(volumes);
        std::thread::sleep(DRIVES_EVERY);
    }
}

/// A drive's names found (or read as kept), then kept current till it is gone.
fn follow(volume: &Volume) {
    let drive = &volume.drive;
    let threads = walk::threads();
    let journal = if drive.by_id() { journal::query(drive).ok() } else { None };
    if let Some(journal) = journal {
        let kept = store::kept(drive.serial).filter(|(_, kept)| kept.journal == journal.id && kept.next >= journal.first && kept.next <= journal.next);
        if let Some((index, kept)) = kept {
            *volume.index.write().unwrap() = index;
            *volume.kept.lock().unwrap() = Some(kept);
        } else {
            // (Where the journal was before listing: what changes while it lists is read after.)
            *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: journal.id, next: journal.next });
            walk::round(&volume.index, drive, &[ROOT], threads);
            keep(volume);
        }
        volume.ready.store(true, Ordering::Relaxed);
        follow_journal(volume, threads);
    } else {
        let (tell, told) = mpsc::channel();
        let root = drive.path.clone();
        let _ = std::thread::Builder::new().name(format!("glance: search watch {}", &root[..1])).spawn(move || watch::watch(&root, tell));
        walk::round(&volume.index, drive, &[ROOT], threads);
        volume.ready.store(true, Ordering::Relaxed);
        follow_watch(volume, &told, threads);
    }
}

/// A drive kept current as its journal says which folders changed.
fn follow_journal(volume: &Volume, threads: usize) {
    let drive = &volume.drive;
    let mut kept_at = Instant::now();
    let mut changed = false;
    while !volume.gone.load(Ordering::Relaxed) {
        std::thread::sleep(JOURNAL_EVERY);
        let Some(kept) = *volume.kept.lock().unwrap() else { return };
        match journal::read(drive, kept.journal, kept.next) {
            Ok(journal::Read::Changed { folders, next }) => {
                if !folders.is_empty() {
                    let places: Vec<u32> = {
                        let index = volume.index.read().unwrap();
                        folders.iter().filter_map(|&id| index.folder(id)).collect()
                    };
                    walk::round(&volume.index, drive, &places, threads);
                    changed = true;
                }
                // (Only once what changed is in the index: kept now, it holds all before `next`.)
                volume.kept.lock().unwrap().as_mut().unwrap().next = next;
            }
            Ok(journal::Read::Lost) => {
                // Changes missed: everything listed again, from where the journal is now.
                let Ok(journal) = journal::query(drive) else { continue };
                *volume.kept.lock().unwrap() = Some(store::Kept { serial: drive.serial, journal: journal.id, next: journal.next });
                relist(volume, threads);
                changed = true;
            }
            // (The drive going, or its journal stopped: looked at again next time.)
            Err(_) => continue,
        }
        if changed && kept_at.elapsed() >= KEEP_EVERY {
            keep(volume);
            kept_at = Instant::now();
            changed = false;
        }
    }
}

/// A drive with no journal kept current as Windows tells of names changed in its folders.
fn follow_watch(volume: &Volume, told: &mpsc::Receiver<watch::Seen>, threads: usize) {
    while !volume.gone.load(Ordering::Relaxed) {
        match told.recv_timeout(DRIVES_EVERY) {
            Ok(watch::Seen::Folders(paths)) => {
                let mut places: Vec<u32> = {
                    let index = volume.index.read().unwrap();
                    paths.iter().map(|path| index.folder_at(path)).collect()
                };
                places.sort_unstable();
                places.dedup();
                walk::round(&volume.index, &volume.drive, &places, threads);
            }
            Ok(watch::Seen::Lost) => relist(volume, threads),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Every folder of the drive listed again.
fn relist(volume: &Volume, threads: usize) {
    let folders: Vec<u32> = {
        let index = volume.index.read().unwrap();
        (0..index.entries().len() as u32).filter(|&place| index.entries()[place as usize].alive() && index.entries()[place as usize].flags & (flag::FOLDER | flag::LINK) == flag::FOLDER).collect()
    };
    walk::round(&volume.index, &volume.drive, &folders, threads);
}

/// A drive's index kept on disk, with where its journal had come to (a drive with none is listed again as Glance
/// starts: nothing kept would say what changed while it was not running).
fn keep(volume: &Volume) {
    let Some(kept) = *volume.kept.lock().unwrap() else { return };
    let index = volume.index.read().unwrap();
    if let Err(error) = store::keep(&index, kept) {
        crate::journal::note(format!("search: {} not kept: {error}", volume.drive.path));
    }
}

/// Every index that changed kept on disk (as Glance quits).
pub fn keep_all() {
    let volumes = VOLUMES.lock().unwrap().clone();
    for volume in volumes {
        if volume.ready.load(Ordering::Relaxed) {
            keep(&volume);
        }
    }
}

/// How each drive's search stands.
pub fn standing() -> Vec<Standing> {
    let volumes = VOLUMES.lock().unwrap().clone();
    volumes.iter().map(|volume| Standing { root: volume.drive.path.clone(), ready: volume.ready.load(Ordering::Relaxed), names: volume.index.read().unwrap().names() }).collect()
}

/// The names meeting `typed` on every drive: how many there are, and the best `most`, best first.
pub fn find(typed: &str, most: usize) -> Found {
    let query = Query::read(typed);
    if query.is_empty() {
        return Found::default();
    }
    let volumes = VOLUMES.lock().unwrap().clone();
    let mut found = Found::default();
    for volume in &volumes {
        let index = volume.index.read().unwrap();
        let (total, hits) = query.find(&index, &volume.drive.path, most);
        found.total += total;
        found.hits.extend(hits);
    }
    found.hits.sort_by(|a, b| b.rank.total_cmp(&a.rank).then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len())));
    found.hits.truncate(most);
    found
}
