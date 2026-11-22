//! Certificate pinning for sources that allow a self-signed certificate (PLAN §2.6).
//!
//! The toggle never turns verification off silently. A source that allows self-signed
//! certificates is pinned to the SHA-256 of the server's leaf certificate (DER, lowercase hex). It
//! is stored on the source row (a fingerprint is not a secret) and registered here by host before
//! the source is used. Every TLS handshake to a host that has a pin must present exactly that
//! certificate; a different one fails the handshake with a marker that [`is_cert_changed`] turns
//! into a blocking, auth-class "The server's certificate changed." until the user trusts the new
//! one (`Service::trust_new_certificate`).
//!
//! Pins are keyed by host (a rustls verifier only sees the server name), so two sources on the
//! same host share a pin. Sources that do not allow self-signed certificates are unaffected and
//! use normal verification.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use reqwest::ClientBuilder;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, OtherError, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

/// Text carried by the handshake error when a pinned certificate changed.
const CHANGED_MARKER: &str = "downlink-pinned-certificate-changed";

static PINS: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(Default::default);

// Debug carries the marker too: rustls prints wrapped errors with `{:?}` in places.
struct CertChanged;

impl std::fmt::Debug for CertChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CHANGED_MARKER)
    }
}

impl std::fmt::Display for CertChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CHANGED_MARKER)
    }
}

impl std::error::Error for CertChanged {}

/// SHA-256 of a DER certificate as lowercase hex.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn key(host: &str) -> String {
    host.trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase()
}

/// Register the pinned fingerprint for a host (replaces any earlier one).
pub fn set_pin(host: &str, fingerprint: &str) {
    if let Ok(mut g) = PINS.write() {
        g.insert(key(host), fingerprint.to_ascii_lowercase());
    }
}

/// Forget a host's pin.
pub fn clear_pin(host: &str) {
    if let Ok(mut g) = PINS.write() {
        g.remove(&key(host));
    }
}

pub fn pin_for(host: &str) -> Option<String> {
    PINS.read().ok().and_then(|g| g.get(&key(host)).cloned())
}

/// Whether an error from the HTTP stack is a pinned-certificate mismatch.
pub fn is_cert_changed(e: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        if err.to_string().contains(CHANGED_MARKER) || format!("{err:?}").contains(CHANGED_MARKER) {
            return true;
        }
        cur = err.source();
    }
    false
}

pub fn cert_changed_error() -> AppError {
    // Blocking and auth-class: nothing else should be tried until the user decides.
    AppError::auth("The server's certificate changed.")
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[derive(Debug)]
struct PinVerifier {
    algs: WebPkiSupportedAlgorithms,
    /// Record the leaf fingerprint and accept (used only to learn a pin the user is about to
    /// confirm). Otherwise the host's registered pin is enforced.
    capture: Option<Arc<Mutex<Option<String>>>>,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let fp = fingerprint(end_entity.as_ref());
        if let Some(c) = &self.capture {
            if let Ok(mut g) = c.lock() {
                *g = Some(fp);
            }
            return Ok(ServerCertVerified::assertion());
        }
        match pin_for(&server_name.to_str()) {
            Some(expected) if expected == fp => Ok(ServerCertVerified::assertion()),
            Some(_) => Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(CertChanged)),
            ))),
            None => Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            )),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn pinned_config(capture: Option<Arc<Mutex<Option<String>>>>) -> Result<rustls::ClientConfig> {
    let p = provider();
    let verifier = PinVerifier {
        algs: p.signature_verification_algorithms,
        capture,
    };
    let mut cfg = rustls::ClientConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()
        .map_err(|_| AppError::local("Could not set up secure connections."))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(cfg)
}

/// Apply the source's TLS policy to a client builder: normal verification when the toggle is off,
/// pin-enforcing verification when it is on.
pub fn apply(builder: ClientBuilder, allow_self_signed: bool) -> Result<ClientBuilder> {
    if !allow_self_signed {
        return Ok(builder);
    }
    Ok(builder.tls_backend_preconfigured(pinned_config(None)?))
}

/// The host (and nothing else) of an `https://` URL; `None` for plain HTTP or anything else, which
/// has no certificate to pin.
pub fn https_host(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    (u.scheme() == "https")
        .then(|| u.host_str().map(str::to_owned))
        .flatten()
}

/// Connect to `url` and return the SHA-256 fingerprint of the certificate it presents, without
/// judging it. Sends one request to the URL's origin root; the response is ignored. The caller
/// shows the fingerprint / asks the user before it is stored.
pub async fn fetch_fingerprint(url: &str) -> Result<String> {
    let u = url::Url::parse(url).map_err(|_| AppError::local("That address is not valid."))?;
    if u.scheme() != "https" {
        return Err(AppError::local("Only https addresses have a certificate."));
    }
    let seen: Arc<Mutex<Option<String>>> = Arc::default();
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(pinned_config(Some(seen.clone()))?)
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut root = u.clone();
    root.set_path("/");
    root.set_query(None);
    root.set_fragment(None);
    let _ = root.set_username("");
    let _ = root.set_password(None);
    // Any HTTP answer is fine; only a failed handshake or connection is an error here.
    client.get(root).send().await?;
    seen.lock()
        .ok()
        .and_then(|g| g.clone())
        .ok_or_else(|| AppError::network("The server did not present a certificate."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_sha256_hex() {
        assert_eq!(
            fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn pins_are_keyed_by_lowercased_host() {
        set_pin("PinHost.Test", "AA");
        assert_eq!(pin_for("pinhost.test").as_deref(), Some("aa"));
        clear_pin("pinhost.test");
        assert_eq!(pin_for("pinhost.test"), None);
    }

    #[test]
    fn marker_is_found_down_the_error_chain() {
        #[derive(Debug)]
        struct Wrap(Box<dyn std::error::Error + Send + Sync>);
        impl std::fmt::Display for Wrap {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("outer")
            }
        }
        impl std::error::Error for Wrap {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(self.0.as_ref())
            }
        }
        assert!(is_cert_changed(&Wrap(Box::new(CertChanged))));
        assert!(!is_cert_changed(&Wrap(Box::new(std::fmt::Error))));
    }
}
