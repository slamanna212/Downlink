#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Media-server façade: VOD rows carry source ids into a Plex match; tokens stay in the secret
//! store and never reach SQLite. Synthetic titles only; Plex is a mock.

use std::sync::Arc;

use downlink_core::auth::{MemorySecretStore, SecretStore, SecretString};
use downlink_core::service::{MediaTargetArg, Service};
use downlink_core::source::{VodItem, VodKind};
use downlink_core::store::{NewSource, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "plex_service_token_123456";

async fn plex() -> MockServer {
    let s = MockServer::start().await;
    let get = |p: &str, body: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path(p.to_owned()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
    };
    get(
        "/identity",
        json!({"MediaContainer": {"machineIdentifier": "m1"}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/sections",
        json!({"MediaContainer": {"Directory": [
            {"key": "1", "type": "movie"}, {"key": "2", "type": "show"}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/sections/1/all",
        json!({"MediaContainer": {"totalSize": 1, "Metadata": [
            {"ratingKey": "11", "title": "Plex Name Of Film", "year": 2021, "Guid": [{"id": "tmdb://424242"}]}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/sections/2/all",
        json!({"MediaContainer": {"totalSize": 1, "Metadata": [
            {"ratingKey": "21", "title": "Sample Series", "Guid": [{"id": "tmdb://515151"}]}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/metadata/21/allLeaves",
        json!({"MediaContainer": {"Metadata": [{"ratingKey": "31", "parentIndex": 2, "index": 3}]}}),
    )
    .mount(&s)
    .await;
    s
}

#[tokio::test]
async fn vod_movie_and_episode_match_and_tokens_stay_out_of_sqlite() {
    let plex = plex().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "S".into(),
            base_url: Some("https://dispatcharr.test".into()),
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let rows = store
        .upsert_vod_items(
            src.id,
            &[
                VodItem {
                    kind: Some(VodKind::Movie),
                    ext_id: "m1".into(),
                    title: "UK: Provider Name Of Film (2021)".into(),
                    tmdb_id: Some("424242".into()),
                    ..Default::default()
                },
                VodItem {
                    kind: Some(VodKind::Series),
                    ext_id: "s1".into(),
                    title: "Sample Series".into(),
                    tmdb_id: Some("515151".into()),
                    ..Default::default()
                },
                VodItem {
                    kind: Some(VodKind::Episode),
                    ext_id: "e1".into(),
                    parent_ext: Some("s1".into()),
                    title: "Episode 3".into(),
                    season: Some(2),
                    episode: Some(3),
                    ..Default::default()
                },
            ],
            1,
        )
        .await
        .unwrap();
    assert_eq!(rows[0].tmdb_id.as_deref(), Some("424242"));
    let svc = Service::with_store(store, secrets.clone(), dir.path().join("img"), "t".into());

    // Nothing configured: no match, no error.
    assert!(
        svc.media_server_match(MediaTargetArg::Vod { id: rows[0].id })
            .await
            .unwrap()
            .is_none()
    );

    svc.plex_connect_direct(&plex.uri(), SecretString::new(TOKEN))
        .await
        .unwrap();
    let status = svc.media_server_status().await.unwrap();
    assert!(status.plex.signed_in);
    assert_eq!(status.plex.server_id.as_deref(), Some("m1"));

    let m = svc
        .media_server_match(MediaTargetArg::Vod { id: rows[0].id })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m.title, "Plex Name Of Film");
    assert!(
        m.url.contains("/server/m1/") && m.url.ends_with("%2F11"),
        "{}",
        m.url
    );

    let ep = svc
        .media_server_match(MediaTargetArg::Vod { id: rows[2].id })
        .await
        .unwrap()
        .unwrap();
    assert!(ep.url.ends_with("%2F31"), "{}", ep.url);
    assert_eq!(serde_json::to_value(&ep).unwrap()["kind"], json!("episode"));

    // The token is in the secret store, not in SQLite.
    assert!(secrets.get("mediaserver:plex:server").unwrap().is_some());
    let (kv, _) = svc
        .store()
        .kv_get("mediaserver.plex")
        .await
        .unwrap()
        .unwrap();
    assert!(!kv.contains(TOKEN), "{kv}");

    svc.media_server_sign_out().await.unwrap();
    assert!(secrets.get("mediaserver:plex:server").unwrap().is_none());
    assert!(
        svc.media_server_match(MediaTargetArg::Vod { id: rows[0].id })
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn target_args_deserialise_from_the_ui_shape() {
    let v: MediaTargetArg =
        serde_json::from_value(json!({"kind": "programme", "id": "4:1700000000"})).unwrap();
    assert!(matches!(v, MediaTargetArg::Programme { .. }));
    let v: MediaTargetArg = serde_json::from_value(json!({"kind": "vod", "id": 7})).unwrap();
    assert!(matches!(v, MediaTargetArg::Vod { id: 7 }));
}
