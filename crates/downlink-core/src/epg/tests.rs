use super::*;
use crate::store::{ChannelUpsert, NewSource};

async fn seeded() -> (Store, i64, i64) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "T".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let g = store
        .sync_groups(src.id, &[("1".into(), "G".into())])
        .await
        .unwrap();
    store
        .sync_channels(
            src.id,
            &[ChannelUpsert {
                ext_id: "1".into(),
                name: "C".into(),
                tvg_id: Some("c.test".into()),
                group_ext_id: Some("1".into()),
                ..Default::default()
            }],
            &g,
        )
        .await
        .unwrap();
    let id = store.channel_by_ext(src.id, "1").await.unwrap().unwrap().id;
    (store, src.id, id)
}

fn p(cid: i64, s: i64, e: i64, t: &str) -> ProgrammeRow {
    ProgrammeRow {
        channel_id: cid,
        start_utc: s,
        end_utc: e,
        ext_id: None,
        title: Some(t.into()),
        subtitle: None,
        description: None,
        season: None,
        episode: None,
        flags: 0,
    }
}

#[tokio::test]
async fn now_next_boundaries() {
    let (store, _, c) = seeded().await;
    store
        .upsert_programmes(&[p(c, 0, 100, "A"), p(c, 100, 200, "B"), p(c, 250, 300, "C")])
        .await
        .unwrap();
    let nn = now_next(&store, c, 50).await.unwrap();
    assert_eq!(
        (
            nn.now.unwrap().title.as_deref(),
            nn.next.unwrap().title.as_deref()
        ),
        (Some("A"), Some("B"))
    );
    // end is exclusive, start inclusive
    let nn = now_next(&store, c, 100).await.unwrap();
    assert_eq!(nn.now.unwrap().title.as_deref(), Some("B"));
    // gap: nothing now, next is C
    let nn = now_next(&store, c, 220).await.unwrap();
    assert!(nn.now.is_none());
    assert_eq!(nn.next.unwrap().title.as_deref(), Some("C"));
    // after everything
    let nn = now_next(&store, c, 400).await.unwrap();
    assert!(nn.now.is_none() && nn.next.is_none());
}

#[tokio::test]
async fn batched_now_next_skips_overlapping_next_and_keeps_empty_channels() {
    let (store, _, c) = seeded().await;
    store
        .upsert_programmes(&[
            p(c, 0, 100, "current"),
            p(c, 80, 110, "overlap"),
            p(c, 120, 140, "next"),
        ])
        .await
        .unwrap();

    let pairs = now_next_many(&store, &[c, c + 1, c], 70).await.unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(
        pairs[&c].now.as_ref().unwrap().title.as_deref(),
        Some("current")
    );
    assert_eq!(
        pairs[&c].next.as_ref().unwrap().title.as_deref(),
        Some("next")
    );
    assert_eq!(pairs[&(c + 1)], NowNext::default());
}

#[tokio::test]
async fn replacing_a_window_keeps_retained_rowids_and_removes_missing_programmes() {
    let (store, src, c) = seeded().await;
    let keep = p(c, 0, 100, "keep");
    store
        .upsert_programmes(&[
            keep.clone(),
            p(c, 100, 200, "obsolete"),
            p(c, 300, 400, "outside"),
        ])
        .await
        .unwrap();
    let rowid: i64 = sqlx::query_scalar("SELECT rowid FROM programme WHERE start_utc = 0")
        .fetch_one(store.pool())
        .await
        .unwrap();
    store
        .replace_programme_window(
            src,
            TimeWindow {
                start_utc: 0,
                end_utc: 250,
            },
            &[keep, p(c, 200, 250, "new")],
        )
        .await
        .unwrap();
    let slots: Vec<(i64, i64)> =
        sqlx::query_as("SELECT rowid, start_utc FROM programme ORDER BY start_utc")
            .fetch_all(store.pool())
            .await
            .unwrap();
    assert_eq!(slots[0], (rowid, 0));
    assert_eq!(
        slots.iter().map(|(_, start)| *start).collect::<Vec<_>>(),
        [0, 200, 300]
    );
}

#[tokio::test]
async fn window_overlap_and_order() {
    let (store, _, c) = seeded().await;
    store
        .upsert_programmes(&[p(c, 200, 300, "B"), p(c, 0, 100, "A"), p(c, 100, 200, "M")])
        .await
        .unwrap();
    let r = window(
        &store,
        &[c],
        TimeWindow {
            start_utc: 50,
            end_utc: 200,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        r.iter()
            .map(|x| x.title.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["A", "M"]
    );
}

#[tokio::test]
async fn window_for_many_channels_is_one_query_in_the_callers_channel_order() {
    let (store, src, c1) = seeded().await;
    let g = store.sync_groups(src, &[]).await.unwrap();
    store
        .sync_channels(
            src,
            &[
                ChannelUpsert {
                    ext_id: "1".into(),
                    name: "One".into(),
                    ..Default::default()
                },
                ChannelUpsert {
                    ext_id: "2".into(),
                    name: "Two".into(),
                    ..Default::default()
                },
            ],
            &g,
        )
        .await
        .unwrap();
    let c2: i64 = sqlx::query_scalar("SELECT id FROM channel WHERE source_id = ? AND ext_id = '2'")
        .bind(src)
        .fetch_one(store.pool())
        .await
        .unwrap();
    store
        .upsert_programmes(&[
            p(c1, 100, 200, "one-b"),
            p(c1, 0, 100, "one-a"),
            p(c2, 0, 100, "two-a"),
        ])
        .await
        .unwrap();
    let w = TimeWindow {
        start_utc: 0,
        end_utc: 500,
    };
    let titles = |rows: Vec<ProgrammeRow>| {
        rows.into_iter()
            .map(|r| r.title.unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles(window(&store, &[c2, c1], w).await.unwrap()),
        ["two-a", "one-a", "one-b"]
    );
    assert_eq!(
        titles(window(&store, &[c1, c2], w).await.unwrap()),
        ["one-a", "one-b", "two-a"]
    );
    assert!(window(&store, &[], w).await.unwrap().is_empty());
}

#[test]
fn windows_and_retention() {
    let w = visible_window(10_000);
    assert_eq!((w.start_utc, w.end_utc), (10_000 - 7200, 10_000 + 43_200));
    assert_eq!(retention_cutoff(1_000_000, 3), 1_000_000 - 3 * 86_400);
    assert_eq!(
        retention_cutoff(1_000_000, 30),
        1_000_000 - 7 * 86_400,
        "capped at 7 days"
    );
}

#[tokio::test]
async fn prune_uses_max_catchup_days() {
    let (store, sid, c) = seeded().await;
    sqlx::query("UPDATE channel SET catchup_days = 1")
        .execute(store.pool())
        .await
        .unwrap();
    let now = 10 * 86_400;
    store
        .upsert_programmes(&[
            p(c, now - 3 * 86_400, now - 3 * 86_400 + 60, "old"),
            p(c, now - 3600, now, "recent"),
        ])
        .await
        .unwrap();
    assert_eq!(prune(&store, now).await.unwrap(), 1);
    let _ = sid;
}
