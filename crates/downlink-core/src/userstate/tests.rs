use std::sync::Mutex;

use super::*;
use crate::store::{ChannelUpsert, NewSource};

#[derive(Default)]
struct Mem {
    v: Mutex<Option<Value>>,
    puts: Mutex<usize>,
}

#[async_trait]
impl AppDataRemote for Mem {
    async fn get(&self, key: &str) -> Result<Option<Value>> {
        assert_eq!(key, APP_DATA_KEY);
        Ok(self.v.lock().unwrap().clone())
    }
    async fn put(&self, key: &str, value: &Value) -> Result<()> {
        assert_eq!(key, APP_DATA_KEY);
        *self.v.lock().unwrap() = Some(value.clone());
        *self.puts.lock().unwrap() += 1;
        Ok(())
    }
}

const SRC_UUID: &str = "11111111-1111-4111-8111-111111111111";

/// A "machine": its own DB, the same server (same source uuid) and channel ext ids, but
/// deliberately different row ids (it inserts a decoy first on machine B).
async fn machine(decoys: usize) -> (Store, i64) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "S".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    sqlx::query("UPDATE source SET uuid = ? WHERE id = ?")
        .bind(SRC_UUID)
        .bind(src.id)
        .execute(store.pool())
        .await
        .unwrap();
    let g = store
        .sync_groups(src.id, &[("1".into(), "G".into())])
        .await
        .unwrap();
    let mut chans: Vec<ChannelUpsert> = (0..decoys)
        .map(|i| ChannelUpsert {
            ext_id: format!("decoy{i}"),
            name: format!("Decoy {i}"),
            ..Default::default()
        })
        .collect();
    chans.extend((1..=4).map(|i| ChannelUpsert {
        ext_id: i.to_string(),
        name: format!("Channel {i}"),
        group_ext_id: Some("1".into()),
        ..Default::default()
    }));
    store.sync_channels(src.id, &chans, &g).await.unwrap();
    (store, src.id)
}

async fn cid(s: &Store, sid: i64, ext: &str) -> i64 {
    s.channel_by_ext(sid, ext).await.unwrap().unwrap().id
}

async fn favs(s: &Store, sid: i64) -> Vec<String> {
    let mut out = vec![];
    for id in s.favourite_channel_ids(Some(sid)).await.unwrap() {
        out.push(s.channel(id).await.unwrap().unwrap().ext_id);
    }
    out
}

#[tokio::test]
async fn local_favourites_recents_resume_and_continue_watching() {
    let (s, sid) = machine(0).await;
    let (c1, c2) = (cid(&s, sid, "1").await, cid(&s, sid, "2").await);
    s.set_favourite(c1, true, 10).await.unwrap();
    s.set_favourite(c2, true, 11).await.unwrap();
    assert_eq!(favs(&s, sid).await, ["1", "2"]);
    s.set_favourite(c1, false, 12).await.unwrap();
    assert_eq!(favs(&s, sid).await, ["2"]);
    s.set_favourite(c1, true, 13).await.unwrap(); // re-adding goes to the end
    assert_eq!(favs(&s, sid).await, ["2", "1"]);

    s.touch_recent(c1, 100).await.unwrap();
    s.touch_recent(c2, 200).await.unwrap();
    s.touch_recent(c1, 300).await.unwrap();
    let r = s.recents(10).await.unwrap();
    assert_eq!((r[0].channel_id, r[1].channel_id), (c1, c2));

    s.set_resume(sid, "catchup", "1:1000", 60_000, Some(3_600_000), 50)
        .await
        .unwrap();
    s.set_resume(sid, "movie", "m1", 3_500_000, Some(3_600_000), 60)
        .await
        .unwrap(); // finished
    s.set_resume(sid, "movie", "m2", 5_000, Some(3_600_000), 70)
        .await
        .unwrap(); // too short
    s.set_resume(sid, "episode", "e1", 120_000, None, 80)
        .await
        .unwrap(); // unknown duration
    let cw = s.continue_watching(10).await.unwrap();
    assert_eq!(
        cw.iter().map(|r| r.item_key.as_str()).collect::<Vec<_>>(),
        ["e1", "1:1000"]
    );
    // older write never overwrites a newer one
    s.set_resume(sid, "episode", "e1", 1, None, 10)
        .await
        .unwrap();
    assert_eq!(
        s.resume(sid, "episode", "e1")
            .await
            .unwrap()
            .unwrap()
            .position_ms,
        120_000
    );
}

#[tokio::test]
async fn two_machine_merge_scenario() {
    let remote = Mem::default();
    let (a, sa) = machine(0).await;
    let (b, sb) = machine(7).await; // different row ids than A
    assert_ne!(cid(&a, sa, "1").await, cid(&b, sb, "1").await);

    // t=100: A favourites ch1 then ch2, watches ch1, saves a catch-up position, and syncs.
    a.set_favourite(cid(&a, sa, "1").await, true, 100)
        .await
        .unwrap();
    a.set_favourite(cid(&a, sa, "2").await, true, 101)
        .await
        .unwrap();
    a.touch_recent(cid(&a, sa, "1").await, 102).await.unwrap();
    a.set_resume(sa, "catchup", "1:5000", 30_000, Some(3_600_000), 103)
        .await
        .unwrap();
    let r = sync_state(&a, &remote, 200).await.unwrap();
    assert!(r.pushed);

    // B pulls everything onto its own row ids.
    let r = sync_state(&b, &remote, 201).await.unwrap();
    assert_eq!(
        (r.applied.favourites, r.applied.recents, r.applied.resume),
        (2, 1, 1)
    );
    assert_eq!(favs(&b, sb).await, ["1", "2"]);
    assert_eq!(
        b.resume(sb, "catchup", "1:5000")
            .await
            .unwrap()
            .unwrap()
            .position_ms,
        30_000
    );

    // t=300 on B: unfavourite ch1, favourite ch3, advance the catch-up position. t=250 on A:
    // an older, conflicting resume write and a new favourite ch4 (A is offline until later).
    b.set_favourite(cid(&b, sb, "1").await, false, 300)
        .await
        .unwrap();
    b.set_favourite(cid(&b, sb, "3").await, true, 301)
        .await
        .unwrap();
    b.set_resume(sb, "catchup", "1:5000", 900_000, Some(3_600_000), 302)
        .await
        .unwrap();
    a.set_resume(sa, "catchup", "1:5000", 45_000, Some(3_600_000), 250)
        .await
        .unwrap();
    a.set_favourite(cid(&a, sa, "4").await, true, 260)
        .await
        .unwrap();

    sync_state(&b, &remote, 400).await.unwrap();
    sync_state(&a, &remote, 401).await.unwrap(); // A catches up; its older resume loses
    sync_state(&b, &remote, 402).await.unwrap(); // and B receives A's ch4

    assert_eq!(favs(&a, sa).await, favs(&b, sb).await, "machines converge");
    let mut f = favs(&a, sa).await;
    f.sort();
    assert_eq!(
        f,
        ["2", "3", "4"],
        "ch1 stayed deleted (tombstone beat the older add)"
    );
    for (s, id) in [(&a, sa), (&b, sb)] {
        assert_eq!(
            s.resume(id, "catchup", "1:5000")
                .await
                .unwrap()
                .unwrap()
                .position_ms,
            900_000
        );
    }
    // Idempotent: a further sync changes nothing and pushes nothing.
    let puts = *remote.puts.lock().unwrap();
    let r = sync_state(&a, &remote, 403).await.unwrap();
    assert!(!r.pushed);
    assert_eq!(*remote.puts.lock().unwrap(), puts);
}

fn fav(c: &str, u: i64, d: bool) -> FavItem {
    FavItem {
        s: "s".into(),
        c: c.into(),
        p: 0,
        u,
        d,
    }
}

#[test]
fn merge_is_symmetric_lww_with_tombstone_expiry() {
    let a = StateDoc {
        favourites: vec![fav("1", 10, false), fav("2", 5, true), fav("old", 1, true)],
        ..Default::default()
    };
    let b = StateDoc {
        favourites: vec![fav("1", 20, true), fav("2", 6, false)],
        ..Default::default()
    };
    let now = 10 + TOMBSTONE_SECS; // tombstone "old" (u=1) is > 30 days old
    let m1 = merge(&a, &b, now);
    let m2 = merge(&b, &a, now);
    assert_eq!(m1, m2);
    let get = |c: &str| m1.favourites.iter().find(|f| f.c == c).cloned();
    assert!(get("1").unwrap().d);
    assert!(!get("2").unwrap().d);
    assert!(get("old").is_none(), "expired tombstone dropped");
    assert_eq!(merge(&m1, &m1, now), m1, "idempotent");
}

#[test]
fn merge_caps_and_prefs() {
    let resume: Vec<ResumeItem> = (0..600)
        .map(|i| ResumeItem {
            s: "s".into(),
            k: "movie".into(),
            i: i.to_string(),
            ms: 1,
            dur: None,
            u: i,
        })
        .collect();
    let recents: Vec<RecentItem> = (0..300)
        .map(|i| RecentItem {
            s: "s".into(),
            c: i.to_string(),
            u: i,
        })
        .collect();
    let a = StateDoc {
        resume,
        recents,
        ..Default::default()
    };
    let m = merge(&a, &StateDoc::default(), 10_000);
    assert_eq!((m.resume.len(), m.recents.len()), (MAX_RESUME, MAX_RECENTS));
    assert_eq!(
        m.resume.last().unwrap().i,
        "100",
        "oldest resume entries dropped first"
    );
    assert_eq!(m.recents[0].c, "299");

    let mut pa = Map::new();
    pa.insert("density".into(), "compact".into());
    pa.insert("u".into(), 5.into());
    let mut pb = Map::new();
    pb.insert("density".into(), "tv".into());
    pb.insert("u".into(), 9.into());
    let m = merge(
        &StateDoc {
            prefs: pa.clone(),
            ..Default::default()
        },
        &StateDoc {
            prefs: pb,
            ..Default::default()
        },
        0,
    );
    assert_eq!(m.prefs["density"], "tv");
}

#[test]
fn document_is_trimmed_to_the_server_limit() {
    let resume: Vec<ResumeItem> = (0..500)
        .map(|i| ResumeItem {
            s: "s".repeat(36),
            k: "episode".into(),
            i: format!("{}{i}", "x".repeat(300)),
            ms: 1,
            dur: Some(2),
            u: i,
        })
        .collect();
    let mut d = StateDoc {
        resume,
        favourites: vec![fav("keep", 1, false)],
        ..Default::default()
    };
    assert!(serde_json::to_vec(&d).unwrap().len() > MAX_BYTES);
    fit_to_limit(&mut d).unwrap();
    assert!(serde_json::to_vec(&d).unwrap().len() <= MAX_BYTES);
    assert_eq!(d.favourites.len(), 1, "favourites are kept");
    assert!(d.resume.first().unwrap().u > d.resume.last().unwrap().u || d.resume.len() == 1);
}

#[tokio::test]
async fn unknown_remote_items_survive_a_round_trip() {
    let remote = Mem::default();
    let other = StateDoc {
        favourites: vec![FavItem {
            s: "other-server".into(),
            c: "9".into(),
            p: 0,
            u: 50,
            d: false,
        }],
        ..Default::default()
    };
    *remote.v.lock().unwrap() = Some(serde_json::to_value(&other).unwrap());
    let (a, sa) = machine(0).await;
    a.set_favourite(cid(&a, sa, "1").await, true, 100)
        .await
        .unwrap();
    sync_state(&a, &remote, 200).await.unwrap();
    let pushed: StateDoc =
        serde_json::from_value(remote.v.lock().unwrap().clone().unwrap()).unwrap();
    assert!(
        pushed.favourites.iter().any(|f| f.s == "other-server"),
        "another machine's entries are preserved"
    );
    assert_eq!(pushed.favourites.len(), 2);
}

#[tokio::test]
async fn garbage_remote_document_is_replaced() {
    let remote = Mem::default();
    *remote.v.lock().unwrap() = Some(serde_json::json!("not a document"));
    let (a, sa) = machine(0).await;
    a.set_favourite(cid(&a, sa, "1").await, true, 100)
        .await
        .unwrap();
    assert!(sync_state(&a, &remote, 200).await.unwrap().pushed);
}

#[test]
fn app_data_key_obeys_dispatcharr_length_rule() {
    assert!((8..=64).contains(&APP_DATA_KEY.len()));
}
