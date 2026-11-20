//! SourceItem -> store rows (PLAN §6.6): quality suffixes into a `quality` field, stable ids,
//! sort keys. No filtering here; adult/radio/hidden/DRM live only in `catalog/`.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::source::{SourceChannel, StreamMode};
use crate::store::ChannelUpsert;

static QUALITY_TAIL: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)[\s|:\-_]*(?:\[|\()?\b(UHD|FHD|HD|SD|4K|8K|HEVC|H\.?265|2160p|1080[pi]|720p|576[pi]|480p)\b(?:\]|\))?\s*$",
    )
    .ok()
});

fn canonical_quality(q: &str) -> String {
    let u = q.to_ascii_uppercase();
    match u.as_str() {
        "2160P" => "4K".into(),
        "1080P" | "1080I" => "FHD".into(),
        "720P" => "HD".into(),
        "576P" | "576I" | "480P" => "SD".into(),
        "H.265" | "H265" => "HEVC".into(),
        _ => u,
    }
}

/// Strip trailing quality markers (possibly several, e.g. "News HD HEVC") into a quality label.
/// The first (rightmost-stripped-last) marker found closest to the name wins as the label
/// ("HD" in "News HD HEVC").
pub fn split_quality(name: &str) -> (String, Option<String>) {
    let mut cur = name.trim().to_owned();
    let mut found: Option<String> = None;
    if let Some(re) = QUALITY_TAIL.as_ref() {
        while let Some(c) = re.captures(&cur) {
            let (Some(whole), Some(q)) = (c.get(0), c.get(1)) else {
                break;
            };
            let label = canonical_quality(q.as_str());
            let start = whole.start();
            if start == 0 {
                break; // never strip the entire name
            }
            found = Some(label);
            cur.truncate(start);
            cur = cur.trim_end().to_owned();
        }
    }
    (cur, found)
}

/// Stable synthetic id: `tvg-id` if present, else `hash(name|group|url-host)` (PLAN §6.6).
pub fn synthetic_id(tvg_id: Option<&str>, name: &str, group: &str, url_host: &str) -> String {
    if let Some(t) = tvg_id.map(str::trim).filter(|t| !t.is_empty()) {
        return t.to_owned();
    }
    let mut h = Sha256::new();
    h.update(name.trim().to_lowercase().as_bytes());
    h.update(b"|");
    h.update(group.trim().to_lowercase().as_bytes());
    h.update(b"|");
    h.update(url_host.trim().to_lowercase().as_bytes());
    let digest = h.finalize();
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Make base ids unique within one refresh: the first holder keeps the id, later duplicates get
/// `#2`, `#3`… in playlist order, so a refresh of an unchanged playlist maps rows to the same ids.
pub fn stable_ids(base: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    base.iter()
        .map(|b| {
            let n = seen.entry(b.as_str()).or_insert(0);
            *n += 1;
            if *n == 1 {
                b.clone()
            } else {
                format!("{b}#{n}")
            }
        })
        .collect()
}

pub fn sort_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Build the row written by sync. `stream_redirect` is the channel's *effective* profile.
pub fn channel_row(c: &SourceChannel, stream_redirect: StreamMode) -> ChannelUpsert {
    let (clean, quality) = split_quality(&c.name);
    let tvg = c
        .tvg_id
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    ChannelUpsert {
        ext_id: c.ext_id.clone(),
        uuid: c.uuid.clone(),
        number: c.number,
        sort_key: Some(sort_key(&clean)),
        name: clean,
        group_ext_id: c.group_ext_id.clone(),
        epg_match: Some(if tvg.is_some() { "id" } else { "none" }.to_owned()),
        tvg_id: tvg,
        tvg_name: c.tvg_name.clone(),
        logo_key: c.logo_key.clone(),
        quality,
        is_adult: c.is_adult,
        is_radio: c.is_radio,
        is_drm: c.is_drm,
        epg_shift_secs: c.epg_shift_secs,
        catchup_days: if c.is_catchup {
            c.catchup_days.max(0)
        } else {
            0
        },
        catchup_template: c.catchup_template.clone(),
        stream_redirect,
        play_url: c.play_url.clone(),
        play_headers: c.play_headers.clone(),
        streams: c.streams.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_variants() {
        assert_eq!(
            split_quality("Channel One HD"),
            ("Channel One".into(), Some("HD".into()))
        );
        assert_eq!(
            split_quality("Channel One [FHD]"),
            ("Channel One".into(), Some("FHD".into()))
        );
        assert_eq!(
            split_quality("Channel One (4K)"),
            ("Channel One".into(), Some("4K".into()))
        );
        assert_eq!(
            split_quality("Channel One | UHD"),
            ("Channel One".into(), Some("UHD".into()))
        );
        assert_eq!(
            split_quality("Channel One 1080p"),
            ("Channel One".into(), Some("FHD".into()))
        );
        assert_eq!(
            split_quality("News HD HEVC"),
            ("News".into(), Some("HD".into()))
        );
        assert_eq!(split_quality("Channel One"), ("Channel One".into(), None));
        assert_eq!(
            split_quality("HD"),
            ("HD".into(), None),
            "never strips the whole name"
        );
        assert_eq!(split_quality("SHD Sports"), ("SHD Sports".into(), None));
    }

    #[test]
    fn synthetic_ids_are_stable() {
        assert_eq!(synthetic_id(Some("one.test"), "x", "y", "z"), "one.test");
        let a = synthetic_id(None, "Channel One", "News", "p.test");
        assert_eq!(
            a,
            synthetic_id(Some("  "), " channel one ", "NEWS", "P.TEST")
        );
        assert_ne!(a, synthetic_id(None, "Channel Two", "News", "p.test"));
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn row_building() {
        let c = SourceChannel {
            ext_id: "1".into(),
            name: "Channel One HD".into(),
            tvg_id: Some(" ".into()),
            is_catchup: true,
            catchup_days: 7,
            ..Default::default()
        };
        let r = channel_row(&c, StreamMode::Redirect);
        assert_eq!(r.name, "Channel One");
        assert_eq!(r.quality.as_deref(), Some("HD"));
        assert_eq!(r.tvg_id, None);
        assert_eq!(r.epg_match.as_deref(), Some("none"));
        assert_eq!(r.catchup_days, 7);
        assert_eq!(r.stream_redirect, StreamMode::Redirect);
        assert_eq!(r.sort_key.as_deref(), Some("channel one"));
    }
}
