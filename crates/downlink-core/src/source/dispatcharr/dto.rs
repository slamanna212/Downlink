//! Wire types for Dispatcharr (PLAN §3.2, §3.3). Tolerant by policy: `#[serde(default)]` on
//! everything optional, unknown fields ignored, ids accepted as number or string, enums kept as
//! strings. Provider URLs (`stream.url`) are deliberately not modelled so they cannot leak.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::source::{Programme, SourceChannel, SourceGroup, UserLevel, flags};

pub fn flex_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn flex_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn flex_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn flex_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => false,
    }
}

fn de_opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_string))
}
fn de_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_string)
        .unwrap_or_default())
}
fn de_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(flex_i64))
}
fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_i64)
        .unwrap_or(0))
}
fn de_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(flex_f64))
}
fn de_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .is_some_and(flex_bool))
}
/// `streams` is a list of ids, or of objects carrying an `id`. Order is preserved.
fn de_stream_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Option::<Vec<Value>>::deserialize(d)?.unwrap_or_default();
    Ok(v.iter()
        .filter_map(|x| match x {
            Value::Object(o) => o.get("id").and_then(flex_string),
            other => flex_string(other),
        })
        .collect())
}

#[derive(Debug, Deserialize, Default)]
pub struct VersionDto {
    #[serde(default, deserialize_with = "de_opt_string")]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct MeDto {
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub id: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub username: Option<String>,
    #[serde(default)]
    pub user_level: Value,
}

impl MeDto {
    /// Dispatcharr levels: 0 streamer, 1 standard, 10 admin (or the names).
    pub fn level(&self) -> Option<UserLevel> {
        match &self.user_level {
            Value::Number(n) => match n.as_i64()? {
                i64::MIN..=0 => Some(UserLevel::Streamer),
                1..=9 => Some(UserLevel::Standard),
                _ => Some(UserLevel::Admin),
            },
            Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "streamer" | "0" => Some(UserLevel::Streamer),
                "standard" | "1" => Some(UserLevel::Standard),
                "admin" | "10" => Some(UserLevel::Admin),
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct SettingDto {
    #[serde(default, deserialize_with = "de_string")]
    pub key: String,
    #[serde(default)]
    pub value: Value,
}

#[derive(Debug, Deserialize, Default)]
pub struct ChannelDto {
    #[serde(default, deserialize_with = "de_i64")]
    pub id: i64,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub uuid: Option<String>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub channel_number: Option<f64>,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub channel_group_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub tvg_id: Option<String>,
    #[serde(default, deserialize_with = "de_stream_ids")]
    pub streams: Vec<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub stream_profile_id: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub logo_id: Option<String>,
    #[serde(default, deserialize_with = "de_i64")]
    pub user_level: i64,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_adult: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_catchup: bool,
    #[serde(default, deserialize_with = "de_i64")]
    pub catchup_days: i64,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_radio: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub hidden_from_output: bool,
    // `effective_*` apply per-channel overrides; preferred when present (0.34+).
    #[serde(default, deserialize_with = "de_opt_string")]
    pub effective_name: Option<String>,
    #[serde(default, deserialize_with = "de_opt_f64")]
    pub effective_channel_number: Option<f64>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub effective_channel_group_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub effective_logo_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub effective_tvg_id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub effective_stream_profile_id: Option<i64>,
    #[serde(default)]
    pub effective_is_radio: Option<bool>,
}

impl ChannelDto {
    pub fn into_source(self) -> SourceChannel {
        SourceChannel {
            ext_id: self.id.to_string(),
            uuid: self.uuid,
            number: self.effective_channel_number.or(self.channel_number),
            name: self
                .effective_name
                .filter(|n| !n.is_empty())
                .unwrap_or(self.name),
            group_ext_id: self.effective_channel_group_id.or(self.channel_group_id),
            tvg_id: self.effective_tvg_id.or(self.tvg_id),
            logo_key: self.effective_logo_id.or(self.logo_id),
            streams: self.streams,
            // null means "server default"; resolved client-side (ProfileInfo::is_redirect).
            stream_profile_id: self.effective_stream_profile_id.or(self.stream_profile_id),
            user_level: self.user_level,
            is_adult: self.is_adult,
            is_catchup: self.is_catchup,
            catchup_days: self.catchup_days,
            is_radio: self.effective_is_radio.unwrap_or(self.is_radio),
            is_drm: false,
            hidden: self.hidden_from_output,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct GroupDto {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
}

impl GroupDto {
    pub fn into_source(self) -> SourceGroup {
        SourceGroup {
            ext_id: self.id,
            name: self.name,
        }
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct ProfileDto {
    #[serde(default, deserialize_with = "de_i64")]
    pub id: i64,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
}

/// A channel profile (`/api/channels/profiles/`).
pub type ChannelProfileDto = ProfileDto;

#[derive(Debug, Deserialize, Default, Clone)]
pub struct StreamProfileDto {
    #[serde(default, deserialize_with = "de_i64")]
    pub id: i64,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_active: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub locked: bool,
    #[serde(default, deserialize_with = "de_string")]
    pub command: String,
}

impl StreamProfileDto {
    /// The built-in profile that answers with a 302 to the provider URL.
    /// Tolerant: there is no `type` field. The built-in is named "Redirect" (locked, empty
    /// command); a renamed copy with an empty command and a "redirect" name also counts.
    pub fn is_redirect(&self) -> bool {
        self.name.trim().eq_ignore_ascii_case("redirect")
            || (self.locked
                && self.command.is_empty()
                && self.name.to_ascii_lowercase().contains("redirect"))
    }
}

/// A stream attached to a channel. The provider `url` is intentionally not captured.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
pub struct StreamDto {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default, deserialize_with = "de_string")]
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProgramDto {
    #[serde(default, deserialize_with = "de_string")]
    pub id: String,
    #[serde(default)]
    pub start_time: Value,
    #[serde(default)]
    pub end_time: Value,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub sub_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "de_string")]
    pub tvg_id: String,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub season: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub episode: Option<i64>,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_new: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_live: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_premiere: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub is_finale: bool,
}

/// RFC 3339 / ISO 8601 string, or epoch seconds, to UTC seconds.
pub fn parse_time(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => {
            time::OffsetDateTime::parse(s.trim(), &time::format_description::well_known::Rfc3339)
                .ok()
                .map(|t| t.unix_timestamp())
        }
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

impl ProgramDto {
    /// `None` when the times are unusable (the row is skipped, not fatal).
    pub fn into_programme(self) -> Option<Programme> {
        let start = parse_time(&self.start_time)?;
        let end = parse_time(&self.end_time)?;
        if end <= start {
            return None;
        }
        let mut f = 0;
        for (on, bit) in [
            (self.is_new, flags::NEW),
            (self.is_live, flags::LIVE),
            (self.is_premiere, flags::PREMIERE),
            (self.is_finale, flags::FINALE),
        ] {
            if on {
                f |= bit;
            }
        }
        Some(Programme {
            channel_ref: self.tvg_id,
            ext_id: if self.id.is_empty() {
                None
            } else {
                Some(self.id)
            },
            start_utc: start,
            end_utc: end,
            title: self.title,
            subtitle: self.sub_title,
            description: self.description,
            season: self.season,
            episode: self.episode,
            flags: f,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TokenDto {
    #[serde(default)]
    pub access: Option<String>,
    #[serde(default)]
    pub refresh: Option<String>,
}

/// A `/api/epg/programs/search/` row. Carries the channels it airs on.
#[derive(Debug, Deserialize, Default)]
pub struct ProgramSearchDto {
    #[serde(flatten)]
    pub program: ProgramDto,
    #[serde(default)]
    pub custom_properties: Value,
    #[serde(default)]
    pub channels: Vec<Value>,
}

impl ProgramSearchDto {
    pub fn into_hit(self) -> Option<crate::source::ProgramHit> {
        let channel_ext_ids: Vec<String> = self
            .channels
            .iter()
            .filter_map(|c| c.get("id").and_then(flex_string))
            .collect();
        let cp = &self.custom_properties;
        let mut dto = self.program;
        if dto.season.is_none() {
            dto.season = cp.get("season").and_then(flex_i64);
        }
        if dto.episode.is_none() {
            dto.episode = cp.get("episode").and_then(flex_i64);
        }
        if cp.get("new").is_some_and(flex_bool) {
            dto.is_new = true;
        }
        Some(crate::source::ProgramHit {
            programme: dto.into_programme()?,
            channel_ext_ids,
        })
    }
}
