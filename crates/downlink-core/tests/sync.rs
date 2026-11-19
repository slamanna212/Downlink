#![allow(clippy::unwrap_used, clippy::expect_used)]
//! End-to-end sync against a mock Dispatcharr: connect -> channels -> guide -> play plan, plus the
//! network-discipline test (PLAN §11.4.4): a guide-browsing session makes zero stream requests.

use downlink_core::auth::{ApiKeyAuth, MemorySecretStore, SecretStore, SecretString};
use downlink_core::catalog::{self, ChannelFilter};
use downlink_core::source::SourceId;
use downlink_core::source::StreamMode;
use downlink_core::source::dispatcharr::{ClientOptions, Credentials, DispatcharrClient};
use downlink_core::store::{NewSource, Store};
use downlink_core::{epg, sync};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_synthetic_key_123456";

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<serde_json::Value>(include_str!(concat!(
            "fixtures/dispatcharr/",
            $name
        )))
        .unwrap()
    };
}

async fn mount(server: &MockServer, p: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn mount_all(server: &MockServer) {
    mount(server, "/api/core/version/", fixture!("version.json")).await;
    mount(server, "/api/accounts/users/me/", fixture!("me.json")).await;
    mount(server, "/api/core/settings/", fixture!("settings.json")).await;
    mount(
        server,
        "/api/core/streamprofiles/",
        fixture!("stream_profiles.json"),
    )
    .await;
    mount(server, "/api/channels/groups/", fixture!("groups.json")).await;
    mount(server, "/api/epg/grid/", fixture!("grid.json")).await;
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
}

async fn setup(server: &MockServer) -> (Store, i64, DispatcharrClient, MemorySecretStore) {
    let store = Store::open_memory().await.unwrap();
    let secrets = MemorySecretStore::new();
    let row = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "Test".into(),
            base_url: Some(server.uri()),
            secret_ref: Some("ref/key".into()),
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    secrets.set("ref/key", &SecretString::new(KEY)).unwrap();
    let client = DispatcharrClient::new(
        SourceId(row.id),
        &server.uri(),
        Credentials::ApiKey(SecretString::new(KEY)),
        ClientOptions::default(),
    )
    .unwrap();
    (store, row.id, client, secrets)
}

#[tokio::test]
async fn full_sync_filters_normalises_and_flags_redirect() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let (store, sid, client, secrets) = setup(&server).await;

    let info = sync::connect_and_record(&client, &store, sid)
        .await
        .unwrap();
    let counts = sync::sync_channels(&client, &store, sid, info.user_level)
        .await
        .unwrap();
    assert_eq!(counts.groups, 2);
    assert_eq!(counts.dropped_hidden, 1);
    assert_eq!(
        (counts.channels, counts.inserted, counts.removed),
        (5, 5, 0)
    );

    let src = store.source(sid).await.unwrap();
    assert_eq!(
        (
            src.server_version.as_deref(),
            src.user_level.as_deref(),
            src.server_tz.as_deref()
        ),
        (Some("0.32.0"), Some("admin"), Some("Europe/Berlin"))
    );
    assert!(src.last_sync_at.is_some() && src.caps > 0);

    // Normalisation: quality split out; stable ids; failover order kept.
    let one = store.channel_by_ext(sid, "1").await.unwrap().unwrap();
    assert_eq!(
        (one.name.as_str(), one.quality.as_deref(), one.catchup_days),
        ("Channel One", Some("HD"), 7)
    );
    assert_eq!(
        store.channel_stream_ids(one.id).await.unwrap(),
        ["11", "12"]
    );
    let six = store.channel_by_ext(sid, "6").await.unwrap().unwrap();
    assert_eq!(
        (
            six.name.as_str(),
            six.quality.as_deref(),
            six.tvg_id.clone()
        ),
        ("Channel Six", Some("FHD"), None)
    );
    assert!(
        store.channel_by_ext(sid, "5").await.unwrap().is_none(),
        "hidden_from_output never stored"
    );

    // Effective stream profile: ch2 -> Redirect(4); ch1 null -> default Proxy(3).
    assert_eq!(
        store
            .channel_by_ext(sid, "2")
            .await
            .unwrap()
            .unwrap()
            .stream_redirect,
        StreamMode::Redirect
    );
    assert_eq!(one.stream_redirect, StreamMode::Proxy);

    // Catalog default view hides adult + radio.
    let v = catalog::list_channels(&store, &ChannelFilter::default(), 50, 0)
        .await
        .unwrap();
    assert_eq!(
        v.iter().map(|c| c.ext_id.as_str()).collect::<Vec<_>>(),
        ["1", "2", "6"]
    );

    // Re-sync is stable.
    let again = sync::sync_channels(&client, &store, sid, info.user_level)
        .await
        .unwrap();
    assert_eq!((again.inserted, again.updated, again.removed), (0, 5, 0));
    assert_eq!(
        store.channel_by_ext(sid, "1").await.unwrap().unwrap().id,
        one.id
    );

    // Guide: join on tvg_id; unreadable row skipped at the client.
    let now = 1_735_725_600 + 1800; // 10:30Z
    let gc = epg::sync_guide(&client, &store, sid, epg::visible_window(now))
        .await
        .unwrap();
    assert_eq!((gc.programmes, gc.unmatched), (3, 0));
    let nn = epg::now_next(&store, one.id, now).await.unwrap();
    assert_eq!(nn.now.unwrap().title.as_deref(), Some("Morning Show"));
    assert_eq!(nn.next.unwrap().title.as_deref(), Some("Midday News"));

    // Play plan resolves from SQLite alone.
    let plan = catalog::resolve_play_plan(&store, &ApiKeyAuth(&secrets), one.id)
        .await
        .unwrap();
    assert!(
        plan.url
            .ends_with("/proxy/ts/stream/00000000-0000-4000-8000-000000000001")
    );
}

#[tokio::test]
async fn failed_sync_leaves_cache_and_records_redacted_error() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let (store, sid, client, _) = setup(&server).await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let e = sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap_err();
    assert_eq!(e.code, downlink_core::ErrorCode::Server);
    assert_eq!(
        catalog::list_channels(&store, &ChannelFilter::default(), 50, 0)
            .await
            .unwrap()
            .len(),
        3
    );
    let (_, err) = store.sync_state(sid, "channels").await.unwrap().unwrap();
    assert!(err.is_some());
}

#[tokio::test]
async fn profile_failure_keeps_stored_redirect_flags() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let (store, sid, client, _) = setup(&server).await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    server.reset().await;
    mount_all(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/core/streamprofiles/"))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(1)
        .mount(&server)
        .await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    assert_eq!(
        store
            .channel_by_ext(sid, "2")
            .await
            .unwrap()
            .unwrap()
            .stream_redirect,
        StreamMode::Redirect
    );
}

#[tokio::test]
async fn current_programs_fill() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/epg/current-programs/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture!("current_programs.json")))
        .mount(&server)
        .await;
    let (store, sid, client, _) = setup(&server).await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    assert_eq!(epg::sync_current(&client, &store, sid).await.unwrap(), 2);
    let two = store.channel_by_ext(sid, "2").await.unwrap().unwrap();
    assert!(
        epg::now_next(&store, two.id, 1_735_725_600 + 3600)
            .await
            .unwrap()
            .now
            .is_some()
    );
}

/// PLAN §11.4.4 / CLAUDE.md rule 4: a browsing session (connect, sync, guide, search, play-plan
/// building, catch-up position/delete housekeeping) never requests a stream endpoint.
#[tokio::test]
async fn network_discipline_zero_stream_endpoint_requests() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    let (store, sid, client, secrets) = setup(&server).await;
    let info = sync::connect_and_record(&client, &store, sid)
        .await
        .unwrap();
    sync::sync_channels(&client, &store, sid, info.user_level)
        .await
        .unwrap();
    epg::sync_guide(&client, &store, sid, epg::visible_window(1_735_727_400))
        .await
        .unwrap();
    for q in ["chan", "2.5", "one"] {
        catalog::list_channels(
            &store,
            &ChannelFilter {
                search: Some(q.into()),
                ..Default::default()
            },
            20,
            0,
        )
        .await
        .unwrap();
    }
    for c in catalog::list_channels(&store, &ChannelFilter::default(), 50, 0)
        .await
        .unwrap()
    {
        let plan = catalog::resolve_play_plan(&store, &ApiKeyAuth(&secrets), c.id)
            .await
            .unwrap();
        let _ = plan.redacted();
    }
    let reqs = server.received_requests().await.unwrap();
    assert!(!reqs.is_empty());
    for r in reqs {
        let p = r.url.path();
        assert!(
            !p.starts_with("/proxy/ts/stream")
                && !p.starts_with("/proxy/hls/")
                && !p.starts_with("/proxy/vod/")
                && !p.starts_with("/proxy/catchup/"),
            "stream endpoint requested: {p}"
        );
    }
}

/// Unreadable profiles + a brand-new channel => Unknown (never Proxy); existing flags are kept.
#[tokio::test]
async fn profile_failure_marks_new_channels_unknown() {
    let server = MockServer::start().await;
    mount_all(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/core/streamprofiles/"))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(1)
        .mount(&server)
        .await;
    let (store, sid, client, _) = setup(&server).await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    for ext in ["1", "2", "6"] {
        assert_eq!(
            store
                .channel_by_ext(sid, ext)
                .await
                .unwrap()
                .unwrap()
                .stream_redirect,
            StreamMode::Unknown
        );
    }
}
