//! Loopback proxy for authenticated HLS. The player receives only local, tokenized URLs.
//! Credentials stay inside reqwest and are attached only on the playlist's configured origin.

use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use downlink_core::hls::{map_uris, rewrite_playlist};
use downlink_core::source::{PlayPlan, Surface};
use downlink_core::{AppError, Result, tls};
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue,
    LOCATION, RANGE,
};
use reqwest::{Client, Response, StatusCode};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use url::Url;

const MAX_REDIRECTS: usize = 10;
const MAX_PLAYLIST_BYTES: usize = 8 * 1024 * 1024;
const PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// Aborts the proxy listener when dropped (new playback, stop, quit).
pub struct HlsProxy {
    task: JoinHandle<()>,
}

impl Drop for HlsProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
struct Resources {
    by_url: HashMap<Url, String>,
    by_route: HashMap<String, Url>,
}

impl Resources {
    fn route(&mut self, url: Url) -> String {
        if let Some(route) = self.by_url.get(&url) {
            return route.clone();
        }
        let id = self.by_url.len();
        let name = url.path().rsplit('/').next().unwrap_or_default();
        let extension = name.rsplit_once('.').map(|(_, ext)| ext).filter(|ext| {
            !ext.is_empty()
                && ext.len() <= 12
                && ext.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
        let route = match extension {
            Some(ext) => format!("{id}.{ext}"),
            None => id.to_string(),
        };
        self.by_url.insert(url.clone(), route.clone());
        self.by_route.insert(route.clone(), url);
        route
    }
}

struct Upstream {
    root: Url,
    recording_base: Option<Url>,
    headers: HeaderMap,
    pinned: Client,
    anonymous: Client,
    local_base: String,
    route_prefix: String,
    resources: Mutex<Resources>,
    initial_playlist: Mutex<Option<Response>>,
    app: Option<AppHandle>,
    certificate_reported: AtomicBool,
}

impl Upstream {
    async fn fetch(&self, mut url: Url, range: Option<HeaderValue>) -> Result<Response> {
        let mut send_credentials = true;
        for hop in 0..=MAX_REDIRECTS {
            let same_origin = url.origin() == self.root.origin();
            send_credentials &= same_origin;
            let client = if same_origin {
                &self.pinned
            } else {
                &self.anonymous
            };
            if !send_credentials {
                let _ = url.set_username("");
                let _ = url.set_password(None);
            }
            let mut req = client.get(url.clone());
            if send_credentials {
                req = req.headers(self.headers.clone());
            }
            if let Some(range) = &range {
                req = req.header(RANGE, range);
            }
            let response = req.send().await?;
            if !matches!(
                response.status(),
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            ) {
                return Ok(response);
            }
            let target = response
                .headers()
                .get(LOCATION)
                .and_then(|location| location.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .filter(|location| matches!(location.scheme(), "http" | "https"))
                .ok_or_else(|| {
                    AppError::stream("The playlist server redirected to an invalid address.")
                })?;
            if hop == MAX_REDIRECTS {
                break;
            }
            url = target;
        }
        Err(AppError::stream(
            "The playlist server redirected too many times.",
        ))
    }

    fn playlist(&self, body: &str, playlist: &Url) -> Result<String> {
        let mut resources = self
            .resources
            .lock()
            .map_err(|_| AppError::local("The playlist proxy is unavailable."))?;
        let mut unsupported = false;
        let rewritten = map_uris(body, &mut |uri| {
            let uri = uri.trim();
            let Ok(mut url) = playlist.join(uri) else {
                unsupported = true;
                return None;
            };
            if let Some(base) = &self.recording_base
                && Url::parse(uri).is_ok()
                && url.origin() != base.origin()
            {
                // Dispatcharr synthesizes recording segment addresses from the upstream
                // Host. Rebase that recording directory, leaving other resources anonymous.
                let (directory, _) = self.root.path().rsplit_once('/')?;
                let configured_prefix = base.path().trim_end_matches('/');
                let upstream_directory = directory
                    .strip_prefix(configured_prefix)
                    .unwrap_or(directory);
                if url
                    .path()
                    .strip_prefix(directory)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                    || url
                        .path()
                        .strip_prefix(upstream_directory)
                        .is_some_and(|suffix| suffix.starts_with('/'))
                {
                    url = Url::parse(&rewrite_playlist(uri, base)).ok()?;
                }
            }
            if !matches!(url.scheme(), "http" | "https") {
                unsupported = true;
                return None;
            }
            let route = resources.route(url);
            Some(format!("{}{route}", self.local_base))
        });
        if unsupported {
            return Err(AppError::stream(
                "The playlist contains an unsupported resource address.",
            ));
        }
        Ok(rewritten)
    }

    fn get(&self, path: &str) -> Option<(Url, bool)> {
        let resource = path.strip_prefix(&self.route_prefix)?;
        if resource == "index.m3u8" {
            return Some((self.root.clone(), true));
        }
        let url = self.resources.lock().ok()?.by_route.get(resource)?.clone();
        Some((url, false))
    }
}

fn request_line(buf: &[u8]) -> Option<(&str, Option<HeaderValue>)> {
    let request = std::str::from_utf8(buf).ok()?;
    let mut lines = request.lines();
    let mut command = lines.next()?.split_whitespace();
    if command.next()? != "GET" {
        return None;
    }
    let path = command.next()?;
    let range = lines
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("range"))
        .and_then(|(_, value)| HeaderValue::from_str(value.trim()).ok());
    Some((path, range))
}

fn media_headers(response: &Response) -> String {
    let status = response.status();
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    );
    if !status.is_success() {
        head.push_str("Content-Length: 0\r\n");
    }
    for name in [CONTENT_TYPE, CONTENT_LENGTH, CONTENT_RANGE, ACCEPT_RANGES] {
        if name == CONTENT_LENGTH && !status.is_success() {
            continue;
        }
        if let Some(value) = response.headers().get(&name).and_then(|v| v.to_str().ok()) {
            use std::fmt::Write;
            let _ = write!(head, "{name}: {value}\r\n");
        }
    }
    head.push_str("\r\n");
    head
}

async fn serve(
    socket: &mut TcpStream,
    upstream: &Upstream,
    url: Url,
    root: bool,
    range: Option<HeaderValue>,
    started: &mut bool,
) -> Result<()> {
    let initial = if root && range.is_none() {
        upstream
            .initial_playlist
            .lock()
            .ok()
            .and_then(|mut cached| cached.take())
    } else {
        None
    };
    let mut response = match initial {
        Some(response) => response,
        None => upstream.fetch(url.clone(), range).await?,
    };
    let known_playlist = root
        || is_playlist_url(response.url())
        || response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("mpegurl"));
    let mut bytes = Vec::new();
    if !known_playlist && response.status().is_success() {
        while bytes.len() < b"#EXTM3U".len() {
            let Some(chunk) = response.chunk().await? else {
                break;
            };
            bytes.extend_from_slice(&chunk);
        }
    }
    let playlist = known_playlist || bytes.starts_with(b"#EXTM3U");
    if playlist && response.status().is_success() {
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_PLAYLIST_BYTES {
                return Err(AppError::stream("The playlist is too large."));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() > MAX_PLAYLIST_BYTES {
            return Err(AppError::stream("The playlist is too large."));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| AppError::stream("The server returned an invalid playlist."))?;
        let body = upstream.playlist(text, &url)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {PLAYLIST_CONTENT_TYPE}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        *started = true;
        socket.write_all(response.as_bytes()).await?;
    } else {
        *started = true;
        socket
            .write_all(media_headers(&response).as_bytes())
            .await?;
        if response.status().is_success() {
            socket.write_all(&bytes).await?;
            while let Some(chunk) = response.chunk().await? {
                socket.write_all(&chunk).await?;
            }
        }
    }
    Ok(())
}

async fn handle(mut socket: TcpStream, upstream: Arc<Upstream>) {
    let mut buf = [0u8; 8192];
    let n = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(0);
    if let Some((path, range)) = request_line(&buf[..n])
        && let Some((url, root)) = upstream.get(path)
    {
        let mut started = false;
        if let Err(error) = serve(&mut socket, &upstream, url, root, range, &mut started).await {
            tracing::debug!("HLS resource request failed: {error}");
            if error.source_id.is_some()
                && let Some(app) = &upstream.app
                && !upstream.certificate_reported.swap(true, Ordering::Relaxed)
            {
                let _ = app.emit("source://certificate", &error);
            }
            if !started {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            }
        }
    } else {
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    }
    let _ = socket.shutdown().await;
}

fn is_playlist_url(url: &Url) -> bool {
    url.path().to_ascii_lowercase().ends_with(".m3u8")
}

fn recording_base(url: &Url) -> Url {
    let mut base = url.clone();
    let prefix = url
        .path()
        .find("/api/")
        .or_else(|| url.path().find("/proxy/"));
    base.set_path(prefix.map_or("/", |i| &url.path()[..i]));
    base.set_query(None);
    base.set_fragment(None);
    base
}

/// Give the player a local URL and keep per-file credentials inside the proxy.
pub async fn prepare(
    plan: &mut PlayPlan,
    recording: bool,
    app: Option<AppHandle>,
) -> Result<Option<HlsProxy>> {
    if plan.headers.is_empty() && !recording {
        return Ok(None);
    }
    let root = Url::parse(&plan.url);
    if plan.surface != Surface::Hls && !root.as_ref().is_ok_and(is_playlist_url) {
        return Ok(None);
    }
    let root = root?;
    if !matches!(root.scheme(), "https" | "http") {
        return Err(AppError::stream("The playlist address is invalid."));
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &plan.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::stream("The playlist has an invalid request header."))?;
        if matches!(
            name.as_str(),
            "host" | "connection" | "proxy-authorization" | "content-length" | "transfer-encoding"
        ) {
            continue;
        }
        let mut value = HeaderValue::from_str(value.expose())
            .map_err(|_| AppError::stream("The playlist has an invalid request header."))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    let builder = || {
        Client::builder()
            .user_agent(downlink_core::user_agent())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(30))
            .referer(false)
    };
    let pinned = tls::apply(builder(), &root, plan.tls_pin.as_ref())?
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let anonymous = builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let route_prefix = format!("/{}/", uuid::Uuid::new_v4().simple());
    let local_base = format!("http://127.0.0.1:{port}{route_prefix}");
    let local = format!("{local_base}index.m3u8");
    let mut upstream = Upstream {
        recording_base: recording.then(|| recording_base(&root)),
        root,
        headers,
        pinned,
        anonymous,
        local_base,
        route_prefix,
        resources: Mutex::default(),
        initial_playlist: Mutex::default(),
        app,
        certificate_reported: AtomicBool::new(false),
    };
    if plan
        .tls_pin
        .as_ref()
        .is_some_and(|pin| pin.matches(&upstream.root))
    {
        // Surface any changed certificate before handing the player its loopback URL.
        let response = upstream.fetch(upstream.root.clone(), None).await?;
        upstream.initial_playlist = Mutex::new(Some(response));
    }
    let upstream = Arc::new(upstream);
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(handle(socket, upstream.clone()));
        }
    });
    plan.redact.add(&local);
    plan.url = local;
    plan.headers.clear();
    Ok(Some(HlsProxy { task }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use downlink_core::auth::SecretString;
    use downlink_core::source::PlayProfile;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn plan(url: String) -> PlayPlan {
        PlayPlan {
            url,
            tls_pin: None,
            headers: vec![("X-API-Key".into(), SecretString::new("fixture-key"))],
            surface: Surface::Hls,
            profile: PlayProfile::Seekable,
            session: None,
            redact: Default::default(),
            needs_redirect_resolve: false,
            unknown_profile: false,
        }
    }

    async fn get(url: &str) -> String {
        reqwest::get(url)
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap()
    }

    fn uri(line: &str) -> &str {
        line.split_once("URI=\"")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0
    }

    #[tokio::test]
    async fn mixed_origins_and_redirects_keep_credentials_on_the_original_origin() {
        let source = MockServer::start().await;
        let other = MockServer::start().await;
        let root = format!(
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n{}/variant\n",
            source.uri()
        );
        let variant = format!(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"{}/key\"\n#EXT-X-MAP:URI=\"{}/init.mp4\"\n#EXTINF:1,\n{}/redirect.ts\n",
            source.uri(),
            other.uri(),
            source.uri(),
        );
        for (path_value, body) in [
            ("/index.m3u8", root),
            ("/variant", variant),
            ("/key", "secret media key".into()),
        ] {
            Mock::given(method("GET"))
                .and(path(path_value))
                .and(header("x-api-key", "fixture-key"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .expect(1)
                .mount(&source)
                .await;
        }
        Mock::given(path("/redirect.ts"))
            .and(header("x-api-key", "fixture-key"))
            .and(header("range", "bytes=1-3"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/segment.ts", other.uri())),
            )
            .expect(1)
            .mount(&source)
            .await;
        Mock::given(path("/init.mp4"))
            .respond_with(ResponseTemplate::new(200).set_body_string("init"))
            .expect(1)
            .mount(&other)
            .await;
        Mock::given(path("/segment.ts"))
            .and(header("range", "bytes=1-3"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-range", "bytes 1-3/4")
                    .set_body_string("abc"),
            )
            .expect(1)
            .mount(&other)
            .await;

        let mut plan = plan(format!("{}/index.m3u8", source.uri()));
        let _proxy = prepare(&mut plan, false, None).await.unwrap().unwrap();
        assert!(plan.headers.is_empty());
        let master = get(&plan.url).await;
        let nested = get(master.lines().last().unwrap()).await;
        let mut lines = nested.lines().skip(1);
        assert_eq!(get(uri(lines.next().unwrap())).await, "secret media key");
        assert_eq!(get(uri(lines.next().unwrap())).await, "init");
        let response = Client::new()
            .get(lines.nth(1).unwrap())
            .header(RANGE, "bytes=1-3")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes 1-3/4");
        assert_eq!(response.text().await.unwrap(), "abc");
        assert!(other.received_requests().await.unwrap().iter().all(|req| {
            !req.headers.contains_key("x-api-key") && !req.headers.contains_key("referer")
        }));

        let forged = format!(
            "{}/invalid/index.m3u8",
            plan.url.rsplit_once('/').unwrap().0
        );
        assert_eq!(
            reqwest::get(forged).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn rewrites_recording_segments_to_the_configured_server() {
        let source = MockServer::start().await;
        let prefix = "/proxy-prefix/api/channels/recordings/1/hls";
        Mock::given(path(format!("{prefix}/index.m3u8")))
            .and(header("X-API-Key", "fixture-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "#EXTM3U\n#EXTINF:4,\nhttp://internal.invalid/api/channels/recordings/1/hls/seg_0.ts\n",
            ))
            .mount(&source)
            .await;
        Mock::given(path(format!("{prefix}/seg_0.ts")))
            .and(header("X-API-Key", "fixture-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("segment"))
            .expect(1)
            .mount(&source)
            .await;
        let mut plan = plan(format!("{}{prefix}/index.m3u8", source.uri()));
        let _proxy = prepare(&mut plan, true, None).await.unwrap().unwrap();
        let playlist = get(&plan.url).await;
        assert_eq!(get(playlist.lines().last().unwrap()).await, "segment");
    }
}
