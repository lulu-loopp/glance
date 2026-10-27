//! A drive's index kept on disk, in Glance's own folder (`%LOCALAPPDATA%\Glance\Search`), with where its journal
//! had come to when it was kept: read again as Glance starts, only what changed since is listed again. Written whole
//! to another file and put in its place, and checked whole as it is read; one that is not as written is not used.

use super::index::Index;
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"GLNCSRCH";
const VERSION: u32 = 1;

/// What the index was kept with: the drive it is of, and its journal and where it had come to (the changes before
/// that are in it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept {
    pub serial: u32,
    pub journal: u64,
    pub next: i64,
}

fn folder() -> PathBuf {
    crate::settings::local_dir().join("Search")
}

fn file(serial: u32) -> PathBuf {
    folder().join(format!("{serial:08X}.index"))
}

/// FNV-1a, 64 bits: whether what is read is what was written.
fn sum(bytes: &[u8]) -> u64 {
    let mut sum = 0xCBF2_9CE4_8422_2325u64;
    for &byte in bytes {
        sum = (sum ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
    }
    sum
}

/// `index` kept, with `kept`.
pub fn keep(index: &Index, kept: Kept) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&kept.serial.to_le_bytes());
    bytes.extend_from_slice(&kept.journal.to_le_bytes());
    bytes.extend_from_slice(&kept.next.to_le_bytes());
    index.to_bytes(&mut bytes);
    let sum = sum(&bytes);
    bytes.extend_from_slice(&sum.to_le_bytes());
    std::fs::create_dir_all(folder())?;
    let path = file(kept.serial);
    let new = path.with_extension("new");
    std::fs::write(&new, &bytes)?;
    std::fs::rename(&new, &path)
}

/// The index kept for the drive of `serial`, and what it was kept with; none where there is none, or it is not as
/// written.
pub fn kept(serial: u32) -> Option<(Index, Kept)> {
    let bytes = std::fs::read(file(serial)).ok()?;
    let (body, sum_bytes) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if sum(body) != u64::from_le_bytes(sum_bytes.try_into().ok()?) || body.get(..8)? != MAGIC {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?));
    let u64_at = |at: usize| Some(u64::from_le_bytes(body.get(at..at + 8)?.try_into().ok()?));
    if u32_at(8)? != VERSION {
        return None;
    }
    let kept = Kept { serial: u32_at(12)?, journal: u64_at(16)?, next: u64_at(24)? as i64 };
    if kept.serial != serial {
        return None;
    }
    Some((Index::from_bytes(body.get(32..)?)?, kept))
}
