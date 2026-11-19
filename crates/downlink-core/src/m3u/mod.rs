//! M3U lexer (PLAN §6.3). Hand-written, streaming, hostile-input safe:
//! - never reads a whole file into one string; lines are bounded ([`MAX_LINE`]) so a 50 MB single
//!   line costs bounded memory and is counted as dropped;
//! - CRLF, CR-only and LF line endings, UTF-8 BOM, lossy UTF-8 with a Windows-1252 fallback;
//! - per-entry error isolation: a bad entry is dropped and counted, never aborts the parse;
//! - unknown attributes are preserved.

mod attrs;
pub mod load;

use std::io::Read;

pub use attrs::{Attrs, parse_attrs};

/// Longest line kept. Longer lines are skipped (and counted) rather than buffered.
pub const MAX_LINE: usize = 1 << 20;

/// Playlist-level attributes from `#EXTM3U`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct M3uHeader {
    /// `url-tvg` / `x-tvg-url`, which may be a comma list.
    pub tvg_urls: Vec<String>,
    pub url_logo: Option<String>,
    pub tvg_shift: Option<f64>,
    pub max_conn: Option<u32>,
    pub catchup: Option<String>,
    pub catchup_days: Option<i64>,
    pub catchup_source: Option<String>,
    pub catchup_timezone: Option<String>,
    pub attrs: Attrs,
}

/// Aux lines attached to one entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Aux {
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub origin: Option<String>,
    /// `#EXTHTTP:{json}` and pipe headers, in order.
    pub headers: Vec<(String, String)>,
    /// Every `#KODIPROP:key=value`.
    pub kodiprops: Vec<(String, String)>,
    /// Every other `#EXTVLCOPT:key=value` (network-caching etc.); preserved, not acted on.
    pub vlcopts: Vec<(String, String)>,
}

impl Aux {
    /// DRM tags (`license_type`, `license_key`, Widevine/PlayReady/FairPlay…): such a channel is
    /// labelled "DRM — not supported" and excluded from play.
    pub fn is_drm(&self) -> bool {
        self.kodiprops.iter().any(|(k, v)| {
            let k = k.to_ascii_lowercase();
            let v = v.to_ascii_lowercase();
            k.contains("license_type")
                || k.contains("license_key")
                || k.contains("server_certificate")
                || k.contains("drm")
                || ["widevine", "playready", "fairplay", "clearkey"]
                    .iter()
                    .any(|d| v.contains(d))
        })
    }

    /// Per-file HTTP headers for the player (UA, referrer, origin, EXTHTTP, pipe headers).
    pub fn http_headers(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut push = |k: &str, v: &str| {
            if !out
                .iter()
                .any(|(ek, _): &(String, String)| ek.eq_ignore_ascii_case(k))
            {
                out.push((k.to_owned(), v.to_owned()));
            }
        };
        if let Some(v) = &self.user_agent {
            push("User-Agent", v);
        }
        if let Some(v) = &self.referrer {
            push("Referer", v);
        }
        if let Some(v) = &self.origin {
            push("Origin", v);
        }
        for (k, v) in &self.headers {
            push(k, v);
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct M3uEntry {
    pub name: String,
    /// The stream URL with any `|headers` suffix removed (those are in `aux.headers`).
    pub url: String,
    pub duration: Option<f64>,
    pub attrs: Attrs,
    /// `group-title`, else the preceding `#EXTGRP`.
    pub group: Option<String>,
    pub aux: Aux,
}

impl M3uEntry {
    pub fn tvg_id(&self) -> Option<&str> {
        self.attrs.get("tvg-id").filter(|s| !s.trim().is_empty())
    }
    pub fn tvg_name(&self) -> Option<&str> {
        self.attrs.get("tvg-name").filter(|s| !s.trim().is_empty())
    }
    pub fn tvg_chno(&self) -> Option<f64> {
        self.attrs
            .get("tvg-chno")
            .and_then(|s| s.trim().parse().ok())
    }
    pub fn logo(&self) -> Option<&str> {
        self.attrs.get("tvg-logo").filter(|s| !s.trim().is_empty())
    }
    pub fn is_radio(&self) -> bool {
        self.attrs
            .get("radio")
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
    }
    /// `type="playlist"`: an include to fetch and merge.
    pub fn is_include(&self) -> bool {
        self.attrs
            .get("type")
            .is_some_and(|v| v.eq_ignore_ascii_case("playlist"))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub bytes: u64,
    pub lines: u64,
    pub entries: u64,
    /// Entries dropped: EXTINF without a URL, orphan URLs, empty URLs.
    pub dropped: u64,
    /// Lines longer than [`MAX_LINE`] that were skipped.
    pub oversized_lines: u64,
    /// Lines that were not valid UTF-8 (decoded as Windows-1252).
    pub non_utf8_lines: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playlist {
    pub header: M3uHeader,
    pub entries: Vec<M3uEntry>,
    pub stats: ParseStats,
}

/// Bounded line reader: `\n`, `\r\n` and bare `\r` all terminate a line. Returns
/// `Ok(Some((bytes, oversized)))`; an oversized line is consumed (but not buffered beyond the
/// cap) and flagged.
struct Lines<R: Read> {
    r: R,
    buf: Box<[u8; 65536]>,
    pos: usize,
    len: usize,
    eof: bool,
    pending_cr: bool,
}

impl<R: Read> Lines<R> {
    fn new(r: R) -> Self {
        Self {
            r,
            buf: Box::new([0; 65536]),
            pos: 0,
            len: 0,
            eof: false,
            pending_cr: false,
        }
    }

    fn fill(&mut self) -> std::io::Result<()> {
        if self.pos < self.len || self.eof {
            return Ok(());
        }
        loop {
            match self.r.read(&mut self.buf[..]) {
                Ok(0) => self.eof = true,
                Ok(n) => {
                    self.pos = 0;
                    self.len = n;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
            return Ok(());
        }
    }

    fn next_line(&mut self, out: &mut Vec<u8>) -> std::io::Result<Option<(u64, bool)>> {
        out.clear();
        let mut oversized = false;
        let mut consumed: u64 = 0;
        loop {
            self.fill()?;
            if self.pos >= self.len {
                // EOF
                return Ok(if consumed == 0 && out.is_empty() && !oversized {
                    None
                } else {
                    Some((consumed, oversized))
                });
            }
            let b = self.buf[self.pos];
            self.pos += 1;
            consumed += 1;
            if self.pending_cr {
                self.pending_cr = false;
                if b == b'\n' {
                    consumed -= 1; // the \n of a \r\n pair belongs to the previous line
                    continue;
                }
            }
            match b {
                b'\n' => return Ok(Some((consumed, oversized))),
                b'\r' => {
                    self.pending_cr = true;
                    return Ok(Some((consumed, oversized)));
                }
                _ => {
                    if out.len() < MAX_LINE {
                        out.push(b);
                    } else {
                        oversized = true;
                    }
                }
            }
        }
    }
}

fn decode(bytes: &[u8], stats: &mut ParseStats) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => {
            stats.non_utf8_lines += 1;
            let (cow, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            cow.into_owned()
        }
    }
}

/// Split `url|User-Agent=x&Referer=y` into the URL and its headers.
pub fn split_pipe_headers(raw: &str) -> (String, Vec<(String, String)>) {
    let Some((url, tail)) = raw.split_once('|') else {
        return (raw.to_owned(), vec![]);
    };
    let headers = tail
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (canonical_header(k), v.trim().to_owned()))
        })
        .collect();
    (url.to_owned(), headers)
}

fn canonical_header(k: &str) -> String {
    match k.to_ascii_lowercase().as_str() {
        "user-agent" | "useragent" | "http-user-agent" => "User-Agent".into(),
        "referer" | "referrer" | "http-referrer" => "Referer".into(),
        "origin" => "Origin".into(),
        _ => k.to_owned(),
    }
}

fn header_from_attrs(a: &Attrs) -> M3uHeader {
    let mut h = M3uHeader {
        attrs: a.clone(),
        ..Default::default()
    };
    for key in ["url-tvg", "x-tvg-url"] {
        if let Some(v) = a.get(key) {
            h.tvg_urls.extend(
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    h.url_logo = a.get("url-logo").map(str::to_owned);
    h.tvg_shift = a.get("tvg-shift").and_then(|s| s.trim().parse().ok());
    h.max_conn = a.get("max-conn").and_then(|s| s.trim().parse().ok());
    h.catchup = a.get("catchup").map(str::to_owned);
    h.catchup_days = a.get("catchup-days").and_then(|s| s.trim().parse().ok());
    h.catchup_source = a.get("catchup-source").map(str::to_owned);
    h.catchup_timezone = a.get("catchup-timezone").map(str::to_owned);
    h
}

struct Pending {
    duration: Option<f64>,
    attrs: Attrs,
    name: String,
    aux: Aux,
    group_line: Option<String>,
}

/// Parse a playlist from any reader, streaming.
pub fn parse<R: Read>(r: R) -> std::io::Result<Playlist> {
    let mut lines = Lines::new(r);
    let mut pl = Playlist::default();
    let mut buf = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut group_line: Option<String> = None;
    let mut first = true;
    let mut have_header = false;

    while let Some((consumed, oversized)) = lines.next_line(&mut buf)? {
        pl.stats.bytes += consumed;
        pl.stats.lines += 1;
        if oversized {
            pl.stats.oversized_lines += 1;
            // An oversized line cannot be a usable entry; if an EXTINF was pending, it is lost.
            if pending.take().is_some() {
                pl.stats.dropped += 1;
            }
            continue;
        }
        let mut bytes: &[u8] = &buf;
        if first {
            first = false;
            if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
                bytes = rest;
            }
        }
        let line = decode(bytes, &mut pl.stats);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let upper = rest.to_ascii_uppercase();
            if upper.starts_with("EXTM3U") {
                if !have_header {
                    pl.header = header_from_attrs(&parse_attrs(&rest[6..]));
                    have_header = true;
                }
            } else if upper.starts_with("EXTINF:") {
                if pending.take().is_some() {
                    pl.stats.dropped += 1; // previous EXTINF never got a URL
                }
                let (duration, attrs, name) = parse_extinf(&rest[7..]);
                pending = Some(Pending {
                    duration,
                    attrs,
                    name,
                    aux: Aux::default(),
                    group_line: group_line.take(),
                });
            } else if upper.starts_with("EXTGRP:") {
                let g = rest[7..].trim();
                if !g.is_empty() {
                    match pending.as_mut() {
                        Some(p) => p.group_line = Some(g.to_owned()),
                        None => group_line = Some(g.to_owned()),
                    }
                }
            } else if upper.starts_with("EXTVLCOPT:") {
                if let Some(p) = pending.as_mut() {
                    apply_vlcopt(&mut p.aux, &rest[10..]);
                }
            } else if upper.starts_with("EXTHTTP:") {
                if let Some(p) = pending.as_mut() {
                    apply_exthttp(&mut p.aux, &rest[8..]);
                }
            } else if upper.starts_with("KODIPROP:")
                && let Some(p) = pending.as_mut()
                && let Some((k, v)) = rest[9..].split_once('=')
            {
                p.aux
                    .kodiprops
                    .push((k.trim().to_owned(), v.trim().to_owned()));
            }
            // Any other `#...` line is a comment (HLS tags are mpv's business, not ours).
            continue;
        }
        // A URL line.
        match pending.take() {
            Some(p) => {
                let (url, pipe) = split_pipe_headers(line);
                if url.trim().is_empty() {
                    pl.stats.dropped += 1;
                    continue;
                }
                let mut aux = p.aux;
                aux.headers.extend(pipe);
                let group = p
                    .attrs
                    .get("group-title")
                    .filter(|g| !g.trim().is_empty())
                    .map(str::to_owned)
                    .or(p.group_line);
                pl.entries.push(M3uEntry {
                    name: p.name,
                    url,
                    duration: p.duration,
                    attrs: p.attrs,
                    group,
                    aux,
                });
                pl.stats.entries += 1;
            }
            None => pl.stats.dropped += 1, // orphan URL
        }
    }
    if pending.is_some() {
        pl.stats.dropped += 1; // truncated: EXTINF at EOF
    }
    Ok(pl)
}

/// Parse `#EXTINF:` payload: `<dur> <attrs>,<name>`.
fn parse_extinf(s: &str) -> (Option<f64>, Attrs, String) {
    let s = s.trim_start();
    let dur_end = s
        .find(|c: char| c.is_whitespace() || c == ',')
        .unwrap_or(s.len());
    let duration = s[..dur_end].parse::<f64>().ok();
    let rest = &s[dur_end..];
    let comma = find_name_comma(rest);
    let (attr_part, name) = match comma {
        Some(i) => (&rest[..i], rest[i + 1..].trim()),
        None => (rest, ""),
    };
    let attrs = parse_attrs(attr_part);
    let name = if name.is_empty() {
        attrs.get("tvg-name").unwrap_or("").to_owned()
    } else {
        name.to_owned()
    };
    (duration, attrs, name)
}

/// The first comma outside quotes; if the quotes do not balance, the first comma at all.
fn find_name_comma(s: &str) -> Option<usize> {
    let mut in_q = false;
    let mut first_any = None;
    let mut first_outside = None;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_q = !in_q,
            ',' => {
                first_any.get_or_insert(i);
                if !in_q && first_outside.is_none() {
                    first_outside = Some(i);
                }
            }
            _ => {}
        }
    }
    if in_q { first_any } else { first_outside }
}

fn apply_vlcopt(aux: &mut Aux, s: &str) {
    let Some((k, v)) = s.split_once('=') else {
        return;
    };
    let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
    match k.as_str() {
        "http-user-agent" => aux.user_agent = Some(v),
        "http-referrer" | "http-referer" => aux.referrer = Some(v),
        "http-origin" => aux.origin = Some(v),
        _ => aux.vlcopts.push((k, v)),
    }
}

fn apply_exthttp(aux: &mut Aux, s: &str) {
    if let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(s.trim()) {
        for (k, v) in o {
            if let Some(v) = v.as_str() {
                aux.headers.push((canonical_header(&k), v.to_owned()));
            }
        }
    }
}

#[cfg(test)]
mod tests;
