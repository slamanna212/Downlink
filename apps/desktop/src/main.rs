//! Downlink desktop shell. M0 spike commands: `play_url`, `stop`, `player_command`,
//! `set_video_bounds`, plus diagnostics and a zap benchmark. Emits `player://state`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use downlink_player::{
    MainThreadDispatch, PlayRequest, Player, PlayerCommand, PlayerEvent, PlayerSnapshot,
    Preferences, Profile, RenderBackend, SessionState, SurfaceBounds, VideoSurface,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

#[cfg(target_os = "linux")]
type Surface = downlink_player::surface::linux::LinuxSurface;
#[cfg(target_os = "windows")]
type Surface = downlink_player::surface::windows::WindowsSurface;
#[cfg(target_os = "macos")]
type Surface = downlink_player::surface::macos::MacSurface;

struct AppState {
    player: Option<Arc<Player>>,
    surface: Option<Arc<Surface>>,
    load_error: Option<String>,
}

/// Serialisable command error: a machine code and a message that never contains URLs.
#[derive(Debug, Serialize)]
struct CmdError {
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
            .load_error
            .clone()
            .unwrap_or_else(|| "the playback engine is not available".into()),
    })
}

#[tauri::command]
async fn play_url(
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
    })?)
}

#[tauri::command]
async fn stop(state: State<'_, AppState>) -> Result<(), CmdError> {
    Ok(player(&state)?.stop()?)
}

#[tauri::command]
async fn player_command(
    state: State<'_, AppState>,
    command: PlayerCommand,
) -> Result<(), CmdError> {
    Ok(player(&state)?.command(command)?)
}

/// Video rect in physical pixels (window client coordinates).
#[tauri::command]
async fn set_video_bounds(
    state: State<'_, AppState>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    visible: bool,
) -> Result<(), CmdError> {
    if let Some(s) = &state.surface {
        s.set_bounds(SurfaceBounds { x, y, w, h, scale });
        s.set_visible(visible);
    }
    Ok(())
}

#[tauri::command]
async fn player_snapshot(state: State<'_, AppState>) -> Result<PlayerSnapshot, CmdError> {
    Ok(player(&state)?.snapshot())
}

#[tauri::command]
async fn player_diagnostics(state: State<'_, AppState>) -> Result<Vec<(String, String)>, CmdError> {
    Ok(player(&state)?.diagnostics())
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ZapResult {
    index: u32,
    state: Option<SessionState>,
    /// loadfile -> first frame as seen by the player.
    first_frame_ms: Option<u64>,
    /// Wall time of the `play` call until the state settled.
    wall_ms: u64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ZapReport {
    results: Vec<ZapResult>,
    p50_ms: Option<u64>,
    p95_ms: Option<u64>,
    render_fps: Option<f64>,
    hwdec_current: Option<String>,
    diagnostics: Vec<(String, String)>,
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
) -> ZapReport {
    let mut results = Vec::new();
    for i in 0..count {
        let url = urls[(i as usize) % urls.len()].clone();
        let t0 = Instant::now();
        let r = p.play(PlayRequest {
            url,
            headers: Vec::new(),
            profile: Profile::Live,
            start_secs: None,
            failover_capable: false,
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
    }
}

#[tauri::command]
async fn zap_bench(
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
        run_zap_bench(&p, surface.as_ref(), &urls, count, dwell)
    })
    .await
    .map_err(|e| CmdError {
        code: "internal",
        message: e.to_string(),
    })
}

fn bundled_mpv_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok().map(|d| d.join("mpv"))
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let mut state = AppState {
        player: None,
        surface: None,
        load_error: None,
    };

    match downlink_player::load(bundled_mpv_dir(&handle).as_deref()) {
        Ok(loaded) => {
            tracing::info!(source = ?loaded.source, api = ?loaded.client_api_version, "libmpv loaded");
            let emit_handle = handle.clone();
            let sink: downlink_player::EventSink = Arc::new(move |e| match e {
                PlayerEvent::State(s) => {
                    let _ = emit_handle.emit("player://state", s);
                }
                PlayerEvent::FailoverNeeded {
                    session,
                    last_error,
                } => {
                    let _ = emit_handle.emit(
                        "player://failover",
                        serde_json::json!({ "session": session, "lastError": last_error }),
                    );
                }
                PlayerEvent::Log { level, text } => {
                    tracing::info!(target: "mpv", "[{level}] {text}")
                }
            });
            match Player::new(
                &loaded,
                Preferences::default(),
                RenderBackend::platform_default(),
                sink,
            ) {
                Ok(p) => {
                    let p = Arc::new(p);
                    let disp_handle = handle.clone();
                    let dispatch: MainThreadDispatch = Arc::new(move |f| {
                        let _ = disp_handle.run_on_main_thread(f);
                    });
                    match attach_surface(&handle, &p, dispatch) {
                        Ok(s) => state.surface = Some(Arc::new(s)),
                        Err(e) => {
                            tracing::error!("video surface unavailable: {e}");
                            state.load_error = Some(e);
                        }
                    }
                    state.player = Some(p);
                }
                Err(e) => state.load_error = Some(e.to_string()),
            }
        }
        Err(e) => {
            tracing::error!("{e}");
            state.load_error = Some(e.to_string());
        }
    }

    if let (Some(p), Ok(urls)) = (state.player.clone(), std::env::var("DOWNLINK_SPIKE_URLS")) {
        spawn_autorun(handle.clone(), p, state.surface.clone(), urls);
    }
    app.manage(state);
    Ok(())
}

#[cfg(target_os = "linux")]
fn attach_surface(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let vbox = win.default_vbox().map_err(|e| e.to_string())?;
    let mut s = Surface::new(p.engine(), dispatch);
    s.attach(&vbox).map_err(|e| e.to_string())?;
    Ok(s)
}

#[cfg(target_os = "windows")]
fn attach_surface(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let mut s = Surface::new(p.engine(), dispatch);
    s.attach(&(hwnd.0 as isize)).map_err(|e| e.to_string())?;
    Ok(s)
}

#[cfg(target_os = "macos")]
fn attach_surface(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let view = win.ns_view().map_err(|e| e.to_string())?;
    let mut s = Surface::new(p.engine(), dispatch);
    s.attach(&view).map_err(|e| e.to_string())?;
    Ok(s)
}

/// Spike automation: `DOWNLINK_SPIKE_URLS=a,b DOWNLINK_SPIKE_COUNT=20 DOWNLINK_SPIKE_OUT=file.json`.
fn spawn_autorun(app: AppHandle, p: Arc<Player>, surface: Option<Arc<Surface>>, urls: String) {
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
        let report = run_zap_bench(&p, surface.as_ref(), &urls, count, 1500);
        match serde_json::to_string_pretty(&report) {
            Ok(j) => match out {
                Some(path) => {
                    let _ = std::fs::write(path, j);
                }
                None => println!("{j}"),
            },
            Err(e) => tracing::error!("report: {e}"),
        }
        if quit {
            let _ = p.stop();
            app.exit(0);
        }
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let result = tauri::Builder::default()
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            play_url,
            stop,
            player_command,
            set_video_bounds,
            player_snapshot,
            player_diagnostics,
            zap_bench
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        tracing::error!("tauri failed: {e}");
        std::process::exit(1);
    }
}
