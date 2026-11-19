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

// ---- catch-up flow (PLAN §5.2) --------------------------------------------------------------

mod catchup_flow {
    use super::*;
    use downlink_core::catchup::{self, PositionReporter};
    use downlink_core::sessions::{self, KIND_CATCHUP};
    use wiremock::matchers::body_json;

    const UUID1: &str = "00000000-0000-4000-8000-000000000001";
    const START: i64 = 1_735_725_600; // grid fixture programme 1001: 10:00Z..11:00Z

    async fn synced(server: &MockServer) -> (Store, i64, DispatcharrClient, i64) {
        mount_all(server).await;
        let (store, sid, client, _) = setup(server).await;
        sync::sync_channels(&client, &store, sid, None)
            .await
            .unwrap();
        epg::sync_guide(&client, &store, sid, epg::visible_window(START + 1800))
            .await
            .unwrap();
        let one = store.channel_by_ext(sid, "1").await.unwrap().unwrap().id;
        (store, sid, client, one)
    }

    #[tokio::test]
    async fn create_play_position_delete_using_programme_row_times() {
        let server = MockServer::start().await;
        let (store, _sid, client, one) = synced(&server).await;
        Mock::given(method("POST")).and(path("/api/catchup/sessions/"))
            .and(body_json(serde_json::json!({"channel_uuid": UUID1, "start": "2025-01-01T10:00:00Z", "duration": 60})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "session_id": "sess-1", "playback_url": format!("/proxy/catchup/{UUID1}?session_id=sess-1"),
                "channel_uuid": UUID1
            })))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/catchup/sessions/sess-1/position/"))
            .and(body_json(serde_json::json!({"position_secs": 12})))
            .respond_with(ResponseTemplate::new(204))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/catchup/sessions/sess-1/"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let now = START + 7200;
        let play = catchup::play_catchup(&store, &client, one, START, now)
            .await
            .unwrap();
        assert_eq!((play.start_utc, play.end_utc), (START, START + 3600));
        assert_eq!(
            play.plan.profile,
            downlink_core::source::PlayProfile::Seekable
        );
        assert_eq!(
            play.plan.url,
            format!("{}/proxy/catchup/{UUID1}?session_id=sess-1", server.uri()),
            "relative playback_url joined"
        );
        // Persisted before playback for crash cleanup.
        assert_eq!(store.list_open_sessions().await.unwrap().len(), 1);

        let mut rep = PositionReporter::new(play.session.clone());
        assert!(rep.report(&client, 0, 12, false).await.unwrap());
        assert!(
            !rep.report(&client, 4_000, 16, false).await.unwrap(),
            "throttled"
        );
        assert!(
            rep.report(&client, 5_000, 12, true).await.unwrap(),
            "forced on pause/seek"
        );

        catchup::stop(&store, &client, &play).await.unwrap();
        assert!(store.list_open_sessions().await.unwrap().is_empty());
        let _ = (sessions::KIND_HLS, KIND_CATCHUP);
    }

    #[tokio::test]
    async fn restart_programme_starts_from_the_airing_programmes_row() {
        let server = MockServer::start().await;
        let (store, _sid, client, one) = synced(&server).await;
        Mock::given(method("POST")).and(path("/api/catchup/sessions/"))
            .and(body_json(serde_json::json!({"channel_uuid": UUID1, "start": "2025-01-01T10:00:00Z", "duration": 60})))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({"session_id": "s2", "playback_url": "/p?s=2"})))
            .expect(1).mount(&server).await;
        // 10:30: "Morning Show" (10:00-11:00) is airing; restart goes to its start.
        let play = catchup::restart_programme(&store, &client, one, START + 1800)
            .await
            .unwrap();
        assert_eq!(play.start_utc, START);
        // Nothing airing -> plain error, no request.
        let e = catchup::restart_programme(&store, &client, one, START + 10 * 86_400)
            .await
            .unwrap_err();
        assert_eq!(e.code, downlink_core::ErrorCode::Stream);
    }

    #[tokio::test]
    async fn refusals_make_no_requests() {
        let server = MockServer::start().await;
        let (store, sid, client, one) = synced(&server).await;
        let before = server.received_requests().await.unwrap().len();
        // Future programme.
        let e = catchup::play_catchup(&store, &client, one, START, START - 10)
            .await
            .unwrap_err();
        assert_eq!(e.code, downlink_core::ErrorCode::Stream);
        // Channel without an archive (channel 2 has catchup_days 0).
        let two = store.channel_by_ext(sid, "2").await.unwrap().unwrap().id;
        let e = catchup::play_catchup(&store, &client, two, START, START + 9999)
            .await
            .unwrap_err();
        assert!(e.message.contains("archive"));
        // Unknown programme.
        assert!(
            catchup::play_catchup(&store, &client, one, 12345, START + 9999)
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), before);
        assert!(store.list_open_sessions().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn server_refusal_is_a_plain_sentence_and_leaves_no_session() {
        let server = MockServer::start().await;
        let (store, _sid, client, one) = synced(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/catchup/sessions/"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_string("{\"error\":\"Catch-up not supported for this channel\"}"),
            )
            .mount(&server)
            .await;
        let e = catchup::play_catchup(&store, &client, one, START, START + 9999)
            .await
            .unwrap_err();
        assert_eq!(e.code, downlink_core::ErrorCode::Stream);
        assert!(
            !e.message.contains("Catch-up not supported"),
            "raw body never shown"
        );
        assert!(store.list_open_sessions().await.unwrap().is_empty());
    }
}

// ---- refresh policy runner ------------------------------------------------------------------

#[tokio::test]
async fn refresh_plan_runner_fills_visible_then_history_and_is_cancellable() {
    use downlink_core::epg::refresh::{self, WindowKind};
    use std::sync::atomic::{AtomicBool, Ordering};
    let server = MockServer::start().await;
    mount_all(&server).await;
    let (store, sid, client, _) = setup(&server).await;
    sync::sync_channels(&client, &store, sid, None)
        .await
        .unwrap();
    let now = 1_735_725_600 + 1800;
    let days = refresh::max_catchup_days(&store, sid).await.unwrap();
    assert_eq!(days, 7);
    let plan = refresh::plan_windows(now, days, refresh::parse_tz(Some("Europe/Berlin")));
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let r = refresh::run_plan(
        &client,
        &store,
        sid,
        &plan,
        &[WindowKind::Visible, WindowKind::CatchupHistory],
        &cancel,
        |d, t| seen.push((d, t)),
    )
    .await
    .unwrap();
    assert!(r.windows_total >= 2 && r.windows_done == r.windows_total && !r.cancelled);
    assert_eq!(
        seen.last().copied(),
        Some((r.windows_done, r.windows_total))
    );
    // Cancel before the first window.
    cancel.store(true, Ordering::Relaxed);
    let r = refresh::run_plan(
        &client,
        &store,
        sid,
        &plan,
        &[WindowKind::Ahead],
        &cancel,
        |_, _| {},
    )
    .await
    .unwrap();
    assert!(r.cancelled && r.windows_done == 0);
}
