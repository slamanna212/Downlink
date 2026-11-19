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
use crate::sessions::{self, SessionStopper};
use crate::source::dispatcharr::{
    ClientOptions, Credentials, DispatcharrClient, logo_url, normalise_base_url,
};
use crate::source::{PlayPlan, Source, SourceId, SourceKind, all_channels};
use crate::store::{NewSource, SourceRow, Store};
use crate::sync::{self, now_utc};
use crate::userstate;

pub use dto::*;
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
    clients: RwLock<HashMap<i64, Arc<DispatcharrClient>>>,
    status: Mutex<HashMap<i64, SourceStatus>>,
    registered_logos: Mutex<HashSet<String>>,
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
            status: Mutex::new(HashMap::new()),
            registered_logos: Mutex::new(HashSet::new()),
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

    fn status_of(&self, row: &SourceRow) -> SourceStatus {
        if let Some(s) = self
            .status
            .lock()
            .ok()
            .and_then(|g| g.get(&row.id).copied())
        {
            return s;
        }
        match row.last_sync_at {
            Some(t) if now_utc() - t <= STALE_AFTER_SECS => SourceStatus::Connected,
            _ => SourceStatus::Stale,
        }
    }

    // ---- connections ------------------------------------------------------------------------

    async fn client(&self, source_id: i64) -> Result<Arc<DispatcharrClient>> {
        if let Some(c) = self.clients.read().await.get(&source_id) {
            return Ok(c.clone());
        }
        let row = self.store.source(source_id).await?;
        if SourceKind::parse(&row.kind) != Some(SourceKind::Dispatcharr) {
            return Err(AppError::local("This kind of source is not available yet."));
        }
        let base = row
            .base_url
            .as_deref()
            .ok_or_else(|| AppError::local("This source has no address."))?;
        let secret = row
            .secret_ref
            .as_deref()
            .map(|r| self.secrets.get(r))
            .transpose()?
            .flatten()
            .ok_or_else(|| {
                AppError::auth(
                    "The saved credentials for this source were not found. Sign in again.",
                )
            })?;
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
        self.clients.write().await.insert(source_id, c.clone());
        Ok(c)
    }

    fn probe_client(input: &AddSourceInput, id: i64) -> Result<DispatcharrClient> {
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
        DispatcharrClient::new(
            SourceId(id),
            &input.url,
            creds,
            ClientOptions {
                insecure_tls: input.allow_self_signed,
                ..Default::default()
            },
        )
    }

    fn check_kind(input: &AddSourceInput) -> Result<()> {
        match SourceKind::parse(&input.kind) {
            Some(SourceKind::Dispatcharr) => Ok(()),
            Some(_) => Err(AppError::local("This kind of source is not available yet.")),
            None => Err(AppError::local("Unknown source type.")),
        }
    }

    // ---- sources ----------------------------------------------------------------------------

    pub async fn list_sources(&self) -> Result<Vec<SourceDto>> {
        Ok(self
            .store
            .list_sources()
            .await?
            .iter()
            .map(|r| SourceDto::from_row(r, self.status_of(r)))
            .collect())
    }

    /// Connect and count what is there, without saving anything.
    pub async fn test_source(&self, input: AddSourceInput) -> Result<TestSourceResult> {
        Self::check_kind(&input)?;
        let c = Self::probe_client(&input, 0)?;
        let info = c.connect().await?;
        let groups = c.groups().await?;
        let (channels, _) = all_channels(&c, 500).await?;
        let programmes = c
            .guide(epg::visible_window(now_utc()))
            .await
            .map(|p| p.len())
            .unwrap_or(0);
        Ok(TestSourceResult {
            channels: channels.iter().filter(|c| !c.hidden).count() as i64,
            groups: groups.len() as i64,
            programmes: programmes as i64,
            server_version: info.server_version,
            user: info.username,
        })
    }

    /// Verify, save (secret to the keychain, reference to SQLite), then do the first sync.
    pub async fn add_source(&self, input: AddSourceInput) -> Result<SourceDto> {
        Self::check_kind(&input)?;
        let name = input.name.trim();
        if name.is_empty() {
            return Err(AppError::local("Give the source a name."));
        }
        let base = normalise_base_url(&input.url)?;
        // Fail fast: nothing is persisted if the server or credentials are wrong.
        Self::probe_client(&input, 0)?.connect().await?;

        let (kind, secret) = match (&input.api_key, &input.password) {
            (Some(k), _) if !k.trim().is_empty() => {
                (SecretKind::ApiKey, SecretString::new(k.trim()))
            }
            (_, Some(p)) => (SecretKind::Password, SecretString::new(p.clone())),
            _ => {
                return Err(AppError::local(
                    "Enter an API key, or a username and password.",
                ));
            }
        };
        let row = self
            .store
            .insert_source(&NewSource {
                kind: SourceKind::Dispatcharr.as_str().to_owned(),
                name: name.to_owned(),
                base_url: Some(base.to_string()),
                secret_ref: None,
                username: matches!(kind, SecretKind::Password)
                    .then(|| input.username.clone().unwrap_or_default().trim().to_owned()),
                tls_insecure: input.allow_self_signed,
                created_at: now_utc(),
            })
            .await?;
        let saved = async {
            let r = put_source_secret(self.secrets.as_ref(), &row.uuid, kind, &secret)?;
            self.store.set_source_secret_ref(row.id, Some(&r)).await
        }
        .await;
        if let Err(e) = saved {
            let _ = self.store.delete_source(row.id).await;
            return Err(e);
        }
        self.sync_source(row.id).await?;
        let row = self.store.source(row.id).await?;
        Ok(SourceDto::from_row(&row, self.status_of(&row)))
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
        self.store.delete_source(source_id).await?;
        self.clients.write().await.remove(&source_id);
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
        self.progress(source_id, "guide", 2, 4);
        if let Err(e) = epg::sync_guide(
            client.as_ref(),
            &self.store,
            source_id,
            epg::visible_window(now_utc()),
        )
        .await
        {
            // The guide is not critical: channels stay usable.
            tracing::warn!(error = %e, "guide sync failed");
        }
        self.progress(source_id, "state", 3, 4);
        if let Err(e) = userstate::sync_state(&self.store, client.as_ref(), now_utc()).await {
            tracing::warn!(error = %e, "state sync failed");
        }
        self.progress(source_id, "done", 4, 4);
        self.emit(Event::CatalogUpdated(self.counts(source_id).await?));
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
        let base = bases.get(&c.source_id)?;
        let url = logo_url(base, logo_id).ok()?;
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
        Ok(SearchDto {
            channels: self.to_channel_dtos(&rows).await?,
            programmes: progs.iter().map(Into::into).collect(),
        })
    }

    /// Home screen: continue watching, favourites, what is on now, recents.
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
        let mut ids: Vec<i64> = favourites.iter().chain(&recents).map(|c| c.id).collect();
        ids.dedup();
        let mut seen = HashSet::new();
        ids.retain(|i| seen.insert(*i));
        let nn = epg::now_next_many(&self.store, &ids, now).await?;
        let on_now = favourites
            .iter()
            .chain(&recents)
            .filter(|c| seen.remove(&c.id)) // first occurrence only
            .filter_map(|c| {
                let p = nn.get(&c.id)?.now.as_ref()?;
                Some(OnNowDto {
                    channel: c.clone(),
                    programme: p.into(),
                })
            })
            .collect();

        let mut continue_watching = Vec::new();
        for r in self.store.continue_watching(20).await? {
            let mut item = ContinueWatchingDto {
                kind: r.kind.clone(),
                item_key: r.item_key.clone(),
                source_id: r.source_id,
                position_ms: r.position_ms,
                duration_ms: r.duration_ms,
                updated_at: r.updated_at,
                channel_id: None,
                programme_id: None,
                title: None,
            };
            if r.kind == "catchup"
                && let Some((ext, start)) = r.item_key.split_once(':')
                && let Ok(start) = start.parse::<i64>()
                && let Some(ch) = self.store.channel_by_ext(r.source_id, ext).await?
            {
                item.channel_id = Some(ch.id);
                item.programme_id = Some(format!("{}:{start}", ch.id));
                item.title = sqlx::query_scalar(
                    "SELECT title FROM programme WHERE channel_id = ? AND start_utc = ?",
                )
                .bind(ch.id)
                .bind(start)
                .fetch_optional(self.store.pool())
                .await?
                .flatten();
            }
            continue_watching.push(item);
        }
        Ok(HomeDto {
            continue_watching,
            favourites,
            on_now,
            recents,
        })
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
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let src = self.store.source(ch.source_id).await?;
        let plan = catalog::resolve_play_plan(&self.store, self, channel_id).await?;
        let plan = plan.resolve_at_play(src.tls_insecure).await?;
        self.store.touch_recent(channel_id, now_utc()).await?;
        Ok(plan)
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
    async fn auth_header(&self, source: &SourceRow) -> Result<(String, SecretString)> {
        self.client(source.id).await?.auth_header().await
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
