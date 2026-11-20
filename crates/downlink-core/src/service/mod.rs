//! `Service`: the façade the Tauri layer holds in state (it does not depend on Tauri). Owns the
//! store, the secret store, the image cache and one live client per source, and exposes the
//! command-level verbs of PLAN §2.5. Returns camelCase serde DTOs matching
//! `apps/ui/src/api/types.ts`; events go out on a broadcast channel the app forwards.

pub mod dto;
mod logtail;

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
pub use logtail::LogTail;

const SETTINGS_KEY: &str = "settings";
const PREF_ADULT: &str = "pref.include_adult";
const PREF_RADIO: &str = "pref.include_radio";
/// Sources not synced for this long show as stale.
const STALE_AFTER_SECS: i64 = 6 * 3600;

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
    registered_logos: Mutex<HashSet<String>>,
    /// Failover position (0-based) believed to be in use, per channel row id.
    current_stream: Mutex<HashMap<i64, usize>>,
    log_tail: LogTail,
    app_version: String,
}

impl Service {
    pub async fn new(cfg: ServiceConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        let store = Store::open(cfg.data_dir.join("downlink.sqlite")).await?;
        Ok(Self::with_store(
            store,
            cfg.secrets,
            cfg.data_dir.join("images"),
            cfg.app_version,
        ))
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
            registered_logos: Mutex::new(HashSet::new()),
            current_stream: Mutex::new(HashMap::new()),
            log_tail: LogTail::new(),
            app_version,
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
        let row = self.store.source(source_id).await?;
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
                        insecure_tls: row.tls_insecure,
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
                Arc::new(XtreamClient::new(
                    SourceId(row.id),
                    base,
                    row.username.as_deref().unwrap_or_default(),
                    pw,
                    row.tls_insecure,
                )?)
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
                Arc::new(M3uSource::new(
                    SourceId(row.id),
                    location,
                    xmltv,
                    row.tls_insecure,
                )?)
            }
        };
        // Restore the credential vault so templated URLs resolve and re-sync keeps placeholders.
        if kind != SourceKind::Dispatcharr
            && let Some(j) = self.optional_secret(&row, SecretKind::Credentials)
            && let Ok(map) = serde_json::from_str(&j)
        {
            c.load_credentials(map);
        }
        self.clients.write().await.insert(source_id, c.clone());
        Ok(c)
    }

    /// Persist credentials the source extracted from URLs (rule 7: keychain, never SQLite).
    fn persist_credentials(&self, row_uuid: &str, client: &dyn SourceClient) -> Result<()> {
        let creds = client.credentials();
        if creds.is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string(&creds)?;
        self.secrets.set(
            &crate::auth::secret_ref(row_uuid, SecretKind::Credentials),
            &SecretString::new(json),
        )
    }

    /// An unsaved connection for `test_source` / `add_source`.
    fn probe_client(input: &AddSourceInput, id: i64) -> Result<Arc<dyn SourceClient>> {
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
                        insecure_tls: input.allow_self_signed,
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
                    input.allow_self_signed,
                )?))
            }
            Some(SourceKind::M3u) => Ok(Arc::new(Self::m3u_from_input(input, id)?)),
            None => Err(AppError::local("Unknown source type.")),
        }
    }

    fn m3u_from_input(input: &AddSourceInput, id: i64) -> Result<M3uSource> {
        let location = Location::parse(&input.url)?;
        let xmltv = input
            .xmltv_url
            .iter()
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty())
            .collect();
        M3uSource::new(SourceId(id), location, xmltv, input.allow_self_signed)
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
        let c = Self::probe_client(&input, 0)?;
        let info = c.connect().await?;
        let groups = c.groups().await?;
        let (channels, _) = all_channels(c.as_ref(), 500).await?;
        let programmes = match c.guide_feed().await {
            Ok(Some(feed)) => {
                let path = feed.file.path().to_path_buf();
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
            Err(_) => 0,
        };
        Ok(TestSourceResult {
            channels: channels.iter().filter(|c| !c.hidden).count() as i64,
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
        // Fail fast: nothing is persisted if the server, file or credentials are wrong.
        let m3u_probe = if kind == SourceKind::M3u {
            Some(Arc::new(Self::m3u_from_input(&input, 0)?))
        } else {
            None
        };
        let probe: Arc<dyn SourceClient> = match &m3u_probe {
            Some(m) => m.clone(),
            None => Self::probe_client(&input, 0)?,
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
        self.clients.write().await.remove(&source_id);
        self.dispatcharr.write().await.remove(&source_id);
        if let Ok(mut g) = self.status.lock() {
            g.remove(&source_id);
        }
        self.emit(Event::CatalogUpdated(CatalogUpdated {
            source_id,
            channels: 0,
            groups: 0,
            programmes: 0,
        }));
        Ok(())
    }

    async fn counts(&self, source_id: i64) -> Result<CatalogUpdated> {
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
    pub async fn sync_source(&self, source_id: i64) -> Result<()> {
        self.set_status(source_id, SourceStatus::Syncing);
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
        r
    }

    async fn sync_inner(&self, source_id: i64) -> Result<()> {
        let client = self.client(source_id).await?;
        self.progress(source_id, "connect", 0, 4);
        let info = sync::connect_and_record(client.as_ref(), &self.store, source_id).await?;
        self.progress(source_id, "channels", 1, 4);
        sync::sync_channels(client.as_ref(), &self.store, source_id, info.user_level).await?;
        let uuid = self.store.source(source_id).await?.uuid;
        self.persist_credentials(&uuid, client.as_ref())?;
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
        self.progress(source_id, "done", 4, 4);
        self.emit(Event::CatalogUpdated(self.counts(source_id).await?));
        Ok(())
    }

    /// Whole-feed guides (XMLTV) for M3U/Xtream; windowed grid for Dispatcharr.
    async fn sync_guide_for(&self, source_id: i64, client: &dyn SourceClient) -> Result<()> {
        let now = now_utc();
        match client.guide_feed().await {
            Ok(Some(feed)) => {
                let row = self.store.source(source_id).await?;
                let tz = refresh::parse_tz(feed.default_tz.as_deref().or(row.server_tz.as_deref()));
                let days = refresh::max_catchup_days(&self.store, source_id)
                    .await?
                    .max(0);
                let keep = crate::source::TimeWindow {
                    start_utc: epg::retention_cutoff(now, days),
                    end_utc: now + 7 * 86_400,
                };
                let r = crate::xmltv::ingest::ingest(
                    &self.store,
                    source_id,
                    feed.file.path().to_path_buf(),
                    tz,
                    keep,
                )
                .await;
                match r {
                    Ok(rep) => {
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
            Ok(None) => {
                if client.kind() == SourceKind::Dispatcharr {
                    epg::sync_guide(client, &self.store, source_id, epg::visible_window(now))
                        .await?;
                }
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
        self.emit(Event::CatalogUpdated(self.counts(source_id).await?));
        Ok(r)
    }

    /// Best-effort favourites/recents/resume sync with the server.
    pub async fn sync_user_state(&self, source_id: i64) -> Result<userstate::StateSyncReport> {
        let client = self.client(source_id).await?;
        userstate::sync_state(&self.store, client.as_ref(), now_utc()).await
    }

    // ---- content filters / settings ---------------------------------------------------------

    async fn filter(&self) -> Result<ChannelFilter> {
        let flag = |k: &'static str| {
            let store = self.store.clone();
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
            .await
    }

    fn default_settings() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("liveBufferSecs".into(), 3.into());
        m.insert("hwdec".into(), "auto-safe".into());
        m.insert("debugLogging".into(), false.into());
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
                cur.insert(k, Value::from(n.round().clamp(1.0, 10.0) as i64));
            } else {
                cur.insert(k, v);
            }
        }
        let out = Value::Object(cur);
        self.store
            .kv_set(SETTINGS_KEY, &serde_json::to_string(&out)?, now_utc())
            .await?;
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
                channel_count: g.channel_count,
                count: g.channel_count,
            })
            .collect())
    }

    async fn logo_for(
        &self,
        bases: &HashMap<i64, url::Url>,
        c: &crate::store::ChannelRow,
    ) -> Option<String> {
        let logo_id = c.logo_key.as_deref()?;
        // M3U/Xtream logos are absolute public URLs; Dispatcharr's are ids on its own server.
        let url = if logo_id.starts_with("http://") || logo_id.starts_with("https://") {
            logo_id.to_owned()
        } else {
            logo_url(bases.get(&c.source_id)?, logo_id).ok()?
        };
        let known = self
            .registered_logos
            .lock()
            .ok()
            .is_some_and(|g| g.contains(&url));
        let dl = self
            .images
            .register(c.source_id, &url, ImageSize::Grid)
            .await
            .ok()?;
        if !known && let Ok(mut g) = self.registered_logos.lock() {
            g.insert(url);
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
        let bases: HashMap<i64, url::Url> = self
            .store
            .list_sources()
            .await?
            .into_iter()
            .filter_map(|s| Some((s.id, normalise_base_url(s.base_url.as_deref()?).ok()?)))
            .collect();
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let logo = self.logo_for(&bases, r).await;
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
        Ok(channel_ids
            .iter()
            .map(|id| {
                let nn = m.get(id).cloned().unwrap_or_default();
                NowNextDto {
                    channel_id: *id,
                    now: nn.now.as_ref().map(Into::into),
                    next: nn.next.as_ref().map(Into::into),
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
        let progs = epg::window(
            &self.store,
            &ids,
            crate::source::TimeWindow {
                start_utc: a.start,
                end_utc: a.end,
            },
        )
        .await?;
        Ok(GuideDto {
            channels: self.to_channel_dtos(&rows).await?,
            programmes: progs.iter().map(Into::into).collect(),
        })
    }

    pub async fn search(&self, query: &str, limit: i64) -> Result<SearchDto> {
        let limit = limit.clamp(1, 200);
        let f = self.filter().await?;
        let rows = catalog::list_channels(
            &self.store,
            &ChannelFilter {
                search: Some(query.to_owned()),
                ..f.clone()
            },
            limit,
            0,
        )
        .await?;
        let progs = catalog::search_programmes(&self.store, &f, query, now_utc(), limit).await?;
        // Each hit carries its full channel DTO (logo included) so the UI needs no second call.
        let ids: Vec<i64> = {
            let mut seen = HashSet::new();
            progs
                .iter()
                .map(|p| p.channel_id)
                .filter(|i| seen.insert(*i))
                .collect()
        };
        let chan_rows = catalog::channels_by_ids(&self.store, &f, &ids).await?;
        let chan_dtos: HashMap<i64, ChannelDto> = self
            .to_channel_dtos(&chan_rows)
            .await?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        let programmes = progs
            .iter()
            .filter_map(|p| {
                Some(ProgrammeHitDto {
                    programme: p.into(),
                    channel: chan_dtos.get(&p.channel_id)?.clone(),
                })
            })
            .collect();
        Ok(SearchDto {
            channels: self.to_channel_dtos(&rows).await?,
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
        let rows = epg::window(
            &self.store,
            &[channel_id],
            crate::source::TimeWindow {
                start_utc: start,
                end_utc: end,
            },
        )
        .await?;
        Ok(rows.iter().map(Into::into).collect())
    }

    async fn programme_dto(&self, channel_id: i64, start_utc: i64) -> Result<Option<ProgrammeDto>> {
        let row: Option<crate::store::ProgrammeRow> = sqlx::query_as(
            "SELECT channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags
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
        _r: &crate::store::ResumeRow,
        _progress: Option<f64>,
        _remaining: Option<i64>,
    ) -> Result<Option<ContinueItemDto>> {
        Ok(None)
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
        if let Ok(mut g) = self.current_stream.lock() {
            g.insert(channel_id, 0);
        }
        self.store.touch_recent(channel_id, now_utc()).await?;
        Ok(plan)
    }

    async fn live_plan(&self, channel_id: i64) -> Result<PlayPlan> {
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let src = self.store.source(ch.source_id).await?;
        if SourceKind::parse(&src.kind) == Some(SourceKind::Dispatcharr) {
            let plan = catalog::resolve_play_plan(&self.store, self, channel_id).await?;
            plan.resolve_at_play(src.tls_insecure).await
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
            .channel_stream_names(&ch.ext_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, (stream_id, name))| AltStreamDto {
                index,
                stream_id,
                name,
            })
            .collect();
        if streams.is_empty() {
            // Fall back to the failover ids stored at sync (names unknown), or a single entry.
            let ids = self.store.channel_stream_ids(channel_id).await?;
            streams = if ids.is_empty() {
                vec![AltStreamDto {
                    index: 0,
                    stream_id: ch.ext_id.clone(),
                    name: ch.name.clone(),
                }]
            } else {
                ids.into_iter()
                    .enumerate()
                    .map(|(index, stream_id)| AltStreamDto {
                        index,
                        name: format!("Stream {}", index + 1),
                        stream_id,
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
        sessions::cleanup_on_startup(&self.store, self).await
    }

    /// Sleep / quit / navigate-away: stop every open session.
    pub async fn stop_all_sessions(&self) -> Result<sessions::CleanupReport> {
        sessions::stop_all(&self.store, self).await
    }

    // ---- images -----------------------------------------------------------------------------

    /// Serve a `dlimg://<hash>/<size>` URL (the custom protocol handler calls this).
    pub async fn image(&self, dlimg_url: &str) -> Result<ImageBytes> {
        let (hash, size) = parse_dlimg(dlimg_url)
            .ok_or_else(|| AppError::local("That image address is not valid."))?;
        self.images.get(&hash, size, &ServiceFetcher(self)).await
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
    async fn fetch(&self, source_id: i64, url: &str) -> Result<RawImage> {
        self.0.client(source_id).await?.fetch_image(url).await
    }
}
