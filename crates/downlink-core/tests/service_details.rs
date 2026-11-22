#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Details façade: the source's programme detail wins, TMDB fills the gaps, lookups are cached,
//! and the TMDB key stays in the secret store. Synthetic titles only; both servers are mocks.

use std::sync::Arc;

use downlink_core::auth::{MemorySecretStore, SecretStore, SecretString};
use downlink_core::service::{MediaTargetArg, Service};
use downlink_core::source::{VodExtra, VodItem, VodKind};
use downlink_core::store::{ChannelUpsert, NewSource, ProgrammeRow, Store};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TMDB_KEY: &str = "tmdbkey0123456789";
const START: i64 = 1_800_000_000;

fn fixture(rel: &str) -> Value {
    let p = format!("{}/../../tools/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

async fn get(server: &MockServer, p: &str, body: Value, times: impl Into<wiremock::Times>) {
    Mock::given(method("GET"))
        .and(path(p.to_owned()))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(times)
        .mount(server)
        .await;
}

struct World {
    svc: Arc<Service>,
    store: Store,
    tmdb: MockServer,
    _dispatcharr: MockServer,
    _dir: tempfile::TempDir,
    channel_id: i64,
    source_id: i64,
}

async fn world() -> World {
    let dispatcharr = MockServer::start().await;
    let tmdb = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set("src-test/apikey", &SecretString::new("dispatcharr-key"))
        .unwrap();
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "S".into(),
            base_url: Some(dispatcharr.uri()),
            secret_ref: Some("src-test/apikey".into()),
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let groups = store.sync_groups(src.id, &[]).await.unwrap();
    store
        .sync_channels(
            src.id,
            &[ChannelUpsert {
                ext_id: "7".into(),
                name: "Synthetic Drama".into(),
                tvg_id: Some("synthetic-drama.test".into()),
                ..Default::default()
            }],
            &groups,
        )
        .await
        .unwrap();
    let channel_id = store.channel_by_ext(src.id, "7").await.unwrap().unwrap().id;
    store
        .upsert_programmes(&[ProgrammeRow {
            channel_id,
            start_utc: START,
            end_utc: START + 3600,
            ext_id: Some("9001".into()),
            title: Some("Harbour Lights".into()),
            subtitle: None,
            description: Some("Guide text for this airing.".into()),
            season: Some(2),
            episode: Some(5),
            flags: 0,
        }])
        .await
        .unwrap();
    // The programme detail is fetched at most once; the cache answers the second view.
    get(
        &dispatcharr,
        "/api/epg/programs/9001/",
        fixture("dispatcharr/epg_program_detail_rich.json"),
        0..=1,
    )
    .await;
    let svc = Service::with_store(store.clone(), secrets, dir.path().join("img"), "t".into());
    svc.set_tmdb_api(&format!("{}/3/", tmdb.uri()));
    World {
        svc,
        store,
        tmdb,
        _dispatcharr: dispatcharr,
        _dir: dir,
        channel_id,
        source_id: src.id,
    }
}

async fn with_key(w: &World) {
    get(
        &w.tmdb,
        "/3/authentication",
        serde_json::json!({"success": true}),
        1,
    )
    .await;
    let st = w
        .svc
        .tmdb_set_key(SecretString::new(TMDB_KEY))
        .await
        .unwrap();
    assert!(st.configured);
}

#[tokio::test]
async fn programme_details_merge_source_first_then_tmdb() {
    let w = world().await;
    with_key(&w).await;
    get(&w.tmdb, "/3/tv/4242", fixture("tmdb/tv.json"), 1).await;
    get(
        &w.tmdb,
        "/3/tv/4242/season/2/episode/5",
        fixture("tmdb/episode.json"),
        1,
    )
    .await;

    let id = format!("{}:{START}", w.channel_id);
    let d = w
        .svc
        .media_details(MediaTargetArg::Programme { id: id.clone() })
        .await
        .unwrap();
    assert_eq!(d.kind, "programme");
    assert_eq!(d.title, "Harbour Lights");
    // The guide's own text and ratings win.
    assert_eq!(
        d.description.as_deref(),
        Some("Guide text for this airing.")
    );
    assert_eq!(d.certification.as_deref(), Some("TV-14"));
    assert_eq!(d.genres, vec!["Drama", "Mystery"]);
    assert_eq!(d.score, Some(8.0));
    assert_eq!(d.air_date.as_deref(), Some("2024-03-02"));
    assert_eq!(d.runtime_mins, Some(44));
    assert!(d.is_repeat);
    // The guide had no episode title: TMDB's fills it.
    assert_eq!(d.episode_title.as_deref(), Some("The Lantern Keeper"));
    assert_eq!((d.season, d.episode), (Some(2), Some(5)));
    // The show's overview comes from TMDB, and differs from the airing's description.
    assert!(
        d.overview
            .as_deref()
            .unwrap()
            .contains("coastal police team")
    );
    assert_eq!(d.network.as_deref(), Some("Synthetic One"));
    // The guide's cast wins; TMDB adds the photo for a name it knows.
    assert_eq!(d.cast[0].name, "Ann Example");
    assert_eq!(d.cast[0].role.as_deref(), Some("Detective Vale"));
    assert!(d.cast[0].photo.as_deref().unwrap().starts_with("dlimg://"));
    assert!(d.cast[1].guest);
    // The guide's poster wins over TMDB's; the episode still is TMDB's.
    assert!(d.poster.is_some() && d.still.is_some());
    assert!(d.from_tmdb);
    assert_eq!(
        d.tmdb_url.as_deref(),
        Some("https://www.themoviedb.org/tv/4242")
    );
    let crew: Vec<_> = d.crew.iter().map(|c| c.job).collect();
    assert_eq!(crew, vec!["creator", "director", "writer"]);

    // Second view: every answer comes from the cache (the mocks expect one call each).
    let again = w
        .svc
        .media_details(MediaTargetArg::Programme { id })
        .await
        .unwrap();
    assert_eq!(again.episode_title, d.episode_title);

    // The key never reaches SQLite.
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, body FROM meta_cache")
        .fetch_all(w.store.pool())
        .await
        .unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|(k, b)| !k.contains(TMDB_KEY) && !b.contains(TMDB_KEY))
    );
}

#[tokio::test]
async fn without_a_key_details_are_the_source_only() {
    let w = world().await;
    let d = w
        .svc
        .media_details(MediaTargetArg::Programme {
            id: format!("{}:{START}", w.channel_id),
        })
        .await
        .unwrap();
    assert!(!d.from_tmdb);
    assert_eq!(d.episode_title, None);
    assert_eq!(d.certification.as_deref(), Some("TV-14"));
    assert!(!w.svc.tmdb_status().await.unwrap().configured);
    assert!(w.tmdb.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn vod_details_and_movie_page_fill_gaps_from_tmdb() {
    let w = world().await;
    with_key(&w).await;
    get(&w.tmdb, "/3/movie/501", fixture("tmdb/movie.json"), 1).await;
    let rows = w
        .store
        .upsert_vod_items(
            w.source_id,
            &[VodItem {
                kind: Some(VodKind::Movie),
                ext_id: "m1".into(),
                title: "The Paper Orchard".into(),
                tmdb_id: Some("501".into()),
                extra: Some(VodExtra {
                    cast: vec!["Bo Sample".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            1,
        )
        .await
        .unwrap();
    let id = rows[0].id;
    let d = w
        .svc
        .media_details(MediaTargetArg::Vod { id })
        .await
        .unwrap();
    assert_eq!(d.kind, "movie");
    assert_eq!(
        d.description.as_deref(),
        Some("An archivist finds a map folded into a library book.")
    );
    assert_eq!(d.tagline.as_deref(), Some("Every page leads somewhere."));
    assert_eq!(d.certification.as_deref(), Some("PG-13"));
    // The source's cast list wins over TMDB's.
    assert_eq!(d.cast.len(), 1);
    assert_eq!(d.cast[0].name, "Bo Sample");
    assert_eq!(
        d.trailer_url.as_deref(),
        Some("https://www.youtube.com/watch?v=trailerKey1")
    );
    assert!(d.logo.is_some());

    // The movie page itself fills its empty plot and genres from the cached title.
    let m = w.svc.get_movie(id).await.unwrap();
    assert_eq!(m.plot, d.description);
    assert_eq!(m.genres, vec!["Drama", "Adventure"]);
    assert_eq!(m.duration_secs, Some(112 * 60));
}

#[tokio::test]
async fn clearing_the_key_forgets_tmdb_answers() {
    let w = world().await;
    with_key(&w).await;
    w.store
        .meta_put("tmdb:en-US:title:tv:1", "null", 1)
        .await
        .unwrap();
    let st = w.svc.tmdb_clear_key().await.unwrap();
    assert!(!st.configured);
    assert!(
        w.store
            .meta_get("tmdb:en-US:title:tv:1")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_rejected_key_is_not_saved() {
    let w = world().await;
    Mock::given(method("GET"))
        .and(path("/3/authentication"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&w.tmdb)
        .await;
    let e = w
        .svc
        .tmdb_set_key(SecretString::new("wrongkey"))
        .await
        .unwrap_err();
    assert!(!e.message.contains("wrongkey"));
    assert!(!w.svc.tmdb_status().await.unwrap().configured);
}
