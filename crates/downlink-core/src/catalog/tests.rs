use std::collections::HashMap;

use super::*;
use crate::auth::{ApiKeyAuth, MemorySecretStore, SecretStore, SecretString};
use crate::source::StreamMode;
use crate::store::{ChannelUpsert, NewSource};

async fn seeded(kind: &str) -> (Store, i64, HashMap<String, i64>) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: kind.into(),
            name: "T".into(),
            base_url: Some("https://dispatcharr.test".into()),
            secret_ref: Some("ref/key".into()),
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let g = store
        .sync_groups(
            src.id,
            &[
                ("1".into(), "News".into()),
                ("2".into(), "Misc".into()),
                ("3".into(), "Empty".into()),
            ],
        )
        .await
        .unwrap();
    (store, src.id, g)
}

fn ch(ext: &str, name: &str, number: f64, group: &str) -> ChannelUpsert {
    ChannelUpsert {
        ext_id: ext.into(),
        uuid: Some(format!("uuid-{ext}")),
        number: Some(number),
        name: name.into(),
        group_ext_id: Some(group.into()),
        tvg_id: Some(format!("{ext}.test")),
        sort_key: Some(name.to_lowercase()),
        stream_redirect: StreamMode::Proxy,
        ..Default::default()
    }
}

async fn fill() -> (Store, i64) {
    let (store, sid, g) = seeded("dispatcharr").await;
    let mut adult = ch("3", "Adult Example", 3.0, "2");
    adult.is_adult = true;
    let mut radio = ch("4", "Radio One", 4.0, "2");
    radio.is_radio = true;
    let mut drm = ch("5", "Drm Channel", 5.0, "2");
    drm.is_drm = true;
    let mut redirect = ch("2", "Channel Two", 2.5, "1");
    redirect.stream_redirect = StreamMode::Redirect;
    store
        .sync_channels(
            sid,
            &[
                ch("1", "Channel One", 1.0, "1"),
                redirect,
                adult,
                radio,
                drm,
                ch("6", "Channel Six", 6.0, "1"),
                ch("10", "Ten News", 10.0, "1"),
                ch("11", "Eleven", 11.0, "1"),
            ],
            &g,
        )
        .await
        .unwrap();
    (store, sid)
}

fn names(v: &[ChannelRow]) -> Vec<&str> {
    v.iter().map(|c| c.name.as_str()).collect()
}

#[tokio::test]
async fn default_visibility_hides_adult_radio_drm() {
    let (store, _) = fill().await;
    let v = list_channels(&store, &ChannelFilter::default(), 100, 0)
        .await
        .unwrap();
    assert_eq!(
        names(&v),
        [
            "Channel One",
            "Channel Two",
            "Channel Six",
            "Ten News",
            "Eleven"
        ]
    );
    let all = ChannelFilter {
        include_adult: true,
        include_radio: true,
        include_drm: true,
        ..Default::default()
    };
    assert_eq!(list_channels(&store, &all, 100, 0).await.unwrap().len(), 8);
    let adult = ChannelFilter {
        include_adult: true,
        ..Default::default()
    };
    assert!(
        names(&list_channels(&store, &adult, 100, 0).await.unwrap()).contains(&"Adult Example")
    );
}

#[tokio::test]
async fn search_respects_visibility_and_prefixes() {
    let (store, _) = fill().await;
    let f = |s: &str| ChannelFilter {
        search: Some(s.into()),
        ..Default::default()
    };
    let mut found: Vec<String> = list_channels(&store, &f("chan"), 10, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    found.sort();
    assert_eq!(found, ["Channel One", "Channel Six", "Channel Two"]);
    assert!(
        list_channels(&store, &f("adult"), 10, 0)
            .await
            .unwrap()
            .is_empty(),
        "adult hidden even in search"
    );
    assert!(
        list_channels(&store, &f("   "), 10, 0).await.unwrap().len() >= 5,
        "blank search lists"
    );
    assert!(
        list_channels(&store, &f("\"*("), 10, 0).await.is_ok(),
        "no FTS syntax injection"
    );
}

#[tokio::test]
async fn numeric_search_ranks_exact_then_proximity() {
    let (store, _) = fill().await;
    let f = ChannelFilter {
        search: Some("2.5".into()),
        ..Default::default()
    };
    let r = list_channels(&store, &f, 10, 0).await.unwrap();
    assert_eq!(r[0].name, "Channel Two");
}

#[tokio::test]
async fn groups_count_only_visible_and_skip_empty() {
    let (store, _) = fill().await;
    let g = list_groups(&store, &ChannelFilter::default())
        .await
        .unwrap();
    assert_eq!(
        g.iter()
            .map(|x| (x.name.as_str(), x.channel_count))
            .collect::<Vec<_>>(),
        [("News", 5)]
    );
    let g = list_groups(
        &store,
        &ChannelFilter {
            include_adult: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(g.len(), 2);
}

#[tokio::test]
async fn number_entry_exact_else_next_higher() {
    let (store, _) = fill().await;
    let f = ChannelFilter::default();
    assert_eq!(
        channel_by_number(&store, &f, 2.5)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Channel Two"
    );
    assert_eq!(
        channel_by_number(&store, &f, 7.0)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Ten News"
    );
    assert_eq!(
        channel_by_number(&store, &f, 3.0)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Channel Six",
        "adult 3 skipped"
    );
    assert!(channel_by_number(&store, &f, 99.0).await.unwrap().is_none());
}

#[test]
fn sync_filter_rules() {
    let base = SourceChannel {
        user_level: 1,
        ..Default::default()
    };
    assert_eq!(
        sync_filter(
            &SourceChannel {
                hidden: true,
                ..base.clone()
            },
            None
        ),
        Some(DropReason::Hidden)
    );
    assert_eq!(
        sync_filter(&base, Some(UserLevel::Streamer)),
        Some(DropReason::AboveUserLevel)
    );
    assert_eq!(sync_filter(&base, Some(UserLevel::Standard)), None);
    assert_eq!(sync_filter(&base, None), None);
}

#[tokio::test]
async fn play_plan_from_sqlite_proxy_and_redirect() {
    let (store, sid) = fill().await;
    let secrets = MemorySecretStore::new();
    secrets
        .set("ref/key", &SecretString::new("sk_test_secret_key"))
        .unwrap();

    let one = store.channel_by_ext(sid, "1").await.unwrap().unwrap();
    let plan = resolve_play_plan(&store, &ApiKeyAuth(&secrets), one.id)
        .await
        .unwrap();
    assert_eq!(plan.url, "https://dispatcharr.test/proxy/ts/stream/uuid-1");
    assert_eq!(plan.headers[0].0, "X-API-Key");
    assert!(!plan.needs_redirect_resolve);

    let two = store.channel_by_ext(sid, "2").await.unwrap().unwrap();
    let plan = resolve_play_plan(&store, &ApiKeyAuth(&secrets), two.id)
        .await
        .unwrap();
    assert!(plan.needs_redirect_resolve);
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(!shown.contains("sk_test_secret_key"), "{shown}");
    assert!(shown.contains("***"));
}

#[tokio::test]
async fn play_plan_errors() {
    let (store, sid) = fill().await;
    let secrets = MemorySecretStore::new();
    let drm = store.channel_by_ext(sid, "5").await.unwrap().unwrap();
    let e = resolve_play_plan(&store, &ApiKeyAuth(&secrets), drm.id)
        .await
        .unwrap_err();
    assert_eq!(e.code, crate::ErrorCode::Media);
    assert!(e.message.contains("DRM"));
    let one = store.channel_by_ext(sid, "1").await.unwrap().unwrap();
    let e = resolve_play_plan(&store, &ApiKeyAuth(&secrets), one.id)
        .await
        .unwrap_err();
    assert_eq!(e.code, crate::ErrorCode::Auth, "missing keychain entry");
    assert_eq!(
        resolve_play_plan(&store, &ApiKeyAuth(&secrets), 9999)
            .await
            .unwrap_err()
            .code,
        crate::ErrorCode::Local
    );
}

#[tokio::test]
async fn m3u_play_plan_uses_stored_url_and_headers() {
    let (store, sid, g) = seeded("m3u").await;
    let mut c = ch("x", "Plain", 1.0, "1");
    c.play_url = Some("http://stream.test/a.ts?token=abc12345".into());
    c.play_headers = Some(r#"{"Referer":"http://ref.test/","X-Token":"hdrsecret1"}"#.into());
    store.sync_channels(sid, &[c], &g).await.unwrap();
    let id = store.channel_by_ext(sid, "x").await.unwrap().unwrap().id;
    let plan = resolve_play_plan(&store, &ApiKeyAuth(&MemorySecretStore::new()), id)
        .await
        .unwrap();
    assert_eq!(plan.headers.len(), 2);
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(
        !shown.contains("abc12345") && !shown.contains("hdrsecret1"),
        "{shown}"
    );
}
