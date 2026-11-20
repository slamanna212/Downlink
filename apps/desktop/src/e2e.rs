//! Developer end-to-end driver (feature `e2e`): runs the real command path (Service + Playback +
//! libmpv + surface) against a local test server without a UI, printing `E2E {json}` lines.
//! Configured by env: DOWNLINK_E2E=full|offline, DOWNLINK_E2E_URL, DOWNLINK_E2E_KEY (never printed).
//! Always quits by itself; keep the window small (`DOWNLINK_WINDOW_SIZE=960x600`).

use std::process::Command;
use std::time::{Duration, Instant};

use downlink_core::service::{AddSourceInput, ListChannelsArgs, Service};
use downlink_player::{PlayerCommand, SessionState};
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

fn say(v: serde_json::Value) {
    println!("E2E {v}");
}

fn open_sessions(db: &str) -> String {
    Command::new("sqlite3")
        .args([db, "select count(*) from open_session"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "?".into())
}

pub fn spawn(app: AppHandle, mode: String) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(4)).await;
        let st = app.state::<AppState>();
        let db = std::env::var("DOWNLINK_DATA_DIR")
            .map(|d| format!("{d}/downlink.sqlite"))
            .unwrap_or_default();
        let r = match mode.as_str() {
            "offline" => offline(&st, &db).await,
            "zap" => zap(&st).await,
            "multiview" => multiview(&st).await,
            "failover" => failover(&st).await,
            "smoke" => smoke(&st).await,
            "perf-setup" => perf_setup(&st).await,
            "perf" => perf(&st).await,
            "launch" => launch(&st).await,
            "degraded" => degraded(&st).await,
            "hls" => hls(&st, &db).await,
            "vod" => vod(&st).await,
            _ => full(&st, &db).await,
        };
        if let Err(e) = r {
            say(json!({ "step": "error", "message": e }));
        }
        if let Some(pb) = st.try_playback() {
            let _ = pb.stop().await;
        }
        app.exit(0);
    });
}

async fn channel_id(st: &AppState, name: &str) -> Result<i64, String> {
    let list = st
        .service_now()
        .list_channels(ListChannelsArgs {
            query: Some(name.to_owned()),
            limit: Some(20),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    list.iter()
        .find(|c| c.name == name)
        .map(|c| c.id)
        .ok_or_else(|| format!("channel {name} not found"))
}

async fn full(st: &AppState, db: &str) -> Result<(), String> {
    let url = std::env::var("DOWNLINK_E2E_URL").map_err(|_| "DOWNLINK_E2E_URL missing")?;
    let key = std::env::var("DOWNLINK_E2E_KEY").map_err(|_| "DOWNLINK_E2E_KEY missing")?;
    let sources = st
        .service_now()
        .list_sources()
        .await
        .map_err(|e| e.message)?;
    if sources.is_empty() {
        let t0 = Instant::now();
        let s = st
            .service_now()
            .add_source(AddSourceInput {
                kind: "dispatcharr".into(),
                name: "E2E".into(),
                url,
                api_key: Some(key),
                username: None,
                password: None,
                xmltv_url: None,
                allow_self_signed: false,
            })
            .await
            .map_err(|e| e.message)?;
        say(
            json!({ "step": "add_source", "id": s.id, "status": s.status, "serverVersion": s.server_version, "ms": t0.elapsed().as_millis() }),
        );
    }
    let channels = st
        .service_now()
        .list_channels(ListChannelsArgs {
            limit: Some(1000),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    let groups = st
        .service_now()
        .list_groups(None)
        .await
        .map_err(|e| e.message)?;
    say(json!({ "step": "catalog", "channels": channels.len(), "groups": groups.len() }));

    let pb = st.playback().await.map_err(|e| e.message)?;
    // Zap between channels (live), including the Redirect-profile channel.
    for name in [
        "Channel One",
        "Test News",
        "Redirect One",
        "Channel One",
        "Test News",
    ] {
        let id = channel_id(st, name).await?;
        let t0 = Instant::now();
        pb.play_channel(id).await.map_err(|e| e.message)?;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(20),
        );
        say(
            json!({ "step": "zap", "channel": name, "state": snap.state, "firstFrameMs": snap.first_frame_ms, "wallMs": t0.elapsed().as_millis(), "hwdec": snap.hwdec_current, "codec": snap.video_codec, "size": [snap.width, snap.height] }),
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }

    // Catch-up on the XC channel: play, seek, then zap away and check the session is released.
    let xc = channel_id(st, "XC Catchup").await?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let w = downlink_core::source::TimeWindow {
        start_utc: now - 3 * 3600,
        end_utc: now,
    };
    let rows = downlink_core::epg::window(st.service_now().store(), &[xc], w)
        .await
        .map_err(|e| e.message)?;
    let start = match rows.iter().rev().find(|r| r.end_utc <= now) {
        Some(r) => r.start_utc,
        None => {
            // The test server's EPG has no XC programmes in the cache; insert one archived hour so
            // the catch-up session flow can still be exercised (times come from the row).
            let start = (now - 2 * 3600) / 3600 * 3600;
            st.service_now()
                .store()
                .upsert_programmes(&[downlink_core::store::ProgrammeRow {
                    channel_id: xc,
                    start_utc: start,
                    end_utc: start + 3600,
                    ext_id: None,
                    title: Some("E2E archive hour".into()),
                    subtitle: None,
                    description: None,
                    season: None,
                    episode: None,
                    flags: 0,
                }])
                .await
                .map_err(|e| e.message)?;
            say(
                json!({ "step": "note", "message": "no cached XC programmes; inserted one archived hour" }),
            );
            start
        }
    };
    let pid = format!("{xc}:{start}");
    let t0 = Instant::now();
    pb.play_catchup(&pid, false).await.map_err(|e| e.message)?;
    let snap = pb.snapshot_wait(
        |s| matches!(s, SessionState::Playing | SessionState::Failed),
        Duration::from_secs(25),
    );
    say(
        json!({ "step": "catchup_play", "state": snap.state, "firstFrameMs": snap.first_frame_ms, "wallMs": t0.elapsed().as_millis(), "durationSecs": snap.duration_secs, "mode": snap.mode, "openSessions": open_sessions(db) }),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let before = pb.snapshot().position_secs;
    pb.command(PlayerCommand::SeekAbsolute {
        secs: before.unwrap_or(0.0) + 30.0,
    })
    .map_err(|e| e.message)?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = pb.snapshot();
    say(
        json!({ "step": "catchup_seek", "before": before, "after": after.position_secs, "state": after.state }),
    );
    pb.command(PlayerCommand::Pause).map_err(|e| e.message)?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    pb.command(PlayerCommand::Resume).map_err(|e| e.message)?;
    // Zap away: the catch-up session must be stopped.
    let live = channel_id(st, "Channel One").await?;
    pb.play_channel(live).await.map_err(|e| e.message)?;
    tokio::time::sleep(Duration::from_secs(4)).await;
    say(
        json!({ "step": "after_zap", "openSessions": open_sessions(db), "state": pb.snapshot().state }),
    );
    // Leave a catch-up session open and quit: RunEvent::Exit must release it.
    pb.play_catchup(&pid, true).await.map_err(|e| e.message)?;
    pb.snapshot_wait(|s| s == SessionState::Playing, Duration::from_secs(25));
    say(json!({ "step": "before_quit", "openSessions": open_sessions(db) }));
    Ok(())
}

async fn offline(st: &AppState, _db: &str) -> Result<(), String> {
    let sources = st
        .service_now()
        .list_sources()
        .await
        .map_err(|e| e.message)?;
    say(
        json!({ "step": "sources_cached", "sources": sources.iter().map(|s| json!({ "name": s.name, "status": s.status })).collect::<Vec<_>>() }),
    );
    let channels = st
        .service_now()
        .list_channels(ListChannelsArgs {
            limit: Some(1000),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    say(json!({ "step": "channels_from_cache", "count": channels.len() }));
    if let Some(s) = sources.first() {
        let r = st.service_now().sync_source(s.id).await;
        say(
            json!({ "step": "sync_while_offline", "error": r.err().map(|e| json!({ "code": e.code, "message": e.message })) }),
        );
        let after = st
            .service_now()
            .list_sources()
            .await
            .map_err(|e| e.message)?;
        say(json!({ "step": "status_after_sync", "status": after.first().map(|s| s.status) }));
    }
    let pb = st.playback().await.map_err(|e| e.message)?;
    if let Some(c) = channels.first() {
        let r = pb.play_channel(c.id).await;
        say(
            json!({ "step": "play_while_offline", "error": r.err().map(|e| json!({ "code": e.code, "message": e.message })), "state": pb.snapshot().state }),
        );
    }
    Ok(())
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    sorted
        .get(((sorted.len() as f64 - 1.0) * p).round() as usize)
        .copied()
}

/// `DOWNLINK_E2E=zap`: N zaps through `Playback::play_channel` between the channels named in
/// `DOWNLINK_E2E_ZAP` (comma separated, default two), `DOWNLINK_E2E_ZAP_COUNT` zaps (default 20).
/// Measures the command phase (call -> `loadfile` issued) and first frame separately.
async fn zap(st: &AppState) -> Result<(), String> {
    let names =
        std::env::var("DOWNLINK_E2E_ZAP").unwrap_or_else(|_| "Channel One,Test News".into());
    let count: usize = std::env::var("DOWNLINK_E2E_ZAP_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let pb = st.playback().await.map_err(|e| e.message)?;
    let mut ids = Vec::new();
    for n in names.split(',') {
        ids.push((n.to_owned(), channel_id(st, n.trim()).await?));
    }
    let (mut cmd_ms, mut first_ms, mut failed) = (Vec::new(), Vec::new(), 0);
    for i in 0..count {
        let (name, id) = &ids[i % ids.len()];
        let t0 = Instant::now();
        pb.play_channel(*id).await.map_err(|e| e.message)?;
        let cmd = t0.elapsed().as_millis() as u64;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(20),
        );
        match (snap.state, snap.first_frame_ms) {
            (Some(SessionState::Playing), Some(ff)) => {
                cmd_ms.push(cmd);
                first_ms.push(ff);
            }
            _ => failed += 1,
        }
        say(
            json!({ "step": "zap", "i": i, "channel": name, "commandMs": cmd, "firstFrameMs": snap.first_frame_ms, "state": snap.state }),
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    cmd_ms.sort_unstable();
    first_ms.sort_unstable();
    say(json!({
        "step": "zap_summary", "zaps": count, "failed": failed,
        "commandMs": { "p50": percentile(&cmd_ms, 0.5), "p95": percentile(&cmd_ms, 0.95) },
        "firstFrameMs": { "p50": percentile(&first_ms, 0.5), "p95": percentile(&first_ms, 0.95) },
    }));
    Ok(())
}

/// `DOWNLINK_E2E=multiview`: main player plus three tiles (PLAN §4.11) laid out 2x2 in the window.
/// Waits for every tile to play, moves audio, removes one tile and leaves multiview; reports tile
/// states at each step. Channels from `DOWNLINK_E2E_MV` (main first, comma separated).
async fn multiview(st: &AppState) -> Result<(), String> {
    use downlink_player::SurfaceBounds;
    let names = std::env::var("DOWNLINK_E2E_MV")
        .unwrap_or_else(|_| "Channel One,Test News,XC News,XC Sports".into());
    let pb = st.playback().await.map_err(|e| e.message)?;
    let mv = pb.multiview.get().cloned().ok_or("multiview unavailable")?;
    let mut ids = Vec::new();
    for n in names.split(',') {
        ids.push(channel_id(st, n.trim()).await?);
    }
    let Some((&main, tiles)) = ids.split_first() else {
        return Err("no channels".into());
    };
    pb.play_channel(main).await.map_err(|e| e.message)?;
    let snap = pb.snapshot_wait(
        |s| matches!(s, SessionState::Playing | SessionState::Failed),
        Duration::from_secs(20),
    );
    say(
        json!({ "step": "mv_main", "state": snap.state, "maxTiles": mv.state(Some(main)).await.max_tiles }),
    );
    let (w, h) = (480u32, 300u32);
    if let Some(s) = st.surface().await {
        use downlink_player::VideoSurface;
        s.set_bounds(SurfaceBounds {
            x: 0,
            y: 0,
            w,
            h,
            scale: 1.0,
        });
        s.set_visible(true);
    }
    for (i, id) in tiles.iter().enumerate() {
        let slot = i as u32 + 1;
        let t0 = Instant::now();
        let r = mv.set_tile(slot, *id, Some(main)).await;
        mv.set_bounds(
            slot,
            SurfaceBounds {
                x: (slot % 2 * w) as i32,
                y: (slot / 2 * h) as i32,
                w,
                h,
                scale: 1.0,
            },
            true,
        )
        .await;
        say(
            json!({ "step": "mv_add", "slot": slot, "ok": r.is_ok(), "error": r.err().map(|e| e.message), "ms": t0.elapsed().as_millis() as u64 }),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let d = mv.state(Some(main)).await;
        let done = d
            .tiles
            .iter()
            .filter(|t| t.slot > 0)
            .all(|t| t.state == "Playing" || t.state == "Failed");
        if done || Instant::now() > deadline {
            say(json!({ "step": "mv_tiles", "state": d }));
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if std::env::var_os("DOWNLINK_E2E_MV_STOPMAIN").is_some() {
        let _ = pb.stop().await;
        say(json!({ "step": "mv_main_stopped" }));
    }
    for _ in 0..3 {
        #[cfg(target_os = "linux")]
        let main_frames = st.surface().await.map(|s| s.render_count());
        #[cfg(not(target_os = "linux"))]
        let main_frames: Option<u64> = None;
        say(
            json!({ "step": "mv_positions", "main": pb.snapshot().position_secs, "mainFrames": main_frames, "tiles": mv.positions().await }),
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    mv.set_audio(2, Some(main)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    say(
        json!({ "step": "mv_audio", "audible": mv.state(Some(main)).await.audible, "mainMuted": pb.snapshot().muted }),
    );
    mv.remove_tile(3, Some(main)).await;
    say(json!({ "step": "mv_remove", "state": mv.state(Some(main)).await }));
    mv.clear().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let d = mv.state(Some(main)).await;
    say(
        json!({ "step": "mv_exit", "tiles": d.tiles.len(), "mainMuted": pb.snapshot().muted, "mainState": pb.snapshot().state }),
    );
    Ok(())
}

/// `DOWNLINK_E2E=failover`: play a channel whose stream cannot start (stop the stream server first)
/// and watch the §4.5 sequence: reconnect, server next-stream attempts, then a plain failure.
async fn failover(st: &AppState) -> Result<(), String> {
    let id = channel_id(st, "Channel One").await?;
    let pb = st.playback().await.map_err(|e| e.message)?;
    let t0 = Instant::now();
    pb.play_channel(id).await.map_err(|e| e.message)?;
    // Wait until the machine settles in Failed with no pending failover (state flips through
    // Resolving/Connecting between attempts), or give up after 90 s.
    let mut last = None;
    while t0.elapsed() < Duration::from_secs(90) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let dto = pb.snapshot_dto();
        if last != Some(dto.state) {
            say(
                json!({ "step": "state", "t": t0.elapsed().as_secs(), "state": dto.state, "error": dto.error.as_ref().map(|e| e.message.clone()) }),
            );
            last = Some(dto.state);
        }
        if dto.state == "Failed" && dto.error.is_some() && t0.elapsed() > Duration::from_secs(20) {
            tokio::time::sleep(Duration::from_secs(6)).await;
            if pb.snapshot_dto().state == "Failed" {
                break;
            }
        }
    }
    say(
        json!({ "step": "failover_done", "state": pb.snapshot_dto().state, "sourceStats": st.service_now().source_position(id).await.ok() }),
    );
    Ok(())
}

/// `DOWNLINK_E2E=smoke`: the M4 read commands against the real source (the test server has no VOD
/// or recordings, so empty lists and no errors are expected).
async fn smoke(st: &AppState) -> Result<(), String> {
    use downlink_core::service::{VodKindArg, VodListArgs};
    let svc = &st.service_now();
    let cats_m = svc
        .list_vod_categories(VodKindArg::Movie)
        .await
        .map_err(|e| e.message)?;
    let cats_s = svc
        .list_vod_categories(VodKindArg::Series)
        .await
        .map_err(|e| e.message)?;
    let movies = svc
        .list_movies(VodListArgs::default())
        .await
        .map_err(|e| e.message)?;
    let series = svc
        .list_series(VodListArgs::default())
        .await
        .map_err(|e| e.message)?;
    let recs = svc.list_recordings().await.map_err(|e| e.message)?;
    let rules = svc.list_recording_rules().await.map_err(|e| e.message)?;
    say(json!({
        "step": "smoke",
        "movieCategories": cats_m.len(), "seriesCategories": cats_s.len(),
        "movies": movies.len(), "series": series.len(),
        "recordings": serde_json::to_value(&recs).unwrap_or_default(),
        "rules": rules.len(),
    }));
    // Play the first watchable recording through the real player path, if there is one.
    let first = recs
        .in_progress
        .first()
        .or(recs.completed.first())
        .map(|r| r.id);
    if let (Some(id), Ok(pb)) = (first, st.playback().await) {
        pb.play_recording(id).await.map_err(|e| e.message)?;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(25),
        );
        say(
            json!({ "step": "play_recording", "id": id, "state": snap.state, "firstFrameMs": snap.first_frame_ms, "mode": snap.mode, "durationSecs": snap.duration_secs }),
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        let dto = pb.snapshot_dto();
        say(
            json!({ "step": "recording_state", "error": dto.error.as_ref().map(|e| e.message.clone()), "mode": dto.mode, "title": dto.title, "seekable": dto.seekable, "position": dto.position }),
        );
    }
    Ok(())
}

// ---- perf harness (PLAN §8.8) -------------------------------------------------------------------

pub static UI_METRICS: std::sync::Mutex<Option<serde_json::Value>> = std::sync::Mutex::new(None);

/// Called by the injected script in the webview with first-render timings.
#[tauri::command]
pub fn perf_report(metrics: serde_json::Value) {
    if let Ok(mut m) = UI_METRICS.lock() {
        *m = Some(metrics);
    }
}

/// Init script (runs before the page): records when `#root` first gets children and reports.
pub fn perf_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let js = r#"(function () {
  const rec = { timeOrigin: performance.timeOrigin };
  const check = () => {
    const root = document.getElementById('root');
    if (root && root.children.length && rec.rootAt === undefined) rec.rootAt = performance.now();
    if (root && root.innerText && root.innerText.length > 40 && rec.contentAt === undefined) rec.contentAt = performance.now();
  };
  new MutationObserver(check).observe(document, { childList: true, subtree: true, characterData: true });
  setTimeout(() => {
    check();
    const fcp = performance.getEntriesByName('first-contentful-paint')[0];
    rec.fcp = fcp ? fcp.startTime : null;
    rec.visibility = document.visibilityState;
    rec.domContentLoaded = performance.timing.domContentLoadedEventEnd - performance.timing.navigationStart;
    window.__TAURI__.core.invoke('perf_report', { metrics: rec });
  }, 3500);
})();"#;
    tauri::plugin::Builder::new("dl-perf")
        .js_init_script(js.to_owned())
        .build()
}

fn rss_kb(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

/// RSS of this process and of the whole process tree (WebKit helper processes), in MiB.
fn memory_mib() -> (u64, u64) {
    let me = std::process::id();
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    if let Ok(rd) = std::fs::read_dir("/proc") {
        for e in rd.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue;
            };
            let ppid = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|t| {
                    t.rsplit_once(')')
                        .and_then(|(_, r)| r.split_whitespace().nth(1)?.parse::<u32>().ok())
                });
            if let Some(pp) = ppid {
                children.entry(pp).or_default().push(pid);
            }
        }
    }
    let mut total = 0;
    let mut stack = vec![me];
    while let Some(p) = stack.pop() {
        total += rss_kb(p);
        if let Some(c) = children.get(&p) {
            stack.extend(c);
        }
    }
    (rss_kb(me) / 1024, total / 1024)
}

fn stats(mut v: Vec<f64>) -> serde_json::Value {
    v.sort_by(f64::total_cmp);
    let at = |p: f64| {
        v.get(((v.len() as f64 - 1.0) * p).round() as usize)
            .copied()
            .map(|x| (x * 100.0).round() / 100.0)
    };
    json!({ "n": v.len(), "p50": at(0.5), "p95": at(0.95), "max": v.last().map(|x| (x * 100.0).round() / 100.0) })
}

async fn timed<T, E>(f: impl std::future::Future<Output = Result<T, E>>) -> (f64, Option<T>) {
    let t = Instant::now();
    let r = f.await.ok();
    (t.elapsed().as_secs_f64() * 1000.0, r)
}

/// `DOWNLINK_E2E=perf-setup`: import the generated 20k-channel playlist and 7-day guide as an M3U
/// source (file paths from DOWNLINK_PERF_M3U / DOWNLINK_PERF_XMLTV).
async fn perf_setup(st: &AppState) -> Result<(), String> {
    let m3u = std::env::var("DOWNLINK_PERF_M3U").map_err(|_| "DOWNLINK_PERF_M3U missing")?;
    let xml = std::env::var("DOWNLINK_PERF_XMLTV").map_err(|_| "DOWNLINK_PERF_XMLTV missing")?;
    let t0 = Instant::now();
    let src = st
        .service_now()
        .add_source(AddSourceInput {
            kind: "m3u".into(),
            name: "perf".into(),
            url: m3u,
            api_key: None,
            username: None,
            password: None,
            xmltv_url: Some(xml),
            allow_self_signed: false,
        })
        .await
        .map_err(|e| e.message)?;
    let (rss, tree) = memory_mib();
    say(
        json!({ "step": "perf_setup", "sourceId": src.id, "importSecs": t0.elapsed().as_secs_f64(), "rssMiB": rss, "treeMiB": tree }),
    );
    Ok(())
}

/// `DOWNLINK_E2E=perf`: budgets on a warm cache (run `perf-setup` first, in a fresh process).
async fn perf(st: &AppState) -> Result<(), String> {
    use downlink_core::service::GuideArgs;
    let svc = &st.service_now();
    let setup_done = crate::resume::SETUP_DONE.get().map(Duration::as_secs_f64);
    let setup_returned = crate::resume::SETUP_RETURNED
        .get()
        .map(Duration::as_secs_f64);
    let core_ready = crate::resume::CORE_READY.get().map(Duration::as_secs_f64);
    let t_ready = Instant::now();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let hour = now / 3600 * 3600;

    // Start -> interactive (Rust side): the commands the first screen needs, in order.
    let mut first = serde_json::Map::new();
    let (ms, _) = timed(svc.list_sources()).await;
    first.insert("list_sources".into(), json!(ms));
    let (ms, counts) = timed(svc.get_counts()).await;
    first.insert("get_counts".into(), json!(ms));
    let (ms, _) = timed(svc.list_groups(None)).await;
    first.insert("list_groups".into(), json!(ms));
    let (ms, chans) = timed(svc.list_channels(ListChannelsArgs {
        limit: Some(100),
        ..Default::default()
    }))
    .await;
    first.insert("list_channels_first_100".into(), json!(ms));
    let (ms, _) = timed(svc.get_home()).await;
    first.insert("get_home".into(), json!(ms));
    let guide = |off: i64, h: i64| GuideArgs {
        start: h - 2 * 3600,
        end: h + 6 * 3600,
        channel_offset: off,
        channel_limit: Some(30),
        group_id: None,
    };
    let (ms_guide_cold, g) = timed(svc.get_guide(guide(0, hour))).await;
    first.insert("get_guide_first_chunk_cold".into(), json!(ms_guide_cold));
    let interactive_ms = t_ready.elapsed().as_secs_f64() * 1000.0;
    say(json!({
        "step": "interactive",
        "setupReturnedSecs": setup_returned,
        "coreReadySecs": core_ready,
        "engineReadySecs": setup_done,
        "firstScreenCommandsMs": first,
        "firstScreenTotalMs": interactive_ms,
        "channels": counts.map(|c| c.all),
        "firstChunkChannels": g.as_ref().map(|g| g.channels.len()),
        "firstChunkProgrammes": g.as_ref().map(|g| g.programmes.len()),
        "list100": chans.map(|c| c.len()),
    }));

    // Guide open (warm) and scrolling: first chunk, then chunks across all channels and days.
    let mut warm = Vec::new();
    for _ in 0..30 {
        warm.push(timed(svc.get_guide(guide(0, hour))).await.0);
    }
    let mut scroll = Vec::new();
    for (i, off) in (0..20_000).step_by(30).enumerate() {
        let h = hour + ((i as i64 % 6) - 1) * 24 * 3600; // jump across days too
        scroll.push(timed(svc.get_guide(guide(off, h))).await.0);
    }
    say(
        json!({ "step": "guide", "firstChunkWarmMs": stats(warm), "scrollChunksMs": stats(scroll) }),
    );

    // Section switch: the commands each screen issues.
    let mut switch = Vec::new();
    for i in 0..60 {
        let t = Instant::now();
        match i % 4 {
            0 => drop(svc.list_groups(None).await),
            1 => drop(svc.get_home().await),
            2 => drop(
                svc.list_channels(ListChannelsArgs {
                    limit: Some(60),
                    offset: Some((i * 60) as i64),
                    ..Default::default()
                })
                .await,
            ),
            _ => drop(svc.get_counts().await),
        }
        switch.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    say(json!({ "step": "section_switch", "ms": stats(switch) }));

    // Search keystrokes: every prefix of two queries, channel search and the unified search.
    let (mut chan_ms, mut uni_ms) = (Vec::new(), Vec::new());
    for q in ["Generated Channel 12345", "Weekend Showcase"] {
        for n in 1..=q.len() {
            let p = &q[..n];
            chan_ms.push(
                timed(svc.list_channels(ListChannelsArgs {
                    query: Some(p.to_owned()),
                    limit: Some(50),
                    ..Default::default()
                }))
                .await
                .0,
            );
            uni_ms.push(timed(svc.search(p, 30)).await.0);
        }
    }
    say(
        json!({ "step": "search_keystroke", "channelListMs": stats(chan_ms), "unifiedSearchMs": stats(uni_ms) }),
    );

    // Memory after all that browsing.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (rss, tree) = memory_mib();
    say(
        json!({ "step": "memory", "rssMiB": rss, "processTreeMiB": tree, "note": "no video playing" }),
    );

    // UI first-render metrics reported by the injected script (needs the embedded UI).
    tokio::time::sleep(Duration::from_secs(3)).await;
    let ui = UI_METRICS.lock().ok().and_then(|m| m.clone());
    let wall = crate::START.get().map(|(_, w)| *w);
    let origin_offset = ui
        .as_ref()
        .and_then(|u| u.get("timeOrigin")?.as_f64())
        .and_then(|o| {
            let start_ms = wall?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs_f64()
                * 1000.0;
            Some((o - start_ms) / 1000.0)
        });
    say(
        json!({ "step": "ui_metrics", "navigationStartAfterProcessStartSecs": origin_offset, "metrics": ui }),
    );
    Ok(())
}

/// `DOWNLINK_E2E=launch`: with `resumeLastChannel` on, report process start -> first frame.
async fn launch(st: &AppState) -> Result<(), String> {
    let pb = st.playback().await.map_err(|e| e.message)?;
    let snap = pb.snapshot_wait(
        |s| matches!(s, SessionState::Playing | SessionState::Failed),
        Duration::from_secs(30),
    );
    let issued = crate::resume::RESUME_ISSUED
        .get()
        .map(Duration::as_secs_f64);
    say(json!({
        "step": "launch",
        "state": snap.state,
        "setupDoneSecs": crate::resume::SETUP_DONE.get().map(Duration::as_secs_f64),
        "resumeIssuedSecs": issued,
        "loadfileToFirstFrameMs": snap.first_frame_ms,
        "startToFirstFrameSecs": issued.zip(snap.first_frame_ms).map(|(i, f)| i + f as f64 / 1000.0),
    }));
    Ok(())
}

/// `DOWNLINK_E2E=degraded`: the data dir holds a library from a newer version. Catalog commands must
/// return the blocking error; archiving the file must let a fresh service open.
async fn degraded(st: &AppState) -> Result<(), String> {
    let r = st.svc().await;
    let e = match r {
        Ok(_) => return Err("expected the service to fail".into()),
        Err(e) => e,
    };
    say(
        json!({ "step": "degraded_error", "message": e.message, "blocking": e.blocking, "code": e.code }),
    );
    let playback = st.playback().await.err().map(|e| e.message);
    say(json!({ "step": "playback_error", "message": playback }));
    let archived = Service::archive_newer_database(&st.data_dir).map_err(|e| e.message)?;
    say(json!({ "step": "archived", "file": archived.file_name().and_then(|n| n.to_str()) }));
    let fresh = Service::new(downlink_core::service::ServiceConfig {
        data_dir: st.data_dir.clone(),
        secrets: std::sync::Arc::new(downlink_core::auth::MemorySecretStore::new()),
        app_version: "test".into(),
    })
    .await
    .map_err(|e| e.message)?;
    let n = fresh.list_sources().await.map_err(|e| e.message)?.len();
    say(json!({ "step": "reopened", "sources": n }));
    Ok(())
}

/// `DOWNLINK_E2E=hls`: switch the source to the opt-in HLS transport, play a channel, check the
/// session is tracked, then stop and check it is released.
async fn hls(st: &AppState, db: &str) -> Result<(), String> {
    use downlink_core::service::SetSourceOptionsArgs;
    let svc = st.service_now();
    let src = svc.list_sources().await.map_err(|e| e.message)?;
    let s = src.first().ok_or("no source")?;
    say(
        json!({ "step": "source", "supportsHlsTransport": s.supports_hls_transport, "hlsTransport": s.hls_transport }),
    );
    let dto = svc
        .set_source_options(SetSourceOptionsArgs {
            source_id: s.id,
            hls_transport: true,
        })
        .await
        .map_err(|e| e.message)?;
    say(json!({ "step": "enabled", "hlsTransport": dto.hls_transport }));
    let id = channel_id(st, "Channel One").await?;
    let pb = st.playback().await.map_err(|e| e.message)?;
    pb.play_channel(id).await.map_err(|e| e.message)?;
    let snap = pb.snapshot_wait(
        |s| matches!(s, SessionState::Playing | SessionState::Failed),
        Duration::from_secs(30),
    );
    say(
        json!({ "step": "hls_play", "state": snap.state, "firstFrameMs": snap.first_frame_ms, "openSessions": open_sessions(db) }),
    );
    pb.stop().await.map_err(|e| e.message)?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    say(json!({ "step": "hls_stopped", "openSessions": open_sessions(db) }));
    svc.set_source_options(SetSourceOptionsArgs {
        source_id: s.id,
        hls_transport: false,
    })
    .await
    .map_err(|e| e.message)?;
    Ok(())
}

/// `DOWNLINK_E2E=vod`: against a server with a VOD library (user/password in
/// DOWNLINK_E2E_USER / DOWNLINK_E2E_PASS, never printed): category counts, paging past the server's
/// page size, then play two movies and a live channel through the real player path.
async fn vod(st: &AppState) -> Result<(), String> {
    use downlink_core::service::{VodKindArg, VodListArgs};
    let svc = st.service_now();
    if svc.list_sources().await.map_err(|e| e.message)?.is_empty() {
        let url = std::env::var("DOWNLINK_E2E_URL").map_err(|_| "DOWNLINK_E2E_URL missing")?;
        svc.add_source(AddSourceInput {
            kind: "dispatcharr".into(),
            name: "E2E".into(),
            url,
            api_key: None,
            username: std::env::var("DOWNLINK_E2E_USER").ok(),
            password: std::env::var("DOWNLINK_E2E_PASS").ok(),
            xmltv_url: None,
            allow_self_signed: false,
        })
        .await
        .map_err(|e| e.message)?;
    }
    let cats = svc
        .list_vod_categories(VodKindArg::Movie)
        .await
        .map_err(|e| e.message)?;
    say(
        json!({ "step": "categories", "count": cats.len(), "empty": cats.iter().filter(|c| c.count == 0).count() }),
    );
    let big = cats.iter().max_by_key(|c| c.count);
    let t0 = Instant::now();
    let page = svc
        .list_movies(VodListArgs {
            category_id: big.map(|c| c.id),
            offset: Some(0),
            limit: Some(300),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    let page2 = svc
        .list_movies(VodListArgs {
            category_id: big.map(|c| c.id),
            offset: Some(150),
            limit: Some(10),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    say(json!({
        "step": "list", "category": big.map(|c| c.count), "got": page.len(), "ms": t0.elapsed().as_millis(),
        "offsetMatches": page.get(150).map(|m| m.id) == page2.first().map(|m| m.id),
    }));
    let pb = st.playback().await.map_err(|e| e.message)?;
    for m in page.iter().take(2) {
        let t0 = Instant::now();
        let r = pb.play_vod("movie", m.id, false).await;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(30),
        );
        let dto = pb.snapshot_dto();
        say(json!({
            "step": "play_vod", "id": m.id, "ok": r.is_ok(), "state": snap.state, "wallMs": t0.elapsed().as_millis(),
            "durationSecs": snap.duration_secs, "error": dto.error.as_ref().map(|e| e.message.clone()),
        }));
        tokio::time::sleep(Duration::from_secs(4)).await;
        say(
            json!({ "step": "vod_after", "state": pb.snapshot().state, "position": pb.snapshot().position_secs }),
        );
        pb.command(PlayerCommand::SeekAbsolute { secs: 900.0 })
            .map_err(|e| e.message)?;
        tokio::time::sleep(Duration::from_secs(6)).await;
        let s = pb.snapshot_dto();
        say(
            json!({ "step": "vod_seek", "state": s.state, "position": s.position, "error": s.error.map(|e| e.message) }),
        );
        let t1 = Instant::now();
        let r = pb.stop().await;
        say(
            json!({ "step": "vod_stop", "ok": r.is_ok(), "ms": t1.elapsed().as_millis(), "state": pb.snapshot().state }),
        );
    }
    // Resume path (start= option) on the first movie.
    if let Some(m) = page.first() {
        let r = pb.play_vod("movie", m.id, true).await;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(30),
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        say(
            json!({ "step": "vod_resume", "ok": r.is_ok(), "state": snap.state, "position": pb.snapshot().position_secs }),
        );
    }
    // Series: detail (provider info + episode list), then play the first episode.
    let series = svc
        .list_series(VodListArgs {
            limit: Some(5),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    if let Some(sr) = series.first() {
        let d = svc.get_series(sr.id).await.map_err(|e| e.message)?;
        let ep = d
            .seasons
            .iter()
            .flat_map(|s| s.episodes.iter())
            .next()
            .map(|e| e.id);
        say(json!({ "step": "series", "seasons": d.seasons.len(), "firstEpisode": ep }));
        if let Some(eid) = ep {
            let r = pb.play_vod("episode", eid, false).await;
            let snap = pb.snapshot_wait(
                |s| matches!(s, SessionState::Playing | SessionState::Failed),
                Duration::from_secs(30),
            );
            say(
                json!({ "step": "play_episode", "ok": r.as_ref().err().map(|e| e.message.clone()), "state": snap.state, "durationSecs": snap.duration_secs, "error": pb.snapshot_dto().error.map(|e| e.message) }),
            );
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
    let chans = svc
        .list_channels(ListChannelsArgs {
            limit: Some(5),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    if let Some(c) = chans.first() {
        let r = pb.play_channel(c.id).await;
        let snap = pb.snapshot_wait(
            |s| matches!(s, SessionState::Playing | SessionState::Failed),
            Duration::from_secs(30),
        );
        say(
            json!({ "step": "live_after_vod", "ok": r.is_ok(), "state": snap.state, "error": pb.snapshot_dto().error.map(|e| e.message) }),
        );
    }
    Ok(())
}
