#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Property tests (the stable-toolchain stand-in for cargo-fuzz; see `fuzz/` for the real fuzz
//! targets): hostile input must never panic, and outputs stay within sane bounds.

use std::io::Write;

use chrono_tz::Tz;
use downlink_core::catchup::template::{CatchupTemplate, ExpandCtx, Mode, TimeSemantics, expand};
use downlink_core::m3u::{parse, parse_attrs, split_pipe_headers};
use downlink_core::redact::redact;
use downlink_core::xmltv::{parse_channels, parse_programmes, parse_time};
use proptest::prelude::*;

const VALID_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><tv><channel id=\"a.test\"><display-name>A &amp; B</display-name></channel>\
<programme start=\"20260101120000 +0000\" stop=\"20260101130000 +0000\" channel=\"a.test\"><title>One</title><desc>Two</desc><episode-num system=\"xmltv_ns\">0.1.</episode-num></programme>\
<programme start=\"20260101130000 +0000\" stop=\"20260101140000 +0000\" channel=\"a.test\"><title>Three</title></programme></tv>";

fn write_tmp(bytes: &[u8]) -> tempfile::NamedTempFile {
    let mut f = tempfile::NamedTempFile::new().unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    f
}

/// Playlist-ish garbage: mixes real tokens with random bytes so the interesting branches run.
fn playlist_like() -> impl Strategy<Value = Vec<u8>> {
    let token = prop_oneof![
        Just(b"#EXTM3U".to_vec()),
        Just(b"#EXTINF:-1 tvg-id=\"x\" group-title=\"G\",Name".to_vec()),
        Just(b"#EXTINF:".to_vec()),
        Just(b"#EXTVLCOPT:http-user-agent=UA".to_vec()),
        Just(b"#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha".to_vec()),
        Just(b"#EXTHTTP:{\"a\":\"b\"}".to_vec()),
        Just(b"#EXTGRP:g".to_vec()),
        Just(b"http://h.test/x.ts|User-Agent=Z&Referer=R".to_vec()),
        Just(b"\xEF\xBB\xBF".to_vec()),
        Just(b"\"".to_vec()),
        Just(b",".to_vec()),
        Just(b"\r".to_vec()),
        Just(b"\n".to_vec()),
        prop::collection::vec(any::<u8>(), 0..40),
    ];
    prop::collection::vec(token, 0..60).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn m3u_parse_never_panics_on_arbitrary_bytes(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let pl = parse(&data[..]).unwrap();
        prop_assert!(pl.stats.entries as usize == pl.entries.len());
        prop_assert!(pl.entries.iter().all(|e| !e.url.trim().is_empty()));
    }

    #[test]
    fn m3u_parse_structured_garbage_stays_bounded(data in playlist_like()) {
        let pl = parse(&data[..]).unwrap();
        // Every entry consumed at least an EXTINF line and a URL line.
        prop_assert!(pl.entries.len() as u64 * 2 <= pl.stats.lines + 1);
        prop_assert!(pl.stats.bytes as usize <= data.len());
    }

    #[test]
    fn attr_scanner_never_panics(s in "\\PC{0,300}") {
        let a = parse_attrs(&s);
        prop_assert!(a.len() <= s.len());
        let _ = split_pipe_headers(&s);
    }

    #[test]
    fn xmltv_parsers_never_panic_on_arbitrary_bytes(data in prop::collection::vec(any::<u8>(), 0..2048)) {
        let f = write_tmp(&data);
        let _ = parse_channels(f.path());
        let _ = parse_programmes(f.path(), Tz::UTC, |_| {});
    }

    #[test]
    fn xmltv_truncation_and_corruption_of_a_valid_doc_never_panic(cut in 0usize..VALID_XML.len(), flips in prop::collection::vec((0usize..VALID_XML.len(), any::<u8>()), 0..6)) {
        let mut bytes = VALID_XML.as_bytes().to_vec();
        for (i, b) in &flips {
            bytes[*i] = *b;
        }
        bytes.truncate(cut);
        let f = write_tmp(&bytes);
        let ch = parse_channels(f.path()).unwrap_or_default();
        prop_assert!(ch.len() <= 1);
        let mut n = 0;
        let _ = parse_programmes(f.path(), Tz::UTC, |_| n += 1);
        prop_assert!(n <= 2);
    }

    #[test]
    fn xmltv_time_parser_never_panics(s in "\\PC{0,40}") {
        let _ = parse_time(&s, Tz::UTC);
        let _ = parse_time(&s, "Europe/Berlin".parse().unwrap());
    }

    #[test]
    fn template_expansion_never_panics(
        src in "\\PC{0,80}",
        start in -4_000_000_000i64..4_000_000_000i64,
        len in -10i64..100_000i64,
        corr in -200_000i64..200_000i64,
        mode in 0u8..4,
        tz in prop_oneof![Just("UTC"), Just("Europe/London"), Just("Nope/Zone"), Just("America/New_York")],
    ) {
        let mode = [Mode::Default, Mode::Append, Mode::Shift, Mode::Xc][mode as usize];
        let t = CatchupTemplate { mode, source: Some(src), time: TimeSemantics::LocalWallclock { tz: tz.into() }, correction_secs: corr, days: 1 };
        let ctx = ExpandCtx { live_url: "http://h.test/live/u/p/1.ts", start_utc: start, end_utc: start + len, now_utc: 0, channel_id: "c", catchup_id: "i", xc: None };
        let _ = expand(&t, &ctx);
    }

    #[test]
    fn redaction_never_panics_and_is_idempotent(s in "\\PC{0,200}") {
        let once = redact(&s);
        prop_assert_eq!(redact(&once), once);
    }
}

#[test]
fn the_valid_document_itself_parses() {
    let f = write_tmp(VALID_XML.as_bytes());
    assert_eq!(parse_channels(f.path()).unwrap().len(), 1);
    let mut n = 0;
    parse_programmes(f.path(), Tz::UTC, |_| n += 1).unwrap();
    assert_eq!(n, 2);
}
