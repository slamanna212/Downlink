#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Façade tests: the command-level verbs against a mock Dispatcharr, DTO shapes as the UI expects.

use std::io::Cursor;
use std::sync::Arc;

use downlink_core::auth::{MemorySecretStore, SecretStore};
use downlink_core::service::{AddSourceInput, Event, GuideArgs, ListChannelsArgs, Service};
use downlink_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_service_key_abcdef123";
const UUID1: &str = "00000000-0000-4000-8000-000000000001";
const UUID2: &str = "00000000-0000-4000-8000-000000000002";
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
    secrets: Arc<MemorySecretStore>,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let store = Store::open_memory().await.unwrap();
    let svc = Service::with_store(
        store,
        secrets.clone(),
        dir.path().join("img"),
        "0.0.0-test".into(),
    );
    Env {
        svc,
        secrets,
        _dir: dir,
    }
}

async fn added() -> (MockServer, Env, i64) {
    let server = MockServer::start().await;
    mount_server(&server).await;
    let e = env().await;
    let s = e.svc.add_source(input(&server)).await.unwrap();
    (server, e, s.id)
}

#[tokio::test]
async fn add_source_persists_reference_not_secret_and_emits_events() {
    let server = MockServer::start().await;
    mount_server(&server).await;
    let e = env().await;
    let mut rx = e.svc.subscribe();
    let dto = e.svc.add_source(input(&server)).await.unwrap();

    let v = serde_json::to_value(&dto).unwrap();
    for k in [
        "id",
        "uuid",
        "kind",
        "name",
        "host",
        "caps",
        "status",
        "serverVersion",
        "lastSyncAt",
    ] {
        assert!(v.get(k).is_some(), "missing {k} in {v}");
    }
    assert_eq!(v["status"], "connected");
    assert_eq!(
        v["caps"],
        1 | 2 | 4 | 32,
        "LIVE|EPG|CATCHUP|FAVOURITES in the UI's bit layout"
    );
    assert_eq!(v["host"], format!("127.0.0.1:{}", server.address().port()));
    assert!(!v.to_string().contains(KEY));

    let row = e.svc.store().source(dto.id).await.unwrap();
    let r = row.secret_ref.unwrap();
    assert_eq!(e.secrets.get(&r).unwrap().unwrap().expose(), KEY);
    assert!(!r.contains(KEY));

    let mut names = vec![];
    let mut updated = None;
    while let Ok(ev) = rx.try_recv() {
        names.push(ev.name());
        if let Event::CatalogUpdated(c) = ev {
            updated = Some(c);
        }
    }
    assert!(names.contains(&"task://progress") && names.contains(&"catalog://updated"));
    let u = updated.unwrap();
    assert_eq!((u.channels, u.groups), (5, 2));
    assert!(u.programmes >= 1);
    let ev = serde_json::to_value(Event::CatalogUpdated(u)).unwrap();
    assert!(ev.get("sourceId").is_some());
}

#[tokio::test]
async fn bad_credentials_persist_nothing() {
    let server = MockServer::start().await;
    get(&server, "/api/core/version/", fixture!("version.json")).await;
    Mock::given(method("GET"))
        .and(path("/api/accounts/users/me/"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let e = env().await;
    let err = e.svc.add_source(input(&server)).await.unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Auth);
    assert!(e.svc.list_sources().await.unwrap().is_empty());
    // unsupported kinds and missing credentials are plain local errors
    let mut bad = input(&server);
    bad.kind = "xtream".into();
    assert!(e.svc.add_source(bad).await.is_err());
    let mut nocreds = input(&server);
    nocreds.api_key = None;
    assert!(e.svc.add_source(nocreds).await.is_err());
}

#[tokio::test]
async fn test_source_counts_without_saving() {
    let server = MockServer::start().await;
    mount_server(&server).await;
    let e = env().await;
    let r = e.svc.test_source(input(&server)).await.unwrap();
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(
        (v["channels"].as_i64(), v["groups"].as_i64()),
        (Some(5), Some(2)),
        "hidden excluded"
    );
    assert_eq!(v["serverVersion"], "0.32.0");
    assert_eq!(v["user"], "alice");
    assert!(e.svc.list_sources().await.unwrap().is_empty());
}

#[tokio::test]
async fn channels_groups_search_guide_and_favourites_dtos() {
    let (_server, e, _sid) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let names: Vec<&str> = chans.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["Channel One", "Channel Two", "Channel Six"],
        "adult/radio/hidden not listed"
    );
    let v = serde_json::to_value(&chans[0]).unwrap();
    for k in [
        "id",
        "sourceId",
        "uuid",
        "number",
        "name",
        "groupId",
        "logo",
        "quality",
        "isCatchup",
        "catchupDays",
        "isRadio",
        "hasDrm",
        "isFavourite",
        "epgMatch",
    ] {
        assert!(v.get(k).is_some(), "missing {k}");
    }
    assert_eq!(
        (
            v["quality"].as_str(),
            v["isCatchup"].as_bool(),
            v["catchupDays"].as_i64()
        ),
        (Some("HD"), Some(true), Some(7))
    );
    assert!(v["logo"].as_str().unwrap().starts_with("dlimg://"), "{v}");
    assert!(chans[1].logo.is_none());
    assert_eq!(chans[2].quality, Some("FHD"));

    let groups = e.svc.list_groups(None).await.unwrap();
    let g = serde_json::to_value(&groups[0]).unwrap();
    assert_eq!(
        (
            g["name"].as_str(),
            g["channelCount"].as_i64(),
            g["count"].as_i64()
        ),
        (Some("News"), Some(2), Some(2))
    );

    // search + favourites
    let found = e
        .svc
        .list_channels(ListChannelsArgs {
            query: Some("two".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    e.svc.set_favourite(found[0].id, true).await.unwrap();
    let favs = e
        .svc
        .list_channels(ListChannelsArgs {
            favourites_only: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(favs[0].is_favourite && favs.len() == 1);

    // adult shows only when the user opts in
    e.svc.set_content_filters(true, false).await.unwrap();
    assert_eq!(
        e.svc
            .list_channels(ListChannelsArgs::default())
            .await
            .unwrap()
            .len(),
        4
    );

    // guide window + programme dto
    let guide = e
        .svc
        .get_guide(GuideArgs {
            start: START,
            end: START + 4 * 3600,
            channel_offset: 0,
            channel_limit: Some(10),
            group_id: None,
        })
        .await
        .unwrap();
    assert!(!guide.programmes.is_empty());
    let pv = serde_json::to_value(&guide.programmes[0]).unwrap();
    for k in [
        "id",
        "channelId",
        "startUtc",
        "endUtc",
        "title",
        "subtitle",
        "description",
        "season",
        "episode",
        "isNew",
        "isLive",
        "isPremiere",
        "isFinale",
        "isScheduled",
    ] {
        assert!(pv.get(k).is_some(), "missing {k}");
    }
    let s = e.svc.search("morning", 10).await.unwrap();
    assert!(
        s.channels.is_empty(),
        "programme search only returns programmes that have not ended"
    );
    assert_eq!(e.svc.get_now_next(&[chans[0].id]).await.unwrap().len(), 1);
}

#[tokio::test]
async fn dlimg_is_served_through_the_cache_with_auth_and_resized() {
    let (server, e, _) = added().await;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        640,
        320,
        image::Rgba([1, 2, 3, 200]),
    ))
    .write_to(&mut png, image::ImageFormat::Png)
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/channels/logos/100/cache"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png.into_inner()))
        .expect(1)
        .mount(&server)
        .await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let logo = chans[0].logo.clone().unwrap();
    let img = e.svc.image(&logo).await.unwrap();
    assert_eq!(img.mime, "image/png");
    assert_eq!(image::load_from_memory(&img.bytes).unwrap().width(), 160);
    e.svc.image(&logo).await.unwrap(); // second call: disk cache, no second request (expect(1))
    assert!(e.svc.image("dlimg://nothex/160").await.is_err());
}

#[tokio::test]
async fn play_channel_proxy_redirect_and_recents() {
    let (server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let plan = e.svc.play_channel(chans[0].id).await.unwrap();
    assert_eq!(
        plan.url,
        format!("{}/proxy/ts/stream/{UUID1}", server.uri())
    );
    assert_eq!(plan.headers[0].0, "X-API-Key");

    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID2}")))
        .and(header("X-API-Key", KEY))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "http://provider.test/live/u/p/2.ts"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let plan = e.svc.play_channel(chans[1].id).await.unwrap();
    assert_eq!(plan.url, "http://provider.test/live/u/p/2.ts");
    assert!(plan.headers.is_empty());

    let home = e.svc.get_home().await.unwrap();
    assert_eq!(home.recents.len(), 2);
    let hv = serde_json::to_value(&home).unwrap();
    for k in ["continueWatching", "favourites", "onNow", "recents"] {
        assert!(hv.get(k).is_some(), "missing {k}");
    }
    // exactly one proxy stream request happened (the redirect resolve); none for the proxy channel
    let n = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().starts_with("/proxy/"))
        .count();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn catchup_through_the_service_tracks_and_cleans_up() {
    let (server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = chans[0].id;
    Mock::given(method("POST")).and(path("/api/catchup/sessions/"))
        .and(body_json(json!({"channel_uuid": UUID1, "start": "2025-01-01T10:00:00Z", "duration": 60})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"session_id": "svc-sess", "playback_url": "/proxy/catchup/x?session_id=svc-sess"})))
        .mount(&server).await;
    Mock::given(method("DELETE"))
        .and(path("/api/catchup/sessions/svc-sess/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let play = e.svc.play_catchup(&format!("{one}:{START}")).await.unwrap();
    assert_eq!(e.svc.store().list_open_sessions().await.unwrap().len(), 1);
    // quit: everything persisted is stopped
    let r = e.svc.stop_all_sessions().await.unwrap();
    assert_eq!(r.stopped, 1);
    assert!(e.svc.store().list_open_sessions().await.unwrap().is_empty());
    let _ = play;
    assert!(e.svc.play_catchup("garbage").await.is_err());
}

#[tokio::test]
async fn settings_roundtrip_and_clamp() {
    let e = env().await;
    let d = e.svc.get_settings().await.unwrap();
    assert_eq!(d["liveBufferSecs"], 3);
    assert_eq!(d["hwdec"], "auto-safe");
    let s = e
        .svc
        .set_settings(json!({"liveBufferSecs": 99, "audioDevice": "pulse", "debugLogging": true}))
        .await
        .unwrap();
    assert_eq!(
        (
            s["liveBufferSecs"].as_i64(),
            s["audioDevice"].as_str(),
            s["debugLogging"].as_bool()
        ),
        (Some(10), Some("pulse"), Some(true))
    );
    assert_eq!(e.svc.get_settings().await.unwrap(), s, "persisted");
    assert!(e.svc.set_settings(json!(5)).await.is_err());
}

#[tokio::test]
async fn remove_source_deletes_secret_rows_and_stops_sessions() {
    let (server, e, sid) = added().await;
    let r = e.svc.store().source(sid).await.unwrap().secret_ref.unwrap();
    e.svc
        .store()
        .add_open_session(sid, "hls", "tok")
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path("/api/proxy/hls/sessions/tok/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    e.svc.remove_source(sid).await.unwrap();
    assert!(e.secrets.get(&r).unwrap().is_none());
    assert!(e.svc.list_sources().await.unwrap().is_empty());
    assert!(
        e.svc
            .list_channels(ListChannelsArgs::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sync_failure_sets_status_and_diagnostics_are_redacted() {
    let (server, e, sid) = added().await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = e.svc.sync_source(sid).await.unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Auth);
    let s = e.svc.list_sources().await.unwrap();
    assert_eq!(serde_json::to_value(&s[0]).unwrap()["status"], "auth_error");

    e.svc
        .log_tail()
        .push("GET http://p.test/live/alice/hunter2/1.ts X-API-Key: sk_zzzzzzzz");
    let d = e.svc.diagnostics().await.unwrap();
    let dv = serde_json::to_value(&d).unwrap();
    assert_eq!(dv["appVersion"], "0.0.0-test");
    assert_eq!(dv["serverVersions"][0], "0.32.0");
    let tail = dv["logTail"].to_string();
    assert!(
        !tail.contains("hunter2") && !tail.contains("sk_zzzzzzzz"),
        "{tail}"
    );
}

#[test]
fn app_error_serialises_camel_case_for_the_ui() {
    let v = serde_json::to_value(downlink_core::AppError::from_status(429, Some(9), "x")).unwrap();
    assert_eq!(v["retryAfter"], 9);
    assert_eq!(v["code"], "server");
    assert!(v["message"].is_string());
}
