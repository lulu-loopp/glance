//! Folders listed again, and every folder found new in them listed in turn, by several threads at once at a low
//! priority; what they find put in the index by one, as each folder is listed, for it to be searched meanwhile.

use super::drive::{self, Drive, Folder, Listing};
use super::index::{flag, Changes, Index};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, RwLock};
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};

/// What a round of listing did: how many folders it listed, how many the user may not, those that could not be
/// listed (by their ids), and the large ones listed (by their ids, with how many names each holds).
#[derive(Default, Debug, Clone)]
pub struct Listed {
    pub folders: usize,
    pub denied: usize,
    pub failed: Vec<u64>,
    pub large: Vec<(u64, usize)>,
}

/// How many names a folder holds to be counted large.
pub const LARGE: usize = 10_000;

/// The folders to list (by their ids, and their paths where the drive keeps no ids), and whether the round is over.
struct Jobs {
    ids: Vec<(u64, Option<PathBuf>)>,
    over: bool,
}

/// How a folder is listed: by its id where the drive keeps them, else by its path.
fn job(index: &Index, drive: &Drive, place: u32, id: u64) -> (u64, Option<PathBuf>) {
    let path = (!index.by_id()).then(|| index.path(place, &format!("\\\\?\\{}", drive.path))).flatten();
    (id, path)
}

/// How many threads list a drive's folders.
pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(2, 16)
}

/// `folders` (live ones, by their places) listed again, and what was found new in them in turn, on `threads`
/// threads.
pub fn round(index: &RwLock<Index>, drive: &Drive, folders: &[u32], threads: usize) -> Listed {
    // What each folder held before, by its id (a folder moved meanwhile is found by it where it went).
    let (ids, mut before) = {
        let index = index.read().unwrap();
        let ids: Vec<(u32, u64)> = folders.iter().filter_map(|&place| index.id(place).map(|id| (place, id))).filter(|&(place, _)| index.entry(place).alive()).collect();
        let children = index.children(&ids.iter().map(|&(place, _)| place).collect::<HashSet<_>>());
        let before: HashMap<u64, Vec<u32>> = ids.iter().map(|&(place, id)| (id, children.get(&place).cloned().unwrap_or_default())).collect();
        (ids.into_iter().map(|(place, id)| job(&index, drive, place, id)).collect::<Vec<_>>(), before)
    };
    let mut listed = Listed::default();
    if ids.is_empty() {
        return listed;
    }
    let mut pending = ids.len();
    let jobs = Mutex::new(Jobs { ids, over: false });
    let more = Condvar::new();
    let (tell, told) = mpsc::channel::<(u64, Listing)>();
    let mut changes = Changes::default();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let tell = tell.clone();
            let (jobs, more) = (&jobs, &more);
            scope.spawn(move || {
                // (Its reading at a low priority, the disk's too: it is never in the way of what the PC is doing.)
                unsafe {
                    let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
                }
                let mut buffer = drive::buffer();
                loop {
                    let job = {
                        let mut jobs = jobs.lock().unwrap();
                        loop {
                            if let Some(job) = jobs.ids.pop() {
                                break Some(job);
                            }
                            if jobs.over {
                                break None;
                            }
                            jobs = more.wait(jobs).unwrap();
                        }
                    };
                    let Some((id, path)) = job else { break };
                    let folder = path.map_or(Folder::Id(id), Folder::Path);
                    if tell.send((id, drive.list(&folder, &mut buffer))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tell);
        while pending > 0 {
            let Ok((id, listing)) = told.recv() else { break };
            pending -= 1;
            listed.folders += 1;
            let mut index = index.write().unwrap();
            let Some(place) = index.folder(id) else { continue };
            let before = before.remove(&id).unwrap_or_default();
            match listing {
                Listing::Items(items) => {
                    if items.len() >= LARGE {
                        listed.large.push((id, items.len()));
                    }
                    index.list(place, &items, &before, &mut changes);
                    index.set_flag(place, flag::UNREAD | flag::STALE, false);
                }
                Listing::Denied => {
                    listed.denied += 1;
                    index.denied(place, &before);
                    index.set_flag(place, flag::STALE, false);
                }
                Listing::Failed(_) => {
                    listed.failed.push(id);
                    index.set_flag(place, flag::UNREAD | flag::STALE, true);
                }
                Listing::Gone => {}
            }
            if !changes.to_list.is_empty() {
                pending += changes.to_list.len();
                let found: Vec<_> = changes.to_list.drain(..).map(|(place, id)| job(&index, drive, place, id)).collect();
                jobs.lock().unwrap().ids.extend(found);
                more.notify_all();
            }
            drop(index);
        }
        jobs.lock().unwrap().over = true;
        more.notify_all();
    });
    index.write().unwrap().finish(changes);
    listed
}
