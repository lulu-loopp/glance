//! The calls on files, all read from Glance's index of each drive's names (see `search`): the drives, a folder's size
//! and what is in it, the largest and oldest files under a folder, what a folder or a drive is made of, and files
//! found by name. Nothing is listed from the disk: the folder asked about is only opened, as the client, to check that
//! its user may (see `server::Call::folder`).

use super::server::Call;
use super::tools::Args;
use super::Failure;
use crate::disk::{self, Kind, Met, Places, KINDS};
use crate::search::index::{flag, Index, ROOT};
use serde_json::{json, Value};
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};

/// What each kind of file is called, in the order of `disk::KINDS`.
pub const KIND_NAMES: [&str; 9] = ["video", "picture", "music", "document", "archive", "program", "game", "system", "other"];

fn kind_name(kind: Kind) -> &'static str {
    KIND_NAMES[kind.at()]
}

/// How much of an answer its lists may take (bytes, written out): an answer is at most `pipe::ANSWER_MOST`.
const LISTED_MOST: usize = 3 << 19;

/// What a list may still take: entries added while they fit.
struct Budget(usize);

impl Budget {
    fn new() -> Budget {
        Budget(LISTED_MOST)
    }

    /// Whether `entry` fits: taken from what is left where it does.
    fn fits(&mut self, entry: &Value) -> bool {
        let size = serde_json::to_vec(entry).map_or(usize::MAX, |bytes| bytes.len() + 1);
        if size > self.0 {
            return false;
        }
        self.0 -= size;
        true
    }
}

/// A file time (100 ns since 1601, UTC) as ISO 8601 (`2026-11-27T05:12:30Z`): none where there is none.
pub fn iso(written: i64) -> Value {
    if written <= 0 {
        return Value::Null;
    }
    let seconds = written / 10_000_000 - 11_644_473_600;
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // (Days since 1970 as a date of the Gregorian calendar, as Howard Hinnant reckons them.)
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * of_year + 2) / 153;
    let day = of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 { month_from_march + 3 } else { month_from_march - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    json!(format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", of_day / 3600, of_day / 60 % 60, of_day % 60))
}

/// A folder asked about, as Glance's index reads it: its drive's root ("C:\\"), its path from there, and its path
/// as it is (through any links on the way).
pub struct Asked {
    pub root: String,
    pub rest: PathBuf,
    pub path: PathBuf,
}

/// What `read` reads of the index of the drive `asked` is on, at the folder's place there, held from changing
/// meanwhile, the drive's search started first where it was not: why not, where it cannot be.
fn with_folder<R>(asked: &Asked, read: impl FnOnce(&Index, u32) -> Result<R, Failure>) -> Result<R, Failure> {
    crate::search::asked();
    let reading = crate::search::read_drive(&asked.root, None, |index, _| match index.place_of(&asked.rest) {
        Some(place) => read(index, place),
        None => Err(Failure::new("not_found", format!("{} is not in Glance's index (it may have been made a moment ago, or be reached through a link).", asked.path.display()))),
    });
    match reading {
        crate::search::Reading::Read { read, .. } => read,
        crate::search::Reading::Listing => {
            let names = crate::search::standing().into_iter().find(|drive| drive.root.eq_ignore_ascii_case(&asked.root)).map_or(0, |drive| drive.names);
            Err(Failure { code: "index_listing", message: format!("Glance is still finding the names on {} ({names} so far). Try again later.", asked.root), more: Some(json!({ "progress": { "names": names } })) })
        }
        crate::search::Reading::Nowhere => Err(Failure::new("not_local", format!("{} is not on a drive Glance indexes (fixed and removable drives by letter).", asked.root))),
    }
}

/// Whether the index of the drive at `root` holds its names as they are now.
fn current(root: &str) -> bool {
    crate::search::standing().into_iter().find(|drive| drive.root.eq_ignore_ascii_case(root)).is_some_and(|drive| drive.current)
}

/// What a folder holds at any depth, as `held` counts it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Held {
    bytes: u64,
    files: u64,
    cloud: u64,
    written: i64,
    unread: u64,
}

/// What the folder at `folder` holds at any depth, nothing through links: its files' sizes (those kept in the cloud
/// alone apart), when anything in it was written last, and the folders in it whose names could not be listed.
fn held(index: &Index, root: &str, places: &Places, folder: u32, going: &mut dyn FnMut() -> bool) -> Option<Held> {
    let mut held = Held::default();
    let walked = disk::walk(index, root, places, folder, true, going, &mut |met| {
        if met.flags & flag::CLOUD != 0 {
            held.cloud += met.stat.size;
            return;
        }
        held.bytes += met.stat.size;
        held.files += 1;
        held.written = held.written.max(met.stat.written);
    })?;
    held.unread = walked.unread;
    Some(held)
}

pub fn drives() -> Result<Value, Failure> {
    crate::search::asked();
    let standing = crate::search::standing();
    let volumes: Vec<Value> = disk::act::drives()
        .into_iter()
        .map(|drive| {
            let index = standing.iter().find(|standing| standing.root == drive.root);
            json!({
                "root": drive.root,
                "label": drive.label,
                "total_bytes": drive.total,
                "free_bytes": drive.free,
                "space_read": drive.total > 0,
                "index": if index.is_some_and(|index| index.ready) { "ready" } else { "listing" },
                "current": index.is_some_and(|index| index.current),
                "names": index.map_or(0, |index| index.names),
            })
        })
        .collect();
    let info = crate::app().controller.info();
    let latest = crate::app().controller.history.latest();
    let physical: Vec<Value> = info
        .drive_ids
        .iter()
        .zip(&info.drives)
        .map(|(&id, model)| {
            let rate = latest.as_ref().and_then(|sample| sample.drive_rates.iter().find(|rate| rate.id == id).copied());
            let celsius = latest.as_ref().and_then(|sample| sample.drive_temps.iter().find(|drive| drive.id == id).map(|drive| drive.celsius));
            json!({
                "id": id,
                "model": model,
                "celsius": celsius.map(|celsius| (f64::from(celsius) * 10.0).round() / 10.0),
                "read_bytes_per_s": rate.and_then(|rate| rate.read).map(|rate| rate.round() as i64),
                "write_bytes_per_s": rate.and_then(|rate| rate.write).map(|rate| rate.round() as i64),
            })
        })
        .collect();
    Ok(json!({ "volumes": volumes, "physical": physical }))
}

pub fn folder_size(args: &Args, call: &Call) -> Result<Value, Failure> {
    let largest = args.int("largest")?.unwrap_or(10) as usize;
    let asked = call.folder(args.string("path")?.unwrap_or_default())?;
    let places = Places::of_this_pc();
    with_folder(&asked, |index, place| {
        let mut going = || call.going().is_ok();
        let all = held(index, &asked.root, &places, place, &mut going).ok_or_else(|| call.stopped())?;
        let mut children: Vec<(String, bool, u64)> = Vec::new();
        let mut entries = Vec::new();
        index.each_in(place, |child, entry| entries.push((child, entry)));
        for (child, entry) in entries {
            let name = index.name(child).to_string_lossy().into_owned();
            if entry.folder() {
                if entry.flags & flag::LINK == 0 {
                    let inner = held(index, &asked.root, &places, child, &mut going).ok_or_else(|| call.stopped())?;
                    children.push((name, true, inner.bytes));
                }
            } else if entry.flags & flag::CLOUD == 0 {
                children.push((name, false, index.stat(child).size));
            }
        }
        children.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        children.retain(|child| child.2 > 0);
        children.truncate(largest);
        Ok(json!({
            "path": asked.path,
            "bytes": all.bytes,
            "files": all.files,
            "cloud_only_bytes": all.cloud,
            "largest": children.iter().map(|(name, folder, bytes)| json!({ "name": name, "folder": folder, "bytes": bytes })).collect::<Vec<_>>(),
            "current": current(&asked.root),
            "skipped_folders": all.unread,
        }))
    })
}

pub fn list_folder(args: &Args, call: &Call) -> Result<Value, Failure> {
    let order = args.choice("order")?.to_string();
    let reversed = args.boolean("reversed")?;
    let limit = args.int("limit")?.unwrap_or(100) as usize;
    let offset = args.int("offset")?.unwrap_or(0) as usize;
    let asked = call.folder(args.string("path")?.unwrap_or_default())?;
    let places = Places::of_this_pc();
    with_folder(&asked, |index, place| {
        let mut going = || call.going().is_ok();
        let mut entries = Vec::new();
        index.each_in(place, |child, entry| entries.push((child, entry)));
        // Each with what it holds (a folder's at any depth), its name's key to be put in order by (as search does).
        let mut listed: Vec<(u32, bool, bool, Held, Vec<u8>)> = Vec::with_capacity(entries.len());
        for (child, entry) in entries {
            let link = entry.flags & flag::LINK != 0;
            let held = if !entry.folder() {
                let stat = index.stat(child);
                let cloud = entry.flags & flag::CLOUD != 0;
                Held { bytes: if cloud { 0 } else { stat.size }, files: 1, cloud: if cloud { stat.size } else { 0 }, written: stat.written, unread: 0 }
            } else if link {
                Held::default()
            } else {
                held(index, &asked.root, &places, child, &mut going).ok_or_else(|| call.stopped())?
            };
            listed.push((child, entry.folder(), link, held, crate::search::query::name_key(index.folded_name(child))));
        }
        let total = listed.len();
        // (Folders before files by name, as Explorer lists them.)
        listed.sort_by(|a, b| {
            let ordering = match order.as_str() {
                "name" => b.1.cmp(&a.1).then_with(|| a.4.cmp(&b.4)),
                "modified" => b.3.written.cmp(&a.3.written).then_with(|| a.4.cmp(&b.4)),
                _ => b.3.bytes.cmp(&a.3.bytes).then_with(|| a.4.cmp(&b.4)),
            };
            if reversed {
                ordering.reverse()
            } else {
                ordering
            }
        });
        let mut budget = Budget::new();
        let mut told = Vec::new();
        let mut truncated = offset + limit < total;
        for (child, folder, link, held, _) in listed.into_iter().skip(offset).take(limit) {
            let entry = index.entry(child);
            let name = index.name(child).to_string_lossy().into_owned();
            let mut item = json!({
                "name": name,
                "path": asked.path.join(&name),
                "folder": folder,
                "bytes": held.bytes,
                "modified": iso(held.written),
                "hidden": entry.flags & flag::HIDDEN != 0,
            });
            if folder {
                item["files"] = json!(held.files);
                item["link"] = json!(link);
                item["unread"] = json!(entry.flags & flag::UNREAD != 0 || held.unread > 0);
            } else {
                item["cloud_only"] = json!(entry.flags & flag::CLOUD != 0);
            }
            if !budget.fits(&item) {
                truncated = true;
                break;
            }
            told.push(item);
        }
        Ok(json!({ "path": asked.path, "total": total, "offset": offset, "entries": told, "current": current(&asked.root), "truncated": truncated }))
    })
}

/// A file that passed, as it is put in order: by the key it is put in order by (the first first) and its place, and
/// the file.
struct Ranked(std::cmp::Reverse<(i64, u32)>, Met);

impl PartialEq for Ranked {
    fn eq(&self, other: &Ranked) -> bool {
        self.0 == other.0
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Ranked) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    fn cmp(&self, other: &Ranked) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

/// What files `largest_files` keeps, as its arguments say.
struct Wanted {
    least: u64,
    /// Written before this (a file time), where only those not written to for a while are.
    before: Option<i64>,
    kinds: Option<Vec<Kind>>,
    extensions: Option<Vec<String>>,
    system: bool,
}

impl Wanted {
    fn passes(&self, index: &Index, met: &Met) -> bool {
        met.flags & flag::CLOUD == 0
            && met.stat.size > 0
            && met.stat.size >= self.least
            && (self.system || !met.is.system)
            && self.before.is_none_or(|before| met.stat.written < before)
            && self.kinds.as_ref().is_none_or(|kinds| kinds.contains(&met.is.kind))
            && self.extensions.as_ref().is_none_or(|extensions| {
                let extension = disk::extension_of(index.folded_name(met.place));
                extensions.iter().any(|wanted| wanted == extension)
            })
    }
}

pub fn largest_files(args: &Args, call: &Call) -> Result<Value, Failure> {
    let order = args.choice("order")?.to_string();
    let limit = args.int("limit")?.unwrap_or(20) as usize;
    let offset = args.int("offset")?.unwrap_or(0) as usize;
    let now = disk::now();
    let wanted = Wanted {
        least: args.int("min_bytes")?.unwrap_or(0) as u64,
        before: args.int("not_modified_days")?.map(|days| now - days * 864_000_000_000),
        kinds: args.strings("kinds")?.map(|kinds| kinds.iter().filter_map(|kind| KIND_NAMES.iter().position(|name| name == kind)).map(|at| KINDS[at]).collect()),
        extensions: args.strings("extensions")?.map(|extensions| extensions.iter().map(|extension| extension.trim_start_matches('.').to_lowercase()).collect()),
        system: args.boolean("include_system")?,
    };
    let bin = args.boolean("include_recycle_bin")?;
    let asked = call.folder(args.string("path")?.unwrap_or_default())?;
    let places = Places::of_this_pc();
    with_folder(&asked, |index, place| {
        let most = offset + limit;
        let key = |met: &Met| -> i64 {
            match order.as_str() {
                "oldest" => -met.stat.written,
                "newest" => met.stat.written,
                _ => met.stat.size.min(i64::MAX as u64) as i64,
            }
        };
        // (The `most` first kept as they come: the last of them let go of as one before it comes.)
        let mut kept: BinaryHeap<Ranked> = BinaryHeap::new();
        let (mut matching, mut matching_bytes) = (0u64, 0u64);
        let mut going = || call.going().is_ok();
        let walked = disk::walk(index, &asked.root, &places, place, bin, &mut going, &mut |met| {
            if !wanted.passes(index, met) {
                return;
            }
            matching += 1;
            matching_bytes += met.stat.size;
            let ranked = Ranked(std::cmp::Reverse((key(met), met.place)), *met);
            if kept.len() < most {
                kept.push(ranked);
            } else if let Some(mut last) = kept.peek_mut() {
                if ranked.0 < last.0 {
                    *last = ranked;
                }
            }
        })
        .ok_or_else(|| call.stopped())?;
        let mut kept: Vec<Ranked> = kept.into_vec();
        kept.sort_by(|a, b| a.0.cmp(&b.0));
        let mut budget = Budget::new();
        let mut files = Vec::new();
        let mut truncated = (offset + limit) < matching as usize;
        for Ranked(_, met) in kept.into_iter().skip(offset) {
            let Some(path) = index.path(met.place, &asked.root) else { continue };
            let file = json!({ "path": path, "bytes": met.stat.size, "modified": iso(met.stat.written), "kind": kind_name(met.is.kind), "system": met.is.system, "in_recycle_bin": met.in_bin });
            if !budget.fits(&file) {
                truncated = true;
                break;
            }
            files.push(file);
        }
        Ok(json!({
            "path": asked.path,
            "files": files,
            "matching": { "files": matching, "bytes": matching_bytes },
            "current": current(&asked.root),
            "skipped_folders": walked.unread,
            "truncated": truncated,
        }))
    })
}

/// What the files under a folder come to (see `disk_breakdown`): those counted, by kind, by extension, by age; those
/// kept in the cloud alone; and, apart, those in the Recycle Bin where it was walked as well.
#[derive(Default, Debug, PartialEq)]
struct Breakdown {
    bytes: u64,
    files: u64,
    kinds: [u64; 9],
    extensions: std::collections::HashMap<String, (u64, u64, [u64; 9])>,
    ages: [(u64, u64); disk::AGES],
    cloud: u64,
    bin: u64,
    unread: u64,
}

/// What the folder at `folder` (of the drive at `root`) is made of at `now` (a file time), the Recycle Bin counted
/// apart where `bin` says it is walked (it is counted in where the folder is in it).
fn breakdown(index: &Index, root: &str, places: &Places, folder: u32, bin: bool, now: i64, going: &mut dyn FnMut() -> bool) -> Option<Breakdown> {
    let mut counted = Breakdown::default();
    let in_bin = |met: &Met| met.in_bin;
    let mut started_in_bin = false;
    index.each_in(ROOT, |place, entry| started_in_bin |= entry.folder() && index.folded_name(place) == "$recycle.bin" && (place == folder || index.lies_in(folder, place)));
    let walked = disk::walk(index, root, places, folder, bin, going, &mut |met| {
        if met.flags & flag::CLOUD != 0 {
            counted.cloud += met.stat.size;
            return;
        }
        if in_bin(met) && !started_in_bin {
            counted.bin += met.stat.size;
            return;
        }
        if met.stat.size == 0 {
            return;
        }
        counted.bytes += met.stat.size;
        counted.files += 1;
        counted.kinds[met.is.kind.at()] += met.stat.size;
        let extension = counted.extensions.entry(disk::extension_of(index.folded_name(met.place)).to_string()).or_default();
        extension.0 += met.stat.size;
        extension.1 += 1;
        extension.2[met.is.kind.at()] += met.stat.size;
        let age = &mut counted.ages[disk::age_of(met.stat.written, now)];
        age.0 += met.stat.size;
        age.1 += 1;
    })?;
    counted.unread = walked.unread;
    Some(counted)
}

/// Where each age the list by time groups files in begins and ends (days before now, the last none: on).
const AGE_DAYS: [(u32, Option<u32>); disk::AGES] = [(0, Some(7)), (7, Some(30)), (30, Some(182)), (182, Some(365)), (365, Some(1095)), (1095, None)];

pub fn disk_breakdown(args: &Args, call: &Call) -> Result<Value, Failure> {
    let extensions = args.int("extensions")?.unwrap_or(20) as usize;
    let asked = call.folder(args.string("path")?.unwrap_or_default())?;
    let places = Places::of_this_pc();
    let is_root = asked.rest.as_os_str().is_empty();
    let now = disk::now();
    with_folder(&asked, |index, place| {
        let mut going = || call.going().is_ok();
        let counted = breakdown(index, &asked.root, &places, place, is_root, now, &mut going).ok_or_else(|| call.stopped())?;
        let mut listed: Vec<_> = counted.extensions.iter().collect();
        listed.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(b.0)));
        let kind_of_most = |kinds: &[u64; 9]| kind_name(KINDS.iter().copied().max_by_key(|kind| (kinds[kind.at()], std::cmp::Reverse(kind.at()))).unwrap_or(Kind::Other));
        let mut told = json!({
            "path": asked.path,
            "snapshot_ms": super::now_ms(),
            "bytes": counted.bytes,
            "files": counted.files,
            "by_kind": KINDS.iter().map(|kind| (kind_name(*kind).to_string(), json!(counted.kinds[kind.at()]))).collect::<serde_json::Map<_, _>>(),
            "by_extension": listed.iter().take(extensions).map(|(extension, (bytes, files, kinds))| json!({ "extension": extension, "bytes": bytes, "files": files, "kind": kind_of_most(kinds) })).collect::<Vec<_>>(),
            "by_age": AGE_DAYS.iter().zip(counted.ages).map(|((from, to), (bytes, files))| json!({ "modified_days": [from, to], "bytes": bytes, "files": files })).collect::<Vec<_>>(),
            "cloud_only_bytes": counted.cloud,
            "current": current(&asked.root),
            "skipped_folders": counted.unread,
        });
        if is_root {
            let (total, free) = disk::act::space(&asked.root);
            // (All that files come to there, the Recycle Bin's too, against what Windows says is used: less where hard
            // links are counted once for each name, and compressed or sparse files at their whole size, as Explorer
            // counts them, which the index cannot tell apart.)
            let used = total.saturating_sub(free);
            let unaccounted = i128::from(used) - i128::from(counted.bytes + counted.bin);
            told["drive"] = json!({ "total_bytes": total, "free_bytes": free, "used_bytes": used, "space_read": total > 0, "recycle_bin_bytes": counted.bin, "unaccounted_bytes": unaccounted as i64 });
        }
        Ok(told)
    })
}

pub fn search_files(args: &Args, call: &Call) -> Result<Value, Failure> {
    let typed = args.string("query")?.unwrap_or_default().to_string();
    let limit = args.int("limit")?.unwrap_or(20) as usize;
    let offset = args.int("offset")?.unwrap_or(0) as usize;
    if offset + limit > 1000 {
        return Err(Failure::invalid("offset + limit must be at most 1000."));
    }
    use crate::search::query::{Only, Order, Query, Sort};
    let by = match args.choice("order")? {
        "modified" => Sort::Written,
        "name" => Sort::Name,
        "size" => Sort::Size,
        _ => Sort::Match,
    };
    let mut query = Query::read(&typed).ordered(Order { by, reversed: args.boolean("reversed")? });
    if query.is_empty() {
        return Err(Failure::invalid("query holds nothing to look for."));
    }
    match args.string("only")? {
        Some("files") => query = query.only(Only::Files),
        Some("folders") => query = query.only(Only::Folders),
        _ => {}
    }
    let under = match args.string("under")? {
        Some(path) => Some(call.folder(path)?),
        None => {
            call.may_read_files()?;
            None
        }
    };
    if let Some(under) = &under {
        query = query.under(&under.root, &under.rest);
    }
    crate::search::asked();
    let most = offset + limit;
    let found = crate::search::find_while(&query, most, || call.going().is_ok()).ok_or_else(|| call.stopped())?;
    let (complete, _) = crate::search::listed();
    // (By match, the best folders and the best files are kept apart, as many of each: the first of both together.)
    let mut hits = found.hits;
    if by == Sort::Match {
        crate::search::query::sort(&mut hits);
    }
    let mut budget = Budget::new();
    let mut told = Vec::new();
    let mut truncated = most < found.total;
    for hit in hits.into_iter().skip(offset).take(limit) {
        let mut item = json!({ "path": hit.path, "name": hit.name, "folder": hit.folder, "modified": iso(hit.stat.written) });
        if !hit.folder {
            item["bytes"] = json!(hit.stat.size);
        }
        if !budget.fits(&item) {
            truncated = true;
            break;
        }
        told.push(item);
    }
    Ok(json!({ "total": found.total, "folders": found.folders, "complete": complete, "truncated": truncated, "offset": offset, "hits": told }))
}

/// `typed` as an absolute path on a lettered drive (`C:\…`, `\\?\C:\…`), its separators made Windows' own: why not,
/// where it is not one (a relative path, or one on the network).
pub fn absolute(typed: &str) -> Result<PathBuf, Failure> {
    let typed = typed.replace('/', "\\");
    let plain = typed.strip_prefix(r"\\?\").or_else(|| typed.strip_prefix(r"\\.\")).unwrap_or(&typed);
    if plain.starts_with(r"\\") || plain.to_ascii_uppercase().starts_with(r"UNC\") {
        return Err(Failure::new("not_local", "Network paths are not indexed by Glance. Give a path on a local drive, such as C:\\Users."));
    }
    let bytes = plain.as_bytes();
    if !(bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\') {
        return Err(Failure::invalid(format!("{typed} is not an absolute path on a lettered drive, such as C:\\Users.")));
    }
    if plain.encode_utf16().count() > 32_767 {
        return Err(Failure::invalid("The path is longer than Windows allows."));
    }
    let path = Path::new(plain);
    if path.components().any(|part| matches!(part, std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return Err(Failure::invalid("The path must not hold . or .. parts."));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_times_are_told_in_utc() {
        // 2026-11-27 05:12:30 UTC, as a file time.
        let unix = 1_795_756_350i64;
        assert_eq!(iso((unix + 11_644_473_600) * 10_000_000), json!("2026-11-27T05:12:30Z"));
        assert_eq!(iso((951_782_400 + 11_644_473_600) * 10_000_000), json!("2000-02-29T00:00:00Z"));
        assert_eq!(iso(0), Value::Null);
    }

    /// Only absolute paths on a lettered drive are taken (as written, or Windows' long form), never the network's.
    #[test]
    fn only_paths_on_a_lettered_drive_are_taken() {
        assert_eq!(absolute("C:\\Users").unwrap(), PathBuf::from("C:\\Users"));
        assert_eq!(absolute("d:/Documents/x").unwrap(), PathBuf::from("d:\\Documents\\x"));
        assert_eq!(absolute(r"\\?\E:\Developer").unwrap(), PathBuf::from("E:\\Developer"));
        assert_eq!(absolute(r"\\server\share").unwrap_err().code, "not_local");
        assert_eq!(absolute(r"\\?\UNC\server\share").unwrap_err().code, "not_local");
        assert_eq!(absolute("Users").unwrap_err().code, "invalid_argument");
        assert_eq!(absolute("C:").unwrap_err().code, "invalid_argument");
        assert_eq!(absolute("C:\\a\\..\\b").unwrap_err().code, "invalid_argument");
    }

    fn index() -> Index {
        use crate::search::index::{Changes, Item, Stat};
        let item = |name: &str, id: u64, folder: bool, size: u64, attributes: u32| Item { name: name.encode_utf16().collect(), id, attributes: if folder { 0x10 } else { 0x20 } | attributes, tag: 0, stat: Stat { written: 133_800_000_000_000_000, size } };
        let mut index = Index::new(5, true);
        let mut changes = Changes::default();
        index.list(ROOT, &[item("Docs", 10, true, 0, 0), item("$Recycle.Bin", 11, true, 0, 0x6), item("big.iso", 12, false, 900, 0)], &[], &mut changes);
        index.list(1, &[item("a.mp4", 20, false, 300, 0), item("b.txt", 21, false, 0, 0), item("c.docx", 22, false, 50, 0x40_0000)], &[], &mut changes);
        index.list(2, &[item("old.zip", 30, false, 70, 0)], &[], &mut changes);
        index.finish(changes);
        index
    }

    /// What a folder holds counts its files at any depth, those in the cloud alone apart; what a drive is made of
    /// counts the Recycle Bin apart, and empty files not at all.
    #[test]
    fn folders_and_drives_are_counted_as_the_organizer_counts_them() {
        let index = index();
        let places = Places::default();
        let at = |path: &str| index.place_of(Path::new(path)).unwrap();
        let all = held(&index, "C:\\", &places, ROOT, &mut || true).unwrap();
        assert_eq!((all.bytes, all.files, all.cloud), (1270, 4, 50));
        let docs = held(&index, "C:\\", &places, at("Docs"), &mut || true).unwrap();
        assert_eq!((docs.bytes, docs.files, docs.cloud), (300, 2, 50));
        let drive = breakdown(&index, "C:\\", &places, ROOT, true, 133_800_000_000_000_000, &mut || true).unwrap();
        assert_eq!((drive.bytes, drive.files, drive.bin, drive.cloud), (1200, 2, 70, 50));
        assert_eq!(drive.kinds[Kind::Archive.at()], 900);
        assert_eq!(drive.kinds[Kind::Video.at()], 300);
        assert_eq!(drive.ages[0], (1200, 2));
        let bin = breakdown(&index, "C:\\", &places, at("$Recycle.Bin"), false, 133_800_000_000_000_000, &mut || true).unwrap();
        assert_eq!((bin.bytes, bin.bin), (70, 0), "the Recycle Bin asked for is counted");
    }
}
