#![allow(clippy::unwrap_used, clippy::expect_used)]
//! M4 through the Service: VOD (movies, series, episodes, resume, next episode, play plans) and DVR
//! (recordings, scheduling, rules, in-progress playback), Dispatcharr and Xtream, wiremock only.

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::auth::MemorySecretStore;
use downlink_core::service::{
    AddSourceInput, ListChannelsArgs, PlayVodArgs, RecordKindArg, Service, VodKindArg, VodListArgs,
};
use downlink_core::store::{ProgrammeRow, Store};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_media_key_123456";

fn fx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/dispatcharr")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

async fn get(server: &MockServer, p: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn dispatcharr() -> (MockServer, Arc<Service>, i64, tempfile::TempDir) {
    let s = MockServer::start().await;
    get(&s, "/api/core/version/", fx("version.json")).await;
    get(&s, "/api/accounts/users/me/", fx("me.json")).await;
    get(&s, "/api/core/settings/", fx("settings.json")).await;
    get(&s, "/api/core/streamprofiles/", fx("stream_profiles.json")).await;
    get(&s, "/api/channels/groups/", fx("groups.json")).await;
    get(&s, "/api/epg/grid/", json!({"data": []})).await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json({
            let mut p = fx("channels_page1.json");
            p["next"] = Value::Null;
            p
        }))
        .mount(&s)
        .await;
    // VOD: categories, lists, details, episodes.
    get(&s, "/api/vod/categories/", fx("vod_categories_full.json")).await;
    Mock::given(method("GET"))
        .and(path("/api/vod/movies/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fx("vod_movies_page1.json")))
        .mount(&s)
        .await;
    get(
        &s,
        "/api/vod/movies/11/",
        fx("vod_movies_page1.json")["results"][0].clone(),
    )
    .await;
    get(&s, "/api/vod/series/", fx("vod_series_page1.json")).await;
    get(
        &s,
        "/api/vod/series/21/",
        fx("vod_series_page1.json")["results"][0].clone(),
    )
    .await;
    get(
        &s,
        "/api/vod/series/21/episodes/",
        fx("vod_episodes_page1.json"),
    )
    .await;
    // DVR
    get(&s, "/api/channels/recordings/", json!([])).await;
    get(
        &s,
        "/api/channels/recurring-rules/",
        fx("recurring_rules_full.json"),
    )
    .await;
    get(
        &s,
        "/api/channels/series-rules/",
        fx("series_rules_full.json"),
    )
    .await;
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
    let dir = tempfile::tempdir().unwrap();
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        Arc::new(MemorySecretStore::new()),
        dir.path().join("img"),
        "t".into(),
    );
    let input: AddSourceInput = serde_json::from_value(json!({"kind": "dispatcharr", "name": "S", "url": s.uri(), "apiKey": KEY, "allowSelfSigned": false})).unwrap();
    let id = svc.add_source(input).await.unwrap().id;
    (s, svc, id, dir)
}

#[tokio::test]
async fn caps_advertise_vod_and_dvr_when_the_account_can_use_them() {
    let (_s, svc, _id, _d) = dispatcharr().await;
    let v = serde_json::to_value(&svc.list_sources().await.unwrap()[0]).unwrap();
    let caps = v["caps"].as_u64().unwrap();
    assert!(caps & 8 != 0, "VOD (bit 3): {caps}");
    assert!(caps & 16 != 0, "DVR (bit 4): {caps}");
}

#[tokio::test]
async fn movies_categories_list_detail_and_dto_shape() {
    let (_s, svc, id, _d) = dispatcharr().await;
    let cats = svc.list_vod_categories(VodKindArg::Movie).await.unwrap();
    assert_eq!(
        cats.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Action", "Comedy"]
    );
    assert_eq!(
        cats[0].id,
        id * 1_000_000 + 1,
        "category ids encode the source"
    );

    let movies = svc
        .list_movies(VodListArgs {
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(movies.len(), 3);
    let v = serde_json::to_value(&movies[0]).unwrap();
    for k in [
        "id",
        "title",
        "year",
        "rating",
        "durationSecs",
        "poster",
        "backdrop",
        "plot",
        "genres",
        "categoryId",
        "resumeSecs",
        "watched",
    ] {
        assert!(v.get(k).is_some(), "missing {k}: {v}");
    }
    assert_eq!(
        (
            v["title"].as_str(),
            v["year"].as_i64(),
            v["rating"].as_f64(),
            v["durationSecs"].as_i64()
        ),
        (Some("Night Train"), Some(2021), Some(7.4), Some(5400))
    );
    assert_eq!(v["genres"], json!(["Action", "Thriller"]));
    assert!(v["poster"].as_str().unwrap().starts_with("dlimg://"));
    assert!(v["backdrop"].as_str().unwrap().starts_with("dlimg://"));
    assert!(v["resumeSecs"].is_null() && v["watched"] == false);
    assert!(
        movies[1].poster.is_none()
            && movies[1].rating.is_none()
            && movies[1].duration_secs.is_none()
    );
    assert!(movies[2].rating.is_none(), "'n/a' is not a rating");

    let one = svc.get_movie(movies[0].id).await.unwrap();
    assert_eq!(one.plot.as_deref(), Some("A synthetic thriller."));
    assert!(svc.get_movie(99_999).await.is_err());
}

#[tokio::test]
async fn series_detail_groups_seasons_and_picks_the_next_episode() {
    let (_s, svc, _id, _d) = dispatcharr().await;
    let series = svc.list_series(VodListArgs::default()).await.unwrap();
    assert_eq!(series[0].title, "Fieldwork");
    let d = svc.get_series(series[0].id).await.unwrap();
    let v = serde_json::to_value(&d).unwrap();
    assert!(
        v["seasons"].is_array()
            && v["series"]["seasonCount"].is_number()
            && v.get("nextEpisodeId").is_some()
    );
    let shape: Vec<(i64, Vec<i64>)> = d
        .seasons
        .iter()
        .map(|s| (s.number, s.episodes.iter().map(|e| e.number).collect()))
        .collect();
    assert_eq!(
        shape,
        [(0, vec![0]), (1, vec![1, 2]), (2, vec![1, 2])],
        "specials first, then seasons in order"
    );
    assert_eq!(d.series.season_count, 2);
    let ep = |n: usize, i: usize| d.seasons[n].episodes[i].id;
    // Nothing watched: start at the first episode (the season-less special sorts first).
    assert_eq!(d.next_episode_id, Some(ep(0, 0)));

    // Halfway through S1E2: continue with it.
    svc.report_vod_position(ep(1, 1), 1000, Some(2640))
        .await
        .unwrap();
    let d = svc.get_series(series[0].id).await.unwrap();
    assert_eq!(d.next_episode_id, Some(ep(1, 1)));
    assert_eq!(d.seasons[1].episodes[1].resume_secs, Some(1000));
    assert!(!d.seasons[1].episodes[1].watched);
    // Finished S1E2: continue with the one after it (S2E1).
    svc.report_vod_position(ep(1, 1), 2600, Some(2640))
        .await
        .unwrap();
    let d = svc.get_series(series[0].id).await.unwrap();
    assert!(d.seasons[1].episodes[1].watched);
    assert_eq!(d.next_episode_id, Some(ep(2, 0)));
    // Finished the very last episode: nothing next.
    svc.report_vod_position(ep(2, 1), 2700, Some(2700))
        .await
        .unwrap();
    assert_eq!(
        svc.get_series(series[0].id).await.unwrap().next_episode_id,
        None
    );
}

#[tokio::test]
async fn play_vod_plans_resume_and_the_continue_watching_tiles() {
    let (server, svc, _id, _d) = dispatcharr().await;
    let movies = svc.list_movies(VodListArgs::default()).await.unwrap();
    let m = movies[0].id;
    let before = server.received_requests().await.unwrap().len();
    let p = svc
        .play_vod(PlayVodArgs {
            kind: "movie".into(),
            id: m,
            resume: false,
        })
        .await
        .unwrap();
    assert_eq!(
        p.plan.url,
        format!(
            "{}/proxy/vod/movie/11111111-0000-4000-8000-000000000011",
            server.uri()
        )
    );
    assert_eq!(p.plan.headers[0].0, "X-API-Key");
    assert_eq!(p.resume_secs, None);
    assert_eq!(p.title, "Night Train");
    let n = server.received_requests().await.unwrap()[before..]
        .iter()
        .filter(|r| r.url.path().starts_with("/proxy/"))
        .count();
    assert_eq!(n, 0, "rule 4: a plan is never requested by core");

    // Resume only applies when asked for, and only for real positions.
    svc.report_vod_position(m, 5, Some(5400)).await.unwrap();
    assert_eq!(
        svc.play_vod(PlayVodArgs {
            kind: "movie".into(),
            id: m,
            resume: true
        })
        .await
        .unwrap()
        .resume_secs,
        None,
        "5 s is not worth resuming"
    );
    svc.report_vod_position(m, 1350, Some(5400)).await.unwrap();
    assert_eq!(
        svc.play_vod(PlayVodArgs {
            kind: "movie".into(),
            id: m,
            resume: true
        })
        .await
        .unwrap()
        .resume_secs,
        Some(1350)
    );
    assert_eq!(
        svc.play_vod(PlayVodArgs {
            kind: "movie".into(),
            id: m,
            resume: false
        })
        .await
        .unwrap()
        .resume_secs,
        None
    );

    // An episode, with the series title in the player title.
    let series = svc.list_series(VodListArgs::default()).await.unwrap();
    let d = svc.get_series(series[0].id).await.unwrap();
    let e = d.seasons[1].episodes[0].id;
    let ep = svc
        .play_vod(PlayVodArgs {
            kind: "episode".into(),
            id: e,
            resume: true,
        })
        .await
        .unwrap();
    assert!(
        ep.plan
            .url
            .ends_with("/proxy/vod/episode/33333333-0000-4000-8000-000000000301")
    );
    assert_eq!(ep.title, "Fieldwork Pilot");
    svc.report_vod_position(e, 600, Some(2700)).await.unwrap();

    // Continue watching tiles for the movie and the episode, with art and progress.
    let home = serde_json::to_value(svc.get_home().await.unwrap()).unwrap();
    let cw = home["continueWatching"].as_array().unwrap();
    let movie = cw.iter().find(|i| i["kind"] == "movie").unwrap();
    assert_eq!(movie["title"], "Night Train");
    assert_eq!(movie["progress"], 0.25);
    assert_eq!(movie["remainingSecs"], 4050);
    assert!(movie["art"].as_str().unwrap().starts_with("dlimg://"));
    let ep_tile = cw.iter().find(|i| i["kind"] == "episode").unwrap();
    assert_eq!(ep_tile["subtitle"], "Fieldwork · S1 E1");
    assert!(
        svc.play_vod(PlayVodArgs {
            kind: "series".into(),
            id: m,
            resume: false
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn recordings_list_groups_by_state_and_flags_scheduled_programmes() {
    let (server, svc, _id, _d) = dispatcharr().await;
    server.reset().await;
    // After the reset only the recordings endpoint matters.
    let mut recs = fx("recordings_full.json");
    // Recording 31 belongs to the channel we create a matching programme for.
    let chans = svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let ch = svc.store().channel(chans[0].id).await.unwrap().unwrap();
    for r in recs.as_array_mut().unwrap() {
        r["channel"] = json!(ch.ext_id.parse::<i64>().unwrap());
    }
    let start = 4_070_980_800i64; // 2099-01-01T20:00:00Z, recording 31's start
    svc.store()
        .upsert_programmes(&[ProgrammeRow {
            channel_id: ch.id,
            start_utc: start,
            end_utc: start + 3600,
            ext_id: Some("p1".into()),
            title: Some("Future Show".into()),
            subtitle: None,
            description: None,
            season: None,
            episode: None,
            flags: 0,
        }])
        .await
        .unwrap();
    get(&server, "/api/channels/recordings/", recs).await;
    get(&server, "/api/channels/recurring-rules/", json!([])).await;
    get(&server, "/api/channels/series-rules/", json!({"rules": []})).await;

    let r = svc.list_recordings().await.unwrap();
    let v = serde_json::to_value(&r).unwrap();
    for k in ["upcoming", "inProgress", "completed"] {
        assert!(v[k].is_array(), "{k}");
    }
    assert_eq!(r.upcoming.len(), 1);
    assert_eq!(r.in_progress.len(), 1);
    assert_eq!(
        r.completed.len(),
        3,
        "completed + failed + derived-from-times; the unreadable row is skipped"
    );
    let up = &v["upcoming"][0];
    for k in [
        "id",
        "title",
        "channelId",
        "channelName",
        "startUtc",
        "endUtc",
        "status",
        "ruleId",
        "ruleLabel",
        "sizeBytes",
        "error",
    ] {
        assert!(up.get(k).is_some(), "missing {k}");
    }
    assert_eq!(
        (
            up["title"].as_str(),
            up["status"].as_str(),
            up["channelName"].as_str()
        ),
        (
            Some("Future Show"),
            Some("scheduled"),
            Some(ch.name.as_str())
        )
    );
    assert_eq!(r.in_progress[0].rule_label.as_deref(), Some("Series"));
    let failed = r.completed.iter().find(|c| c.status == "failed").unwrap();
    assert_eq!(
        failed.error.as_deref(),
        Some("Provider refused the connection.")
    );
    assert_eq!(
        r.completed
            .iter()
            .find(|c| c.size_bytes.is_some())
            .unwrap()
            .size_bytes,
        Some(123_456_789)
    );

    // The guide now shows the programme as scheduled.
    let progs = svc
        .get_programmes(ch.id, start - 10, start + 4000)
        .await
        .unwrap();
    assert!(
        progs[0].is_scheduled,
        "isScheduled follows the recordings table"
    );
    let nn = svc.get_now_next(&[ch.id]).await.unwrap();
    assert_eq!(nn.len(), 1);
}

#[tokio::test]
async fn schedule_once_and_series_cancel_stop_extend_delete() {
    let (server, svc, _id, _d) = dispatcharr().await;
    let chans = svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let ch = svc.store().channel(chans[0].id).await.unwrap().unwrap();
    let start = downlink_core::sync::now_utc() + 3600;
    svc.store()
        .upsert_programmes(&[ProgrammeRow {
            channel_id: ch.id,
            start_utc: start,
            end_utc: start + 1800,
            ext_id: Some("px".into()),
            title: Some("Fieldwork".into()),
            subtitle: None,
            description: Some("d".into()),
            season: None,
            episode: None,
            flags: 0,
        }])
        .await
        .unwrap();
    let pid = format!("{}:{start}", ch.id);
    Mock::given(method("POST")).and(path("/api/channels/recordings/")).and(header("X-API-Key", KEY))
        .and(body_partial_json(json!({"channel": ch.ext_id.parse::<i64>().unwrap(), "custom_properties": {"program": {"title": "Fieldwork"}}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 50}))).expect(1).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/"))
        .and(body_partial_json(
            json!({"title": "Fieldwork", "mode": "new"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .expect(1)
        .mount(&server)
        .await;
    // The server now reports three recordings in the three live states.
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    let now = downlink_core::sync::now_utc();
    let c = ch.ext_id.parse::<i64>().unwrap();
    server.reset().await; // drop the generic empty list set up by the helper
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 50})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/series-rules/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&server)
        .await;
    get(&server, "/api/channels/recordings/", json!([
        {"id": 50, "start_time": iso(start), "end_time": iso(start + 1800), "channel": c, "custom_properties": {"status": "scheduled", "program": {"title": "Fieldwork"}}},
        {"id": 51, "start_time": iso(now - 600), "end_time": iso(now + 600), "channel": c, "custom_properties": {"status": "recording", "program": {"title": "Running"}}},
        {"id": 52, "start_time": iso(now - 9000), "end_time": iso(now - 7200), "channel": c, "custom_properties": {"status": "completed", "program": {"title": "Done"}}},
    ])).await;
    get(
        &server,
        "/api/channels/series-rules/",
        json!({"rules": [{"tvg_id": "x", "mode": "new", "title": "Fieldwork", "channel_id": c}]}),
    )
    .await;
    get(&server, "/api/channels/recurring-rules/", json!([])).await;

    svc.schedule_recording(&pid, RecordKindArg::Once)
        .await
        .unwrap();
    svc.schedule_recording(&pid, RecordKindArg::Series)
        .await
        .unwrap();
    let r = svc.list_recordings().await.unwrap();
    let (up, run, done) = (r.upcoming[0].id, r.in_progress[0].id, r.completed[0].id);

    // Wrong-state actions are refused locally, with a sentence.
    assert!(svc.stop_recording(up).await.is_err());
    assert!(svc.extend_recording(up, 10).await.is_err());
    assert!(svc.cancel_recording(run).await.is_err());

    Mock::given(method("DELETE"))
        .and(path("/api/channels/recordings/50/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/51/stop/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/channels/recordings/51/extend/"))
        .and(body_partial_json(json!({"extra_minutes": 30})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/recordings/52/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    svc.cancel_recording(up).await.unwrap();
    svc.stop_recording(run).await.unwrap();
    svc.extend_recording(run, 30).await.unwrap();
    svc.delete_recording(done).await.unwrap();
    assert!(svc.cancel_recording(99_999).await.is_err());
}

#[tokio::test]
async fn rules_list_with_channel_names_and_delete() {
    let (server, svc, _id, _d) = dispatcharr().await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/recurring-rules/7/"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/api/channels/series-rules/"))
        .and(query_param("title", "Quiz Hour"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let rules = svc.list_recording_rules().await.unwrap();
    let v = serde_json::to_value(&rules).unwrap();
    for k in ["id", "title", "channelName", "kind", "schedule"] {
        assert!(v[0].get(k).is_some(), "missing {k}");
    }
    let rec = rules.iter().find(|r| r.kind == "recurring").unwrap();
    assert_eq!(
        (rec.title.as_str(), rec.schedule.as_str()),
        ("Evening News", "Weekdays 19:00")
    );
    let ser = rules.iter().find(|r| r.title == "Quiz Hour").unwrap();
    assert_eq!(
        (ser.kind, ser.schedule.as_str()),
        ("series", "All episodes")
    );
    svc.delete_recording_rule(rec.id).await.unwrap();
    svc.delete_recording_rule(ser.id).await.unwrap();
    assert!(svc.delete_recording_rule(424_242).await.is_err());
}

#[tokio::test]
async fn play_recording_in_progress_hls_and_completed_file_with_resume() {
    let (server, svc, _id, _d) = dispatcharr().await;
    let chans = svc
        .list_channels(ListChannelsArgs::default())
        .await
        .unwrap();
    let c = svc
        .store()
        .channel(chans[0].id)
        .await
        .unwrap()
        .unwrap()
        .ext_id
        .parse::<i64>()
        .unwrap();
    server.reset().await;
    let now = downlink_core::sync::now_utc();
    let iso = |t: i64| {
        chrono::DateTime::from_timestamp(t, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    };
    get(&server, "/api/channels/recordings/", json!([
        {"id": 61, "start_time": iso(now - 600), "end_time": iso(now + 600), "channel": c, "custom_properties": {"status": "recording", "file_url": "/api/channels/recordings/61/hls/index.m3u8", "program": {"title": "Running"}}},
        {"id": 62, "start_time": iso(now - 9000), "end_time": iso(now - 5400), "channel": c, "custom_properties": {"status": "completed", "program": {"title": "Done"}}},
        {"id": 63, "start_time": iso(now + 9000), "end_time": iso(now + 12000), "channel": c, "custom_properties": {"status": "scheduled"}},
    ])).await;
    get(&server, "/api/channels/recurring-rules/", json!([])).await;
    get(&server, "/api/channels/series-rules/", json!({"rules": []})).await;
    let r = svc.list_recordings().await.unwrap();
    let live = svc.play_recording(r.in_progress[0].id).await.unwrap();
    assert!(live.in_progress);
    assert_eq!(
        live.plan.url,
        format!("{}/api/channels/recordings/61/hls/index.m3u8", server.uri())
    );
    assert_eq!(live.plan.headers[0].0, "X-API-Key");
    let done_id = r.completed[0].id;
    let done = svc.play_recording(done_id).await.unwrap();
    assert!(done.plan.url.ends_with("/api/channels/recordings/62/file/") && !done.in_progress);
    assert_eq!(done.resume_secs, None);
    svc.report_recording_position(done_id, 900, None)
        .await
        .unwrap();
    assert_eq!(
        svc.play_recording(done_id).await.unwrap().resume_secs,
        Some(900)
    );
    assert!(
        svc.play_recording(r.upcoming[0].id).await.is_err(),
        "nothing to watch yet"
    );
    let home = serde_json::to_value(svc.get_home().await.unwrap()).unwrap();
    let tile = home["continueWatching"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "recording")
        .unwrap()
        .clone();
    assert_eq!(tile["title"], "Done");
    assert!(tile["progress"].as_f64().unwrap() > 0.0);
}

// ---- Xtream VOD -------------------------------------------------------------------------------

fn xtream_fx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/xtream")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[tokio::test]
async fn xtream_vod_through_the_service() {
    let s = MockServer::start().await;
    struct NoAction;
    impl wiremock::Match for NoAction {
        fn matches(&self, r: &wiremock::Request) -> bool {
            !r.url.query_pairs().any(|(k, _)| k == "action")
        }
    }
    Mock::given(method("GET"))
        .and(path("/player_api.php"))
        .and(NoAction)
        .respond_with(ResponseTemplate::new(200).set_body_json(xtream_fx("player_api_login.json")))
        .mount(&s)
        .await;
    for (a, body) in [
        ("get_live_categories", xtream_fx("get_live_categories.json")),
        ("get_live_streams", xtream_fx("get_live_streams.json")),
        (
            "get_vod_categories",
            json!([{"category_id": "1", "category_name": "Test Entertainment", "parent_id": 0}]),
        ),
        ("get_vod_streams", xtream_fx("get_vod_streams.json")),
        (
            "get_series_categories",
            json!([{"category_id": "1", "category_name": "Test Series", "parent_id": 0}]),
        ),
        ("get_series", xtream_fx("get_series.json")),
        ("get_series_info", xtream_fx("get_series_info.json")),
    ] {
        Mock::given(method("GET"))
            .and(path("/player_api.php"))
            .and(query_param("action", a))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&s)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/xmltv.php"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<tv/>"))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        Arc::new(MemorySecretStore::new()),
        dir.path().join("img"),
        "t".into(),
    );
    let input: AddSourceInput = serde_json::from_value(json!({"kind": "xtream", "name": "Panel", "url": s.uri(), "username": "testuser", "password": "testpass", "allowSelfSigned": false})).unwrap();
    svc.add_source(input).await.unwrap();
    let v = serde_json::to_value(&svc.list_sources().await.unwrap()[0]).unwrap();
    assert!(
        v["caps"].as_u64().unwrap() & 8 != 0,
        "VOD advertised: the panel has movies and series"
    );
    assert!(v["caps"].as_u64().unwrap() & 16 == 0, "no DVR on Xtream");

    let cats = svc.list_vod_categories(VodKindArg::Movie).await.unwrap();
    assert!(cats[0].count >= 1, "counts come from the panel's list");
    let movies = svc
        .list_movies(VodListArgs {
            limit: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(movies.len(), 2);
    assert_eq!(movies[0].title, "Test Movie 1 (2021)");
    assert_eq!(movies[0].year, Some(2021), "year recovered from the title");
    let searched = svc
        .list_movies(VodListArgs {
            query: Some("movie 2".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(searched.len(), 1);
    let paged = svc
        .list_movies(VodListArgs {
            offset: Some(1),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(paged[0].title, "Test Movie 2 (2022)");

    let p = svc
        .play_vod(PlayVodArgs {
            kind: "movie".into(),
            id: movies[0].id,
            resume: false,
        })
        .await
        .unwrap();
    assert_eq!(
        p.plan.url,
        format!("{}/movie/testuser/testpass/2001.mkv", s.uri())
    );
    assert!(
        !serde_json::to_string(&p.plan.redacted())
            .unwrap()
            .contains("testpass")
    );

    let series = svc.list_series(VodListArgs::default()).await.unwrap();
    assert_eq!(series[0].title, "Test Series 1");
    let d = svc.get_series(series[0].id).await.unwrap();
    assert!(!d.seasons.is_empty());
    let first = &d.seasons[0].episodes[0];
    let ep = svc
        .play_vod(PlayVodArgs {
            kind: "episode".into(),
            id: first.id,
            resume: false,
        })
        .await
        .unwrap();
    assert!(
        ep.plan.url.contains("/series/testuser/testpass/"),
        "{}",
        ep.plan.url.replace("testpass", "***")
    );
    // recordings are not an Xtream thing
    assert!(svc.list_recordings().await.unwrap().upcoming.is_empty());
}

// ---- the provider-credential blob never reaches SQLite -----------------------------------------

#[tokio::test]
async fn provider_credentials_in_the_episodes_payload_never_reach_sqlite() {
    let (server, _svc, _id, _d) = dispatcharr().await;
    server.reset().await;
    // A file-backed Service so the raw database files can be scanned.
    get(&server, "/api/core/version/", fx("version.json")).await;
    get(&server, "/api/accounts/users/me/", fx("me.json")).await;
    get(&server, "/api/core/settings/", fx("settings.json")).await;
    get(
        &server,
        "/api/core/streamprofiles/",
        fx("stream_profiles.json"),
    )
    .await;
    get(&server, "/api/channels/groups/", fx("groups.json")).await;
    get(&server, "/api/epg/grid/", json!({"data": []})).await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    get(
        &server,
        "/api/vod/categories/",
        fx("vod_categories_full.json"),
    )
    .await;
    get(
        &server,
        "/api/vod/movies/",
        json!({"count": 0, "next": null, "results": []}),
    )
    .await;
    get(
        &server,
        "/api/vod/series/",
        fx("vod_series_reference_shape.json"),
    )
    .await;
    get(
        &server,
        "/api/vod/series/21/",
        fx("vod_series_reference_shape.json")["results"][0].clone(),
    )
    .await;
    get(
        &server,
        "/api/vod/series/21/episodes/",
        fx("vod_episodes_reference_shape.json"),
    )
    .await;
    get(&server, "/api/channels/recordings/", json!([])).await;
    get(&server, "/api/channels/recurring-rules/", json!([])).await;
    get(&server, "/api/channels/series-rules/", json!({"rules": []})).await;
    Mock::given(method("GET"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(
            "/api/accounts/users/me/app-data/com.downlink.state.v1/",
        ))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let svc = Service::new(downlink_core::service::ServiceConfig {
        data_dir: dir.path().to_path_buf(),
        secrets: Arc::new(MemorySecretStore::new()),
        app_version: "t".into(),
    })
    .await
    .unwrap();
    let input: AddSourceInput = serde_json::from_value(json!({"kind": "dispatcharr", "name": "S", "url": server.uri(), "apiKey": KEY, "allowSelfSigned": false})).unwrap();
    svc.add_source(input).await.unwrap();
    let series = svc.list_series(VodListArgs::default()).await.unwrap();
    assert_eq!(series[0].title, "Fieldwork");
    let d = svc.get_series(series[0].id).await.unwrap();
    assert_eq!(d.seasons.iter().map(|s| s.episodes.len()).sum::<usize>(), 3);
    let first = d.seasons[0].episodes[0].id;
    let p = svc
        .play_vod(PlayVodArgs {
            kind: "episode".into(),
            id: first,
            resume: false,
        })
        .await
        .unwrap();
    assert!(
        p.plan.url.contains("/proxy/vod/episode/33333333-"),
        "played via the uuid, not the provider's direct source"
    );
    assert!(
        !serde_json::to_string(&d)
            .unwrap()
            .contains("FAKEPROVIDERPASS")
    );

    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(svc.store().pool())
        .await
        .unwrap();
    for e in std::fs::read_dir(dir.path()).unwrap() {
        let p = e.unwrap().path();
        if let Ok(bytes) = std::fs::read(&p) {
            for secret in [
                "FAKEPROVIDERPASS-7731",
                "fakeprovideruser",
                "provider.example.test",
            ] {
                assert!(
                    !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                    "{secret} found in {p:?}"
                );
            }
        }
    }
}

// ---- captured 0.34.1 behaviour through the Service ----------------------------------------------

fn tfx(name: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/dispatcharr")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// A Dispatcharr with only the connect/sync basics mounted, so a test mounts its own VOD/DVR mocks.
async fn bare_server() -> MockServer {
    let s = MockServer::start().await;
    get(&s, "/api/core/version/", fx("version.json")).await;
    get(&s, "/api/accounts/users/me/", fx("me.json")).await;
    get(&s, "/api/core/settings/", fx("settings.json")).await;
    get(&s, "/api/core/streamprofiles/", fx("stream_profiles.json")).await;
    get(&s, "/api/channels/groups/", fx("groups.json")).await;
    get(&s, "/api/epg/grid/", json!({"data": []})).await;
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&s)
        .await;
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
    s
}

async fn svc_for(server: &MockServer) -> Arc<Service> {
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let svc = Service::with_store(
        Store::open_memory().await.unwrap(),
        Arc::new(MemorySecretStore::new()),
        dir.path().join("img"),
        "t".into(),
    );
    let input: AddSourceInput = serde_json::from_value(json!({"kind": "dispatcharr", "name": "S", "url": server.uri(), "apiKey": KEY, "allowSelfSigned": false})).unwrap();
    svc.add_source(input).await.unwrap();
    svc
}

#[tokio::test]
async fn movie_details_are_lazy_provider_info_fills_them_once() {
    let s = bare_server().await;
    let mut movies = tfx("vod_movies.json");
    for m in movies["results"].as_array_mut().unwrap() {
        m["description"] = json!("");
        m["genre"] = Value::Null;
        m["duration_secs"] = Value::Null;
    }
    let row = movies["results"][0].clone();
    let id = row["id"].as_i64().unwrap();
    get(&s, "/api/vod/categories/", tfx("vod_categories.json")).await;
    get(&s, "/api/vod/movies/", movies).await;
    get(&s, "/api/vod/series/", json!({"count": 0, "results": []})).await;
    get(&s, &format!("/api/vod/movies/{id}/"), row).await;
    let mut info = tfx("vod_movie_provider_info.json");
    info["id"] = json!(id);
    Mock::given(method("GET"))
        .and(path(format!("/api/vod/movies/{id}/provider-info/")))
        .and(header("X-API-Key", KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(info))
        .expect(1)
        .mount(&s)
        .await;
    let svc = svc_for(&s).await;

    let list = svc.list_movies(VodListArgs::default()).await.unwrap();
    let first = list
        .iter()
        .find(|m| m.title.contains("Documentary Three"))
        .unwrap();
    assert!(
        first.plot.is_none() && first.genres.is_empty() && first.duration_secs.is_none(),
        "empty before provider-info"
    );
    let detail = svc.get_movie(first.id).await.unwrap();
    assert_eq!(
        detail.plot.as_deref(),
        Some("Synthetic plot for Test Documentary Three.")
    );
    assert_eq!(detail.genres, ["Test"]);
    assert_eq!(detail.duration_secs, Some(20));
    // Cached: a second call does not ask the provider again (expect(1) above).
    svc.get_movie(first.id).await.unwrap();
}

#[tokio::test]
async fn series_episodes_are_lazy_empty_until_provider_info_has_run() {
    let s = bare_server().await;
    get(&s, "/api/vod/categories/", tfx("vod_categories.json")).await;
    get(&s, "/api/vod/movies/", json!({"count": 0, "results": []})).await;
    get(&s, "/api/vod/series/", tfx("vod_series.json")).await;
    let series_id = tfx("vod_series.json")["results"][0]["id"].as_i64().unwrap();
    get(
        &s,
        &format!("/api/vod/series/{series_id}/"),
        tfx("vod_series_detail.json"),
    )
    .await;
    // First read: [] (nothing fetched yet). After provider-info: the real list.
    Mock::given(method("GET"))
        .and(path(format!("/api/vod/series/{series_id}/episodes/")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/vod/series/{series_id}/episodes/")))
        .respond_with(ResponseTemplate::new(200).set_body_json(tfx("vod_series_episodes.json")))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/api/vod/series/{series_id}/provider-info/")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(tfx("vod_series_provider_info.json")),
        )
        .expect(1)
        .mount(&s)
        .await;
    let svc = svc_for(&s).await;
    let list = svc.list_series(VodListArgs::default()).await.unwrap();
    let d = svc.get_series(list[0].id).await.unwrap();
    let total: usize = d.seasons.iter().map(|x| x.episodes.len()).sum();
    assert_eq!(total, 5, "episodes appear after provider-info");
    assert!(d.next_episode_id.is_some());
    assert!(
        !serde_json::to_string(&d).unwrap().contains("password"),
        "the embedded provider account never reaches the DTO"
    );
}

#[tokio::test]
async fn recording_rows_link_to_their_rules_and_play_via_the_captured_urls() {
    let s = bare_server().await;
    get(&s, "/api/vod/movies/", json!({"count": 0, "results": []})).await;
    get(&s, "/api/vod/series/", json!({"count": 0, "results": []})).await;
    get(&s, "/api/channels/recordings/", tfx("recordings_list.json")).await;
    get(
        &s,
        "/api/channels/recurring-rules/",
        tfx("recurring_rules.json"),
    )
    .await;
    get(&s, "/api/channels/series-rules/", json!({"rules": [{"tvg_id": "test-news.test", "mode": "all", "title": "Pattern Playhouse", "title_mode": "exact"}]})).await;
    let svc = svc_for(&s).await;
    let rules = svc.list_recording_rules().await.unwrap();
    let recurring = rules.iter().find(|r| r.kind == "recurring").unwrap().id;
    let series = rules.iter().find(|r| r.kind == "series").unwrap().id;
    let r = svc.list_recordings().await.unwrap();
    let weekday = r
        .upcoming
        .iter()
        .find(|x| x.title == "Synthetic Weekday Rule")
        .unwrap();
    assert_eq!(
        (weekday.rule_id, weekday.rule_label.as_deref()),
        (Some(recurring), Some("Recurring"))
    );
    let pp = r
        .upcoming
        .iter()
        .find(|x| x.title == "Pattern Playhouse")
        .unwrap();
    assert_eq!(
        (pp.rule_id, pp.rule_label.as_deref()),
        (Some(series), Some("Series")),
        "series recordings match by title + tvg_id"
    );
    let plain = r
        .upcoming
        .iter()
        .find(|x| x.title == "Synthetic Upcoming Recording");
    if let Some(p) = plain {
        assert_eq!(p.rule_id, None);
    }
    // Stopped = finished, plays from /file/.
    let stopped = r
        .completed
        .iter()
        .find(|x| x.title == "Synthetic In Progress Recording")
        .unwrap();
    let play = svc.play_recording(stopped.id).await.unwrap();
    assert!(play.plan.url.ends_with("/api/channels/recordings/29/file/"));
    assert_eq!(play.plan.headers[0].0, "X-API-Key");
}
