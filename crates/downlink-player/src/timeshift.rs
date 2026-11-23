//! Live timeshift bookkeeping: the seekable range of mpv's demuxer cache and the anchor that maps
//! media time to wall-clock time. Pure functions; the player feeds them property values.

use serde::Serialize;

/// One point where media time and wall-clock time are known to line up. Media time advances at
/// real-time speed, so `utc(t) = anchor.utc - (anchor.media - t)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAnchor {
    pub media_secs: f64,
    pub utc_ms: i64,
}

/// Re-anchor when the cache end would otherwise sit this far in the future.
const AHEAD_TOLERANCE_SECS: f64 = 1.0;
/// A cache end this far behind the anchor is a timestamp discontinuity (reconnect, new stream).
const BACKWARDS_JUMP_SECS: f64 = 30.0;

/// The seekable range of `demuxer-cache-state` (JSON) that holds `position`, or the last range.
pub fn parse_seek_range(json: &str, position: Option<f64>) -> Option<(f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let ranges: Vec<(f64, f64)> = v
        .get("seekable-ranges")?
        .as_array()?
        .iter()
        .filter_map(|r| {
            let s = r.get("start")?.as_f64()?;
            let e = r.get("end")?.as_f64()?;
            (s.is_finite() && e.is_finite() && e >= s).then_some((s, e))
        })
        .collect();
    if let Some(p) = position
        && let Some(r) = ranges.iter().find(|(s, e)| p >= *s - 0.5 && p <= *e + 0.5)
    {
        return Some(*r);
    }
    ranges.last().copied()
}

/// Update the anchor for a new cache end seen at `now_ms`. The first value anchors; after that the
/// anchor only moves when the cache end would land in the future (data arrived faster than real
/// time: the initial burst) or jumped backwards (discontinuity). A stall keeps the anchor, so the
/// gap to live grows honestly.
pub fn advance_anchor(prev: Option<LiveAnchor>, cache_end: f64, now_ms: i64) -> LiveAnchor {
    let fresh = LiveAnchor {
        media_secs: cache_end,
        utc_ms: now_ms,
    };
    let Some(a) = prev else {
        return fresh;
    };
    if cache_end < a.media_secs - BACKWARDS_JUMP_SECS {
        return fresh;
    }
    let predicted = media_to_utc(a, cache_end);
    if predicted > now_ms as f64 / 1000.0 + AHEAD_TOLERANCE_SECS {
        return fresh;
    }
    a
}

/// Wall-clock UTC seconds of media time `t`.
pub fn media_to_utc(a: LiveAnchor, t: f64) -> f64 {
    a.utc_ms as f64 / 1000.0 - (a.media_secs - t)
}

/// Media time of wall-clock UTC seconds `utc`.
pub fn utc_to_media(a: LiveAnchor, utc: f64) -> f64 {
    a.media_secs - (a.utc_ms as f64 / 1000.0 - utc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_range_holding_the_position() {
        let j =
            r#"{"seekable-ranges":[{"start":0,"end":10},{"start":20,"end":50}],"cache-end":50}"#;
        assert_eq!(parse_seek_range(j, Some(5.0)), Some((0.0, 10.0)));
        assert_eq!(parse_seek_range(j, Some(30.0)), Some((20.0, 50.0)));
        assert_eq!(parse_seek_range(j, None), Some((20.0, 50.0)));
        assert_eq!(parse_seek_range(j, Some(15.0)), Some((20.0, 50.0)));
    }

    #[test]
    fn bad_cache_state_is_none() {
        assert_eq!(parse_seek_range("{}", None), None);
        assert_eq!(parse_seek_range("nope", None), None);
        assert_eq!(parse_seek_range(r#"{"seekable-ranges":[]}"#, None), None);
    }

    #[test]
    fn anchor_maps_both_ways() {
        let a = LiveAnchor {
            media_secs: 100.0,
            utc_ms: 1_000_000,
        };
        assert_eq!(media_to_utc(a, 90.0), 990.0);
        assert_eq!(utc_to_media(a, 990.0), 90.0);
    }

    #[test]
    fn anchor_holds_through_real_time_growth_and_stalls() {
        let a = advance_anchor(None, 10.0, 1_000_000);
        // 5 s later, 5 s more media: same anchor.
        assert_eq!(advance_anchor(Some(a), 15.0, 1_005_000), a);
        // Stall: 10 s later, no new media.
        assert_eq!(advance_anchor(Some(a), 15.0, 1_010_000), a);
    }

    #[test]
    fn burst_and_discontinuity_reanchor() {
        let a = advance_anchor(None, 10.0, 1_000_000);
        // Initial burst: 38 s of media in 1 s.
        let b = advance_anchor(Some(a), 48.0, 1_001_000);
        assert_eq!(b.media_secs, 48.0);
        assert_eq!(b.utc_ms, 1_001_000);
        // Timestamps reset.
        let c = advance_anchor(Some(b), 0.5, 1_002_000);
        assert_eq!(c.media_secs, 0.5);
    }
}
