//! Save, list, update and delete against an in-memory database with invented channels.

use std::collections::HashMap;

use super::*;
use crate::store::{ChannelUpsert, NewSource};

const NOW: i64 = 1_795_780_800;

fn ch(ext: &str, name: &str) -> ChannelUpsert {
    ChannelUpsert {
        ext_id: ext.into(),
        name: name.into(),
        number: ext.parse().ok(),
        group_ext_id: Some("g1".into()),
        streams: vec![format!("s{ext}")],
        ..Default::default()
    }
}

async fn sync(store: &Store, src: i64, exts: &[&str]) -> HashMap<String, i64> {
    let groups = store
        .sync_groups(src, &[("g1".into(), "News".into())])
        .await
        .unwrap();
    let chans: Vec<ChannelUpsert> = exts
        .iter()
        .map(|e| ch(e, &format!("Channel {e}")))
        .collect();
    store.sync_channels(src, &chans, &groups).await.unwrap();
    sqlx::query_as::<_, (String, i64)>("SELECT ext_id, id FROM channel")
        .fetch_all(store.pool())
        .await
        .unwrap()
        .into_iter()
        .collect()
}

async fn seeded() -> (Store, i64, HashMap<String, i64>) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "Test".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: NOW,
        })
        .await
        .unwrap();
    let ids = sync(&store, src.id, &["1", "2", "3"]).await;
    (store, src.id, ids)
}

fn save_req(id: Option<i64>, name: &str, tiles: &[(i64, i64)]) -> ValidSave {
    ValidSave {
        id,
        name: name.into(),
        layout_style: "grid".into(),
        audible: 0,
        rail_json: "{}".into(),
        tiles: tiles.to_vec(),
    }
}

fn slots(rows: &[TileRow]) -> Vec<(i64, i64)> {
    rows.iter().map(|t| (t.slot, t.channel_id)).collect()
}

#[tokio::test]
async fn save_list_get_and_update_replace_tiles() {
    let (store, _, ids) = seeded().await;
    let id = save(
        &store,
        &save_req(None, "Morning wall", &[(0, ids["1"]), (2, ids["2"])]),
        NOW,
    )
    .await
    .unwrap();
    let other = save(&store, &save_req(None, "Late wall", &[]), NOW + 5)
        .await
        .unwrap();

    let all = list(&store).await.unwrap();
    assert_eq!(
        all.iter().map(|c| c.id).collect::<Vec<_>>(),
        [other, id],
        "newest first"
    );
    let row = get(&store, id).await.unwrap().unwrap();
    assert_eq!(row.name, "Morning wall");
    assert_eq!(row.created_utc, NOW);
    assert_eq!(
        slots(&tiles(&store, Some(id)).await.unwrap()),
        [(0, ids["1"]), (2, ids["2"])]
    );

    let mut upd = save_req(Some(id), "Morning wall 2", &[(1, ids["3"])]);
    upd.layout_style = "featured".into();
    upd.audible = 1;
    upd.rail_json = r#"{"feeds":[]}"#.into();
    assert_eq!(save(&store, &upd, NOW + 10).await.unwrap(), id);
    let row = get(&store, id).await.unwrap().unwrap();
    assert_eq!(
        (row.name.as_str(), row.layout_style.as_str(), row.audible),
        ("Morning wall 2", "featured", 1)
    );
    assert_eq!(row.rail_json, r#"{"feeds":[]}"#);
    assert_eq!((row.created_utc, row.updated_utc), (NOW, NOW + 10));
    assert_eq!(
        slots(&tiles(&store, Some(id)).await.unwrap()),
        [(1, ids["3"])]
    );
    assert_eq!(tiles(&store, None).await.unwrap().len(), 1);
}

#[tokio::test]
async fn updating_a_missing_center_fails_and_unknown_channels_are_skipped() {
    let (store, _, ids) = seeded().await;
    assert!(
        save(&store, &save_req(Some(999), "Gone", &[]), NOW)
            .await
            .is_err()
    );
    let id = save(
        &store,
        &save_req(None, "Wall", &[(0, ids["1"]), (1, 987_654)]),
        NOW,
    )
    .await
    .unwrap();
    assert_eq!(
        slots(&tiles(&store, Some(id)).await.unwrap()),
        [(0, ids["1"])]
    );
}

#[tokio::test]
async fn delete_cascades_and_tiles_survive_a_resync_until_the_channel_goes() {
    let (store, src, ids) = seeded().await;
    let id = save(
        &store,
        &save_req(None, "Wall", &[(0, ids["1"]), (1, ids["2"])]),
        NOW,
    )
    .await
    .unwrap();

    // Same channels again: ids are kept, so are the tiles.
    let again = sync(&store, src, &["1", "2", "3"]).await;
    assert_eq!(again, ids);
    assert_eq!(tiles(&store, Some(id)).await.unwrap().len(), 2);

    // Channel 2 disappears from the source: its tile goes with it.
    sync(&store, src, &["1", "3"]).await;
    assert_eq!(
        slots(&tiles(&store, Some(id)).await.unwrap()),
        [(0, ids["1"])]
    );

    delete(&store, id).await.unwrap();
    assert!(get(&store, id).await.unwrap().is_none());
    assert!(tiles(&store, None).await.unwrap().is_empty());
    delete(&store, id).await.unwrap(); // already gone is fine
}
