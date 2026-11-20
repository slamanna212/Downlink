#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Dispatcharr VOD and DVR endpoints, one wiremock test each, synthetic fixtures shaped from the
//! OpenAPI schema (the local server has no VOD or recordings data).

use std::path::PathBuf;

use downlink_core::ErrorCode;
use downlink_core::auth::SecretString;
use downlink_core::source::dispatcharr::{ClientOptions, Credentials, DispatcharrClient};
use downlink_core::source::{
    RecStatus, RecordRequest, RuleKind, Source, SourceId, VodItem, VodKind, VodListQuery,
};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_vod_key_123456";

fn fx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dispatcharr")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
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

async fn get(server: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn endpoint_vod_categories_by_type_with_counts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/vod/categories/"))
        .and(query_param("category_type", "movie"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fx("vod_categories_full.json")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .and(query_param("category", "Action"))
        .and(query_param("page_size", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"count": 7, "next": null, "results": []})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .and(query_param("category", "Comedy"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"count": 2, "next": null, "results": []})),
        )
        .mount(&server)
        .await;
    let c = client(&server)
        .vod_categories(VodKind::Movie)
        .await
        .unwrap();
    assert_eq!(
        c.iter()
            .map(|x| (x.name.as_str(), x.count))
            .collect::<Vec<_>>(),
        [("Action", Some(7)), ("Comedy", Some(2))],
        "series category filtered out"
    );
}

#[tokio::test]
async fn endpoint_movies_list_paging_category_search_and_tolerant_fields() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/vod/categories/",
        fx("vod_categories_full.json"),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .and(query_param("page", "1"))
        .and(query_param("page_size", "100"))
        .and(query_param("category", "Action"))
        .and(query_param("search", "night"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(fx("vod_movies_page1.json")))
        .mount(&server)
        .await;
    let page = client(&server)
        .vod_list(
            VodKind::Movie,
            &VodListQuery {
                category_ext: Some("1".into()),
                search: Some("night".into()),
                offset: 0,
                limit: 50,
            },
        )
        .await
        .unwrap();
    assert_eq!(page.total, Some(3));
    let m = &page.items[0];
    assert_eq!(
        (m.ext_id.as_str(), m.title.as_str(), m.year, m.duration_secs),
        ("11", "Night Train", Some(2021), Some(5400))
    );
    assert_eq!(m.rating.as_deref(), Some("7.4"));
    assert_eq!(
        m.poster.as_deref(),
        Some(format!("{}/api/vod/vodlogos/5/cache/", server.uri()).as_str()),
        "the logo cache URL (same server)"
    );
    assert_eq!(m.backdrop.as_deref(), Some("http://img.test/nt-bd.jpg"));
    assert_eq!(
        m.uuid.as_deref(),
        Some("11111111-0000-4000-8000-000000000011")
    );
    assert!(
        page.items[1].poster.is_none() && page.items[1].duration_secs.is_none(),
        "no logo = no poster; 0 duration = unknown"
    );
    assert_eq!(
        page.items[2].rating.as_deref(),
        Some("n/a"),
        "kept verbatim; the DTO decides it is not a number"
    );
}

#[tokio::test]
async fn movies_unaligned_offset_slices_a_larger_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .and(query_param("page", "1"))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fx("vod_movies_page1.json")))
        .mount(&server)
        .await;
    let page = client(&server)
        .vod_list(
            VodKind::Movie,
            &VodListQuery {
                offset: 1,
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|i| i.ext_id.as_str())
            .collect::<Vec<_>>(),
        ["12", "13"]
    );
}

fn movie_rows(ids: std::ops::Range<u32>) -> Vec<Value> {
    ids.map(|i| json!({"id": i, "name": format!("Film {i}"), "uuid": format!("00000000-0000-4000-8000-{i:012}")}))
        .collect()
}

/// The server caps `page_size` below what we ask for (here 2) and says so only through `next`:
/// the window is assembled from as many server pages as it spans.
#[tokio::test]
async fn movies_window_spans_pages_when_the_server_caps_page_size() {
    let server = MockServer::start().await;
    for page in 1..=4u32 {
        let start = (page - 1) * 2 + 1;
        let end = (start + 2).min(8);
        let next =
            (page < 4).then(|| format!("{}/api/vod/movies/?page={}", server.uri(), page + 1));
        Mock::given(method("GET"))
            .and(path("/api/vod/movies/"))
            .and(query_param("page", page.to_string()))
            .and(query_param("page_size", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"count": 7, "next": next, "results": movie_rows(start..end)}),
            ))
            .mount(&server)
            .await;
    }
    // The first request asks for 100 and gets 2 back with a `next`.
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .and(query_param("page", "1"))
        .and(query_param("page_size", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 7, "next": format!("{}/api/vod/movies/?page=2", server.uri()), "results": movie_rows(1..3)
        })))
        .mount(&server)
        .await;
    let ids = |p: &downlink_core::source::VodListPage| {
        p.items.iter().map(|i| i.ext_id.clone()).collect::<Vec<_>>()
    };
    let c = client(&server);
    let all = c
        .vod_list(
            VodKind::Movie,
            &VodListQuery {
                offset: 0,
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&all), ["1", "2", "3", "4", "5", "6", "7"]);
    assert_eq!(all.total, Some(7));
    let mid = c
        .vod_list(
            VodKind::Movie,
            &VodListQuery {
                offset: 3,
                limit: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ids(&mid), ["4", "5", "6"]);
}

/// A JWT is never handed to mpv for `/proxy/vod/`: it would be replayed on every seek after it
/// expired. The proxy needs no credentials.
#[tokio::test]
async fn vod_plan_carries_no_jwt() {
    let server = MockServer::start().await;
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
    let item = VodItem {
        kind: Some(VodKind::Movie),
        ext_id: "11".into(),
        uuid: Some("11111111-0000-4000-8000-000000000011".into()),
        ..Default::default()
    };
    let plan = c.vod_plan(&item).await.unwrap();
    assert!(
        plan.url
            .ends_with("/proxy/vod/movie/11111111-0000-4000-8000-000000000011")
    );
    assert!(
        plan.headers.is_empty(),
        "{:?}",
        plan.headers.iter().map(|h| &h.0).collect::<Vec<_>>()
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "no login, no request"
    );
}

#[tokio::test]
async fn endpoint_movie_and_series_detail_and_episodes() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/vod/movies/11/",
        fx("vod_movies_page1.json")["results"][0].clone(),
    )
    .await;
    get(
        &server,
        "/api/vod/series/21/",
        fx("vod_series_page1.json")["results"][0].clone(),
    )
    .await;
    get(
        &server,
        "/api/vod/series/21/episodes/",
        fx("vod_episodes_page1.json"),
    )
    .await;
    let c = client(&server);
    let m = c.vod_detail(VodKind::Movie, "11").await.unwrap();
    assert_eq!(m.plot.as_deref(), Some("A synthetic thriller."));
    assert_eq!(m.genre.as_deref(), Some("Action, Thriller"));
    let s = c.vod_detail(VodKind::Series, "21").await.unwrap();
    assert_eq!(s.title, "Fieldwork");
    let eps = c.series_episodes("21").await.unwrap();
    let order: Vec<_> = eps
        .iter()
        .map(|e| {
            (
                e.season.unwrap_or(0),
                e.episode.unwrap_or(0),
                e.ext_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            (0, 0, "305"),
            (1, 1, "301"),
            (1, 2, "302"),
            (2, 1, "303"),
            (2, 2, "304")
        ],
        "sorted; season-less specials first"
    );
    assert!(
        eps.iter().all(|e| e.parent_ext.as_deref() == Some("21")),
        "series as object or bare id"
    );
}

#[tokio::test]
async fn play_vod_builds_proxy_urls_without_any_request() {
    let server = MockServer::start().await;
    let c = client(&server);
    let movie = VodItem {
        kind: Some(VodKind::Movie),
        ext_id: "11".into(),
        uuid: Some("uuid-m".into()),
        ..Default::default()
    };
    let p = c.play_vod(&movie).await.unwrap();
    assert_eq!(p.url, format!("{}/proxy/vod/movie/uuid-m", server.uri()));
    assert_eq!(p.headers[0].0, "X-API-Key");
    assert_eq!(p.profile, downlink_core::source::PlayProfile::Seekable);
    let ep = VodItem {
        kind: Some(VodKind::Episode),
        uuid: Some("uuid-e".into()),
        ..Default::default()
    };
    assert!(
        c.play_vod(&ep)
            .await
            .unwrap()
            .url
            .ends_with("/proxy/vod/episode/uuid-e")
    );
    assert!(!format!("{p:?}").contains(KEY));
    assert!(
        c.play_vod(&VodItem {
            kind: Some(VodKind::Series),
            uuid: Some("x".into()),
            ..Default::default()
        })
        .await
        .is_err()
    );
    assert!(
        c.play_vod(&VodItem {
            kind: Some(VodKind::Movie),
            uuid: None,
            ..Default::default()
        })
        .await
        .is_err()
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "rule 4: building a VOD plan never requests it"
    );
}

#[tokio::test]
async fn endpoint_recordings_list_statuses_and_tolerance() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/recordings/",
        fx("recordings_full.json"),
    )
    .await;
    let r = client(&server).recordings().await.unwrap();
    assert_eq!(r.len(), 5, "an unreadable row is skipped");
    let by = |id: &str| r.iter().find(|x| x.ext_id == id).unwrap();
    assert_eq!(
        (by("31").status, by("31").title.as_str()),
        (RecStatus::Scheduled, "Future Show")
    );
    assert_eq!(by("32").status, RecStatus::Recording);
    assert_eq!(by("32").rule_label.as_deref(), Some("Series"));
    assert_eq!(
        by("32").file_url.as_deref(),
        Some("/api/channels/recordings/32/hls/index.m3u8")
    );
    assert_eq!(
        (by("33").status, by("33").size_bytes),
        (RecStatus::Completed, Some(123_456_789))
    );
    assert_eq!(
        (by("34").status, by("34").error.as_deref()),
        (RecStatus::Failed, Some("Provider refused the connection."))
    );
    assert_eq!(
        (by("35").status, by("35").title.as_str()),
        (RecStatus::Completed, "Recording"),
        "no status: derived from times"
    );
}

#[tokio::test]
async fn endpoint_schedule_one_off_and_series_rule() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/channels/recordings/")).and(header("X-API-Key", KEY))
        .and(body_partial_json(json!({"channel": 3, "start_time": "2026-01-01T20:00:00Z", "end_time": "2026-01-01T21:00:00Z",
            "custom_properties": {"program": {"title": "Fieldwork", "tvg_id": "one.test"}}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 9}))).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/channels/series-rules/"))
        .and(body_partial_json(json!({"tvg_id": "one.test", "mode": "new", "title": "Fieldwork", "title_mode": "exact", "channel_id": 3})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true}))).expect(1).mount(&server).await;
    let req = RecordRequest {
        channel_ext_id: "3".into(),
        tvg_id: Some("one.test".into()),
        programme_ext_id: Some("p".into()),
        title: "Fieldwork".into(),
        description: None,
        start_utc: 1_767_297_600,
        end_utc: 1_767_301_200,
    };
    let c = client(&server);
    c.schedule_recording(&req).await.unwrap();
    c.schedule_series_rule(&req).await.unwrap();
    // a channel without a guide id cannot get a series rule; a non-numeric channel cannot be recorded
    assert!(
        c.schedule_series_rule(&RecordRequest {
            tvg_id: None,
            ..req.clone()
        })
        .await
        .is_err()
    );
    assert!(
        c.schedule_recording(&RecordRequest {
            channel_ext_id: "x".into(),
            ..req
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn endpoints_stop_extend_delete_recording() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/32/stop/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 32})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/32/extend/"))
        .and(body_partial_json(json!({"extra_minutes": 15})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 32})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/recordings/31/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/recordings/99/"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.stop_recording("32").await.unwrap();
    c.extend_recording("32", 15).await.unwrap();
    c.delete_recording("31").await.unwrap();
    c.delete_recording("99").await.unwrap(); // already gone
    assert_eq!(
        c.extend_recording("32", 0).await.unwrap_err().code,
        ErrorCode::Local
    );
    assert!(c.extend_recording("32", 100_000).await.is_err());
}

#[tokio::test]
async fn recording_plans_hls_while_running_file_when_done_same_origin_only() {
    let server = MockServer::start().await;
    let c = client(&server);
    let mk = |id: &str, status, file: Option<&str>| downlink_core::source::RecordingInfo {
        ext_id: id.into(),
        channel_ext_id: "1".into(),
        title: "T".into(),
        start_utc: 0,
        end_utc: 1,
        status,
        rule_label: None,
        size_bytes: None,
        error: None,
        file_url: file.map(Into::into),
        ..Default::default()
    };
    let live = c
        .recording_plan(&mk(
            "32",
            RecStatus::Recording,
            Some("/api/channels/recordings/32/hls/index.m3u8"),
        ))
        .await
        .unwrap();
    assert_eq!(
        live.url,
        format!("{}/api/channels/recordings/32/hls/index.m3u8", server.uri())
    );
    assert_eq!(live.surface, downlink_core::source::Surface::Hls);
    assert_eq!(live.headers[0].0, "X-API-Key");
    let running_nofile = c
        .recording_plan(&mk("32", RecStatus::Recording, None))
        .await
        .unwrap();
    assert!(
        running_nofile
            .url
            .ends_with("/recordings/32/hls/index.m3u8")
    );
    let done = c
        .recording_plan(&mk("33", RecStatus::Completed, None))
        .await
        .unwrap();
    assert_eq!(
        done.url,
        format!("{}/api/channels/recordings/33/file/", server.uri())
    );
    assert_eq!(done.surface, downlink_core::source::Surface::Direct);
    assert!(
        c.recording_plan(&mk(
            "34",
            RecStatus::Completed,
            Some("http://elsewhere.test/f.mkv")
        ))
        .await
        .is_err(),
        "the key never goes to another server"
    );
    assert!(!format!("{live:?}").contains(KEY));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn endpoints_rules_list_and_delete() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/recurring-rules/",
        fx("recurring_rules_full.json"),
    )
    .await;
    get(
        &server,
        "/api/channels/series-rules/",
        fx("series_rules_full.json"),
    )
    .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/recurring-rules/7/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/series-rules/"))
        .and(query_param("tvg_id", "one.test"))
        .and(query_param("title", "Fieldwork"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let rules = c.recording_rules().await.unwrap();
    assert_eq!(rules.len(), 3);
    let rec = rules
        .iter()
        .find(|r| r.kind == RuleKind::Recurring)
        .unwrap();
    assert_eq!(
        (
            rec.title.as_str(),
            rec.schedule.as_str(),
            rec.channel_ext_id.as_deref()
        ),
        ("Evening News", "Weekdays 19:00", Some("2"))
    );
    let ser = rules.iter().find(|r| r.title == "Fieldwork").unwrap();
    assert_eq!(
        (ser.kind, ser.schedule.as_str(), ser.ext_id.as_str()),
        (RuleKind::Series, "New episodes", "one.test|Fieldwork")
    );
    assert_eq!(
        rules
            .iter()
            .find(|r| r.title == "Quiz Hour")
            .unwrap()
            .schedule,
        "All episodes"
    );
    c.delete_rule(rec).await.unwrap();
    c.delete_rule(ser).await.unwrap();
}

#[tokio::test]
async fn caps_vod_and_dvr_follow_the_account() {
    use downlink_core::source::Caps;
    let server = MockServer::start().await;
    get(&server, "/api/core/version/", json!({"version": "0.34.1"})).await;
    get(
        &server,
        "/api/accounts/users/me/",
        json!({"id": 1, "username": "u", "user_level": 10}),
    )
    .await;
    get(&server, "/api/core/settings/", json!([])).await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 0, "results": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/vod/series/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 4, "results": []})))
        .mount(&server)
        .await;
    let c = client(&server);
    assert!(
        !c.caps().contains(Caps::VOD) && !c.caps().contains(Caps::DVR),
        "nothing advertised before connect"
    );
    c.connect().await.unwrap();
    assert!(
        c.caps().contains(Caps::VOD | Caps::DVR),
        "series exist, admin can record"
    );

    // A Streamer on a server with no VOD content gets neither.
    let s2 = MockServer::start().await;
    get(&s2, "/api/core/version/", json!({"version": "0.34.1"})).await;
    get(
        &s2,
        "/api/accounts/users/me/",
        json!({"id": 2, "username": "s", "user_level": 0}),
    )
    .await;
    get(&s2, "/api/core/settings/", json!([])).await;
    Mock::given(method("GET"))
        .and(path_regex_vod())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"count": 0, "results": []})))
        .mount(&s2)
        .await;
    let c2 = client(&s2);
    c2.connect().await.unwrap();
    assert!(!c2.caps().contains(Caps::VOD) && !c2.caps().contains(Caps::DVR));
}

fn path_regex_vod() -> wiremock::matchers::PathRegexMatcher {
    wiremock::matchers::path_regex(r"^/api/vod/(movies|series)/$")
}

/// Shape observed on a real 0.34.1 server (scrubbed): bare array, padded recording window,
/// programme times and title under `custom_properties.program`, size in `bytes_written`.
#[tokio::test]
async fn reference_server_completed_recording_shape() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/recordings/",
        fx("recordings_completed_synthetic.json"),
    )
    .await;
    let r = client(&server).recordings().await.unwrap();
    assert_eq!(r.len(), 1);
    let rec = &r[0];
    assert_eq!(rec.title, "Synthetic Race Coverage");
    // Displayed times are the programme's (04:30-06:30Z), not the padded 04:24-06:36.
    assert_eq!(rec.start_utc, 1_794_112_200);
    assert_eq!(rec.end_utc, 1_794_119_400);
    assert_eq!(rec.status, RecStatus::Completed);
    assert_eq!(rec.size_bytes, Some(123_456_789));
    assert_eq!(
        rec.file_url.as_deref(),
        Some("/api/channels/recordings/19/file/")
    );
    assert_eq!(rec.channel_ext_id, "3");
    let plan = client(&server).recording_plan(rec).await.unwrap();
    assert_eq!(
        plan.url,
        format!("{}/api/channels/recordings/19/file/", server.uri())
    );
}

#[tokio::test]
async fn reference_server_empty_rule_lists() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/series-rules/",
        fx("series_rules_empty.json"),
    )
    .await;
    get(&server, "/api/channels/recurring-rules/", json!([])).await;
    assert!(
        client(&server).recording_rules().await.unwrap().is_empty(),
        "{{\"rules\": []}} envelope and bare [] both tolerated"
    );
    // A padded recording with no status derives it from the padded window, not the programme's.
    let server = MockServer::start().await;
    let now = downlink_core::sync::now_utc();
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    get(&server, "/api/channels/recordings/", json!([{
        "id": 5, "channel": 1, "start_time": iso(now - 120), "end_time": iso(now + 3600),
        "custom_properties": {"program": {"title": "Padded", "start_time": iso(now + 200), "end_time": iso(now + 3400)}}
    }])).await;
    let r = client(&server).recordings().await.unwrap();
    assert_eq!(
        r[0].status,
        RecStatus::Recording,
        "pre-roll padding has started even though the programme has not"
    );
    assert_eq!(r[0].start_utc, now + 200);
}

// ---- provider credentials in the episodes payload (reference-server shape) ----------------------

const FAKE_PASS: &str = "FAKEPROVIDERPASS-7731";
const FAKE_USER: &str = "fakeprovideruser";

#[tokio::test]
async fn episodes_with_provider_account_blobs_never_expose_the_credentials() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/vod/series/21/episodes/",
        fx("vod_episodes_reference_shape.json"),
    )
    .await;
    let raw = serde_json::to_string(&fx("vod_episodes_reference_shape.json")).unwrap();
    assert!(
        raw.contains(FAKE_PASS),
        "the fixture really carries the fake provider password"
    );

    // Capture everything logged while fetching (the core logs counts only).
    let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    #[derive(Clone)]
    struct W(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for W {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let w = W(sink.clone());
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || w.clone())
        .finish();
    let _g = tracing::subscriber::set_default(sub);

    let eps = client(&server).series_episodes("21").await.unwrap();
    assert_eq!(eps.len(), 3);
    assert_eq!(eps[0].title, "Pilot");
    assert_eq!(
        (eps[0].season, eps[0].episode, eps[0].duration_secs),
        (Some(1), Some(1), Some(2700))
    );
    assert_eq!(
        eps[0].uuid.as_deref(),
        Some("33333333-0000-4000-8000-000000000301")
    );
    assert_eq!(eps[0].rating.as_deref(), Some("7.5"));
    assert_eq!(eps[0].parent_ext.as_deref(), Some("21"));
    // Nothing from the provider blob survives into our types, not even through Debug.
    let shown = format!("{eps:?}");
    for secret in [FAKE_PASS, FAKE_USER, "provider.example.test"] {
        assert!(
            !shown.contains(secret),
            "{secret} leaked into the parsed items: {shown}"
        );
    }
    assert!(
        eps.iter().all(|e| e.container.is_none()),
        "provider container/direct_source are not read"
    );
    let logs = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
    assert!(
        !logs.contains(FAKE_PASS) && !logs.contains(FAKE_USER),
        "{logs}"
    );
}

#[tokio::test]
async fn a_broken_episodes_body_yields_an_error_without_any_body_text() {
    let server = MockServer::start().await;
    let full = serde_json::to_string(&fx("vod_episodes_reference_shape.json")).unwrap();
    // Truncated mid-way inside a provider blob, and a body that is valid JSON of the wrong shape.
    let cut = &full[..full.find("FAKEPROVIDERPASS").unwrap() + 8];
    for body in [
        cut.to_owned(),
        format!("{{\"results\": \"{FAKE_PASS}\"}}"),
        format!("[{{\"id\": {{\"password\": \"{FAKE_PASS}\"}}, \"providers\": [}}"),
    ] {
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/vod/series/21/episodes/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        let r = client(&server).series_episodes("21").await;
        if let Err(e) = r {
            let shown = format!("{e:?} {e}");
            assert!(
                !shown.contains(FAKE_PASS) && !shown.contains(FAKE_USER),
                "{shown}"
            );
            assert_eq!(e.code, ErrorCode::Server);
        }
    }
    // A 500 whose body quotes credentials: the error never includes the body.
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(500).set_body_string(format!("boom {FAKE_USER}:{FAKE_PASS}")),
        )
        .mount(&server)
        .await;
    let e = client(&server).series_episodes("21").await.unwrap_err();
    assert!(!format!("{e:?}").contains(FAKE_PASS));
}

#[tokio::test]
async fn series_list_art_prefers_the_cache_url_then_the_external_url() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/vod/series/",
        fx("vod_series_reference_shape.json"),
    )
    .await;
    let page = client(&server)
        .vod_list(
            VodKind::Series,
            &VodListQuery {
                offset: 0,
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let a = &page.items[0];
    assert_eq!(
        a.poster.as_deref(),
        Some(format!("{}/api/vod/vodlogos/6/cache/", server.uri()).as_str())
    );
    assert_eq!(
        a.year,
        Some(2022),
        "year from custom_properties.releaseDate when `year` is null"
    );
    assert_eq!(
        a.backdrop.as_deref(),
        Some("http://img.example.test/fw-bd.jpg")
    );
    assert_eq!(
        page.items[1].poster.as_deref(),
        Some("http://img.example.test/ext.jpg"),
        "empty cache_url falls back to the external logo URL"
    );
}

#[tokio::test]
async fn categories_are_a_bare_array_even_with_page_params() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/vod/categories/"))
        .and(query_param("category_type", "series"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": 2, "name": "Drama", "category_type": "series", "category_type_display": "Series",
             "m3u_accounts": [{"category": 2, "m3u_account": 1, "enabled": true}]}
        ])))
        .mount(&server)
        .await;
    let cats = client(&server)
        .vod_category_list(VodKind::Series)
        .await
        .unwrap();
    assert_eq!(cats[0].name, "Drama");
}

// ---- canonical fixtures captured from a local 0.34.1 server (tools/fixtures/dispatcharr) --------

fn tfx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/dispatcharr")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[tokio::test]
async fn captured_recordings_list_states_rules_and_tags() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/recordings/",
        tfx("recordings_list.json"),
    )
    .await;
    let r = client(&server).recordings().await.unwrap();
    let by = |id: &str| r.iter().find(|x| x.ext_id == id).unwrap();
    // Recurring-rule recordings: tagged with the rule, status "scheduled", programme times shown.
    let rec = by("11");
    assert_eq!(
        (
            rec.status,
            rec.rule_label.as_deref(),
            rec.rule_ext.as_deref()
        ),
        (RecStatus::Scheduled, Some("Recurring"), Some("1"))
    );
    assert_eq!(rec.title, "Synthetic Weekday Rule");
    // Series-evaluation recordings carry no rule tag but do carry title + tvg_id for matching.
    assert_eq!(by("26").rule_label, None);
    assert_eq!(
        (by("26").tvg_id.as_deref(), by("26").title.as_str()),
        (Some("test-news.test"), "Pattern Playhouse")
    );
    // `stopped` = finished with a partial file; it plays via /file/.
    assert_eq!(
        (by("29").status, by("29").file_url.as_deref()),
        (
            RecStatus::Completed,
            Some("/api/channels/recordings/29/file/")
        )
    );
    assert_eq!(by("4").status, RecStatus::Completed);
}

#[tokio::test]
async fn absent_status_is_derived_from_the_window_and_odd_statuses_are_mapped() {
    let server = MockServer::start().await;
    let now = downlink_core::sync::now_utc();
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let row = |id: i64, s: i64, e: i64, status: Option<&str>| {
        let mut cp = json!({"program": {"title": format!("R{id}")}});
        if let Some(st) = status {
            cp["status"] = json!(st);
        }
        json!({"id": id, "channel": 1, "start_time": iso(s), "end_time": iso(e), "custom_properties": cp})
    };
    get(
        &server,
        "/api/channels/recordings/",
        json!([
            row(1, now + 3600, now + 7200, None), // future: scheduled
            row(2, now - 600, now + 600, None),   // inside the window: recording
            row(3, now - 7200, now - 3600, None), // past: completed
            row(4, now - 7200, now - 3600, Some("interrupted")),
            row(5, now + 3600, now + 7200, Some("cancelled")),
            row(6, now - 7200, now - 3600, Some("stopped")),
        ]),
    )
    .await;
    let r = client(&server).recordings().await.unwrap();
    let st = |id: &str| r.iter().find(|x| x.ext_id == id).map(|x| x.status);
    assert_eq!(
        (st("1"), st("2"), st("3")),
        (
            Some(RecStatus::Scheduled),
            Some(RecStatus::Recording),
            Some(RecStatus::Completed)
        )
    );
    assert_eq!(st("4"), Some(RecStatus::Failed));
    assert_eq!(
        r.iter().find(|x| x.ext_id == "4").unwrap().error.as_deref(),
        Some("The recording was interrupted.")
    );
    assert_eq!(st("5"), None, "cancelled recordings are hidden");
    assert_eq!(
        st("6"),
        Some(RecStatus::Completed),
        "stopped = completed with a partial file"
    );
}

#[tokio::test]
async fn extend_uses_extra_minutes_and_maps_the_finished_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/31/extend/"))
        .and(body_partial_json(json!({"extra_minutes": 15})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(tfx("recording_extend_in_progress.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/29/extend/"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(tfx("recording_extend_finished_error.json")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    c.extend_recording("31", 15).await.unwrap();
    let e = c.extend_recording("29", 15).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Stream);
    assert_eq!(e.message, "That recording has already finished.");
}

#[tokio::test]
async fn a_past_one_off_gets_a_plain_sentence() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/"))
        .respond_with(ResponseTemplate::new(400).set_body_json(tfx("recording_past_error.json")))
        .mount(&server)
        .await;
    let req = RecordRequest {
        channel_ext_id: "1".into(),
        tvg_id: None,
        programme_ext_id: None,
        title: "T".into(),
        description: None,
        start_utc: 1,
        end_utc: 2,
    };
    let e = client(&server).schedule_recording(&req).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Stream);
    assert!(
        e.message.contains("already finished") && !e.message.contains("non_field"),
        "{}",
        e.message
    );
}

#[tokio::test]
async fn series_rule_create_is_followed_by_an_evaluate() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/channels/series-rules/"))
        .and(body_partial_json(json!({"tvg_id": "test-news.test", "title": "Pattern Playhouse", "mode": "new", "title_mode": "exact"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(tfx("series_rule_create.json"))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/evaluate/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"success": true, "scheduled": 5, "details": []})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let req = RecordRequest {
        channel_ext_id: "2".into(),
        tvg_id: Some("test-news.test".into()),
        programme_ext_id: None,
        title: "Pattern Playhouse".into(),
        description: None,
        start_utc: 1,
        end_utc: 2,
    };
    client(&server).schedule_series_rule(&req).await.unwrap();
}

#[tokio::test]
async fn captured_rules_and_weekday_labels() {
    let server = MockServer::start().await;
    get(
        &server,
        "/api/channels/recurring-rules/",
        tfx("recurring_rules.json"),
    )
    .await;
    get(
        &server,
        "/api/channels/series-rules/",
        tfx("series_rules.json"),
    )
    .await;
    let rules = client(&server).recording_rules().await.unwrap();
    let rec = rules
        .iter()
        .find(|r| r.kind == RuleKind::Recurring)
        .unwrap();
    assert_eq!(
        rec.schedule, "Mon, Wed, Fri 20:00",
        "0 = Monday (Python weekday)"
    );
    assert_eq!(rec.ext_id, "1");
}

#[tokio::test]
async fn rating_as_number_custom_properties_as_string_and_server_built_urls() {
    let server = MockServer::start().await;
    let mut movies = tfx("vod_movies.json");
    // /api/vod/all/ style: numeric rating and a JSON *string* in custom_properties.
    movies["results"][0]["rating"] = json!(7.5);
    movies["results"][0]["custom_properties"] =
        json!("{\"backdrop_path\": [\"http://streams.dispatcharr.test/img/backdrop.png\"]}");
    get(&server, "/api/vod/movies/", movies).await;
    let page = client(&server)
        .vod_list(
            VodKind::Movie,
            &VodListQuery {
                offset: 0,
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m = &page.items[0];
    assert_eq!(m.rating.as_deref(), Some("7.5"));
    assert_eq!(
        m.backdrop.as_deref(),
        Some("http://streams.dispatcharr.test/img/backdrop.png"),
        "string-encoded custom_properties parsed"
    );
    assert!(
        m.poster
            .as_deref()
            .unwrap()
            .starts_with(&format!("{}/api/vod/vodlogos/", server.uri())),
        "an absolute cache_url built from the request Host is rewritten onto our base: {:?}",
        m.poster
    );
    assert!(
        m.poster.as_deref().unwrap().contains("?v="),
        "the cache-busting query is kept"
    );
}

#[tokio::test]
async fn image_urls_keep_their_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/3/image/"))
        .and(query_param("kind", "backdrop"))
        .and(query_param("index", "0"))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![7u8]))
        .expect(1)
        .mount(&server)
        .await;
    let url = format!(
        "{}/api/vod/movies/3/image/?kind=backdrop&index=0&v=abc",
        server.uri()
    );
    assert_eq!(client(&server).fetch_image(&url).await.unwrap().bytes, [7]);
}

#[tokio::test]
async fn provider_info_is_typed_and_never_exposes_the_account() {
    let server = MockServer::start().await;
    let mut info = tfx("vod_movie_provider_info.json");
    info["m3u_account"] = json!({"id": 3, "name": "X", "account_type": "XC", "password": "FAKEPROVIDERPASS-7731", "username": "fakeprovideruser"});
    get(&server, "/api/vod/movies/3/provider-info/", info).await;
    let item = client(&server)
        .vod_enrich(VodKind::Movie, "3")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        item.plot.as_deref(),
        Some("Synthetic plot for Test Documentary Three.")
    );
    assert_eq!(
        (item.year, item.genre.as_deref(), item.duration_secs),
        (Some(2019), Some("Test"), Some(20))
    );
    assert!(
        item.backdrop.as_deref().unwrap().starts_with(&server.uri()),
        "image URL rewritten onto our base"
    );
    assert!(!format!("{item:?}").contains("FAKEPROVIDERPASS"));
    // 404 means "nothing to add", not an error.
    assert!(
        client(&server)
            .vod_enrich(VodKind::Movie, "999")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn series_rule_carries_the_epg_source_of_the_channels_guide_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/2/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2, "epg_data_id": 41, "effective_epg_data_id": 41
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/epg/epgdata/41/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": 41, "tvg_id": "test-news.test", "epg_source": 7})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/"))
        .and(body_partial_json(
            json!({"tvg_id": "test-news.test", "epg_source_id": 7, "channel_id": 2}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(tfx("series_rule_create.json")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/evaluate/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&server)
        .await;
    let req = RecordRequest {
        channel_ext_id: "2".into(),
        tvg_id: Some("test-news.test".into()),
        programme_ext_id: None,
        title: "Pattern Playhouse".into(),
        description: None,
        start_utc: 1,
        end_utc: 2,
    };
    client(&server).schedule_series_rule(&req).await.unwrap();
}

#[tokio::test]
async fn series_rule_omits_epg_source_when_it_is_unknown() {
    let server = MockServer::start().await;
    // No channel or guide-data endpoints mounted: the lookup fails and the rule is still created.
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tfx("series_rule_create.json")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/evaluate/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&server)
        .await;
    let req = RecordRequest {
        channel_ext_id: "2".into(),
        tvg_id: Some("test-news.test".into()),
        programme_ext_id: None,
        title: "Pattern Playhouse".into(),
        description: None,
        start_utc: 1,
        end_utc: 2,
    };
    client(&server).schedule_series_rule(&req).await.unwrap();
    let posted = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.method.as_str() == "POST" && r.url.path() == "/api/channels/series-rules/")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&posted.body).unwrap();
    assert!(body.get("epg_source_id").is_none(), "{body}");
}
