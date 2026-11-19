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
