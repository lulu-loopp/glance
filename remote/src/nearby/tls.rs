//! This PC's certificate for nearby transfer, and the TLS each side of a transfer speaks.
//!
//! LocalSend's devices know each other by their certificates: a device's fingerprint is the SHA-256 of its
//! certificate (DER), in capitals, and a sender pins the receiver's to the one it announced. So this PC makes one
//! certificate of its own, once, and keeps it (with its key) in the helper's folder: its fingerprint is this PC's
//! for as long as that folder is kept.
//!
//! A sender may present a certificate of its own (LocalSend's newer apps do, and Glance's app does): it is asked for
//! and not required, and one presented is checked to be held by whoever presents it (the handshake's signature), so
//! its fingerprint is the sender's beyond doubt. Its issuer is nobody's business: every device signs its own.

use std::path::Path;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use sha2::{Digest, Sha256};

/// The certificate's file, and its key's (both DER), in the helper's folder.
const CERT: &str = "nearby-cert.der";
const KEY: &str = "nearby-key.der";

/// This PC's certificate and its key.
pub struct Own {
    pub cert: CertificateDer<'static>,
    key: Vec<u8>,
    /// Its fingerprint, as LocalSend's devices know it by.
    pub fingerprint: String,
}

impl Own {
    /// The certificate kept in `folder`, or one made now and kept there (none could be read: one made anew, and this
    /// PC is a new device to those that knew it).
    pub fn in_folder(folder: &Path) -> std::io::Result<Own> {
        let (cert_at, key_at) = (folder.join(CERT), folder.join(KEY));
        if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_at), std::fs::read(&key_at)) {
            if let Some(own) = Own::of(cert, key) {
                return Ok(own);
            }
        }
        let (cert, key) = made()?;
        std::fs::create_dir_all(folder)?;
        // (The key first: a certificate kept without its key is made again.)
        std::fs::write(&key_at, &key)?;
        std::fs::write(&cert_at, &cert)?;
        Own::of(cert, key).ok_or_else(|| std::io::Error::other("the certificate just made does not hold"))
    }

    /// A certificate and its key as kept, if they are one pair that rustls takes.
    fn of(cert: Vec<u8>, key: Vec<u8>) -> Option<Own> {
        let own = Own { fingerprint: fingerprint(&cert), cert: CertificateDer::from(cert), key };
        own.server().ok()?;
        Some(own)
    }

    fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }

    /// How this PC answers as a receiver: by its certificate, asking the sender for one of its own (not requiring it).
    pub fn server(&self) -> Result<Arc<rustls::ServerConfig>, rustls::Error> {
        let provider = provider();
        let verifier = Arc::new(AnyOwnCert { algorithms: provider.signature_verification_algorithms });
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![self.cert.clone()], self.key())?;
        Ok(Arc::new(config))
    }

    /// How this PC sends to the device known by `fingerprint`: presenting its own certificate, and talking on only
    /// to one that presents the certificate of that fingerprint.
    pub fn client(&self, fingerprint: &str) -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
        let provider = provider();
        let verifier = Arc::new(Pinned { fingerprint: fingerprint.to_ascii_uppercase(), algorithms: provider.signature_verification_algorithms });
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(vec![self.cert.clone()], self.key())?;
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
impl Own {
    /// As `client`, presenting no certificate (as an older LocalSend does).
    pub fn client_anonymous(&self, fingerprint: &str) -> Arc<rustls::ClientConfig> {
        let provider = provider();
        let verifier = Arc::new(Pinned { fingerprint: fingerprint.to_ascii_uppercase(), algorithms: provider.signature_verification_algorithms });
        Arc::new(rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().dangerous().with_custom_certificate_verifier(verifier).with_no_client_auth())
    }
}

/// A certificate (DER)'s fingerprint, as LocalSend's devices write it: its SHA-256 in capital hex.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|byte| format!("{byte:02X}")).collect()
}

/// A certificate of this PC's own, and its key (PKCS#8), both DER: ECDSA P-256, named "Glance", good from 2000 to
/// the year 2999 (a device's certificate is its name for as long as it is kept: it is not to run out).
fn made() -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let failed = |error: rcgen::Error| std::io::Error::other(error.to_string());
    let key = rcgen::KeyPair::generate().map_err(failed)?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(failed)?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(rcgen::DnType::CommonName, "Glance");
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2999, 12, 31);
    let cert = params.self_signed(&key).map_err(failed)?;
    Ok((cert.der().to_vec(), key.serialize_der()))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A sender's certificate, whoever made it, so long as it is held by the one presenting it (which the handshake's
/// signature, checked here, shows).
#[derive(Debug)]
struct AnyOwnCert {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyOwnCert {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, signed, &self.algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, signed, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// The receiver's certificate: the one of the fingerprint it announced, and no other (its name and issuer say
/// nothing: every device signs its own).
#[derive(Debug)]
struct Pinned {
    fingerprint: String,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(&self, cert: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>, _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(cert) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, signed, &self.algorithms)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, signed: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, signed, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made once and kept, the certificate is the same one read again (the same fingerprint), in 64 capital hex
    /// digits.
    #[test]
    fn a_certificate_made_is_kept_and_read_again() {
        let folder = std::env::temp_dir().join(format!("glance-nearby-cert-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let made = Own::in_folder(&folder).unwrap();
        let again = Own::in_folder(&folder).unwrap();
        assert_eq!(made.fingerprint, again.fingerprint);
        assert_eq!(made.fingerprint.len(), 64);
        assert!(made.fingerprint.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)));
        // One that cannot be read is made anew.
        std::fs::write(folder.join(KEY), b"broken").unwrap();
        let anew = Own::in_folder(&folder).unwrap();
        assert_ne!(anew.fingerprint, made.fingerprint);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
