#![allow(clippy::unwrap_used, clippy::expect_used)]
//! PLAN §5.4: the same programme row played through a Dispatcharr session and through an Xtream
//! `timeshift` template starts at the same instant and runs the same length.

use std::path::PathBuf;

use chrono::TimeZone;
use downlink_core::auth::SecretString;
use downlink_core::source::dispatcharr::{ClientOptions, Credentials, DispatcharrClient};
use downlink_core::source::xtream::XtreamClient;
use downlink_core::source::{ChannelKey, Programme, Source, SourceId, StreamMode};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UUID1: &str = "00000000-0000-4000-8000-000000000001";

#[tokio::test]
async fn one_programme_row_gives_the_same_start_and_length_on_both_paths() {
    // 2026-07-01 12:15:00Z, 95 minutes: not on an hour, so rounding and zones both show.
    let start = 1_782_908_100;
    let prog = Programme {
        start_utc: start,
        end_utc: start + 95 * 60,
        ..Default::default()
    };

    // Dispatcharr: the session request carries the start (ISO, UTC) and the length in minutes.
    let d = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/catchup/sessions/"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"session_id": "s1", "playback_url": "/proxy/catchup/x?session_id=s1"}),
        ))
        .mount(&d)
        .await;
    let dc = DispatcharrClient::new(
        SourceId(1),
        &d.uri(),
        Credentials::ApiKey(SecretString::new("sk_test_equivalence")),
        ClientOptions::default(),
    )
    .unwrap();
    dc.catchup(
        ChannelKey {
            ext_id: "1".into(),
            uuid: Some(UUID1.into()),
            stream_mode: StreamMode::Proxy,
            ..Default::default()
        },
        &prog,
    )
    .await
    .unwrap();
    let body: Value =
        serde_json::from_slice(&d.received_requests().await.unwrap()[0].body).unwrap();
    let d_start = chrono::DateTime::parse_from_rfc3339(body["start"].as_str().unwrap())
        .unwrap()
        .timestamp();
    let d_minutes = body["duration"].as_i64().unwrap();

    // Xtream: the template carries the start as wall-clock time in the server's zone.
    let x = MockServer::start().await;
    let login: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/fixtures/xtream/player_api_login.json"),
        )
        .unwrap(),
    )
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(query_param("username", "testuser"))
        .respond_with(ResponseTemplate::new(200).set_body_json(login))
        .mount(&x)
        .await;
    let xc = XtreamClient::new(
        SourceId(2),
        &x.uri(),
        "testuser",
        SecretString::new("testpass_secret"),
        None,
    )
    .unwrap();
    xc.connect().await.unwrap(); // learns the server zone (Europe/London in the fixture)
    let plan = xc
        .catchup(
            ChannelKey {
                ext_id: "1002".into(),
                ..Default::default()
            },
            &prog,
        )
        .await
        .unwrap();
    // …/timeshift/<user>/<pass>/<minutes>/<YYYY-MM-DD:HH-MM>/<id>.ts
    let parts: Vec<&str> = plan.url.rsplit('/').collect();
    let (when, minutes) = (parts[1], parts[2]);
    let naive = chrono::NaiveDateTime::parse_from_str(when, "%Y-%m-%d:%H-%M").unwrap();
    let x_start = chrono_tz::Europe::London
        .from_local_datetime(&naive)
        .unwrap()
        .timestamp();

    assert_eq!(
        d_start, start,
        "Dispatcharr session start is the row's start"
    );
    assert_eq!(x_start, start, "Xtream template start is the row's start");
    assert_eq!(d_minutes, 95);
    assert_eq!(minutes.parse::<i64>().unwrap(), d_minutes, "same length");
    assert_eq!(d_start, x_start);
}
