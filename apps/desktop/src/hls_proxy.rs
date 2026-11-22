//! Loopback proxy for authenticated HTTP playback. The player receives only local, tokenized URLs.
//! Credentials stay inside reqwest and are attached only on the playlist's configured origin.

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use downlink_core::hls::{map_uris, rewrite_playlist};
use downlink_core::source::{PlayPlan, Surface};
use downlink_core::{AppError, Result, tls};
use hashlink::LinkedHashMap;
use reqwest::header::{
    ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, HeaderMap,
    HeaderName, HeaderValue, LOCATION, RANGE,
};
use reqwest::{Client, Response, StatusCode};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use url::Url;

const MAX_REDIRECTS: usize = 10;
const MAX_PLAYLIST_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESOURCE_URL_BYTES: usize = 8192;
const MAX_RESOURCE_COUNT: usize = 32_768;
const MAX_REGISTRY_BYTES: usize = 16 * 1024 * 1024;
const MAX_HANDLERS: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const IO_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// Aborts the listener and all active upstream requests when dropped.
pub struct HlsProxy {
    task: JoinHandle<()>,
}

impl Drop for HlsProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Resources {
    by_url: LinkedHashMap<Url, String>,
    by_route: HashMap<String, Url>,
    max_count: usize,
    url_bytes: usize,
    next_id: u64,
}

impl Resources {
    fn new(max_count: usize) -> Self {
        Self {
            by_url: LinkedHashMap::new(),
            by_route: HashMap::new(),
            max_count,
            url_bytes: 0,
            next_id: 0,
        }
    }

    fn route(&mut self, url: Url) -> String {
        if let Some(route) = self.by_url.to_back(&url) {
            return route.clone();
        }
        let url_len = url.as_str().len();
        while self.by_route.len() >= self.max_count || self.url_bytes + url_len > MAX_REGISTRY_BYTES
        {
            let Some((old_url, old_route)) = self.by_url.pop_front() else {
                break;
            };
            self.url_bytes -= old_url.as_str().len();
            self.by_route.remove(&old_route);
        }
        let id = self.next_id;
        self.next_id += 1;
        let route = with_extension(id.to_string(), &url);
        self.by_url.insert(url.clone(), route.clone());
        self.by_route.insert(route.clone(), url);
        self.url_bytes += url_len;
        route
    }

    fn resolve(&mut self, route: &str) -> Option<Url> {
        let url = self.by_route.get(route)?.clone();
        self.by_url.to_back(&url);
        Some(url)
    }
}

struct Upstream {
    root: Url,
    root_route: String,
    expected_playlist: bool,
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
    async fn fetch(&self, url: Url, range: Option<HeaderValue>) -> Result<Response> {
        tokio::time::timeout(RESPONSE_TIMEOUT, self.follow_redirects(url, range))
            .await
            .map_err(|_| AppError::network("The stream server took too long to answer."))?
    }

    async fn follow_redirects(&self, mut url: Url, range: Option<HeaderValue>) -> Result<Response> {
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
        let mut too_large = false;
        let mut expanded_bytes = body.len();
        let mut used = HashSet::new();
        let rewritten = map_uris(body, &mut |uri| {
            if unsupported || too_large {
                return None;
            }
            let original_len = uri.len();
            let uri = uri.trim();
            let Ok(mut url) = playlist.join(uri) else {
                unsupported = true;
                return None;
            };
            if let Some(base) = &self.recording_base
                && playlist.origin() == base.origin()
                && (Url::parse(uri).is_ok() || uri.starts_with("//"))
                && url.origin() != base.origin()
            {
                // Dispatcharr synthesizes recording segment addresses from the upstream
                // Host. Rebase that recording directory, leaving other resources anonymous.
                let (directory, _) = playlist.path().rsplit_once('/')?;
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
                    url = Url::parse(&rewrite_playlist(url.as_str(), base)).ok()?;
                }
            }
            if !matches!(url.scheme(), "http" | "https") {
                unsupported = true;
                return None;
            }
            if url.as_str().len() > MAX_RESOURCE_URL_BYTES {
                too_large = true;
                return None;
            }
            let route = resources.route(url);
            let local = format!("{}{route}", self.local_base);
            expanded_bytes += local.len().saturating_sub(original_len);
            if expanded_bytes > MAX_PLAYLIST_BYTES {
                too_large = true;
                return None;
            }
            used.insert(route);
            Some(local)
        });
        if unsupported {
            return Err(AppError::stream(
                "The playlist contains an unsupported resource address.",
            ));
        }
        if too_large
            || used
                .iter()
                .any(|route| !resources.by_route.contains_key(route))
        {
            return Err(AppError::stream("The playlist is too large."));
        }
        Ok(rewritten)
    }

    fn get(&self, path: &str) -> Option<(Url, bool)> {
        let resource = path.strip_prefix(&self.route_prefix)?;
        if resource == self.root_route {
            return Some((self.root.clone(), true));
        }
        let url = self.resources.lock().ok()?.resolve(resource)?;
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
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_RANGE,
        ACCEPT_RANGES,
        CONTENT_ENCODING,
    ] {
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
    let initial = if root {
        // libmpv probes the entire representation as an open-ended range. The un-ranged
        // prefetched response already contains everything requested by that first GET.
        upstream
            .initial_playlist
            .lock()
            .ok()
            .and_then(|mut cached| cached.take())
            .filter(|_| range.as_ref().is_none_or(|range| range == "bytes=0-"))
    } else {
        None
    };
    let mut response = match initial {
        Some(response) => response,
        None => upstream.fetch(url, range).await?,
    };
    let known_playlist = (root && upstream.expected_playlist)
        || is_playlist_url(response.url())
        || response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("mpegurl"));
    let mut bytes = Vec::new();
    const PREFIXES: [&[u8]; 2] = [b"#EXTM3U", b"\xef\xbb\xbf#EXTM3U"];
    if !known_playlist && response.status().is_success() {
        while PREFIXES
            .iter()
            .any(|prefix| bytes.len() < prefix.len() && prefix.starts_with(&bytes))
        {
            let Some(chunk) = response.chunk().await? else {
                break;
            };
            bytes.extend_from_slice(&chunk);
        }
    }
    let playlist = known_playlist || PREFIXES.iter().any(|prefix| bytes.starts_with(prefix));
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
        let body = upstream.playlist(
            text.strip_prefix('\u{feff}').unwrap_or(text),
            response.url(),
        )?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {PLAYLIST_CONTENT_TYPE}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        *started = true;
        write_downstream(socket, response.as_bytes()).await?;
    } else {
        *started = true;
        write_downstream(socket, media_headers(&response).as_bytes()).await?;
        if response.status().is_success() {
            write_downstream(socket, &bytes).await?;
            while let Some(chunk) = response.chunk().await? {
                write_downstream(socket, &chunk).await?;
            }
        }
    }
    Ok(())
}

async fn write_downstream(socket: &mut TcpStream, mut bytes: &[u8]) -> Result<()> {
    while !bytes.is_empty() {
        let count = tokio::time::timeout(IO_IDLE_TIMEOUT, socket.write(bytes))
            .await
            .map_err(|_| AppError::network("The player stopped reading the stream."))??;
        if count == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
        }
        bytes = &bytes[count..];
    }
    Ok(())
}

async fn handle(mut socket: TcpStream, upstream: Arc<Upstream>) {
    let mut buf = [0u8; 8192];
    let request_size = async {
        let mut read = 0;
        while read < buf.len() {
            let chunk = socket.read(&mut buf[read..]).await.ok()?;
            if chunk == 0 {
                return None;
            }
            let previous = read;
            read += chunk;
            if buf[previous.saturating_sub(3)..read]
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
            {
                return Some(read);
            }
        }
        None
    };
    let size = tokio::time::timeout(REQUEST_TIMEOUT, request_size)
        .await
        .ok()
        .flatten();
    if let Some((path, range)) = size.and_then(|size| request_line(&buf[..size]))
        && let Some((url, root)) = upstream.get(path)
    {
        let mut started = false;
        if let Err(error) = serve(&mut socket, &upstream, url, root, range, &mut started).await {
            tracing::debug!("HLS resource request failed: {error}");
            if error.source_id().is_some()
                && let Some(app) = &upstream.app
                && !upstream.certificate_reported.swap(true, Ordering::Relaxed)
            {
                let _ = app.emit("source://certificate", &error);
            }
            if !started {
                let _ = write_downstream(
                    &mut socket,
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            }
        }
    } else {
        let _ = write_downstream(
            &mut socket,
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;
    }
    let _ = socket.shutdown().await;
}

fn with_extension(mut route: String, url: &Url) -> String {
    let name = url.path().rsplit('/').next().unwrap_or_default();
    if let Some((_, extension)) = name.rsplit_once('.')
        && !extension.is_empty()
        && extension.len() <= 12
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        route.push('.');
        route.push_str(extension);
    }
    route
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
    let root = match Url::parse(&plan.url) {
        Ok(url) => url,
        Err(error)
            if plan.url.split_once("://").is_some_and(|(scheme, _)| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            }) =>
        {
            return Err(error.into());
        }
        Err(_) => return Ok(None),
    };
    let expected_playlist = plan.surface == Surface::Hls || is_playlist_url(&root);
    if !matches!(root.scheme(), "https" | "http") || (plan.headers.is_empty() && !expected_playlist)
    {
        return Ok(None);
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &plan.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::stream("The stream has an invalid request header."))?;
        if matches!(
            name.as_str(),
            "host"
                | "connection"
                | "proxy-authorization"
                | "content-length"
                | "transfer-encoding"
                | "accept-encoding"
        ) {
            continue;
        }
        let mut value = HeaderValue::from_str(value.expose())
            .map_err(|_| AppError::stream("The stream has an invalid request header."))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    let builder = || {
        Client::builder()
            .user_agent(downlink_core::user_agent())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(IO_IDLE_TIMEOUT)
            .referer(false)
            .redirect(reqwest::redirect::Policy::none())
    };
    let anonymous = builder().build()?;
    let pin = plan.tls_pin.as_ref().filter(|pin| pin.matches(&root));
    let pinned = match pin {
        Some(pin) => tls::apply(builder(), &root, Some(pin))?
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        None => anonymous.clone(),
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let route_prefix = format!("/{}/", uuid::Uuid::new_v4().simple());
    let local_base = format!("http://127.0.0.1:{port}{route_prefix}");
    let root_route = if expected_playlist {
        "index.m3u8".to_owned()
    } else {
        with_extension("source".to_owned(), &root)
    };
    let local = format!("{local_base}{root_route}");
    let mut upstream = Upstream {
        recording_base: recording.then(|| recording_base(&root)),
        root,
        root_route,
        expected_playlist,
        headers,
        pinned,
        anonymous,
        local_base,
        route_prefix,
        resources: Mutex::new(Resources::new(MAX_RESOURCE_COUNT)),
        initial_playlist: Mutex::default(),
        app,
        certificate_reported: AtomicBool::new(false),
    };
    if expected_playlist && pin.is_some() {
        // Surface a changed certificate before starting a known playlist. Generic streams
        // are classified only when the player requests them.
        let response = upstream.fetch(upstream.root.clone(), None).await?;
        upstream.initial_playlist = Mutex::new(Some(response));
    }
    let upstream = Arc::new(upstream);
    let task = tokio::spawn(async move {
        let mut handlers = JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept(), if handlers.len() < MAX_HANDLERS => {
                    let Ok((socket, _)) = connection else { break };
                    handlers.spawn(handle(socket, upstream.clone()));
                }
                _ = handlers.join_next(), if !handlers.is_empty() => {}
            }
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
    use wiremock::matchers::{header, method, path, query_param};
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
    async fn extensionless_playlist_is_detected_on_the_first_player_request() {
        let source = MockServer::start().await;
        Mock::given(path("/watch"))
            .and(header("x-api-key", "fixture-key"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                b"\xef\xbb\xbf#EXTM3U\n#EXTINF:1,\npart.ts\n".to_vec(),
                "application/octet-stream",
            ))
            .expect(1)
            .mount(&source)
            .await;
        Mock::given(path("/part.ts"))
            .and(header("x-api-key", "fixture-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string("segment"))
            .expect(1)
            .mount(&source)
            .await;
        let mut plan = plan(format!("{}/watch", source.uri()));
        plan.surface = Surface::Direct;
        let _proxy = prepare(&mut plan, false, None).await.unwrap().unwrap();
        assert!(plan.headers.is_empty());
        assert!(source.received_requests().await.unwrap().is_empty());
        let body = get(&plan.url).await;
        assert!(body.starts_with("#EXTM3U\n"));
        assert_eq!(get(body.lines().last().unwrap()).await, "segment");
    }

    #[tokio::test]
    async fn direct_media_keeps_binary_bytes_and_range_semantics() {
        let source = MockServer::start().await;
        let data = [0, 255, 17, 23];
        Mock::given(path("/watch"))
            .and(header("x-api-key", "fixture-key"))
            .and(header("range", "bytes=2-5"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("content-range", "bytes 2-5/10")
                    .insert_header("accept-ranges", "bytes")
                    .set_body_raw(data.to_vec(), "video/mp2t"),
            )
            .expect(1)
            .mount(&source)
            .await;
        let mut plan = plan(format!("{}/watch", source.uri()));
        plan.surface = Surface::Ts;
        let _proxy = prepare(&mut plan, false, None).await.unwrap().unwrap();
        assert!(source.received_requests().await.unwrap().is_empty());
        let response = Client::new()
            .get(&plan.url)
            .header(RANGE, "bytes=2-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(response.headers()[ACCEPT_RANGES], "bytes");
        assert_eq!(response.bytes().await.unwrap().as_ref(), data);
    }

    #[test]
    fn route_cache_evicts_the_oldest_unused_path_without_reusing_it() {
        let mut resources = Resources::new(2);
        let first = Url::parse("http://example.test/1.ts").unwrap();
        let second = Url::parse("http://example.test/2.ts").unwrap();
        let third = Url::parse("http://example.test/3.ts").unwrap();
        let first_route = resources.route(first.clone());
        let second_route = resources.route(second.clone());
        assert_eq!(resources.resolve(&first_route), Some(first));
        let third_route = resources.route(third.clone());
        assert!(resources.resolve(&second_route).is_none());
        assert_eq!(resources.resolve(&third_route), Some(third));
        assert_ne!(resources.route(second), second_route);
        assert_eq!(resources.by_route.len(), 2);
        assert_eq!(resources.by_url.len(), 2);
    }

    #[tokio::test]
    async fn dropping_the_proxy_closes_an_in_flight_upstream_request() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let mut plan = plan(format!(
            "http://{}/index.m3u8",
            listener.local_addr().unwrap()
        ));
        let proxy = prepare(&mut plan, false, None).await.unwrap().unwrap();
        let request = tokio::spawn(async move { Client::new().get(plan.url).send().await });
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await.unwrap());
        }
        drop(proxy);
        let mut trailing = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut trailing))
            .await
            .expect("proxy dropped the upstream connection")
            .unwrap();
        assert_eq!(read, 0);
        assert!(request.await.unwrap().is_err());
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

    #[tokio::test]
    async fn resolves_segment_and_map_references_using_the_redirected_playlist_address() {
        let source = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(path("/index.m3u8"))
            .and(header("X-API-Key", "fixture-key"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                format!("{}/nested/video/live.m3u8", other.uri()),
            ))
            .expect(1)
            .mount(&source)
            .await;
        Mock::given(path("/nested/video/live.m3u8"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "#EXTM3U\n#EXT-X-MAP:URI=\"../init.mp4?quality=hd\"\n#EXTINF:4,\nseg.ts\n",
            ))
            .expect(1)
            .mount(&other)
            .await;
        Mock::given(path("/nested/init.mp4"))
            .and(query_param("quality", "hd"))
            .respond_with(ResponseTemplate::new(200).set_body_string("init"))
            .expect(1)
            .mount(&other)
            .await;
        Mock::given(path("/nested/video/seg.ts"))
            .respond_with(ResponseTemplate::new(200).set_body_string("video"))
            .expect(1)
            .mount(&other)
            .await;

        let mut plan = plan(format!("{}/index.m3u8", source.uri()));
        let _proxy = prepare(&mut plan, false, None).await.unwrap().unwrap();
        let playlist = get(&plan.url).await;
        let mut lines = playlist.lines().skip(1);
        assert_eq!(get(uri(lines.next().unwrap())).await, "init");
        assert_eq!(get(lines.nth(1).unwrap()).await, "video");
        assert!(other.received_requests().await.unwrap().iter().all(|req| {
            !req.headers.contains_key("x-api-key") && !req.headers.contains_key("referer")
        }));
    }
}
