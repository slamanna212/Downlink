#![allow(clippy::unwrap_used, clippy::expect_used)]
//! PLAN §7.1 / M2: channel search < 30 ms on a 150k-channel fixture.
//! Slow to build, so ignored by default:
//! `cargo test -p downlink-core --release --test search_bench -- --ignored --nocapture`

use std::time::Instant;

use downlink_core::catalog::{self, ChannelFilter};
use downlink_core::store::{ChannelUpsert, NewSource, Store};

const N: usize = 150_000;
const WORDS: [&str; 16] = [
    "news", "sport", "movies", "kids", "music", "docs", "world", "local", "action", "comedy",
    "drama", "science", "nature", "history", "premier", "classic",
];

async fn build() -> (Store, i64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("bench.sqlite")).await.unwrap();
    std::mem::forget(dir); // keep the file for the process lifetime
    let src = store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "bench".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let groups: Vec<(String, String)> = (0..50)
        .map(|i| (i.to_string(), format!("Group {i}")))
        .collect();
    let g = store.sync_groups(src.id, &groups).await.unwrap();
    let chans: Vec<ChannelUpsert> = (0..N)
        .map(|i| ChannelUpsert {
            ext_id: i.to_string(),
            number: Some(i as f64 / 4.0),
            name: format!("{} {} {}", WORDS[i % 16], WORDS[(i / 16) % 16], i),
            group_ext_id: Some((i % 50).to_string()),
            tvg_id: Some(format!("c{i}.test")),
            ..Default::default()
        })
        .collect();
    store.sync_channels(src.id, &chans, &g).await.unwrap();
    (store, src.id)
}

#[tokio::test]
#[ignore = "builds a 150k-channel database; run with --release --ignored"]
async fn search_under_30ms_on_150k_channels() {
    let (store, sid) = build().await;
    let budget_ms: u128 = std::env::var("SEARCH_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    for q in [
        "news",
        "ne",
        "sport mov",
        "12345",
        "premier classic 9",
        "zzz",
    ] {
        let f = ChannelFilter {
            source_id: Some(sid),
            search: Some(q.into()),
            ..Default::default()
        };
        // warm
        catalog::list_channels(&store, &f, 50, 0).await.unwrap();
        let t = Instant::now();
        let r = catalog::list_channels(&store, &f, 50, 0).await.unwrap();
        let ms = t.elapsed().as_millis();
        println!("search {q:?}: {} rows in {ms} ms", r.len());
        assert!(
            ms < budget_ms,
            "search {q:?} took {ms} ms (budget {budget_ms})"
        );
    }
}
