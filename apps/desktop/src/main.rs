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
        mode: downlink_player::PlayMode::Live,
        source_index: 0,
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
async fn set_preferences(
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), CmdError> {
    player(&state)?.set_preferences(preferences);
    Ok(())
}

#[tauri::command]
async fn audio_devices(
    state: State<'_, AppState>,
) -> Result<Vec<downlink_player::AudioDevice>, CmdError> {
    Ok(player(&state)?.audio_devices())
}

/// Returns the audio mode in effect (passthrough falls back to surround on unsupported devices).
#[tauri::command]
async fn set_audio_device(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<downlink_player::AudioMode, CmdError> {
    Ok(player(&state)?.set_audio_device(name))
}

#[tauri::command]
async fn set_stats_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), CmdError> {
    player(&state)?.set_stats_enabled(enabled);
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
    seek: Option<SeekReport>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SeekReport {
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
        run_zap_bench(&p, surface.as_ref(), &urls, count, dwell, &[])
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

/// Where the engine lives relative to the executable when there is no Tauri resource dir yet
/// (`--self-test` runs before the app is built): Windows `mpv\`, AppImage `usr/lib`, deb/rpm
/// private dir, or `DOWNLINK_MPV_DIR` for CI staging.
fn exe_relative_mpv_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DOWNLINK_MPV_DIR") {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("mpv"),
        dir.join("resources").join("mpv"),
        dir.join("../lib/downlink/mpv"),
        dir.join("../lib"),
        dir.join("../Frameworks"),
        dir.join("../../Frameworks"),
    ];
    let names = downlink_player::loader::library_file_names();
    candidates
        .into_iter()
        .find(|c| names.iter().any(|n| c.join(n).is_file()))
}

/// `--self-test`: load the bundled libmpv, create an engine, play a synthetic lavfi source with
/// `vo=null` for 2 s, print OK/FAIL. Used by the CI clean-machine smoke test; never touches the
/// network. Set `DOWNLINK_REQUIRE_BUNDLED=1` to forbid the system libmpv.
fn self_test() -> Result<String, String> {
    use downlink_player::engine::{Engine, EngineEvent};
    let loaded = downlink_player::load_from_env(exe_relative_mpv_dir().as_deref())
        .map_err(|e| e.to_string())?;
    let opts: Vec<(String, String)> = [
        ("vo", "null"),
        ("ao", "null"),
        ("idle", "yes"),
        ("terminal", "no"),
        ("ytdl", "no"),
        ("load-scripts", "no"),
        ("input-default-bindings", "no"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    let engine = Engine::new(&loaded, &opts).map_err(|e| e.to_string())?;
    let version = engine.get_property("mpv-version").unwrap_or_default();
    engine
        .command(&[
            "loadfile",
            "av://lavfi:testsrc=size=320x240:rate=25",
            "replace",
        ])
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut first_frame = None;
    while Instant::now() < deadline {
        match engine.wait_event(0.1) {
            EngineEvent::PlaybackRestart if first_frame.is_none() => {
                first_frame = Some(Instant::now())
            }
            EngineEvent::EndFile { error, .. } if !error.is_empty() => {
                return Err(format!("playback failed: {error}"));
            }
            _ => {}
        }
        if first_frame.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
            break;
        }
    }
    if first_frame.is_none() {
        return Err("no first frame within 15 s".into());
    }
    let pos: f64 = engine
        .get_property("time-pos")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    if pos < 1.0 {
        return Err(format!("playback did not advance (time-pos {pos:.2})"));
    }
    Ok(format!(
        "{version}, engine source {:?}, advanced {pos:.1}s",
        loaded.source
    ))
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let mut state = AppState {
        player: None,
        surface: None,
        load_error: None,
    };

    match downlink_player::load_from_env(bundled_mpv_dir(&handle).as_deref()) {
        Ok(loaded) => {
            tracing::info!(source = ?loaded.source, api = ?loaded.client_api_version, "libmpv loaded");
            let emit_handle = handle.clone();
            let sink: downlink_player::EventSink = Arc::new(move |e| match e {
                PlayerEvent::State(s) => {
                    let _ = emit_handle.emit("player://state", s);
                }
                PlayerEvent::Stats(st) => {
                    let _ = emit_handle.emit("session://stats", st);
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
                    let attach = if std::env::var_os("DOWNLINK_SPIKE_NO_SURFACE").is_some() {
                        Err("surface disabled by DOWNLINK_SPIKE_NO_SURFACE".to_owned())
                    } else {
                        attach_surface(&handle, &p, dispatch)
                    };
                    match attach {
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
    // Rule 6: sleep and quit pause and release the stream (protects provider slots).
    let quit = downlink_platform::QuitHooks::new();
    if let Some(p) = state.player.clone() {
        let p2 = p.clone();
        quit.register(move || {
            let _ = p2.stop();
        });
        let cb: downlink_platform::PowerCallback = Arc::new(move |e| {
            if e == downlink_platform::PowerEvent::Sleep {
                let _ = p.stop();
            }
        });
        if let Err(e) = downlink_platform::watch_power(cb) {
            tracing::warn!("sleep notifications unavailable: {e}");
        }
    }
    app.manage(quit);
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
    let view = win.ns_window().map_err(|e| e.to_string())?;
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

fn main() {
    if std::env::args().any(|a| a == "--self-test") {
        match self_test() {
            Ok(msg) => {
                println!("OK {msg}");
                std::process::exit(0);
            }
            Err(e) => {
                println!("FAIL {e}");
                std::process::exit(1);
            }
        }
    }
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
            set_preferences,
            audio_devices,
            set_audio_device,
            set_stats_enabled,
            player_diagnostics,
            zap_bench
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(q) = handle.try_state::<downlink_platform::QuitHooks>()
            {
                q.run_all();
            }
        }),
        Err(e) => {
            tracing::error!("tauri failed: {e}");
            std::process::exit(1);
        }
    }
}
