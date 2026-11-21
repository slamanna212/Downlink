//! Tolerant Xtream wire types: numbers as strings, missing fields, `null` for "none".

use base64::Engine;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::source::Programme;
use crate::source::dispatcharr::dto::{flex_f64, flex_i64, flex_string};

fn opt_str<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_string))
}
fn str_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_string)
        .unwrap_or_default())
}
/// A string, or the first string of an array (`backdrop_path` is usually a list).
fn first_str<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::Array(a)) => a.iter().find_map(flex_string),
        Some(v) => flex_string(&v),
        None => None,
    })
}
fn opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(flex_f64))
}
fn opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(flex_i64))
}
fn truthy<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::Bool(b)) => b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        Some(Value::String(s)) => matches!(s.trim(), "1" | "true" | "yes"),
        _ => false,
    })
}
fn zero_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_i64)
        .unwrap_or(0))
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CategoryDto {
    #[serde(rename = "category_id", default, deserialize_with = "str_or_empty")]
    pub id: String,
    #[serde(rename = "category_name", default, deserialize_with = "str_or_empty")]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LiveStreamDto {
    #[serde(default, deserialize_with = "str_or_empty")]
    pub stream_id: String,
    #[serde(default, deserialize_with = "str_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "opt_f64")]
    pub num: Option<f64>,
    #[serde(default, deserialize_with = "opt_str")]
    pub stream_type: Option<String>,
    #[serde(rename = "stream_icon", default, deserialize_with = "opt_str")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub epg_channel_id: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub category_id: Option<String>,
    #[serde(default, deserialize_with = "truthy")]
    pub is_adult: bool,
    #[serde(default, deserialize_with = "truthy")]
    pub tv_archive: bool,
    #[serde(rename = "tv_archive_duration", default, deserialize_with = "zero_i64")]
    pub tv_archive_days: i64,
    #[serde(default, deserialize_with = "opt_str")]
    pub direct_source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VodStreamDto {
    #[serde(default, deserialize_with = "str_or_empty")]
    pub stream_id: String,
    #[serde(default, deserialize_with = "str_or_empty")]
    pub name: String,
    #[serde(rename = "stream_icon", default, deserialize_with = "opt_str")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub category_id: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub container_extension: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub rating: Option<String>,
    #[serde(default, deserialize_with = "opt_i64")]
    pub added: Option<i64>,
    #[serde(default, deserialize_with = "opt_i64")]
    pub year: Option<i64>,
    /// Some panels send the TMDB id as `tmdb`, others as `tmdb_id` (separate fields, not an
    /// alias: a row carrying both must still parse).
    #[serde(default, deserialize_with = "opt_str")]
    pub tmdb: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub tmdb_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SeriesDto {
    #[serde(default, deserialize_with = "str_or_empty")]
    pub series_id: String,
    #[serde(default, deserialize_with = "str_or_empty")]
    pub name: String,
    #[serde(default, deserialize_with = "opt_str")]
    pub cover: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub plot: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub category_id: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub rating: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub genre: Option<String>,
    #[serde(rename = "releaseDate", default, deserialize_with = "opt_str")]
    pub release_date: Option<String>,
    #[serde(default, deserialize_with = "first_str")]
    pub backdrop_path: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub tmdb: Option<String>,
    #[serde(default, deserialize_with = "opt_str")]
    pub tmdb_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EpisodeDto {
    pub id: String,
    pub title: String,
    pub season: i64,
    pub episode: i64,
    pub container_extension: Option<String>,
    pub duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesInfoDto {
    pub name: Option<String>,
    pub plot: Option<String>,
    pub cover: Option<String>,
    pub episodes: Vec<EpisodeDto>,
}

impl SeriesInfoDto {
    /// `episodes` is `{ "<season>": [ {id, title, episode_num, container_extension, info:{…}} ] }`
    /// (or, on some panels, a flat array).
    pub fn from_value(v: &Value) -> Self {
        let info = v.get("info");
        let mut out = Self {
            name: info.and_then(|i| i.get("name")).and_then(flex_string),
            plot: info.and_then(|i| i.get("plot")).and_then(flex_string),
            cover: info.and_then(|i| i.get("cover")).and_then(flex_string),
            episodes: vec![],
        };
        let mut push = |season_hint: Option<i64>, e: &Value| {
            let Some(id) = e.get("id").and_then(flex_string) else {
                return;
            };
            out.episodes.push(EpisodeDto {
                id,
                title: e.get("title").and_then(flex_string).unwrap_or_default(),
                season: e
                    .get("season")
                    .and_then(flex_i64)
                    .or(season_hint)
                    .unwrap_or(0),
                episode: e.get("episode_num").and_then(flex_i64).unwrap_or(0),
                container_extension: e.get("container_extension").and_then(flex_string),
                duration_secs: e
                    .get("info")
                    .and_then(|i| i.get("duration_secs"))
                    .and_then(flex_i64),
            });
        };
        match v.get("episodes") {
            Some(Value::Object(o)) => {
                for (season, list) in o {
                    if let Value::Array(a) = list {
                        for e in a {
                            push(season.parse().ok(), e);
                        }
                    }
                }
            }
            Some(Value::Array(a)) => {
                for e in a {
                    push(None, e);
                }
            }
            _ => {}
        }
        out.episodes.sort_by_key(|e| (e.season, e.episode));
        out
    }
}

fn b64(s: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Panels base64-encode title/description; a value that is not valid base64 is used as is.
fn text(v: Option<&Value>) -> Option<String> {
    let s = v.and_then(flex_string)?;
    let t = b64(&s).unwrap_or(s);
    let t = t.trim().to_owned();
    (!t.is_empty()).then_some(t)
}

/// `epg_listings` -> programmes. Epoch `start_timestamp`/`stop_timestamp` win; the wall-clock
/// `start`/`end` strings are read in the server's timezone only as a fallback.
pub fn epg_listings(v: &Value, server_tz: Option<&str>) -> Vec<Programme> {
    let tz: chrono_tz::Tz = server_tz
        .and_then(|t| t.parse().ok())
        .unwrap_or(chrono_tz::Tz::UTC);
    let wall = |s: Option<&Value>| -> Option<i64> {
        use chrono::TimeZone;
        let s = s.and_then(Value::as_str)?;
        let n = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
        tz.from_local_datetime(&n).earliest().map(|d| d.timestamp())
    };
    let Some(items) = v.get("epg_listings").and_then(Value::as_array) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|e| {
            let start = e
                .get("start_timestamp")
                .and_then(flex_i64)
                .filter(|t| *t > 0)
                .or_else(|| wall(e.get("start")))?;
            let end = e
                .get("stop_timestamp")
                .and_then(flex_i64)
                .filter(|t| *t > 0)
                .or_else(|| wall(e.get("end")))?;
            if end <= start {
                return None;
            }
            Some(Programme {
                channel_ref: e
                    .get("channel_id")
                    .and_then(flex_string)
                    .unwrap_or_default(),
                ext_id: e.get("id").and_then(flex_string),
                start_utc: start,
                end_utc: end,
                title: text(e.get("title")),
                subtitle: None,
                description: text(e.get("description")),
                season: None,
                episode: None,
                // `now_playing` is a snapshot only some panels send; "live now" comes from the times.
                flags: 0,
            })
        })
        .collect()
}
