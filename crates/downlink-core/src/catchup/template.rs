//! Catch-up URL template engine (PLAN §5.3): `default`, `append`, `shift`, `xc`.
//!
//! Time semantics are **data** on the template ([`TimeSemantics`]), never inferred from the URL
//! shape; that is what prevents "catch-up plays the wrong hour". All instants are UTC seconds taken
//! from the programme row; the template only decides how to *render* them.
//!
//! Placeholders (case-sensitive, braces):
//! - `{utc}` / `{start}`: start, epoch seconds. `{utcend}` / `{end}`: end, epoch seconds.
//! - `{utc:FMT}`, `{start:FMT}`, `{utcend:FMT}`, `{end:FMT}`: strftime format in the template's
//!   zone (UTC for `utc_seconds`).
//! - `{Y}{m}{d}{H}{M}{S}`: start broken down in the template's zone.
//! - `{duration}`: seconds; `{duration:N}`: seconds divided by N (Kodi convention, so
//!   `{duration:60}` is minutes; PLAN §5.3's prose says "seconds", which contradicts Kodi and the
//!   providers' own templates; see the engine tests).
//! - `{lutc}` / `{timestamp}`: now, epoch seconds; `{offset}`: now minus start, seconds.
//! - `{channel-id}`, `{catchup-id}`.
//!
//! `catchup-correction` (hours) shifts start and end before expansion.

use chrono::TimeZone;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::m3u::{Attrs, M3uHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Default,
    Append,
    Shift,
    Xc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimeSemantics {
    /// Render in UTC.
    UtcSeconds,
    /// Render as wall-clock time in an IANA zone (e.g. Xtream `server_info.timezone`).
    LocalWallclock { tz: String },
}

impl TimeSemantics {
    fn zone(&self) -> Tz {
        match self {
            Self::UtcSeconds => Tz::UTC,
            Self::LocalWallclock { tz } => tz.parse().unwrap_or(Tz::UTC),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchupTemplate {
    pub mode: Mode,
    /// `catchup-source` (default/append).
    #[serde(default)]
    pub source: Option<String>,
    pub time: TimeSemantics,
    /// `catchup-correction`, converted to seconds.
    #[serde(default)]
    pub correction_secs: i64,
    /// `catchup-days`: bounds what the guide offers.
    #[serde(default)]
    pub days: i64,
}

impl CatchupTemplate {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }

    /// Build from an M3U entry (falling back to header-level defaults). `None` when the entry has
    /// no usable catch-up (no mode, unsupported mode such as `flussonic`, or `default` without a
    /// source).
    pub fn from_m3u(entry: &Attrs, header: &M3uHeader, default_tz: Option<&str>) -> Option<Self> {
        let get = |k: &str| entry.get(k).map(str::trim).filter(|v| !v.is_empty());
        // Dispatcharr's own attributes: tv_archive="1" + tv_archive_duration="N".
        let archive = get("tv_archive") == Some("1");
        let mode_s = get("catchup")
            .or(header.catchup.as_deref())
            .map(str::to_ascii_lowercase);
        let source = get("catchup-source")
            .map(str::to_owned)
            .or_else(|| header.catchup_source.clone());
        let days = get("catchup-days")
            .and_then(|d| d.parse().ok())
            .or_else(|| {
                archive
                    .then(|| get("tv_archive_duration").and_then(|d| d.parse().ok()))
                    .flatten()
            })
            .or(header.catchup_days)
            .unwrap_or(0);
        let mode = match mode_s.as_deref() {
            Some("default") => Mode::Default,
            Some("append") => Mode::Append,
            Some("shift") => Mode::Shift,
            Some("xc") => Mode::Xc,
            Some(_) => return None, // flussonic etc.: out of scope
            None => return None,
        };
        if matches!(mode, Mode::Default | Mode::Append) && source.is_none() {
            return None;
        }
        let tz = get("catchup-timezone")
            .map(str::to_owned)
            .or_else(|| header.catchup_timezone.clone())
            .or_else(|| default_tz.map(str::to_owned));
        let time = match (mode, tz) {
            (Mode::Xc, Some(tz)) => TimeSemantics::LocalWallclock { tz },
            (_, Some(tz)) if !tz.eq_ignore_ascii_case("utc") => {
                TimeSemantics::LocalWallclock { tz }
            }
            _ => TimeSemantics::UtcSeconds,
        };
        let correction_secs = get("catchup-correction")
            .and_then(|c| c.parse::<f64>().ok())
            .map_or(0, |h| (h * 3600.0).round() as i64);
        Some(Self {
            mode,
            source,
            time,
            correction_secs,
            days,
        })
    }
}

/// Parts of an Xtream-style URL, for the `xc` mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XcParts {
    /// `scheme://host[:port]` with no trailing slash.
    pub base: String,
    pub username: String,
    pub password: String,
    pub stream_id: String,
}

impl XcParts {
    /// Recover the parts from a live URL `…/live/<user>/<pass>/<id>.<ext>`.
    pub fn from_live_url(url: &str) -> Option<Self> {
        let u = url::Url::parse(url).ok()?;
        let seg: Vec<&str> = u.path_segments()?.collect();
        let i = seg.iter().position(|s| *s == "live")?;
        let (user, pass, id) = (seg.get(i + 1)?, seg.get(i + 2)?, seg.get(i + 3)?);
        let id = id.rsplit_once('.').map_or(*id, |(a, _)| a);
        let mut base = format!("{}://{}", u.scheme(), u.host_str()?);
        if let Some(p) = u.port() {
            base.push_str(&format!(":{p}"));
        }
        Some(Self {
            base,
            username: (*user).into(),
            password: (*pass).into(),
            stream_id: id.into(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ExpandCtx<'a> {
    pub live_url: &'a str,
    pub start_utc: i64,
    pub end_utc: i64,
    /// The clock: injected so tests use a fake one.
    pub now_utc: i64,
    pub channel_id: &'a str,
    pub catchup_id: &'a str,
    /// For `xc` mode when there is no live URL to take them from (Xtream sources).
    pub xc: Option<XcParts>,
}

fn unsupported(what: &str) -> AppError {
    AppError::stream(format!(
        "This channel's archive link uses an unsupported setting ({what})."
    ))
}

fn zoned(t: i64, tz: Tz) -> Result<chrono::DateTime<Tz>> {
    tz.timestamp_opt(t, 0)
        .single()
        .ok_or_else(|| unsupported("time out of range"))
}

fn fmt_in(t: i64, tz: Tz, f: &str) -> Result<String> {
    use std::fmt::Write;
    let mut out = String::new();
    write!(out, "{}", zoned(t, tz)?.format(f)).map_err(|_| unsupported("time format"))?;
    Ok(out)
}

/// Expand placeholders in `src`.
fn fill(src: &str, t: &CatchupTemplate, c: &ExpandCtx<'_>, start: i64, end: i64) -> Result<String> {
    let tz = t.time.zone();
    let mut out = String::with_capacity(src.len() + 32);
    let mut rest = src;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let tok = &after[..j];
        rest = &after[j + 1..];
        let (name, arg) = match tok.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (tok, None),
        };
        let epoch_or_fmt = |v: i64| -> Result<String> {
            match arg {
                Some(f) => fmt_in(v, tz, f),
                None => Ok(v.to_string()),
            }
        };
        let piece = match name {
            "utc" | "start" => epoch_or_fmt(start)?,
            "utcend" | "end" => epoch_or_fmt(end)?,
            "lutc" | "timestamp" => c.now_utc.to_string(),
            "offset" => (c.now_utc - start).to_string(),
            "duration" => {
                let secs = end - start;
                match arg {
                    None => secs.to_string(),
                    Some(n) => {
                        let n: i64 = n.parse().map_err(|_| unsupported("duration divisor"))?;
                        if n <= 0 {
                            return Err(unsupported("duration divisor"));
                        }
                        (secs / n).to_string()
                    }
                }
            }
            "channel-id" => c.channel_id.to_owned(),
            "catchup-id" => c.catchup_id.to_owned(),
            "Y" => fmt_in(start, tz, "%Y")?,
            "m" => fmt_in(start, tz, "%m")?,
            "d" => fmt_in(start, tz, "%d")?,
            "H" => fmt_in(start, tz, "%H")?,
            "M" => fmt_in(start, tz, "%M")?,
            "S" => fmt_in(start, tz, "%S")?,
            _ => return Err(unsupported(&format!("placeholder {{{name}}}"))),
        };
        out.push_str(&piece);
    }
    out.push_str(rest);
    Ok(out)
}

/// Build the archive URL for a programme.
pub fn expand(t: &CatchupTemplate, c: &ExpandCtx<'_>) -> Result<String> {
    if c.end_utc <= c.start_utc {
        return Err(AppError::stream("This programme has no valid time range."));
    }
    let start = c.start_utc + t.correction_secs;
    let end = c.end_utc + t.correction_secs;
    match t.mode {
        Mode::Default => {
            let src = t
                .source
                .as_deref()
                .ok_or_else(|| unsupported("missing catch-up source"))?;
            fill(src, t, c, start, end)
        }
        Mode::Append => {
            let src = t
                .source
                .as_deref()
                .ok_or_else(|| unsupported("missing catch-up source"))?;
            let mut tail = fill(src, t, c, start, end)?;
            if c.live_url.contains('?') && tail.starts_with('?') {
                tail.replace_range(0..1, "&");
            }
            Ok(format!("{}{}", c.live_url, tail))
        }
        Mode::Shift => {
            let sep = if c.live_url.contains('?') { '&' } else { '?' };
            Ok(format!("{}{sep}utc={start}&lutc={}", c.live_url, c.now_utc))
        }
        Mode::Xc => {
            let parts = match &c.xc {
                Some(p) => p.clone(),
                None => XcParts::from_live_url(c.live_url)
                    .ok_or_else(|| unsupported("not an Xtream-style live URL"))?,
            };
            let minutes = ((end - start) + 59) / 60;
            let when = fmt_in(start, t.time.zone(), "%Y-%m-%d:%H-%M")?;
            Ok(format!(
                "{}/timeshift/{}/{}/{minutes}/{when}/{}.ts",
                parts.base, parts.username, parts.password, parts.stream_id
            ))
        }
    }
}

#[cfg(test)]
mod tests;
