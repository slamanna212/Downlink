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
            icon: None,
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
            icon: None,
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

// ---- rule 7: no credential ever lands in SQLite ------------------------------------------------

fn data_files_contain(dir: &std::path::Path, needle: &str) -> Vec<String> {
    let mut hits = vec![];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_file()
            && let Ok(bytes) = std::fs::read(&p)
            && bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
        {
            hits.push(p.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    hits
}

const CRED_PLAYLIST: &str = "#EXTM3U catchup=\"default\" catchup-days=\"3\" catchup-source=\"http://p.test/timeshift.php?username=ZUSER&password=ZPASSWORD1&stream=1&utc={utc}&duration={duration:60}\"\n\
#EXTINF:-1 tvg-id=\"a.test\" tvg-logo=\"http://p.test/logo.png?token=ZLOGOTOKEN1\" group-title=\"G\",Cred Channel\n\
#EXTHTTP:{\"Authorization\":\"Bearer ZBEARER1\",\"Referer\":\"http://r.test/\"}\n\
http://ZUSER:ZPASSWORD1@p.test/live/ZUSER/ZPASSWORD1/1.ts?username=ZUSER&password=ZPASSWORD1&token=ZTOKEN12345\n\
#EXTINF:-1 tvg-id=\"b.test\" group-title=\"G\",Other Token\n\
http://p.test/live/2.ts?token=ZOTHERTOKEN2\n";

#[tokio::test]
async fn credentialed_playlist_is_templated_in_sqlite_and_resolved_from_the_keychain() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", CRED_PLAYLIST.to_owned()).await;
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let svc = Service::new(downlink_core::service::ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        secrets: secrets.clone(),
        app_version: "t".into(),
    })
    .await
    .unwrap();
    let url = format!(
        "{}/get.php?username=ZUSER&password=ZPLAYLISTPASS",
        server.uri()
    );
    let dto = svc.add_source(m3u_input(&url, None)).await.unwrap();

    // Flush the WAL so the main file holds everything, then scan every file for every secret.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(svc.store().pool())
        .await
        .unwrap();
    for secret in [
        "ZUSER",
        "ZPASSWORD1",
        "ZLOGOTOKEN1",
        "ZBEARER1",
        "ZTOKEN12345",
        "ZOTHERTOKEN2",
        "ZPLAYLISTPASS",
    ] {
        let hits = data_files_contain(dir.path(), secret);
        assert!(hits.is_empty(), "{secret} found in {hits:?}");
    }
    // What the rows hold is the templated shape.
    let (play_url, headers, tpl): (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT play_url, play_headers, catchup_template FROM channel WHERE tvg_id = 'a.test'",
    )
    .fetch_one(svc.store().pool())
    .await
    .unwrap();
    assert!(
        play_url.contains("{dl:user}")
            && play_url.contains("{dl:pass}")
            && play_url.contains("{dl:token"),
        "{play_url}"
    );
    assert!(headers.unwrap().contains("{dl:hdr}"));
    assert!(
        tpl.unwrap().contains("{dl:pass}"),
        "catch-up source templated too"
    );
    let other: String = sqlx::query_scalar("SELECT play_url FROM channel WHERE tvg_id = 'b.test'")
        .fetch_one(svc.store().pool())
        .await
        .unwrap();
    assert!(
        other.contains("{dl:token:"),
        "a second, different token is keyed by a short hash: {other}"
    );

    // The keychain holds one deduplicated vault entry (user/pass/token shared across entries).
    let row = svc.store().source(dto.id).await.unwrap();
    let vault: std::collections::HashMap<String, String> = serde_json::from_str(
        secrets
            .get(&downlink_core::auth::secret_ref(
                &row.uuid,
                downlink_core::auth::SecretKind::Credentials,
            ))
            .unwrap()
            .unwrap()
            .expose(),
    )
    .unwrap();
    assert_eq!(vault.get("user").map(String::as_str), Some("ZUSER"));
    assert_eq!(vault.get("pass").map(String::as_str), Some("ZPASSWORD1"));
    assert!(vault.len() >= 5);

    // Play resolves in memory, and still does after an "app restart" (new Service, same keychain).
    let chans = svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let a = chans.iter().find(|c| c.name == "Cred Channel").unwrap();
    let plan = svc.play_channel(a.id).await.unwrap();
    assert_eq!(
        plan.url,
        "http://ZUSER:ZPASSWORD1@p.test/live/ZUSER/ZPASSWORD1/1.ts?username=ZUSER&password=ZPASSWORD1&token=ZTOKEN12345"
    );
    assert!(
        plan.headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v.expose() == "Bearer ZBEARER1")
    );
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(
        !shown.contains("ZPASSWORD1") && !shown.contains("ZBEARER1"),
        "{shown}"
    );

    drop(svc);
    let svc2 = Service::new(downlink_core::service::ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        secrets: secrets.clone(),
        app_version: "t".into(),
    })
    .await
    .unwrap();
    let plan2 = svc2.play_channel(a.id).await.unwrap();
    assert_eq!(plan2.url, plan.url, "vault restored from the keychain");

    // Catch-up expands only after the vault is resolved.
    let start = downlink_core::sync::now_utc() - 7200;
    svc2.store()
        .upsert_programmes(&[ProgrammeRow {
            channel_id: a.id,
            start_utc: start,
            end_utc: start + 3600,
            ext_id: None,
            title: Some("P".into()),
            subtitle: None,
            description: None,
            season: None,
            episode: None,
            flags: 0,
            icon: None,
        }])
        .await
        .unwrap();
    let cu = svc2
        .play_catchup(&format!("{}:{start}", a.id))
        .await
        .unwrap();
    assert_eq!(
        cu.plan.url,
        format!(
            "http://p.test/timeshift.php?username=ZUSER&password=ZPASSWORD1&stream=1&utc={start}&duration=60"
        )
    );

    // A resync keeps placeholders stable (no row churn) and still stores no secret.
    svc2.sync_source(dto.id).await.unwrap();
    let again: String = sqlx::query_scalar("SELECT play_url FROM channel WHERE tvg_id = 'a.test'")
        .fetch_one(svc2.store().pool())
        .await
        .unwrap();
    assert_eq!(again, play_url);
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(svc2.store().pool())
        .await
        .unwrap();
    assert!(data_files_contain(dir.path(), "ZPASSWORD1").is_empty());

    // Removal clears the vault from the keychain.
    svc2.remove_source(dto.id).await.unwrap();
    assert!(
        secrets
            .get(&downlink_core::auth::secret_ref(
                &row.uuid,
                downlink_core::auth::SecretKind::Credentials
            ))
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn xtream_source_keeps_passwords_and_direct_source_credentials_out_of_sqlite() {
    let server = xc_server().await;
    // One channel carries a credential-bearing direct_source and a tokenised logo.
    let mut streams = fx_json("xtream/get_live_streams.json");
    streams[0]["direct_source"] = json!("http://cdn.test/live/xcuser/XCPASSWORD9/1001.ts");
    streams[0]["stream_icon"] = json!("http://cdn.test/logo/1.png?token=XCLOGOTOKEN9");
    server.reset().await;
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
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(query_param("action", "get_live_categories"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fx_json("xtream/get_live_categories.json")),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(query_param("action", "get_live_streams"))
        .respond_with(ResponseTemplate::new(200).set_body_json(streams))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/xmltv.php"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<tv/>"))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let svc = Service::new(downlink_core::service::ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        secrets: secrets.clone(),
        app_version: "t".into(),
    })
    .await
    .unwrap();
    let mut input = xc_input(&server);
    input.password = Some("XCPLAINPASS7".into());
    // The mock ignores credentials, so the login succeeds with any password.
    let dto = svc.add_source(input).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(svc.store().pool())
        .await
        .unwrap();
    for secret in ["XCPLAINPASS7", "XCPASSWORD9", "XCLOGOTOKEN9"] {
        assert!(
            data_files_contain(dir.path(), secret).is_empty(),
            "{secret} found in SQLite files"
        );
    }
    let chans = svc
        .list_channels(ListChannelsArgs {
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    let c1 = chans.iter().find(|c| c.name == "Test Channel 1").unwrap();
    let plan = svc.play_channel(c1.id).await.unwrap();
    assert_eq!(
        plan.url, "http://cdn.test/live/xcuser/XCPASSWORD9/1001.ts",
        "direct_source resolved at play time"
    );
    let shown = serde_json::to_string(&plan.redacted()).unwrap();
    assert!(!shown.contains("XCPASSWORD9"), "{shown}");
    assert!(
        svc.list_sources()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == dto.id)
    );
}

/// PLAN §5.4: "restart this programme" from live starts at the airing programme's start for every
/// template mode, whatever zone the template renders in. (Dispatcharr: `sync.rs`
/// `restart_programme_starts_from_the_airing_programmes_row` pins the session API `start`.)
#[tokio::test]
async fn restart_programme_starts_at_the_programme_start_in_all_template_modes_and_zones() {
    use chrono::TimeZone;
    let playlist = "#EXTM3U\n\
#EXTINF:-1 tvg-id=\"r1.test\" catchup=\"default\" catchup-days=\"3\" catchup-timezone=\"America/New_York\" catchup-source=\"http://iptv.test/ts/1?at={Y}{m}{d}-{H}{M}&utc={utc}\" group-title=\"R\",Default NY\n\
http://iptv.test/live/r1.ts\n\
#EXTINF:-1 tvg-id=\"r2.test\" catchup=\"append\" catchup-days=\"3\" catchup-timezone=\"Asia/Kolkata\" catchup-source=\"?begin={start:%H:%M}&len={duration:60}\" group-title=\"R\",Append Kolkata\n\
http://iptv.test/live/r2.ts\n\
#EXTINF:-1 tvg-id=\"r3.test\" catchup=\"shift\" catchup-days=\"3\" group-title=\"R\",Shift UTC\n\
http://iptv.test/live/r3.ts\n\
#EXTINF:-1 tvg-id=\"r4.test\" catchup=\"xc\" catchup-days=\"3\" catchup-timezone=\"Pacific/Auckland\" group-title=\"R\",Xc Auckland\n\
http://iptv.test/live/testuser/testpass/4.ts\n";
    let server = MockServer::start().await;
    serve_text(&server, "/r.m3u", playlist.to_owned()).await;
    let e = env().await;
    e.svc
        .add_source(m3u_input(&format!("{}/r.m3u", server.uri()), None))
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
    // Airing now: started 25 minutes ago, one hour long.
    let start = downlink_core::sync::now_utc() - 1500;
    let local = |tz: chrono_tz::Tz, f: &str| {
        tz.timestamp_opt(start, 0)
            .single()
            .unwrap()
            .format(f)
            .to_string()
    };
    for c in &chans {
        e.svc
            .store()
            .upsert_programmes(&[ProgrammeRow {
                channel_id: c.id,
                start_utc: start,
                end_utc: start + 3600,
                ext_id: None,
                title: Some("Airing".into()),
                subtitle: None,
                description: None,
                season: None,
                episode: None,
                flags: 0,
                icon: None,
            }])
            .await
            .unwrap();
    }
    let url_for = |name: &str| {
        let id = chans.iter().find(|c| c.name == name).unwrap().id;
        let svc = e.svc.clone();
        let name = name.to_owned();
        async move {
            let play = svc.restart_programme(id).await.unwrap();
            assert_eq!(play.start_utc, start, "{name}: row start");
            assert!(play.session.0.is_empty());
            play.plan.url
        }
    };
    let ny = local(chrono_tz::America::New_York, "%Y%m%d-%H%M");
    assert_eq!(
        url_for("Default NY").await,
        format!("http://iptv.test/ts/1?at={ny}&utc={start}")
    );
    let kol = local(chrono_tz::Asia::Kolkata, "%H:%M");
    assert_eq!(
        url_for("Append Kolkata").await,
        format!("http://iptv.test/live/r2.ts?begin={kol}&len=60")
    );
    let shift = url_for("Shift UTC").await;
    assert!(
        shift.starts_with(&format!("http://iptv.test/live/r3.ts?utc={start}&lutc=")),
        "{shift}"
    );
    let akl = local(chrono_tz::Pacific::Auckland, "%Y-%m-%d:%H-%M");
    assert_eq!(
        url_for("Xc Auckland").await,
        format!("http://iptv.test/timeshift/testuser/testpass/60/{akl}/4.ts")
    );
}

#[tokio::test]
async fn m3u_tvg_shift_header_and_entry_become_per_channel_seconds() {
    let playlist = "#EXTM3U tvg-shift=\"1.5\"\n\
#EXTINF:-1 tvg-id=\"a.test\",Header Shift\nhttp://iptv.test/a.ts\n\
#EXTINF:-1 tvg-id=\"b.test\" tvg-shift=\"-0.5\",Entry Negative\nhttp://iptv.test/b.ts\n\
#EXTINF:-1 tvg-id=\"c.test\" tvg-shift=\"0\",Entry Zero\nhttp://iptv.test/c.ts\n\
#EXTINF:-1 tvg-id=\"d.test\" tvg-shift=\"junk\",Entry Junk\nhttp://iptv.test/d.ts\n";
    let server = MockServer::start().await;
    serve_text(&server, "/s.m3u", playlist.to_owned()).await;
    let e = env().await;
    e.svc
        .add_source(m3u_input(&format!("{}/s.m3u", server.uri()), None))
        .await
        .unwrap();
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT name, epg_shift_secs FROM channel ORDER BY name")
            .fetch_all(e.svc.store().pool())
            .await
            .unwrap();
    assert_eq!(
        rows,
        [
            ("Entry Junk".to_owned(), 5400),
            ("Entry Negative".to_owned(), -1800),
            ("Entry Zero".to_owned(), 0),
            ("Header Shift".to_owned(), 5400),
        ]
    );
}

// ---- guide feed caching ---------------------------------------------------------------------------

fn guide_xml(extra_title: &str) -> String {
    let now = downlink_core::sync::now_utc();
    let t = |o: i64| {
        chrono::DateTime::from_timestamp(now + o, 0)
            .unwrap()
            .format("%Y%m%d%H%M%S +0000")
            .to_string()
    };
    format!(
        "<tv><channel id=\"one.test\"><display-name>Channel One</display-name></channel>\
<programme start=\"{}\" stop=\"{}\" channel=\"one.test\"><title>{extra_title}</title></programme></tv>",
        t(-600),
        t(3000)
    )
}

async fn programme_count(e: &Env) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM programme")
        .fetch_one(e.svc.store().pool())
        .await
        .unwrap()
}

async fn add_m3u(server: &MockServer, e: &Env) -> i64 {
    e.svc
        .add_source(m3u_input(
            &format!("{}/get.php", server.uri()),
            Some(&format!("{}/epg.xml", server.uri())),
        ))
        .await
        .unwrap()
        .id
}

/// A feed the server says is unchanged (ETag -> 304) is neither downloaded nor re-imported.
#[tokio::test]
async fn unchanged_guide_feed_is_not_downloaded_again() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    Mock::given(method("GET"))
        .and(path("/epg.xml"))
        .and(wiremock::matchers::header("if-none-match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/epg.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_string(guide_xml("Now On One")),
        )
        .mount(&server)
        .await;
    let e = env().await;
    let sid = add_m3u(&server, &e).await;
    assert_eq!(programme_count(&e).await, 1);
    e.svc.sync_source(sid).await.unwrap();
    let epg: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/epg.xml")
        .collect();
    assert_eq!(epg.len(), 2);
    assert!(epg[0].headers.get("if-none-match").is_none());
    assert_eq!(
        epg[1]
            .headers
            .get("if-none-match")
            .unwrap()
            .to_str()
            .unwrap(),
        "\"v1\"",
        "the second sync asks whether the feed changed"
    );
    assert_eq!(programme_count(&e).await, 1, "the guide is kept");
    // No credentials or URLs are persisted with the validators.
    let kv: Vec<String> =
        sqlx::query_scalar("SELECT value FROM app_kv WHERE key LIKE 'guide_feed.%'")
            .fetch_all(e.svc.store().pool())
            .await
            .unwrap();
    assert_eq!(kv.len(), 1);
    assert!(
        !kv[0].contains("epg.xml") && !kv[0].contains("http"),
        "{}",
        kv[0]
    );
}

/// Without validators, identical content is still recognised and the import skipped; changed
/// content imports again.
#[tokio::test]
async fn identical_guide_content_skips_the_import_until_the_feed_changes() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    serve_text(&server, "/epg.xml", guide_xml("Now On One")).await;
    let e = env().await;
    let sid = add_m3u(&server, &e).await;
    assert_eq!(programme_count(&e).await, 1);

    // Same bytes: the import is skipped (a deleted row is not written back).
    sqlx::query("DELETE FROM programme")
        .execute(e.svc.store().pool())
        .await
        .unwrap();
    e.svc.sync_source(sid).await.unwrap();
    assert_eq!(
        programme_count(&e).await,
        0,
        "unchanged feed: import skipped"
    );

    // New content: imported.
    server.reset().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    serve_text(&server, "/epg.xml", guide_xml("Something New")).await;
    e.svc.sync_source(sid).await.unwrap();
    assert_eq!(programme_count(&e).await, 1, "changed feed: imported");
}

/// The same feed with changed matching inputs (channels or remaps) is imported again.
#[tokio::test]
async fn changed_channel_list_reimports_an_unchanged_guide() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    serve_text(&server, "/epg.xml", guide_xml("Now On One")).await;
    let e = env().await;
    let sid = add_m3u(&server, &e).await;
    sqlx::query("DELETE FROM programme")
        .execute(e.svc.store().pool())
        .await
        .unwrap();
    // The user points the unmatched channel at the feed's channel (a remap survives syncs).
    let ext: String = sqlx::query_scalar(
        "SELECT ext_id FROM channel WHERE source_id = ? AND name = 'No Group, No Id'",
    )
    .bind(sid)
    .fetch_one(e.svc.store().pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO epg_remap (source_id, channel_ext_id, xmltv_id) VALUES (?, ?, 'one.test')",
    )
    .bind(sid)
    .bind(&ext)
    .execute(e.svc.store().pool())
    .await
    .unwrap();
    e.svc.sync_source(sid).await.unwrap();
    assert!(
        programme_count(&e).await >= 1,
        "channel inputs changed: imported"
    );
}

#[tokio::test]
async fn unavailable_playlist_include_preserves_existing_channels() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let playlist = dir.path().join("top.m3u");
    std::fs::write(
        &playlist,
        format!(
            "#EXTM3U\n#EXTINF:-1 type=\"playlist\",Included\n{}/child.m3u\n",
            server.uri()
        ),
    )
    .unwrap();
    serve_text(&server, "/child.m3u", fx_text("m3u/basic.m3u")).await;

    let e = env().await;
    let src = e
        .svc
        .add_source(m3u_input(playlist.to_str().unwrap(), None))
        .await
        .unwrap();
    let before = names(&e).await;
    assert!(!before.is_empty());

    server.reset().await;
    assert!(e.svc.sync_source(src.id).await.is_err());
    assert_eq!(names(&e).await, before);
}

#[tokio::test]
async fn truncated_guide_does_not_replace_cached_validators() {
    let server = MockServer::start().await;
    serve_text(&server, "/get.php", fx_text("m3u/basic.m3u")).await;
    Mock::given(method("GET"))
        .and(path("/epg.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_string(guide_xml("Now On One")),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    let e = env().await;
    let sid = add_m3u(&server, &e).await;
    let state_key = format!("guide_feed.{sid}");
    let before = e.svc.store().kv_get(&state_key).await.unwrap().unwrap();

    let mut body = guide_xml("Updated programme");
    body.truncate(body.len() - "</tv>".len());
    Mock::given(method("GET"))
        .and(path("/epg.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v2\"")
                .set_body_string(body),
        )
        .with_priority(1)
        .mount(&server)
        .await;

    e.svc.sync_source(sid).await.unwrap();
    assert_eq!(
        e.svc.store().kv_get(&state_key).await.unwrap(),
        Some(before)
    );
    assert!(programme_count(&e).await >= 1);
}
