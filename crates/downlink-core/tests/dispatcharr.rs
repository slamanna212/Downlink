#![allow(clippy::unwrap_used, clippy::expect_used)]
//! One wiremock integration test per Dispatcharr endpoint used. Fixtures are synthetic.

use downlink_core::auth::SecretString;
use downlink_core::source::dispatcharr::{ClientOptions, Credentials, DispatcharrClient};
use downlink_core::source::{
    Caps, ChannelKey, Page, Programme, SessionId, Source, SourceId, StreamMode, TimeWindow,
    all_channels, resolve_redirect,
};
use downlink_core::{AppError, ErrorCode};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_synthetic_key_123456";
const UUID1: &str = "00000000-0000-4000-8000-000000000001";
const UUID2: &str = "00000000-0000-4000-8000-000000000002";

macro_rules! fixture {
    ($name:literal) => {
        serde_json::from_str::<serde_json::Value>(include_str!(concat!(
            "fixtures/dispatcharr/",
            $name
        )))
        .unwrap()
    };
}

fn json(v: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(v)
}

fn client(server: &MockServer) -> DispatcharrClient {
    DispatcharrClient::new(
        SourceId(1),
        &server.uri(),
        Credentials::ApiKey(SecretString::new(KEY)),
        ClientOptions::default(),
    )
    .unwrap()
}

async fn get_mock(server: &MockServer, p: &str, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .and(header("X-API-Key", KEY))
        .respond_with(json(body))
        .mount(server)
        .await;
}

async fn mount_connect(server: &MockServer) {
    get_mock(server, "/api/core/version/", fixture!("version.json")).await;
    get_mock(server, "/api/accounts/users/me/", fixture!("me.json")).await;
    get_mock(server, "/api/core/settings/", fixture!("settings.json")).await;
}

#[tokio::test]
async fn endpoint_version_me_settings_connect() {
    let server = MockServer::start().await;
    mount_connect(&server).await;
    let info = client(&server).connect().await.unwrap();
    assert_eq!(info.server_version.as_deref(), Some("0.32.0"));
    assert_eq!(info.username.as_deref(), Some("alice"));
    assert_eq!(
        info.user_level,
        Some(downlink_core::source::UserLevel::Admin)
    );
    assert_eq!(info.timezone.as_deref(), Some("Europe/Berlin"));
    assert_eq!(
        client(&server).caps(),
        Caps::LIVE | Caps::EPG | Caps::CATCHUP | Caps::FAVOURITES | Caps::FAILOVER | Caps::PROFILES
    );
}

#[tokio::test]
async fn old_server_version_is_a_blocking_server_error() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/core/version/",
        serde_json::json!({"version": "0.30.2"}),
    )
    .await;
    let e = client(&server).connect().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Server);
    assert!(e.blocking);
    assert!(e.message.contains("0.30.2"));
}

#[tokio::test]
async fn bad_api_key_is_a_blocking_auth_error_and_leaks_nothing() {
    let server = MockServer::start().await;
    get_mock(&server, "/api/core/version/", fixture!("version.json")).await;
    Mock::given(method("GET"))
        .and(path("/api/accounts/users/me/"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .mount(&server)
        .await;
    let e: AppError = client(&server).connect().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth);
    assert!(e.blocking);
    assert!(!format!("{e:?}").contains(KEY));
}

#[tokio::test]
async fn settings_failure_is_tolerated() {
    let server = MockServer::start().await;
    get_mock(&server, "/api/core/version/", fixture!("version.json")).await;
    get_mock(&server, "/api/accounts/users/me/", fixture!("me.json")).await;
    Mock::given(method("GET"))
        .and(path("/api/core/settings/"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let info = client(&server).connect().await.unwrap();
    assert!(info.timezone.is_none());
}

#[tokio::test]
async fn endpoint_channels_paginates_without_trusting_page_size() {
    let server = MockServer::start().await;
    for (n, f) in [
        ("1", fixture!("channels_page1.json")),
        ("2", fixture!("channels_page2.json")),
        ("3", fixture!("channels_page3.json")),
    ] {
        Mock::given(method("GET"))
            .and(path("/api/channels/channels/"))
            .and(query_param("page", n))
            .and(query_param("page_size", "500"))
            .and(header("X-API-Key", KEY))
            .respond_with(json(f))
            .mount(&server)
            .await;
    }
    // Server returned 2 per page although we asked for 500.
    let (chs, count) = all_channels(&client(&server), 500).await.unwrap();
    assert_eq!(chs.len(), 6);
    assert_eq!(count, Some(5));
    assert_eq!(chs[0].name, "Channel One HD");
    assert_eq!(chs[0].streams, ["11", "12"], "failover order preserved");
    assert_eq!(chs[1].stream_profile_id, Some(4));
    assert_eq!(chs[5].number, Some(6.0), "string channel_number tolerated");
    assert!(chs[2].is_adult && chs[3].is_radio && chs[4].hidden);
}

#[tokio::test]
async fn channels_accepts_unpaginated_array() {
    let server = MockServer::start().await;
    let arr = fixture!("channels_page1.json")["results"].clone();
    get_mock(&server, "/api/channels/channels/", arr).await;
    let p = client(&server).channels(Page::default()).await.unwrap();
    assert_eq!(p.channels.len(), 2);
    assert!(!p.has_next);
}

#[tokio::test]
async fn channels_search_and_group_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .and(query_param("search", "one"))
        .and(query_param("channel_group", "1"))
        .respond_with(json(fixture!("channels_page3.json")))
        .mount(&server)
        .await;
    let p = client(&server)
        .channel_page(Page::default(), Some("one"), Some("1"))
        .await
        .unwrap();
    assert_eq!(p.channels.len(), 2);
}

#[tokio::test]
async fn endpoint_groups() {
    let server = MockServer::start().await;
    get_mock(&server, "/api/channels/groups/", fixture!("groups.json")).await;
    let g = client(&server).groups().await.unwrap();
    assert_eq!(g.len(), 2);
    assert_eq!((g[0].ext_id.as_str(), g[0].name.as_str()), ("1", "News"));
}

#[tokio::test]
async fn endpoint_channel_profiles() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/channels/profiles/",
        fixture!("channel_profiles.json"),
    )
    .await;
    let p = client(&server).channel_profiles().await.unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].name, "All");
}

#[tokio::test]
async fn endpoint_channel_streams_keeps_order_and_drops_provider_urls() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    let s = client(&server).channel_streams(1).await.unwrap();
    assert_eq!(
        s.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(),
        ["12", "11"]
    );
    assert!(!format!("{s:?}").contains("provider.test"));
}

#[tokio::test]
async fn endpoint_channel_stream_info_names_providers_and_drops_their_credentials() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    get_mock(&server, "/api/m3u/accounts/", fixture!("m3u_accounts.json")).await;
    let s = client(&server).channel_stream_info(1).await.unwrap();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].id, "12");
    assert_eq!(s[0].provider.as_deref(), Some("Provider Alpha"));
    assert_eq!(s[0].resolution.as_deref(), Some("1920x1080"));
    assert_eq!(s[0].video_codec.as_deref(), Some("h264"));
    assert_eq!(s[0].audio_channels.as_deref(), Some("stereo"));
    assert_eq!(s[0].viewers, Some(2));
    assert!(s[0].bitrate_kbps.is_some_and(|b| b > 6000.0));
    assert_eq!(s[1].provider.as_deref(), Some("Provider Beta"));
    assert_eq!(s[1].resolution, None, "missing stats stay empty");
    let dump = format!("{s:?}");
    assert!(!dump.contains("provider.test") && !dump.contains("somepass"));
}

#[tokio::test]
async fn channel_stream_info_without_account_access_still_lists_streams() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/m3u/accounts/"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let s = c.channel_stream_info(1).await.unwrap();
    assert_eq!(s.len(), 2);
    assert!(s.iter().all(|x| x.provider.is_none()));
    // The refusal is cached: a second open does not ask again.
    c.channel_stream_info(1).await.unwrap();
}

#[tokio::test]
async fn endpoint_stream_profiles_and_redirect_detection() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        "/api/core/streamprofiles/",
        fixture!("stream_profiles.json"),
    )
    .await;
    get_mock(&server, "/api/core/settings/", fixture!("settings.json")).await;
    let info = client(&server).profile_info().await.unwrap();
    assert_eq!(info.default_id, Some(3));
    assert!(info.redirect_ids.contains(&4) && !info.redirect_ids.contains(&3));
    assert_eq!(info.mode(Some(4)), StreamMode::Redirect);
    assert_eq!(
        info.mode(Some(999)),
        StreamMode::Unknown,
        "unlisted profile is never assumed Proxy"
    );
    assert_eq!(
        info.mode(None),
        StreamMode::Proxy,
        "null profile falls back to default (Proxy)"
    );
}

#[tokio::test]
async fn endpoint_epg_grid() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/epg/grid/"))
        .and(query_param("start", "2025-01-01T10:00:00Z"))
        .and(query_param("end", "2025-01-01T14:00:00Z"))
        .and(header("X-API-Key", KEY))
        .respond_with(json(fixture!("grid.json")))
        .mount(&server)
        .await;
    let w = TimeWindow {
        start_utc: 1_735_725_600,
        end_utc: 1_735_739_999 + 1,
    };
    let p = client(&server).guide(w).await.unwrap();
    assert_eq!(p.len(), 3, "unreadable row skipped");
    assert_eq!(p[0].start_utc, 1_735_725_600);
    assert_eq!(
        p[0].ext_id.as_deref(),
        Some("1001"),
        "integer ids decode to String"
    );
    assert_eq!(p[1].ext_id.as_deref(), Some("dummy-77"), "ids are strings");
    assert_eq!(p[1].end_utc, 1_735_731_000 + 3600);
}

#[tokio::test]
async fn endpoint_current_programs() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/epg/current-programs/"))
        .and(header("X-API-Key", KEY))
        .and(body_json(
            serde_json::json!({"channel_uuids": [UUID1, UUID2]}),
        ))
        .respond_with(json(fixture!("current_programs.json")))
        .mount(&server)
        .await;
    let r = client(&server)
        .current_programmes(&[UUID1.into(), UUID2.into()])
        .await
        .unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, UUID1);
    assert_eq!(r[1].1.title.as_deref(), Some("Channel Two Film"));
}

#[tokio::test]
async fn endpoint_logo_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/channels/logos/100/cache"))
        .and(header("X-API-Key", KEY))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(vec![0x89, b'P', b'N', b'G']),
        )
        .mount(&server)
        .await;
    let l = client(&server).logo("100").await.unwrap();
    assert_eq!(l.bytes, [0x89, b'P', b'N', b'G']);
    assert_eq!(l.content_type.as_deref(), Some("image/png"));
}

fn programme() -> Programme {
    Programme {
        channel_ref: "one.test".into(),
        start_utc: 1_735_725_600,
        end_utc: 1_735_725_600 + 3600 + 30,
        ..Default::default()
    }
}

#[tokio::test]
async fn endpoint_catchup_create_position_delete() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/"))
        .and(header("X-API-Key", KEY))
        .and(body_json(serde_json::json!({
            "channel_uuid": UUID1, "start": "2025-01-01T10:00:00Z", "duration": 61
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(fixture!("catchup_session.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/sess-abc-123/position/"))
        .and(body_json(serde_json::json!({"position_secs": 42})))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/catchup/sessions/sess-abc-123/"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let c = client(&server);
    let key = ChannelKey {
        ext_id: "1".into(),
        uuid: Some(UUID1.into()),
        stream_mode: StreamMode::Proxy,
        ..Default::default()
    };
    let plan = c.catchup(key, &programme()).await.unwrap();
    assert_eq!(plan.profile, downlink_core::source::PlayProfile::Seekable);
    let sid = plan.session.clone().unwrap();
    assert_eq!(sid.0, "sess-abc-123");
    assert!(plan.url.starts_with(&server.uri()));
    let shown = format!("{:?}", plan.redacted());
    assert!(
        !shown.contains("sess-abc-123") && !shown.contains("SYNTHETICSIG") && !shown.contains(KEY),
        "{shown}"
    );
    c.catchup_position(&sid, 42).await.unwrap();
    c.stop_session(sid.clone()).await.unwrap();
}

#[tokio::test]
async fn catchup_position_404_before_playback_is_not_fatal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/s1/position/"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("{\"error\":\"No active playback for this session\"}"),
        )
        .mount(&server)
        .await;
    client(&server)
        .catchup_position(&SessionId("s1".into()), 0)
        .await
        .unwrap();
}

#[tokio::test]
async fn endpoint_ts_status_takes_uuid_and_masks_provider_url() {
    let server = MockServer::start().await;
    get_mock(
        &server,
        &format!("/proxy/ts/status/{UUID1}"),
        fixture!("ts_status.json"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/proxy/ts/status/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server);
    let v = c.ts_status(UUID1).await.unwrap().unwrap();
    assert!(!v.to_string().contains("provider.test") && !v.to_string().contains("somepass"));
    assert_eq!(v["state"], "active");
    assert!(c.ts_status("missing").await.unwrap().is_none());
}

#[tokio::test]
async fn app_data_key_length_is_validated() {
    let server = MockServer::start().await;
    assert!(client(&server).app_data_get("short").await.is_err());
}

#[tokio::test]
async fn catchup_4xx_becomes_a_plain_stream_sentence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/"))
        .respond_with(ResponseTemplate::new(400).set_body_string("{\"detail\":\"outside window\"}"))
        .mount(&server)
        .await;
    let key = ChannelKey {
        ext_id: "1".into(),
        uuid: Some(UUID1.into()),
        stream_mode: StreamMode::Proxy,
        ..Default::default()
    };
    let e = client(&server)
        .catchup(key, &programme())
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Stream);
    assert!(!e.blocking);
}

#[tokio::test]
async fn delete_catchup_404_is_ok() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/catchup/sessions/gone/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    client(&server)
        .stop_session(SessionId("gone".into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn endpoint_app_data_get_put_and_missing() {
    let server = MockServer::start().await;
    let key = "com.downlink.state.v1"; // 21 chars: within 8-64
    get_mock(
        &server,
        &format!("/api/accounts/users/me/app-data/{key}/"),
        fixture!("app_data.json"),
    )
    .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/accounts/users/me/app-data/{key}/")))
        .and(body_json(fixture!("app_data.json")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/accounts/users/me/app-data/other-key/"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server);
    let v = c.app_data_get(key).await.unwrap().unwrap();
    assert_eq!(v["v"], 1);
    c.app_data_put(key, &v).await.unwrap();
    assert!(c.app_data_get("other-key").await.unwrap().is_none());
}

#[tokio::test]
async fn jwt_login_and_refresh_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/"))
        .and(body_json(
            serde_json::json!({"username": "alice", "password": "pw-synthetic"}),
        ))
        .respond_with(json(fixture!("token.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/refresh/"))
        .and(body_json(
            serde_json::json!({"refresh": "REFRESH.TOKEN.ONE"}),
        ))
        .respond_with(json(fixture!("token_refreshed.json")))
        .expect(1)
        .mount(&server)
        .await;
    // Old access token is rejected; the refreshed one works.
    Mock::given(method("GET"))
        .and(path("/api/channels/groups/"))
        .and(header("Authorization", "Bearer ACCESS.TOKEN.ONE"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/channels/groups/"))
        .and(header("Authorization", "Bearer ACCESS.TOKEN.TWO"))
        .respond_with(json(fixture!("groups.json")))
        .mount(&server)
        .await;
    let c = DispatcharrClient::new(
        SourceId(1),
        &server.uri(),
        Credentials::Password {
            username: "alice".into(),
            password: SecretString::new("pw-synthetic"),
        },
        ClientOptions::default(),
    )
    .unwrap();
    assert_eq!(c.groups().await.unwrap().len(), 2);
    // Tokens are in the redaction set.
    let redacted = c
        .redaction_set()
        .redact("Authorization: Bearer ACCESS.TOKEN.TWO tok=REFRESH.TOKEN.ONE");
    assert!(
        !redacted.contains("ACCESS.TOKEN") && !redacted.contains("REFRESH.TOKEN"),
        "{redacted}"
    );
}

/// Synthetic JWTs (header and signature are junk; only `iat`/`exp` are read): 60 s and 30 min lifetimes.
const JWT_EXPIRED: &str = "e30.eyJpYXQiOjEsImV4cCI6NjF9.sig-one";
const JWT_FRESH: &str = "e30.eyJpYXQiOjAsImV4cCI6MTgwMH0.sig-two";

fn password_client(server: &MockServer) -> DispatcharrClient {
    DispatcharrClient::new(
        SourceId(1),
        &server.uri(),
        Credentials::Password {
            username: "alice".into(),
            password: SecretString::new("pw-synthetic"),
        },
        ClientOptions::default(),
    )
    .unwrap()
}

/// Play plans take the header without an API call in between, so an access token that is about to
/// expire must be refreshed up front (a stale one is rejected even by open stream endpoints).
#[tokio::test]
async fn jwt_near_expiry_is_refreshed_before_it_is_handed_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/"))
        .respond_with(json(
            serde_json::json!({"access": JWT_EXPIRED, "refresh": "REFRESH.TOKEN.ONE"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/refresh/"))
        .and(body_json(
            serde_json::json!({"refresh": "REFRESH.TOKEN.ONE"}),
        ))
        .respond_with(json(serde_json::json!({"access": JWT_FRESH})))
        .expect(1)
        .mount(&server)
        .await;
    let c = password_client(&server);
    // The login's token is handed out as received (one refresh per call at most)...
    let (name, first) = c.auth_header().await.unwrap();
    assert_eq!(name, "Authorization");
    assert_eq!(first.expose(), format!("Bearer {JWT_EXPIRED}"));
    // ...and a cached one that is about to expire is refreshed before reuse; then kept.
    for _ in 0..2 {
        let (_, v) = c.auth_header().await.unwrap();
        assert_eq!(v.expose(), format!("Bearer {JWT_FRESH}"));
    }
}

#[tokio::test]
async fn jwt_with_time_left_is_not_refreshed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/"))
        .respond_with(json(
            serde_json::json!({"access": JWT_FRESH, "refresh": "REFRESH.TOKEN.ONE"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/refresh/"))
        .respond_with(json(serde_json::json!({"access": JWT_EXPIRED})))
        .expect(0)
        .mount(&server)
        .await;
    let c = password_client(&server);
    for _ in 0..2 {
        let (_, v) = c.auth_header().await.unwrap();
        assert_eq!(v.expose(), format!("Bearer {JWT_FRESH}"));
    }
}

#[tokio::test]
async fn jwt_bad_password_is_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/accounts/token/"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let c = DispatcharrClient::new(
        SourceId(1),
        &server.uri(),
        Credentials::Password {
            username: "alice".into(),
            password: SecretString::new("nope"),
        },
        ClientOptions::default(),
    )
    .unwrap();
    let e = c.groups().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth);
}

#[tokio::test]
async fn rate_limit_carries_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/channels/groups/"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "17"))
        .mount(&server)
        .await;
    let e = client(&server).groups().await.unwrap_err();
    assert_eq!(e.retry_after, Some(17));
}

// ---- stream-endpoint discipline ---------------------------------------------------------------

#[tokio::test]
async fn building_plans_makes_no_requests_at_all() {
    let server = MockServer::start().await;
    let c = client(&server);
    let key = ChannelKey {
        ext_id: "1".into(),
        uuid: Some(UUID1.into()),
        stream_mode: StreamMode::Proxy,
        ..Default::default()
    };
    let plan = c.live_plan(&key).await.unwrap();
    assert_eq!(
        plan.url,
        format!("{}/proxy/ts/stream/{UUID1}", server.uri())
    );
    assert_eq!(plan.headers.len(), 1);
    assert_eq!(plan.headers[0].0, "X-API-Key");
    assert!(!plan.needs_redirect_resolve);
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(!format!("{plan:?}").contains(KEY));
}

#[tokio::test]
async fn proxy_channel_play_goes_straight_to_the_player_with_the_header() {
    let server = MockServer::start().await;
    let key = ChannelKey {
        ext_id: "1".into(),
        uuid: Some(UUID1.into()),
        stream_mode: StreamMode::Proxy,
        ..Default::default()
    };
    let plan = client(&server).play(key).await.unwrap();
    assert!(plan.url.contains("/proxy/ts/stream/"));
    assert_eq!(plan.headers[0].1.expose(), KEY);
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "no preflight, no HEAD"
    );
}

#[tokio::test]
async fn redirect_channel_resolves_location_without_following_and_drops_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID2}")))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            "http://provider.test/live/someuser/somepass/13.ts",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let key = ChannelKey {
        ext_id: "2".into(),
        uuid: Some(UUID2.into()),
        stream_mode: StreamMode::Redirect,
        ..Default::default()
    };
    let plan = client(&server).play(key).await.unwrap();
    assert_eq!(
        plan.url,
        "http://provider.test/live/someuser/somepass/13.ts"
    );
    assert!(
        plan.headers.is_empty(),
        "the key must never reach the provider"
    );
    assert!(!plan.needs_redirect_resolve);
    let shown = format!("{:?}", plan.redacted());
    assert!(
        !shown.contains("someuser")
            && !shown.contains("somepass")
            && !shown.contains("provider.test"),
        "{shown}"
    );
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        reqs.len(),
        1,
        "provider was never contacted; redirect not followed"
    );
}

#[tokio::test]
async fn resolve_redirect_function_variants() {
    let server = MockServer::start().await;
    let url = format!("{}/proxy/ts/stream/x", server.uri());
    let h = vec![("X-API-Key".to_owned(), SecretString::new(KEY))];

    Mock::given(method("GET"))
        .and(path("/proxy/ts/stream/x"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/relative/out.ts"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let loc = resolve_redirect(&url, &h, None).await.unwrap();
    assert_eq!(loc.expose(), format!("{}/relative/out.ts", server.uri()));
    assert_eq!(format!("{loc:?}"), "Location(<redirect-url>)");

    Mock::given(method("GET"))
        .and(path("/proxy/ts/stream/x"))
        .respond_with(ResponseTemplate::new(200))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert_eq!(
        resolve_redirect(&url, &h, None).await.unwrap_err().code,
        ErrorCode::Stream
    );

    Mock::given(method("GET"))
        .and(path("/proxy/ts/stream/x"))
        .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "5"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let e = resolve_redirect(&url, &h, None).await.unwrap_err();
    assert_eq!((e.code, e.retry_after), (ErrorCode::Stream, Some(5)));

    Mock::given(method("GET"))
        .and(path("/proxy/ts/stream/x"))
        .respond_with(ResponseTemplate::new(401))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert_eq!(
        resolve_redirect(&url, &h, None).await.unwrap_err().code,
        ErrorCode::Auth
    );
}

#[tokio::test]
async fn unknown_profile_302_hands_location_without_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID2}")))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://provider.test/x/1.ts"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let key = ChannelKey {
        ext_id: "2".into(),
        uuid: Some(UUID2.into()),
        stream_mode: StreamMode::Unknown,
        ..Default::default()
    };
    let plan = client(&server).play(key).await.unwrap();
    assert_eq!(plan.url, "http://provider.test/x/1.ts");
    assert!(plan.headers.is_empty() && !plan.needs_redirect_resolve);
}

#[tokio::test]
async fn unknown_profile_200_falls_back_to_original_url_with_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID2}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 16]))
        .expect(1)
        .mount(&server)
        .await;
    let key = ChannelKey {
        ext_id: "2".into(),
        uuid: Some(UUID2.into()),
        stream_mode: StreamMode::Unknown,
        ..Default::default()
    };
    let plan = client(&server).play(key).await.unwrap();
    assert_eq!(
        plan.url,
        format!("{}/proxy/ts/stream/{UUID2}", server.uri())
    );
    assert_eq!(
        plan.headers[0].1.expose(),
        KEY,
        "header kept: it is our own server"
    );
    assert!(!plan.needs_redirect_resolve && !plan.unknown_profile);
}

#[test]
fn unknown_is_never_defaulted_to_proxy() {
    assert_eq!(StreamMode::default(), StreamMode::Unknown);
    let info = downlink_core::source::ProfileInfo::default();
    assert_eq!(info.mode(None), StreamMode::Unknown);
}

#[tokio::test]
async fn fetch_image_is_same_origin_only() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/channels/logos/100/cache"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8, 2, 3]))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let url = c.logo_url("100").unwrap();
    assert_eq!(c.fetch_image(&url).await.unwrap().bytes, [1, 2, 3]);
    // Another host (a poster CDN) is fetched anonymously: the key never goes elsewhere.
    let cdn = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/poster.jpg"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9u8, 9]))
        .expect(1)
        .mount(&cdn)
        .await;
    let got = c
        .fetch_image(&format!("{}/poster.jpg", cdn.uri()))
        .await
        .unwrap();
    assert_eq!(got.bytes, [9, 9]);
    let req = &cdn.received_requests().await.unwrap()[0];
    assert!(
        req.headers.get("x-api-key").is_none(),
        "no credentials sent to another origin"
    );
    assert!(req.headers.get("authorization").is_none());
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "nothing else hit the server"
    );
}

#[tokio::test]
async fn endpoint_programme_search() {
    use downlink_core::source::ProgramSearch;
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "count": 2, "next": "http://dispatcharr.test/x?page=2", "previous": null,
        "results": [
            {"id": 222, "title": "Colour Bars Live", "sub_title": "Episode 6", "description": "Synthetic.",
             "start_time": "2025-01-01T08:00:00Z", "end_time": "2025-01-01T09:00:00Z", "tvg_id": "one.test",
             "custom_properties": {"new": true, "season": 1, "episode": 6},
             "channels": [{"id": 7, "name": "Channel One"}, {"id": 8}], "streams": []},
            {"id": "x", "title": "Bad", "start_time": "nope", "end_time": "nope", "channels": []}
        ]
    });
    Mock::given(method("GET"))
        .and(path("/api/epg/programs/search/"))
        .and(query_param("title", "colour"))
        .and(query_param("start_after", "2025-01-01T00:00:00Z"))
        .and(query_param("page_size", "500"))
        .and(header("X-API-Key", KEY))
        .respond_with(json(body))
        .mount(&server)
        .await;
    let q = ProgramSearch {
        title: Some("colour".into()),
        start_after: Some(1_735_689_600),
        page: 1,
        page_size: 9999,
        ..Default::default()
    };
    let (hits, more) = client(&server).search_programs(&q).await.unwrap();
    assert!(more);
    assert_eq!(hits.len(), 1, "unreadable row skipped");
    assert_eq!(hits[0].channel_ext_ids, ["7", "8"]);
    let p = &hits[0].programme;
    assert_eq!(
        (p.season, p.episode, p.flags),
        (Some(1), Some(6), downlink_core::source::flags::NEW)
    );
}

#[tokio::test]
async fn endpoint_hls_session_delete_and_crash_cleanup_end_to_end() {
    use downlink_core::sessions::{self, KIND_CATCHUP, KIND_HLS};
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/proxy/hls/sessions/tok1/"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/catchup/sessions/sessA/"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/proxy/hls/sessions/gone/"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let store = downlink_core::store::Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&downlink_core::store::NewSource {
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
    for (k, id) in [
        (KIND_HLS, "tok1"),
        (KIND_CATCHUP, "sessA"),
        (KIND_HLS, "gone"),
    ] {
        sessions::track(&store, src.id, k, id).await.unwrap();
    }
    // "next start": everything persisted is deleted; an already-gone session counts as stopped.
    let r = sessions::cleanup_on_startup(&store, &client(&server))
        .await
        .unwrap();
    assert_eq!((r.stopped, r.failed), (3, 0));
    assert!(store.list_open_sessions().await.unwrap().is_empty());
}
