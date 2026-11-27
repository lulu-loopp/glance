//! What a page sends and fetches, enciphered on the way (see `nearby::page`): the page is served over plain HTTP (a
//! phone's browser trusts no certificate of this PC's making), and its files and words cross the network as anyone on
//! it may read them. Each is enciphered with ChaCha20 under a key made from the one the page paired by, which the
//! network never carried (it came in the pairing code, after the link's #), with a number of its own used once. The
//! request or answer it travels in is signed over what is enciphered, so it cannot be changed on the way either.

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// How long the number used once is that each sealed thing begins with.
pub const NONCE: usize = 12;

/// The key what a page sends and fetches is sealed with: made from the key it paired by, never that key itself.
pub fn key(paired: &[u8; 32]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(paired).expect("a key of any length");
    mac.update(b"glance transfer");
    mac.finalize().into_bytes().into()
}

/// `plain` enciphered under `key`: a number used once, then what it enciphers to.
pub fn seal(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE];
    getrandom::getrandom(&mut nonce).expect("the system's random numbers");
    let mut sealed = Vec::with_capacity(NONCE + plain.len());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(plain);
    ChaCha20::new(key.into(), &nonce.into()).apply_keystream(&mut sealed[NONCE..]);
    sealed
}

/// What `sealed` (as `seal` makes it) was, where it is long enough to be one. (Whether it is what was sealed is the
/// signature's to say, over the sealed bytes.)
pub fn open(key: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    let (nonce, body) = sealed.split_at_checked(NONCE)?;
    let mut plain = body.to_vec();
    ChaCha20::new(key.into(), nonce.into()).apply_keystream(&mut plain);
    Some(plain)
}

#[cfg(test)]
mod tests {
    /// RFC 8439, 2.4.2: the cipher's own test, its counter at 1 (sealing begins at 0: the first block is the one
    /// before).
    #[test]
    fn it_enciphers_as_chacha20_does() {
        let key: [u8; 32] = std::array::from_fn(|at| at as u8);
        let nonce = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
        let plain = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let mut sealed = nonce.to_vec();
        sealed.extend_from_slice(&[0u8; 64]);
        sealed.extend_from_slice(plain);
        // (The block of counter 0 enciphers 64 zeros: the plain text then starts at counter 1, as the RFC's.)
        let opened = super::open(&key, &sealed).unwrap();
        let wanted = "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d";
        assert_eq!(crate::sign::hex(&opened[64..]), wanted);
    }

    #[test]
    fn what_is_sealed_opens_as_it_was_and_never_twice_alike() {
        let key = super::key(&[7; 32]);
        let plain = b"a photo's bytes";
        let (one, two) = (super::seal(&key, plain), super::seal(&key, plain));
        assert_ne!(one, two);
        assert!(!one.windows(plain.len()).any(|part| part == plain));
        assert_eq!(super::open(&key, &one).unwrap(), plain);
        assert_eq!(super::open(&key, &two).unwrap(), plain);
        assert_eq!(super::open(&key, &super::seal(&key, b"")).unwrap(), b"");
        assert!(super::open(&key, &[0; 11]).is_none());
    }
}
