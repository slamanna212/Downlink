//! `Service`: the façade the Tauri layer holds in state (it does not depend on Tauri). Owns the
//! store, the secret store, the image cache and one live client per source, and exposes the
//! command-level verbs of PLAN §2.5. Returns camelCase serde DTOs matching
//! `apps/ui/src/api/types.ts`; events go out on a broadcast channel the app forwards.

mod details;
pub mod dto;
mod logtail;
mod media;
mod mediaserver;
mod sports;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Map, Value};
use tokio::sync::{RwLock, broadcast};

use crate::auth::{AuthProvider, SecretKind, SecretStore, SecretString, put_source_secret};
use crate::catalog::{self, ChannelFilter};
use crate::catchup::{self, CatchupPlay, PositionReporter};
use crate::epg::{self, refresh};
use crate::error::{AppError, Result};
use crate::images::{ImageBytes, ImageCache, ImageFetcher, ImageSize, RawImage, parse_dlimg};
use crate::m3u::load::Location;
use crate::sessions::{self, SessionStopper};
use crate::source::dispatcharr::{
    ClientOptions, Credentials, DispatcharrClient, logo_url, normalise_base_url,
};
use crate::source::m3u::M3uSource;
use crate::source::xtream::XtreamClient;
use crate::source::{ChannelKey, PlayPlan, SourceClient, SourceId, SourceKind, all_channels};
use crate::store::{NewSource, SourceRow, Store};
use crate::sync::{self, now_utc};
use crate::userstate;

pub use dto::*;

/// What `failover_next` hands the desktop: a fresh plan and where we are in the stream list.
#[derive(Debug)]
pub struct FailoverStep {
    pub plan: PlayPlan,
    /// 1-based index of the provider stream now believed to be in use.
    pub source_index: usize,
    pub source_count: usize,
    /// The server was asked to (and did) switch to its next stream.
    pub switched: bool,
}

/// A credential-free form of a playlist URL for display: no userinfo, no query.
fn display_url(u: &str) -> String {
    match url::Url::parse(u) {
        Ok(mut p) => {
            let _ = p.set_username("");
            let _ = p.set_password(None);
            p.set_query(None);
            p.set_fragment(None);
            p.to_string()
        }
        Err(_) => String::new(),
    }
}
/// Whether a source last synced at `last_sync_at` is due a background refresh every
/// `interval_secs` (0 = auto refresh off: never due).
fn sync_due(last_sync_at: Option<i64>, now: i64, interval_secs: i64) -> bool {
    interval_secs > 0 && last_sync_at.is_none_or(|t| now - t >= interval_secs)
}

/// The auto refresh settings, read from the settings object.
#[derive(Debug, Clone, Copy)]
struct RefreshPrefs {
    interval_secs: i64,
    during_playback: bool,
}

impl RefreshPrefs {
    fn from_settings(v: &Value) -> Self {
        let hours = v
            .get(SET_AUTO_REFRESH_HOURS)
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_AUTO_REFRESH_HOURS)
            .clamp(0, MAX_AUTO_REFRESH_HOURS);
        Self {
            interval_secs: hours * 3600,
            during_playback: v
                .get(SET_REFRESH_DURING_PLAYBACK)
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }
}

/// What the last XMLTV import of a source was built from, to skip re-importing an unchanged feed.
/// No URLs or credentials: validators are keyed by a URL hash.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GuideFeedState {
    validators: crate::m3u::load::Validators,
    content_hash: String,
    inputs_hash: String,
    ingested_at: i64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Streaming SHA-256 of a file (decompression not needed: identical bytes, identical feed).
async fn file_sha256(path: PathBuf) -> Result<String> {
    tokio::task::spawn_blocking(move || -> Result<String> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let file = crate::util::open_import_file_limited(&path, crate::util::MAX_GUIDE_FILE_BYTES)?;
        let mut f = crate::util::BoundedReader::new(file, crate::util::MAX_GUIDE_FILE_BYTES);
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(hex(&h.finalize()))
    })
    .await
    .map_err(|_| AppError::local("Reading the guide was interrupted."))?
}

pub use details::{CrewDto, MediaDetailsDto, PersonDto, TmdbStatusDto};
pub use logtail::LogTail;
pub use media::{RecordingPlay, VodPlay};
pub use mediaserver::{
    MediaMatchDto, MediaServerStatusDto, MediaTargetArg, PlexPinDto, PlexServerDto, PlexStatusDto,
};

const SETTINGS_KEY: &str = "settings";
const PREF_ADULT: &str = "pref.include_adult";
const PREF_RADIO: &str = "pref.include_radio";
/// With auto refresh off, sources not synced for this long show as stale. With it on, age alone
/// never makes a source stale: only a failed refresh does.
const STALE_AFTER_SECS: i64 = 6 * 3600;
/// Setting: hours between background refreshes (0 = off). Default and allowed range.
const SET_AUTO_REFRESH_HOURS: &str = "autoRefreshHours";
const DEFAULT_AUTO_REFRESH_HOURS: i64 = 4;
const MAX_AUTO_REFRESH_HOURS: i64 = 24;
/// Setting: whether background refreshes may run while something is playing (default on).
const SET_REFRESH_DURING_PLAYBACK: &str = "refreshDuringPlayback";
/// Per-source state of the last XMLTV download/import (`GuideFeedState`), in the kv table.
const GUIDE_FEED_KEY_PREFIX: &str = "guide_feed.";
/// An unchanged XMLTV feed is still fully re-imported this often (retention pruning).
const GUIDE_REINGEST_SECS: i64 = 24 * 3600;
/// kv key: when old guide entries were last pruned, and how often that runs.
const HOUSEKEEPING_KEY: &str = "housekeeping.guide_pruned_at";
const HOUSEKEEPING_SECS: i64 = 24 * 3600;
/// The light guide refresh re-fetches a windowed guide's visible hours once it is this old.
pub const VISIBLE_GUIDE_REFRESH_SECS: i64 = 3600;

pub struct ServiceConfig {
    /// Directory for the database and the image cache.
    pub data_dir: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
    pub app_version: String,
}

pub struct Service {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    images: ImageCache,
    events: broadcast::Sender<Event>,
    clients: RwLock<HashMap<i64, Arc<dyn SourceClient>>>,
    /// The same Dispatcharr clients, concretely (auth headers for play plans).
    dispatcharr: RwLock<HashMap<i64, Arc<DispatcharrClient>>>,
    status: Mutex<HashMap<i64, SourceStatus>>,
    /// One sync at a time per source: a manual sync during an auto refresh waits for it.
    sync_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// One download of a source's whole VOD catalogue (movies or series) at a time.
    vod_catalog_locks: Mutex<HashMap<(i64, &'static str), Arc<tokio::sync::Mutex<()>>>>,
    /// Auto refresh interval in seconds (0 = off), cached from settings for `status_of`.
    auto_refresh_secs: std::sync::atomic::AtomicI64,
    registered_logos: Mutex<HashSet<String>>,
    /// Failover position (0-based) believed to be in use, per channel row id.
    current_stream: Mutex<HashMap<i64, usize>>,
    /// The live HLS session in use, `(source id, token)`: one at a time.
    live_hls: Mutex<Option<(i64, String)>>,
    /// Channels whose HLS session was already re-minted since their last `play_channel`.
    hls_reminted: Mutex<HashSet<i64>>,
    /// Category id -> (source id, server category id) for ids that were hashed.
    vod_cats: Mutex<HashMap<i64, (i64, String)>>,
    log_tail: LogTail,
    app_version: String,
    /// The connected Plex server, built lazily from settings + keychain.
    plex: tokio::sync::Mutex<Option<Arc<crate::mediaserver::plex::PlexServer>>>,
    plex_tv_url: std::sync::RwLock<String>,
    /// TMDB client, built lazily from the keychain: `None` = not looked yet, `Some(None)` = no key.
    #[allow(clippy::option_option)]
    tmdb: tokio::sync::Mutex<Option<Option<Arc<crate::tmdb::TmdbClient>>>>,
    tmdb_api: std::sync::RwLock<String>,
    /// One sports match rebuild at a time.
    sports_rebuild: Arc<tokio::sync::Mutex<()>>,
    /// Set when a background rebuild is asked for while one runs; the running one goes again.
    sports_dirty: Arc<std::sync::atomic::AtomicBool>,
}

impl Service {
    pub async fn new(cfg: ServiceConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let store =
            Store::open_with_secrets(&cfg.data_dir.join("downlink.sqlite"), cfg.secrets.as_ref())
                .await?;
        let service = Self::with_store(
            store,
            cfg.secrets,
            cfg.data_dir.join("images"),
            cfg.app_version,
        );
        if let Err(error) = service.images.reconcile().await {
            tracing::warn!(%error, "artwork cache reconciliation will be retried");
        }
        Ok(service)
    }

    /// After `new` failed with "created by a newer version": move that library aside (kept as
    /// `downlink.sqlite.newer-<epoch>.bak`, with its WAL files) so the next `new` starts fresh.
    /// Static because there is no open Service in that state. Returns the archived file's path.
    pub fn archive_newer_database(data_dir: &std::path::Path) -> Result<PathBuf> {
        crate::store::archive_database(&data_dir.join("downlink.sqlite"), now_utc())
    }

    /// For tests and the CLI: an already-open store.
    pub fn with_store(
        store: Store,
        secrets: Arc<dyn SecretStore>,
        image_dir: PathBuf,
        app_version: String,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(256);
        Arc::new(Self {
            images: ImageCache::new(image_dir, store.clone()),
            store,
            secrets,
            events,
            clients: RwLock::new(HashMap::new()),
            dispatcharr: RwLock::new(HashMap::new()),
            status: Mutex::new(HashMap::new()),
            sync_locks: Mutex::new(HashMap::new()),
            vod_catalog_locks: Mutex::new(HashMap::new()),
            auto_refresh_secs: std::sync::atomic::AtomicI64::new(DEFAULT_AUTO_REFRESH_HOURS * 3600),
            registered_logos: Mutex::new(HashSet::new()),
            current_stream: Mutex::new(HashMap::new()),
            live_hls: Mutex::new(None),
            hls_reminted: Mutex::new(HashSet::new()),
            vod_cats: Mutex::new(HashMap::new()),
            log_tail: LogTail::new(),
            app_version,
            plex: tokio::sync::Mutex::new(None),
            plex_tv_url: std::sync::RwLock::new(crate::mediaserver::plex::PLEX_TV.to_owned()),
            tmdb: tokio::sync::Mutex::new(None),
            tmdb_api: std::sync::RwLock::new(crate::tmdb::TMDB_API.to_owned()),
            sports_rebuild: Arc::new(tokio::sync::Mutex::new(())),
            sports_dirty: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Subscribe to `catalog://updated` / `task://progress` events (forward them to the UI).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// A tracing writer target that feeds the diagnostics log tail.
    pub fn log_tail(&self) -> LogTail {
        self.log_tail.clone()
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e); // no receivers is fine
    }

    fn progress(&self, source_id: i64, phase: &str, done: usize, total: usize) {
        self.emit(Event::TaskProgress(TaskProgress {
            source_id,
            phase: phase.to_owned(),
            done,
            total,
        }));
    }

    fn set_status(&self, id: i64, s: SourceStatus) {
        if let Ok(mut g) = self.status.lock() {
            g.insert(id, s);
        }
    }

    /// `Connecting` until the first probe/sync of this run; then the recorded outcome, with a
    /// successful-but-old sync shown as stale.
    fn status_of(&self, row: &SourceRow) -> SourceStatus {
        match self
            .status
            .lock()
            .ok()
            .and_then(|g| g.get(&row.id).copied())
        {
            None => SourceStatus::Connecting,
            // Auto refresh on: a failed refresh says stale, age alone does not.
            Some(SourceStatus::Connected)
                if self
                    .auto_refresh_secs
                    .load(std::sync::atomic::Ordering::Relaxed)
                    > 0 =>
            {
                SourceStatus::Connected
            }
            Some(SourceStatus::Connected) => match row.last_sync_at {
                Some(t) if now_utc() - t <= STALE_AFTER_SECS => SourceStatus::Connected,
                _ => SourceStatus::Stale,
            },
            Some(s) => s,
        }
    }

    // ---- connections ------------------------------------------------------------------------

    async fn secret_of(&self, row: &SourceRow) -> Result<SecretString> {
        row.secret_ref
            .as_deref()
            .map(|r| self.secrets.get(r))
            .transpose()?
            .flatten()
            .ok_or_else(|| {
                AppError::auth(
                    "The saved credentials for this source were not found. Sign in again.",
                )
            })
    }

    fn optional_secret(&self, row: &SourceRow, kind: SecretKind) -> Option<String> {
        let r = crate::auth::secret_ref(&row.uuid, kind);
        self.secrets
            .get(&r)
            .ok()
            .flatten()
            .map(|s| s.expose().to_owned())
    }

    /// A live connection for a source, built from its row and keychain entries on first use.
    async fn client(&self, source_id: i64) -> Result<Arc<dyn SourceClient>> {
        if let Some(c) = self.clients.read().await.get(&source_id) {
            return Ok(c.clone());
        }
        let mut clients = self.clients.write().await;
        if let Some(c) = clients.get(&source_id) {
            return Ok(c.clone());
        }
        let row = self.store.source(source_id).await?;
        let tls_pin = self.source_pin(&row)?;
        let kind =
            SourceKind::parse(&row.kind).ok_or_else(|| AppError::local("Unknown source type."))?;
        let c: Arc<dyn SourceClient> = match kind {
            SourceKind::Dispatcharr => {
                let base = row
                    .base_url
                    .as_deref()
                    .ok_or_else(|| AppError::local("This source has no address."))?;
                let secret = self.secret_of(&row).await?;
                let creds = match (&row.username, row.secret_ref.as_deref()) {
                    (Some(u), Some(r)) if r.ends_with("/password") => Credentials::Password {
                        username: u.clone(),
                        password: secret,
                    },
                    _ => Credentials::ApiKey(secret),
                };
                let c = Arc::new(DispatcharrClient::new(
                    SourceId(row.id),
                    base,
                    creds,
                    ClientOptions {
                        tls_pin,
                        ..Default::default()
                    },
                )?);
                self.dispatcharr.write().await.insert(source_id, c.clone());
                c
            }
            SourceKind::Xtream => {
                let base = row
                    .base_url
                    .as_deref()
                    .ok_or_else(|| AppError::local("This source has no address."))?;
                let pw = self.secret_of(&row).await?;
                let client = XtreamClient::new(
                    SourceId(row.id),
                    base,
                    row.username.as_deref().unwrap_or_default(),
                    pw,
                    tls_pin,
                )?;
                client.attach_credentials(self.secrets.clone(), &row.uuid)?;
                Arc::new(client)
            }
            SourceKind::M3u => {
                let location = match row.secret_ref.as_deref() {
                    Some(_) => Location::parse(self.secret_of(&row).await?.expose())?,
                    None => Location::parse(row.base_url.as_deref().unwrap_or_default())?,
                };
                let xmltv = self
                    .optional_secret(&row, SecretKind::XmltvUrl)
                    .into_iter()
                    .collect();
                let client = M3uSource::new(SourceId(row.id), location, xmltv, tls_pin)?;
                client.attach_credentials(self.secrets.clone(), &row.uuid)?;
                Arc::new(client)
            }
        };
        clients.insert(source_id, c.clone());
        Ok(c)
    }

    /// An unsaved connection for `test_source` / `add_source`.
    fn probe_client(
        input: &AddSourceInput,
        id: i64,
        tls_pin: Option<crate::tls::Pin>,
    ) -> Result<Arc<dyn SourceClient>> {
        match SourceKind::parse(&input.kind) {
            Some(SourceKind::Dispatcharr) => {
                let creds = match (&input.api_key, &input.username, &input.password) {
                    (Some(k), _, _) if !k.trim().is_empty() => {
                        Credentials::ApiKey(SecretString::new(k.trim()))
                    }
                    (_, Some(u), Some(p)) if !u.trim().is_empty() && !p.is_empty() => {
                        Credentials::Password {
                            username: u.trim().to_owned(),
                            password: SecretString::new(p.clone()),
                        }
                    }
                    _ => {
                        return Err(AppError::local(
                            "Enter an API key, or a username and password.",
                        ));
                    }
                };
                Ok(Arc::new(DispatcharrClient::new(
                    SourceId(id),
                    &input.url,
                    creds,
                    ClientOptions {
                        tls_pin,
                        ..Default::default()
                    },
                )?))
            }
            Some(SourceKind::Xtream) => {
                let (u, p) = match (&input.username, &input.password) {
                    (Some(u), Some(p)) if !u.trim().is_empty() && !p.is_empty() => (u.trim(), p),
                    _ => return Err(AppError::local("Enter the username and password.")),
                };
                Ok(Arc::new(XtreamClient::new(
                    SourceId(id),
                    &input.url,
                    u,
                    SecretString::new(p.clone()),
                    tls_pin,
                )?))
            }
            Some(SourceKind::M3u) => Ok(Arc::new(Self::m3u_from_input(input, id, tls_pin)?)),
            None => Err(AppError::local("Unknown source type.")),
        }
    }

    fn m3u_from_input(
        input: &AddSourceInput,
        id: i64,
        tls_pin: Option<crate::tls::Pin>,
    ) -> Result<M3uSource> {
        let location = Location::parse(&input.url)?;
        let xmltv = input
            .xmltv_url
            .iter()
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty())
            .collect();
        M3uSource::new(SourceId(id), location, xmltv, tls_pin)
    }

    /// Learn the certificate for an explicit add/test of an HTTPS source with the toggle on.
    async fn pin_for_input(input: &AddSourceInput) -> Result<Option<crate::tls::Pin>> {
        if !input.allow_self_signed || !crate::tls::is_https(&input.url) {
            return Ok(None);
        }
        let fp = crate::tls::fetch_fingerprint(&input.url).await?;
        Ok(Some(crate::tls::Pin::new(&input.url, fp, None)?))
    }

    /// The address whose certificate a stored source is pinned to.
    fn pin_url(&self, row: &SourceRow) -> Option<String> {
        if row.kind == "m3u" && row.secret_ref.is_some() {
            return self.optional_secret(row, SecretKind::PlaylistUrl);
        }
        row.base_url.clone()
    }

    /// Load the trust decision for this particular source.
    fn source_pin(&self, row: &SourceRow) -> Result<Option<crate::tls::Pin>> {
        if !row.tls_insecure {
            return Ok(None);
        }
        let Some(url) = self.pin_url(row).filter(|url| crate::tls::is_https(url)) else {
            return Ok(None);
        };
        let fingerprint = row.tls_pin.clone().ok_or_else(|| {
            AppError::auth("This source has no trusted certificate.").with_source_id(Some(row.id))
        })?;
        Ok(Some(crate::tls::Pin::new(&url, fingerprint, Some(row.id))?))
    }

    /// The user confirmed that the server's new certificate is fine (after "The server's
    /// certificate changed."): learn it, store it, and drop the cached connection so the next call
    /// uses it. Never called implicitly.
    pub async fn trust_new_certificate(&self, source_id: i64) -> Result<()> {
        let row = self.store.source(source_id).await?;
        if !row.tls_insecure {
            return Err(AppError::local(
                "This source does not allow a self-signed certificate.",
            ));
        }
        let url = self
            .pin_url(&row)
            .ok_or_else(|| AppError::local("This source has no address."))?;
        let fp = crate::tls::fetch_fingerprint(&url).await?;
        let mut clients = self.clients.write().await;
        self.store.set_source_tls_pin(source_id, Some(&fp)).await?;
        clients.remove(&source_id);
        self.dispatcharr.write().await.remove(&source_id);
        Ok(())
    }

    fn check_kind(input: &AddSourceInput) -> Result<()> {
        SourceKind::parse(&input.kind)
            .map(|_| ())
            .ok_or_else(|| AppError::local("Unknown source type."))
    }

    // ---- sources ----------------------------------------------------------------------------

    pub async fn list_sources(&self) -> Result<Vec<SourceDto>> {
        let mut out = Vec::new();
        for r in self.store.list_sources().await? {
            let days = refresh::max_catchup_days(&self.store, r.id).await?;
            out.push(SourceDto::from_row(&r, self.status_of(&r), days));
        }
        Ok(out)
    }

    /// Connect and count what is there, without saving anything.
    pub async fn test_source(&self, input: AddSourceInput) -> Result<TestSourceResult> {
        Self::check_kind(&input)?;
        let pin = Self::pin_for_input(&input).await?;
        let c = Self::probe_client(&input, 0, pin)?;
        let info = c.connect().await?;
        let groups = c.groups().await?;
        let (channels, _) = all_channels(c.as_ref(), 500).await?;
        let programmes = match c.guide_feed(None).await {
            Ok(Some(crate::source::GuideFeed {
                file: Some(file), ..
            })) => {
                let path = file.path().to_path_buf();
                tokio::task::spawn_blocking(move || {
                    crate::xmltv::parse_programmes(&path, chrono_tz::Tz::UTC, |_| {})
                        .map(|s| s.programmes as usize)
                })
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(0)
            }
            Ok(None) => c
                .guide(epg::visible_window(now_utc()))
                .await
                .map(|p| p.len())
                .unwrap_or(0),
            // No validators were sent, so a 304 cannot happen here.
            Ok(Some(_)) | Err(_) => 0,
        };
        Ok(TestSourceResult {
            channels: crate::catalog::visible_count(&channels),
            groups: groups.len() as i64,
            programmes: programmes as i64,
            server_version: info.server_version,
            user: info.username,
        })
    }

    /// Verify, save (secrets to the keychain, references to SQLite), then do the first sync.
    pub async fn add_source(&self, input: AddSourceInput) -> Result<SourceDto> {
        Self::check_kind(&input)?;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(AppError::local("Give the source a name."));
        }
        let kind = SourceKind::parse(&input.kind).unwrap_or(SourceKind::Dispatcharr);
        let pin = Self::pin_for_input(&input).await?;
        // Fail fast: nothing is persisted if the server, file or credentials are wrong.
        let m3u_probe = if kind == SourceKind::M3u {
            Some(Arc::new(Self::m3u_from_input(&input, 0, pin.clone())?))
        } else {
            None
        };
        let probe: Arc<dyn SourceClient> = match &m3u_probe {
            Some(m) => m.clone(),
            None => Self::probe_client(&input, 0, pin.clone())?,
        };
        probe.connect().await?;

        // What goes where: the keychain gets every secret (keys, passwords, URLs that may embed
        // credentials); SQLite gets only a credential-free address for display.
        let mut secrets: Vec<(SecretKind, SecretString)> = Vec::new();
        let (base_url, username, primary): (String, Option<String>, Option<usize>) = match kind {
            SourceKind::Dispatcharr => {
                let base = normalise_base_url(&input.url)?.to_string();
                match (&input.api_key, &input.password) {
                    (Some(k), _) if !k.trim().is_empty() => {
                        secrets.push((SecretKind::ApiKey, SecretString::new(k.trim())));
                        (base, None, Some(0))
                    }
                    (_, Some(p)) => {
                        secrets.push((SecretKind::Password, SecretString::new(p.clone())));
                        (
                            base,
                            Some(input.username.clone().unwrap_or_default().trim().to_owned()),
                            Some(0),
                        )
                    }
                    _ => {
                        return Err(AppError::local(
                            "Enter an API key, or a username and password.",
                        ));
                    }
                }
            }
            SourceKind::Xtream => {
                let base = crate::source::xtream::normalise_base(&input.url)?.to_string();
                secrets.push((
                    SecretKind::Password,
                    SecretString::new(input.password.clone().unwrap_or_default()),
                ));
                (
                    base,
                    Some(input.username.clone().unwrap_or_default().trim().to_owned()),
                    Some(0),
                )
            }
            SourceKind::M3u => match Location::parse(&input.url)? {
                Location::File(p) => (format!("file://{}", p.display()), None, None),
                Location::Url(u) => {
                    secrets.push((SecretKind::PlaylistUrl, SecretString::new(u.clone())));
                    (display_url(&u), None, Some(0))
                }
            },
        };
        if let Some(x) = input
            .xmltv_url
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
        {
            secrets.push((SecretKind::XmltvUrl, SecretString::new(x)));
        }

        let row = self
            .store
            .insert_source(&NewSource {
                kind: kind.as_str().to_owned(),
                name: name.to_owned(),
                base_url: Some(base_url),
                secret_ref: None,
                username,
                tls_insecure: input.allow_self_signed,
                created_at: now_utc(),
            })
            .await?;
        if let Some(pin) = &pin {
            self.store
                .set_source_tls_pin(row.id, Some(pin.fingerprint()))
                .await?;
        }
        let saved = async {
            let mut primary_ref = None;
            for (i, (k, v)) in secrets.iter().enumerate() {
                let r = put_source_secret(self.secrets.as_ref(), &row.uuid, *k, v)?;
                if Some(i) == primary {
                    primary_ref = Some(r);
                }
            }
            self.store
                .set_source_secret_ref(row.id, primary_ref.as_deref())
                .await
        }
        .await;
        if let Err(e) = saved {
            self.delete_secrets(&row.uuid);
            let _ = self.store.delete_source(row.id).await;
            return Err(e);
        }
        // Reuse the verified probe for M3U so a large playlist is not downloaded twice.
        if let Some(m) = m3u_probe {
            m.set_id(SourceId(row.id));
            m.attach_credentials(self.secrets.clone(), &row.uuid)?;
            self.clients.write().await.insert(row.id, m);
        }
        self.sync_source(row.id).await?;
        let row = self.store.source(row.id).await?;
        let days = refresh::max_catchup_days(&self.store, row.id).await?;
        Ok(SourceDto::from_row(&row, self.status_of(&row), days))
    }

    fn delete_secrets(&self, uuid: &str) {
        for k in [
            SecretKind::ApiKey,
            SecretKind::Password,
            SecretKind::RefreshToken,
            SecretKind::PlaylistUrl,
            SecretKind::XmltvUrl,
            SecretKind::Credentials,
        ] {
            let _ = self.secrets.delete(&crate::auth::secret_ref(uuid, k));
        }
    }

    /// Cheap reachability probe at launch (PLAN §2.5 offline mode): Dispatcharr asks for the
    /// account (version + identity, so a revoked key shows as `auth_error`), Xtream logs in, an M3U
    /// file checks it still exists. A remote M3U playlist is not downloaded just to probe: its
    /// state follows the last sync. Offline sources keep their cached catalogue and show
    /// `last_sync_at` as "last updated".
    pub async fn probe_source(&self, source_id: i64) -> Result<SourceStatus> {
        let row = self.store.source(source_id).await?;
        let status = match self.probe_inner(&row).await {
            Ok(()) => SourceStatus::Connected,
            Err(e) => match e.code {
                crate::ErrorCode::Auth => SourceStatus::AuthError,
                crate::ErrorCode::Network => SourceStatus::Offline,
                _ => SourceStatus::Stale,
            },
        };
        self.set_status(source_id, status);
        Ok(status)
    }

    async fn probe_inner(&self, row: &SourceRow) -> Result<()> {
        match SourceKind::parse(&row.kind) {
            Some(SourceKind::Dispatcharr) => {
                self.client(row.id).await?;
                let c = self.dispatcharr.read().await.get(&row.id).cloned();
                c.ok_or_else(|| AppError::local("This source is not available."))?
                    .me()
                    .await
                    .map(|_| ())
            }
            Some(SourceKind::Xtream) => self.client(row.id).await?.connect().await.map(|_| ()),
            Some(SourceKind::M3u) => {
                if let Some(p) = row
                    .base_url
                    .as_deref()
                    .and_then(|u| u.strip_prefix("file://"))
                    && !std::path::Path::new(p).exists()
                {
                    return Err(AppError::network(
                        "The playlist file is not there any more.",
                    ));
                }
                Ok(())
            }
            None => Err(AppError::local("Unknown source type.")),
        }
    }

    /// Probe every source concurrently (call once at startup, after `list_sources` has shown
    /// `connecting`). A reachable source whose data is old is synced straight away, so it goes
    /// `connecting` -> `syncing` -> `connected` and never shows as stale unless that sync fails.
    /// Emits `catalog://updated` per source so the UI refetches the statuses.
    pub async fn probe_sources(&self) -> Result<()> {
        let prefs = self.refresh_prefs().await?;
        let rows = self.store.list_sources().await?;
        let probes = rows.iter().map(|r| async move {
            let probed = self.probe_source(r.id).await;
            if matches!(probed, Ok(SourceStatus::Connected))
                && sync_due(r.last_sync_at, now_utc(), prefs.interval_secs)
            {
                if let Err(e) = self.sync_source(r.id).await {
                    tracing::warn!(source_id = r.id, error = %e, "launch refresh failed");
                }
                return; // sync_source emitted the outcome
            }
            self.emit_status(r.id).await;
        });
        futures_util::future::join_all(probes).await;
        Ok(())
    }

    /// Periodic background refresh: sync every source whose data is older than the
    /// `autoRefreshHours` setting, one at a time. Offline sources are retried here too, so they
    /// recover on their own (probed every tick even when no sync is due or auto refresh is off);
    /// `auth_error` waits for the user, and sources still being probed or
    /// synced are left alone. `playing`: something is on screen; with `refreshDuringPlayback`
    /// off the refresh waits for a later tick.
    pub async fn auto_refresh(&self, playing: bool) -> Result<()> {
        let prefs = self.refresh_prefs().await?;
        let syncs_allowed = prefs.interval_secs > 0 && !(playing && !prefs.during_playback);
        let now = now_utc();
        for r in self.store.list_sources().await? {
            let status = self.status.lock().ok().and_then(|g| g.get(&r.id).copied());
            let eligible = matches!(
                status,
                Some(SourceStatus::Connected | SourceStatus::Offline | SourceStatus::Stale)
            );
            if !eligible {
                continue;
            }
            if syncs_allowed && sync_due(r.last_sync_at, now, prefs.interval_secs) {
                tracing::info!(source_id = r.id, "auto refresh");
                if let Err(e) = self.sync_source(r.id).await {
                    tracing::warn!(source_id = r.id, error = %e, "auto refresh failed");
                }
                continue;
            }
            // No sync now, but the server was unreachable: a cheap probe (not a stream) lets the
            // source recover without waiting for the next sync, and the UI refetches what failed.
            if status == Some(SourceStatus::Offline)
                && matches!(self.probe_source(r.id).await, Ok(SourceStatus::Connected))
            {
                tracing::info!(source_id = r.id, "source reachable again");
                if let Ok(c) = self.counts(r.id, CatalogChange::SyncComplete).await {
                    self.emit(Event::CatalogUpdated(c));
                }
            }
        }
        Ok(())
    }

    /// Current auto refresh settings; also refreshes the cached interval `status_of` uses.
    async fn refresh_prefs(&self) -> Result<RefreshPrefs> {
        let prefs = RefreshPrefs::from_settings(&self.get_settings().await?);
        self.auto_refresh_secs
            .store(prefs.interval_secs, std::sync::atomic::Ordering::Relaxed);
        Ok(prefs)
    }

    async fn emit_status(&self, source_id: i64) {
        if let Ok(c) = self.counts(source_id, CatalogChange::Status).await {
            self.emit(Event::CatalogUpdated(c));
        }
    }

    /// Stop its sessions, delete its rows and its keychain entries.
    pub async fn remove_source(&self, source_id: i64) -> Result<()> {
        let row = self.store.source(source_id).await?;
        // Best effort: an unreachable server must not block removal.
        for s in self
            .store
            .list_open_sessions()
            .await?
            .into_iter()
            .filter(|s| s.source_id == source_id)
        {
            let _ = sessions::release(&self.store, self, s.source_id, &s.kind, &s.session_id).await;
        }
        if let Some(r) = row.secret_ref.as_deref() {
            self.secrets.delete(r)?;
        }
        self.delete_secrets(&row.uuid);
        self.store.delete_source(source_id).await?;
        // Cached lookups keyed by this source id, so a future source reusing it starts clean.
        for prefix in [
            crate::source::xtream::vod_cache_prefix(source_id),
            format!("dpg:{source_id}:"),
        ] {
            if let Err(e) = self.store.meta_clear_prefix(&prefix).await {
                tracing::warn!(error = %e, "clearing cached lookups failed");
            }
        }
        self.clients.write().await.remove(&source_id);
        self.dispatcharr.write().await.remove(&source_id);
        if let Ok(mut g) = self.status.lock() {
            g.remove(&source_id);
        }
        self.emit(Event::CatalogUpdated(CatalogUpdated {
            scope: CatalogChange::Sources,
            source_id,
            channels: 0,
            groups: 0,
            programmes: 0,
        }));
        Ok(())
    }

    async fn counts(&self, source_id: i64, scope: CatalogChange) -> Result<CatalogUpdated> {
        let one = |sql: &'static str| {
            let pool = self.store.pool().clone();
            async move {
                sqlx::query_scalar::<_, i64>(sql)
                    .bind(source_id)
                    .fetch_one(&pool)
                    .await
            }
        };
        Ok(CatalogUpdated {
            scope,
            source_id,
            channels: one("SELECT count(*) FROM channel WHERE source_id = ?").await?,
            groups: one("SELECT count(*) FROM channel_group WHERE source_id = ?").await?,
            programmes: one(
                "SELECT count(*) FROM programme WHERE channel_id IN (SELECT id FROM channel WHERE source_id = ?)",
            )
            .await?,
        })
    }

    /// Connect, sync channels + groups, fill the visible guide window, then sync user state.
    /// Emits `task://progress` per phase and `catalog://updated` at the end.
    /// If a sync of this source is already running, waits for it and returns without syncing
    /// again (its outcome is already recorded and emitted).
    pub async fn sync_source(&self, source_id: i64) -> Result<()> {
        let lock = self
            .sync_locks
            .lock()
            .map_err(|_| AppError::local("Sync state is unavailable."))?
            .entry(source_id)
            .or_default()
            .clone();
        let _guard = match lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                let _done = lock.lock().await;
                return Ok(());
            }
        };
        self.set_status(source_id, SourceStatus::Syncing);
        self.emit_status(source_id).await;
        let r = self.sync_inner(source_id).await;
        match &r {
            Ok(()) => self.set_status(source_id, SourceStatus::Connected),
            Err(e) => self.set_status(
                source_id,
                match e.code {
                    crate::ErrorCode::Auth => SourceStatus::AuthError,
                    crate::ErrorCode::Network => SourceStatus::Offline,
                    _ => SourceStatus::Stale,
                },
            ),
        }
        // The final status is recorded after sync_inner's last event: tell the UI.
        self.emit_status(source_id).await;
        r
    }

    async fn sync_inner(&self, source_id: i64) -> Result<()> {
        let client = self.client(source_id).await?;
        self.progress(source_id, "connect", 0, 4);
        let info = sync::connect_and_record(client.as_ref(), &self.store, source_id).await?;
        self.progress(source_id, "channels", 1, 4);
        sync::sync_channels_with(
            client.as_ref(),
            &self.store,
            source_id,
            info.user_level,
            // (fetched, total): total is 0 when the source does not say.
            &|done, total| {
                self.progress(source_id, "channels", done, total.map_or(0, |t| t as usize))
            },
        )
        .await?;
        // The channel list is complete and committed: show it now, the guide fills in behind it.
        self.emit(Event::CatalogUpdated(
            self.counts(source_id, CatalogChange::Channels).await?,
        ));
        // Which VOD categories have titles is recomputed here, not per view.
        if crate::source::Caps::from_db(self.store.source(source_id).await?.caps)
            .contains(crate::source::Caps::VOD)
            && let Err(e) = self.refresh_vod_categories(source_id).await
        {
            tracing::warn!(error = %e, "vod category sync failed");
        }
        self.progress(source_id, "guide", 2, 4);
        if let Err(e) = self.sync_guide_for(source_id, client.as_ref()).await {
            // The guide is not critical: channels stay usable.
            tracing::warn!(error = %e, "guide sync failed");
        }
        self.progress(source_id, "state", 3, 4);
        if client.kind() == SourceKind::Dispatcharr
            && let Err(e) = userstate::sync_state(&self.store, client.as_ref(), now_utc()).await
        {
            tracing::warn!(error = %e, "state sync failed");
        }
        // Channel and guide row counts may have moved by millions: refresh planner statistics.
        if let Err(e) = self.store.optimize().await {
            tracing::warn!(error = %e, "database optimize failed");
        }
        self.progress(source_id, "done", 4, 4);
        self.emit(Event::CatalogUpdated(
            self.counts(source_id, CatalogChange::SyncComplete).await?,
        ));
        // Guide and channels are final: re-match followed sports (docs/SPORTS.md §4).
        self.rebuild_sports_logged().await;
        Ok(())
    }

    /// Whole-feed guides (XMLTV) for M3U/Xtream; windowed grid for Dispatcharr.
    async fn sync_guide_for(&self, source_id: i64, client: &dyn SourceClient) -> Result<()> {
        let now = now_utc();
        let state_key = format!("{GUIDE_FEED_KEY_PREFIX}{source_id}");
        let prev: Option<GuideFeedState> = self
            .store
            .kv_get(&state_key)
            .await?
            .and_then(|(v, _)| serde_json::from_str(&v).ok());
        // A full re-import at least daily, so the retention window still prunes old programmes.
        let prev = prev.filter(|p| now - p.ingested_at < GUIDE_REINGEST_SECS);
        match client
            .guide_feed(prev.as_ref().map(|p| &p.validators))
            .await
        {
            Ok(Some(crate::source::GuideFeed { file: None, .. })) => {
                tracing::info!("guide unchanged on the server (304), import skipped");
                self.store.set_sync_ok(source_id, "guide", now).await?;
                Ok(())
            }
            Ok(Some(crate::source::GuideFeed {
                file: Some(file),
                validators,
                default_tz,
            })) => {
                let content_hash = match file_sha256(file.path().to_path_buf()).await {
                    Ok(hash) => hash,
                    Err(e) => {
                        self.store
                            .set_sync_error(source_id, "guide", &e.message)
                            .await?;
                        return Err(e);
                    }
                };
                let inputs_hash = self.guide_inputs_hash(source_id).await?;
                if let Some(p) = prev.as_ref()
                    && p.content_hash == content_hash
                    && p.inputs_hash == inputs_hash
                {
                    tracing::info!("guide content unchanged, import skipped");
                    let state = GuideFeedState {
                        validators,
                        ..p.clone()
                    };
                    self.store
                        .kv_set(&state_key, &serde_json::to_string(&state)?, now)
                        .await?;
                    self.store.set_sync_ok(source_id, "guide", now).await?;
                    return Ok(());
                }
                let feed = crate::source::GuideFeed {
                    file: Some(file),
                    validators: validators.clone(),
                    default_tz,
                };
                let r = self.ingest_feed(source_id, feed, now).await;
                if r.is_ok() {
                    let state = GuideFeedState {
                        validators,
                        content_hash,
                        inputs_hash,
                        ingested_at: now,
                    };
                    self.store
                        .kv_set(&state_key, &serde_json::to_string(&state)?, now)
                        .await?;
                }
                r
            }
            Ok(None) => self.sync_windowed_guide(source_id, client, now).await,
            Err(e) => {
                self.store
                    .set_sync_error(source_id, "guide", &e.message)
                    .await?;
                Err(e)
            }
        }
    }

    /// Fingerprint of everything besides the file that decides what an XMLTV import writes:
    /// the source's channels (ids, EPG keys, names, shifts) and the user's EPG remaps.
    async fn guide_inputs_hash(&self, source_id: i64) -> Result<String> {
        use sha2::{Digest, Sha256};
        let pool = self.store.pool();
        let chans: Option<String> = sqlx::query_scalar(
            "SELECT json_group_array(json_array(id, ext_id, tvg_id, tvg_name, name, epg_shift_secs))
             FROM (SELECT * FROM channel WHERE source_id = ? ORDER BY id)",
        )
        .bind(source_id)
        .fetch_one(pool)
        .await?;
        let remaps: Option<String> = sqlx::query_scalar(
            "SELECT json_group_array(json_array(channel_ext_id, xmltv_id))
             FROM (SELECT * FROM epg_remap WHERE source_id = ? ORDER BY channel_ext_id)",
        )
        .bind(source_id)
        .fetch_one(pool)
        .await?;
        let mut h = Sha256::new();
        h.update(chans.unwrap_or_default());
        h.update([0]);
        h.update(remaps.unwrap_or_default());
        Ok(hex(&h.finalize()))
    }

    /// Ingest a downloaded XMLTV feed (the file must be present).
    async fn ingest_feed(
        &self,
        source_id: i64,
        feed: crate::source::GuideFeed,
        now: i64,
    ) -> Result<()> {
        let Some(file) = feed.file else {
            return Ok(());
        };
        let row = self.store.source(source_id).await?;
        let tz = refresh::parse_tz(feed.default_tz.as_deref().or(row.server_tz.as_deref()));
        let days = refresh::max_catchup_days(&self.store, source_id)
            .await?
            .max(0);
        let keep = crate::source::TimeWindow {
            start_utc: epg::retention_cutoff(now, days),
            end_utc: now + 7 * 86_400,
        };
        // Programmes written so far after each batch (total unknown: 0).
        let r = crate::xmltv::ingest::ingest_with_progress(
            &self.store,
            source_id,
            file.path().to_path_buf(),
            tz,
            keep,
            &|n| self.progress(source_id, "guide", n, 0),
        )
        .await;
        let r = r.and_then(|rep| {
            if rep.truncated {
                Err(AppError::server(
                    "The guide ended before it finished loading.",
                ))
            } else {
                Ok(rep)
            }
        });
        match r {
            Ok(rep) => {
                if rep.capped > 0 {
                    tracing::warn!(
                        capped = rep.capped,
                        "guide is larger than the stored-guide limit; the rest was skipped"
                    );
                }
                tracing::info!(
                    programmes = rep.programmes,
                    by_name = rep.matches.by_name,
                    unmatched = rep.matches.unmatched.len(),
                    "guide ingested"
                );
                self.store.set_sync_ok(source_id, "guide", now).await?;
                Ok(())
            }
            Err(e) => {
                self.store
                    .set_sync_error(source_id, "guide", &e.message)
                    .await?;
                Err(e)
            }
        }
    }

    /// Windowed guide (Dispatcharr): the visible window first, then history back to the longest
    /// `catchup_days` (so catch-up from the guide has programme rows to play), then the days ahead
    /// to `AHEAD_DAYS` so the guide can be browsed beyond tonight. History and ahead are best
    /// effort: the visible guide is already in place.
    async fn sync_windowed_guide(
        &self,
        source_id: i64,
        client: &dyn SourceClient,
        now: i64,
    ) -> Result<()> {
        if client.kind() != SourceKind::Dispatcharr {
            return Ok(());
        }
        epg::sync_guide(client, &self.store, source_id, epg::visible_window(now)).await?;
        // The visible window is in: show it before the longer fills.
        self.emit(Event::CatalogUpdated(
            self.counts(source_id, CatalogChange::Guide).await?,
        ));
        let days = refresh::max_catchup_days(&self.store, source_id).await?;
        let row = self.store.source(source_id).await?;
        let plan = refresh::plan_windows(now, days, refresh::parse_tz(row.server_tz.as_deref()));
        let cancel = AtomicBool::new(false);
        for (kind, phase) in [
            (refresh::WindowKind::CatchupHistory, "history"),
            // Shown to the user as "Syncing guide…".
            (refresh::WindowKind::Ahead, "guide"),
        ] {
            if let Err(e) = refresh::run_plan(
                client,
                &self.store,
                source_id,
                &plan,
                &[kind],
                &cancel,
                |d, t| self.progress(source_id, phase, d, t),
            )
            .await
            {
                tracing::warn!(error = %e, phase, "guide fill failed");
            }
        }
        Ok(())
    }

    /// Light guide refresh between full syncs: re-fetch only the visible window (now - 2 h ..
    /// now + 12 h) of windowed (Dispatcharr) guides whose guide was last fetched more than
    /// [`VISIBLE_GUIDE_REFRESH_SECS`] ago, so late schedule changes reach now/next. Guide endpoints
    /// only (never a stream); runs during playback too. Sources being synced are skipped.
    pub async fn refresh_visible_guides(&self) -> Result<()> {
        let now = now_utc();
        for r in self.store.list_sources().await? {
            let connected = matches!(
                self.status.lock().ok().and_then(|g| g.get(&r.id).copied()),
                Some(SourceStatus::Connected)
            );
            if !connected {
                continue;
            }
            let last_ok = self
                .store
                .sync_state(r.id, "guide")
                .await?
                .and_then(|(ok, _)| ok);
            if last_ok.is_some_and(|t| now - t < VISIBLE_GUIDE_REFRESH_SECS) {
                continue;
            }
            let lock = self
                .sync_locks
                .lock()
                .map_err(|_| AppError::local("Sync state is unavailable."))?
                .entry(r.id)
                .or_default()
                .clone();
            let Ok(_guard) = lock.try_lock() else {
                continue; // a full sync is running and refreshes the guide itself
            };
            let client = match self.client(r.id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(source_id = r.id, error = %e, "guide refresh skipped");
                    continue;
                }
            };
            if client.kind() != SourceKind::Dispatcharr {
                continue; // whole-feed guides are refreshed by the full sync
            }
            match epg::sync_guide(client.as_ref(), &self.store, r.id, epg::visible_window(now))
                .await
            {
                Ok(c) => {
                    tracing::info!(
                        source_id = r.id,
                        programmes = c.programmes,
                        "guide refreshed"
                    );
                    self.emit(Event::CatalogUpdated(
                        self.counts(r.id, CatalogChange::Guide).await?,
                    ));
                    self.rebuild_sports_logged().await;
                }
                Err(e) => tracing::warn!(source_id = r.id, error = %e, "guide refresh failed"),
            }
        }
        Ok(())
    }

    /// Background guide fill: catch-up history and the days ahead (cancellable).
    pub async fn refresh_guide(
        &self,
        source_id: i64,
        kinds: &[refresh::WindowKind],
        cancel: &AtomicBool,
    ) -> Result<refresh::RefreshReport> {
        let client = self.client(source_id).await?;
        let row = self.store.source(source_id).await?;
        let days = refresh::max_catchup_days(&self.store, source_id).await?;
        let plan =
            refresh::plan_windows(now_utc(), days, refresh::parse_tz(row.server_tz.as_deref()));
        let r = refresh::run_plan(
            client.as_ref(),
            &self.store,
            source_id,
            &plan,
            kinds,
            cancel,
            |d, t| {
                self.progress(source_id, "guide", d, t);
            },
        )
        .await?;
        self.emit(Event::CatalogUpdated(
            self.counts(source_id, CatalogChange::Guide).await?,
        ));
        self.rebuild_sports_logged().await;
        Ok(r)
    }

    /// Best-effort favourites/recents/resume sync with the server.
    pub async fn sync_user_state(&self, source_id: i64) -> Result<userstate::StateSyncReport> {
        let client = self.client(source_id).await?;
        userstate::sync_state(&self.store, client.as_ref(), now_utc()).await
    }

    // ---- content filters / settings ---------------------------------------------------------

    async fn filter(&self) -> Result<ChannelFilter> {
        channel_filter(&self.store).await
    }

    /// Show/hide adult and radio channels (enforced in `catalog/`).
    pub async fn set_content_filters(
        &self,
        include_adult: bool,
        include_radio: bool,
    ) -> Result<()> {
        let now = now_utc();
        self.store
            .kv_set(PREF_ADULT, if include_adult { "1" } else { "0" }, now)
            .await?;
        self.store
            .kv_set(PREF_RADIO, if include_radio { "1" } else { "0" }, now)
            .await?;
        // Matches only cover visible channels.
        self.rebuild_sports_logged().await;
        Ok(())
    }

    fn default_settings() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("liveBufferSecs".into(), 3.into());
        m.insert("hwdec".into(), "auto-safe".into());
        m.insert("debugLogging".into(), false.into());
        m.insert(
            SET_AUTO_REFRESH_HOURS.into(),
            DEFAULT_AUTO_REFRESH_HOURS.into(),
        );
        m.insert(SET_REFRESH_DURING_PLAYBACK.into(), true.into());
        m
    }

    /// Machine-local settings (non-secret), persisted in SQLite. Defaults cover the fields core
    /// knows; the UI owns the rest and they round-trip verbatim.
    pub async fn get_settings(&self) -> Result<SettingsDto> {
        let mut m = Self::default_settings();
        if let Some((v, _)) = self.store.kv_get(SETTINGS_KEY).await?
            && let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(&v)
        {
            m.extend(saved);
        }
        Ok(Value::Object(m))
    }

    pub async fn set_settings(&self, patch: Value) -> Result<SettingsDto> {
        let Value::Object(patch) = patch else {
            return Err(AppError::local("Settings must be an object."));
        };
        let mut cur = match self.get_settings().await? {
            Value::Object(m) => m,
            _ => Self::default_settings(),
        };
        for (k, v) in patch {
            if k == "liveBufferSecs" {
                let n = v
                    .as_f64()
                    .ok_or_else(|| AppError::local("Live buffer must be a number."))?;
                cur.insert(k, Value::from(n.round().clamp(3.0, 10.0) as i64));
            } else if k == SET_AUTO_REFRESH_HOURS {
                let n = v
                    .as_f64()
                    .ok_or_else(|| AppError::local("Refresh interval must be a number."))?;
                cur.insert(
                    k,
                    Value::from(n.round().clamp(0.0, MAX_AUTO_REFRESH_HOURS as f64) as i64),
                );
            } else {
                cur.insert(k, v);
            }
        }
        let out = Value::Object(cur);
        self.store
            .kv_set(SETTINGS_KEY, &serde_json::to_string(&out)?, now_utc())
            .await?;
        let prefs = RefreshPrefs::from_settings(&out);
        let was = self
            .auto_refresh_secs
            .swap(prefs.interval_secs, std::sync::atomic::Ordering::Relaxed);
        if (was > 0) != (prefs.interval_secs > 0) {
            // Turning auto refresh on/off changes whether old data shows as stale.
            self.emit(Event::CatalogUpdated(CatalogUpdated {
                scope: CatalogChange::Status,
                source_id: 0,
                channels: 0,
                groups: 0,
                programmes: 0,
            }));
        }
        Ok(out)
    }

    // ---- catalogue --------------------------------------------------------------------------

    pub async fn list_groups(&self, source_id: Option<i64>) -> Result<Vec<GroupDto>> {
        let f = ChannelFilter {
            source_id,
            ..self.filter().await?
        };
        Ok(catalog::list_groups(&self.store, &f)
            .await?
            .into_iter()
            .map(|g| GroupDto {
                id: g.id,
                source_id: g.source_id,
                name: g.name,
                position: g.position,
                channel_count: g.channel_count,
                count: g.channel_count,
            })
            .collect())
    }

    async fn logo_for(
        &self,
        sources: &HashMap<i64, (String, Option<url::Url>)>,
        c: &crate::store::ChannelRow,
    ) -> Option<String> {
        let logo_id = c.logo_key.as_deref()?;
        let (source_uuid, base) = sources.get(&c.source_id)?;
        // M3U/Xtream logos are absolute public URLs; Dispatcharr's are ids on its own server.
        let url = if logo_id.starts_with("http://") || logo_id.starts_with("https://") {
            logo_id.to_owned()
        } else {
            logo_url(base.as_ref()?, logo_id).ok()?
        };
        let hash = crate::images::account_hash(source_uuid, &url);
        let known = self
            .registered_logos
            .lock()
            .ok()
            .is_some_and(|g| g.contains(&hash));
        if known {
            // Already in image_origin: no write on the read path.
            return Some(crate::images::dlimg_url(&hash, ImageSize::Grid));
        }
        let dl = self
            .images
            .register_scoped(c.source_id, source_uuid, &url, ImageSize::Grid)
            .await
            .ok()?;
        if !known && let Ok(mut g) = self.registered_logos.lock() {
            g.insert(hash);
        }
        Some(dl)
    }

    async fn to_channel_dtos(&self, rows: &[crate::store::ChannelRow]) -> Result<Vec<ChannelDto>> {
        let favs: HashSet<i64> = self
            .store
            .favourite_channel_ids(None)
            .await?
            .into_iter()
            .collect();
        let sources = self
            .store
            .list_sources()
            .await?
            .into_iter()
            .map(|s| {
                let base = s
                    .base_url
                    .as_deref()
                    .and_then(|base| normalise_base_url(base).ok());
                (s.id, (s.uuid, base))
            })
            .collect();
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let logo = self.logo_for(&sources, r).await;
            out.push(ChannelDto::from_row(r, logo, favs.contains(&r.id)));
        }
        Ok(out)
    }

    pub async fn list_channels(&self, a: ListChannelsArgs) -> Result<Vec<ChannelDto>> {
        let f = ChannelFilter {
            source_id: a.source_id,
            group_id: a.group_id,
            search: a.query,
            favourites_only: a.favourites_only,
            ..self.filter().await?
        };
        let rows = catalog::list_channels(
            &self.store,
            &f,
            a.limit.unwrap_or(500).clamp(1, 5000),
            a.offset.unwrap_or(0).max(0),
        )
        .await?;
        self.to_channel_dtos(&rows).await
    }

    pub async fn get_now_next(&self, channel_ids: &[i64]) -> Result<Vec<NowNextDto>> {
        let m = epg::now_next_many(&self.store, channel_ids, now_utc()).await?;
        // Collect every programme once, flag the ones being recorded, then rebuild the pairs.
        let mut flat: Vec<ProgrammeDto> = Vec::new();
        for id in channel_ids {
            if let Some(nn) = m.get(id) {
                flat.extend(nn.now.iter().chain(nn.next.iter()).map(ProgrammeDto::from));
            }
        }
        self.mark_scheduled(&mut flat).await?;
        let marked: HashMap<(i64, i64), ProgrammeDto> = flat
            .into_iter()
            .map(|p| ((p.channel_id, p.start_utc), p))
            .collect();
        let pick = |p: &Option<crate::store::ProgrammeRow>| {
            p.as_ref()
                .and_then(|p| marked.get(&(p.channel_id, p.start_utc)).cloned())
        };
        Ok(channel_ids
            .iter()
            .map(|id| {
                let nn = m.get(id).cloned().unwrap_or_default();
                NowNextDto {
                    channel_id: *id,
                    now: pick(&nn.now),
                    next: pick(&nn.next),
                }
            })
            .collect())
    }

    /// A page of channels plus their programmes in `[start, end)`.
    pub async fn get_guide(&self, a: GuideArgs) -> Result<GuideDto> {
        let f = ChannelFilter {
            group_id: a.group_id,
            ..self.filter().await?
        };
        let rows = catalog::list_channels(
            &self.store,
            &f,
            a.channel_limit.unwrap_or(50).clamp(1, 500),
            a.channel_offset.max(0),
        )
        .await?;
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        Ok(GuideDto {
            channels: self.to_channel_dtos(&rows).await?,
            programmes: self.get_programme_window(&ids, a.start, a.end).await?,
        })
    }

    pub async fn search(&self, query: &str, limit: i64) -> Result<SearchDto> {
        let limit = limit.clamp(1, 200);
        let f = self.filter().await?;
        let chan_filter = ChannelFilter {
            search: Some(query.to_owned()),
            ..f.clone()
        };
        // Both legs are independent reads: run them concurrently on the pool.
        let (rows, progs) = tokio::join!(
            catalog::list_channels(&self.store, &chan_filter, limit, 0),
            catalog::search_programmes(&self.store, &f, query, now_utc(), limit),
        );
        let (rows, progs) = (rows?, progs?);
        // Each hit carries its full channel DTO (logo included) so the UI needs no second call.
        let ids: Vec<i64> = {
            let mut seen = HashSet::new();
            progs
                .iter()
                .map(|p| p.channel_id)
                .filter(|i| seen.insert(*i))
                .collect()
        };
        let hit_rows = catalog::channels_by_ids(&self.store, &f, &ids).await?;
        // One DTO pass (favourites, sources, logos) for both legs.
        let mut all_rows = rows.clone();
        let have: HashSet<i64> = rows.iter().map(|r| r.id).collect();
        all_rows.extend(hit_rows.into_iter().filter(|r| !have.contains(&r.id)));
        let mut dtos = self.to_channel_dtos(&all_rows).await?;
        let extra = dtos.split_off(rows.len());
        let channels = dtos;
        let chan_dtos: HashMap<i64, ChannelDto> = channels
            .iter()
            .cloned()
            .chain(extra)
            .map(|c| (c.id, c))
            .collect();
        let mut programmes: Vec<ProgrammeHitDto> = progs
            .iter()
            .filter_map(|p| {
                Some(ProgrammeHitDto {
                    programme: p.into(),
                    channel: chan_dtos.get(&p.channel_id)?.clone(),
                })
            })
            .collect();
        let mut flat: Vec<ProgrammeDto> = programmes.iter().map(|h| h.programme.clone()).collect();
        self.mark_scheduled(&mut flat).await?;
        for (h, p) in programmes.iter_mut().zip(flat) {
            h.programme = p;
        }
        Ok(SearchDto {
            channels,
            programmes,
        })
    }

    /// One channel as the UI sees it (logo `dlimg://` URL, favourite flag).
    pub async fn get_channel(&self, channel_id: i64) -> Result<ChannelDto> {
        let row = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        self.to_channel_dtos(std::slice::from_ref(&row))
            .await?
            .pop()
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))
    }

    pub async fn find_channel_number(&self, number: i64) -> Result<Option<i64>> {
        let filter = self.filter().await?;
        Ok(
            catalog::channel_by_number(&self.store, &filter, number as f64)
                .await?
                .map(|row| row.id),
        )
    }

    /// The channels before and after `channel_id` in list order (channel number), wrapping at the ends,
    /// under the user's catalog filters. Player zapping (PgUp/PgDn, ↑/↓). Catalog-only: nothing plays.
    pub async fn channel_neighbours(&self, channel_id: i64) -> Result<ChannelNeighboursDto> {
        let f = self.filter().await?;
        let rows = catalog::list_channels(&self.store, &f, 100_000, 0).await?;
        let Some(i) = rows.iter().position(|r| r.id == channel_id) else {
            return Ok(ChannelNeighboursDto {
                prev: None,
                next: None,
            });
        };
        let n = rows.len();
        if n < 2 {
            return Ok(ChannelNeighboursDto {
                prev: None,
                next: None,
            });
        }
        let pick = [rows[(i + n - 1) % n].clone(), rows[(i + 1) % n].clone()];
        let mut dtos = self.to_channel_dtos(&pick).await?.into_iter();
        Ok(ChannelNeighboursDto {
            prev: dtos.next(),
            next: dtos.next(),
        })
    }

    /// Channel totals under the user's catalog filters (adult/radio/DRM hiding).
    pub async fn get_counts(&self) -> Result<CountsDto> {
        let f = self.filter().await?;
        let all = catalog::count_channels(&self.store, &f).await?;
        let favourites = catalog::count_channels(
            &self.store,
            &ChannelFilter {
                favourites_only: true,
                ..f
            },
        )
        .await?;
        Ok(CountsDto { all, favourites })
    }

    /// Programmes of one channel overlapping `[start, end)`.
    pub async fn get_programmes(
        &self,
        channel_id: i64,
        start: i64,
        end: i64,
    ) -> Result<Vec<ProgrammeDto>> {
        self.get_programme_window(&[channel_id], start, end).await
    }

    /// Programmes for a page of channel IDs already loaded by the guide.
    pub async fn get_programme_window(
        &self,
        channel_ids: &[i64],
        start: i64,
        end: i64,
    ) -> Result<Vec<ProgrammeDto>> {
        let rows = epg::window(
            &self.store,
            channel_ids,
            crate::source::TimeWindow {
                start_utc: start,
                end_utc: end,
            },
        )
        .await?;
        let mut out: Vec<ProgrammeDto> = rows.iter().map(Into::into).collect();
        self.mark_scheduled(&mut out).await?;
        Ok(out)
    }

    async fn programme_dto(&self, channel_id: i64, start_utc: i64) -> Result<Option<ProgrammeDto>> {
        let row: Option<crate::store::ProgrammeRow> = sqlx::query_as(
            "SELECT channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags, icon
             FROM programme WHERE channel_id = ? AND start_utc = ?",
        )
        .bind(channel_id)
        .bind(start_utc)
        .fetch_optional(self.store.pool())
        .await?;
        Ok(row.as_ref().map(Into::into))
    }

    /// Home screen: continue watching, favourites, what is on now per favourite, recents. Every
    /// item carries full channel/programme DTOs (logos included).
    pub async fn get_home(&self) -> Result<HomeDto> {
        let f = self.filter().await?;
        let fav_rows = catalog::list_channels(
            &self.store,
            &ChannelFilter {
                favourites_only: true,
                ..f.clone()
            },
            50,
            0,
        )
        .await?;
        let recent_ids: Vec<i64> = self
            .store
            .recents(20)
            .await?
            .into_iter()
            .map(|r| r.channel_id)
            .collect();
        let recent_rows = catalog::channels_by_ids(&self.store, &f, &recent_ids).await?;
        let favourites = self.to_channel_dtos(&fav_rows).await?;
        let recents = self.to_channel_dtos(&recent_rows).await?;

        let now = now_utc();
        let fav_ids: Vec<i64> = favourites.iter().map(|c| c.id).collect();
        let nn = epg::now_next_many(&self.store, &fav_ids, now).await?;
        let on_now = favourites
            .iter()
            .map(|c| OnNowItemDto {
                channel: c.clone(),
                now: nn.get(&c.id).and_then(|n| n.now.as_ref()).map(Into::into),
                next: nn.get(&c.id).and_then(|n| n.next.as_ref()).map(Into::into),
            })
            .collect();

        let mut continue_watching = Vec::new();
        // The channel last watched live comes first.
        if let Some(c) = recents.first() {
            let p = epg::now_next(&self.store, c.id, now)
                .await?
                .now
                .as_ref()
                .map(ProgrammeDto::from);
            continue_watching.push(ContinueItemDto {
                kind: "live".into(),
                id: format!("live:{}", c.id),
                channel: Some(c.clone()),
                programme: p.clone(),
                title: p
                    .as_ref()
                    .and_then(|p| p.title.clone())
                    .unwrap_or_else(|| c.name.clone()),
                subtitle: Some(c.name.clone()),
                art: c.logo.clone(),
                progress: None,
                remaining_secs: None,
            });
        }
        for r in self.store.continue_watching(20).await? {
            if let Some(item) = self.continue_item(&f, &r).await? {
                continue_watching.push(item);
            }
        }
        Ok(HomeDto {
            continue_watching,
            favourites,
            on_now,
            recents,
        })
    }

    async fn continue_item_vod(
        &self,
        r: &crate::store::ResumeRow,
        progress: Option<f64>,
        remaining: Option<i64>,
    ) -> Result<Option<ContinueItemDto>> {
        self.continue_item_vod_impl(r, progress, remaining).await
    }

    async fn continue_item(
        &self,
        f: &ChannelFilter,
        r: &crate::store::ResumeRow,
    ) -> Result<Option<ContinueItemDto>> {
        let progress = r
            .duration_ms
            .filter(|d| *d > 0)
            .map(|d| (r.position_ms as f64 / d as f64).clamp(0.0, 1.0));
        let remaining = r
            .duration_ms
            .filter(|d| *d > 0)
            .map(|d| ((d - r.position_ms).max(0)) / 1000);
        match r.kind.as_str() {
            "catchup" => {
                let Some((ext, start)) = r.item_key.split_once(':') else {
                    return Ok(None);
                };
                let Ok(start) = start.parse::<i64>() else {
                    return Ok(None);
                };
                let Some(ch) = self.store.channel_by_ext(r.source_id, ext).await? else {
                    return Ok(None);
                };
                let rows = catalog::channels_by_ids(&self.store, f, &[ch.id]).await?;
                let Some(cd) = self.to_channel_dtos(&rows).await?.pop() else {
                    return Ok(None);
                };
                let prog = self.programme_dto(ch.id, start).await?;
                Ok(Some(ContinueItemDto {
                    kind: "catchup".into(),
                    id: format!("catchup:{}:{start}", ch.id),
                    title: prog
                        .as_ref()
                        .and_then(|p| p.title.clone())
                        .unwrap_or_else(|| cd.name.clone()),
                    subtitle: Some(cd.name.clone()),
                    art: cd.logo.clone(),
                    channel: Some(cd),
                    programme: prog,
                    progress,
                    remaining_secs: remaining,
                }))
            }
            _ => self.continue_item_vod(r, progress, remaining).await,
        }
    }

    pub async fn set_favourite(&self, channel_id: i64, favourite: bool) -> Result<()> {
        self.store
            .set_favourite(channel_id, favourite, now_utc())
            .await
    }

    /// Save a watch position (catch-up key: `catchup::resume_key`).
    pub async fn set_watch_position(
        &self,
        source_id: i64,
        kind: &str,
        item_key: &str,
        position_ms: i64,
        duration_ms: Option<i64>,
    ) -> Result<()> {
        self.store
            .set_resume(
                source_id,
                kind,
                item_key,
                position_ms,
                duration_ms,
                now_utc(),
            )
            .await
    }

    // ---- playback ---------------------------------------------------------------------------

    /// Live play plan (from SQLite; redirect/unknown channels make the one allowed play-time
    /// request). Records a recent. The caller hands the plan to the player.
    pub async fn play_channel(&self, channel_id: i64) -> Result<PlayPlan> {
        let plan = self.live_plan(channel_id).await?;
        if let Ok(mut g) = self.hls_reminted.lock() {
            g.remove(&channel_id);
        }
        if plan.session.is_none() {
            // Zapped from an HLS channel to a normal one: release the old session.
            self.stop_live_hls().await;
        }
        if let Ok(mut g) = self.current_stream.lock() {
            g.insert(channel_id, 0);
        }
        self.store.touch_recent(channel_id, now_utc()).await?;
        Ok(plan)
    }

    /// Mint a live HLS session at play time (`GET /proxy/ts/stream/{uuid}?output_format=hls`, one
    /// request, API key, no redirects followed). The resulting URL has no headers: the token in
    /// the path is the credential. The token is persisted in `open_session` before the plan is
    /// returned (crash cleanup), and the previous live session, if any, is released.
    async fn mint_hls_plan(
        &self,
        ch: &crate::store::ChannelRow,
        src: &SourceRow,
    ) -> Result<PlayPlan> {
        use crate::redact::RedactionSet;
        use crate::source::{PlayProfile, SessionId, Surface};
        let uuid = ch
            .uuid
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                AppError::stream(
                    "This channel has no stream address yet; refresh the channel list.",
                )
            })?;
        self.client(src.id).await?; // builds and caches the typed client
        let c = self.dispatcharr.read().await.get(&src.id).cloned();
        let c = c.ok_or_else(|| AppError::local("This source is not available."))?;
        let (url, token) = c.mint_hls(uuid).await?;
        sessions::track(&self.store, src.id, sessions::KIND_HLS, &token).await?;
        let previous = self
            .live_hls
            .lock()
            .ok()
            .and_then(|mut g| g.replace((src.id, token.clone())));
        if let Some((sid, old)) = previous
            && old != token
        {
            let _ = sessions::release(&self.store, self, sid, sessions::KIND_HLS, &old).await;
        }
        let mut redact = RedactionSet::new();
        redact.add(&token);
        Ok(PlayPlan {
            tls_pin: None,
            url,
            headers: Vec::new(),
            surface: Surface::Hls,
            profile: PlayProfile::Live,
            session: Some(SessionId(token)),
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    /// Release the live HLS session, if one is open (navigate-away, stop, zap to a non-HLS channel).
    /// Failures stay in `open_session` and are retried at the next start.
    pub async fn stop_live_hls(&self) {
        let cur = self.live_hls.lock().ok().and_then(|mut g| g.take());
        if let Some((sid, token)) = cur {
            let _ = sessions::release(&self.store, self, sid, sessions::KIND_HLS, &token).await;
        }
    }

    /// The player got `410` on a live HLS playlist or segment: the session ended on the server.
    /// Mint a fresh one, at most once per `play_channel`; a second call is an error so a dying
    /// stream cannot loop.
    pub async fn remint_hls(&self, channel_id: i64) -> Result<PlayPlan> {
        let first = self
            .hls_reminted
            .lock()
            .map(|mut g| g.insert(channel_id))
            .unwrap_or(false);
        if !first {
            return Err(AppError::stream(
                "The live stream ended and could not be restarted.",
            ));
        }
        self.live_plan(channel_id).await
    }

    /// Change per-source options (currently the opt-in HLS transport). Only sources that support
    /// the option accept it. Takes effect at the next play.
    pub async fn set_source_options(&self, args: SetSourceOptionsArgs) -> Result<SourceDto> {
        let row = self.store.source(args.source_id).await?;
        if row.kind != "dispatcharr" {
            return Err(AppError::local(
                "This source does not offer an HLS transport option.",
            ));
        }
        if !args.hls_transport {
            self.stop_live_hls().await;
        }
        self.store
            .set_source_hls_transport(args.source_id, args.hls_transport)
            .await?;
        let row = self.store.source(args.source_id).await?;
        let days = refresh::max_catchup_days(&self.store, row.id).await?;
        Ok(SourceDto::from_row(&row, self.status_of(&row), days))
    }

    async fn live_plan(&self, channel_id: i64) -> Result<PlayPlan> {
        self.live_plan_with(channel_id, true).await
    }

    /// Play plan for a multiview tile (PLAN §4.11). Always the TS path: the opt-in live HLS
    /// transport keeps one session for the main player, and a tile must never replace it.
    pub async fn tile_plan(&self, channel_id: i64) -> Result<PlayPlan> {
        let plan = self.live_plan_with(channel_id, false).await?;
        self.store.touch_recent(channel_id, now_utc()).await?;
        Ok(plan)
    }

    async fn live_plan_with(&self, channel_id: i64, allow_hls: bool) -> Result<PlayPlan> {
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let src = self.store.source(ch.source_id).await?;
        if SourceKind::parse(&src.kind) == Some(SourceKind::Dispatcharr) {
            // Opt-in HLS transport: only for channels the server proxies (a Redirect-profile channel
            // is answered before the output format is considered, so it keeps its normal path).
            if allow_hls
                && src.hls_transport
                && ch.stream_redirect == crate::source::StreamMode::Proxy
            {
                return self.mint_hls_plan(&ch, &src).await;
            }
            let mut plan = catalog::resolve_play_plan(&self.store, self, channel_id).await?;
            let pin = self.source_pin(&src)?;
            plan.tls_pin = pin.clone();
            plan.resolve_at_play(pin.as_ref()).await
        } else {
            if ch.is_drm {
                return Err(AppError::media("DRM — not supported"));
            }
            self.client(ch.source_id)
                .await?
                .play(ChannelKey::from_row(&ch))
                .await
        }
    }

    /// The channel's provider streams in failover order (Dispatcharr: names from the server, no
    /// provider URLs). Other sources report a single stream.
    pub async fn channel_streams(&self, channel_id: i64) -> Result<StreamListDto> {
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let client = self.client(ch.source_id).await?;
        let mut streams: Vec<AltStreamDto> = client
            .channel_stream_info(&ch.ext_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, s)| AltStreamDto {
                index,
                stream_id: s.id,
                name: s.name,
                provider: s.provider,
                resolution: s.resolution,
                fps: s.fps,
                video_codec: s.video_codec,
                audio_codec: s.audio_codec,
                audio_channels: s.audio_channels,
                bitrate_kbps: s.bitrate_kbps,
                viewers: s.viewers,
                stale: s.stale,
            })
            .collect();
        if streams.is_empty() {
            // Fall back to the failover ids stored at sync (names unknown), or a single entry.
            let ids = self.store.channel_stream_ids(channel_id).await?;
            streams = if ids.is_empty() {
                vec![AltStreamDto::bare(0, ch.ext_id.clone(), ch.name.clone())]
            } else {
                ids.into_iter()
                    .enumerate()
                    .map(|(index, stream_id)| {
                        AltStreamDto::bare(index, stream_id, format!("Stream {}", index + 1))
                    })
                    .collect()
            };
        }
        let cur = self
            .current_stream
            .lock()
            .ok()
            .and_then(|g| g.get(&channel_id).copied());
        Ok(StreamListDto {
            channel_id,
            source_index: cur.map(|c| c.min(streams.len().saturating_sub(1)) + 1),
            source_count: streams.len(),
            streams,
        })
    }

    /// Stream picker: switch the live channel to the stream at `index` (0-based, failover order).
    /// Moves the server's shared channel session (PLAN §3.4, D18). Returns `(1-based index, count)`.
    pub async fn switch_stream(&self, channel_id: i64, index: usize) -> Result<(usize, usize)> {
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let src = self.store.source(ch.source_id).await?;
        if !crate::source::Caps::from_db(src.caps).contains(crate::source::Caps::STREAM_SWITCH) {
            return Err(AppError::local("This account can't switch streams."));
        }
        let list = self.channel_streams(channel_id).await?;
        let target = list
            .streams
            .get(index)
            .ok_or_else(|| AppError::local("That stream is no longer available."))?;
        let client = self.client(ch.source_id).await?;
        let switched = client
            .change_stream(&ChannelKey::from_row(&ch), &target.stream_id)
            .await?;
        if !switched {
            return Err(AppError::stream(
                "This channel can't switch streams while it plays this way.",
            ));
        }
        if let Ok(mut g) = self.current_stream.lock() {
            g.insert(channel_id, index);
        }
        Ok((index + 1, list.source_count))
    }

    /// `(1-based index in use, count)` for PlayerState, from local data only.
    pub async fn source_position(&self, channel_id: i64) -> Result<(usize, usize)> {
        let count = self
            .store
            .channel_stream_ids(channel_id)
            .await?
            .len()
            .max(1);
        let cur = self
            .current_stream
            .lock()
            .ok()
            .and_then(|g| g.get(&channel_id).copied())
            .unwrap_or(0);
        Ok((cur.min(count - 1) + 1, count))
    }

    /// `(1-based index in use, count)` for a channel that is playing now, from the server: the
    /// live stream list gives the count and the status endpoint the stream in use. Falls back to
    /// the locally tracked position when the server can't say. Never touches a stream endpoint.
    pub async fn live_source_position(&self, channel_id: i64) -> Result<(usize, usize)> {
        let (local_idx, local_count) = self.source_position(channel_id).await?;
        let Some(ch) = self.store.channel(channel_id).await? else {
            return Ok((local_idx, local_count));
        };
        let client = self.client(ch.source_id).await?;
        let streams = client
            .channel_stream_info(&ch.ext_id)
            .await
            .unwrap_or_default();
        let count = if streams.is_empty() {
            local_count
        } else {
            streams.len()
        };
        let active = client
            .active_stream_id(&ChannelKey::from_row(&ch))
            .await
            .ok()
            .flatten();
        let idx0 = active
            .and_then(|id| streams.iter().position(|s| s.id == id))
            .unwrap_or(local_idx - 1)
            .min(count - 1);
        if let Ok(mut g) = self.current_stream.lock() {
            g.insert(channel_id, idx0);
        }
        Ok((idx0 + 1, count))
    }

    /// Next step after a playback failure (PLAN §4.5). `attempt` is the number of retries already
    /// made, starting at 1 for the first retry:
    /// 1. reconnect the same URL (no server change);
    /// 2. and 3. ask the server for the channel's next provider stream (Dispatcharr), then a fresh
    ///    plan for the player;
    ///
    /// Beyond that a plain sentence: never loop forever.
    pub async fn failover_next(&self, channel_id: i64, attempt: u32) -> Result<FailoverStep> {
        if attempt == 0 || attempt > 3 {
            return Err(AppError::stream(
                "No working stream was found for this channel.",
            ));
        }
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let (_, count) = self.source_position(channel_id).await?;
        let mut switched = false;
        if attempt >= 2 && count > 1 {
            let client = self.client(ch.source_id).await?;
            switched = client
                .next_stream(&ChannelKey::from_row(&ch))
                .await
                .unwrap_or(false);
            if switched && let Ok(mut g) = self.current_stream.lock() {
                let e = g.entry(channel_id).or_insert(0);
                *e = (*e + 1) % count;
            }
        }
        let plan = self.live_plan(channel_id).await?;
        let (source_index, source_count) = self.source_position(channel_id).await?;
        Ok(FailoverStep {
            plan,
            source_index,
            source_count,
            switched,
        })
    }

    /// Archive playback of a programme (`ProgrammeDto::id`). Times come from the programme row.
    pub async fn play_catchup(&self, programme_id: &str) -> Result<CatchupPlay> {
        let (channel_id, start) = parse_programme_id(programme_id)
            .ok_or_else(|| AppError::local("That programme is not valid."))?;
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let client = self.client(ch.source_id).await?;
        let play =
            catchup::play_catchup(&self.store, client.as_ref(), channel_id, start, now_utc())
                .await?;
        self.store.touch_recent(channel_id, now_utc()).await?;
        Ok(play)
    }

    /// Restart the programme airing now from its start.
    pub async fn restart_programme(&self, channel_id: i64) -> Result<CatchupPlay> {
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let client = self.client(ch.source_id).await?;
        catchup::restart_programme(&self.store, client.as_ref(), channel_id, now_utc()).await
    }

    /// Leave archive playback and return to live: stops catch-up sessions, then plays live.
    pub async fn go_live(&self, channel_id: i64) -> Result<PlayPlan> {
        let _ = sessions::stop_kind(&self.store, self, sessions::KIND_CATCHUP).await;
        self.play_channel(channel_id).await
    }

    pub async fn stop_catchup(&self, play: &CatchupPlay) -> Result<()> {
        catchup::stop(&self.store, self, play).await
    }

    /// Throttled position report for a catch-up playback; also persists the resume position.
    pub async fn report_catchup_position(
        &self,
        play: &CatchupPlay,
        reporter: &mut PositionReporter,
        now_ms: u64,
        position_secs: u64,
        force: bool,
    ) -> Result<bool> {
        let client = self.client(play.source_id).await?;
        let ch = self.store.channel(play.channel_id).await?;
        if let Some(ch) = ch {
            self.store
                .set_resume(
                    play.source_id,
                    "catchup",
                    &catchup::resume_key(&ch.ext_id, play.start_utc),
                    i64::try_from(position_secs)
                        .unwrap_or(0)
                        .saturating_mul(1000),
                    Some((play.end_utc - play.start_utc) * 1000),
                    now_utc(),
                )
                .await?;
        }
        // 404 before playback starts is already tolerated inside the client.
        reporter
            .report(client.as_ref(), now_ms, position_secs, force)
            .await
    }

    // ---- session hygiene --------------------------------------------------------------------

    /// App start: delete sessions persisted by a crashed run.
    pub async fn startup_cleanup(&self) -> Result<sessions::CleanupReport> {
        if let Err(e) = self.prune_metadata().await {
            tracing::warn!("metadata cache prune failed: {}", e.message);
        }
        self.prune_guide().await;
        sessions::cleanup_on_startup(&self.store, self).await
    }

    /// Daily housekeeping from the background tick (startup does it too): old guide entries.
    pub async fn housekeeping(&self) -> Result<()> {
        let now = now_utc();
        let last: Option<i64> = self
            .store
            .kv_get(HOUSEKEEPING_KEY)
            .await?
            .and_then(|(v, _)| v.parse().ok());
        if last.is_none_or(|t| now - t >= HOUSEKEEPING_SECS) {
            self.prune_guide().await;
        }
        Ok(())
    }

    /// Drop programmes past each source's retention window. Windowed (Dispatcharr) guides only
    /// replace the windows they fetch, so without this their history would grow forever.
    async fn prune_guide(&self) {
        let now = now_utc();
        match epg::prune(&self.store, now).await {
            Ok(n) if n > 0 => tracing::info!(pruned = n, "old guide entries pruned"),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("guide prune failed: {}", e.message);
                return;
            }
        }
        if let Err(e) = self
            .store
            .kv_set(HOUSEKEEPING_KEY, &now.to_string(), now)
            .await
        {
            tracing::warn!("recording housekeeping time failed: {}", e.message);
        }
    }

    /// Sleep / quit / navigate-away: stop every open session.
    pub async fn stop_all_sessions(&self) -> Result<sessions::CleanupReport> {
        if let Ok(mut g) = self.live_hls.lock() {
            g.take();
        }
        sessions::stop_all(&self.store, self).await
    }

    // ---- images -----------------------------------------------------------------------------

    /// Serve a `dlimg://<hash>/<size>` URL (the custom protocol handler calls this).
    pub async fn image(&self, dlimg_url: &str) -> Result<ImageBytes> {
        let (hash, size) = parse_dlimg(dlimg_url)
            .ok_or_else(|| AppError::local("That image address is not valid."))?;
        self.images.get(&hash, size, &ServiceFetcher(self)).await
    }

    /// Re-download an image that [`Self::image`] served as `stale` (call it in the background).
    pub async fn refresh_image(&self, dlimg_url: &str) -> Result<()> {
        let (hash, _) = parse_dlimg(dlimg_url)
            .ok_or_else(|| AppError::local("That image address is not valid."))?;
        self.images.refresh(&hash, &ServiceFetcher(self)).await
    }

    // ---- diagnostics ------------------------------------------------------------------------

    pub async fn diagnostics(&self) -> Result<DiagnosticsData> {
        let mut versions: Vec<String> = self
            .store
            .list_sources()
            .await?
            .into_iter()
            .filter_map(|s| s.server_version)
            .collect();
        versions.sort();
        versions.dedup();
        Ok(DiagnosticsData {
            app_version: self.app_version.clone(),
            server_versions: versions,
            os: std::env::consts::OS.to_owned(),
            log_tail: self.log_tail.tail(200),
        })
    }
}

#[async_trait]
impl AuthProvider for Service {
    async fn credentials(&self, source: &SourceRow) -> Result<HashMap<String, String>> {
        self.client(source.id).await.map(|c| c.credentials())
    }

    async fn auth_header(&self, source: &SourceRow) -> Result<(String, SecretString)> {
        // Only Dispatcharr sources use auth headers; build the client if it is not live yet.
        self.client(source.id).await?;
        let c = self.dispatcharr.read().await.get(&source.id).cloned();
        c.ok_or_else(|| AppError::local("This source does not use header authentication."))?
            .auth_header()
            .await
    }
}

#[async_trait]
impl SessionStopper for Service {
    async fn stop(&self, source_id: i64, kind: &str, session_id: &str) -> Result<()> {
        let c = self.client(source_id).await?;
        SessionStopper::stop(c.as_ref(), source_id, kind, session_id).await
    }
}

struct ServiceFetcher<'a>(&'a Service);

#[async_trait]
impl ImageFetcher for ServiceFetcher<'_> {
    async fn fetch_for(&self, source_id: i64, source_uuid: &str, url: &str) -> Result<RawImage> {
        if is_tmdb_artwork(url) {
            return crate::images::fetch_public(url, None).await;
        }
        let client = self.0.client(source_id).await?;
        if self.0.store.source(source_id).await?.uuid != source_uuid {
            return Err(AppError::local("That image is no longer available."));
        }
        client.fetch_image(url).await
    }

    async fn fetch(&self, source_id: i64, url: &str) -> Result<RawImage> {
        if is_tmdb_artwork(url) {
            return crate::images::fetch_public(url, None).await;
        }
        self.0.client(source_id).await?.fetch_image(url).await
    }
}

fn is_tmdb_artwork(url: &str) -> bool {
    reqwest::Url::parse(url)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some(crate::tmdb::IMAGE_HOST))
}

/// The content filter from settings (adult/radio visibility). A free function so background
/// tasks holding only a `Store` can use it.
async fn channel_filter(store: &Store) -> Result<ChannelFilter> {
    let flag = |k: &'static str| {
        let store = store.clone();
        async move {
            store
                .kv_get(k)
                .await
                .map(|v| v.is_some_and(|(v, _)| v == "1"))
        }
    };
    Ok(ChannelFilter {
        include_adult: flag(PREF_ADULT).await?,
        include_radio: flag(PREF_RADIO).await?,
        ..Default::default()
    })
}
