//! Hand-written Dispatcharr client (PLAN §3). reqwest + serde, tolerant deserialisation, DRF
//! pagination that never assumes `page_size`, API key primary with JWT fallback.
//!
//! **Network discipline (CLAUDE.md rule 4):** nothing in this module requests a stream endpoint
//! (`/proxy/ts/stream/`, `/proxy/hls/`, `/proxy/vod/`, catch-up session URLs). Plan builders only
//! construct URLs. The one exception is `resolve_redirect` in `source::play`, used at play time.

pub mod dto;
mod vod;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Method, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::Semaphore;

use self::dto::*;
use super::{
    AccountInfo, Caps, ChannelKey, ChannelPage, Page, PlayPlan, PlayProfile, ProfileInfo,
    ProgramSearch, Programme, RecordRequest, RecordingInfo, RuleInfo, SessionId, Source,
    SourceChannel, SourceGroup, SourceId, SourceKind, StreamInfo, StreamMode, Surface, TimeWindow,
    VodCategoryInfo, VodItem, VodKind, VodListPage, VodListQuery,
};
use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::redact::RedactionSet;

/// Minimum supported server version (PLAN §3.2).
pub const MIN_VERSION: (u32, u32, u32) = (0, 32, 0);
/// Per-source request concurrency (PLAN §2.5).
const MAX_CONCURRENT: usize = 2;
const MAX_PAGES: u32 = 10_000;

#[derive(Clone)]
pub enum Credentials {
    /// `X-API-Key` (primary).
    ApiKey(SecretString),
    /// JWT fallback: login, then refresh in the background of use.
    Password {
        username: String,
        password: SecretString,
    },
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey(_) => f.write_str("Credentials::ApiKey(***)"),
            Self::Password { username, .. } => write!(f, "Credentials::Password({username}, ***)"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// Self-signed certificates, behind an explicit per-source toggle only.
    pub insecure_tls: bool,
    pub timeout: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            insecure_tls: false,
            timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Default)]
struct Tokens {
    access: Option<SecretString>,
    /// When the access token expires, on the local clock (UTC seconds), when it could be read.
    access_exp: Option<i64>,
    refresh: Option<SecretString>,
}

/// Refresh the access token this long before it expires: play plans hand it to mpv, and no API
/// call (whose 401 would trigger a refresh) happens on the way.
const ACCESS_REFRESH_MARGIN_SECS: i64 = 120;

/// When a just-received JWT expires on the local clock: now plus its lifetime (`exp - iat`), so a
/// skewed client clock does not matter; `exp` itself when there is no `iat`. Read without verifying
/// (the server does that). Never logged.
fn jwt_exp(token: &str) -> Option<i64> {
    use base64::Engine as _;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims = serde_json::from_slice::<Value>(&bytes).ok()?;
    let exp = claims.get("exp").and_then(Value::as_i64)?;
    Some(match claims.get("iat").and_then(Value::as_i64) {
        Some(iat) => OffsetDateTime::now_utc().unix_timestamp() + (exp - iat),
        None => exp,
    })
}

#[derive(Debug, Clone, Default)]
struct ServerSettings {
    timezone: Option<String>,
    default_stream_profile: Option<i64>,
}

struct Reply {
    status: StatusCode,
    retry_after: Option<u64>,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn ok(self, what: &str) -> Result<Self> {
        if self.status.is_success() {
            Ok(self)
        } else {
            Err(AppError::from_status(
                self.status.as_u16(),
                self.retry_after,
                what,
            ))
        }
    }

    fn json(&self) -> Result<Value> {
        if self.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&self.body)?)
    }
}

/// A fetched logo.
#[derive(Debug, Clone)]
pub struct Logo {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// A created catch-up session.
#[derive(Clone)]
pub struct CatchupSession {
    pub id: SessionId,
    /// Absolute playback URL. A stream endpoint: give it to the player, never fetch it.
    pub url: String,
}

impl std::fmt::Debug for CatchupSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CatchupSession(***)")
    }
}

pub struct DispatcharrClient {
    id: SourceId,
    http: reqwest::Client,
    base: Url,
    creds: Credentials,
    insecure_tls: bool,
    tokens: tokio::sync::Mutex<Tokens>,
    sem: Semaphore,
    secrets: Mutex<RedactionSet>,
    settings: Mutex<Option<ServerSettings>>,
    has_vod: std::sync::atomic::AtomicBool,
    has_dvr: std::sync::atomic::AtomicBool,
    /// Admin accounts may switch a live channel's provider stream (`change_stream` is admin-only).
    can_switch: std::sync::atomic::AtomicBool,
    /// `epg data id -> tvg_id`, reused across the channel pages of one sync (see [`EPG_MAP_TTL`]).
    epg_map: tokio::sync::Mutex<Option<(std::time::Instant, EpgMap)>>,
    /// `m3u account id -> name` for the stream picker (see [`ACCOUNT_NAMES_TTL`]).
    account_names: tokio::sync::Mutex<Option<(std::time::Instant, AccountNames)>>,
    vod_categories: tokio::sync::Mutex<HashMap<VodKind, CachedVodCategories>>,
}

type CachedVodCategories = (std::time::Instant, Vec<VodCategoryInfo>);
const VOD_CATEGORY_TTL: Duration = Duration::from_secs(120);

/// `m3u account id -> name`.
type AccountNames = std::sync::Arc<HashMap<i64, String>>;

/// How long the provider account names serve the stream picker before they are fetched again.
/// A refused fetch (non-admin accounts) is cached for as long, so it is not retried on every open.
const ACCOUNT_NAMES_TTL: Duration = Duration::from_secs(600);

/// `epg data id -> tvg_id`.
type EpgMap = std::sync::Arc<HashMap<i64, String>>;

/// How long one fetched EPG data list serves channel pages before it is fetched again.
const EPG_MAP_TTL: Duration = Duration::from_secs(120);

impl std::fmt::Debug for DispatcharrClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DispatcharrClient({})",
            crate::redact::redact(self.base.as_str())
        )
    }
}

/// Parse `"0.32.0"`, `"v0.32.1-dev"`, `"0.33"` into a triple.
pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+', ' ']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u32>().ok());
    let major = it.next()??;
    let minor = it.next().flatten().unwrap_or(0);
    let patch = it.next().flatten().unwrap_or(0);
    Some((major, minor, patch))
}

fn fmt_time(t: i64) -> Result<String> {
    OffsetDateTime::from_unix_timestamp(t)
        .map_err(|_| AppError::local("A time was out of range."))?
        .format(&Rfc3339)
        .map_err(|_| AppError::local("A time could not be formatted."))
}

/// Accepts a bare host (`dispatcharr.test:9191`) or a URL; keeps any path prefix.
pub fn normalise_base_url(input: &str) -> Result<Url> {
    let trimmed = input.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let mut url = Url::parse(&with_scheme)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::local(
            "The server address must start with http:// or https://.",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = format!("{}/", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

impl DispatcharrClient {
    pub fn new(
        id: SourceId,
        base_url: &str,
        creds: Credentials,
        opts: ClientOptions,
    ) -> Result<Self> {
        let base = normalise_base_url(base_url)?;
        let http = crate::tls::apply(reqwest::Client::builder(), opts.insecure_tls)?
            .timeout(opts.timeout)
            .user_agent(crate::user_agent())
            .build()?;
        let mut secrets = RedactionSet::new();
        match &creds {
            Credentials::ApiKey(k) => {
                secrets.add(k.expose());
            }
            Credentials::Password { password, .. } => {
                secrets.add(password.expose());
            }
        }
        Ok(Self {
            id,
            http,
            base,
            creds,
            insecure_tls: opts.insecure_tls,
            tokens: tokio::sync::Mutex::new(Tokens::default()),
            sem: Semaphore::new(MAX_CONCURRENT),
            secrets: Mutex::new(secrets),
            settings: Mutex::new(None),
            has_vod: std::sync::atomic::AtomicBool::new(false),
            has_dvr: std::sync::atomic::AtomicBool::new(false),
            can_switch: std::sync::atomic::AtomicBool::new(false),
            epg_map: tokio::sync::Mutex::new(None),
            account_names: tokio::sync::Mutex::new(None),
            vod_categories: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Literal secrets this client knows about (API key, tokens) for redaction.
    pub fn redaction_set(&self) -> RedactionSet {
        self.secrets.lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn remember_secret(&self, s: &str) {
        if let Ok(mut g) = self.secrets.lock() {
            g.add(s);
        }
    }

    fn url(&self, path: &str) -> Result<Url> {
        Ok(self.base.join(path.trim_start_matches('/'))?)
    }

    // ---- auth -------------------------------------------------------------------------------

    async fn login(&self, tokens: &mut Tokens) -> Result<()> {
        let Credentials::Password { username, password } = &self.creds else {
            return Err(AppError::auth("The server rejected the API key."));
        };
        let resp = self
            .http
            .post(self.url("api/accounts/token/")?)
            .json(&json!({ "username": username, "password": password.expose() }))
            .send()
            .await?;
        let status = resp.status();
        let retry = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok()?.parse().ok());
        if !status.is_success() {
            return Err(
                if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
                    AppError::auth("The username or password was not accepted.")
                } else {
                    AppError::from_status(status.as_u16(), retry, "sign-in")
                },
            );
        }
        let t: TokenDto = resp.json().await?;
        let access = t
            .access
            .ok_or_else(|| AppError::server("The server did not return a token."))?;
        self.remember_secret(&access);
        tokens.access_exp = jwt_exp(&access);
        tokens.access = Some(SecretString::new(access));
        if let Some(r) = t.refresh {
            self.remember_secret(&r);
            tokens.refresh = Some(SecretString::new(r));
        }
        Ok(())
    }

    /// Refresh the access token; fall back to a fresh login if the refresh token is spent.
    async fn refresh(&self, tokens: &mut Tokens) -> Result<()> {
        if let Some(refresh) = tokens.refresh.clone() {
            let resp = self
                .http
                .post(self.url("api/accounts/token/refresh/")?)
                .json(&json!({ "refresh": refresh.expose() }))
                .send()
                .await?;
            if resp.status().is_success() {
                let t: TokenDto = resp.json().await?;
                if let Some(a) = t.access {
                    self.remember_secret(&a);
                    tokens.access_exp = jwt_exp(&a);
                    tokens.access = Some(SecretString::new(a));
                    if let Some(r) = t.refresh {
                        self.remember_secret(&r);
                        tokens.refresh = Some(SecretString::new(r));
                    }
                    return Ok(());
                }
            }
        }
        self.login(tokens).await
    }

    /// The auth header for a request: `X-API-Key`, or `Authorization: Bearer` (logging in lazily).
    pub async fn auth_header(&self) -> Result<(String, SecretString)> {
        match &self.creds {
            Credentials::ApiKey(k) => Ok(("X-API-Key".to_owned(), k.clone())),
            Credentials::Password { .. } => {
                let mut t = self.tokens.lock().await;
                if t.access.is_none() {
                    self.login(&mut t).await?;
                } else if t.access_exp.is_some_and(|exp| {
                    exp - OffsetDateTime::now_utc().unix_timestamp() < ACCESS_REFRESH_MARGIN_SECS
                }) {
                    self.refresh(&mut t).await?;
                }
                let a = t
                    .access
                    .clone()
                    .ok_or_else(|| AppError::auth("Not signed in."))?;
                Ok((
                    "Authorization".to_owned(),
                    SecretString::new(format!("Bearer {}", a.expose())),
                ))
            }
        }
    }

    // ---- transport --------------------------------------------------------------------------

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Reply> {
        let url = self.url(path)?;
        for attempt in 0..2 {
            let (hname, hval) = self.auth_header().await?;
            let _permit = self
                .sem
                .acquire()
                .await
                .map_err(|_| AppError::local("The client is shutting down."))?;
            let mut req = self.http.request(method.clone(), url.clone()).query(query);
            let mut v = reqwest::header::HeaderValue::from_str(hval.expose())
                .map_err(|_| AppError::auth("The credential contains invalid characters."))?;
            v.set_sensitive(true);
            req = req.header(hname, v);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            let status = resp.status();
            if status == StatusCode::UNAUTHORIZED
                && attempt == 0
                && matches!(self.creds, Credentials::Password { .. })
            {
                drop(_permit);
                let mut t = self.tokens.lock().await;
                self.refresh(&mut t).await?;
                continue;
            }
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok()?.trim().parse().ok());
            let content_type = resp
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = resp.bytes().await?.to_vec();
            return Ok(Reply {
                status,
                retry_after,
                content_type,
                body,
            });
        }
        Err(AppError::auth("The server rejected the credentials."))
    }

    async fn get_json(&self, path: &str, query: &[(&str, String)], what: &str) -> Result<Value> {
        self.call(Method::GET, path, query, None)
            .await?
            .ok(what)?
            .json()
    }

    /// One DRF page. Handles a plain array (not paginated), `{results,next,count}` and `{data}`.
    async fn page_values(
        &self,
        path: &str,
        extra: &[(&str, String)],
        page: u32,
        page_size: u32,
        what: &str,
    ) -> Result<(Vec<Value>, Option<u64>, bool)> {
        let mut q: Vec<(&str, String)> = vec![
            ("page", page.to_string()),
            ("page_size", page_size.to_string()),
        ];
        q.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        let v = self.get_json(path, &q, what).await?;
        match v {
            Value::Array(a) => Ok((a, None, false)),
            Value::Object(mut o) => {
                let count = o
                    .get("count")
                    .and_then(flex_i64)
                    .and_then(|c| u64::try_from(c).ok());
                let has_next = o
                    .get("next")
                    .is_some_and(|n| n.as_str().is_some_and(|s| !s.is_empty()));
                let items = match o.remove("results").or_else(|| o.remove("data")) {
                    Some(Value::Array(a)) => a,
                    _ => {
                        return Err(AppError::server(format!(
                            "The server's {what} had an unexpected shape."
                        )));
                    }
                };
                Ok((items, count, has_next))
            }
            _ => Err(AppError::server(format!(
                "The server's {what} had an unexpected shape."
            ))),
        }
    }

    /// Every page of a list endpoint. Never assumes `page_size` was honoured: it follows `next`
    /// until the server says there is none (or an empty page arrives).
    async fn fetch_all(
        &self,
        path: &str,
        extra: &[(&str, String)],
        what: &str,
    ) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut page = 1;
        loop {
            let (items, _count, has_next) = self.page_values(path, extra, page, 500, what).await?;
            let empty = items.is_empty();
            out.extend(items);
            if !has_next || empty || page >= MAX_PAGES {
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    // ---- identity ---------------------------------------------------------------------------

    pub async fn version(&self) -> Result<Option<String>> {
        let v = self
            .get_json("api/core/version/", &[], "its version")
            .await?;
        Ok(serde_json::from_value::<VersionDto>(v)?.version)
    }

    pub async fn me(&self) -> Result<MeDto> {
        let v = self
            .get_json("api/accounts/users/me/", &[], "your account")
            .await?;
        Ok(serde_json::from_value(v)?)
    }

    /// Server timezone and default stream profile from core settings. Failure is tolerated
    /// (non-admin keys may not read settings).
    async fn load_settings(&self) -> ServerSettings {
        let items = match self
            .fetch_all("api/core/settings/", &[], "its settings")
            .await
        {
            Ok(i) => i,
            Err(e) => {
                tracing::debug!(error = %e, "settings unavailable; continuing without them");
                return ServerSettings::default();
            }
        };
        let mut s = ServerSettings::default();
        for item in items {
            let Ok(dto) = serde_json::from_value::<SettingDto>(item) else {
                continue;
            };
            let key = dto.key.to_ascii_lowercase().replace('_', "-");
            // 0.34: `system_settings.time_zone`, `stream_settings.default_stream_profile`.
            // Older/flat shapes are accepted too.
            let obj = dto.value.as_object();
            let tz_of = |v: &Value| match v {
                Value::String(t) if !t.is_empty() => Some(t.clone()),
                Value::Object(o) => o
                    .get("time_zone")
                    .or_else(|| o.get("timezone"))
                    .and_then(|t| t.as_str())
                    .map(str::to_owned),
                _ => None,
            };
            if (key == "system-settings" || key.contains("time-zone") || key.contains("timezone"))
                && let Some(t) = tz_of(&dto.value)
            {
                s.timezone = Some(t);
            }
            if key == "stream-settings" {
                if let Some(id) = obj
                    .and_then(|o| o.get("default_stream_profile"))
                    .and_then(flex_i64)
                {
                    s.default_stream_profile = Some(id);
                }
            } else if key.contains("default-stream-profile") {
                s.default_stream_profile = flex_i64(&dto.value);
            }
        }
        s
    }

    async fn settings(&self) -> ServerSettings {
        if let Some(s) = self.settings.lock().ok().and_then(|g| g.clone()) {
            return s;
        }
        let s = self.load_settings().await;
        if let Ok(mut g) = self.settings.lock() {
            *g = Some(s.clone());
        }
        s
    }

    /// Connect sequence (PLAN §3.1): version, users/me, timezone/settings.
    pub async fn connect_info(&self) -> Result<AccountInfo> {
        let version = self.version().await?;
        if let Some(v) = version.as_deref() {
            match parse_version(v) {
                Some(found) if found < MIN_VERSION => {
                    return Err(AppError::unsupported_version(
                        v,
                        &format!("{}.{}.{}", MIN_VERSION.0, MIN_VERSION.1, MIN_VERSION.2),
                    ));
                }
                Some(_) => {}
                None => tracing::warn!("unrecognised server version string; continuing"),
            }
        }
        let me = self.me().await?;
        // Fresh settings on every connect.
        if let Ok(mut g) = self.settings.lock() {
            *g = None;
        }
        let settings = self.settings().await;
        self.probe_features(me.level()).await;
        Ok(AccountInfo {
            server_version: version,
            user_id: me.id,
            username: me.username.clone(),
            user_level: me.level(),
            timezone: settings.timezone,
            expires_at: None,
            max_connections: me.stream_limit(),
        })
    }

    // ---- catalog ----------------------------------------------------------------------------

    fn parse_items<T: DeserializeOwned>(items: Vec<Value>, what: &str) -> Vec<T> {
        let mut skipped = 0usize;
        let out: Vec<T> = items
            .into_iter()
            .filter_map(|v| match serde_json::from_value::<T>(v) {
                Ok(t) => Some(t),
                Err(_) => {
                    skipped += 1;
                    None
                }
            })
            .collect();
        if skipped > 0 {
            tracing::warn!(skipped, what, "skipped unreadable items");
        }
        out
    }

    /// One page of channels, optionally filtered.
    pub async fn channel_page(
        &self,
        page: Page,
        search: Option<&str>,
        group: Option<&str>,
    ) -> Result<ChannelPage> {
        let mut extra = Vec::new();
        if let Some(s) = search {
            extra.push(("search", s.to_owned()));
        }
        if let Some(g) = group {
            extra.push(("channel_group", g.to_owned()));
        }
        let (items, count, has_next) = self
            .page_values(
                "api/channels/channels/",
                &extra,
                page.page,
                page.page_size,
                "its channels",
            )
            .await?;
        let dtos = Self::parse_items::<ChannelDto>(items, "channels");
        // The grid tags programmes with the *EPG row's* tvg_id, which can differ from the
        // channel's own (Dispatcharr matches EPG by name, or the user picks one by hand). The
        // assignment wins; the channel's tvg_id is only the join key when it has none.
        let epg_map = if dtos.iter().any(|d| d.epg_id().is_some()) {
            self.epg_data_map().await
        } else {
            std::sync::Arc::default()
        };
        let channels: Vec<SourceChannel> = dtos
            .into_iter()
            .map(|d| {
                let epg = d.epg_id();
                let mut c = d.into_source();
                if let Some(t) = epg.and_then(|e| epg_map.get(&e)) {
                    c.tvg_id = Some(t.clone());
                }
                c
            })
            .collect();
        Ok(ChannelPage {
            channels,
            count,
            has_next,
        })
    }

    /// `epg data id -> tvg_id` (tolerant: an unreadable list just means no EPG joins). Cached
    /// for [`EPG_MAP_TTL`] so a multi-page channel sync fetches the list once.
    async fn epg_data_map(&self) -> EpgMap {
        let mut cache = self.epg_map.lock().await;
        if let Some((at, m)) = cache.as_ref()
            && at.elapsed() < EPG_MAP_TTL
        {
            return m.clone();
        }
        let m: HashMap<i64, String> = match self
            .fetch_all("api/epg/epgdata/", &[], "its guide channels")
            .await
        {
            Ok(items) => Self::parse_items::<EpgDataDto>(items, "epg data")
                .into_iter()
                .filter_map(|e| Some((e.id, e.tvg_id.filter(|t| !t.is_empty())?)))
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "epg data list unavailable");
                // Not cached: the next page retries.
                return std::sync::Arc::default();
            }
        };
        let m = std::sync::Arc::new(m);
        *cache = Some((std::time::Instant::now(), m.clone()));
        m
    }

    pub async fn group_list(&self) -> Result<Vec<SourceGroup>> {
        let items = self
            .fetch_all("api/channels/groups/", &[], "its channel groups")
            .await?;
        Ok(Self::parse_items::<GroupDto>(items, "groups")
            .into_iter()
            .map(GroupDto::into_source)
            .collect())
    }

    pub async fn channel_profiles(&self) -> Result<Vec<ChannelProfileDto>> {
        let items = self
            .fetch_all("api/channels/profiles/", &[], "its channel profiles")
            .await?;
        Ok(Self::parse_items(items, "channel profiles"))
    }

    /// Streams attached to a channel, in failover order.
    pub async fn channel_streams(&self, channel_id: i64) -> Result<Vec<StreamDto>> {
        let v = self
            .get_json(
                &format!("api/channels/channels/{channel_id}/streams/"),
                &[],
                "that channel's streams",
            )
            .await?;
        let items = match v {
            Value::Array(a) => a,
            Value::Object(mut o) => match o.remove("results") {
                Some(Value::Array(a)) => a,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        Ok(Self::parse_items(items, "streams"))
    }

    /// Provider account names by id. Only `id` and `name` are decoded; the reply's provider
    /// credentials are dropped at decode and never stored. Empty when the server refuses.
    async fn account_name_map(&self) -> AccountNames {
        let mut cache = self.account_names.lock().await;
        if let Some((at, m)) = cache.as_ref()
            && at.elapsed() < ACCOUNT_NAMES_TTL
        {
            return m.clone();
        }
        let m: HashMap<i64, String> = match self
            .fetch_all("api/m3u/accounts/", &[], "its provider accounts")
            .await
        {
            Ok(items) => Self::parse_items::<M3uAccountNameDto>(items, "provider accounts")
                .into_iter()
                .filter_map(|a| Some((a.id?, a.name?)))
                .collect(),
            Err(e) => {
                tracing::debug!(error = %e, "provider account names unavailable");
                HashMap::new()
            }
        };
        let m = std::sync::Arc::new(m);
        *cache = Some((std::time::Instant::now(), m.clone()));
        m
    }

    /// A channel's streams with their provider names filled in where the server shares them.
    pub async fn channel_stream_info(&self, channel_id: i64) -> Result<Vec<StreamInfo>> {
        let streams = self.channel_streams(channel_id).await?;
        let names = if streams.iter().any(|s| s.m3u_account.is_some()) {
            self.account_name_map().await
        } else {
            AccountNames::default()
        };
        Ok(streams
            .into_iter()
            .map(|s| {
                let st = s.stream_stats.unwrap_or_default();
                StreamInfo {
                    provider: s.m3u_account.and_then(|a| names.get(&a).cloned()),
                    id: s.id,
                    name: s.name,
                    resolution: st.resolution,
                    fps: st.source_fps.filter(|f| *f > 0.0),
                    video_codec: st.video_codec,
                    audio_codec: st.audio_codec,
                    audio_channels: st.audio_channels,
                    bitrate_kbps: st.ffmpeg_output_bitrate.filter(|b| *b > 0.0),
                    viewers: s.current_viewers,
                    stale: s.is_stale,
                }
            })
            .collect())
    }

    pub async fn stream_profiles(&self) -> Result<Vec<StreamProfileDto>> {
        let items = self
            .fetch_all("api/core/streamprofiles/", &[], "its stream profiles")
            .await?;
        Ok(Self::parse_items(items, "stream profiles"))
    }

    pub async fn load_profile_info(&self) -> Result<ProfileInfo> {
        let profiles = self.stream_profiles().await?;
        let redirect_ids: HashSet<i64> = profiles
            .iter()
            .filter(|p| p.is_redirect())
            .map(|p| p.id)
            .collect();
        let default_id = self.settings().await.default_stream_profile;
        let known_ids: HashSet<i64> = profiles.iter().map(|p| p.id).collect();
        Ok(ProfileInfo {
            redirect_ids,
            known_ids,
            default_id,
        })
    }

    // ---- guide ------------------------------------------------------------------------------

    pub async fn grid(
        &self,
        window: TimeWindow,
        channel_profile_id: Option<i64>,
    ) -> Result<Vec<Programme>> {
        let mut q = vec![
            ("start", fmt_time(window.start_utc)?),
            ("end", fmt_time(window.end_utc)?),
        ];
        if let Some(p) = channel_profile_id {
            q.push(("channel_profile_id", p.to_string()));
        }
        let v = self.get_json("api/epg/grid/", &q, "the guide").await?;
        let items = match v {
            Value::Object(mut o) => match o.remove("data") {
                Some(Value::Array(a)) => a,
                _ => Vec::new(),
            },
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        Ok(Self::parse_items::<ProgramDto>(items, "programmes")
            .into_iter()
            // Standard dummy blocks are 4 h placeholders titled with the channel name, generated
            // for channels with no EPG at all; the guide's own empty row says that better.
            // Custom dummy-EPG rows carry real titles and stay (they key on the channel uuid).
            .filter(|p| !p.id.starts_with("dummy-standard-"))
            .filter_map(ProgramDto::into_programme)
            .collect())
    }

    /// `POST /api/epg/current-programs/`. Tolerates an array, `{data|programs|results: [...]}` or a
    /// `uuid -> programme` map; each entry may nest the programme under `program`.
    pub async fn current_programs(
        &self,
        channel_uuids: &[String],
    ) -> Result<Vec<(String, Programme)>> {
        let body = json!({ "channel_uuids": channel_uuids });
        let v = self
            .call(Method::POST, "api/epg/current-programs/", &[], Some(&body))
            .await?
            .ok("current programmes")?
            .json()?;
        let mut pairs: Vec<(Option<String>, Value)> = Vec::new();
        fn collect(v: Value, pairs: &mut Vec<(Option<String>, Value)>) {
            match v {
                Value::Array(a) => {
                    for item in a {
                        let uuid = item.get("channel_uuid").and_then(flex_string);
                        pairs.push((uuid, item));
                    }
                }
                Value::Object(mut o) => {
                    for k in ["data", "programs", "results"] {
                        if let Some(inner) = o.remove(k) {
                            return collect(inner, pairs);
                        }
                    }
                    for (k, val) in o {
                        pairs.push((Some(k), val));
                    }
                }
                _ => {}
            }
        }
        collect(v, &mut pairs);
        let mut out = Vec::new();
        for (uuid, item) in pairs {
            let prog = item.get("program").cloned().unwrap_or(item);
            let Some(uuid) = uuid else { continue };
            if let Ok(dto) = serde_json::from_value::<ProgramDto>(prog)
                && let Some(p) = dto.into_programme()
            {
                out.push((uuid, p));
            }
        }
        Ok(out)
    }

    /// `GET /api/epg/programs/search/`. Returns one page; `has_next` tells whether more exist.
    pub async fn search_programs(
        &self,
        q: &ProgramSearch,
    ) -> Result<(Vec<crate::source::ProgramHit>, bool)> {
        let mut extra: Vec<(&str, String)> = Vec::new();
        if let Some(t) = &q.title {
            extra.push(("title", t.clone()));
        }
        if let Some(d) = &q.description {
            extra.push(("description", d.clone()));
        }
        if let Some(c) = &q.channel_id {
            extra.push(("channel_id", c.clone()));
        }
        if let Some(t) = q.start_after {
            extra.push(("start_after", fmt_time(t)?));
        }
        if let Some(t) = q.airing_at {
            extra.push(("airing_at", fmt_time(t)?));
        }
        let page = q.page.max(1);
        let size = if q.page_size == 0 {
            50
        } else {
            q.page_size.min(500)
        };
        let (items, _count, has_next) = self
            .page_values(
                "api/epg/programs/search/",
                &extra,
                page,
                size,
                "the programme search",
            )
            .await?;
        let hits = Self::parse_items::<ProgramSearchDto>(items, "programme search")
            .into_iter()
            .filter_map(ProgramSearchDto::into_hit)
            .collect();
        Ok((hits, has_next))
    }

    /// `POST /proxy/ts/next_stream/{channel_uuid}`: switch an *active* channel to its next provider
    /// stream. A control endpoint, not a stream endpoint. `Ok(false)` when the channel is not
    /// active (404): nothing to switch.
    pub async fn next_stream(&self, channel_uuid: &str) -> Result<bool> {
        let r = self
            .call(
                Method::POST,
                &format!("proxy/ts/next_stream/{channel_uuid}"),
                &[],
                Some(&json!({})),
            )
            .await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        r.ok("the next stream")?;
        Ok(true)
    }

    /// `POST /proxy/ts/change_stream/{channel_uuid}` with `{"stream_id": id}`.
    pub async fn change_stream(&self, channel_uuid: &str, stream_id: &str) -> Result<bool> {
        let r = self
            .call(
                Method::POST,
                &format!("proxy/ts/change_stream/{channel_uuid}"),
                &[],
                Some(&json!({ "stream_id": stream_id })),
            )
            .await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        r.ok("that stream")?;
        Ok(true)
    }

    // ---- images -----------------------------------------------------------------------------

    pub async fn logo(&self, logo_id: &str) -> Result<Logo> {
        let r = self
            .call(
                Method::GET,
                &format!("api/channels/logos/{logo_id}/cache"),
                &[],
                None,
            )
            .await?
            .ok("that logo")?;
        Ok(Logo {
            bytes: r.body,
            content_type: r.content_type,
        })
    }

    /// Absolute URL of a logo (what gets hashed into its `dlimg://` URL; carries no credentials).
    pub fn logo_url(&self, logo_id: &str) -> Result<String> {
        Ok(self
            .url(&format!("api/channels/logos/{logo_id}/cache"))?
            .to_string())
    }

    /// Authenticated fetch of an image URL. Refuses anything outside this server's own origin so
    /// the key can never be sent elsewhere.
    pub async fn fetch_image(&self, url: &str) -> Result<crate::images::RawImage> {
        let u = Url::parse(url)?;
        if u.origin() != self.base.origin() || !u.path().starts_with(self.base.path()) {
            // Another host (a poster CDN): fetched anonymously. The key never goes elsewhere.
            return crate::images::fetch_public(url, self.insecure_tls).await;
        }
        let rel = u.path().strip_prefix(self.base.path()).unwrap_or_default();
        // Keep the query: image URLs select the variant (`?kind=backdrop&index=0`).
        let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
        let q: Vec<(&str, String)> = pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        let r = self
            .call(Method::GET, rel, &q, None)
            .await?
            .ok("that image")?;
        Ok(crate::images::RawImage {
            bytes: r.body,
            content_type: r.content_type,
        })
    }

    // ---- catch-up ---------------------------------------------------------------------------

    /// `POST /api/catchup/sessions/`. `start` and duration come from the EPG programme row.
    pub async fn create_catchup(
        &self,
        channel_uuid: &str,
        p: &Programme,
    ) -> Result<CatchupSession> {
        let minutes = ((p.end_utc - p.start_utc + 59) / 60).clamp(1, 480);
        let body = json!({
            "channel_uuid": channel_uuid,
            "start": fmt_time(p.start_utc)?,
            "duration": minutes,
        });
        let r = self
            .call(Method::POST, "api/catchup/sessions/", &[], Some(&body))
            .await?;
        if matches!(r.status.as_u16(), 400 | 404 | 409 | 410 | 422) {
            // The session endpoint is the authority (PLAN §3.8.6): plain sentence, no raw body.
            return Err(AppError::stream(
                "This programme is not available to watch from the archive.",
            ));
        }
        let v = r.ok("catch-up")?.json()?;
        let id = ["session_id", "id", "uuid"]
            .iter()
            .find_map(|k| v.get(*k).and_then(flex_string))
            .ok_or_else(|| AppError::server("The server did not return a catch-up session."))?;
        let url = ["playback_url", "url", "stream_url", "manifest_url"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
            .ok_or_else(|| AppError::server("The server did not return a catch-up address."))?;
        let url = self.base.join(url)?.to_string();
        self.remember_secret(&id);
        self.remember_secret(&url);
        Ok(CatchupSession {
            id: SessionId(id),
            url,
        })
    }

    pub async fn catchup_position(&self, session: &SessionId, position_secs: u64) -> Result<()> {
        let path = format!("api/catchup/sessions/{}/position/", session.0);
        let r = self
            .call(
                Method::POST,
                &path,
                &[],
                Some(&json!({ "position_secs": position_secs })),
            )
            .await?;
        // 404 "No active playback" until the player's first GET: not an error.
        if r.status == StatusCode::NOT_FOUND {
            tracing::debug!("catch-up position ignored: playback has not started");
            return Ok(());
        }
        r.ok("that catch-up session")?;
        Ok(())
    }

    /// Idempotent: a 404 means the session is already gone.
    pub async fn delete_catchup(&self, session: &SessionId) -> Result<()> {
        let path = format!("api/catchup/sessions/{}/", session.0);
        let r = self.call(Method::DELETE, &path, &[], None).await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        r.ok("that catch-up session")?;
        Ok(())
    }

    /// Start a live HLS session: `GET /proxy/ts/stream/{uuid}?output_format=hls` with the API key and
    /// redirects off. The server answers `302 Location: /proxy/hls/<token>/index.m3u8`; the token is
    /// the only credential of the resulting URL. This IS a play (it opens the channel): call it only
    /// at an actual play. Returns `(playlist url, token)`.
    pub async fn mint_hls(&self, channel_uuid: &str) -> Result<(String, String)> {
        let url = self
            .base
            .join(&format!("proxy/ts/stream/{channel_uuid}?output_format=hls"))?;
        let auth = self.auth_header().await?;
        match crate::source::probe_redirect(url.as_str(), &[auth], self.insecure_tls).await? {
            crate::source::RedirectOutcome::Redirect(loc) => {
                let u = reqwest::Url::parse(loc.expose())?;
                let token = hls_token_of(u.path()).ok_or_else(|| {
                    AppError::server("The server did not start a live HLS session.")
                })?;
                Ok((u.to_string(), token))
            }
            crate::source::RedirectOutcome::Direct => Err(AppError::server(
                "The server did not start a live HLS session.",
            )),
        }
    }

    /// `DELETE /api/proxy/hls/sessions/{token}/`. Idempotent (404 means already gone).
    pub async fn delete_hls_session(&self, token: &str) -> Result<()> {
        let r = self
            .call(
                Method::DELETE,
                &format!("api/proxy/hls/sessions/{token}/"),
                &[],
                None,
            )
            .await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        r.ok("that HLS session")?;
        Ok(())
    }

    // ---- app data ---------------------------------------------------------------------------

    pub async fn app_data_get(&self, key: &str) -> Result<Option<Value>> {
        check_app_data_key(key)?;
        let r = self
            .call(
                Method::GET,
                &format!("api/accounts/users/me/app-data/{key}/"),
                &[],
                None,
            )
            .await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(r.ok("synced data")?.json()?))
    }

    pub async fn app_data_put(&self, key: &str, value: &Value) -> Result<()> {
        check_app_data_key(key)?;
        self.call(
            Method::PUT,
            &format!("api/accounts/users/me/app-data/{key}/"),
            &[],
            Some(value),
        )
        .await?
        .ok("synced data")?;
        Ok(())
    }

    /// `GET /proxy/ts/status/{channel_uuid}` (a status endpoint, safe to poll; takes the UUID, not
    /// the numeric id). The payload carries the provider `url`, so it is masked here; the
    /// returned value is safe to log or show. `None` when nothing is playing (404).
    pub async fn ts_status(&self, channel_uuid: &str) -> Result<Option<Value>> {
        let r = self
            .call(
                Method::GET,
                &format!("proxy/ts/status/{channel_uuid}"),
                &[],
                None,
            )
            .await?;
        if r.status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let mut v = r.ok("the session status")?.json()?;
        mask_urls(&mut v);
        Ok(Some(v))
    }

    // ---- play plans (no network except JWT login) --------------------------------------------

    /// Live TS plan: `/proxy/ts/stream/{uuid}`. Redirect-profile channels are marked for a
    /// play-time resolve and carry no headers into the plan the player sees.
    pub async fn live_plan(&self, ch: &ChannelKey) -> Result<PlayPlan> {
        let auth = self.auth_header().await?;
        build_live_plan(&self.base, ch, Some(auth), self.redaction_set())
    }
}

/// Absolute logo URL for a base URL (no client needed; carries no credentials).
pub fn logo_url(base: &Url, logo_id: &str) -> Result<String> {
    Ok(base
        .join(&format!("api/channels/logos/{logo_id}/cache"))?
        .to_string())
}

/// Dispatcharr requires app-data keys of 8-64 characters.
fn check_app_data_key(key: &str) -> Result<()> {
    if (8..=64).contains(&key.len()) {
        Ok(())
    } else {
        Err(AppError::local(
            "Synced data keys must be 8 to 64 characters long.",
        ))
    }
}

/// The active provider stream id in a `/proxy/ts/status` payload (number or string).
fn status_stream_id(v: &Value) -> Option<String> {
    match v.get("stream_id")? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Mask every `url` value (provider URLs) in a status payload, recursively.
fn mask_urls(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, val) in o.iter_mut() {
                if k.eq_ignore_ascii_case("url") || k.ends_with("_url") {
                    *val = Value::String(crate::redact::MASK.to_owned());
                } else {
                    mask_urls(val);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(mask_urls),
        Value::String(s) => *s = crate::redact::redact(s),
        _ => {}
    }
}

/// The session token of a `/proxy/hls/<token>/…` path.
fn hls_token_of(path: &str) -> Option<String> {
    let rest = path.split("/proxy/hls/").nth(1)?;
    let token = rest.split('/').next()?;
    (!token.is_empty()).then(|| token.to_owned())
}

/// Pure plan construction. Never performs I/O.
pub fn build_live_plan(
    base: &Url,
    ch: &ChannelKey,
    auth: Option<(String, SecretString)>,
    mut redact: RedactionSet,
) -> Result<PlayPlan> {
    let uuid = ch
        .uuid
        .as_deref()
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            AppError::stream("This channel has no stream address yet; refresh the channel list.")
        })?;
    let url = base.join(&format!("proxy/ts/stream/{uuid}"))?;
    if let Some((_, v)) = &auth {
        redact.add(v.expose());
    }
    Ok(PlayPlan {
        url: url.to_string(),
        headers: auth.into_iter().collect(),
        surface: Surface::Ts,
        profile: PlayProfile::Live,
        session: None,
        redact,
        needs_redirect_resolve: ch.stream_mode != StreamMode::Proxy,
        unknown_profile: ch.stream_mode == StreamMode::Unknown,
    })
}

#[async_trait]
impl crate::sessions::SessionStopper for DispatcharrClient {
    async fn stop(&self, _source_id: i64, kind: &str, session_id: &str) -> Result<()> {
        match kind {
            crate::sessions::KIND_CATCHUP => {
                self.delete_catchup(&SessionId(session_id.to_owned())).await
            }
            crate::sessions::KIND_HLS => self.delete_hls_session(session_id).await,
            _ => Ok(()),
        }
    }
}

#[async_trait]
impl Source for DispatcharrClient {
    fn id(&self) -> SourceId {
        self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Dispatcharr
    }

    /// VOD and DVR are advertised only when this account can use them (probed at connect): a
    /// Streamer cannot record, and a server with no VOD content shows no VOD section.
    fn caps(&self) -> Caps {
        use std::sync::atomic::Ordering::Relaxed;
        let mut c = Caps::LIVE
            | Caps::EPG
            | Caps::CATCHUP
            | Caps::FAVOURITES
            | Caps::FAILOVER
            | Caps::PROFILES;
        if self.has_vod.load(Relaxed) {
            c |= Caps::VOD;
        }
        if self.has_dvr.load(Relaxed) {
            c |= Caps::DVR;
        }
        if self.can_switch.load(Relaxed) {
            c |= Caps::STREAM_SWITCH;
        }
        c
    }

    async fn connect(&self) -> Result<AccountInfo> {
        self.connect_info().await
    }

    async fn channels(&self, page: Page) -> Result<ChannelPage> {
        self.channel_page(page, None, None).await
    }

    async fn guide(&self, window: TimeWindow) -> Result<Vec<Programme>> {
        self.grid(window, None).await
    }

    async fn play(&self, ch: ChannelKey) -> Result<PlayPlan> {
        let plan = self.live_plan(&ch).await?;
        // Redirect and Unknown profiles: resolve in Rust with redirects disabled; mpv gets the
        // Location and no key. A 2xx (profile was really Proxy) hands back the original plan.
        plan.resolve_at_play(self.insecure_tls).await
    }

    async fn catchup(&self, ch: ChannelKey, p: &Programme) -> Result<PlayPlan> {
        let uuid = ch
            .uuid
            .as_deref()
            .ok_or_else(|| AppError::stream("This channel has no archive address."))?;
        let s = self.create_catchup(uuid, p).await?;
        let auth = self.auth_header().await?;
        let mut redact = self.redaction_set();
        redact.add(s.url.clone());
        redact.add(s.id.0.clone());
        redact.add(auth.1.expose());
        Ok(PlayPlan {
            url: s.url,
            headers: vec![auth],
            surface: Surface::Direct,
            profile: PlayProfile::Seekable,
            session: Some(s.id),
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    async fn stop_session(&self, s: SessionId) -> Result<()> {
        self.delete_catchup(&s).await
    }

    async fn catchup_position(&self, s: &SessionId, position_secs: u64) -> Result<()> {
        DispatcharrClient::catchup_position(self, s, position_secs).await
    }

    async fn vod_category_names(&self, kind: VodKind) -> Result<Vec<VodCategoryInfo>> {
        self.cached_vod_categories(kind).await
    }

    async fn vod_categories(&self, kind: VodKind) -> Result<Vec<VodCategoryInfo>> {
        use futures_util::{StreamExt, stream};

        let mut cats = self.cached_vod_categories(kind).await?;
        let names: Vec<_> = cats.iter().take(80).map(|c| c.name.clone()).collect();
        let counts = stream::iter(names)
            .map(|name| async move { self.vod_category_count(kind, &name).await.ok() })
            .buffered(MAX_CONCURRENT)
            .collect::<Vec<_>>()
            .await;
        for (category, count) in cats.iter_mut().zip(counts) {
            category.count = count;
        }
        Ok(cats)
    }

    async fn vod_list(&self, kind: VodKind, q: &VodListQuery) -> Result<VodListPage> {
        self.vod_page(kind, q).await
    }

    async fn vod_detail(&self, kind: VodKind, ext_id: &str) -> Result<VodItem> {
        self.vod_item_detail(kind, ext_id).await
    }

    async fn series_episodes(&self, series_ext_id: &str) -> Result<Vec<VodItem>> {
        self.series_episode_list(series_ext_id).await
    }

    async fn vod_enrich(&self, kind: VodKind, ext_id: &str) -> Result<Option<VodItem>> {
        self.provider_info(kind, ext_id).await
    }

    async fn play_vod(&self, item: &VodItem) -> Result<PlayPlan> {
        self.vod_plan(item).await
    }

    async fn recordings(&self) -> Result<Vec<RecordingInfo>> {
        self.recording_list().await
    }

    async fn schedule_recording(&self, r: &RecordRequest) -> Result<()> {
        self.create_recording(r).await
    }

    async fn schedule_series_rule(&self, r: &RecordRequest) -> Result<()> {
        self.create_series_rule(r).await
    }

    async fn stop_recording(&self, ext_id: &str) -> Result<()> {
        self.stop_recording_by_id(ext_id).await
    }

    async fn delete_recording(&self, ext_id: &str) -> Result<()> {
        self.delete_recording_by_id(ext_id).await
    }

    async fn extend_recording(&self, ext_id: &str, minutes: i64) -> Result<()> {
        self.extend_recording_by_id(ext_id, minutes).await
    }

    async fn recording_plan(&self, rec: &RecordingInfo) -> Result<PlayPlan> {
        self.recording_play_plan(rec).await
    }

    async fn recording_rules(&self) -> Result<Vec<RuleInfo>> {
        self.rule_list().await
    }

    async fn delete_rule(&self, rule: &RuleInfo) -> Result<()> {
        self.delete_rule_by(rule).await
    }

    async fn next_stream(&self, ch: &ChannelKey) -> Result<bool> {
        match ch.uuid.as_deref() {
            Some(u) => DispatcharrClient::next_stream(self, u).await,
            None => Ok(false),
        }
    }

    async fn change_stream(&self, ch: &ChannelKey, stream_id: &str) -> Result<bool> {
        match ch.uuid.as_deref() {
            Some(u) => DispatcharrClient::change_stream(self, u, stream_id).await,
            None => Ok(false),
        }
    }

    async fn active_stream_id(&self, ch: &ChannelKey) -> Result<Option<String>> {
        let Some(u) = ch.uuid.as_deref() else {
            return Ok(None);
        };
        Ok(self.ts_status(u).await?.as_ref().and_then(status_stream_id))
    }

    async fn channel_stream_info(&self, ch_ext_id: &str) -> Result<Vec<StreamInfo>> {
        let id: i64 = ch_ext_id
            .parse()
            .map_err(|_| AppError::local("Not a channel id."))?;
        DispatcharrClient::channel_stream_info(self, id).await
    }

    async fn fetch_image(&self, url: &str) -> Result<crate::images::RawImage> {
        DispatcharrClient::fetch_image(self, url).await
    }

    async fn groups(&self) -> Result<Vec<SourceGroup>> {
        self.group_list().await
    }

    async fn profile_info(&self) -> Result<ProfileInfo> {
        self.load_profile_info().await
    }

    async fn current_programmes(
        &self,
        channel_uuids: &[String],
    ) -> Result<Vec<(String, Programme)>> {
        self.current_programs(channel_uuids).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::StreamMode;

    #[test]
    fn versions() {
        assert_eq!(parse_version("0.32.0"), Some((0, 32, 0)));
        assert_eq!(parse_version("v0.33.1-dev"), Some((0, 33, 1)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("garbage"), None);
        assert!(parse_version("0.31.9") < Some(MIN_VERSION));
    }

    #[test]
    fn base_urls() {
        assert_eq!(
            normalise_base_url("dispatcharr.test:9191")
                .unwrap()
                .as_str(),
            "http://dispatcharr.test:9191/"
        );
        assert_eq!(
            normalise_base_url("https://d.test/prefix//?x=1")
                .unwrap()
                .as_str(),
            "https://d.test/prefix/"
        );
        assert!(normalise_base_url("ftp://d.test").is_err());
    }

    #[test]
    fn live_plan_is_pure_and_redirect_marked() {
        let base = normalise_base_url("https://d.test/sub").unwrap();
        let key = ChannelKey {
            ext_id: "1".into(),
            uuid: Some("u-1".into()),
            stream_mode: StreamMode::Redirect,
            ..Default::default()
        };
        let plan = build_live_plan(
            &base,
            &key,
            Some(("X-API-Key".into(), SecretString::new("sk_secret_value"))),
            RedactionSet::new(),
        )
        .unwrap();
        assert_eq!(plan.url, "https://d.test/sub/proxy/ts/stream/u-1");
        assert!(plan.needs_redirect_resolve);
        let dbg = format!("{plan:?}");
        assert!(!dbg.contains("sk_secret_value"), "{dbg}");
    }

    #[test]
    fn plan_requires_uuid() {
        let base = normalise_base_url("https://d.test").unwrap();
        let key = ChannelKey {
            ext_id: "1".into(),
            uuid: None,
            stream_mode: StreamMode::Proxy,
            ..Default::default()
        };
        assert!(build_live_plan(&base, &key, None, RedactionSet::new()).is_err());
    }

    #[test]
    fn channel_dto_is_tolerant() {
        let v = json!({
            "id": "7", "uuid": "u", "channel_number": "5.5", "name": "Channel One",
            "channel_group_id": 3, "streams": [{"id": 9, "url": "http://provider.test/live/a/b/1"}, 4, "5"],
            "future_field": {"x": 1}, "is_adult": 0, "user_level": "1"
        });
        let c: ChannelDto = serde_json::from_value(v).unwrap();
        let s = c.into_source();
        assert_eq!(s.ext_id, "7");
        assert_eq!(s.number, Some(5.5));
        assert_eq!(s.group_ext_id.as_deref(), Some("3"));
        assert_eq!(s.streams, ["9", "4", "5"]);
        assert_eq!(s.user_level, 1);
    }

    #[test]
    fn program_dto_skips_bad_times() {
        let ok: ProgramDto = serde_json::from_value(json!({
            "id": "dummy-1", "start_time": "2025-01-01T10:00:00Z", "end_time": "2025-01-01T11:00:00+00:00",
            "title": "Show", "tvg_id": "one.test", "is_new": true
        }))
        .unwrap();
        let p = ok.into_programme().unwrap();
        assert_eq!(p.end_utc - p.start_utc, 3600);
        assert_eq!(p.flags, flags_new());
        let bad: ProgramDto = serde_json::from_value(json!({"start_time": "nope"})).unwrap();
        assert!(bad.into_programme().is_none());
    }

    fn flags_new() -> i64 {
        crate::source::flags::NEW
    }
}
