use std::collections::HashMap;

use super::*;

fn ch(ext: &str, name: &str, number: f64) -> ChannelUpsert {
    ChannelUpsert {
        ext_id: ext.into(),
        uuid: Some(format!("uuid-{ext}")),
        number: Some(number),
        name: name.into(),
        group_ext_id: Some("g1".into()),
        tvg_id: Some(format!("tvg.{ext}")),
        streams: vec!["30".into(), "10".into(), "20".into()],
        ..Default::default()
    }
}

async fn seeded() -> (Store, SourceRow, HashMap<String, i64>) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "Test".into(),
            base_url: Some("https://dispatcharr.test".into()),
            secret_ref: Some("downlink/source/x/api_key".into()),
            username: None,
            tls_insecure: false,
            created_at: 1_700_000_000,
        })
        .await
        .unwrap();
    let groups = store
        .sync_groups(src.id, &[("g1".into(), "News".into())])
        .await
        .unwrap();
    (store, src, groups)
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sqlite");
    let s = Store::open(&path).await.unwrap();
    s.close().await;
    let s = Store::open(&path).await.unwrap(); // re-open re-runs migrate
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type IN ('table')")
            .fetch_all(s.pool())
            .await
            .unwrap();
    for t in [
        "source",
        "channel",
        "channel_group",
        "channel_stream",
        "programme",
        "open_session",
        "channel_fts",
        "sync_state",
        "favourite",
        "image_cache",
    ] {
        assert!(tables.iter().any(|x| x == t), "missing table {t}");
    }
}

#[tokio::test]
async fn newer_database_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sqlite");
    let s = Store::open(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (9999, 'future', 1, x'00', 0)",
    )
    .execute(s.pool())
    .await
    .unwrap();
    s.close().await;
    let err = Store::open(&path).await.unwrap_err();
    assert!(err.blocking, "{err}");
    assert_eq!(err.code, crate::ErrorCode::Local);
    assert_eq!(
        err.message,
        "This library was created by a newer version of Downlink."
    );
    // Nothing was backed up or migrated on the way.
    assert!(!dir.path().join("d.sqlite.bak1").exists());
}

#[tokio::test]
async fn archiving_a_newer_database_lets_a_fresh_one_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("downlink.sqlite");
    let s = Store::open(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (9999, 'future', 1, x'00', 0)",
    )
    .execute(s.pool())
    .await
    .unwrap();
    s.close().await;
    assert!(Store::open(&path).await.is_err());
    std::fs::write(dir.path().join("downlink.sqlite-wal"), b"w").unwrap();
    let moved = crate::service::Service::archive_newer_database(dir.path()).unwrap();
    let name = moved.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(
        name.starts_with("downlink.sqlite.newer-") && name.ends_with(".bak"),
        "{name}"
    );
    assert!(moved.exists());
    assert!(!path.exists());
    assert!(dir.path().read_dir().unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_str()
            .unwrap()
            .ends_with(".bak-wal")
    }));
    // A fresh library opens.
    Store::open(&path).await.unwrap().close().await;
}

#[tokio::test]
async fn channel_upsert_is_stable_and_ordered() {
    let (store, src, groups) = seeded().await;
    let c = vec![ch("1", "Channel One", 1.0), ch("2", "Channel Two", 2.5)];
    let n = store.sync_channels(src.id, &c, &groups).await.unwrap();
    assert_eq!((n.inserted, n.updated, n.removed), (2, 0, 0));
    let first = store.channel_by_ext(src.id, "1").await.unwrap().unwrap();
    // Re-sync unchanged: same ids, zero inserts.
    let n = store.sync_channels(src.id, &c, &groups).await.unwrap();
    assert_eq!((n.inserted, n.updated, n.removed), (0, 2, 0));
    let again = store.channel_by_ext(src.id, "1").await.unwrap().unwrap();
    assert_eq!(first.id, again.id);
    // Failover order is preserved, not sorted.
    assert_eq!(
        store.channel_stream_ids(first.id).await.unwrap(),
        ["30", "10", "20"]
    );
    // Removal.
    let n = store.sync_channels(src.id, &c[..1], &groups).await.unwrap();
    assert_eq!(n.removed, 1);
    assert!(store.channel_by_ext(src.id, "2").await.unwrap().is_none());
}

#[tokio::test]
async fn fts_triggers_follow_changes() {
    let (store, src, groups) = seeded().await;
    store
        .sync_channels(src.id, &[ch("1", "Channel One", 5.5)], &groups)
        .await
        .unwrap();
    let q = |m: &str| {
        let pool = store.pool().clone();
        let m = m.to_owned();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT rowid FROM channel_fts WHERE channel_fts MATCH ?")
                .bind(m)
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(q("chan*").await.len(), 1);
    assert_eq!(q("number_text:\"5.5\"").await.len(), 1);
    store
        .sync_channels(src.id, &[ch("1", "Renamed", 5.5)], &groups)
        .await
        .unwrap();
    assert!(q("chan*").await.is_empty());
    assert_eq!(q("rena*").await.len(), 1);
    store.sync_channels(src.id, &[], &groups).await.unwrap();
    assert!(q("rena*").await.is_empty());
}

#[tokio::test]
async fn programmes_upsert_window_prune() {
    let (store, src, groups) = seeded().await;
    store
        .sync_channels(src.id, &[ch("1", "Channel One", 1.0)], &groups)
        .await
        .unwrap();
    let cid = store.channel_by_ext(src.id, "1").await.unwrap().unwrap().id;
    let mk = |s: i64, e: i64, t: &str| ProgrammeRow {
        channel_id: cid,
        start_utc: s,
        end_utc: e,
        ext_id: Some(format!("p{s}")),
        title: Some(t.into()),
        subtitle: None,
        description: None,
        season: Some(1),
        episode: None,
        flags: 0,
        icon: None,
    };
    store
        .upsert_programmes(&[mk(100, 200, "A"), mk(200, 300, "B")])
        .await
        .unwrap();
    store
        .upsert_programmes(&[mk(100, 200, "A2")])
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM programme")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        store
            .delete_programmes_overlapping(src.id, 150, 250)
            .await
            .unwrap(),
        2
    );
    store.upsert_programmes(&[mk(100, 200, "A")]).await.unwrap();
    assert_eq!(store.prune_programmes(150).await.unwrap(), 0);
    assert_eq!(store.prune_programmes(250).await.unwrap(), 1);
}

#[tokio::test]
async fn open_sessions_roundtrip() {
    let (store, src, _) = seeded().await;
    store
        .add_open_session(src.id, "catchup", "abc")
        .await
        .unwrap();
    store
        .add_open_session(src.id, "catchup", "abc")
        .await
        .unwrap();
    store.add_open_session(src.id, "hls", "tok").await.unwrap();
    assert_eq!(store.list_open_sessions().await.unwrap().len(), 2);
    store
        .remove_open_session(src.id, "catchup", "abc")
        .await
        .unwrap();
    assert_eq!(store.list_open_sessions().await.unwrap().len(), 1);
}

#[tokio::test]
async fn sync_state_and_source_update() {
    let (store, src, _) = seeded().await;
    store
        .set_sync_error(src.id, "channels", "boom http://p.test/live/a/b/1.ts")
        .await
        .unwrap();
    let (ok, err) = store.sync_state(src.id, "channels").await.unwrap().unwrap();
    assert!(ok.is_none());
    assert!(!err.unwrap().contains("/a/b/"));
    store.set_sync_ok(src.id, "channels", 5).await.unwrap();
    assert_eq!(
        store
            .sync_state(src.id, "channels")
            .await
            .unwrap()
            .unwrap()
            .0,
        Some(5)
    );
    store
        .update_source(
            src.id,
            &SourceUpdate {
                user_level: Some("admin".into()),
                server_version: Some("0.32.0".into()),
                server_tz: Some("UTC".into()),
                caps: 7,
                stream_limit: Some(4),
            },
        )
        .await
        .unwrap();
    let s = store.source(src.id).await.unwrap();
    assert_eq!((s.caps, s.user_level.as_deref()), (7, Some("admin")));
    assert_eq!(s.stream_limit, Some(4));
}

/// PLAN §2.6: a test asserts no secret is ever written to the DB. The secret exists only in the
/// keychain; the DB receives the reference. We write a full dataset and scan the raw file bytes
/// (main DB + WAL) for the secret.
#[tokio::test]
async fn no_secret_is_ever_written_to_the_db() {
    const SECRET: &str = "sk_test_SUPERSECRETVALUE_0123456789";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sqlite");
    let store = Store::open(&path).await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "Test".into(),
            base_url: Some("https://dispatcharr.test".into()),
            secret_ref: Some("downlink/source/abc/api_key".into()),
            username: Some("alice".into()),
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let groups = store
        .sync_groups(src.id, &[("g1".into(), "News".into())])
        .await
        .unwrap();
    store
        .sync_channels(src.id, &[ch("1", "Channel One", 1.0)], &groups)
        .await
        .unwrap();
    // An error that happened to embed the secret must be scrubbed by the redactor first.
    let mut set = crate::redact::RedactionSet::new();
    set.add(SECRET);
    let e = crate::AppError::with_secrets(
        crate::ErrorCode::Network,
        format!("bad X-API-Key: {SECRET}"),
        &set,
    );
    store
        .set_sync_error(src.id, "channels", &e.message)
        .await
        .unwrap();
    store
        .add_open_session(src.id, "hls", "session-token")
        .await
        .unwrap();
    store.close().await;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        // WAL/shm files may be removed concurrently while the pool shuts down.
        let Ok(bytes) = std::fs::read(entry.unwrap().path()) else {
            continue;
        };
        assert!(!bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()));
    }
    let row = Store::open(&path)
        .await
        .unwrap()
        .source(src.id)
        .await
        .unwrap();
    assert!(row.secret_ref.unwrap().starts_with("downlink/source/"));
}

#[tokio::test]
async fn pending_migration_backs_up_and_keeps_two_copies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("d.sqlite");
    let bak = |n: usize| super::backup_path(&path, n);

    // Fresh database: nothing to back up.
    let s = Store::open(&path).await.unwrap();
    s.insert_source(&NewSource {
        kind: "m3u".into(),
        name: "KEEP".into(),
        base_url: None,
        secret_ref: None,
        username: None,
        tls_insecure: false,
        created_at: 1,
    })
    .await
    .unwrap();
    s.close().await;
    assert!(!bak(1).exists());
    // Up to date: still nothing.
    Store::open(&path).await.unwrap().close().await;
    assert!(!bak(1).exists());

    // Simulate "older app version": forget the newest migration so it is pending again, then run
    // the pre-migration backup step directly (re-applying the migration itself would fail here
    // because its effects are still in the schema).
    let s = Store::open(&path).await.unwrap();
    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version = (SELECT max(version) FROM _sqlx_migrations)",
    )
    .execute(s.pool())
    .await
    .unwrap();
    for round in 1..=3 {
        super::backup_before_migrate(s.pool(), &path).await.unwrap();
        assert!(bak(1).exists(), "round {round}");
        assert_eq!(bak(2).exists(), round >= 2, "round {round}");
        assert!(
            !super::backup_path(&path, 3).exists(),
            "only two copies are kept"
        );
    }
    s.close().await;
    // The backup is a usable database holding the data.
    let url = format!("sqlite://{}?mode=ro", bak(1).display());
    let b = sqlx::SqlitePool::connect(&url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM source")
        .fetch_one(&b)
        .await
        .unwrap();
    assert_eq!(name, "KEEP");
}

#[tokio::test]
async fn whole_vod_catalogue_sync_keeps_ids_and_details_and_drops_removed_titles() {
    use crate::source::{VodItem, VodKind};
    let (store, src, _) = seeded().await;
    let movie = |id: &str, title: &str, cat: &str| VodItem {
        kind: Some(VodKind::Movie),
        ext_id: id.into(),
        title: title.into(),
        category_ext: Some(cat.into()),
        ..Default::default()
    };
    let first = [movie("1", "Alpha Test", "a"), movie("2", "Beta Test", "b")];
    store
        .sync_vod_catalog(src.id, VodKind::Movie, &first, 10)
        .await
        .unwrap();
    let alpha = store
        .vod_by_ext(src.id, "movie", "1")
        .await
        .unwrap()
        .unwrap();
    // A detail fetched on demand is not erased by the next list refresh.
    let mut detail = movie("1", "Alpha Test", "a");
    detail.plot = Some("Fetched on demand.".into());
    store.upsert_vod_items(src.id, &[detail], 11).await.unwrap();

    let second = [movie("3", "Gamma Test", "a"), movie("1", "Alpha Test", "a")];
    store
        .sync_vod_catalog(src.id, VodKind::Movie, &second, 20)
        .await
        .unwrap();
    let (rows, total) = store
        .vod_catalog_page(src.id, VodKind::Movie, None, None, 0, 10)
        .await
        .unwrap();
    assert_eq!(total, 2, "Beta is gone");
    assert_eq!(
        rows[0].name, "Gamma Test",
        "provider order follows the new list"
    );
    assert_eq!(rows[1].id, alpha.id, "row ids are stable");
    assert_eq!(rows[1].description.as_deref(), Some("Fetched on demand."));
    assert_eq!(
        store
            .vod_catalog_counts(src.id, VodKind::Movie)
            .await
            .unwrap()["a"],
        2
    );
    // Phrase search: the words in order, the last one a prefix.
    let (hits, _) = store
        .vod_catalog_page(src.id, VodKind::Movie, None, Some("gamma te"), 0, 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let (hits, _) = store
        .vod_catalog_page(src.id, VodKind::Movie, None, Some("test gamma"), 0, 10)
        .await
        .unwrap();
    assert!(hits.is_empty());
    assert_eq!(
        store.search_vod("beta", 10).await.unwrap().len(),
        0,
        "removed from search"
    );
    assert_eq!(store.search_vod("alpha", 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn programme_search_indexes_the_opening_of_long_descriptions() {
    let (store, src, groups) = seeded().await;
    store
        .sync_channels(src.id, &[ch("1", "Channel One", 1.0)], &groups)
        .await
        .unwrap();
    let cid = store.channel_by_ext(src.id, "1").await.unwrap().unwrap().id;
    let long = format!("Openingword {} trailingword", "filler ".repeat(60));
    let row = |description: &str| ProgrammeRow {
        channel_id: cid,
        start_utc: 100,
        end_utc: 200,
        ext_id: None,
        title: Some("Title".into()),
        subtitle: None,
        description: Some(description.into()),
        season: None,
        episode: None,
        flags: 0,
        icon: None,
    };
    let hits = |q: &'static str| {
        let store = store.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM programme_fts WHERE programme_fts MATCH ?",
            )
            .bind(q)
            .fetch_one(store.pool())
            .await
            .unwrap()
        }
    };
    store.upsert_programmes(&[row(&long)]).await.unwrap();
    assert_eq!(hits("openingword").await, 1);
    assert_eq!(hits("trailingword").await, 0, "past the indexed opening");
    // Updates and deletes remove exactly what was indexed (no stale terms left behind).
    store
        .upsert_programmes(&[row("Replacement text")])
        .await
        .unwrap();
    assert_eq!(hits("openingword").await, 0);
    assert_eq!(hits("replacement").await, 1);
    store.prune_programmes(1_000).await.unwrap();
    assert_eq!(hits("replacement").await, 0);
    assert_eq!(hits("title").await, 0);
}
