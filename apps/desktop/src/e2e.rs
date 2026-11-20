//! Developer end-to-end driver (feature `e2e`): runs the real command path (Service + Playback +
//! libmpv + surface) against a local test server without a UI, printing `E2E {json}` lines.
//! Configured by env: DOWNLINK_E2E=full|offline, DOWNLINK_E2E_URL, DOWNLINK_E2E_KEY (never printed).
//! Always quits by itself; keep the window small (`DOWNLINK_WINDOW_SIZE=960x600`).

use std::process::Command;
use std::time::{Duration, Instant};

use downlink_core::service::{AddSourceInput, ListChannelsArgs};
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
            _ => full(&st, &db).await,
        };
        if let Err(e) = r {
            say(json!({ "step": "error", "message": e }));
        }
        if let Some(pb) = &st.playback {
            let _ = pb.stop().await;
        }
        app.exit(0);
    });
}

async fn channel_id(st: &AppState, name: &str) -> Result<i64, String> {
    let list = st
        .service
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
    let sources = st.service.list_sources().await.map_err(|e| e.message)?;
    if sources.is_empty() {
        let t0 = Instant::now();
        let s = st
            .service
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
        .service
        .list_channels(ListChannelsArgs {
            limit: Some(1000),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    let groups = st.service.list_groups(None).await.map_err(|e| e.message)?;
    say(json!({ "step": "catalog", "channels": channels.len(), "groups": groups.len() }));

    let pb = st.playback().map_err(|e| e.message)?;
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
    let rows = downlink_core::epg::window(st.service.store(), &[xc], w)
        .await
        .map_err(|e| e.message)?;
    let start = match rows.iter().rev().find(|r| r.end_utc <= now) {
        Some(r) => r.start_utc,
        None => {
            // The test server's EPG has no XC programmes in the cache; insert one archived hour so
            // the catch-up session flow can still be exercised (times come from the row).
            let start = (now - 2 * 3600) / 3600 * 3600;
            st.service
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
    pb.play_catchup(&pid).await.map_err(|e| e.message)?;
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
    pb.play_catchup(&pid).await.map_err(|e| e.message)?;
    pb.snapshot_wait(|s| s == SessionState::Playing, Duration::from_secs(25));
    say(json!({ "step": "before_quit", "openSessions": open_sessions(db) }));
    Ok(())
}

async fn offline(st: &AppState, _db: &str) -> Result<(), String> {
    let sources = st.service.list_sources().await.map_err(|e| e.message)?;
    say(
        json!({ "step": "sources_cached", "sources": sources.iter().map(|s| json!({ "name": s.name, "status": s.status })).collect::<Vec<_>>() }),
    );
    let channels = st
        .service
        .list_channels(ListChannelsArgs {
            limit: Some(1000),
            ..Default::default()
        })
        .await
        .map_err(|e| e.message)?;
    say(json!({ "step": "channels_from_cache", "count": channels.len() }));
    if let Some(s) = sources.first() {
        let r = st.service.sync_source(s.id).await;
        say(
            json!({ "step": "sync_while_offline", "error": r.err().map(|e| json!({ "code": e.code, "message": e.message })) }),
        );
        let after = st.service.list_sources().await.map_err(|e| e.message)?;
        say(json!({ "step": "status_after_sync", "status": after.first().map(|s| s.status) }));
    }
    let pb = st.playback().map_err(|e| e.message)?;
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
    let pb = st.playback().map_err(|e| e.message)?;
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
