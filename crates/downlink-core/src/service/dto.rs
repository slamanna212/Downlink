//! DTOs crossing the IPC boundary. Field names follow `apps/ui/src/api/types.ts` (the UI owns that
//! contract); everything is camelCase and all timestamps are UTC epoch seconds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::source::flags;
use crate::store::{ChannelRow, ProgrammeRow, SourceRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Not probed yet since launch.
    Connecting,
    Connected,
    Syncing,
    Offline,
    Stale,
    AuthError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub id: i64,
    pub uuid: String,
    pub kind: String,
    pub name: String,
    /// Host (and port) only; never a URL that could carry credentials.
    pub host: Option<String>,
    pub caps: u32,
    pub status: SourceStatus,
    pub server_version: Option<String>,
    pub last_sync_at: Option<i64>,
    /// Longest catch-up window across the source's channels, in days (0 = none).
    pub archive_days: i64,
    /// Whether live playback goes through the server's HLS output (user opt-in).
    pub hls_transport: bool,
    /// Whether this source can offer that option (Dispatcharr only); the UI shows the toggle on this,
    /// never on `kind`.
    pub supports_hls_transport: bool,
    /// Concurrent streams the account may hold; null when unknown or unlimited.
    pub stream_limit: Option<i64>,
}

/// Arguments of `set_source_options`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSourceOptionsArgs {
    pub source_id: i64,
    pub hls_transport: bool,
}

impl SourceDto {
    pub fn from_row(r: &SourceRow, status: SourceStatus, archive_days: i64) -> Self {
        let host = r
            .base_url
            .as_deref()
            .and_then(|u| url::Url::parse(u).ok())
            .and_then(|u| {
                u.host_str().map(|h| match u.port() {
                    Some(p) => format!("{h}:{p}"),
                    None => h.to_owned(),
                })
            });
        Self {
            id: r.id,
            uuid: r.uuid.clone(),
            kind: r.kind.clone(),
            name: r.name.clone(),
            host,
            caps: crate::source::Caps::from_db(r.caps).bits() & crate::source::Caps::UI_MASK,
            status,
            server_version: r.server_version.clone(),
            last_sync_at: r.last_sync_at,
            archive_days,
            hls_transport: r.hls_transport,
            supports_hls_transport: r.kind == "dispatcharr",
            stream_limit: r.stream_limit,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub id: i64,
    pub source_id: i64,
    pub name: String,
    pub channel_count: i64,
    /// Same value as `channel_count` (the newer UI contract calls it `count`).
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub id: i64,
    pub source_id: i64,
    pub uuid: Option<String>,
    pub number: Option<f64>,
    pub name: String,
    pub group_id: Option<i64>,
    /// `dlimg://<hash>/160` or null.
    pub logo: Option<String>,
    pub quality: Option<&'static str>,
    pub is_catchup: bool,
    pub catchup_days: i64,
    pub is_radio: bool,
    pub has_drm: bool,
    pub is_favourite: bool,
    pub epg_match: String,
}

/// UI quality union is SD | HD | FHD | 4K. UHD/8K collapse to 4K; codec-only labels have no
/// resolution and map to null.
fn ui_quality(q: Option<&str>) -> Option<&'static str> {
    match q? {
        "SD" => Some("SD"),
        "HD" => Some("HD"),
        "FHD" => Some("FHD"),
        "UHD" | "4K" | "8K" => Some("4K"),
        _ => None,
    }
}

impl ChannelDto {
    pub fn from_row(r: &ChannelRow, logo: Option<String>, is_favourite: bool) -> Self {
        Self {
            id: r.id,
            source_id: r.source_id,
            uuid: r.uuid.clone(),
            number: r.number,
            name: r.name.clone(),
            group_id: r.group_id,
            logo,
            quality: ui_quality(r.quality.as_deref()),
            is_catchup: r.catchup_days > 0,
            catchup_days: r.catchup_days,
            is_radio: r.is_radio,
            has_drm: r.is_drm,
            is_favourite,
            epg_match: r.epg_match.clone().unwrap_or_else(|| "none".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammeDto {
    /// `"<channelId>:<startUtc>"`: pass back to `play_catchup`.
    pub id: String,
    pub channel_id: i64,
    pub start_utc: i64,
    pub end_utc: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub is_new: bool,
    pub is_live: bool,
    pub is_premiere: bool,
    pub is_finale: bool,
    /// Scheduled for recording (DVR is not built yet).
    pub is_scheduled: bool,
}

impl From<&ProgrammeRow> for ProgrammeDto {
    fn from(p: &ProgrammeRow) -> Self {
        Self {
            id: format!("{}:{}", p.channel_id, p.start_utc),
            channel_id: p.channel_id,
            start_utc: p.start_utc,
            end_utc: p.end_utc,
            title: p.title.clone(),
            subtitle: p.subtitle.clone(),
            description: p.description.clone(),
            season: p.season,
            episode: p.episode,
            is_new: p.flags & flags::NEW != 0,
            is_live: p.flags & flags::LIVE != 0,
            is_premiere: p.flags & flags::PREMIERE != 0,
            is_finale: p.flags & flags::FINALE != 0,
            is_scheduled: false,
        }
    }
}

/// Parse `ProgrammeDto::id`.
pub fn parse_programme_id(id: &str) -> Option<(i64, i64)> {
    let (c, s) = id.split_once(':')?;
    Some((c.parse().ok()?, s.parse().ok()?))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowNextDto {
    pub channel_id: i64,
    pub now: Option<ProgrammeDto>,
    pub next: Option<ProgrammeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogUpdated {
    pub source_id: i64,
    pub channels: i64,
    pub groups: i64,
    pub programmes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub source_id: i64,
    pub phase: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Event {
    CatalogUpdated(CatalogUpdated),
    TaskProgress(TaskProgress),
}

impl Event {
    /// The event name the UI listens on (PLAN §2.5).
    pub fn name(&self) -> &'static str {
        match self {
            Self::CatalogUpdated(_) => "catalog://updated",
            Self::TaskProgress(_) => "task://progress",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddSourceInput {
    pub kind: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub xmltv_url: Option<String>,
    #[serde(default)]
    pub allow_self_signed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSourceResult {
    pub channels: i64,
    pub groups: i64,
    pub programmes: i64,
    pub server_version: Option<String>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListChannelsArgs {
    #[serde(default)]
    pub source_id: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub favourites_only: bool,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideArgs {
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub channel_offset: i64,
    #[serde(default)]
    pub channel_limit: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuideDto {
    pub channels: Vec<ChannelDto>,
    pub programmes: Vec<ProgrammeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgrammeHitDto {
    pub programme: ProgrammeDto,
    pub channel: ChannelDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDto {
    pub channels: Vec<ChannelDto>,
    pub programmes: Vec<ProgrammeHitDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountsDto {
    pub all: i64,
    pub favourites: i64,
}

/// One "continue watching" tile: `kind` is live | catchup | movie | episode | recording.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinueItemDto {
    pub kind: String,
    /// `live:<channelId>`, `catchup:<channelId>:<startUtc>`, `movie:<id>`, `episode:<id>`, `recording:<id>`.
    pub id: String,
    pub channel: Option<ChannelDto>,
    pub programme: Option<ProgrammeDto>,
    pub title: String,
    pub subtitle: Option<String>,
    /// `dlimg://` artwork or null.
    pub art: Option<String>,
    /// 0..1, null for live.
    pub progress: Option<f64>,
    pub remaining_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnNowItemDto {
    pub channel: ChannelDto,
    pub now: Option<ProgrammeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeDto {
    pub continue_watching: Vec<ContinueItemDto>,
    pub favourites: Vec<ChannelDto>,
    /// Current programme per favourite.
    pub on_now: Vec<OnNowItemDto>,
    pub recents: Vec<ChannelDto>,
}

/// One entry of a channel's failover list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AltStreamDto {
    /// 0-based failover position.
    pub index: usize,
    pub stream_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamListDto {
    pub channel_id: i64,
    /// 1-based index of the stream believed to be in use, when known.
    pub source_index: Option<usize>,
    pub source_count: usize,
    pub streams: Vec<AltStreamDto>,
}

/// The parts of `Diagnostics` core can fill; the desktop layer adds `playerState`,
/// `hwdecCurrent` and `transitions`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsData {
    pub app_version: String,
    pub server_versions: Vec<String>,
    pub os: String,
    /// Already redacted.
    pub log_tail: Vec<String>,
}

/// Settings are an open JSON object (the UI owns the field set); core validates only
/// `liveBufferSecs` (1..=10) and persists the rest verbatim.
pub type SettingsDto = Value;

// ---- VOD ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VodKindArg {
    Movie,
    Series,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VodCategoryDto {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VodListArgs {
    #[serde(default)]
    pub category_id: Option<i64>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieDto {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    /// 0..10 or null.
    pub rating: Option<f64>,
    pub duration_secs: Option<i64>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub plot: Option<String>,
    pub genres: Vec<String>,
    pub category_id: Option<i64>,
    /// Saved position in seconds, null when never watched.
    pub resume_secs: Option<i64>,
    pub watched: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDto {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub plot: Option<String>,
    pub genres: Vec<String>,
    pub category_id: Option<i64>,
    pub season_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeDto {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub plot: Option<String>,
    pub duration_secs: Option<i64>,
    pub still: Option<String>,
    pub resume_secs: Option<i64>,
    pub watched: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonDto {
    pub number: i64,
    pub episodes: Vec<EpisodeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesDetailDto {
    pub series: SeriesDto,
    pub seasons: Vec<SeasonDto>,
    /// Episode to continue with (resume or next unwatched), null when none.
    pub next_episode_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayVodArgs {
    /// `movie` or `episode`.
    pub kind: String,
    pub id: i64,
    #[serde(default)]
    pub resume: bool,
}

// ---- DVR ------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordKindArg {
    Once,
    Series,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDto {
    pub id: i64,
    pub title: String,
    pub channel_id: Option<i64>,
    pub channel_name: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub status: &'static str,
    pub rule_id: Option<i64>,
    pub rule_label: Option<String>,
    pub size_bytes: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingsDto {
    pub upcoming: Vec<RecordingDto>,
    pub in_progress: Vec<RecordingDto>,
    /// Finished recordings, failed ones included (their `status` says so).
    pub completed: Vec<RecordingDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRuleDto {
    pub id: i64,
    pub title: String,
    pub channel_name: String,
    pub kind: &'static str,
    pub schedule: String,
}
