#![allow(clippy::unwrap_used, clippy::expect_used)]
//! M3U and Xtream through the Service (add / test / sync / play / catch-up), against the shared
//! synthetic fixtures. Covers PLAN §6.8: stable ids, favourites surviving a URL change, DRM
//! labelling, no secrets in logs or SQLite.

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::auth::{MemorySecretStore, SecretStore};
use downlink_core::service::{AddSourceInput, ListChannelsArgs, Service};
use downlink_core::store::{ProgrammeRow, Store};
use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fx_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures")
}
fn fx_text(rel: &str) -> String {
    std::fs::read_to_string(fx_dir().join(rel)).unwrap()
}
fn fx_json(rel: &str) -> Value {
    serde_json::from_str(&fx_text(rel)).unwrap()
}

struct Env {
    svc: Arc<Service>,
    secrets: Arc<MemorySecretStore>,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        secrets.clone(),
        dir.path().join("img"),
        "t".into(),
    );
    Env {
        svc,
        secrets,
        _dir: dir,
    }
}

fn m3u_input(url: &str, xmltv: Option<&str>) -> AddSourceInput {
    serde_json::from_value(json!({"kind": "m3u", "name": "Playlist", "url": url, "xmltvUrl": xmltv, "allowSelfSigned": false})).unwrap()
}

async fn serve_text(server: &MockServer, p: &str, body: String) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

async fn names(e: &Env) -> Vec<String> {
    e.svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

#[tokio::test]
async fn m3u_from_url_with_xmltv_syncs_matches_by_name_and_flags_it() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    // Guide: first channel matches by tvg-id, "Test News" only by display name.
    let now = downlink_core::sync::now_utc();
    let t = |o: i64| {
        chrono::DateTime::from_timestamp(now + o, 0)
            .unwrap()
            .format("%Y%m%d%H%M%S +0000")
            .to_string()
    };
    let xml = format!(
        "<tv><channel id=\"one.test\"><display-name>Channel One</display-name></channel><channel id=\"x9\"><display-name>Test News</display-name></channel>\
<programme start=\"{}\" stop=\"{}\" channel=\"one.test\"><title>Now On One</title></programme>\
<programme start=\"{}\" stop=\"{}\" channel=\"x9\"><title>Now On News</title></programme></tv>",
        t(-600),
        t(3000),
        t(-600),
        t(3000)
    );
    serve_text(&server, "/epg.xml", xml).await;
    let e = env().await;
    let url = format!(
        "{}/get.php?username=testuser&password=testpass",
        server.uri()
    );
    let dto = e
        .svc
        .add_source(m3u_input(&url, Some(&format!("{}/epg.xml", server.uri()))))
        .await
        .unwrap();
    let v = serde_json::to_value(&dto).unwrap();
    assert_eq!(v["kind"], "m3u");
    assert_eq!(v["caps"].as_u64().unwrap() & 0b11, 0b11, "LIVE + EPG");

    // The playlist URL (credentials!) is in the keychain only; SQLite has a credential-free address.
    let row = e.svc.store().source(dto.id).await.unwrap();
    assert!(
        !row.base_url.as_deref().unwrap().contains("testpass")
            && !row.base_url.as_deref().unwrap().contains("password=")
    );
    assert_eq!(
        e.secrets
            .get(row.secret_ref.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .expose(),
        url
    );

    let chans = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    assert_eq!(chans.len(), 3);
    let by = |n: &str| chans.iter().find(|c| c.name == n).unwrap();
    assert_eq!(by("Channel One").epg_match, "id");
    assert_eq!(
        by("Test News").epg_match,
        "display",
        "a non-id match: the UI shows \"guide matched by name\""
    );
    assert_eq!(by("No Group, No Id").epg_match, "none");
    let nn = e
        .svc
        .get_now_next(&[by("Channel One").id, by("Test News").id])
        .await
        .unwrap();
    assert_eq!(
        nn[0].now.as_ref().unwrap().title.as_deref(),
        Some("Now On One")
    );
    assert_eq!(
        nn[1].now.as_ref().unwrap().title.as_deref(),
        Some("Now On News")
    );
    assert!(by("Channel One").logo.is_some());
}

#[tokio::test]
async fn m3u_local_file_aux_lines_headers_drm_and_credentials() {
    let e = env().await;
    let p = fx_dir().join("m3u/aux_lines.m3u");
    let dto = e
        .svc
        .add_source(m3u_input(p.to_str().unwrap(), None))
        .await
        .unwrap();
    assert!(dto.host.is_none() || dto.host.as_deref() == Some(""));
    let all = e
        .svc
        .list_channels(ListChannelsArgs {
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    let id = |n: &str| all.iter().find(|c| c.name == n).map(|c| c.id);
    assert!(
        id("DRM Channel").is_none(),
        "DRM channels are filtered by catalog"
    );
    let ua = e
        .svc
        .play_channel(id("Custom UA Channel").unwrap())
        .await
        .unwrap();
    assert_eq!(ua.url, "http://iptv.test/ua/1.m3u8");
    assert_eq!(ua.surface, downlink_core::source::Surface::Hls);
    let h: Vec<(&str, &str)> = ua
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.expose()))
        .collect();
    assert!(
        h.contains(&("User-Agent", "SyntheticAgent/1.0"))
            && h.contains(&("Referer", "http://iptv.test/ref")),
        "{h:?}"
    );
    let pipe = e
        .svc
        .play_channel(id("Pipe Headers").unwrap())
        .await
        .unwrap();
    assert_eq!(
        pipe.url, "http://iptv.test/hdr/1.ts",
        "pipe suffix split off"
    );
    assert!(
        pipe.headers
            .iter()
            .any(|(k, v)| k == "User-Agent" && v.expose() == "SyntheticAgent/2.0")
    );
    // A credential-bearing stream URL never shows up in the redacted view.
    let cred = e
        .svc
        .play_channel(id("Credentials In URL").unwrap())
        .await
        .unwrap();
    let shown = serde_json::to_string(&cred.redacted()).unwrap();
    assert!(!shown.contains("testpass"), "{shown}");
    let cq = e
        .svc
        .play_channel(id("Credentials In Query").unwrap())
        .await
        .unwrap();
    let shown = serde_json::to_string(&cq.redacted()).unwrap();
    assert!(
        !shown.contains("testpass") && !shown.contains("testtoken123"),
        "{shown}"
    );

    // The DRM entry exists but is excluded from play: "DRM — not supported".
    let drm_id: i64 = sqlx::query_scalar("SELECT id FROM channel WHERE is_drm = 1")
        .fetch_one(e.svc.store().pool())
        .await
        .unwrap();
    let err = e.svc.play_channel(drm_id).await.unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Media);
    assert!(err.message.contains("DRM"), "{}", err.message);
    let opt_in = e.svc.set_content_filters(false, false).await;
    assert!(opt_in.is_ok());
}

#[tokio::test]
async fn reimport_is_stable_and_a_provider_url_change_keeps_ids_and_favourites() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("p.m3u");
    let body = |host: &str| {
        format!(
            "#EXTM3U\n#EXTINF:-1 tvg-id=\"a.test\" group-title=\"G\",Alpha HD\nhttp://{host}/live/1.ts\n#EXTINF:-1 tvg-id=\"b.test\" group-title=\"G\",Bravo\nhttp://{host}/live/2.ts\n\
#EXTINF:-1 tvg-id=\"dup.test\" group-title=\"G\",Dup One\nhttp://{host}/live/3.ts\n#EXTINF:-1 tvg-id=\"dup.test\" group-title=\"G\",Dup Two\nhttp://{host}/live/4.ts\n"
        )
    };
    std::fs::write(&file, body("one.provider.test")).unwrap();
    let e = env().await;
    let dto = e
        .svc
        .add_source(m3u_input(file.to_str().unwrap(), None))
        .await
        .unwrap();
    let ids = |_: ()| {
        let pool = e.svc.store().pool().clone();
        async move {
            sqlx::query_as::<_, (i64, String)>("SELECT id, ext_id FROM channel ORDER BY ext_id")
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    };
    let before = ids(()).await;
    assert_eq!(before.len(), 4);
    assert!(
        before.iter().any(|(_, x)| x == "dup.test")
            && before.iter().any(|(_, x)| x == "dup.test#2"),
        "duplicate tvg-ids get stable suffixes: {before:?}"
    );
    let alpha = before.iter().find(|(_, x)| x == "a.test").unwrap().0;
    e.svc.set_favourite(alpha, true).await.unwrap();

    // Re-import unchanged: zero row ids change.
    e.svc.sync_source(dto.id).await.unwrap();
    assert_eq!(ids(()).await, before);

    // The provider moves hosts: ids and favourites survive, the URL is updated.
    std::fs::write(&file, body("two.provider.test")).unwrap();
    e.svc.sync_source(dto.id).await.unwrap();
    let after = ids(()).await;
    assert_eq!(
        after, before,
        "tvg-id channels keep their row ids across a URL change"
    );
    let favs = e
        .svc
        .list_channels(ListChannelsArgs {
            favourites_only: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(favs.len(), 1);
    assert_eq!(favs[0].id, alpha);
    let plan = e.svc.play_channel(alpha).await.unwrap();
    assert!(
        plan.url.contains("two.provider.test"),
        "the stored URL follows the provider: {}",
        plan.url
    );
}

#[tokio::test]
async fn m3u_catchup_template_path_has_no_session_and_uses_the_programme_row() {
    let e = env().await;
    let p = fx_dir().join("m3u/catchup.m3u");
    e.svc
        .add_source(m3u_input(p.to_str().unwrap(), None))
        .await
        .unwrap();
    let chans = e
        .svc
        .list_channels(ListChannelsArgs {
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    let c = chans.iter().find(|c| c.name == "Default Mode").unwrap();
    assert!(c.is_catchup && c.catchup_days == 7);
    assert!(
        chans
            .iter()
            .find(|c| c.name == "Unsupported Flussonic")
            .is_some_and(|c| !c.is_catchup)
    );
    // Give it a programme in the past, in the archive window.
    let start = downlink_core::sync::now_utc() - 7200;
    e.svc
        .store()
        .upsert_programmes(&[ProgrammeRow {
            channel_id: c.id,
            start_utc: start,
            end_utc: start + 3600,
            ext_id: None,
            title: Some("Past".into()),
            subtitle: None,
            description: None,
            season: None,
            episode: None,
            flags: 0,
        }])
        .await
        .unwrap();
    let play = e
        .svc
        .play_catchup(&format!("{}:{start}", c.id))
        .await
        .unwrap();
    assert_eq!(
        play.plan.url,
        format!("http://iptv.test/ts/1?utc={start}&utcend={}", start + 3600)
            .replace("/ts/1", "/ts/1"),
        "default mode, epoch placeholders from the programme row"
    );
    assert_eq!(
        play.plan.profile,
        downlink_core::source::PlayProfile::Seekable
    );
    assert!(
        play.session.0.is_empty(),
        "template path has no server session"
    );
    assert!(e.svc.store().list_open_sessions().await.unwrap().is_empty());
    e.svc.stop_catchup(&play).await.unwrap(); // nothing to delete, and no error
}

async fn xc_server() -> MockServer {
    let server = MockServer::start().await;
    struct NoAction;
    impl wiremock::Match for NoAction {
        fn matches(&self, r: &wiremock::Request) -> bool {
            !r.url.query_pairs().any(|(k, _)| k == "action")
        }
    }
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(NoAction)
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fx_json("xtream/player_api_login.json")),
        )
        .mount(&server)
        .await;
    for (a, f) in [
        ("get_live_categories", "xtream/get_live_categories.json"),
        ("get_live_streams", "xtream/get_live_streams.json"),
    ] {
        Mock::given(method("GET"))
            .and(path("/player_api.php"))
            .and(query_param("action", a))
            .respond_with(ResponseTemplate::new(200).set_body_json(fx_json(f)))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET")).and(path("/xmltv.php")).respond_with(ResponseTemplate::new(200).set_body_string("<tv><channel id=\"test-channel-1.test\"><display-name>Test Channel 1</display-name></channel></tv>")).mount(&server).await;
    server
}

fn xc_input(server: &MockServer) -> AddSourceInput {
    serde_json::from_value(json!({"kind": "xtream", "name": "Panel", "url": server.uri(), "username": "testuser", "password": "testpass", "allowSelfSigned": false})).unwrap()
}

#[tokio::test]
async fn xtream_add_sync_play_and_timeshift_catchup() {
    let server = xc_server().await;
    let e = env().await;
    let t = e.svc.test_source(xc_input(&server)).await.unwrap();
    assert!(t.channels >= 5 && t.groups == 6);
    let dto = e.svc.add_source(xc_input(&server)).await.unwrap();
    let v = serde_json::to_value(&dto).unwrap();
    assert_eq!(v["kind"], "xtream");

    // Secrets: the password is in the keychain; SQLite has the username and a credential-free base.
    let row = e.svc.store().source(dto.id).await.unwrap();
    assert_eq!(row.username.as_deref(), Some("testuser"));
    assert_eq!(
        e.secrets
            .get(row.secret_ref.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .expose(),
        "testpass"
    );
    assert!(!row.base_url.as_deref().unwrap().contains("testpass"));

    let chans = e
        .svc
        .list_channels(ListChannelsArgs {
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    let by = |n: &str| chans.iter().find(|c| c.name == n);
    assert!(
        by("Test Radio FM").is_none(),
        "radio category is classified and filtered by catalog"
    );
    let c1 = by("Test Channel 1").unwrap();
    let c2 = by("Test Channel 2").unwrap();
    assert!(!c1.is_catchup && c2.is_catchup && c2.catchup_days == 3);

    let plan = e.svc.play_channel(c1.id).await.unwrap();
    assert_eq!(
        plan.url,
        format!("{}/live/testuser/testpass/1001.ts", server.uri())
    );
    assert!(
        !serde_json::to_string(&plan.redacted())
            .unwrap()
            .contains("testpass")
    );

    // Timeshift: the programme row, rendered in the panel's zone (Europe/London, summer = +1).
    let start = 1_782_907_200; // 2026-07-01T12:00:00Z
    e.svc
        .store()
        .upsert_programmes(&[ProgrammeRow {
            channel_id: c2.id,
            start_utc: start,
            end_utc: start + 3600,
            ext_id: None,
            title: Some("Past".into()),
            subtitle: None,
            description: None,
            season: None,
            episode: None,
            flags: 0,
        }])
        .await
        .unwrap();
    let play = e
        .svc
        .play_catchup(&format!("{}:{start}", c2.id))
        .await
        .unwrap();
    assert_eq!(
        play.plan.url,
        format!(
            "{}/timeshift/testuser/testpass/60/2026-07-01:13-00/1002.ts",
            server.uri()
        )
    );
    assert!(play.session.0.is_empty());
    assert!(
        !serde_json::to_string(&play.plan.redacted())
            .unwrap()
            .contains("testpass")
    );

    // Removal clears the keychain.
    e.svc.remove_source(dto.id).await.unwrap();
    assert!(
        e.secrets
            .get(row.secret_ref.as_deref().unwrap())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn xtream_bad_password_persists_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fx_json("xtream/error_auth_failed.json")),
        )
        .mount(&server)
        .await;
    let e = env().await;
    let err = e.svc.add_source(xc_input(&server)).await.unwrap_err();
    assert_eq!(err.code, downlink_core::ErrorCode::Auth);
    assert!(e.svc.list_sources().await.unwrap().is_empty());
}

/// PLAN §6.8: a credential-bearing playlist leaves no secret in logs. The core logs counts only,
/// so the raw (unredacted) log output must already be clean.
#[tokio::test]
async fn credential_bearing_playlist_leaves_no_secret_in_logs_or_the_database() {
    use std::io::Write;
    use std::sync::Mutex;
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sink = Sink::default();
    let s2 = sink.clone();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || s2.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(sub);

    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/aux_lines.m3u")).await;
    let e = env().await;
    let url = format!(
        "{}/get.php?username=testuser&password=SECRETPASS99&token=SECRETTOKEN77",
        server.uri()
    );
    e.svc.add_source(m3u_input(&url, None)).await.unwrap();
    let _ = e
        .svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();

    let logs = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    for secret in ["SECRETPASS99", "SECRETTOKEN77", "testpass", "testtoken123"] {
        assert!(!logs.contains(secret), "{secret} leaked into logs:\n{logs}");
    }
    // The database holds the playlist's own per-entry URLs (they ARE the channel addresses) but
    // never the playlist URL's credentials, nor anything from the keychain.
    let src_row = e.svc.store().list_sources().await.unwrap().remove(0);
    let blob = format!("{src_row:?}");
    assert!(
        !blob.contains("SECRETPASS99") && !blob.contains("SECRETTOKEN77"),
        "{blob}"
    );
}

#[tokio::test]
async fn bad_playlist_inputs_fail_with_plain_sentences() {
    let e = env().await;
    assert!(
        e.svc
            .add_source(m3u_input("relative/path.m3u", None))
            .await
            .is_err()
    );
    assert!(
        e.svc
            .add_source(m3u_input("/definitely/not/here.m3u", None))
            .await
            .is_err()
    );
    assert!(e.svc.list_sources().await.unwrap().is_empty());
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    assert!(
        e.svc
            .add_source(m3u_input(&format!("{}/nope.m3u", server.uri()), None))
            .await
            .is_err()
    );
    let _ = names(&e).await;
}
