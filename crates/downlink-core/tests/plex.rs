#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Plex client against a mock server: library index, guid / title matching, episodes, plex.tv
//! sign-in and server discovery. Synthetic titles and ids only.

use downlink_core::auth::SecretString;
use downlink_core::mediaserver::plex::{PlexServer, PlexTv, pick_connection};
use downlink_core::mediaserver::{MatchKind, MediaQuery, QueryKind};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "plex_test_token_abcdef";
const MACHINE: &str = "machine0001";

async fn library() -> MockServer {
    let s = MockServer::start().await;
    let get = |p: &str, body: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path(p.to_owned()))
            .and(header("X-Plex-Token", TOKEN))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
    };
    get(
        "/library/sections",
        json!({"MediaContainer": {"Directory": [
            {"key": "1", "type": "movie", "title": "Films"},
            {"key": "2", "type": "show", "title": "TV"},
            {"key": "3", "type": "artist", "title": "Music"}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/sections/1/all",
        json!({"MediaContainer": {"totalSize": 3, "Metadata": [
            {"ratingKey": "101", "title": "Sample Film", "year": 2020,
             "Guid": [{"id": "tmdb://5550001"}, {"id": "imdb://tt5550001"}]},
            {"ratingKey": "102", "title": "Twin Title", "year": 1999, "Guid": []},
            {"ratingKey": "103", "title": "Twin Title", "year": 2019, "Guid": []}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/sections/2/all",
        json!({"MediaContainer": {"totalSize": 1, "Metadata": [
            {"ratingKey": "201", "title": "Example Show", "year": 2018,
             "Guid": [{"id": "tmdb://7770001"}, {"id": "tvdb://8880001"}]}
        ]}}),
    )
    .mount(&s)
    .await;
    get(
        "/library/metadata/201/allLeaves",
        json!({"MediaContainer": {"Metadata": [
            {"ratingKey": "301", "parentIndex": 1, "index": 1},
            {"ratingKey": "302", "parentIndex": 1, "index": 2}
        ]}}),
    )
    .mount(&s)
    .await;
    s
}

fn server(s: &MockServer) -> PlexServer {
    PlexServer::new(&s.uri(), SecretString::new(TOKEN), MACHINE, "client-1").unwrap()
}

fn movie(title: &str) -> MediaQuery {
    MediaQuery {
        kind: QueryKind::Movie,
        title: title.into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn movie_matches_on_tmdb_guid_whatever_the_title() {
    let s = library().await;
    let p = server(&s);
    let m = p
        .find(&MediaQuery {
            tmdb_id: Some("5550001".into()),
            ..movie("4K: Something Else Entirely (2020)")
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m.kind, MatchKind::Movie);
    assert_eq!(m.title, "Sample Film");
    assert_eq!(
        m.url,
        format!(
            "https://app.plex.tv/desktop/#!/server/{MACHINE}/details?key=%2Flibrary%2Fmetadata%2F101"
        )
    );
    let by_imdb = p
        .find(&MediaQuery {
            imdb_id: Some("tt5550001".into()),
            ..movie("x")
        })
        .await
        .unwrap();
    assert!(by_imdb.is_some());
}

#[tokio::test]
async fn movie_title_match_strips_decoration_and_needs_a_unique_hit() {
    let s = library().await;
    let p = server(&s);
    assert!(
        p.find(&movie("EN - Sample Film (2020)"))
            .await
            .unwrap()
            .is_some()
    );
    // Two films share the title: only the year decides.
    assert!(p.find(&movie("Twin Title")).await.unwrap().is_none());
    let m = p.find(&movie("Twin Title (2019)")).await.unwrap().unwrap();
    assert!(m.url.ends_with("%2F103"), "{}", m.url);
    // Show guids never match a movie query (tmdb ids are per type).
    let none = p
        .find(&MediaQuery {
            tmdb_id: Some("7770001".into()),
            ..movie("nothing")
        })
        .await
        .unwrap();
    assert!(none.is_none());
}

#[tokio::test]
async fn episode_resolves_through_the_show() {
    let s = library().await;
    let p = server(&s);
    let q = MediaQuery {
        kind: QueryKind::Episode,
        title: "Example Show".into(),
        tmdb_id: Some("7770001".into()),
        season: Some(1),
        episode: Some(2),
        ..Default::default()
    };
    let m = p.find(&q).await.unwrap().unwrap();
    assert_eq!(m.kind, MatchKind::Episode);
    assert!(m.url.ends_with("%2F302"), "{}", m.url);
    // Missing episode: the show.
    let m = p
        .find(&MediaQuery {
            episode: Some(9),
            ..q
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m.kind, MatchKind::Show);
    assert!(m.url.ends_with("%2F201"), "{}", m.url);
}

#[tokio::test]
async fn live_titles_try_movies_then_shows() {
    let s = library().await;
    let p = server(&s);
    let show = p
        .find(&MediaQuery {
            title: "Example Show".into(),
            season: Some(1),
            episode: Some(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(show.kind, MatchKind::Episode);
    let film = p
        .find(&MediaQuery {
            title: "Sample Film".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(film.kind, MatchKind::Movie);
    assert!(
        p.find(&MediaQuery {
            title: "Evening News".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn index_is_built_once_and_paged() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/library/sections"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"MediaContainer": {"Directory": [{"key": "1", "type": "movie"}]}}),
        ))
        .expect(1)
        .mount(&s)
        .await;
    for (start, key) in [("0", "1"), ("1", "2")] {
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("X-Plex-Container-Start", start))
            .and(query_param("includeGuids", "1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"MediaContainer": {
                    "totalSize": 2,
                    "Metadata": [{"ratingKey": key, "title": format!("Film {key}")}]
                }})),
            )
            .expect(1)
            .mount(&s)
            .await;
    }
    let p = server(&s);
    assert_eq!(p.index().await.unwrap().len(), 2);
    assert_eq!(p.index().await.unwrap().len(), 2);
}

#[tokio::test]
async fn unauthorised_errors_do_not_carry_the_token() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&s)
        .await;
    let e = server(&s).find(&movie("x")).await.unwrap_err();
    assert_eq!(e.code, downlink_core::ErrorCode::Auth);
    assert!(!e.message.contains(TOKEN));
}

#[tokio::test]
async fn pin_sign_in_and_server_discovery() {
    let tv = MockServer::start().await;
    let pms = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v2/pins"))
        .and(header("X-Plex-Client-Identifier", "client-1"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 42, "code": "abcd"})))
        .mount(&tv)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/pins/42"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": 42, "code": "abcd", "authToken": TOKEN})),
        )
        .mount(&tv)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/resources"))
        .and(header("X-Plex-Token", TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"name": "Living room", "provides": "client,player", "clientIdentifier": "tv1"},
            {"name": "Home server", "provides": "server", "clientIdentifier": MACHINE,
             "owned": true, "accessToken": "server_token_xyz",
             "connections": [
                {"uri": "http://203.0.113.9:1/", "local": false, "relay": true},
                {"uri": pms.uri(), "local": true, "relay": false}
             ]}
        ])))
        .mount(&tv)
        .await;
    Mock::given(method("GET"))
        .and(path("/identity"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"MediaContainer": {"machineIdentifier": MACHINE}})),
        )
        .mount(&pms)
        .await;

    let api = PlexTv::new(&tv.uri(), "client-1").unwrap();
    let pin = api.create_pin().await.unwrap();
    assert_eq!(pin.code, "abcd");
    let url = api.auth_url(&pin.code);
    assert!(
        url.starts_with("https://app.plex.tv/auth#?clientID=client-1&code=abcd"),
        "{url}"
    );
    let token = api.check_pin(pin.id).await.unwrap().unwrap();
    let servers = api.servers(&token).await.unwrap();
    assert_eq!(servers.len(), 1);
    let srv = &servers[0];
    assert_eq!(srv.id, MACHINE);
    let tok = srv.access_token.clone().unwrap();
    assert_eq!(
        pick_connection(srv, &tok).await.as_deref(),
        Some(pms.uri().as_str())
    );
}
