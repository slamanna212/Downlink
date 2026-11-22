//! Xtream Codes client and `Source` (PLAN §6.5, §5.3). Credentials live only in the keychain and
//! in this client's memory; every URL it builds carries them, so URLs are never logged and every
//! error is URL-free (`AppError` redacts again as a second line of defence).
//!
//! Quirks absorbed: numbers as strings, `auth: 0` with HTTP 200, empty results as `[]`/`{}`/`null`,
//! `allowed_output_formats` picks `.ts` vs `.m3u8`, `server_info.timezone` drives `xc` catch-up,
//! base64 EPG text. Play URLs are built at play time, never cached (load-balanced panels).

mod dto;

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{StatusCode, Url};
use serde_json::Value;
use tokio::sync::Semaphore;

pub use dto::*;

use super::dispatcharr::dto::{flex_i64, flex_string};
use super::{
    AccountInfo, Caps, ChannelKey, ChannelPage, Page, PlayPlan, PlayProfile, Programme, SessionId,
    Source, SourceChannel, SourceGroup, SourceId, SourceKind, Surface, TimeWindow, UserLevel,
    VodCategoryInfo, VodItem, VodKind, VodListPage, VodListQuery,
};
use crate::auth::SecretString;
use crate::catchup::template::{CatchupTemplate, ExpandCtx, Mode, TimeSemantics, XcParts, expand};
use crate::error::{AppError, Result};
use crate::m3u::load::{Fetched, Fetcher, HttpFetcher};
use crate::redact::RedactionSet;
use crate::source::vod::clean_tmdb;

const MAX_CONCURRENT: usize = 2;

#[derive(Debug, Clone, Default)]
struct AccountState {
    server_tz: Option<String>,
    formats: Vec<String>,
    connected: bool,
}

pub struct XtreamClient {
    id: SourceId,
    http: reqwest::Client,
    tls_pin: Option<crate::tls::Pin>,
    base: Url,
    username: String,
    password: SecretString,
    sem: Semaphore,
    state: Mutex<AccountState>,
    vault: crate::vault::Vault,
    has_vod: std::sync::atomic::AtomicBool,
    vod_cache: tokio::sync::Mutex<VodCache>,
    /// Persists the VOD lists across restarts; `None` for throwaway clients (connection tests).
    store: Option<crate::store::Store>,
}

/// Xtream has no server-side paging or search: the movie and series lists are fetched whole and
/// served from memory, backed by `meta_cache` so a restart does not download them again.
#[derive(Default)]
struct VodCache {
    movies: Slot<VodStreamDto>,
    series: Slot<SeriesDto>,
}

/// `(fetched_at UTC seconds, rows)`.
type Slot<T> = Option<(i64, std::sync::Arc<Vec<T>>)>;

/// How long a fetched movie or series list is served before it is fetched again (only when VOD is
/// being browsed: nothing refetches it in the background). VOD libraries change slowly.
const VOD_CACHE_TTL_SECS: i64 = 6 * 3600;

/// A cached VOD row whose artwork URLs are templated through the vault before it is written to
/// SQLite (rule 7: no credentials in the database).
trait VodRow: serde::Serialize + serde::de::DeserializeOwned + Send + Sync {
    fn template(&mut self, vault: &crate::vault::Vault);
}

impl VodRow for VodStreamDto {
    fn template(&mut self, vault: &crate::vault::Vault) {
        self.icon = self.icon.as_deref().map(|u| vault.template_url(u));
    }
}

impl VodRow for SeriesDto {
    fn template(&mut self, vault: &crate::vault::Vault) {
        self.cover = self.cover.as_deref().map(|u| vault.template_url(u));
        self.backdrop_path = self.backdrop_path.as_deref().map(|u| vault.template_url(u));
    }
}

/// `meta_cache` key prefix for a source's VOD lists (trailing `:` so source 1 never matches 12).
pub fn vod_cache_prefix(source_id: i64) -> String {
    format!("xtvod:{source_id}:")
}

impl std::fmt::Debug for XtreamClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("XtreamClient(***)")
    }
}

/// `http://host:port` or `host:port`; any path, query or credentials are dropped.
pub fn normalise_base(input: &str) -> Result<Url> {
    let t = input.trim();
    let with = if t.contains("://") {
        t.to_owned()
    } else {
        format!("http://{t}")
    };
    let mut u = Url::parse(&with)?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err(AppError::local(
            "The server address must start with http:// or https://.",
        ));
    }
    // Keep a path prefix (some panels live under /panel) but drop the script and the query.
    let path = u.path().to_owned();
    let trimmed = path
        .trim_end_matches('/')
        .trim_end_matches("player_api.php")
        .trim_end_matches("get.php")
        .trim_end_matches('/')
        .to_owned();
    u.set_path(&format!("{trimmed}/"));
    u.set_query(None);
    u.set_fragment(None);
    let _ = u.set_username("");
    let _ = u.set_password(None);
    Ok(u)
}

/// `Movie Title (2021)` -> 2021.
/// Panels send `""`, `"0"` or `0` for "no rating"; none of those is a rating.
fn real_rating(r: Option<&str>) -> Option<String> {
    let r = r?.trim();
    match r.parse::<f64>() {
        Ok(v) if v <= 0.0 => None,
        _ if r.is_empty() => None,
        _ => Some(r.to_owned()),
    }
}

/// The 0-10 `rating`, or `rating_5based` doubled when the panel only fills that one.
fn pick_rating(ten: Option<&str>, five: Option<&str>) -> Option<String> {
    real_rating(ten).or_else(|| {
        let v: f64 = real_rating(five)?.parse().ok()?;
        (v <= 5.0).then(|| format!("{:.1}", v * 2.0))
    })
}

fn year_in_title(name: &str) -> Option<i64> {
    let t = name.trim_end();
    let inner = t.strip_suffix(')')?.rsplit_once('(')?.1;
    (inner.len() == 4)
        .then(|| inner.parse().ok())
        .flatten()
        .filter(|y| (1850..2200).contains(y))
}

fn some_str(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(flex_string)
}

impl XtreamClient {
    pub fn new(
        id: SourceId,
        base_url: &str,
        username: &str,
        password: SecretString,
        tls_pin: Option<crate::tls::Pin>,
    ) -> Result<Self> {
        let base = normalise_base(base_url)?;
        Ok(Self {
            id,
            http: crate::http::authenticated(crate::tls::apply(
                reqwest::Client::builder(),
                &base,
                tls_pin.as_ref(),
            )?)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(20))
            .user_agent(crate::user_agent())
            .build()?,
            tls_pin,
            base,
            username: username.to_owned(),
            password,
            sem: Semaphore::new(MAX_CONCURRENT),
            state: Mutex::new(AccountState::default()),
            vault: crate::vault::Vault::new(),
            has_vod: std::sync::atomic::AtomicBool::new(false),
            vod_cache: tokio::sync::Mutex::new(VodCache::default()),
            store: None,
        })
    }

    /// Keep the VOD lists in `store` as well as in memory.
    #[must_use]
    pub fn with_store(mut self, store: crate::store::Store) -> Self {
        self.store = Some(store);
        self
    }

    pub(crate) fn attach_credentials(
        &self,
        secrets: std::sync::Arc<dyn crate::auth::SecretStore>,
        uuid: &str,
    ) -> Result<()> {
        self.vault.attach(secrets, uuid)
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Literal secrets for redaction (username and password).
    pub fn redaction_set(&self) -> RedactionSet {
        let mut s = RedactionSet::new();
        s.add(self.username.clone());
        s.add(self.password.expose());
        s
    }

    fn st(&self) -> AccountState {
        self.state.lock().map(|g| g.clone()).unwrap_or_default()
    }

    // ---- URL builders (pure; the results carry credentials) ---------------------------------

    fn script_url(
        &self,
        script: &str,
        action: Option<&str>,
        extra: &[(&str, String)],
    ) -> Result<Url> {
        let mut u = self.base.join(script)?;
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("username", &self.username);
            q.append_pair("password", self.password.expose());
            if let Some(a) = action {
                q.append_pair("action", a);
            }
            for (k, v) in extra {
                q.append_pair(k, v);
            }
        }
        Ok(u)
    }

    fn path_url(&self, kind: &str, id: &str, ext: &str) -> Result<String> {
        let mut u = self.base.clone();
        u.path_segments_mut()
            .map_err(|_| AppError::local("The server address cannot be used."))?
            .pop_if_empty()
            .extend([
                kind,
                &self.username,
                self.password.expose(),
                &format!("{id}.{ext}"),
            ]);
        Ok(u.to_string())
    }

    /// Output container: `.ts` when the account allows it (lowest latency), else `.m3u8`.
    pub fn live_ext(&self) -> &'static str {
        let f = self.st().formats;
        if f.is_empty() || f.iter().any(|x| x.eq_ignore_ascii_case("ts")) {
            "ts"
        } else {
            "m3u8"
        }
    }

    /// `/live/<user>/<pass>/<id>.<ext>`.
    pub fn live_url(&self, stream_id: &str) -> Result<String> {
        self.path_url("live", stream_id, self.live_ext())
    }

    pub fn movie_url(&self, stream_id: &str, ext: &str) -> Result<String> {
        self.path_url("movie", stream_id, ext)
    }

    pub fn series_url(&self, episode_id: &str, ext: &str) -> Result<String> {
        self.path_url("series", episode_id, ext)
    }

    /// `xmltv.php?username=&password=`.
    pub fn xmltv_url(&self) -> Result<String> {
        Ok(self.script_url("xmltv.php", None, &[])?.to_string())
    }

    /// `streaming/timeshift.php?…` (the query form some panels use instead of the path form).
    pub fn timeshift_php_url(
        &self,
        stream_id: &str,
        start_wallclock: &str,
        duration_minutes: i64,
    ) -> Result<String> {
        Ok(self
            .script_url(
                "streaming/timeshift.php",
                None,
                &[
                    ("stream", stream_id.to_owned()),
                    ("start", start_wallclock.to_owned()),
                    ("duration", duration_minutes.to_string()),
                ],
            )?
            .to_string())
    }

    /// `get.php?username=&password=&type=m3u_plus`: the panel's own playlist URL.
    pub fn playlist_url(&self) -> Result<String> {
        Ok(self
            .script_url(
                "get.php",
                None,
                &[("type", "m3u_plus".to_owned()), ("output", "ts".to_owned())],
            )?
            .to_string())
    }

    // ---- transport --------------------------------------------------------------------------

    async fn get_json(
        &self,
        action: Option<&str>,
        extra: &[(&str, String)],
        what: &str,
    ) -> Result<Value> {
        let url = self.script_url("player_api.php", action, extra)?;
        let _permit = self
            .sem
            .acquire()
            .await
            .map_err(|_| AppError::local("The client is shutting down."))?;
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(match status {
                // Dispatcharr's XC surface answers only XC-enabled users: a permissions problem.
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AppError::auth(
                    "The server refused this account's Xtream access. Check that Xtream is enabled for this user.",
                ),
                s => AppError::from_status(s.as_u16(), None, what),
            });
        }
        let bytes = resp.bytes().await?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn list<T: serde::de::DeserializeOwned>(
        &self,
        action: &str,
        extra: &[(&str, String)],
        what: &str,
    ) -> Result<Vec<T>> {
        let v = self.get_json(Some(action), extra, what).await?;
        let items = match v {
            Value::Array(a) => a,
            // Some panels wrap, some answer `{}` for "nothing".
            Value::Object(mut o) => match o.remove("data").or_else(|| o.remove("results")) {
                Some(Value::Array(a)) => a,
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        Ok(items
            .into_iter()
            .filter_map(|i| serde_json::from_value(i).ok())
            .collect())
    }

    // ---- API --------------------------------------------------------------------------------

    /// `player_api.php` with no action: account and server info. `auth: 0` is an auth error even
    /// though the HTTP status is 200.
    pub async fn account(&self) -> Result<AccountInfo> {
        let v = self.get_json(None, &[], "its account").await?;
        let ui = v.get("user_info");
        let authed = ui.and_then(|u| u.get("auth")).and_then(flex_i64);
        if ui.is_none() || authed == Some(0) {
            return Err(AppError::auth("The username or password was not accepted."));
        }
        let ui = ui.cloned().unwrap_or(Value::Null);
        let si = v.get("server_info").cloned().unwrap_or(Value::Null);
        let formats: Vec<String> = ui
            .get("allowed_output_formats")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_ascii_lowercase))
                    .collect()
            })
            .unwrap_or_default();
        let max = ui
            .get("max_connections")
            .and_then(flex_i64)
            .and_then(|n| u32::try_from(n).ok());
        let exp = ui.get("exp_date").and_then(flex_i64).filter(|e| *e > 0);
        let tz = some_str(&si, "timezone");
        if let Ok(mut g) = self.state.lock() {
            *g = AccountState {
                server_tz: tz.clone(),
                formats,
                connected: true,
            };
        }
        if ui
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("expired"))
        {
            tracing::warn!("account reports status expired");
        }
        Ok(AccountInfo {
            server_version: None,
            user_id: None,
            username: some_str(&ui, "username").or_else(|| Some(self.username.clone())),
            user_level: Some(UserLevel::Standard),
            timezone: tz,
            expires_at: exp,
            max_connections: max,
        })
    }

    async fn ensure_account(&self) -> Result<()> {
        if !self.st().connected {
            self.account().await?;
        }
        Ok(())
    }

    pub async fn live_categories(&self) -> Result<Vec<CategoryDto>> {
        self.list("get_live_categories", &[], "its live categories")
            .await
    }

    pub async fn live_streams(&self, category: Option<&str>) -> Result<Vec<LiveStreamDto>> {
        let extra: Vec<(&str, String)> = category
            .map(|c| ("category_id", c.to_owned()))
            .into_iter()
            .collect();
        self.list("get_live_streams", &extra, "its live channels")
            .await
    }

    pub async fn vod_category_list(&self) -> Result<Vec<CategoryDto>> {
        self.list("get_vod_categories", &[], "its movie categories")
            .await
    }

    pub async fn vod_streams(&self, category: Option<&str>) -> Result<Vec<VodStreamDto>> {
        let extra: Vec<(&str, String)> = category
            .map(|c| ("category_id", c.to_owned()))
            .into_iter()
            .collect();
        self.list("get_vod_streams", &extra, "its movies").await
    }

    pub async fn series_categories(&self) -> Result<Vec<CategoryDto>> {
        self.list("get_series_categories", &[], "its series categories")
            .await
    }

    pub async fn series(&self, category: Option<&str>) -> Result<Vec<SeriesDto>> {
        let extra: Vec<(&str, String)> = category
            .map(|c| ("category_id", c.to_owned()))
            .into_iter()
            .collect();
        self.list("get_series", &extra, "its series").await
    }

    pub async fn series_info(&self, series_id: &str) -> Result<SeriesInfoDto> {
        let v = self
            .get_json(
                Some("get_series_info"),
                &[("series_id", series_id.to_owned())],
                "that series",
            )
            .await?;
        Ok(SeriesInfoDto::from_value(&v))
    }

    /// `get_short_epg`: the next few programmes of one channel.
    pub async fn short_epg(&self, stream_id: &str, limit: u32) -> Result<Vec<Programme>> {
        self.ensure_account().await.ok();
        let tz = self.st().server_tz;
        let v = self
            .get_json(
                Some("get_short_epg"),
                &[
                    ("stream_id", stream_id.to_owned()),
                    ("limit", limit.to_string()),
                ],
                "the guide",
            )
            .await?;
        Ok(dto::epg_listings(&v, tz.as_deref()))
    }

    /// `get_simple_data_table`: the channel's full EPG.
    pub async fn full_epg(&self, stream_id: &str) -> Result<Vec<Programme>> {
        self.ensure_account().await.ok();
        let tz = self.st().server_tz;
        let v = self
            .get_json(
                Some("get_simple_data_table"),
                &[("stream_id", stream_id.to_owned())],
                "the guide",
            )
            .await?;
        Ok(dto::epg_listings(&v, tz.as_deref()))
    }

    fn fetcher(&self) -> Result<HttpFetcher> {
        HttpFetcher::new(self.tls_pin.as_ref())
    }

    async fn movies_cached(&self) -> Result<std::sync::Arc<Vec<VodStreamDto>>> {
        self.list_cached(VodKind::Movie, |c| &mut c.movies, || self.vod_streams(None))
            .await
    }

    async fn series_cached(&self) -> Result<std::sync::Arc<Vec<SeriesDto>>> {
        self.list_cached(VodKind::Series, |c| &mut c.series, || self.series(None))
            .await
    }

    /// Memory, then `meta_cache`, then the server. A failed fetch serves the newest stale copy.
    async fn list_cached<T, F, Fut>(
        &self,
        kind: VodKind,
        slot: impl Fn(&mut VodCache) -> &mut Slot<T>,
        fetch: F,
    ) -> Result<std::sync::Arc<Vec<T>>>
    where
        T: VodRow + Clone,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<T>>>,
    {
        let now = crate::sync::now_utc();
        let mut g = self.vod_cache.lock().await;
        if let Some((at, v)) = slot(&mut g)
            && now - *at < VOD_CACHE_TTL_SECS
        {
            return Ok(v.clone());
        }
        let key = format!(
            "{}{}",
            vod_cache_prefix(self.id.0),
            if kind == VodKind::Movie {
                "movie"
            } else {
                "series"
            }
        );
        if slot(&mut g).is_none()
            && let Some(store) = &self.store
            && let Ok(Some((body, at))) = store.meta_get(&key).await
            && let Ok(rows) = serde_json::from_str::<Vec<T>>(&body)
        {
            // Too old: kept only as the fallback for a failed fetch below.
            let v = std::sync::Arc::new(rows);
            *slot(&mut g) = Some((at, v.clone()));
            if now - at < VOD_CACHE_TTL_SECS {
                return Ok(v);
            }
        }
        match fetch().await {
            Ok(mut rows) => {
                rows.iter_mut().for_each(|r| r.template(&self.vault));
                self.vault.persist().await?;
                if let Some(store) = &self.store {
                    match serde_json::to_string(&rows) {
                        Ok(body) => {
                            if let Err(e) = store.meta_put(&key, &body, now).await {
                                tracing::warn!(error = %e, "saving the VOD list failed");
                            }
                        }
                        Err(e) => tracing::warn!(error = %e, "saving the VOD list failed"),
                    }
                }
                let v = std::sync::Arc::new(rows);
                *slot(&mut g) = Some((now, v.clone()));
                Ok(v)
            }
            Err(e) => match slot(&mut g) {
                Some((_, v)) => {
                    tracing::warn!(error = %e, "serving the cached VOD list");
                    Ok(v.clone())
                }
                None => Err(e),
            },
        }
    }

    async fn category_counts(
        &self,
        kind: VodKind,
    ) -> Result<std::collections::HashMap<String, i64>> {
        let mut m = std::collections::HashMap::new();
        if kind == VodKind::Movie {
            for x in self.movies_cached().await?.iter() {
                *m.entry(x.category_id.clone().unwrap_or_default())
                    .or_insert(0) += 1;
            }
        } else {
            for x in self.series_cached().await?.iter() {
                *m.entry(x.category_id.clone().unwrap_or_default())
                    .or_insert(0) += 1;
            }
        }
        Ok(m)
    }

    fn movie_item(&self, m: &VodStreamDto) -> VodItem {
        let year = m.year.or_else(|| year_in_title(&m.name));
        VodItem {
            kind: Some(VodKind::Movie),
            ext_id: m.stream_id.clone(),
            title: m.name.clone(),
            year,
            rating: pick_rating(m.rating.as_deref(), m.rating_5based.as_deref()),
            poster: m
                .icon
                .as_deref()
                .filter(|i| !i.is_empty())
                .map(|i| self.vault.template_url(i)),
            category_ext: m.category_id.clone(),
            container: m.container_extension.clone(),
            tmdb_id: clean_tmdb(m.tmdb.clone()).or_else(|| clean_tmdb(m.tmdb_id.clone())),
            ..Default::default()
        }
    }

    fn series_item(&self, s: &SeriesDto) -> VodItem {
        VodItem {
            kind: Some(VodKind::Series),
            ext_id: s.series_id.clone(),
            title: s.name.clone(),
            year: s
                .release_date
                .as_deref()
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse().ok()),
            rating: pick_rating(s.rating.as_deref(), s.rating_5based.as_deref()),
            genre: s.genre.clone().filter(|g| !g.is_empty()),
            plot: s.plot.clone().filter(|p| !p.is_empty()),
            poster: s
                .cover
                .as_deref()
                .filter(|i| !i.is_empty())
                .map(|i| self.vault.template_url(i)),
            backdrop: s
                .backdrop_path
                .as_deref()
                .filter(|i| !i.is_empty())
                .map(|i| self.vault.template_url(i)),
            category_ext: s.category_id.clone(),
            tmdb_id: clean_tmdb(s.tmdb.clone()).or_else(|| clean_tmdb(s.tmdb_id.clone())),
            ..Default::default()
        }
    }

    fn live_to_channel(&self, s: &LiveStreamDto, tz: Option<&str>) -> SourceChannel {
        let template = (s.tv_archive && s.tv_archive_days > 0).then(|| {
            CatchupTemplate {
                mode: Mode::Xc,
                source: None,
                time: TimeSemantics::LocalWallclock {
                    tz: tz.unwrap_or("UTC").to_owned(),
                },
                correction_secs: 0,
                days: s.tv_archive_days,
            }
            .to_json()
        });
        SourceChannel {
            ext_id: s.stream_id.clone(),
            number: s.num,
            name: s.name.clone(),
            group_ext_id: s.category_id.clone(),
            tvg_id: s.epg_channel_id.clone(),
            logo_key: s.icon.as_deref().map(|l| self.vault.template_url(l)),
            is_adult: s.is_adult,
            is_radio: s.stream_type.as_deref() == Some("radio_streams"),
            is_catchup: s.tv_archive,
            catchup_days: if s.tv_archive { s.tv_archive_days } else { 0 },
            catchup_template: template,
            play_url: s
                .direct_source
                .as_deref()
                .map(|u| self.vault.template_url(u)),
            ..Default::default()
        }
    }
}

#[async_trait]
impl Source for XtreamClient {
    fn id(&self) -> SourceId {
        self.id
    }

    fn kind(&self) -> SourceKind {
        SourceKind::Xtream
    }

    /// VOD is advertised only when the panel actually has movies or series (probed at connect).
    fn caps(&self) -> Caps {
        let c = Caps::LIVE | Caps::EPG | Caps::CATCHUP | Caps::FAVOURITES | Caps::LIMITS;
        if self.has_vod.load(std::sync::atomic::Ordering::Relaxed) {
            c | Caps::VOD
        } else {
            c
        }
    }

    async fn connect(&self) -> Result<AccountInfo> {
        let info = self.account().await?;
        let vod = !self
            .vod_category_list()
            .await
            .unwrap_or_default()
            .is_empty()
            || !self
                .series_categories()
                .await
                .unwrap_or_default()
                .is_empty();
        self.has_vod
            .store(vod, std::sync::atomic::Ordering::Relaxed);
        Ok(info)
    }

    /// Xtream is not paginated: everything arrives as one page.
    async fn channels(&self, _page: Page) -> Result<ChannelPage> {
        self.ensure_account().await?;
        let tz = self.st().server_tz;
        let streams = self.live_streams(None).await?;
        let channels: Vec<SourceChannel> = streams
            .iter()
            .map(|s| self.live_to_channel(s, tz.as_deref()))
            .collect();
        self.vault.persist().await?;
        Ok(ChannelPage {
            count: Some(channels.len() as u64),
            channels,
            has_next: false,
        })
    }

    async fn groups(&self) -> Result<Vec<SourceGroup>> {
        Ok(self
            .live_categories()
            .await?
            .into_iter()
            .map(|c| SourceGroup {
                ext_id: c.id,
                name: c.name,
            })
            .collect())
    }

    /// No windowed guide API: the guide comes from `guide_feed` (`xmltv.php`).
    async fn guide(&self, _window: TimeWindow) -> Result<Vec<Programme>> {
        Ok(Vec::new())
    }

    async fn guide_feed(
        &self,
        prev: Option<&crate::m3u::load::Validators>,
    ) -> Result<Option<super::GuideFeed>> {
        use crate::m3u::load::Conditional;
        let url = self.xmltv_url()?;
        let (file, validators): (Option<Fetched>, _) =
            match self.fetcher()?.fetch_if_changed(&url, prev).await? {
                Conditional::NotModified => (None, prev.cloned().unwrap_or_default()),
                Conditional::Fetched(f, v) => (Some(f), v),
            };
        Ok(Some(super::GuideFeed {
            file,
            validators,
            default_tz: self.st().server_tz,
        }))
    }

    async fn play(&self, ch: ChannelKey) -> Result<PlayPlan> {
        self.ensure_account().await?;
        // A direct source, when the panel provides one, wins; otherwise build at play time.
        let url = match ch.play_url.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => self.vault.resolve(u)?,
            None => self.live_url(&ch.ext_id)?,
        };
        let hls = url.contains(".m3u8");
        let mut redact = self.redaction_set();
        redact.add(url.clone());
        Ok(PlayPlan {
            tls_pin: self.tls_pin.clone(),
            url,
            headers: vec![],
            surface: if hls { Surface::Hls } else { Surface::Ts },
            profile: PlayProfile::Live,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    /// `timeshift` template path: times come from the programme row; the wall clock is rendered
    /// in the panel's declared timezone (carried on the channel's template as data).
    async fn catchup(&self, ch: ChannelKey, p: &Programme) -> Result<PlayPlan> {
        self.ensure_account().await?;
        let template = ch
            .catchup_template
            .as_deref()
            .and_then(CatchupTemplate::from_json)
            .unwrap_or_else(|| CatchupTemplate {
                mode: Mode::Xc,
                source: None,
                time: TimeSemantics::LocalWallclock {
                    tz: self.st().server_tz.unwrap_or_else(|| "UTC".into()),
                },
                correction_secs: 0,
                days: 0,
            });
        let ctx = ExpandCtx {
            live_url: "",
            start_utc: p.start_utc,
            end_utc: p.end_utc,
            now_utc: crate::sync::now_utc(),
            channel_id: &ch.ext_id,
            catchup_id: &ch.ext_id,
            xc: Some(XcParts {
                base: self.base.as_str().trim_end_matches('/').to_owned(),
                username: self.username.clone(),
                password: self.password.expose().to_owned(),
                stream_id: ch.ext_id.clone(),
            }),
        };
        let url = expand(&template, &ctx)?;
        let mut redact = self.redaction_set();
        redact.add(url.clone());
        // Template-path sessions have no server session to delete.
        Ok(PlayPlan {
            tls_pin: self.tls_pin.clone(),
            url,
            headers: vec![],
            surface: Surface::Direct,
            profile: PlayProfile::Seekable,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    async fn stop_session(&self, _s: SessionId) -> Result<()> {
        Ok(())
    }

    async fn vod_categories(&self, kind: VodKind) -> Result<Vec<VodCategoryInfo>> {
        let cats = if kind == VodKind::Movie {
            self.vod_category_list().await?
        } else {
            self.series_categories().await?
        };
        let counts = self.category_counts(kind).await.unwrap_or_default();
        Ok(cats
            .into_iter()
            .map(|c| VodCategoryInfo {
                count: Some(counts.get(&c.id).copied().unwrap_or(0)),
                ext_id: c.id,
                name: c.name,
            })
            .collect())
    }

    async fn vod_list(&self, kind: VodKind, q: &VodListQuery) -> Result<VodListPage> {
        let needle = q
            .search
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let matches = |name: &str, cat: &Option<String>| {
            needle
                .as_ref()
                .is_none_or(|n| name.to_lowercase().contains(n.as_str()))
                && q.category_ext
                    .as_ref()
                    .is_none_or(|c| cat.as_deref() == Some(c.as_str()))
        };
        let (offset, limit) = (
            usize::try_from(q.offset.max(0)).unwrap_or(0),
            usize::try_from(q.limit.clamp(1, 500)).unwrap_or(50),
        );
        let mut items = Vec::new();
        let mut total = 0usize;
        if kind == VodKind::Movie {
            for m in self
                .movies_cached()
                .await?
                .iter()
                .filter(|m| matches(&m.name, &m.category_id))
            {
                if total >= offset && items.len() < limit {
                    items.push(self.movie_item(m));
                }
                total += 1;
            }
        } else {
            for sr in self
                .series_cached()
                .await?
                .iter()
                .filter(|s| matches(&s.name, &s.category_id))
            {
                if total >= offset && items.len() < limit {
                    items.push(self.series_item(sr));
                }
                total += 1;
            }
        }
        Ok(VodListPage {
            items,
            total: Some(i64::try_from(total).unwrap_or(0)),
        })
    }

    async fn vod_detail(&self, kind: VodKind, ext_id: &str) -> Result<VodItem> {
        let found = if kind == VodKind::Movie {
            self.movies_cached()
                .await?
                .iter()
                .find(|m| m.stream_id == ext_id)
                .map(|m| self.movie_item(m))
        } else {
            self.series_cached()
                .await?
                .iter()
                .find(|s| s.series_id == ext_id)
                .map(|s| self.series_item(s))
        };
        found.ok_or_else(|| AppError::stream("That title is not in the library any more."))
    }

    async fn series_episodes(&self, series_ext_id: &str) -> Result<Vec<VodItem>> {
        let info = self.series_info(series_ext_id).await?;
        Ok(info
            .episodes
            .into_iter()
            .map(|e| VodItem {
                kind: Some(VodKind::Episode),
                ext_id: e.id,
                parent_ext: Some(series_ext_id.to_owned()),
                title: e.title,
                season: Some(e.season).filter(|s| *s > 0),
                episode: Some(e.episode).filter(|n| *n > 0),
                duration_secs: e.duration_secs.filter(|d| *d > 0),
                container: e.container_extension,
                ..Default::default()
            })
            .collect())
    }

    /// `/movie/<u>/<p>/<id>.<ext>` or `/series/<u>/<p>/<episode id>.<ext>`: built at play time.
    async fn play_vod(&self, item: &VodItem) -> Result<PlayPlan> {
        self.ensure_account().await?;
        let ext = item
            .container
            .as_deref()
            .filter(|e| !e.is_empty())
            .unwrap_or("mp4");
        let url = match item.kind {
            Some(VodKind::Movie) => self.movie_url(&item.ext_id, ext)?,
            Some(VodKind::Episode) => self.series_url(&item.ext_id, ext)?,
            _ => return Err(AppError::stream("Only movies and episodes can be played.")),
        };
        let mut redact = self.redaction_set();
        redact.add(url.clone());
        Ok(PlayPlan {
            tls_pin: self.tls_pin.clone(),
            surface: if ext == "m3u8" {
                Surface::Hls
            } else {
                Surface::Direct
            },
            url,
            headers: vec![],
            profile: PlayProfile::Seekable,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    fn credentials(&self) -> std::collections::HashMap<String, String> {
        self.vault.export()
    }

    fn load_credentials(&self, map: std::collections::HashMap<String, String>) {
        self.vault.load(map);
    }

    async fn fetch_image(&self, url: &str) -> Result<crate::images::RawImage> {
        crate::images::fetch_public(&self.vault.resolve(url)?, self.tls_pin.as_ref()).await
    }
}

#[async_trait]
impl crate::sessions::SessionStopper for XtreamClient {
    async fn stop(&self, _source_id: i64, _kind: &str, _session_id: &str) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl crate::userstate::AppDataRemote for XtreamClient {
    async fn get(&self, _key: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn put(&self, _key: &str, _value: &Value) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
