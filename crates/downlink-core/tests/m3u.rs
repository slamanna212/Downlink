#![allow(clippy::unwrap_used, clippy::expect_used)]
//! M3U integration tests against tools/fixtures/m3u (synthetic) and generated large playlists.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use async_trait::async_trait;
use downlink_core::m3u::load::{self, Fetched, Fetcher, Location};
use downlink_core::m3u::parse;
use downlink_core::util::peak_rss_kib;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/m3u")
        .join(name)
}

#[test]
fn basic_fixture() {
    let pl = parse(std::fs::File::open(fx("basic.m3u")).unwrap()).unwrap();
    assert!(!pl.entries.is_empty());
    assert!(
        pl.entries
            .iter()
            .all(|e| !e.url.is_empty() && !e.name.is_empty())
    );
    assert_eq!(pl.stats.entries as usize, pl.entries.len());
}

#[test]
fn aux_lines_fixture() {
    let pl = parse(std::fs::File::open(fx("aux_lines.m3u")).unwrap()).unwrap();
    let by_id = |id: &str| pl.entries.iter().find(|e| e.tvg_id() == Some(id)).unwrap();
    assert_eq!(
        by_id("ua.test").aux.user_agent.as_deref(),
        Some("SyntheticAgent/1.0")
    );
    assert!(by_id("drm.test").aux.is_drm());
    assert!(!by_id("ua.test").aux.is_drm());
    assert_eq!(
        by_id("grp.test").group.as_deref(),
        Some("Group From EXTGRP")
    );
    let pipe = by_id("hdr.test");
    assert_eq!(pipe.url, "http://iptv.test/hdr/1.ts");
    assert!(
        pipe.aux
            .http_headers()
            .iter()
            .any(|(k, v)| k == "User-Agent" && v == "SyntheticAgent/2.0")
    );
}

#[test]
fn catchup_fixture_attributes() {
    let pl = parse(std::fs::File::open(fx("catchup.m3u")).unwrap()).unwrap();
    assert_eq!(pl.header.catchup.as_deref(), Some("default"));
    assert_eq!(pl.header.catchup_timezone.as_deref(), Some("UTC"));
    let e = pl
        .entries
        .iter()
        .find(|e| e.tvg_id() == Some("cu4.test"))
        .unwrap();
    assert_eq!(e.attrs.get("catchup"), Some("xc"));
    assert_eq!(e.attrs.get("catchup-correction"), Some("-1"));
}

#[test]
fn malformed_fixture_never_aborts() {
    let pl = parse(std::fs::File::open(fx("malformed.m3u")).unwrap()).unwrap();
    assert!(
        pl.stats.dropped >= 2,
        "dropped entries are counted: {:?}",
        pl.stats
    );
    assert!(
        pl.entries.iter().any(|e| e.tvg_id() == Some("crlf.test")),
        "BOM + CRLF handled"
    );
    assert!(
        pl.entries.iter().any(|e| e.tvg_id() == Some("after.test")),
        "entry after a URL-less EXTINF survives"
    );
}

struct MemFetcher(HashMap<String, String>);

#[async_trait]
impl Fetcher for MemFetcher {
    async fn fetch(&self, url: &str) -> downlink_core::Result<Fetched> {
        let body = self
            .0
            .get(url)
            .ok_or_else(|| downlink_core::AppError::network("nope"))?;
        let mut t = tempfile::NamedTempFile::new().unwrap();
        t.write_all(body.as_bytes()).unwrap();
        Ok(Fetched::Temp(t))
    }
}

#[tokio::test]
async fn includes_merge_with_cycle_detection_and_depth_limit() {
    let mut m = HashMap::new();
    m.insert("http://a.test/sub1.m3u".to_owned(), "#EXTINF:-1,S1\nhttp://s.test/s1\n#EXTINF:-1 type=\"playlist\",Cycle\nhttp://a.test/sub1.m3u\n#EXTINF:-1 type=\"playlist\",Deeper\nhttp://a.test/sub2.m3u\n".to_owned());
    m.insert("http://a.test/sub2.m3u".to_owned(), "#EXTINF:-1,S2\nhttp://s.test/s2\n#EXTINF:-1 type=\"playlist\",TooDeep\nhttp://a.test/sub3.m3u\n".to_owned());
    m.insert(
        "http://a.test/sub3.m3u".to_owned(),
        "#EXTINF:-1,S3\nhttp://s.test/s3\n".to_owned(),
    );
    let dir = tempfile::tempdir().unwrap();
    let top = dir.path().join("top.m3u");
    std::fs::write(&top, "#EXTM3U\n#EXTINF:-1,Top\nhttp://s.test/top\n#EXTINF:-1 type=\"playlist\",Inc\nhttp://a.test/sub1.m3u\n#EXTINF:-1 type=\"playlist\",Missing\nhttp://a.test/missing.m3u\n#EXTINF:-1 type=\"playlist\",Local\nfile:///etc/passwd\n").unwrap();
    let (pl, st) = load::load(&Location::File(top), &MemFetcher(m))
        .await
        .unwrap();
    let names: Vec<&str> = pl.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["Top", "S1", "S2"],
        "depth 2 reached, depth 3 refused"
    );
    assert_eq!(st.includes_followed, 2);
    assert_eq!(
        st.includes_skipped, 4,
        "self-include cycle, too deep, missing, and a non-http include"
    );
}

#[test]
fn location_parsing() {
    assert!(matches!(
        Location::parse("http://x.test/p.m3u").unwrap(),
        Location::Url(_)
    ));
    assert!(matches!(
        Location::parse("file:///tmp/p.m3u").unwrap(),
        Location::File(_)
    ));
    assert!(matches!(
        Location::parse("/tmp/p.m3u").unwrap(),
        Location::File(_)
    ));
    assert!(Location::parse("ftp://x").is_err());
    assert!(Location::parse("relative.m3u").is_err());
}

/// PLAN §6.8: 150,000 entries in < 3 s with < 300 MB peak RSS. Generated in-test to a temp file.
#[tokio::test]
async fn huge_playlist_is_fast_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.m3u");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        writeln!(f, "#EXTM3U x-tvg-url=\"http://iptv.test/epg.xml\"").unwrap();
        for i in 1..=150_000u32 {
            writeln!(f, "#EXTINF:-1 tvg-id=\"gen-{i}.test\" tvg-name=\"Generated {i}\" tvg-logo=\"http://iptv.test/logo/{i}.png\" tvg-chno=\"{i}\" group-title=\"Group {}\",Generated Channel {i}\nhttp://iptv.test/live/{i}.ts", i % 200).unwrap();
        }
    }
    let before = peak_rss_kib();
    let t = std::time::Instant::now();
    let pl = load::parse_file(path).await.unwrap();
    let el = t.elapsed();
    assert_eq!(pl.entries.len(), 150_000);
    let budget = if cfg!(debug_assertions) { 20 } else { 3 }; // debug builds are ~10x slower
    assert!(el.as_secs() < budget, "took {el:?}");
    if let (Some(b), Some(a)) = (before, peak_rss_kib()) {
        let grown_mb = a.saturating_sub(b) / 1024;
        assert!(grown_mb < 300, "peak RSS grew by {grown_mb} MB");
        println!("150k entries: {el:?}, RSS +{grown_mb} MB");
    }
}

#[tokio::test]
async fn generated_20k_fixture_if_present() {
    let p = fx("generated/large.m3u");
    if !p.exists() {
        return; // generated by tools/fixtures/gen-large-m3u.py, gitignored
    }
    let pl = load::parse_file(p).await.unwrap();
    assert_eq!(pl.entries.len(), 20_000);
    assert_eq!(pl.header.tvg_urls, ["http://iptv.test/epg.xml"]);
}

#[tokio::test]
async fn include_breadth_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let top = dir.path().join("wide.m3u");
    let mut entries = String::new();
    for i in 0..100 {
        entries.push_str(&format!(
            "#EXTINF:-1 type=\"playlist\",Include\nhttp://example.test/{i}\n"
        ));
    }
    std::fs::write(&top, entries).unwrap();
    let error = load::load(&Location::File(top), &MemFetcher(HashMap::new()))
        .await
        .unwrap_err();
    assert_eq!(
        error.message,
        "That playlist includes too many other playlists."
    );
}
