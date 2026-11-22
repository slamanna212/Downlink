#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Xtream client against tools/fixtures/xtream (synthetic) with wiremock.

use std::path::PathBuf;

use downlink_core::ErrorCode;
use downlink_core::auth::SecretString;
use downlink_core::redact::redact;
use downlink_core::source::xtream::XtreamClient;
use downlink_core::source::{ChannelKey, Page, Programme, Source, SourceId, VodKind};
use serde_json::Value;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const USER: &str = "testuser";
const PASS: &str = "testpass_secret";

fn fx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/xtream")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn client(server: &MockServer) -> XtreamClient {
    XtreamClient::new(
        SourceId(1),
        &server.uri(),
        USER,
        SecretString::new(PASS),
        false,
    )
    .unwrap()
}

/// Matches requests that carry no `action` parameter (the bare login call).
struct NoAction;

impl wiremock::Match for NoAction {
    fn matches(&self, r: &wiremock::Request) -> bool {
        !r.url.query_pairs().any(|(k, _)| k == "action")
    }
}

async fn action(server: &MockServer, a: Option<&str>, body: Value) {
    let mut m = Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(query_param("username", USER))
        .and(query_param("password", PASS));
    m = match a {
        Some(a) => m.and(query_param("action", a)),
        None => m.and(NoAction),
    };
    m.respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn login_populates_account_info() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    let c = client(&server);
    let info = c.connect().await.unwrap();
    assert_eq!(info.timezone.as_deref(), Some("Europe/London"));
    assert_eq!(
        info.max_connections,
        Some(2),
        "numbers as strings tolerated"
    );
    assert_eq!(info.expires_at, Some(1_798_761_600));
    assert_eq!(info.username.as_deref(), Some("testuser"));
}

#[tokio::test]
async fn auth_zero_with_http_200_is_an_auth_error() {
    let server = MockServer::start().await;
    action(&server, None, fx("error_auth_failed.json")).await;
    let e = client(&server).connect().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth);
    assert!(e.blocking);
}

#[tokio::test]
async fn http_401_is_a_permissions_problem_not_a_wrong_password() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let e = client(&server).connect().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth);
    assert!(e.message.contains("Xtream"), "{}", e.message);
    assert!(!e.message.contains("password"), "{}", e.message);
}

#[tokio::test]
async fn categories_streams_vod_series_and_series_info() {
    let server = MockServer::start().await;
    action(
        &server,
        Some("get_live_categories"),
        fx("get_live_categories.json"),
    )
    .await;
    action(
        &server,
        Some("get_live_streams"),
        fx("get_live_streams.json"),
    )
    .await;
    action(&server, Some("get_vod_streams"), fx("get_vod_streams.json")).await;
    action(&server, Some("get_series"), fx("get_series.json")).await;
    action(&server, Some("get_series_info"), fx("get_series_info.json")).await;
    action(&server, Some("get_vod_categories"), serde_json::json!([])).await;
    action(
        &server,
        Some("get_series_categories"),
        serde_json::json!({}),
    )
    .await;
    let c = client(&server);
    let cats = c.live_categories().await.unwrap();
    assert_eq!(cats.len(), 6);
    assert_eq!(
        (cats[1].id.as_str(), cats[1].name.as_str()),
        ("2", "Test News")
    );
    let live = c.live_streams(None).await.unwrap();
    assert!(live.len() >= 3);
    assert_eq!(live[0].stream_id, "1001");
    assert!(!live[0].tv_archive && live[1].tv_archive && live[1].tv_archive_days == 3);
    let vod = c.vod_streams(None).await.unwrap();
    assert_eq!(vod[0].container_extension.as_deref(), Some("mkv"));
    let series = c.series(None).await.unwrap();
    assert_eq!(series[0].series_id, "3001");
    let info = c.series_info("3001").await.unwrap();
    assert_eq!(info.name.as_deref(), Some("Test Series 1"));
    assert!(!info.episodes.is_empty());
    assert!(
        info.episodes
            .windows(2)
            .all(|w| (w[0].season, w[0].episode) <= (w[1].season, w[1].episode))
    );
    assert!(
        c.vod_category_list().await.unwrap().is_empty(),
        "[] = nothing"
    );
    assert!(
        c.series_categories().await.unwrap().is_empty(),
        "{{}} = nothing"
    );
}

#[tokio::test]
async fn short_and_full_epg_decode_base64_and_use_epoch_times() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    action(&server, Some("get_short_epg"), fx("get_short_epg.json")).await;
    action(
        &server,
        Some("get_simple_data_table"),
        fx("get_short_epg.json"),
    )
    .await;
    let c = client(&server);
    let p = c.short_epg("1001", 4).await.unwrap();
    assert!(p.len() >= 2);
    assert_eq!(p[0].title.as_deref(), Some("Morning Test Show"));
    assert_eq!(p[0].start_utc, 1_767_225_600);
    assert_eq!(p[0].end_utc, 1_767_229_200);
    assert_eq!(p[0].channel_ref, "test-channel-1.test");
    assert!(
        p[0].description
            .as_deref()
            .unwrap()
            .starts_with("Synthetic description")
    );
    assert_eq!(c.full_epg("1001").await.unwrap().len(), p.len());
}

#[tokio::test]
async fn empty_short_epg_is_fine() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    action(
        &server,
        Some("get_short_epg"),
        fx("get_short_epg_empty.json"),
    )
    .await;
    assert!(client(&server).short_epg("1", 4).await.unwrap().is_empty());
}

#[tokio::test]
async fn built_urls_have_the_right_shape_and_all_redact() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    let c = client(&server);
    c.connect().await.unwrap();
    let base = server.uri();
    assert_eq!(
        c.live_url("1001").unwrap(),
        format!("{base}/live/{USER}/{PASS}/1001.ts")
    );
    assert_eq!(
        c.movie_url("2001", "mkv").unwrap(),
        format!("{base}/movie/{USER}/{PASS}/2001.mkv")
    );
    assert_eq!(
        c.series_url("77", "mp4").unwrap(),
        format!("{base}/series/{USER}/{PASS}/77.mp4")
    );
    let xml = c.xmltv_url().unwrap();
    assert!(xml.contains("/xmltv.php?") && xml.contains("username=testuser"));
    let ts = c.timeshift_php_url("1001", "2026-07-01:13-00", 60).unwrap();
    assert!(ts.contains("/streaming/timeshift.php?"));
    assert!(c.playlist_url().unwrap().contains("/get.php?"));
    for u in [
        c.live_url("1").unwrap(),
        c.movie_url("2", "mkv").unwrap(),
        c.series_url("3", "mp4").unwrap(),
        xml,
        ts,
        c.playlist_url().unwrap(),
    ] {
        let r = redact(&u);
        assert!(!r.contains(PASS) && !r.contains(USER), "{u} -> {r}");
    }
}

#[tokio::test]
async fn m3u8_only_accounts_get_hls_urls() {
    let server = MockServer::start().await;
    let mut login = fx("player_api_login.json");
    login["user_info"]["allowed_output_formats"] = serde_json::json!(["m3u8"]);
    action(&server, None, login).await;
    let c = client(&server);
    c.connect().await.unwrap();
    assert!(c.live_url("5").unwrap().ends_with("/5.m3u8"));
    let plan = c
        .play(ChannelKey {
            ext_id: "5".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(plan.surface, downlink_core::source::Surface::Hls);
}

#[tokio::test]
async fn source_channels_groups_and_play_plan() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    action(
        &server,
        Some("get_live_streams"),
        fx("get_live_streams.json"),
    )
    .await;
    action(
        &server,
        Some("get_live_categories"),
        fx("get_live_categories.json"),
    )
    .await;
    let c = client(&server);
    let page = c.channels(Page::default()).await.unwrap();
    assert!(!page.has_next);
    let ch = &page.channels[1];
    assert_eq!(
        (
            ch.ext_id.as_str(),
            ch.tvg_id.as_deref(),
            ch.group_ext_id.as_deref()
        ),
        ("1002", Some("test-channel-2.test"), Some("3"))
    );
    assert!(ch.is_catchup && ch.catchup_days == 3);
    assert!(
        ch.catchup_template
            .as_deref()
            .unwrap()
            .contains("Europe/London"),
        "server timezone carried as data"
    );
    assert!(page.channels[0].catchup_template.is_none());
    assert_eq!(c.groups().await.unwrap()[3].name, "Adult Test");

    let plan = c
        .play(ChannelKey {
            ext_id: "1001".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        plan.url,
        format!("{}/live/{USER}/{PASS}/1001.ts", server.uri())
    );
    assert!(
        plan.headers.is_empty(),
        "credentials travel in the URL path, not headers"
    );
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(!shown.contains(PASS) && !shown.contains(USER), "{shown}");
    assert!(!format!("{plan:?}").contains(PASS));
}

#[tokio::test]
async fn timeshift_plan_renders_the_programme_row_in_the_servers_zone() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await; // timezone Europe/London
    let c = client(&server);
    // 2026-07-01 12:00:00Z .. 13:00:00Z = 13:00 BST on the panel's clock.
    let start = 1_782_907_200;
    let prog = Programme {
        start_utc: start,
        end_utc: start + 3600,
        ..Default::default()
    };
    let plan = c
        .catchup(
            ChannelKey {
                ext_id: "1002".into(),
                ..Default::default()
            },
            &prog,
        )
        .await
        .unwrap();
    assert_eq!(
        plan.url,
        format!(
            "{}/timeshift/{USER}/{PASS}/60/2026-07-01:13-00/1002.ts",
            server.uri()
        )
    );
    assert_eq!(plan.profile, downlink_core::source::PlayProfile::Seekable);
    assert!(
        plan.session.is_none(),
        "template path has no server session"
    );
    assert!(
        !serde_json::to_string(&plan.redacted())
            .unwrap()
            .contains(PASS)
    );
    // A different zone on the channel's template changes the rendering, not the instant.
    let t = downlink_core::catchup::template::CatchupTemplate {
        mode: downlink_core::catchup::template::Mode::Xc,
        source: None,
        time: downlink_core::catchup::template::TimeSemantics::LocalWallclock {
            tz: "America/New_York".into(),
        },
        correction_secs: 0,
        days: 3,
    };
    let plan = c
        .catchup(
            ChannelKey {
                ext_id: "1002".into(),
                catchup_template: Some(t.to_json()),
                ..Default::default()
            },
            &prog,
        )
        .await
        .unwrap();
    assert!(
        plan.url.contains("/2026-07-01:08-00/"),
        "{}",
        plan.url.replace(PASS, "***")
    );
}

#[tokio::test]
async fn guide_feed_downloads_xmltv_php() {
    let server = MockServer::start().await;
    action(&server, None, fx("player_api_login.json")).await;
    Mock::given(method("GET"))
        .and(path("/xmltv.php"))
        .and(query_param("username", USER))
        .and(query_param("password", PASS))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<tv><channel id=\"x.test\"><display-name>X</display-name></channel></tv>",
        ))
        .mount(&server)
        .await;
    let c = client(&server);
    c.connect().await.unwrap();
    let feed = c.guide_feed(None).await.unwrap().unwrap();
    assert_eq!(feed.default_tz.as_deref(), Some("Europe/London"));
    assert_eq!(
        downlink_core::xmltv::parse_channels(feed.file.as_ref().unwrap().path())
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn errors_never_contain_credentials() {
    // 5xx
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string(format!("oops {USER} {PASS}")))
        .mount(&server)
        .await;
    let e = client(&server).connect().await.unwrap_err();
    assert!(
        !format!("{e:?}").contains(PASS) && !format!("{e:?}").contains(USER),
        "{e:?}"
    );
    // connection refused: the URL (with credentials) must not leak into the message
    let dead = XtreamClient::new(
        SourceId(1),
        "http://127.0.0.1:1",
        USER,
        SecretString::new(PASS),
        false,
    )
    .unwrap();
    let e = dead.connect().await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Network);
    assert!(
        !format!("{e:?}").contains(PASS) && !format!("{e:?}").contains(USER),
        "{e:?}"
    );
    // garbage body
    let junk = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&junk)
        .await;
    let e = client(&junk).connect().await.unwrap_err();
    assert!(!format!("{e:?}").contains(PASS));
}

#[tokio::test]
async fn base_url_forms() {
    use downlink_core::source::xtream::normalise_base;
    assert_eq!(
        normalise_base("panel.test:8080").unwrap().as_str(),
        "http://panel.test:8080/"
    );
    assert_eq!(
        normalise_base("https://p.test/player_api.php?username=a&password=b")
            .unwrap()
            .as_str(),
        "https://p.test/"
    );
    assert_eq!(
        normalise_base("http://u:p@p.test/panel/get.php")
            .unwrap()
            .as_str(),
        "http://p.test/panel/"
    );
    assert!(normalise_base("ftp://p.test").is_err());
}

// ---- PLAN §6.8: two panels, one model --------------------------------------------------------

fn fx_in(dir: &str, name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/xtream")
        .join(dir)
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Everything the client normalises, as comparable text.
async fn normalised(dir: &str) -> Vec<String> {
    let server = MockServer::start().await;
    let f = |n: &str| fx_in(dir, n);
    action(&server, None, f("player_api_login.json")).await;
    for (a, file) in [
        ("get_live_categories", "get_live_categories.json"),
        ("get_live_streams", "get_live_streams.json"),
        ("get_vod_streams", "get_vod_streams.json"),
        ("get_series", "get_series.json"),
        ("get_series_info", "get_series_info.json"),
        ("get_short_epg", "get_short_epg.json"),
        ("get_simple_data_table", "get_short_epg.json"),
    ] {
        action(&server, Some(a), f(file)).await;
    }
    action(&server, Some("get_vod_categories"), serde_json::json!([])).await;
    action(
        &server,
        Some("get_series_categories"),
        serde_json::json!([]),
    )
    .await;
    let c = client(&server);
    let acct = c.connect().await.unwrap();
    let page = c.channels(Page::default()).await.unwrap();
    let mut out = vec![
        format!("{acct:?}"),
        format!("{:?}", c.caps()),
        format!("{:?}", c.groups().await.unwrap()),
        format!("{:?}", page.channels),
        format!("{:?}", c.live_categories().await.unwrap()),
        format!("{:?}", c.short_epg("1001", 4).await.unwrap()),
        format!("{:?}", c.full_epg("1001").await.unwrap()),
    ];
    let q = downlink_core::source::VodListQuery {
        limit: 100,
        ..Default::default()
    };
    out.push(format!(
        "{:?}",
        c.vod_list(VodKind::Movie, &q).await.unwrap()
    ));
    out.push(format!(
        "{:?}",
        c.vod_list(VodKind::Series, &q).await.unwrap()
    ));
    out.push(format!("{:?}", c.series_info("3001").await.unwrap()));
    out
}

#[tokio::test]
async fn panel_b_decodes_into_the_same_models_as_the_base_set() {
    let base = normalised("").await;
    let b = normalised("panel-b").await;
    let names = [
        "account",
        "caps",
        "groups",
        "channels",
        "live categories",
        "short epg",
        "full epg",
        "vod",
        "series",
        "series info",
    ];
    for (i, n) in names.iter().enumerate() {
        assert_eq!(base[i], b[i], "{n} differ between panels");
    }
}
