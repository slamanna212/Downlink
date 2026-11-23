#![allow(clippy::unwrap_used, clippy::expect_used)]
//! XMLTV ingestion + matching against tools/fixtures/xmltv.

use std::path::PathBuf;

use chrono_tz::Tz;
use downlink_core::source::TimeWindow;
use downlink_core::store::{ChannelUpsert, NewSource, Store};
use downlink_core::xmltv::ingest::{apply_matches, ingest};
use downlink_core::xmltv::parse_channels;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/xmltv")
        .join(name)
}

const ALL: TimeWindow = TimeWindow {
    start_utc: 0,
    end_utc: i64::MAX,
};

async fn setup(chans: &[(&str, Option<&str>, Option<&str>, &str)]) -> (Store, i64) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "T".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let g = store.sync_groups(src.id, &[]).await.unwrap();
    let rows: Vec<ChannelUpsert> = chans
        .iter()
        .map(|(ext, tvg, tvg_name, name)| ChannelUpsert {
            ext_id: (*ext).into(),
            tvg_id: tvg.map(Into::into),
            tvg_name: tvg_name.map(Into::into),
            name: (*name).into(),
            ..Default::default()
        })
        .collect();
    store.sync_channels(src.id, &rows, &g).await.unwrap();
    (store, src.id)
}

async fn epg_match(store: &Store, sid: i64, ext: &str) -> String {
    store
        .channel_by_ext(sid, ext)
        .await
        .unwrap()
        .unwrap()
        .epg_match
        .unwrap()
}

#[tokio::test]
async fn edge_case_feed_ingests_matched_channels_only() {
    let (store, sid) = setup(&[
        ("1", Some("a.test"), None, "Channel A"),
        ("2", None, Some("Channel B & Co"), "B"),
        ("3", None, None, "Nobody"),
    ])
    .await;
    let r = ingest(&store, sid, fx("edge_cases.xml"), Tz::UTC, ALL)
        .await
        .unwrap();
    assert_eq!(
        (
            r.matches.by_id,
            r.matches.by_name,
            r.matches.unmatched.len()
        ),
        (1, 1, 1)
    );
    assert_eq!(r.matches.unmatched, ["Nobody"]);
    assert!(
        r.unmatched_programmes >= 1,
        "unknown.test + empty-name.test: {r:?}"
    );
    assert!(r.dropped >= 5);
    assert_eq!(epg_match(&store, sid, "1").await, "id");
    assert_eq!(
        epg_match(&store, sid, "2").await,
        "name",
        "warn flag: matched by name"
    );
    assert_eq!(epg_match(&store, sid, "3").await, "none");
    let a = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
    let rows = downlink_core::epg::window(&store, &[a], ALL).await.unwrap();
    assert_eq!(
        rows.len(),
        1,
        "duplicate (channel,start) collapses; bad rows dropped"
    );
    assert_eq!(
        rows[0].title.as_deref(),
        Some("Entities & <Tags> \"Quotes\" \u{a9}")
    ); // first title wins
}

#[tokio::test]
async fn window_bounds_the_stored_programmes() {
    let (store, sid) = setup(&[("1", Some("tz.test"), None, "TZ")]).await;
    let noon = 1_767_268_800;
    let r = ingest(
        &store,
        sid,
        fx("timezones.xml"),
        Tz::UTC,
        TimeWindow {
            start_utc: noon,
            end_utc: noon + 3600,
        },
    )
    .await
    .unwrap();
    assert!(r.outside_window >= 3);
    assert!(r.programmes >= 2 && r.programmes < 7);
}

#[tokio::test]
async fn truncated_feed_keeps_what_it_has_and_never_deletes() {
    let (store, sid) = setup(&[("1", Some("t.test"), None, "T")]).await;
    let c = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
    sqlx::query("UPDATE channel SET epg_match = 'manual' WHERE id = ?")
        .bind(c)
        .execute(store.pool())
        .await
        .unwrap();
    // A future programme already in the cache survives a bad refresh.
    store
        .upsert_programmes(&[downlink_core::store::ProgrammeRow {
            channel_id: c,
            start_utc: 9_000_000_000,
            end_utc: 9_000_003_600,
            ext_id: None,
            title: Some("Future".into()),
            subtitle: None,
            description: None,
            season: None,
            episode: None,
            flags: 0,
            icon: None,
        }])
        .await
        .unwrap();
    let r = ingest(&store, sid, fx("truncated.xml"), Tz::UTC, ALL)
        .await
        .unwrap();
    assert!(r.truncated);
    assert_eq!(r.programmes, 1);
    let all = downlink_core::epg::window(&store, &[c], ALL).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(epg_match(&store, sid, "1").await, "manual");
}

#[tokio::test]
async fn manual_remap_wins_and_is_reported() {
    let (store, sid) = setup(&[("1", Some("wrong.id"), None, "Mystery")]).await;
    let ch = parse_channels(&fx("edge_cases.xml")).unwrap();
    let (_, s) = apply_matches(&store, sid, &ch).await.unwrap();
    assert_eq!(s.unmatched, ["Mystery"]);
    store.set_epg_remap(sid, "1", "B.TEST").await.unwrap(); // case-insensitive
    let (map, s) = apply_matches(&store, sid, &ch).await.unwrap();
    assert_eq!((s.manual, s.unmatched.len()), (1, 0));
    assert!(map.contains_key("b.test"));
    assert_eq!(epg_match(&store, sid, "1").await, "manual");
    store.clear_epg_remap(sid, "1").await.unwrap();
    assert_eq!(apply_matches(&store, sid, &ch).await.unwrap().1.manual, 0);
}

#[tokio::test]
async fn offset_less_stamps_use_the_source_timezone() {
    let (store, sid) = setup(&[("1", Some("tz.test"), None, "TZ")]).await;
    let berlin: Tz = "Europe/Berlin".parse().unwrap();
    let r = ingest(&store, sid, fx("timezones.xml"), berlin, ALL)
        .await
        .unwrap();
    assert_eq!(r.assumed_tz, 1);
    let c = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
    let rows = downlink_core::epg::window(&store, &[c], ALL).await.unwrap();
    let t = rows
        .iter()
        .find(|p| p.title.as_deref() == Some("No Offset (assume UTC)"))
        .unwrap();
    assert_eq!(
        t.start_utc,
        1_767_268_800 + 3 * 3600 - 3600,
        "15:00 Berlin = 14:00Z"
    );
}

#[tokio::test]
async fn gzip_feed_ingests() {
    let (store, sid) = setup(&[]).await;
    let r = ingest(&store, sid, fx("basic.xml.gz"), Tz::UTC, ALL)
        .await
        .unwrap();
    assert!(r.parsed >= 1, "{r:?}");
}

fn write_feed(
    dir: &std::path::Path,
    name: &str,
    progs: &[(&str, &str, &str)],
    close: bool,
) -> PathBuf {
    let mut s = String::from(
        "<?xml version=\"1.0\"?><tv><channel id=\"a.test\"><display-name>A</display-name></channel>",
    );
    for (start, stop, title) in progs {
        s.push_str(&format!(
            "<programme start=\"{start} +0000\" stop=\"{stop} +0000\" channel=\"a.test\"><title>{title}</title></programme>"
        ));
    }
    if close {
        s.push_str("</tv>");
    } else {
        s.push_str("<programme start=\"20300101130000 +0000\" stop=\"2030010114");
    }
    let p = dir.join(name);
    std::fs::write(&p, s).unwrap();
    p
}

async fn titles(store: &Store, ch: i64) -> Vec<String> {
    downlink_core::epg::window(store, &[ch], ALL)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|p| p.title)
        .collect()
}

#[tokio::test]
async fn refresh_replaces_programmes_inside_the_fed_window() {
    let (store, sid) = setup(&[("1", Some("a.test"), None, "A")]).await;
    let c = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
    let dir = tempfile::tempdir().unwrap();
    let day = "20300101";
    let first = write_feed(
        dir.path(),
        "1.xml",
        &[
            (&format!("{day}080000"), &format!("{day}090000"), "Early"),
            (&format!("{day}100000"), &format!("{day}110000"), "One"),
            (&format!("{day}110000"), &format!("{day}120000"), "Two"),
            (&format!("{day}120000"), &format!("{day}130000"), "Three"),
            (&format!("{day}140000"), &format!("{day}150000"), "Late"),
        ],
        true,
    );
    ingest(&store, sid, first, Tz::UTC, ALL).await.unwrap();
    assert_eq!(titles(&store, c).await.len(), 5);
    // The refreshed feed covers 10:00-12:30 for the channel: "Two" moved and "Three" is gone.
    let second = write_feed(
        dir.path(),
        "2.xml",
        &[
            (&format!("{day}100000"), &format!("{day}110000"), "One"),
            (
                &format!("{day}113000"),
                &format!("{day}123000"),
                "Two moved",
            ),
        ],
        true,
    );
    ingest(&store, sid, second, Tz::UTC, ALL).await.unwrap();
    assert_eq!(
        titles(&store, c).await,
        ["Early", "One", "Two moved", "Late"],
        "inside the window replaced; before and after it untouched"
    );
}

#[tokio::test]
async fn truncated_refresh_never_replaces_anything() {
    let (store, sid) = setup(&[("1", Some("a.test"), None, "A")]).await;
    let c = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
    let dir = tempfile::tempdir().unwrap();
    let day = "20300101";
    let full = write_feed(
        dir.path(),
        "1.xml",
        &[
            (&format!("{day}100000"), &format!("{day}110000"), "One"),
            (&format!("{day}110000"), &format!("{day}120000"), "Two"),
            (&format!("{day}120000"), &format!("{day}130000"), "Three"),
        ],
        true,
    );
    ingest(&store, sid, full, Tz::UTC, ALL).await.unwrap();
    let cut = write_feed(
        dir.path(),
        "2.xml",
        &[(&format!("{day}100000"), &format!("{day}110000"), "One v2")],
        false,
    );
    let r = ingest(&store, sid, cut, Tz::UTC, ALL).await.unwrap();
    assert!(r.truncated);
    assert_eq!(titles(&store, c).await.len(), 3, "nothing deleted");
    // A feed that fails outright changes nothing either.
    let bad = dir.path().join("bad.xml");
    std::fs::write(&bad, "not xml at all").unwrap();
    let _ = ingest(&store, sid, bad, Tz::UTC, ALL).await;
    assert_eq!(titles(&store, c).await.len(), 3);
}

#[tokio::test]
async fn channel_shift_moves_guide_times() {
    let (store, sid) = setup(&[]).await;
    let g = store.sync_groups(sid, &[]).await.unwrap();
    let mk = |ext: &str, shift: i64| ChannelUpsert {
        ext_id: ext.into(),
        tvg_id: Some("a.test".into()),
        name: ext.into(),
        epg_shift_secs: shift,
        ..Default::default()
    };
    store
        .sync_channels(
            sid,
            &[mk("plain", 0), mk("late", 5400), mk("early", -1800)],
            &g,
        )
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let f = write_feed(
        dir.path(),
        "1.xml",
        &[("20300101100000", "20300101110000", "Show")],
        true,
    );
    ingest(&store, sid, f, Tz::UTC, ALL).await.unwrap();
    let base = 1_893_492_000; // 2030-01-01 10:00:00Z
    for (ext, shift) in [("plain", 0), ("late", 5400), ("early", -1800)] {
        let id = store.channel_by_ext(sid, ext).await.unwrap().unwrap().id;
        let rows = downlink_core::epg::window(&store, &[id], ALL)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "{ext}");
        assert_eq!(
            (rows[0].start_utc, rows[0].end_utc),
            (base + shift, base + 3600 + shift),
            "{ext}"
        );
    }
}
