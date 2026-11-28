#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Façade tests: the command-level verbs against a mock Dispatcharr, DTO shapes as the UI expects.

use std::io::Cursor;
use std::sync::Arc;

use downlink_core::auth::{MemorySecretStore, SecretStore};
use downlink_core::service::dto::CatalogChange;
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

/// First import: channel fetch reports progress per page, and the channel list is announced
/// (and committed) before the guide phase starts.
#[tokio::test]
async fn first_import_reports_channel_progress_and_shows_channels_before_the_guide() {
    use downlink_core::service::Event;
    let server = MockServer::start().await;
    mount_server(&server).await;
    let e = env().await;
    let mut rx = e.svc.subscribe();
    e.svc.add_source(input(&server)).await.unwrap();
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    let pos = |pred: &dyn Fn(&Event) -> bool| events.iter().position(pred);
    let channel_pages: Vec<usize> = events
        .iter()
        .filter_map(|e| match e {
            Event::TaskProgress(p) if p.phase == "channels" && p.done > 0 => Some(p.done),
            _ => None,
        })
        .collect();
    assert!(
        channel_pages.len() >= 2,
        "one report per page: {channel_pages:?}"
    );
    assert!(
        channel_pages.windows(2).all(|w| w[0] < w[1]),
        "{channel_pages:?}"
    );
    let first_catalog = pos(&|e| matches!(e, Event::CatalogUpdated(_))).unwrap();
    let guide_start = pos(&|e| matches!(e, Event::TaskProgress(p) if p.phase == "guide")).unwrap();
    assert!(
        first_catalog < guide_start,
        "channels are announced before the guide phase begins"
    );
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
        1 | 2 | 4 | 16 | 32 | 512,
        "LIVE|EPG|CATCHUP|DVR|FAVOURITES|STREAM_SWITCH (admin fixture) in the UI bit layout"
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
        // The sports rebuild runs in the background and may land after the sync's own update.
        if let Event::CatalogUpdated(c) = ev
            && !matches!(c.scope, CatalogChange::Sports)
        {
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

fn hls_redirect(token: &str) -> ResponseTemplate {
    ResponseTemplate::new(302)
        .insert_header(
            "Location",
            format!("/proxy/hls/{token}/index.m3u8").as_str(),
        )
        .insert_header("X-Dispatcharr-Session-Token", token)
}

#[tokio::test]
async fn hls_transport_is_opt_in_minted_at_play_tracked_and_released() {
    use downlink_core::service::SetSourceOptionsArgs;
    let (server, e, sid) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let dto = &e.svc.list_sources().await.unwrap()[0];
    assert!(dto.supports_hls_transport && !dto.hls_transport);

    // Off by default: the plain proxy plan, no mint request.
    let plan = e.svc.play_channel(chans[0].id).await.unwrap();
    assert!(plan.url.ends_with(&format!("/proxy/ts/stream/{UUID1}")));
    assert!(
        !server.received_requests().await.unwrap().iter().any(|r| r
            .url
            .query()
            .unwrap_or("")
            .contains("output_format")),
        "nothing is minted unless the user opted in"
    );

    let dto = e
        .svc
        .set_source_options(SetSourceOptionsArgs {
            source_id: sid,
            hls_transport: true,
        })
        .await
        .unwrap();
    assert!(dto.hls_transport);

    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID1}")))
        .and(query_param("output_format", "hls"))
        .and(header("X-API-Key", KEY))
        .respond_with(hls_redirect("TokenOne_abcdefghijklmnopqrstu"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID1}")))
        .and(query_param("output_format", "hls"))
        .respond_with(hls_redirect("TokenTwo_abcdefghijklmnopqrstu"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/api/proxy/hls/sessions/TokenOne_abcdefghijklmnopqrstu/",
        ))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/api/proxy/hls/sessions/TokenTwo_abcdefghijklmnopqrstu/",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let plan = e.svc.play_channel(chans[0].id).await.unwrap();
    assert_eq!(
        plan.url,
        format!(
            "{}/proxy/hls/TokenOne_abcdefghijklmnopqrstu/index.m3u8",
            server.uri()
        )
    );
    assert!(
        plan.headers.is_empty(),
        "the token in the path is the credential"
    );
    assert_eq!(plan.surface, downlink_core::source::Surface::Hls);
    assert_eq!(plan.profile, downlink_core::source::PlayProfile::Live);
    let open = e.svc.store().list_open_sessions().await.unwrap();
    assert_eq!(
        (open.len(), open[0].kind.as_str()),
        (1, "hls"),
        "persisted before playback (crash cleanup)"
    );
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(!shown.contains("TokenOne"), "{shown}");

    // 410 on playback: re-mint once; the ended session is released; a second time is refused.
    let plan2 = e.svc.remint_hls(chans[0].id).await.unwrap();
    assert!(plan2.url.contains("TokenTwo_"));
    assert_eq!(e.svc.store().list_open_sessions().await.unwrap().len(), 1);
    assert!(e.svc.remint_hls(chans[0].id).await.is_err());

    // Navigate away: the session is deleted and forgotten.
    e.svc.stop_live_hls().await;
    assert!(e.svc.store().list_open_sessions().await.unwrap().is_empty());
}

#[tokio::test]
async fn hls_session_is_released_on_zap_and_on_quit_and_by_crash_cleanup() {
    use downlink_core::service::SetSourceOptionsArgs;
    let (server, e, sid) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    e.svc
        .set_source_options(SetSourceOptionsArgs {
            source_id: sid,
            hls_transport: true,
        })
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID1}")))
        .and(query_param("output_format", "hls"))
        .respond_with(hls_redirect("ZapToken_abcdefghijklmnopqrstu"))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/api/proxy/hls/sessions/ZapToken_abcdefghijklmnopqrstu/",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(2)
        .mount(&server)
        .await;
    // A Redirect-profile channel keeps its normal path even with the option on.
    Mock::given(method("GET"))
        .and(path(format!("/proxy/ts/stream/{UUID2}")))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "http://provider.test/live/u/p/2.ts"),
        )
        .mount(&server)
        .await;

    e.svc.play_channel(chans[0].id).await.unwrap();
    assert_eq!(e.svc.store().list_open_sessions().await.unwrap().len(), 1);
    // Zap to the redirect channel: first DELETE.
    let plan = e.svc.play_channel(chans[1].id).await.unwrap();
    assert_eq!(plan.url, "http://provider.test/live/u/p/2.ts");
    assert!(e.svc.store().list_open_sessions().await.unwrap().is_empty());

    // Quit (sleep): second DELETE.
    e.svc.play_channel(chans[0].id).await.unwrap();
    let r = e.svc.stop_all_sessions().await.unwrap();
    assert_eq!(r.stopped, 1);
    assert!(e.svc.store().list_open_sessions().await.unwrap().is_empty());

    // A crashed run left a token behind: startup cleanup deletes it (404 = already gone is fine).
    e.svc
        .store()
        .add_open_session(sid, "hls", "Crashed_abcdefghijklmnopqrstu")
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path(
            "/api/proxy/hls/sessions/Crashed_abcdefghijklmnopqrstu/",
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let r = e.svc.startup_cleanup().await.unwrap();
    assert_eq!(r.stopped, 1);
}

#[tokio::test]
async fn only_dispatcharr_sources_accept_the_hls_option() {
    use downlink_core::service::SetSourceOptionsArgs;
    let (_s, e, _) = added().await;
    let row = e
        .svc
        .store()
        .insert_source(&downlink_core::store::NewSource {
            kind: "m3u".into(),
            name: "P".into(),
            base_url: Some("file:///x.m3u".into()),
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let err = e
        .svc
        .set_source_options(SetSourceOptionsArgs {
            source_id: row.id,
            hls_transport: true,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Local);
    let m3u_dto = e
        .svc
        .list_sources()
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == row.id)
        .unwrap();
    assert!(!m3u_dto.supports_hls_transport);
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

/// PLAN M4: resume lands within 2 s of the saved position (catch-up, from the position reports).
#[tokio::test]
async fn catchup_resume_position_is_within_two_seconds_of_the_last_report() {
    let (server, e, sid) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = chans[0].id;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"session_id": "rs", "playback_url": "/proxy/catchup/x?session_id=rs"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/rs/position/"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let play = e.svc.play_catchup(&format!("{one}:{START}")).await.unwrap();
    let mut rep = downlink_core::catchup::PositionReporter::new(play.session.clone());
    let saved_secs = 1_234u64;
    e.svc
        .report_catchup_position(&play, &mut rep, 0, saved_secs, true)
        .await
        .unwrap();
    let ch = e.svc.store().channel(one).await.unwrap().unwrap();
    let row = e
        .svc
        .store()
        .resume(
            sid,
            "catchup",
            &downlink_core::catchup::resume_key(&ch.ext_id, play.start_utc),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        (row.position_ms - (saved_secs as i64) * 1000).abs() <= 2_000,
        "saved {} ms",
        row.position_ms
    );
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

// ---- failover / counts / channel DTOs / home + search shapes -----------------------------------

#[tokio::test]
async fn failover_walks_the_stream_list_with_a_hard_attempt_limit() {
    let (server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = chans[0].id; // Channel One: streams [11, 12]
    get(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!("/proxy/ts/next_stream/{UUID1}")))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .expect(2)
        .mount(&server)
        .await;

    let list = e.svc.channel_streams(one).await.unwrap();
    let v = serde_json::to_value(&list).unwrap();
    assert_eq!(v["sourceCount"], 2);
    assert_eq!(
        v["streams"][0]["streamId"], "12",
        "failover order from the server, never reordered"
    );
    assert_eq!(v["streams"][0]["name"], "Source B");
    assert!(
        !v.to_string().contains("provider.test"),
        "provider URLs never leave core"
    );

    e.svc.play_channel(one).await.unwrap();
    assert_eq!(e.svc.source_position(one).await.unwrap(), (1, 2));
    // attempt 1: reconnect the same URL, no server change
    let s1 = e.svc.failover_next(one, 1).await.unwrap();
    assert!(!s1.switched);
    assert_eq!((s1.source_index, s1.source_count), (1, 2));
    assert_eq!(
        s1.plan.url,
        format!("{}/proxy/ts/stream/{UUID1}", server.uri())
    );
    // attempts 2 and 3: ask the server for the next stream (wraps around)
    let s2 = e.svc.failover_next(one, 2).await.unwrap();
    assert!(s2.switched);
    assert_eq!(s2.source_index, 2);
    let s3 = e.svc.failover_next(one, 3).await.unwrap();
    assert!(s3.switched);
    assert_eq!(s3.source_index, 1);
    // never loop forever
    let err = e.svc.failover_next(one, 4).await.unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Stream);
    assert!(e.svc.failover_next(one, 0).await.is_err());
    // zero stream-endpoint requests were made by any of this except via the proxy plan (no GETs).
    let streams_gets = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "GET" && r.url.path().starts_with("/proxy/"))
        .count();
    assert_eq!(streams_gets, 0);
}

#[tokio::test]
async fn failover_on_an_inactive_channel_does_not_pretend_to_switch() {
    let (server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path(format!("/proxy/ts/next_stream/{UUID1}")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let s = e.svc.failover_next(chans[0].id, 2).await.unwrap();
    assert!(!s.switched);
    assert_eq!(s.source_index, 1);
    // single-stream channel: nothing to switch to, plan still returned
    let six = chans.iter().find(|c| c.name == "Channel Six").unwrap();
    assert!(!e.svc.failover_next(six.id, 3).await.unwrap().switched);
}

#[tokio::test]
async fn counts_channel_by_id_and_filters() {
    let (_server, e, _) = added().await;
    let c = e.svc.get_counts().await.unwrap();
    assert_eq!((c.all, c.favourites), (3, 0));
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    e.svc.set_favourite(chans[0].id, true).await.unwrap();
    assert_eq!(e.svc.get_counts().await.unwrap().favourites, 1);
    let v = serde_json::to_value(e.svc.get_counts().await.unwrap()).unwrap();
    assert_eq!(
        (v["all"].as_i64(), v["favourites"].as_i64()),
        (Some(3), Some(1))
    );
    // adult opt-in raises the totals; a hidden channel's favourite never counts
    e.svc.set_content_filters(true, true).await.unwrap();
    assert_eq!(e.svc.get_counts().await.unwrap().all, 5);

    let one = e.svc.get_channel(chans[0].id).await.unwrap();
    assert!(one.logo.as_deref().unwrap().starts_with("dlimg://") && one.is_favourite);
    assert_eq!(one.name, "Channel One");
    assert!(e.svc.get_channel(99_999).await.is_err());
}

#[tokio::test]
async fn channel_neighbours_follow_list_order_and_wrap() {
    let (_server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    assert_eq!(chans.len(), 3);
    let mid = e.svc.channel_neighbours(chans[1].id).await.unwrap();
    assert_eq!(mid.prev.map(|c| c.id), Some(chans[0].id));
    assert_eq!(mid.next.map(|c| c.id), Some(chans[2].id));
    let first = e.svc.channel_neighbours(chans[0].id).await.unwrap();
    assert_eq!(
        first.prev.map(|c| c.id),
        Some(chans[2].id),
        "wraps to the last"
    );
    let last = e.svc.channel_neighbours(chans[2].id).await.unwrap();
    assert_eq!(
        last.next.map(|c| c.id),
        Some(chans[0].id),
        "wraps to the first"
    );
    let gone = e.svc.channel_neighbours(99_999).await.unwrap();
    assert!(gone.prev.is_none() && gone.next.is_none());
}

#[tokio::test]
async fn home_and_search_carry_full_channel_dtos_in_the_ui_shapes() {
    let (_server, e, sid) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = &chans[0];
    let now = downlink_core::sync::now_utc();
    // A programme airing now and one resume position for catch-up.
    let mk = |s: i64, t: &str| downlink_core::store::ProgrammeRow {
        channel_id: one.id,
        start_utc: s,
        end_utc: s + 3600,
        ext_id: None,
        title: Some(t.into()),
        subtitle: None,
        description: None,
        season: None,
        episode: None,
        flags: 0,
        icon: None,
    };
    let past = now - 7200;
    e.svc
        .store()
        .upsert_programmes(&[mk(now - 600, "Airing Now"), mk(past, "Yesterday Show")])
        .await
        .unwrap();
    e.svc.set_favourite(one.id, true).await.unwrap();
    e.svc.play_channel(one.id).await.unwrap(); // becomes the 'live' continue item and a recent
    let ch = e.svc.store().channel(one.id).await.unwrap().unwrap();
    e.svc
        .set_watch_position(
            sid,
            "catchup",
            &downlink_core::catchup::resume_key(&ch.ext_id, past),
            900_000,
            Some(3_600_000),
        )
        .await
        .unwrap();

    let home = serde_json::to_value(e.svc.get_home().await.unwrap()).unwrap();
    let cw = home["continueWatching"].as_array().unwrap();
    assert_eq!(cw[0]["kind"], "live");
    assert_eq!(cw[0]["title"], "Airing Now");
    assert!(
        cw[0]["channel"]["logo"]
            .as_str()
            .unwrap()
            .starts_with("dlimg://")
    );
    assert!(cw[0]["progress"].is_null());
    let cu = cw.iter().find(|i| i["kind"] == "catchup").unwrap();
    for k in [
        "id",
        "channel",
        "programme",
        "title",
        "subtitle",
        "art",
        "progress",
        "remainingSecs",
    ] {
        assert!(cu.get(k).is_some(), "missing {k}: {cu}");
    }
    assert_eq!(cu["title"], "Yesterday Show");
    assert_eq!(cu["progress"], 0.25);
    assert_eq!(cu["remainingSecs"], 2700);
    assert_eq!(cu["programme"]["id"], format!("{}:{past}", one.id));
    assert!(cu["channel"]["logo"].is_string());
    let on_now = home["onNow"].as_array().unwrap();
    assert_eq!(on_now[0]["now"]["title"], "Airing Now");
    assert!(on_now[0]["channel"]["logo"].is_string());
    assert!(home["favourites"][0]["logo"].is_string() && home["recents"][0]["logo"].is_string());

    // Search: programme hits carry {programme, channel} with the channel's logo.
    e.svc
        .store()
        .upsert_programmes(&[mk(now + 1800, "Colour Futures")])
        .await
        .unwrap();
    let sr = serde_json::to_value(e.svc.search("colour", 10).await.unwrap()).unwrap();
    let hit = &sr["programmes"][0];
    assert_eq!(hit["programme"]["title"], "Colour Futures");
    assert!(
        hit["channel"]["logo"]
            .as_str()
            .unwrap()
            .starts_with("dlimg://"),
        "{hit}"
    );
    assert_eq!(hit["channel"]["id"], one.id);

    let progs = e
        .svc
        .get_programmes(one.id, past - 10, now + 7200)
        .await
        .unwrap();
    assert!(progs.len() >= 3);
}

// ---- source status at launch --------------------------------------------------------------------

#[tokio::test]
async fn status_is_connecting_until_probed_then_connected_offline_or_auth_error() {
    let (server, e, sid) = added().await;
    // "Launch": a fresh Service over the same store has not probed anything yet.
    let dir = tempfile::tempdir().unwrap();
    let relaunched = Service::with_store(
        e.svc.store().clone(),
        e.secrets.clone(),
        dir.path().join("img"),
        "t".into(),
    );
    async fn status(s: &Service) -> Value {
        serde_json::to_value(&s.list_sources().await.unwrap()[0]).unwrap()["status"].clone()
    }
    assert_eq!(
        status(&relaunched).await,
        "connecting",
        "never `connected` before the first probe"
    );
    assert!(
        relaunched.list_sources().await.unwrap()[0]
            .last_sync_at
            .is_some(),
        "last updated is still shown"
    );

    // Probe uses cheap identity calls only (no channel/guide fetch, no stream endpoints).
    let before = server.received_requests().await.unwrap().len();
    relaunched.probe_sources().await.unwrap();
    assert_eq!(status(&relaunched).await, "connected");
    let new: Vec<String> = server.received_requests().await.unwrap()[before..]
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(new, ["/api/accounts/users/me/"], "{new:?}");

    // Bad key -> auth_error; server gone -> offline.
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    relaunched.probe_source(sid).await.unwrap();
    assert_eq!(status(&relaunched).await, "auth_error");
    // Server gone: a fresh launch pointed at a dead port.
    sqlx::query("UPDATE source SET base_url = 'http://127.0.0.1:1/' WHERE id = ?")
        .bind(sid)
        .execute(e.svc.store().pool())
        .await
        .unwrap();
    let offline = Service::with_store(
        e.svc.store().clone(),
        e.secrets.clone(),
        dir.path().join("img2"),
        "t".into(),
    );
    offline.probe_source(sid).await.unwrap();
    assert_eq!(status(&offline).await, "offline");
    let relaunched = offline;
    // The cached catalogue is still browsable while offline.
    assert_eq!(
        relaunched
            .list_channels(ListChannelsArgs::default())
            .await
            .unwrap()
            .len(),
        3
    );
}

fn relaunch(e: &Env, dir: &tempfile::TempDir, name: &str) -> Arc<Service> {
    Service::with_store(
        e.svc.store().clone(),
        e.secrets.clone(),
        dir.path().join(name),
        "t".into(),
    )
}

async fn age_last_sync(e: &Env, sid: i64, hours: i64) {
    sqlx::query("UPDATE source SET last_sync_at = last_sync_at - ? WHERE id = ?")
        .bind(hours * 3600)
        .bind(sid)
        .execute(e.svc.store().pool())
        .await
        .unwrap();
}

async fn status_of(s: &Service) -> Value {
    serde_json::to_value(&s.list_sources().await.unwrap()[0]).unwrap()["status"].clone()
}

/// Launch with old data: the source is refreshed straight away instead of being shown as stale.
#[tokio::test]
async fn launch_refreshes_old_data_instead_of_showing_stale() {
    let (server, e, sid) = added().await;
    age_last_sync(&e, sid, 24).await;
    let dir = tempfile::tempdir().unwrap();
    let relaunched = relaunch(&e, &dir, "img");
    let before = server.received_requests().await.unwrap().len();
    relaunched.probe_sources().await.unwrap();
    assert_eq!(status_of(&relaunched).await, "connected");
    let fetched_channels = server.received_requests().await.unwrap()[before..]
        .iter()
        .any(|r| r.url.path() == "/api/channels/channels/");
    assert!(fetched_channels, "old data triggers a sync at launch");
    let row = &relaunched.list_sources().await.unwrap()[0];
    assert!(row.last_sync_at.unwrap() > downlink_core::sync::now_utc() - 60);
}

/// Fresh data at launch: probe only, no sync.
#[tokio::test]
async fn launch_with_fresh_data_does_not_resync() {
    let (server, e, _sid) = added().await;
    let dir = tempfile::tempdir().unwrap();
    let relaunched = relaunch(&e, &dir, "img");
    let before = server.received_requests().await.unwrap().len();
    relaunched.probe_sources().await.unwrap();
    relaunched.auto_refresh(false).await.unwrap();
    let new: Vec<String> = server.received_requests().await.unwrap()[before..]
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(new, ["/api/accounts/users/me/"], "{new:?}");
}

/// The stale warning only shows when the refresh itself fails.
#[tokio::test]
async fn stale_shows_only_when_the_auto_refresh_fails() {
    let (server, e, sid) = added().await;
    age_last_sync(&e, sid, 24).await;
    // Identity works, the channel list does not: probe passes, the sync fails (not network/auth).
    server.reset().await;
    get(&server, "/api/accounts/users/me/", fixture!("me.json")).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let relaunched = relaunch(&e, &dir, "img");
    relaunched.probe_sources().await.unwrap();
    let s = status_of(&relaunched).await;
    assert!(s == "stale" || s == "offline", "{s}");
    // The next tick retries; once the server is back the source recovers by itself.
    server.reset().await;
    mount_server(&server).await;
    relaunched.auto_refresh(false).await.unwrap();
    assert_eq!(status_of(&relaunched).await, "connected");
}

/// The periodic tick syncs a source whose data aged past the threshold while the app ran.
#[tokio::test]
async fn auto_refresh_syncs_sources_that_aged_while_running() {
    let (server, e, sid) = added().await;
    age_last_sync(&e, sid, 4).await;
    let before = server.received_requests().await.unwrap().len();
    e.svc.auto_refresh(false).await.unwrap();
    let synced = server.received_requests().await.unwrap()[before..]
        .iter()
        .any(|r| r.url.path() == "/api/channels/channels/");
    assert!(synced);
    assert_eq!(status_of(&e.svc).await, "connected");
}

/// `autoRefreshHours` 0 turns auto refresh off: no launch sync, and old data shows as stale.
#[tokio::test]
async fn auto_refresh_off_leaves_old_data_stale() {
    let (server, e, sid) = added().await;
    e.svc
        .set_settings(json!({"autoRefreshHours": 0}))
        .await
        .unwrap();
    age_last_sync(&e, sid, 24).await;
    let dir = tempfile::tempdir().unwrap();
    let relaunched = relaunch(&e, &dir, "img");
    let before = server.received_requests().await.unwrap().len();
    relaunched.probe_sources().await.unwrap();
    relaunched.auto_refresh(false).await.unwrap();
    let new: Vec<String> = server.received_requests().await.unwrap()[before..]
        .iter()
        .map(|r| r.url.path().to_owned())
        .collect();
    assert_eq!(new, ["/api/accounts/users/me/"], "{new:?}");
    assert_eq!(status_of(&relaunched).await, "stale");
}

/// The interval setting decides when data is due, and is clamped to 0..=24 hours.
#[tokio::test]
async fn auto_refresh_interval_setting_is_respected_and_clamped() {
    let (server, e, sid) = added().await;
    let v = e
        .svc
        .set_settings(json!({"autoRefreshHours": 99}))
        .await
        .unwrap();
    assert_eq!(v["autoRefreshHours"], 24);
    let d = e.svc.get_settings().await.unwrap();
    assert_eq!(d["refreshDuringPlayback"], true, "default on");
    e.svc
        .set_settings(json!({"autoRefreshHours": 12}))
        .await
        .unwrap();
    age_last_sync(&e, sid, 6).await;
    let before = server.received_requests().await.unwrap().len();
    e.svc.auto_refresh(false).await.unwrap();
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        before,
        "6 h old is not due on a 12 h interval"
    );
    assert_eq!(
        status_of(&e.svc).await,
        "connected",
        "age alone is not stale"
    );
    age_last_sync(&e, sid, 7).await;
    e.svc.auto_refresh(false).await.unwrap();
    assert!(server.received_requests().await.unwrap().len() > before);
}

/// `refreshDuringPlayback` off: nothing is synced while playing; the next idle tick catches up.
#[tokio::test]
async fn refresh_waits_for_playback_to_end_when_that_setting_is_off() {
    let (server, e, sid) = added().await;
    e.svc
        .set_settings(json!({"refreshDuringPlayback": false}))
        .await
        .unwrap();
    age_last_sync(&e, sid, 24).await;
    let before = server.received_requests().await.unwrap().len();
    e.svc.auto_refresh(true).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    e.svc.auto_refresh(false).await.unwrap();
    assert!(server.received_requests().await.unwrap().len() > before);
    // Default (on): playing does not hold the refresh back.
    e.svc
        .set_settings(json!({"refreshDuringPlayback": true}))
        .await
        .unwrap();
    age_last_sync(&e, sid, 24).await;
    let before = server.received_requests().await.unwrap().len();
    e.svc.auto_refresh(true).await.unwrap();
    assert!(server.received_requests().await.unwrap().len() > before);
}

/// Dispatcharr sync fills the guide ahead to 7 days, not just the visible 12 h window.
#[tokio::test]
async fn dispatcharr_sync_fills_the_guide_seven_days_ahead() {
    let (server, _e, _sid) = added().await;
    let now = downlink_core::sync::now_utc();
    let ends: Vec<i64> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/epg/grid/")
        .filter_map(|r| {
            r.url
                .query_pairs()
                .find(|(k, _)| k == "end")
                .and_then(|(_, v)| chrono::DateTime::parse_from_rfc3339(&v).ok())
                .map(|d| d.timestamp())
        })
        .collect();
    let furthest = ends.iter().copied().max().unwrap();
    assert!(
        furthest >= now + 7 * 86_400 - 120,
        "furthest window ends {} h ahead",
        (furthest - now) / 3600
    );
}

/// Stream picker (PLAN §3.4, D18): an admin account switches the live channel to a chosen stream
/// through `change_stream`, and the reported position follows.
#[tokio::test]
async fn stream_picker_switches_through_change_stream_for_admins() {
    let (server, e, sid) = added().await;
    let src = e.svc.store().source(sid).await.unwrap();
    assert!(
        downlink_core::source::Caps::from_db(src.caps)
            .contains(downlink_core::source::Caps::STREAM_SWITCH),
        "admin fixture account may switch streams"
    );
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = chans[0].id; // Channel One: streams [12, 11] in failover order
    get(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(format!("/proxy/ts/change_stream/{UUID1}")))
        .and(header("X-API-Key", KEY))
        .and(wiremock::matchers::body_json(json!({"stream_id": "11"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .expect(1)
        .mount(&server)
        .await;
    e.svc.play_channel(one).await.unwrap();
    assert_eq!(e.svc.switch_stream(one, 1).await.unwrap(), (2, 2));
    assert_eq!(e.svc.source_position(one).await.unwrap(), (2, 2));
    // An index past the list is refused without a server call.
    assert!(e.svc.switch_stream(one, 9).await.is_err());
}

/// A non-admin account never gets the stream picker, and the service refuses without asking the server.
#[tokio::test]
async fn stream_picker_is_hidden_and_refused_for_non_admins() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/accounts/users/me/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": 2, "username": "viewer", "user_level": 1, "stream_limit": 3}),
        ))
        .with_priority(1)
        .mount(&server)
        .await;
    mount_server(&server).await;
    let e = env().await;
    let s = e.svc.add_source(input(&server)).await.unwrap();
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["caps"].as_u64().unwrap() & 512, 0, "no STREAM_SWITCH bit");
    assert_eq!(
        v["streamLimit"], 3,
        "stream limit recorded for the multiview budget"
    );
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    assert!(e.svc.switch_stream(chans[0].id, 0).await.is_err());
    let switched = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().starts_with("/proxy/ts/change_stream/"))
        .count();
    assert_eq!(switched, 0);
}

#[tokio::test]
async fn live_source_position_reads_the_server_stream_list_and_status() {
    let (server, e, _) = added().await;
    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let one = chans[0].id; // Channel One: streams [12, 11] on the server
    get(
        &server,
        "/api/channels/channels/1/streams/",
        fixture!("channel_streams.json"),
    )
    .await;
    get(
        &server,
        &format!("/proxy/ts/status/{UUID1}"),
        json!({"state": "active", "stream_id": 11}),
    )
    .await;
    e.svc.play_channel(one).await.unwrap();
    // the server is on its second stream, not the first
    assert_eq!(e.svc.live_source_position(one).await.unwrap(), (2, 2));
    assert_eq!(e.svc.source_position(one).await.unwrap().0, 2);
}

/// Between full syncs, a Dispatcharr guide's visible hours are re-fetched once they are an hour
/// old, with a single grid request (no stream endpoints).
#[tokio::test]
async fn visible_guide_refreshes_only_once_it_is_an_hour_old() {
    let (server, e, _sid) = added().await;
    let grid_requests = || async {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == "/api/epg/grid/")
            .count()
    };
    let after_sync = grid_requests().await;
    e.svc.refresh_visible_guides().await.unwrap();
    assert_eq!(
        grid_requests().await,
        after_sync,
        "just synced: nothing to do"
    );

    sqlx::query("UPDATE sync_state SET last_ok = ? WHERE entity = 'guide'")
        .bind(downlink_core::sync::now_utc() - 2 * 3600)
        .execute(e.svc.store().pool())
        .await
        .unwrap();
    e.svc.refresh_visible_guides().await.unwrap();
    assert_eq!(grid_requests().await, after_sync + 1);
    e.svc.refresh_visible_guides().await.unwrap();
    assert_eq!(grid_requests().await, after_sync + 1, "fresh again");
}

/// Saved command centers: DTO shapes, hidden channels dropped on load, validation, delete.
#[tokio::test]
async fn command_centers_save_list_get_and_delete() {
    use downlink_core::service::CommandCenterSaveDto;
    let (_server, e, _) = added().await;
    let shown = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    e.svc.set_content_filters(true, true).await.unwrap();
    let all = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    e.svc.set_content_filters(false, false).await.unwrap();
    let hidden = all
        .iter()
        .find(|c| !shown.iter().any(|s| s.id == c.id))
        .unwrap();

    let save: CommandCenterSaveDto = serde_json::from_value(json!({
        "id": null, "name": " Newsroom ", "layoutStyle": "grid", "audible": 1,
        "rail": { "feeds": [{ "id": 3 }] },
        "tiles": [
            { "slot": 0, "channelId": shown[0].id },
            { "slot": 1, "channelId": shown[1].id },
            { "slot": 2, "channelId": hidden.id }
        ]
    }))
    .unwrap();
    let id = e.svc.command_center_save(save.clone()).await.unwrap();

    let list = serde_json::to_value(e.svc.command_centers_list().await.unwrap()).unwrap();
    assert_eq!(list[0]["id"], json!(id));
    assert_eq!(list[0]["name"], json!("Newsroom"));
    assert_eq!(list[0]["tileCount"], json!(2), "hidden channel not counted");
    assert_eq!(list[0]["channelIds"], json!([shown[0].id, shown[1].id]));
    assert!(list[0]["updatedUtc"].is_i64());

    let got = serde_json::to_value(e.svc.command_center_get(id).await.unwrap()).unwrap();
    for k in [
        "id",
        "name",
        "layoutStyle",
        "audible",
        "rail",
        "tiles",
        "createdUtc",
        "updatedUtc",
    ] {
        assert!(got.get(k).is_some(), "{k}");
    }
    assert_eq!(got["rail"], json!({ "feeds": [{ "id": 3 }] }));
    assert_eq!(got["tiles"].as_array().unwrap().len(), 2);
    assert_eq!(got["tiles"][1]["slot"], json!(1));
    assert_eq!(got["tiles"][1]["channel"]["name"], json!(shown[1].name));

    let mut bad = save.clone();
    bad.name = String::new();
    assert!(e.svc.command_center_save(bad).await.is_err());

    let mut renamed = save;
    renamed.id = Some(id);
    renamed.name = "Desk".into();
    assert_eq!(e.svc.command_center_save(renamed).await.unwrap(), id);
    assert_eq!(e.svc.command_center_get(id).await.unwrap().name, "Desk");

    e.svc.command_center_delete(id).await.unwrap();
    assert!(e.svc.command_centers_list().await.unwrap().is_empty());
    assert!(e.svc.command_center_get(id).await.is_err());
}
