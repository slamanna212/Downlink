#![allow(clippy::unwrap_used, clippy::expect_used)]
//! PLAN §2.6: self-signed sources are pinned to the server's leaf certificate; a changed
//! certificate blocks until the user trusts it. Against a local rustls server with two
//! synthetic self-signed certificates (tests/fixtures/tls, generated with openssl, no real
//! identity).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use downlink_core::auth::MemorySecretStore;
use downlink_core::service::Service;
use downlink_core::store::{NewSource, Store};
use downlink_core::{ErrorCode, tls};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

const HOST: &str = "127.0.0.1";

fn fx(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/tls/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn fp(name: &str) -> String {
    String::from_utf8(fx(name)).unwrap().trim().to_owned()
}

fn config(cert: &str) -> Arc<rustls::ServerConfig> {
    let certs = CertificateDer::pem_slice_iter(&fx(&format!("{cert}.crt")))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let key = PrivateKeyDer::from_pem_slice(&fx(&format!("{cert}.key"))).unwrap();
    Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap(),
    )
}

/// A one-thread HTTPS server answering every request with `ok`. The certificate it presents can
/// be switched while it runs.
struct TlsServer {
    port: u16,
    current: Arc<Mutex<Arc<rustls::ServerConfig>>>,
}

impl TlsServer {
    fn start(cert: &str) -> Self {
        let listener = TcpListener::bind((HOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let current = Arc::new(Mutex::new(config(cert)));
        let cur = current.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(tcp) = stream else { continue };
                let cfg = cur.lock().unwrap().clone();
                std::thread::spawn(move || {
                    let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                        return;
                    };
                    let mut tls = rustls::StreamOwned::new(conn, tcp);
                    let mut buf = [0u8; 2048];
                    // Reading drives the handshake; a client that rejects the certificate errors here.
                    if tls.read(&mut buf).is_ok() {
                        let _ = tls.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        );
                        let _ = tls.flush();
                    }
                });
            }
        });
        Self { port, current }
    }

    fn switch_to(&self, cert: &str) {
        *self.current.lock().unwrap() = config(cert);
    }

    fn url(&self) -> String {
        format!("https://{HOST}:{}", self.port)
    }
}

async fn get(url: &str, pin: Option<&tls::Pin>) -> downlink_core::Result<String> {
    let url = reqwest::Url::parse(url)?;
    let client = tls::apply(reqwest::Client::builder(), &url, pin)?
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    Ok(client.get(url).send().await?.text().await?)
}

#[tokio::test]
async fn the_fingerprint_of_what_the_server_presents_is_learned() {
    let s = TlsServer::start("a");
    assert_eq!(
        tls::fetch_fingerprint(&s.url()).await.unwrap(),
        fp("a.sha256")
    );
    s.switch_to("b");
    assert_eq!(
        tls::fetch_fingerprint(&s.url()).await.unwrap(),
        fp("b.sha256")
    );
}

#[tokio::test]
async fn a_pinned_certificate_connects_and_a_different_one_is_refused() {
    let s = TlsServer::start("a");
    let pin = tls::Pin::new(&s.url(), fp("a.sha256"), Some(42)).unwrap();
    assert_eq!(get(&s.url(), Some(&pin)).await.unwrap(), "ok");

    s.switch_to("b");
    let e = get(&s.url(), Some(&pin)).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth, "{e:?}");
    assert!(e.blocking);
    assert_eq!(e.message, "The server's certificate changed.");
    assert_eq!(e.source_id, Some(42));

    // Only an explicit new pin lets the new certificate in.
    let replacement = tls::Pin::new(&s.url(), fp("b.sha256"), Some(42)).unwrap();
    assert_eq!(get(&s.url(), Some(&replacement)).await.unwrap(), "ok");
}

async fn service_with_source(url: &str, pin: Option<&str>) -> (Arc<Service>, i64) {
    let dir = tempfile::tempdir().unwrap();
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        Arc::new(MemorySecretStore::new()),
        dir.path().join("img"),
        "t".into(),
    );
    let row = svc
        .store()
        .insert_source(&NewSource {
            kind: "xtream".into(),
            name: "T".into(),
            base_url: Some(url.to_owned()),
            secret_ref: None,
            username: Some("u".into()),
            tls_insecure: true,
            created_at: 1,
        })
        .await
        .unwrap();
    svc.store().set_source_tls_pin(row.id, pin).await.unwrap();
    std::mem::forget(dir); // the image dir is never written here
    (svc, row.id)
}

#[tokio::test]
async fn trusting_the_new_certificate_re_pins_the_source() {
    let s = TlsServer::start("a");
    let (svc, id) = service_with_source(&s.url(), Some(&fp("a.sha256"))).await;
    s.switch_to("b");
    svc.trust_new_certificate(id).await.unwrap();
    let row = svc.store().source(id).await.unwrap();
    assert_eq!(row.tls_pin.as_deref(), Some(fp("b.sha256").as_str()));
}

#[tokio::test]
async fn trust_new_certificate_refuses_a_source_that_does_not_allow_self_signed() {
    let (svc, id) = service_with_source("https://example.invalid", None).await;
    sqlx::query("UPDATE source SET tls_insecure = 0 WHERE id = ?")
        .bind(id)
        .execute(svc.store().pool())
        .await
        .unwrap();
    assert!(svc.trust_new_certificate(id).await.is_err());
}

#[tokio::test]
async fn a_missing_pin_rejects_a_self_signed_certificate() {
    let s = TlsServer::start("a");
    assert!(get(&s.url(), None).await.is_err());
}

#[tokio::test]
async fn a_saved_https_source_does_not_enroll_a_missing_pin() {
    let s = TlsServer::start("a");
    let (svc, id) = service_with_source(&s.url(), None).await;
    let error = svc.sync_source(id).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Auth);
    assert!(error.message.contains("no trusted certificate"));
    assert_eq!(error.source_id, Some(id));
    assert!(svc.store().source(id).await.unwrap().tls_pin.is_none());
    svc.trust_new_certificate(id).await.unwrap();
    assert_eq!(
        svc.store().source(id).await.unwrap().tls_pin,
        Some(fp("a.sha256"))
    );
}

#[tokio::test]
async fn pins_are_isolated_between_sources_on_the_same_host() {
    let a = TlsServer::start("a");
    let b = TlsServer::start("b");
    let pin_a = tls::Pin::new(&a.url(), fp("a.sha256"), Some(1)).unwrap();
    let pin_b = tls::Pin::new(&b.url(), fp("b.sha256"), Some(2)).unwrap();
    let url_a = a.url();
    let url_b = b.url();
    let (response_a, response_b) =
        tokio::join!(get(&url_a, Some(&pin_a)), get(&url_b, Some(&pin_b)));
    assert_eq!(response_a.unwrap(), "ok");
    assert_eq!(response_b.unwrap(), "ok");
    assert_eq!(
        get(&b.url(), Some(&pin_a)).await.unwrap_err().code,
        ErrorCode::Network
    );

    // Two sources can also trust different fingerprints at exactly the same address.
    let sibling = tls::Pin::new(&a.url(), fp("b.sha256"), Some(3)).unwrap();
    assert_eq!(
        get(&a.url(), Some(&sibling)).await.unwrap_err().source_id,
        Some(3)
    );
    assert_eq!(get(&a.url(), Some(&pin_a)).await.unwrap(), "ok");
}
