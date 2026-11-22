use serde_json::Value;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn fixture(name: &str) -> Value {
    let p = format!(
        "{}/../../tools/fixtures/tmdb/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn movie_is_normalised() {
    let m = parse_movie(&fixture("movie"), "US", "en").unwrap();
    assert_eq!(m.kind, Some(TmdbKind::Movie));
    assert_eq!(m.title, "The Paper Orchard");
    assert_eq!(m.original_title.as_deref(), Some("Le Verger de Papier"));
    assert_eq!(m.year, Some(2021));
    assert_eq!(m.runtime_mins, Some(112));
    assert_eq!(m.genres, vec!["Drama", "Adventure"]);
    // The US entry's first non-empty certification.
    assert_eq!(m.certification.as_deref(), Some("PG-13"));
    assert_eq!(m.score, Some(7.4));
    assert_eq!(m.cast[0].role.as_deref(), Some("Mireille"));
    assert_eq!(
        m.cast[0].photo.as_deref(),
        Some("https://image.tmdb.org/t/p/w185/ann.jpg")
    );
    assert_eq!(m.cast[1].photo, None);
    assert_eq!(m.directors, vec!["Rae Director"]);
    assert_eq!(m.writers, vec!["Wes Writer"]);
    // SVG logos cannot be decoded; the PNG one is picked.
    assert_eq!(
        m.logo.as_deref(),
        Some("https://image.tmdb.org/t/p/w500/logo501.png")
    );
    assert_eq!(m.trailer.as_deref(), Some("trailerKey1"));
    assert_eq!(m.imdb_id.as_deref(), Some("tt7654321"));
    // Region preference applies when present.
    let fr = parse_movie(&fixture("movie"), "FR", "fr").unwrap();
    assert_eq!(fr.certification.as_deref(), Some("U"));
}

#[test]
fn tv_is_normalised() {
    let t = parse_tv(&fixture("tv"), "GB", "en").unwrap();
    assert_eq!(t.kind, Some(TmdbKind::Tv));
    assert_eq!(t.original_title, None, "same as the title");
    assert_eq!(t.tagline, None, "empty strings are dropped");
    assert_eq!(t.certification.as_deref(), Some("15"));
    assert_eq!(t.runtime_mins, Some(44));
    assert_eq!(t.score, None, "two votes is noise");
    assert_eq!(t.backdrop, None);
    assert_eq!(t.networks, vec!["Synthetic One"]);
    assert_eq!(t.creators, vec!["Cy Creator"]);
    assert_eq!(t.cast[0].role.as_deref(), Some("Detective Vale"));
    assert_eq!(t.imdb_id.as_deref(), Some("tt1112223"));
    assert_eq!(t.season_count, Some(3));
}

#[test]
fn episode_is_normalised() {
    let e = parse_episode(&fixture("episode")).unwrap();
    assert_eq!((e.season, e.episode), (2, 5));
    assert_eq!(e.name.as_deref(), Some("The Lantern Keeper"));
    assert_eq!(
        e.still.as_deref(),
        Some("https://image.tmdb.org/t/p/w780/still2x5.jpg")
    );
    assert_eq!(e.guest_stars[0].name, "Bo Sample");
    assert_eq!(e.directors, vec!["Rae Director"]);
    assert_eq!(e.writers, vec!["Wes Writer"]);
}

#[test]
fn matching_needs_the_same_title() {
    let hits = parse_search(&fixture("search_tv"));
    // Exact title only; the year picks between the two same-named shows.
    assert_eq!(
        best_match(&hits, "Harbour Lights", Some(2022), false),
        Some(4242)
    );
    assert_eq!(
        best_match(&hits, "Harbour Lights", Some(1995), false),
        Some(77)
    );
    // No year: the first exact hit (TMDB's relevance order).
    assert_eq!(best_match(&hits, "Harbour Lights", None, false), Some(77));
    // Provider decoration and case never decide.
    assert_eq!(
        best_match(&hits, "UK: harbour lights (2022)", None, false),
        Some(4242)
    );
    // A year nobody has: lenient keeps the first, strict refuses.
    assert_eq!(
        best_match(&hits, "Harbour Lights", Some(2010), false),
        Some(77)
    );
    assert_eq!(best_match(&hits, "Harbour Lights", Some(2010), true), None);
    assert_eq!(best_match(&hits, "Harbour", None, false), None);
    assert_eq!(best_match(&hits, "  ", None, false), None);
}

#[test]
fn locale_tags() {
    assert_eq!(locale("en-US"), ("en-US".into(), "US".into(), "en".into()));
    assert_eq!(locale("de"), ("de-US".into(), "US".into(), "de".into()));
    assert_eq!(
        locale("garbage"),
        ("en-US".into(), "US".into(), "en".into())
    );
}

#[test]
fn the_v3_key_query_shape_is_redacted() {
    let url =
        "https://api.themoviedb.org/3/tv/1?language=en-US&api_key=0123456789abcdef0123456789abcdef";
    let out = crate::redact::redact(url);
    assert!(!out.contains("0123456789abcdef"), "{out}");
    let header = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.c2ln";
    let out = crate::redact::redact(header);
    assert!(!out.contains("eyJzdWIiOiJ4In0"), "{out}");
}

#[tokio::test]
async fn v3_keys_go_in_the_query_and_v4_tokens_in_a_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/3/authentication"))
        .and(query_param("api_key", "v3key000"))
        .and(query_param("language", "en-US"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"success\":true}"))
        .expect(1)
        .mount(&server)
        .await;
    let token = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.c2ln";
    Mock::given(method("GET"))
        .and(path("/3/authentication"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"success\":true}"))
        .expect(1)
        .mount(&server)
        .await;
    let base = format!("{}/3/", server.uri());
    TmdbClient::new(&base, SecretString::new("v3key000"), "en-US")
        .unwrap()
        .check()
        .await
        .unwrap();
    TmdbClient::new(&base, SecretString::new(token), "en-US")
        .unwrap()
        .check()
        .await
        .unwrap();
}

#[tokio::test]
async fn a_rejected_key_is_a_non_blocking_auth_error_without_the_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let base = format!("{}/3/", server.uri());
    let e = TmdbClient::new(&base, SecretString::new("secretkey123"), "en-US")
        .unwrap()
        .check()
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::Auth);
    assert!(!e.blocking);
    assert!(!e.message.contains("secretkey123"));
}

#[tokio::test]
async fn transport_errors_never_carry_the_key() {
    // Nothing listens here: the error message is built from the request URL.
    let e = TmdbClient::new(
        "http://127.0.0.1:9/3/",
        SecretString::new("leakykey42"),
        "en-US",
    )
    .unwrap()
    .check()
    .await
    .unwrap_err();
    assert!(!e.message.contains("leakykey42"), "{}", e.message);
}

#[tokio::test]
async fn missing_titles_are_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = TmdbClient::new(
        &format!("{}/3/", server.uri()),
        SecretString::new("k1"),
        "en-US",
    )
    .unwrap();
    assert_eq!(c.title(TmdbKind::Tv, 1).await.unwrap(), None);
    assert_eq!(c.episode(1, 1, 1).await.unwrap(), None);
    assert!(c.season(1, 1).await.unwrap().is_empty());
    assert!(
        c.search(TmdbKind::Movie, "x", None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn bad_keys_are_refused_up_front() {
    assert!(TmdbClient::new(TMDB_API, SecretString::new("  "), "en-US").is_err());
    assert!(TmdbClient::new(TMDB_API, SecretString::new("a b"), "en-US").is_err());
}
