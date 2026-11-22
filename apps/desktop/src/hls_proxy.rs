//! Loopback playlist proxy for in-progress recordings (PLAN M4, `docs/dispatcharr-endpoints.md`).
//!
//! Dispatcharr writes the absolute segment URLs of a recording's HLS playlist from the request
//! `Host`; behind a reverse proxy they can point at an origin the client cannot reach. When core's
//! `needs_playlist_rewrite` says so, mpv is given `http://127.0.0.1:<port>/<token>/index.m3u8`
//! instead. Every request to it fetches the real playlist (with the per-file auth header, an
//! actual play-time request), rewrites it with `downlink_core::hls::rewrite_playlist`, and
//! returns it. Segments still go straight to the server, with the per-file header.
//! The path token is random and only loopback is bound, so other local processes cannot use it
//! to reach the server with our credentials unless they can read the token.

use std::time::Duration;

use downlink_core::hls::{needs_playlist_rewrite, playlist_origin, rewrite_playlist};
use downlink_core::source::PlayPlan;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

/// Aborts the proxy task when dropped (new playback, stop, quit).
pub struct HlsProxy {
    task: JoinHandle<()>,
}

impl Drop for HlsProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn fetch(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
) -> Result<String, String> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| e.without_url().to_string())?;
    if !resp.status().is_success() {
        return Err(format!(
            "playlist request answered {}",
            resp.status().as_u16()
        ));
    }
    resp.text().await.map_err(|e| e.without_url().to_string())
}

fn token() -> String {
    format!(
        "{}{}",
        downlink_player::player::new_crash_id(),
        downlink_player::player::new_crash_id()
    )
}

/// If `plan` is an HLS playlist whose segment URLs need rewriting, start the proxy and return
/// `(local playlist URL, proxy handle)`. `None` means play the plan as it is.
pub async fn prepare(plan: &PlayPlan) -> Option<(String, HlsProxy)> {
    let plan_url = Url::parse(&plan.url).ok()?;
    // The configured server base: scheme, host, port and any path prefix before `/proxy/`.
    let mut base = plan_url.clone();
    let prefix = plan_url
        .path()
        .find("/proxy/")
        .map_or("", |i| &plan_url.path()[..i]);
    base.set_path(if prefix.is_empty() { "/" } else { prefix });
    base.set_query(None);
    let headers: Vec<(String, String)> = plan
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.expose().to_owned()))
        .collect();
    let client = downlink_core::http::authenticated(reqwest::Client::builder())
        .user_agent(downlink_core::user_agent())
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let body = match fetch(&client, &plan.url, &headers).await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!("playlist probe failed, playing the original URL: {e}");
            return None;
        }
    };
    let origin = playlist_origin(&body)?;
    if !needs_playlist_rewrite(&base, &origin) {
        return None;
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.ok()?;
    let port = listener.local_addr().ok()?.port();
    let tok = token();
    let local = format!("http://127.0.0.1:{port}/{tok}/index.m3u8");
    tracing::info!(
        "recording playlist origin differs from the server; serving a rewritten playlist locally"
    );
    let (upstream, hdrs) = (plan.url.clone(), headers);
    let want = format!("/{tok}/index.m3u8");
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (client, upstream, hdrs, base, want) = (
                client.clone(),
                upstream.clone(),
                hdrs.clone(),
                base.clone(),
                want.clone(),
            );
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("");
                let (status, body) = if path == want {
                    match fetch(&client, &upstream, &hdrs).await {
                        Ok(b) => ("200 OK", rewrite_playlist(&b, &base)),
                        Err(e) => {
                            tracing::debug!("playlist proxy upstream failed: {e}");
                            ("502 Bad Gateway", String::new())
                        }
                    }
                } else {
                    ("404 Not Found", String::new())
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Some((local, HlsProxy { task }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use downlink_core::source::{PlayProfile, Surface};

    /// Minimal upstream answering every request with `body(port)`; returns its port.
    async fn upstream(body: impl Fn(u16) -> String + Send + 'static) -> u16 {
        let l = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = l.local_addr().expect("addr").port();
        let b = body(port);
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = l.accept().await else {
                    break;
                };
                let b = b.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf).await;
                    let r = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{b}",
                        b.len()
                    );
                    let _ = s.write_all(r.as_bytes()).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        port
    }

    async fn get(url: &str) -> (u16, String) {
        let r = reqwest::get(url).await.expect("get");
        (r.status().as_u16(), r.text().await.unwrap_or_default())
    }

    fn plan(url: String) -> PlayPlan {
        PlayPlan {
            url,
            headers: vec![],
            surface: Surface::Hls,
            profile: PlayProfile::Seekable,
            session: None,
            redact: Default::default(),
            needs_redirect_resolve: false,
            unknown_profile: false,
        }
    }

    #[tokio::test]
    async fn rewrites_foreign_segment_urls_and_guards_the_path() {
        let pl = "#EXTM3U\n#EXTINF:6,\nhttp://other.invalid:9999/seg/1.ts\n#EXTINF:6,\nhttp://other.invalid:9999/seg/2.ts\n";
        let port = upstream(move |_| pl.to_owned()).await;
        let p = plan(format!("http://127.0.0.1:{port}/proxy/hls/rec/index.m3u8"));
        let Some((local, _guard)) = prepare(&p).await else {
            panic!("expected a proxy")
        };
        let (status, body) = get(&local).await;
        assert_eq!(status, 200);
        assert!(
            body.contains(&format!("http://127.0.0.1:{port}/seg/1.ts")),
            "{body}"
        );
        assert!(!body.contains("other.invalid"));
        // Anything but the exact tokenised path is refused.
        let (head, _) = local.rsplit_once('/').expect("path");
        assert_eq!(get(&format!("{head}/other.m3u8")).await.0, 404);
        let host = &local[..local[8..].find('/').map(|i| i + 8).expect("slash")];
        assert_eq!(get(&format!("{host}/guess/index.m3u8")).await.0, 404);
        assert_eq!(get(&format!("{host}/")).await.0, 404);
    }

    #[tokio::test]
    async fn same_origin_playlists_are_left_alone() {
        let port =
            upstream(|port| format!("#EXTM3U\n#EXTINF:6,\nhttp://127.0.0.1:{port}/seg/1.ts\n"))
                .await;
        assert!(
            prepare(&plan(format!("http://127.0.0.1:{port}/x.m3u8")))
                .await
                .is_none()
        );
    }
}
