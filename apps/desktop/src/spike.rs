//! M0 spike support: the spike page's commands, a zap benchmark and an env-driven autorun.
//! Compiled only with the `spike` feature. Never enters fullscreen (a fullscreen window locked a
//! remote session once); runs are expected to set `DOWNLINK_SPIKE_QUIT=1` or a `timeout`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use downlink_player::{PlayRequest, Player, PlayerCommand, PlayerSnapshot, Profile, SessionState};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;
use crate::surface::Surface;

#[derive(Debug, Serialize)]
pub struct CmdError {
    code: &'static str,
    message: String,
}

impl From<downlink_player::PlayerError> for CmdError {
    fn from(e: downlink_player::PlayerError) -> Self {
        Self {
            code: "player",
            message: e.to_string(),
        }
    }
}

fn player(state: &State<'_, AppState>) -> Result<Arc<Player>, CmdError> {
    state.player.clone().ok_or_else(|| CmdError {
        code: "engine",
        message: state
            .engine_error
            .clone()
            .unwrap_or_else(|| "the playback engine is not available".into()),
    })
}

#[tauri::command]
pub async fn play_url(
    state: State<'_, AppState>,
    url: String,
    profile: Option<Profile>,
    start_secs: Option<f64>,
) -> Result<u64, CmdError> {
    let p = player(&state)?;
    Ok(p.play(PlayRequest {
        url,
        headers: Vec::new(),
        profile: profile.unwrap_or(Profile::Live),
        start_secs,
        failover_capable: false,
        mode: downlink_player::PlayMode::Live,
        source_index: 0,
    })?)
}

#[tauri::command]
pub async fn player_snapshot(state: State<'_, AppState>) -> Result<PlayerSnapshot, CmdError> {
    Ok(player(&state)?.snapshot())
}

#[tauri::command]
pub async fn player_diagnostics(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String)>, CmdError> {
    Ok(player(&state)?.diagnostics())
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ZapResult {
    index: u32,
    state: Option<SessionState>,
    /// loadfile -> first frame as seen by the player.
    first_frame_ms: Option<u64>,
    /// Wall time of the `play` call until the state settled.
    wall_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ZapReport {
    results: Vec<ZapResult>,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    render_fps: Option<f64>,
    hwdec_current: Option<String>,
    diagnostics: Vec<(String, String)>,
    seek: Option<SeekReport>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SeekReport {
    before_secs: Option<f64>,
    after_secs: Option<f64>,
    state_after: Option<SessionState>,
}

/// Play `url` with the seekable profile, seek +`secs`, report positions.
fn run_seek_test(
    p: &Arc<Player>,
    url: &str,
    headers: &[(String, String)],
    secs: f64,
) -> SeekReport {
    let _ = p.play(PlayRequest {
        url: url.to_owned(),
        headers: headers.to_vec(),
        profile: Profile::Seekable,
        start_secs: None,
        failover_capable: false,
        mode: downlink_player::PlayMode::Live,
        source_index: 0,
    });
    p.wait_for(|s| s == SessionState::Playing, Duration::from_secs(12));
    std::thread::sleep(Duration::from_secs(3));
    let before = p.snapshot().position_secs;
    let _ = p.command(PlayerCommand::SeekRelative { secs });
    std::thread::sleep(Duration::from_secs(3));
    let after = p.snapshot();
    SeekReport {
        before_secs: before,
        after_secs: after.position_secs,
        state_after: after.state,
    }
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted.get(idx).copied()
}

/// Zap between `urls` round-robin `count` times, dwelling `dwell_ms` on each, and measure.
fn run_zap_bench(
    p: &Arc<Player>,
    surface: Option<&Arc<Surface>>,
    urls: &[String],
    count: u32,
    dwell_ms: u64,
    headers: &[(String, String)],
) -> ZapReport {
    let mut results = Vec::new();
    for i in 0..count {
        let url = urls[(i as usize) % urls.len()].clone();
        let t0 = Instant::now();
        let r = p.play(PlayRequest {
            url,
            headers: headers.to_vec(),
            profile: Profile::Live,
            start_secs: None,
            failover_capable: false,
            mode: downlink_player::PlayMode::Live,
            source_index: 0,
        });
        if let Err(e) = r {
            tracing::error!("zap {i}: {e}");
            continue;
        }
        let snap = p.wait_for(
            |s| {
                matches!(
                    s,
                    SessionState::Playing | SessionState::Failed | SessionState::Paused
                )
            },
            Duration::from_secs(12),
        );
        results.push(ZapResult {
            index: i,
            state: snap.state,
            first_frame_ms: snap.first_frame_ms,
            wall_ms: t0.elapsed().as_millis() as u64,
        });
        std::thread::sleep(Duration::from_millis(dwell_ms));
    }
    // FPS: count GLArea renders over 4 s of steady playback.
    let render_fps = surface.map(|s| {
        let (c0, t0) = (s.render_count(), Instant::now());
        std::thread::sleep(Duration::from_secs(4));
        (s.render_count() - c0) as f64 / t0.elapsed().as_secs_f64()
    });
    let mut ff: Vec<u64> = results.iter().filter_map(|r| r.first_frame_ms).collect();
    ff.sort_unstable();
    let snap = p.snapshot();
    ZapReport {
        results,
        p50_ms: percentile(&ff, 0.50),
        p95_ms: percentile(&ff, 0.95),
        render_fps,
        hwdec_current: snap.hwdec_current,
        diagnostics: p.diagnostics(),
        seek: None,
    }
}

#[tauri::command]
pub async fn zap_bench(
    state: State<'_, AppState>,
    urls: Vec<String>,
    count: u32,
    dwell_ms: Option<u64>,
) -> Result<ZapReport, CmdError> {
    let p = player(&state)?;
    if urls.is_empty() {
        return Err(CmdError {
            code: "args",
            message: "no urls".into(),
        });
    }
    let surface = state.surface.clone();
    let dwell = dwell_ms.unwrap_or(1500);
    tauri::async_runtime::spawn_blocking(move || {
        run_zap_bench(&p, surface.as_ref(), &urls, count, dwell, &[])
    })
    .await
    .map_err(|e| CmdError {
        code: "internal",
        message: e.to_string(),
    })
}

pub fn spawn_autorun(app: AppHandle, p: Arc<Player>, surface: Option<Arc<Surface>>, urls: String) {
    let urls: Vec<String> = urls
        .split(',')
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
        .collect();
    let count = std::env::var("DOWNLINK_SPIKE_COUNT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let out = std::env::var("DOWNLINK_SPIKE_OUT").ok();
    let quit = std::env::var("DOWNLINK_SPIKE_QUIT").is_ok();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        if urls.is_empty() {
            return;
        }
        let headers: Vec<(String, String)> = std::env::var("DOWNLINK_SPIKE_HEADER")
            .ok()
            .and_then(|h| {
                h.split_once(": ")
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .into_iter()
            .collect();
        let seek_secs: Option<f64> = std::env::var("DOWNLINK_SPIKE_SEEK")
            .ok()
            .and_then(|v| v.parse().ok());
        let mut report = if count == 0 {
            run_zap_bench(&p, surface.as_ref(), &urls, 0, 0, &headers)
        } else {
            run_zap_bench(&p, surface.as_ref(), &urls, count, 1500, &headers)
        };
        if let Some(secs) = seek_secs {
            report.seek = Some(run_seek_test(&p, &urls[0], &headers, secs));
        }
        match serde_json::to_string_pretty(&report) {
            Ok(j) => match out {
                Some(path) => {
                    let _ = std::fs::write(path, j);
                }
                None => println!("{j}"),
            },
            Err(e) => tracing::error!("report: {e}"),
        }
        if std::env::var_os("DOWNLINK_SPIKE_FULLSCREEN").is_some()
            && let Some(win) = app.get_webview_window("main")
        {
            let _ = p.play(PlayRequest {
                url: urls[0].clone(),
                headers: Vec::new(),
                profile: Profile::Live,
                start_secs: None,
                failover_capable: false,
                mode: downlink_player::PlayMode::Live,
                source_index: 0,
            });
            p.wait_for(|s| s == SessionState::Playing, Duration::from_secs(12));
            for fs in [true, false, true, false] {
                let _ = win.set_fullscreen(fs);
                std::thread::sleep(Duration::from_secs(2));
                let c0 = surface.as_ref().map_or(0, |s| s.render_count());
                std::thread::sleep(Duration::from_secs(2));
                let c1 = surface.as_ref().map_or(0, |s| s.render_count());
                let size = win
                    .inner_size()
                    .map(|s| format!("{}x{}", s.width, s.height))
                    .unwrap_or_default();
                println!(
                    "fullscreen={fs} is_fullscreen={:?} inner={size} render_fps={:.1} state={:?}",
                    win.is_fullscreen().ok(),
                    (c1 - c0) as f64 / 2.0,
                    p.snapshot().state
                );
            }
        }
        #[cfg(target_os = "linux")]
        if let Ok(path) = std::env::var("DOWNLINK_SPIKE_SHOT") {
            let _ = p.play(PlayRequest {
                url: urls[0].clone(),
                headers: Vec::new(),
                profile: Profile::Live,
                start_secs: None,
                failover_capable: false,
                mode: downlink_player::PlayMode::Live,
                source_index: 0,
            });
            p.wait_for(|s| s == SessionState::Playing, Duration::from_secs(12));
            std::thread::sleep(Duration::from_secs(2));
            let a = app.clone();
            let _ = app.run_on_main_thread(move || dump_window(&a, &path));
            std::thread::sleep(Duration::from_secs(2));
        }
        if quit {
            let _ = p.stop();
            app.exit(0);
        }
    });
}

/// Dev-only (spike): dump (a) the GLArea contents and (b) the webview snapshot as raw BGRA files
/// (`<path>.glarea.argb`, `<path>.webview.argb`, with `.txt` sizes) to check compositing inputs.
#[cfg(target_os = "linux")]
fn dump_window(app: &AppHandle, path: &str) {
    use gtk::prelude::*;
    fn find_glarea(w: &gtk::Widget) -> Option<gtk::Widget> {
        if w.type_().name() == "GtkGLArea" {
            return Some(w.clone());
        }
        let c = w.clone().downcast::<gtk::Container>().ok()?;
        c.children().iter().find_map(find_glarea)
    }
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if let Ok(gw) = win.gtk_window()
        && let Some(gl) = find_glarea(gw.upcast_ref())
    {
        let (w, h) = (gl.allocated_width(), gl.allocated_height());
        if let Ok(surface) = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h)
            && let Ok(cr) = gtk::cairo::Context::new(&surface)
        {
            gl.draw(&cr);
            drop(cr);
            surface.flush();
            if let Ok(d) = surface.take_data() {
                let _ = std::fs::write(format!("{path}.glarea.argb"), &*d);
                let _ = std::fs::write(format!("{path}.glarea.txt"), format!("{w}x{h}"));
            }
        }
    }
    let path = path.to_owned();
    let _ = win.with_webview(move |wv| {
        use webkit2gtk::WebViewExt;
        let view = wv.inner();
        view.snapshot(
            webkit2gtk::SnapshotRegion::Visible,
            webkit2gtk::SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |res| {
                if let Ok(surf) = res
                    && let Ok(mut img) = gtk::cairo::ImageSurface::try_from(surf)
                {
                    let (w, h) = (img.width(), img.height());
                    if let Ok(d) = img.data() {
                        let _ = std::fs::write(format!("{path}.webview.argb"), &*d);
                        let _ = std::fs::write(format!("{path}.webview.txt"), format!("{w}x{h}"));
                    }
                }
            },
        );
    });
}
