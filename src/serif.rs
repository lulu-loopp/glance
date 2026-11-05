//! The serif face the music full screen's lines are drawn in: Noto Serif SC Black (Simplified Chinese, with Latin),
//! under the SIL Open Font License. Some 12 MB, it is not shipped: it is taken from Glance's site the first time the
//! full screen opens (served there as Noto's own project published it: see `scripts\site.ps1`), kept in Glance's
//! local folder, and checked against its digest before it is used, each time it is read. Till it is had, the lines
//! are drawn in Microsoft YaHei.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};

use crate::ui::canvas::Family;

/// Where it is taken from, and the digest it has (SHA-256: the file as Noto's project published it, at
/// notofonts/noto-cjk f8d157532fbfaeda587e826d4cd5b21a49186f7c, Serif/SubsetOTF/SC/NotoSerifSC-Black.otf).
const URL: &str = "https://glancepc.com/dl/NotoSerifSC-Black.otf";
pub const DIGEST: &str = "91dde57622845beeb663f300813999febdb7fcd5e451bde199d7c954a9740cb4";
/// The name it is kept under in the local folder.
pub const FILE: &str = "NotoSerifSC-Black.otf";
/// The most it is read of a file kept under that name (one larger is not it).
const MOST: u64 = 16 * 1024 * 1024;

/// The face, once had.
static FACE: OnceLock<Vec<u8>> = OnceLock::new();
/// It is being looked for (read where it is kept, or taken from the site).
static SOUGHT: AtomicBool = AtomicBool::new(false);

/// The face, if it is had.
pub fn face() -> Option<&'static [u8]> {
    FACE.get().map(Vec::as_slice)
}

/// The family the full screen's lines are drawn in now: the serif face, once had; Microsoft YaHei till then.
pub fn family() -> Family {
    if face().is_some() { Family::Serif } else { Family::YaHei }
}

/// The face wanted (the full screen opening): read where it was kept, else taken from the site and kept, on a thread
/// of its own. Not had (the site not reached), it is looked for again the next time.
pub fn want() {
    if face().is_some() || SOUGHT.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new().name("serif".into()).spawn(|| {
        let folder = crate::settings::local_dir();
        let kept = crate::elevation::read_bytes_in_place(&folder, FILE, MOST).filter(|bytes| is_it(bytes));
        let had = kept.or_else(|| {
            let taken = crate::update::get(URL).filter(|bytes| is_it(bytes));
            match &taken {
                Some(bytes) => {
                    if let Err(error) = crate::elevation::ensure_folder(&folder).and_then(|()| crate::elevation::write_in_place(&folder, FILE, bytes)) {
                        crate::journal::note(format!("serif: taken, not kept ({error})"));
                    }
                }
                None => crate::journal::note("serif: not taken from the site; the lines stay in YaHei"),
            }
            taken
        });
        if let Some(bytes) = had {
            let _ = FACE.set(bytes);
        }
        SOUGHT.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        SOUGHT.store(false, Ordering::Release);
    }
}

/// The face had (the studio's frames, a picture's test drawing with it), if `bytes` are it.
#[cfg(feature = "studio")]
pub fn have(bytes: Vec<u8>) {
    assert!(is_it(&bytes), "not the face");
    let _ = FACE.set(bytes);
}

/// Whether `bytes` are the face: its digest.
fn is_it(bytes: &[u8]) -> bool {
    sha256(bytes).is_some_and(|digest| hex(&digest) == DIGEST)
}

pub(crate) fn sha256(bytes: &[u8]) -> Option<[u8; 32]> {
    let mut digest = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, bytes, &mut digest) }.ok().ok()?;
    Some(digest)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest is taken as SHA-256 is.
    #[test]
    fn digests_as_sha256_does() {
        assert_eq!(hex(&sha256(b"abc").unwrap()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
