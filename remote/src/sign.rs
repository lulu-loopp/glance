//! How a request proves it comes from the paired phone: signed with the key
//! shared at pairing, over what it asks, when, and a number used once. The
//! key itself never travels; a request overheard cannot be sent again.

use std::collections::VecDeque;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// How far a request's time may be from this PC's (the phone's clock and
/// the PC's are not one), and so how long its number is remembered.
const SKEW: Duration = Duration::from_secs(90);

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}

/// The key kept as `text`, if it is one (32 bytes in hex).
pub fn key(text: &str) -> Option<[u8; 32]> {
    unhex(text)?.try_into().ok()
}

/// What a phone names its key by, looking for its PC on the network (the
/// PC's address may have changed): the start of the key's digest, which
/// says nothing of the key to whoever hears it.
pub fn key_id(key: &[u8; 32]) -> String {
    use sha2::Digest;
    hex(&Sha256::digest(key)[..8])
}

/// The PC's answer to a phone looking for it: proof that it holds the
/// key, over the number the phone made up for this asking.
pub fn here(key: &[u8; 32], nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("a key of any length");
    mac.update(b"here\n");
    mac.update(nonce.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// Whether `given` is the key kept as `kept` (hex): compared as a whole, in the same time however much of it is
/// right, so that it cannot be found out a part at a time.
pub fn is_key(given: &str, kept: &str) -> bool {
    let (Some(given), Some(kept)) = (unhex(given), key(kept)) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(&kept).expect("a key of any length");
    mac.update(b"key\n");
    let mut asked = Hmac::<Sha256>::new_from_slice(&given).expect("a key of any length");
    asked.update(b"key\n");
    mac.verify(&asked.finalize().into_bytes()).is_ok()
}

/// A request as it came: what it asks, and what it gives to prove whose
/// it is.
pub struct Request<'a> {
    pub method: &'a str,
    pub path: &'a str,
    /// Seconds since 1970, by the phone's clock.
    pub time: &'a str,
    /// Any text of its own, used once.
    pub nonce: &'a str,
    /// Its signature, in hex.
    pub sign: &'a str,
    /// The number the phone gave itself, and what it calls itself (as it
    /// was sent: see `unescaped` where it is read).
    pub phone: &'a str,
    pub name: &'a str,
    /// The helper's run the request is meant for (see `Helper::session`).
    pub session: &'a str,
    pub body: &'a [u8],
}

fn mac(key: &[u8; 32], parts: &[&str], body: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("a key of any length");
    for part in parts {
        mac.update(part.as_bytes());
        mac.update(b"\n");
    }
    mac.update(body);
    mac
}

/// What a request signs: its method, its path, its time, its number,
/// which phone it is and what it is called, the run of the helper it is
/// for, and its body.
fn signing(key: &[u8; 32], request: &Request) -> Hmac<Sha256> {
    mac(key, &[request.method, request.path, request.time, request.nonce, request.phone, request.name, request.session], request.body)
}

/// A request's signature, in hex: what goes in its `sign`.
pub fn sign(key: &[u8; 32], request: &Request) -> String {
    hex(&signing(key, request).finalize().into_bytes())
}

/// The signature of an answer: by the key of the request it answers, over
/// that request's number, so that it is this PC's and for that asking.
pub fn answer(key: &[u8; 32], nonce: &str, status: u16, body: &[u8]) -> String {
    hex(&mac(key, &["answer", nonce, &status.to_string()], body).finalize().into_bytes())
}

/// Why a request is not taken.
#[derive(Debug, PartialEq)]
pub enum Refused {
    Unsigned,
    /// Of another time than now, or meant for an earlier run of the helper.
    Stale,
    Replayed,
    Forged,
    /// Its key is another phone's: this one is to be paired for itself.
    Taken,
}

/// Whether the request is signed with `key`: then the time it says it is of.
pub fn verify(key: &[u8; 32], request: &Request) -> Result<u64, Refused> {
    let (Ok(at), Some(given)) = (request.time.parse::<u64>(), unhex(request.sign)) else { return Err(Refused::Unsigned) };
    if request.nonce.is_empty() || request.nonce.len() > 64 {
        return Err(Refused::Unsigned);
    }
    // Compared whole, in the same time whatever differs.
    signing(key, request).verify_slice(&given).map_err(|_| Refused::Forged)?;
    Ok(at)
}

/// Whether a request of that time is of now, near enough.
pub fn timely(at: u64, now: u64) -> bool {
    at.abs_diff(now) <= SKEW.as_secs()
}

/// The numbers of the requests taken lately, each with its time and the
/// key it was signed with (by that key's name): one seen again under the
/// same key is a request sent again.
#[derive(Default)]
pub struct Seen(VecDeque<(u64, String, String)>);

impl Seen {
    /// Takes a request's number as used, unless it was already.
    pub fn take(&mut self, key: &[u8; 32], at: u64, nonce: &str, now: u64) -> Result<(), Refused> {
        let id = key_id(key);
        self.0.retain(|(seen, ..)| seen.abs_diff(now) <= 2 * SKEW.as_secs());
        if self.0.iter().any(|(_, whose, seen)| *whose == id && seen == nonce) {
            return Err(Refused::Replayed);
        }
        self.0.push_back((at, id, nonce.to_string()));
        Ok(())
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_named_and_answered_for_without_being_said() {
        let key = [7u8; 32];
        assert_eq!(key_id(&key).len(), 16);
        assert_ne!(key_id(&key), key_id(&[8u8; 32]));
        // The answer is for that asking alone, and that key's alone.
        assert_ne!(here(&key, "aa"), here(&key, "ab"));
        assert_ne!(here(&key, "aa"), here(&[8u8; 32], "aa"));
        assert_eq!(here(&key, "aa").len(), 64);
    }

    #[test]
    fn only_the_paired_phones_requests_are_taken_and_once() {
        let key = [7u8; 32];
        let asked = |path: &'static str, nonce: &'static str, phone: &'static str, body: &'static [u8]| Request { method: "POST", path, time: "1000", nonce, sign: "", phone, name: "a%20phone", session: "s1", body };
        let signed = |request: &Request| hex(&signing(&key, request).finalize().into_bytes());
        let good = asked("/v1/lights", "a1", "p1", b"{\"on\":false}");
        let sign = signed(&good);
        assert_eq!(verify(&key, &Request { sign: &sign, ..asked("/v1/lights", "a1", "p1", b"{\"on\":false}") }), Ok(1000));
        // Another body, path, phone, name, run or key under the same signature.
        assert_eq!(verify(&key, &Request { sign: &sign, ..asked("/v1/lights", "a1", "p1", b"{\"on\":true}") }), Err(Refused::Forged));
        assert_eq!(verify(&key, &Request { sign: &sign, ..asked("/v1/sleep", "a1", "p1", b"{\"on\":false}") }), Err(Refused::Forged));
        assert_eq!(verify(&key, &Request { sign: &sign, ..asked("/v1/lights", "a1", "p2", b"{\"on\":false}") }), Err(Refused::Forged));
        assert_eq!(verify(&key, &Request { sign: &sign, name: "another", ..asked("/v1/lights", "a1", "p1", b"{\"on\":false}") }), Err(Refused::Forged));
        assert_eq!(verify(&key, &Request { sign: &sign, session: "s2", ..asked("/v1/lights", "a1", "p1", b"{\"on\":false}") }), Err(Refused::Forged));
        assert_eq!(verify(&[8u8; 32], &Request { sign: &sign, ..asked("/v1/lights", "a1", "p1", b"{\"on\":false}") }), Err(Refused::Forged));
        // Not signed at all.
        assert_eq!(verify(&key, &Request { time: "", nonce: "", ..asked("/v1/sleep", "", "", b"") }), Err(Refused::Unsigned));
        // Of now, or kept and sent much later.
        assert!(timely(1000, 1010) && timely(1000, 950) && !timely(1000, 5000));
        // Once: again under the same key is a request sent again; under another key it is that key's own.
        let mut seen = Seen::default();
        assert_eq!(seen.take(&key, 1000, "a1", 1010), Ok(()));
        assert_eq!(seen.take(&key, 1000, "a1", 1011), Err(Refused::Replayed));
        assert_eq!(seen.take(&[8u8; 32], 1000, "a1", 1011), Ok(()));
        // An answer is for that asking, that outcome and those words alone.
        assert_ne!(answer(&key, "a1", 200, b"{}"), answer(&key, "a2", 200, b"{}"));
        assert_ne!(answer(&key, "a1", 200, b"{}"), answer(&key, "a1", 401, b"{}"));
        assert_ne!(answer(&key, "a1", 200, b"{}"), answer(&key, "a1", 200, b"{ }"));
        assert!(super::key(&hex(&key)).is_some() && super::key("12").is_none());
    }

    /// The night key is taken whole and only so: not a part of it, nor another key, nor what is no key.
    #[test]
    fn the_night_key_is_taken_whole() {
        let kept = hex(&[7u8; 32]);
        assert!(super::is_key(&kept, &kept));
        assert!(super::is_key(&kept.to_uppercase(), &kept));
        assert!(!super::is_key(&kept[..62], &kept));
        assert!(!super::is_key(&hex(&[8u8; 32]), &kept));
        assert!(!super::is_key("", &kept));
        assert!(!super::is_key("zz", &kept));
        assert!(!super::is_key(&kept, ""));
    }
}
