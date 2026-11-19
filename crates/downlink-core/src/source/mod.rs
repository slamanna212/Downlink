//! Content origins. Every origin is a [`Source`] returning the same normalised model plus a
//! [`Caps`] bitflag; no UI or player code branches on source kind (CLAUDE.md rule 9).

pub mod dispatcharr;
pub mod play;

use std::collections::HashSet;

use async_trait::async_trait;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

pub use play::{
    Location, PlayPlan, PlayProfile, RedactedPlan, RedirectOutcome, SessionId, Surface,
    probe_redirect, resolve_redirect,
};

/// Local row id of a configured source (`source.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Dispatcharr,
    Xtream,
    M3u,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatcharr => "dispatcharr",
            Self::Xtream => "xtream",
            Self::M3u => "m3u",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dispatcharr" => Some(Self::Dispatcharr),
            "xtream" => Some(Self::Xtream),
            "m3u" => Some(Self::M3u),
            _ => None,
        }
    }
}

bitflags! {
    /// What a source can do. The UI branches on these, never on [`SourceKind`] (PLAN §6.1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Caps: u32 {
        // Bits 0-5 match `Cap` in apps/ui/src/api/types.ts (the UI owns that contract).
        const LIVE       = 1 << 0;
        const EPG        = 1 << 1;
        const CATCHUP    = 1 << 2;
        const VOD        = 1 << 3;
        const DVR        = 1 << 4;
        const FAVOURITES = 1 << 5;
        // Core-only flags (PLAN §6.1): never needed by the UI.
        const FAILOVER   = 1 << 6;
        const LIMITS     = 1 << 7;
        const PROFILES   = 1 << 8;
    }
}

/// Serialised as the raw bits (a `u32`) so the UI gets a stable integer.
impl Serialize for Caps {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_u32(self.bits())
    }
}

impl<'de> Deserialize<'de> for Caps {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Self::from_bits_truncate(u32::deserialize(d)?))
    }
}

impl Caps {
    /// The bits the UI knows (`Cap` in types.ts).
    pub const UI_MASK: u32 = 0b11_1111;

    /// Value stored in `source.caps`.
    pub fn to_db(self) -> i64 {
        i64::from(self.bits())
    }

    pub fn from_db(v: i64) -> Self {
        Self::from_bits_truncate(u32::try_from(v).unwrap_or(0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserLevel {
    Streamer,
    Standard,
    Admin,
}

impl UserLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Streamer => "streamer",
            Self::Standard => "standard",
            Self::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub server_version: Option<String>,
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub user_level: Option<UserLevel>,
    pub timezone: Option<String>,
    /// Account expiry, UTC seconds (Xtream).
    pub expires_at: Option<i64>,
    pub max_connections: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub page: u32,
    pub page_size: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: 500,
        }
    }
}

/// A channel as a source reports it, before normalisation and filtering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceChannel {
    pub ext_id: String,
    pub uuid: Option<String>,
    pub number: Option<f64>,
    pub name: String,
    pub group_ext_id: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_name: Option<String>,
    pub logo_key: Option<String>,
    /// Failover order. Never reorder.
    pub streams: Vec<String>,
    pub stream_profile_id: Option<i64>,
    pub user_level: i64,
    pub is_adult: bool,
    pub is_catchup: bool,
    pub catchup_days: i64,
    pub is_radio: bool,
    pub is_drm: bool,
    /// M3U only: stream URL (any `|headers` suffix removed) and per-file headers as a JSON object.
    pub play_url: Option<String>,
    pub play_headers: Option<String>,
    /// JSON `catchup::template::CatchupTemplate` (M3U/Xtream template path).
    pub catchup_template: Option<String>,
    /// `hidden_from_output`: never shown in Downlink.
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGroup {
    pub ext_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct ChannelPage {
    pub channels: Vec<SourceChannel>,
    pub count: Option<u64>,
    pub has_next: bool,
}

/// UTC seconds, half-open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start_utc: i64,
    pub end_utc: i64,
}

pub mod flags {
    pub const NEW: i64 = 1;
    pub const LIVE: i64 = 1 << 1;
    pub const PREMIERE: i64 = 1 << 2;
    pub const FINALE: i64 = 1 << 3;
}

/// A guide entry. Times are UTC seconds. Ids are strings (dummy EPG ids are synthetic).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Programme {
    /// EPG channel reference (`tvg_id`) that maps this programme onto channels.
    pub channel_ref: String,
    pub ext_id: Option<String>,
    pub start_utc: i64,
    pub end_utc: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub flags: i64,
}

/// A guide-search result from the server, with the channels (by ext id) it airs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramHit {
    pub programme: Programme,
    pub channel_ext_ids: Vec<String>,
}

/// Server-side programme search filters (`/api/epg/programs/search/`).
#[derive(Debug, Clone, Default)]
pub struct ProgramSearch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub channel_id: Option<String>,
    /// UTC seconds.
    pub start_after: Option<i64>,
    pub airing_at: Option<i64>,
    pub page: u32,
    /// Capped at 500 by the server.
    pub page_size: u32,
}

#[derive(Debug, Clone, Default)]
pub struct VodQuery {
    pub search: Option<String>,
    pub page: Option<Page>,
}

#[derive(Debug, Clone, Default)]
pub struct VodPage {
    pub count: u64,
}

/// What `play` needs to know about a channel; built from SQLite, no network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelKey {
    pub ext_id: String,
    pub uuid: Option<String>,
    /// Effective stream profile is Redirect (PLAN §3.5).
    pub stream_mode: StreamMode,
}

/// How a channel's stream endpoint answers (PLAN §3.5). Tri-state so that "we do not know" can
/// never be mistaken for "proxy": `Unknown` is played like `Redirect` (fail safe, never leak the
/// key to a provider).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, sqlx::Type)]
#[repr(i64)]
pub enum StreamMode {
    Proxy = 0,
    Redirect = 1,
    #[default]
    Unknown = 2,
}

/// Stream profiles relevant to redirect detection.
#[derive(Debug, Clone, Default)]
pub struct ProfileInfo {
    pub redirect_ids: HashSet<i64>,
    /// Every profile id the server listed (redirect or not).
    pub known_ids: HashSet<i64>,
    pub default_id: Option<i64>,
}

impl ProfileInfo {
    /// Effective profile = the channel's own, else the server default. A profile we cannot find
    /// in the server's list is `Unknown`.
    pub fn mode(&self, channel_profile: Option<i64>) -> StreamMode {
        match channel_profile.or(self.default_id) {
            Some(id) if self.redirect_ids.contains(&id) => StreamMode::Redirect,
            Some(id) if self.known_ids.contains(&id) => StreamMode::Proxy,
            _ => StreamMode::Unknown,
        }
    }
}

#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> SourceId;
    fn kind(&self) -> SourceKind;
    fn caps(&self) -> Caps;

    async fn connect(&self) -> Result<AccountInfo>;
    async fn channels(&self, page: Page) -> Result<ChannelPage>;
    async fn guide(&self, window: TimeWindow) -> Result<Vec<Programme>>;
    async fn vod(&self, _q: VodQuery) -> Result<VodPage> {
        Err(AppError::server(
            "This source does not offer on-demand video.",
        ))
    }
    /// A play plan. Only call at an actual play: redirect-profile channels make one request.
    async fn play(&self, ch: ChannelKey) -> Result<PlayPlan>;
    async fn catchup(&self, ch: ChannelKey, p: &Programme) -> Result<PlayPlan>;
    async fn stop_session(&self, s: SessionId) -> Result<()>;
    /// Report catch-up playback position (seconds). Not an error while playback has not begun.
    async fn catchup_position(&self, _s: &SessionId, _position_secs: u64) -> Result<()> {
        Ok(())
    }
    /// Authenticated image fetch (same-origin only).
    async fn fetch_image(&self, _url: &str) -> Result<crate::images::RawImage> {
        Err(AppError::local("This source has no images."))
    }

    /// Groups, for sync. Not part of PLAN §6.1's sketch but needed by ingestion.
    async fn groups(&self) -> Result<Vec<SourceGroup>>;
    /// Redirect-profile metadata for sync (no stream endpoint is touched).
    async fn profile_info(&self) -> Result<ProfileInfo> {
        Ok(ProfileInfo::default())
    }
    /// Now-playing programmes for the given channel uuids, as `(channel_uuid, programme)`.
    async fn current_programmes(
        &self,
        _channel_uuids: &[String],
    ) -> Result<Vec<(String, Programme)>> {
        Ok(Vec::new())
    }
}

/// Fetch every channel page. Never assumes the server honoured `page_size`.
pub async fn all_channels(
    src: &dyn Source,
    page_size: u32,
) -> Result<(Vec<SourceChannel>, Option<u64>)> {
    let mut out = Vec::new();
    let mut count = None;
    let mut page = 1u32;
    loop {
        let p = src.channels(Page { page, page_size }).await?;
        count = count.or(p.count);
        let empty = p.channels.is_empty();
        out.extend(p.channels);
        if !p.has_next || empty || page >= 10_000 {
            break;
        }
        page += 1;
    }
    Ok((out, count))
}

/// Everything the app needs from a live source connection. Blanket-implemented, so a new source
/// kind only implements the individual traits.
pub trait SourceClient:
    Source + crate::sessions::SessionStopper + crate::userstate::AppDataRemote
{
}
impl<T> SourceClient for T where
    T: Source + crate::sessions::SessionStopper + crate::userstate::AppDataRemote
{
}
