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
use tokio::sync::Mutex as AsyncMutex;

const HOST: &str = "127.0.0.1";

/// The pin registry is per host and these tests share one host: run them one at a time.
static SERIAL: AsyncMutex<()> = AsyncMutex::const_new(());

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

async fn get(url: &str) -> downlink_core::Result<String> {
    let client = tls::apply(reqwest::Client::builder(), true)?
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    Ok(client.get(url).send().await?.text().await?)
}

#[tokio::test]
async fn the_fingerprint_of_what_the_server_presents_is_learned() {
    let _g = SERIAL.lock().await;
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
    let _g = SERIAL.lock().await;
    let s = TlsServer::start("a");
    tls::set_pin(HOST, &fp("a.sha256"));
    assert_eq!(get(&s.url()).await.unwrap(), "ok");

    s.switch_to("b");
    let e = get(&s.url()).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth, "{e:?}");
    assert!(e.blocking);
    assert_eq!(e.message, "The server's certificate changed.");

    // Only an explicit new pin lets the new certificate in.
    tls::set_pin(HOST, &fp("b.sha256"));
    assert_eq!(get(&s.url()).await.unwrap(), "ok");
    tls::clear_pin(HOST);
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
    let _g = SERIAL.lock().await;
    let s = TlsServer::start("a");
    let (svc, id) = service_with_source(&s.url(), Some(&fp("a.sha256"))).await;
    s.switch_to("b");
    svc.trust_new_certificate(id).await.unwrap();
    let row = svc.store().source(id).await.unwrap();
    assert_eq!(row.tls_pin.as_deref(), Some(fp("b.sha256").as_str()));
    assert_eq!(tls::pin_for(HOST).as_deref(), Some(fp("b.sha256").as_str()));
    tls::clear_pin(HOST);
}

#[tokio::test]
async fn trust_new_certificate_refuses_a_source_that_does_not_allow_self_signed() {
    let _g = SERIAL.lock().await;
    let (svc, id) = service_with_source("https://example.invalid", None).await;
    sqlx::query("UPDATE source SET tls_insecure = 0 WHERE id = ?")
        .bind(id)
        .execute(svc.store().pool())
        .await
        .unwrap();
    assert!(svc.trust_new_certificate(id).await.is_err());
}
