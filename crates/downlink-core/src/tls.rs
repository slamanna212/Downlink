//! Certificate pinning for sources that allow a self-signed certificate (PLAN §2.6).
//!
//! The toggle never turns verification off silently. A source that allows self-signed
//! certificates is pinned to the SHA-256 of the server's leaf certificate (DER, lowercase hex). It
//! is stored on the source row and only used by that source's HTTP clients on its exact HTTPS
//! origin. Every handshake on that origin must present the pinned certificate. A changed
//! certificate blocks the source until the user trusts it (`Service::trust_new_certificate`).

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};

use reqwest::{ClientBuilder, Url};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, OtherError, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

/// Text carried by the handshake error when a pinned certificate changed.
const CHANGED_MARKER: &str = "downlink-pinned-certificate-changed";

/// A fingerprint trusted for one source and HTTPS origin.
#[derive(Clone, Debug)]
pub struct Pin {
    origin: url::Origin,
    server_name: String,
    fingerprint: String,
    source_id: Arc<AtomicI64>,
}

impl Pin {
    pub fn new(url: &str, fingerprint: String, source_id: Option<i64>) -> Result<Self> {
        let url = Url::parse(url.trim())?;
        let server_name = url
            .host_str()
            .filter(|_| url.scheme() == "https")
            .ok_or_else(|| AppError::local("Only https addresses have a certificate."))?;
        Ok(Self {
            origin: url.origin(),
            server_name: server_name.trim_matches(['[', ']']).to_owned(),
            fingerprint: fingerprint.to_ascii_lowercase(),
            source_id: Arc::new(AtomicI64::new(source_id.unwrap_or(0))),
        })
    }

    pub fn matches(&self, url: &Url) -> bool {
        self.origin == url.origin()
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub(crate) fn set_source_id(&self, source_id: i64) {
        self.source_id.store(source_id, Ordering::Relaxed);
    }

    fn source_id(&self) -> Option<i64> {
        let id = self.source_id.load(Ordering::Relaxed);
        (id != 0).then_some(id)
    }

    pub(crate) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        // rustls receives the hostname without a port, so keep redirects within the pinned
        // request's origin. Callers select this builder only for matching request URLs.
        Ok(crate::http::authenticated(
            builder.tls_backend_preconfigured(pinned_config(Verification::Pinned(self.clone()))?),
        ))
    }
}

/// Apply a pin only on its exact origin. Other destinations use ordinary certificate verification.
pub fn apply(builder: ClientBuilder, url: &Url, pin: Option<&Pin>) -> Result<ClientBuilder> {
    match pin.filter(|pin| pin.matches(url)) {
        Some(pin) => pin.apply(builder),
        None => Ok(builder),
    }
}

// Debug carries the marker too: rustls prints wrapped errors with `{:?}` in places.
struct CertChanged(Option<i64>);

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

pub(crate) fn certificate_error(e: &(dyn std::error::Error + 'static)) -> Option<AppError> {
    let mut cur = Some(e);
    let mut found_marker = false;
    while let Some(err) = cur {
        let changed = err.downcast_ref::<CertChanged>().or_else(|| {
            let rustls::Error::InvalidCertificate(CertificateError::Other(other)) =
                err.downcast_ref::<rustls::Error>()?
            else {
                return None;
            };
            other.0.downcast_ref::<CertChanged>()
        });
        if let Some(changed) = changed {
            return Some(
                AppError::auth("The server's certificate changed.").with_source_id(changed.0),
            );
        }
        found_marker |=
            err.to_string().contains(CHANGED_MARKER) || format!("{err:?}").contains(CHANGED_MARKER);
        cur = err
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| err.source());
    }
    found_marker.then(|| AppError::auth("The server's certificate changed."))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

#[derive(Debug)]
struct PinVerifier {
    algs: WebPkiSupportedAlgorithms,
    verification: Verification,
}

#[derive(Debug)]
enum Verification {
    /// Record the certificate without credentials during an explicit trust operation.
    Capture(Arc<Mutex<Option<String>>>),
    Pinned(Pin),
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
        match &self.verification {
            Verification::Capture(seen) => {
                if let Ok(mut recorded) = seen.lock() {
                    *recorded = Some(fp);
                }
                Ok(ServerCertVerified::assertion())
            }
            Verification::Pinned(pin)
                if pin.server_name.eq_ignore_ascii_case(&server_name.to_str()) =>
            {
                if pin.fingerprint == fp {
                    return Ok(ServerCertVerified::assertion());
                }
                Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                    OtherError(Arc::new(CertChanged(pin.source_id()))),
                )))
            }
            Verification::Pinned(_) => Err(rustls::Error::InvalidCertificate(
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

fn pinned_config(verification: Verification) -> Result<rustls::ClientConfig> {
    let p = provider();
    let verifier = PinVerifier {
        algs: p.signature_verification_algorithms,
        verification,
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

/// Whether an address has an HTTPS certificate.
pub fn is_https(url: &str) -> bool {
    Url::parse(url.trim()).is_ok_and(|url| url.scheme() == "https")
}

/// Connect to `url` and return the SHA-256 fingerprint of the certificate it presents, without
/// judging it. Sends one request to the URL's origin root; the response is ignored. The caller
/// shows the fingerprint / asks the user before it is stored.
pub async fn fetch_fingerprint(url: &str) -> Result<String> {
    let u = Url::parse(url.trim()).map_err(|_| AppError::local("That address is not valid."))?;
    if u.scheme() != "https" {
        return Err(AppError::local("Only https addresses have a certificate."));
    }
    let seen: Arc<Mutex<Option<String>>> = Arc::default();
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(pinned_config(Verification::Capture(seen.clone()))?)
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
        assert_eq!(
            certificate_error(&Wrap(Box::new(CertChanged(Some(1)))))
                .and_then(|error| error.source_id),
            Some(1)
        );
        assert!(certificate_error(&Wrap(Box::new(std::fmt::Error))).is_none());
    }
}
