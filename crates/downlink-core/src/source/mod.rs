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
    Location, PlayPlan, PlayProfile, RedactedPlan, SessionId, Surface, resolve_redirect,
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
        const EPG      = 1 << 0;
        const VOD      = 1 << 1;
        const DVR      = 1 << 2;
        const CATCHUP  = 1 << 3;
        const FAILOVER = 1 << 4;
        const LIMITS   = 1 << 5;
        const PROFILES = 1 << 6;
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
    pub stream_redirect: bool,
}

/// Stream profiles relevant to redirect detection.
#[derive(Debug, Clone, Default)]
pub struct ProfileInfo {
    pub redirect_ids: HashSet<i64>,
    pub default_id: Option<i64>,
}

impl ProfileInfo {
    /// Effective profile = the channel's own, else the server default.
    pub fn is_redirect(&self, channel_profile: Option<i64>) -> bool {
        channel_profile
            .or(self.default_id)
            .is_some_and(|id| self.redirect_ids.contains(&id))
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
