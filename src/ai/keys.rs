//! The keys of the user's AI providers' accounts: each one taken from the environment variable its provider's own
//! tools read (as `DEEPSEEK_API_KEY`) where that is set, else as the user gave it on the AI component's page, kept in
//! Glance's local folder protected for this Windows user alone (DPAPI), never in the settings. A key is only ever sent
//! to its own provider, and never shown or written down as it is.

use std::collections::BTreeMap;
use std::path::Path;

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

/// The file the keys given are kept in (in the local folder): each provider's, protected, as hex.
const FILE: &str = "ai-keys.json";
/// What the keys are protected with besides the user's own: a program of the user's that reads them must know it.
const ENTROPY: &[u8] = b"Glance AI provider key";

/// Where a provider's key is taken from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kept {
    /// The environment variable of that name.
    Env(&'static str),
    /// As the user gave it.
    Given,
    None,
}

/// Where the key of provider `id` (whose tools read it from the variables `env`) is taken from: the one given here,
/// where one is (given in Glance, it is what is meant), else the first of `env` set.
pub fn kept(id: &str, env: &[&'static str]) -> Kept {
    kept_in(&crate::settings::local_dir(), id, env)
}

fn kept_in(dir: &Path, id: &str, env: &[&'static str]) -> Kept {
    if read(dir).contains_key(id) {
        return Kept::Given;
    }
    env.iter().find(|name| from_env(name).is_some()).map_or(Kept::None, |name| Kept::Env(name))
}

/// The key of provider `id`, as given, else from the first of `env` set (none where neither is: see `kept`).
pub fn key(id: &str, env: &[&'static str]) -> Option<String> {
    key_in(&crate::settings::local_dir(), id, env)
}

fn key_in(dir: &Path, id: &str, env: &[&'static str]) -> Option<String> {
    match read(dir).get(id) {
        Some(given) => unprotect(&hex_bytes(given)?).and_then(|bytes| String::from_utf8(bytes).ok()),
        None => env.iter().find_map(|name| from_env(name)),
    }
}

/// The variable `name`, where it is set to something (spaces round it taken away).
fn from_env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

/// Keeps `key` as the one given for provider `id` (spaces round it taken away); an empty one is none.
pub fn give(id: &str, key: &str) -> std::io::Result<()> {
    give_in(&crate::settings::local_dir(), id, key)
}

fn give_in(dir: &Path, id: &str, key: &str) -> std::io::Result<()> {
    let mut keys = read(dir);
    match key.trim() {
        "" => keys.remove(id),
        key => keys.insert(id.to_string(), hex(&protect(key.as_bytes()).ok_or_else(std::io::Error::last_os_error)?)),
    };
    std::fs::create_dir_all(dir)?;
    super::write_whole(dir, FILE, &serde_json::to_vec_pretty(&keys).unwrap_or_default())
}

/// The keys given, by provider, protected (none where the file is not there or does not read).
fn read(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read(dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// `plain` protected for this Windows user (and Glance's entropy).
fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    let given = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptProtectData(&given, windows::core::PCWSTR::null(), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(taken(out))
}

/// What `protected` was, where it was protected for this Windows user with Glance's entropy.
fn unprotect(protected: &[u8]) -> Option<Vec<u8>> {
    let given = CRYPT_INTEGER_BLOB { cbData: protected.len() as u32, pbData: protected.as_ptr() as *mut u8 };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe { CryptUnprotectData(&given, None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(taken(out))
}

/// The bytes of `blob` (as DPAPI gives them), its memory let go of.
fn taken(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(blob.pbData.cast()))) };
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    text.len().is_multiple_of(2).then_some(())?;
    (0..text.len()).step_by(2).map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key given is kept protected (not as it was given) and read back as it was; given empty, it is none; a
    /// variable set is taken where none is given, and the one given before it.
    #[test]
    fn keeps_a_key_given_protected_and_before_the_variable() {
        let dir = crate::settings::local_dir().join("keys");
        let made_up = "sk-made-up-0123456789";
        give_in(&dir, "deepseek", &format!(" {made_up} ")).unwrap();
        let file = std::fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(!file.contains(made_up) && !file.contains(&hex(made_up.as_bytes())));
        assert_eq!(key_in(&dir, "deepseek", &[]).as_deref(), Some(made_up));
        assert_eq!((kept_in(&dir, "deepseek", &[]), kept_in(&dir, "kimi", &[])), (Kept::Given, Kept::None));
        // (A variable only this test sets.)
        const VARIABLE: &str = "GLANCE_TEST_AI_KEY_VARIABLE";
        unsafe { std::env::set_var(VARIABLE, "sk-from-the-variable") };
        assert_eq!(key_in(&dir, "deepseek", &[VARIABLE]).as_deref(), Some(made_up));
        assert_eq!(kept_in(&dir, "deepseek", &[VARIABLE]), Kept::Given);
        give_in(&dir, "deepseek", "  ").unwrap();
        assert_eq!(key_in(&dir, "deepseek", &[VARIABLE]).as_deref(), Some("sk-from-the-variable"));
        assert_eq!(kept_in(&dir, "deepseek", &[VARIABLE]), Kept::Env(VARIABLE));
        unsafe { std::env::remove_var(VARIABLE) };
        assert_eq!((key_in(&dir, "deepseek", &[]), kept_in(&dir, "deepseek", &[])), (None, Kept::None));
        // Protected data that is not Glance's reads as none.
        assert!(unprotect(b"not protected").is_none() && hex_bytes("abc").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
