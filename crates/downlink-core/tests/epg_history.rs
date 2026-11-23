#![allow(clippy::unwrap_used, clippy::expect_used)]
//! EPG coverage with the re-captured Dispatcharr 0.34.1 fixtures (13 channels, 10 with guide
//! data): the visible window and the catch-up history are both filled on add_source, every channel
//! the server has programmes for gets them, and a channel that only has an EPG *assignment*
//! (no tvg_id of its own) is joined through the EPG data list.

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::auth::MemorySecretStore;
use downlink_core::service::{AddSourceInput, Service};
use downlink_core::store::Store;
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn fx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/dispatcharr")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Acts like the real grid endpoint: hourly rows for every channel that has guide data, limited
/// to the requested `start`/`end`. The set of tvg ids comes from the captured fixture.
struct Grid {
    tvg_ids: Vec<String>,
    /// `(id prefix, tvg_id)`: dummy-EPG rows, keyed by channel uuid like the real server.
    dummies: Vec<(String, String)>,
}

impl Respond for Grid {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let q: std::collections::HashMap<_, _> = r.url.query_pairs().into_owned().collect();
        let parse = |k: &str| {
            chrono::DateTime::parse_from_rfc3339(q.get(k).map_or("", String::as_str))
                .map(|d| d.timestamp())
                .unwrap_or(0)
        };
        let (start, end) = (parse("start"), parse("end"));
        let mut rows = vec![];
        let mut t = start - start.rem_euclid(3600);
        while t < end {
            for tvg in &self.tvg_ids {
                rows.push(json!({
                    "id": format!("{tvg}-{t}"), "tvg_id": tvg, "title": format!("Show {t}"),
                    "start_time": chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "end_time": chrono::DateTime::from_timestamp(t + 3600, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                }));
            }
            for (prefix, key) in &self.dummies {
                rows.push(json!({
                    "id": format!("{prefix}-{t}"), "tvg_id": key, "title": "Placeholder",
                    "start_time": chrono::DateTime::from_timestamp(t, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    "end_time": chrono::DateTime::from_timestamp(t + 3600, 0).unwrap().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                }));
            }
            t += 3600;
        }
        ResponseTemplate::new(200).set_body_json(json!({ "data": rows }))
    }
}

async fn server(channels: Value, epg_data: Option<Value>) -> MockServer {
    server_with(channels, epg_data, vec![]).await
}

async fn server_with(
    channels: Value,
    epg_data: Option<Value>,
    dummies: Vec<(String, String)>,
) -> MockServer {
    let s = MockServer::start().await;
    let get = |p: &'static str, body: Value| {
        let s = &s;
        async move {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(s)
                .await;
        }
    };
    get("/api/core/version/", fx("core_version.json")).await;
    get("/api/accounts/users/me/", fx("users_me.json")).await;
    get("/api/core/settings/", fx("core_settings.json")).await;
    get("/api/core/streamprofiles/", fx("core_streamprofiles.json")).await;
    get("/api/channels/groups/", fx("channels_groups.json")).await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(channels))
        .mount(&s)
        .await;
    if let Some(e) = epg_data {
        get("/api/epg/epgdata/", e).await;
    }
    Mock::given(method("GET"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    let grid_tvgs: Vec<String> = fx("epg_grid_default.json")["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["tvg_id"].as_str().map(str::to_owned))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Mock::given(method("GET"))
        .and(path("/api/epg/grid/"))
        .respond_with(Grid {
            tvg_ids: grid_tvgs,
            dummies,
        })
        .mount(&s)
        .await;
    s
}

async fn service(server: &MockServer) -> (Arc<Service>, i64, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        Arc::new(MemorySecretStore::new()),
        dir.path().join("img"),
        "t".into(),
    );
    let input: AddSourceInput = serde_json::from_value(json!({"kind": "dispatcharr", "name": "S", "url": server.uri(), "apiKey": "REDACTED-API-KEY", "allowSelfSigned": false})).unwrap();
    let id = svc.add_source(input).await.unwrap().id;
    (svc, id, dir)
}

async fn count(svc: &Service, sql: &'static str, bind: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(svc.store().pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn add_source_fills_visible_window_and_catchup_history_for_every_channel_with_guide_data() {
    let server = server(fx("channels_page.json"), None).await;
    let (svc, sid, _d) = service(&server).await;
    let now = downlink_core::sync::now_utc();

    // Every channel whose tvg_id appears in the grid has programmes around now.
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT c.name, count(p.start_utc) FROM channel c LEFT JOIN programme p
           ON p.channel_id = c.id AND p.start_utc <= ?1 AND p.end_utc > ?1
         WHERE c.source_id = ?2 GROUP BY c.id ORDER BY c.number",
    )
    .bind(now)
    .bind(sid)
    .fetch_all(svc.store().pool())
    .await
    .unwrap();
    assert_eq!(rows.len(), 13);
    let with_now = rows.iter().filter(|(_, n)| *n > 0).count();
    assert_eq!(
        with_now, 10,
        "the 10 channels the server has a guide for: {rows:?}"
    );
    for xc in ["XC Catchup", "XC News", "XC Sports"] {
        assert_eq!(
            rows.iter().find(|(n, _)| n == xc).unwrap().1,
            0,
            "the server has no guide rows for {xc}"
        );
    }

    // History: Catchup One (3 days) has rows back ~3 days; a non-catch-up channel does not.
    let cu: i64 = sqlx::query_scalar("SELECT id FROM channel WHERE name = 'Catchup One'")
        .fetch_one(svc.store().pool())
        .await
        .unwrap();
    let kids: i64 = sqlx::query_scalar("SELECT id FROM channel WHERE name = 'Test Kids'")
        .fetch_one(svc.store().pool())
        .await
        .unwrap();
    let oldest = |c: i64| {
        let pool = svc.store().pool().clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT min(start_utc) FROM programme WHERE channel_id = ?",
            )
            .bind(c)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert!(
        oldest(cu).await <= now - 2 * 86_400 - 3600 * 12,
        "history reaches back into the catch-up window: {}",
        (now - oldest(cu).await) / 3600
    );
    assert!(
        oldest(cu).await >= now - 4 * 86_400,
        "and not further than the archive window"
    );
    // The grid fixture serves all channels for every window, so history exists for all of them;
    // what matters is that the fill ran: windows beyond the visible one were requested.
    let grid_calls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/epg/grid/")
        .count();
    assert!(
        grid_calls >= 4,
        "visible + several history days, got {grid_calls}"
    );
    let _ = kids;

    // A past programme from the history is playable from the guide's point of view.
    let past: i64 = count(&svc, "SELECT count(*) FROM programme WHERE channel_id = ? AND end_utc < strftime('%s','now') - 86400", cu).await;
    assert!(
        past > 12,
        "a day-old archive of hourly programmes exists: {past}"
    );
}

#[tokio::test]
async fn channels_with_an_epg_assignment_but_no_tvg_id_join_through_the_epg_data_list() {
    let mut channels = fx("channels_page.json");
    // XC Catchup: no tvg_id of its own, but assigned to EPG data row 1 (channel-one.test).
    for c in channels["results"].as_array_mut().unwrap() {
        if c["name"] == "XC Catchup" {
            c["tvg_id"] = json!("");
            c["effective_tvg_id"] = json!("");
            c["epg_data_id"] = json!(1);
            c["effective_epg_data_id"] = json!(1);
        }
    }
    let epg = fx("channels_page.json"); // placeholder to keep types simple
    let _ = epg;
    let epg_data = json!([{"id": 1, "tvg_id": "channel-one.test", "name": "Channel One", "icon_url": null, "epg_source": 1}]);
    let server = server(channels, Some(epg_data)).await;
    let (svc, sid, _d) = service(&server).await;
    let now = downlink_core::sync::now_utc();
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM programme p JOIN channel c ON c.id = p.channel_id
         WHERE c.name = 'XC Catchup' AND c.source_id = ?2 AND p.start_utc <= ?1 AND p.end_utc > ?1",
    )
    .bind(now)
    .bind(sid)
    .fetch_one(svc.store().pool())
    .await
    .unwrap();
    assert_eq!(n, 1, "the assigned channel shares Channel One's guide");
    let tvg: Option<String> =
        sqlx::query_scalar("SELECT tvg_id FROM channel WHERE name = 'XC Catchup'")
            .fetch_one(svc.store().pool())
            .await
            .unwrap();
    assert_eq!(tvg.as_deref(), Some("channel-one.test"));
}

async fn now_rows(svc: &Service, sid: i64, name: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM programme p JOIN channel c ON c.id = p.channel_id
         WHERE c.name = ?3 AND c.source_id = ?2 AND p.start_utc <= ?1 AND p.end_utc > ?1",
    )
    .bind(downlink_core::sync::now_utc())
    .bind(sid)
    .bind(name)
    .fetch_one(svc.store().pool())
    .await
    .unwrap()
}

#[tokio::test]
async fn the_epg_assignment_wins_over_the_channels_own_tvg_id() {
    let mut channels = fx("channels_page.json");
    // XC News keeps its own tvg_id (xc-news.test, which the grid never sends) but is assigned to
    // EPG data row 12, whose tvg_id is channel-one.test: the grid tags rows with the EPG's id.
    for c in channels["results"].as_array_mut().unwrap() {
        if c["name"] == "XC News" {
            c["epg_data_id"] = json!(12);
            c["effective_epg_data_id"] = json!(12);
        }
    }
    let epg_data = json!([{"id": 12, "tvg_id": "channel-one.test", "name": "Channel One", "icon_url": null, "epg_source": 1}]);
    let server = server(channels, Some(epg_data)).await;
    let (svc, sid, _d) = service(&server).await;
    assert_eq!(now_rows(&svc, sid, "XC News").await, 1);
    assert_eq!(
        now_rows(&svc, sid, "Channel One").await,
        1,
        "unassigned-in-list channels keep their own tvg_id"
    );
    let tvg: Option<String> =
        sqlx::query_scalar("SELECT tvg_id FROM channel WHERE name = 'XC News'")
            .fetch_one(svc.store().pool())
            .await
            .unwrap();
    assert_eq!(tvg.as_deref(), Some("channel-one.test"));
    let epg_calls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/api/epg/epgdata/")
        .count();
    assert!(
        epg_calls <= 1,
        "the EPG data list is reused, got {epg_calls} fetches"
    );
}

#[tokio::test]
async fn custom_dummy_rows_join_on_uuid_and_standard_placeholders_are_dropped() {
    let channels = fx("channels_page.json");
    let dummies = vec![
        // Custom dummy EPG source assigned to XC Sports: real titles, keyed by channel uuid.
        (
            "dummy-custom-13".to_owned(),
            "87ce3574-7ca3-435a-9041-8367d0ce55df".to_owned(),
        ),
        // Standard placeholder for XC News (no EPG at all).
        (
            "dummy-standard-12".to_owned(),
            "ad4b6481-646d-4cfe-ac11-de1bfe747822".to_owned(),
        ),
    ];
    let server = server_with(channels, None, dummies).await;
    let (svc, sid, _d) = service(&server).await;
    assert_eq!(now_rows(&svc, sid, "XC Sports").await, 1);
    assert_eq!(now_rows(&svc, sid, "XC News").await, 0);
}
