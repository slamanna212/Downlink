#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Network discipline (PLAN §11.4, CLAUDE.md rule 4): nothing but an actual play touches a stream endpoint.

use std::sync::Arc;

use downlink_core::auth::MemorySecretStore;
use downlink_core::service::{AddSourceInput, GuideArgs, ListChannelsArgs, Service};
use downlink_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_service_key_abcdef123";

const START: i64 = 1_735_725_600;

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<Value>(include_str!(concat!("fixtures/dispatcharr/", $name)))
            .unwrap()
    };
}

async fn get(server: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_server(server: &MockServer) {
    get(server, "/api/core/version/", fixture!("version.json")).await;
    get(server, "/api/accounts/users/me/", fixture!("me.json")).await;
    get(server, "/api/core/settings/", fixture!("settings.json")).await;
    get(
        server,
        "/api/core/streamprofiles/",
        fixture!("stream_profiles.json"),
    )
    .await;
    get(server, "/api/channels/groups/", fixture!("groups.json")).await;
    get(server, "/api/epg/grid/", fixture!("grid.json")).await;
    for (n, f) in [
        ("1", fixture!("channels_page1.json")),
        ("2", fixture!("channels_page2.json")),
        ("3", fixture!("channels_page3.json")),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/channels/channels/"))
            .and(query_param("page", n))
            .respond_with(ResponseTemplate::new(200).set_body_json(f))
            .mount(server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
}

fn input(server: &MockServer) -> AddSourceInput {
    serde_json::from_value(json!({
        "kind": "dispatcharr", "name": "Test", "url": server.uri(), "apiKey": KEY, "allowSelfSigned": false
    }))
    .unwrap()
}

struct Env {
    svc: Arc<Service>,

    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let store = Store::open_memory().await.unwrap();
    let svc = Service::with_store(store, secrets, dir.path().join("img"), "0.0.0-test".into());
    Env { svc, _dir: dir }
}

async fn added() -> (MockServer, Env, i64) {
    let server = MockServer::start().await;
    mount_server(&server).await;
    // Anything that looks like a stream endpoint answers 500 so a stray request is also visible
    // as a failure in the code under test, not only in the request log.
    for p in [
        "/proxy/ts/stream",
        "/proxy/hls",
        "/proxy/vod",
        "/proxy/catchup",
        "/live",
        "/movie",
        "/series",
        "/timeshift",
    ] {
        Mock::given(wiremock::matchers::path_regex(format!("^{p}(/|$)")))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
    }
    let e = env().await;
    let s = e.svc.add_source(input(&server)).await.unwrap();
    (server, e, s.id)
}

fn is_stream_path(p: &str) -> bool {
    [
        "/proxy/ts/stream",
        "/proxy/hls",
        "/proxy/vod",
        "/proxy/catchup",
        "/live/",
        "/movie/",
        "/series/",
        "/timeshift/",
    ]
    .iter()
    .any(|s| p.starts_with(s))
}

/// PLAN §11.4 / M2 acceptance: browsing the guide for 60 s with the cursor moving (10 focus changes
/// per second, dwell prefetch on every row held > 300 ms) generates zero requests to stream endpoints.
#[tokio::test]
async fn guide_browsing_with_dwell_prefetch_never_touches_stream_endpoints() {
    let (server, e, _sid) = added().await;
    let before = server.received_requests().await.unwrap().len();

    let channels = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    assert!(!channels.is_empty());
    let ids: Vec<i64> = channels.iter().map(|c| c.id).collect();

    // 60 s x 10 Hz of cursor movement over the list.
    for tick in 0..600usize {
        let id = ids[tick % ids.len()];
        // Dwell prefetch (PLAN §4.6): now/next, the play plan and the logo, all from SQLite/cache.
        e.svc.get_now_next(&[id]).await.unwrap();
        let _plan = downlink_core::catalog::resolve_play_plan(e.svc.store(), e.svc.as_ref(), id)
            .await
            .unwrap();
        if tick % 10 == 0 {
            // Scrolling the guide re-queries windows and groups.
            e.svc
                .get_guide(GuideArgs {
                    start: START,
                    end: START + 4 * 3600,
                    channel_offset: (tick as i64 / 10) % 3,
                    channel_limit: Some(10),
                    group_id: None,
                })
                .await
                .unwrap();
            e.svc.list_groups(None).await.unwrap();
        }
        if tick % 50 == 0 {
            e.svc.search("test", 20).await.unwrap();
            e.svc.get_home().await.unwrap();
        }
        if let Some(logo) = channels[tick % channels.len()].logo.as_deref() {
            let _ = e.svc.image(logo).await; // images are not stream endpoints
        }
    }

    let reqs = server.received_requests().await.unwrap();
    let stream_hits: Vec<String> = reqs
        .iter()
        .map(|r| r.url.path().to_owned())
        .filter(|p| is_stream_path(p))
        .collect();
    assert!(
        stream_hits.is_empty(),
        "stream endpoints were requested: {stream_hits:?}"
    );
    // Beyond the initial sync the only traffic allowed is image (logo) fetches; the mock serves no
    // logos, so those repeat. Everything else is answered from SQLite.
    let other: Vec<String> = reqs[before..]
        .iter()
        .map(|r| r.url.path().to_owned())
        .filter(|p| !p.starts_with("/api/channels/logos/"))
        .collect();
    assert!(other.is_empty(), "browsing made API requests: {other:?}");
}
