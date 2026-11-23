#![allow(clippy::unwrap_used, clippy::expect_used)]
//! PLAN §7.1 / M2: channel search < 30 ms on a 150k-channel fixture.
//! Slow to build, so ignored by default:
//! `cargo test -p downlink-core --release --test search_bench -- --ignored --nocapture`

use std::time::Instant;

use downlink_core::catalog::{self, ChannelFilter};
use downlink_core::store::{ChannelUpsert, NewSource, ProgrammeRow, Store};

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
            // Absolute public logo, as M3U/Xtream sources carry: exercises the image path.
            logo_key: Some(format!("https://img.example.test/{i}.png")),
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

// ---- unified search: the programme leg on a 20k-channel / 672k-programme guide ----------------

const TITLE_A: [&str; 40] = [
    "Morning", "Evening", "Late", "World", "Local", "Daily", "Weekend", "Sunday", "Prime",
    "Classic", "Great", "Wild", "Secret", "Hidden", "Modern", "Ancient", "Golden", "Final",
    "First", "Last", "Big", "Small", "Live", "Special", "Grand", "Royal", "Open", "Night", "Day",
    "City", "Country", "Ocean", "Mountain", "Space", "Home", "Garden", "Kitchen", "Street", "Road",
    "River",
];
const TITLE_B: [&str; 40] = [
    "News",
    "Report",
    "Show",
    "Hour",
    "Story",
    "Files",
    "Journey",
    "Match",
    "League",
    "Cup",
    "Cinema",
    "Theatre",
    "Tonight",
    "Weekly",
    "Review",
    "Update",
    "Live",
    "Express",
    "Digest",
    "Talk",
    "Quiz",
    "Challenge",
    "Kitchen",
    "Planet",
    "Detectives",
    "Mysteries",
    "Tales",
    "Chronicles",
    "Collection",
    "Hits",
    "Concert",
    "Session",
    "Debate",
    "Tour",
    "Diaries",
    "Rescue",
    "Makeover",
    "Auction",
    "Academy",
    "Games",
];

async fn build_guide(channels: usize, per_channel: usize, now: i64) -> (Store, i64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("guide.sqlite")).await.unwrap();
    std::mem::forget(dir);
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
    let g = store.sync_groups(src.id, &[]).await.unwrap();
    let chans: Vec<ChannelUpsert> = (0..channels)
        .map(|i| ChannelUpsert {
            ext_id: i.to_string(),
            number: Some(i as f64),
            name: format!("Channel {i}"),
            tvg_id: Some(format!("c{i}.test")),
            ..Default::default()
        })
        .collect();
    store.sync_channels(src.id, &chans, &g).await.unwrap();
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM channel WHERE source_id = ? ORDER BY id")
            .bind(src.id)
            .fetch_all(store.pool())
            .await
            .unwrap();
    // 10 days back to 7 days ahead, evenly spread per channel.
    let span = 17 * 86_400;
    let step = span / per_channel as i64;
    let mut batch = Vec::with_capacity(5_000);
    for (ci, id) in ids.iter().enumerate() {
        for k in 0..per_channel {
            let start = now - 10 * 86_400 + k as i64 * step + (ci as i64 % 7) * 60;
            let n = ci * 31 + k * 7;
            batch.push(ProgrammeRow {
                channel_id: *id,
                start_utc: start,
                end_utc: start + step,
                ext_id: None,
                title: Some(format!(
                    "{} {} {}",
                    TITLE_A[n % 40],
                    TITLE_B[(n / 40) % 40],
                    n % 97
                )),
                subtitle: None,
                description: Some(format!(
                    "{} {} special about {} and {}",
                    TITLE_B[n % 40],
                    TITLE_A[(n / 3) % 40],
                    TITLE_A[(n / 7) % 40],
                    TITLE_B[(n / 11) % 40]
                )),
                season: None,
                episode: None,
                flags: 0,
                icon: None,
            });
            if batch.len() == 5_000 {
                store.upsert_programmes(&batch).await.unwrap();
                batch.clear();
            }
        }
    }
    store.upsert_programmes(&batch).await.unwrap();
    (store, src.id)
}

/// Programme search p95 < 30 ms on 672k programmes (PLAN §8.8, docs/perf.md).
/// `cargo test -p downlink-core --release --test search_bench -- --ignored --nocapture programme`
#[tokio::test]
#[ignore = "builds a 672k-programme database; run with --release --ignored"]
async fn programme_search_p95_under_30ms_on_672k_programmes() {
    let now = downlink_core::sync::now_utc();
    let (store, sid) = build_guide(20_000, 34, now).await;
    let f = ChannelFilter {
        source_id: Some(sid),
        ..Default::default()
    };
    let budget_ms: f64 = std::env::var("SEARCH_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    // What a user types, keystroke by keystroke, plus whole words.
    let mut queries: Vec<String> = Vec::new();
    for w in [
        "news",
        "morning",
        "weekend cinema",
        "kitchen",
        "tonight",
        "grand",
        "quiz",
        "xqzv",
    ] {
        for n in 1..=w.len() {
            queries.push(w[..n].to_owned());
        }
    }
    // warm-up
    for q in queries.iter().take(8) {
        let _ = catalog::search_programmes(&store, &f, q, now, 20)
            .await
            .unwrap();
    }
    let mut times = Vec::new();
    for q in &queries {
        let t = Instant::now();
        let r = catalog::search_programmes(&store, &f, q, now, 20)
            .await
            .unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        times.push(ms);
        assert!(r.len() <= 20);
        assert!(
            r.iter().all(|p| p.end_utc >= now - 7 * 86_400),
            "{q:?}: results stay inside the archive window"
        );
    }
    times.sort_by(f64::total_cmp);
    let p = |q: f64| times[((times.len() as f64 * q) as usize).min(times.len() - 1)];
    println!(
        "programme search over {} queries: p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        times.len(),
        p(0.5),
        p(0.95),
        times.last().unwrap()
    );
    assert!(
        p(0.95) < budget_ms,
        "p95 {:.1} ms exceeds {budget_ms} ms",
        p(0.95)
    );
}

/// Service-level search (what the app calls): channel leg + programme leg + channel DTOs with
/// logos, favourites and scheduled flags, over the same 672k-programme fixture. Mirrors the app
/// path, so it includes the work the core bench leaves out. Debug builds are slower: compare
/// before/after in the same profile, or set SEARCH_BUDGET_MS.
/// `cargo test -p downlink-core --test search_bench -- --ignored --nocapture service_search`
#[tokio::test]
#[ignore = "builds a 672k-programme database; run with --ignored"]
async fn service_search_p95_under_30ms_on_672k_programmes() {
    use downlink_core::auth::MemorySecretStore;
    use downlink_core::service::Service;
    use std::sync::Arc;
    let now = downlink_core::sync::now_utc();
    let (store, _sid) = build_guide(20_000, 34, now).await;
    let img = tempfile::tempdir().unwrap();
    let svc = Service::with_store(
        store,
        Arc::new(MemorySecretStore::new()),
        img.path().to_path_buf(),
        "0.0.0-bench".into(),
    );
    let budget_ms: f64 = std::env::var("SEARCH_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let mut queries: Vec<String> = Vec::new();
    for w in [
        "news",
        "morning",
        "weekend cinema",
        "kitchen",
        "tonight",
        "grand",
        "quiz",
        "xqzv",
        "channel 1",
    ] {
        for n in 1..=w.len() {
            queries.push(w[..n].to_owned());
        }
    }
    for q in queries.iter().take(8) {
        svc.search(q, 20).await.unwrap();
    }
    let mut times = Vec::new();
    for q in &queries {
        let t = Instant::now();
        let r = svc.search(q, 20).await.unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(r.programmes.len() <= 20 && r.channels.len() <= 20);
    }
    times.sort_by(f64::total_cmp);
    let p = |q: f64| times[((times.len() as f64 * q) as usize).min(times.len() - 1)];
    println!(
        "service search over {} queries: p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        times.len(),
        p(0.5),
        p(0.95),
        times.last().unwrap()
    );
    assert!(
        p(0.95) < budget_ms,
        "p95 {:.1} ms exceeds {budget_ms} ms",
        p(0.95)
    );
}
