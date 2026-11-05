//! Glance's own signature on its installers, beside whatever Windows is shown: an installer of a release has a file
//! `<installer>.sig` beside it, ECDSA P-256 over the words of `message`, in hexadecimal (r, then s), made by
//! `scripts\update-signature.ps1` with a key that is Glance's alone, which no service lends and none can take back.
//! An update is run on this (see `update`) whatever became of the certificates of the releases. The version is signed
//! with the bytes: an older installer cannot be passed off under a newer name.

use windows::Win32::Security::Cryptography::{BCryptDestroyKey, BCryptImportKeyPair, BCryptVerifySignature, BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALG_HANDLE, BCRYPT_ECDSA_PUBLIC_P256_MAGIC, BCRYPT_FLAGS, BCRYPT_KEY_HANDLE};

use crate::serif::{hex, sha256};

/// The public half of Glance's update key (x, then y). Its private half is in the environment of the one who releases
/// (see `scripts\new-update-key.ps1`), never here.
const PUBLIC: &str = "c452fb3d3f2618e0b4b0cc0d46b5f9e3c2d3c3f3629484509981f0d5b7812309cdcb2eb7c45ec3f5949812b0b5f69ff2a6c62d087dac012d0e79aac2aa821ff6";

/// Whether `signature` (what the installer's `.sig` holds) is Glance's own, on these bytes as this version.
pub fn is_glances(version: &str, installer: &[u8], signature: &[u8]) -> bool {
    verifies(PUBLIC, version, installer, signature)
}

fn verifies(public: &str, version: &str, installer: &[u8], signature: &[u8]) -> bool {
    let (Some(public), Some(signature)) = (unhexed(public.as_bytes()), unhexed(signature.trim_ascii())) else { return false };
    let Some(message) = message(version, installer) else { return false };
    let Some(digest) = sha256(message.as_bytes()) else { return false };
    // (As Windows has a public key: what it is, how long each of its two numbers is, and the numbers.)
    let mut blob = Vec::with_capacity(8 + public.len());
    blob.extend(BCRYPT_ECDSA_PUBLIC_P256_MAGIC.to_le_bytes());
    blob.extend(32u32.to_le_bytes());
    blob.extend(public);
    let mut key = BCRYPT_KEY_HANDLE::default();
    unsafe {
        if BCryptImportKeyPair(BCRYPT_ECDSA_P256_ALG_HANDLE, None, BCRYPT_ECCPUBLIC_BLOB, &mut key, &blob, 0).is_err() {
            return false;
        }
        let verified = BCryptVerifySignature(key, None, &digest, &signature, BCRYPT_FLAGS(0)).is_ok();
        let _ = BCryptDestroyKey(key);
        verified
    }
}

/// What is signed.
fn message(version: &str, installer: &[u8]) -> Option<String> {
    Some(format!("Glance installer\n{version}\n{}\n", hex(&sha256(installer)?)))
}

/// 64 bytes, from their 128 hexadecimal digits, or nothing.
fn unhexed(digits: &[u8]) -> Option<[u8; 64]> {
    if digits.len() != 128 {
        return None;
    }
    let digit = |d: u8| (d as char).to_digit(16).map(|value| value as u8);
    let mut bytes = [0u8; 64];
    for (byte, pair) in bytes.iter_mut().zip(digits.chunks(2)) {
        *byte = digit(pair[0])? << 4 | digit(pair[1])?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Signed by `scripts\update-signature.ps1` with the update key itself, as a release is: `test-installer.bin` holding
    // the bytes below, as a version no Glance would ever take (see `update::newer`).
    const VERSION: &str = "0.0.0-test";
    const INSTALLER: &[u8] = b"not an installer\n";
    const SIGNATURE: &[u8] = b"b833527066209c6858f83789fd1977f4f2133b59ea18828696660d73621a483e6828f24e7ceec1500324f7b379e2bc896d20f9621de0d1e69e7a31cc9047762e";

    #[test]
    fn what_the_release_script_signs_is_believed() {
        assert!(is_glances(VERSION, INSTALLER, SIGNATURE));
        // (As a file holds it, ended by a line or not.)
        assert!(is_glances(VERSION, INSTALLER, &[SIGNATURE, b"\r\n"].concat()));
    }

    #[test]
    fn an_installer_altered_is_refused() {
        assert!(!is_glances(VERSION, b"not an installer!\n", SIGNATURE));
    }

    /// The version is signed with the bytes: an older installer under a newer name is not taken.
    #[test]
    fn an_installer_under_another_version_is_refused() {
        assert!(!is_glances("99.0.0", INSTALLER, SIGNATURE));
    }

    #[test]
    fn a_signature_altered_or_not_one_is_refused() {
        let mut altered = SIGNATURE.to_vec();
        altered[10] = if altered[10] == b'0' { b'1' } else { b'0' };
        assert!(!is_glances(VERSION, INSTALLER, &altered));
        for not_one in [&b""[..], b"zz", &SIGNATURE[2..], &[b'z'; 128], &[0xE4; 128], &[b'0'; 128]] {
            assert!(!is_glances(VERSION, INSTALLER, not_one));
        }
    }

    /// Signed by another key is signed by no one.
    #[test]
    fn another_key_is_refused() {
        // (Another point on the curve: the base point's, the public half of the key that is 1.)
        let another = "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
        assert!(!verifies(another, VERSION, INSTALLER, SIGNATURE));
        assert!(!verifies("not a key", VERSION, INSTALLER, SIGNATURE));
    }
}
