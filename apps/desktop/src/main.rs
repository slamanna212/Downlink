//! Downlink desktop shell: Tauri 2 commands over the core `Service`, the libmpv `Player`, the
//! native video surface under the transparent webview, logging, updates and the `dlimg://` protocol.

mod bundle;
mod commands;
mod deeplink;
mod dto;
#[cfg(feature = "e2e")]
mod e2e;
mod logging;
mod player_glue;
mod protocol;
mod resume;
mod selftest;
#[cfg(feature = "spike")]
mod spike;
mod state;
mod surface;
mod tray;
mod updates;

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::auth::KeyringStore;
use downlink_core::service::{Service, ServiceConfig};
use downlink_player::{MainThreadDispatch, Player, RenderBackend};
use tauri::{Emitter, Manager};

use crate::player_glue::{Playback, SinkContext};
use crate::state::AppState;

/// Installed on every page load: F11 toggles fullscreen and Esc always leaves it, so a fullscreen
/// window can never trap the user.
const KEYS_JS: &str = r#"(function () {
  if (window.__dlKeys || !window.__TAURI__) return;
  window.__dlKeys = true;
  const T = window.__TAURI__;
  window.addEventListener('keydown', async (e) => {
    if (e.key !== 'Escape' && e.key !== 'F11') return;
    try {
      const fs = await T.window.getCurrentWindow().isFullscreen();
      if (e.key === 'F11') { e.preventDefault(); await T.core.invoke('set_fullscreen', { enabled: !fs }); }
      else if (fs) { await T.core.invoke('set_fullscreen', { enabled: false }); }
    } catch (_) {}
  }, true);
})();"#;

fn data_dir(app: &tauri::App) -> PathBuf {
    std::env::var_os("DOWNLINK_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| app.path().app_data_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();
    let version = std::env::var("DOWNLINK_VERSION_OVERRIDE")
        .unwrap_or_else(|_| app.package_info().version.to_string());

    // Optional small window for automated runs (never fullscreen).
    if let (Ok(size), Some(win)) = (
        std::env::var("DOWNLINK_WINDOW_SIZE"),
        app.get_webview_window("main"),
    ) && let Some((w, h)) = size
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?)))
    {
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
    }

    let service = tauri::async_runtime::block_on(Service::new(ServiceConfig {
        data_dir: data_dir(app),
        secrets: Arc::new(KeyringStore::new()),
        app_version: version.clone(),
    }))
    .map_err(|e| e.message)?;
    let settings = tauri::async_runtime::block_on(service.get_settings()).unwrap_or_default();
    let debug = settings
        .get("debugLogging")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let log = logging::init(&log_dir, service.log_tail(), debug);
    tracing::info!("Downlink {version} starting");

    // Core events -> webview (catalog://updated, task://progress).
    {
        let mut rx = service.subscribe();
        let h = handle.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        let _ = h.emit(ev.name(), &ev);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        });
    }

    // Crash recovery: delete sessions a previous run left open (rule 6).
    {
        let s = service.clone();
        tauri::async_runtime::spawn(async move {
            match s.startup_cleanup().await {
                Ok(r) if r.stopped + r.failed > 0 => tracing::info!(
                    "startup cleanup: {} stopped, {} failed",
                    r.stopped,
                    r.failed
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("startup cleanup failed: {}", e.message),
            }
        });
    }

    // Reachability probe for every source (offline sources keep their cache); emits
    // `catalog://updated` per source through the event forwarder above.
    {
        let s = service.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = s.probe_sources().await {
                tracing::warn!("source probe failed: {}", e.message);
            }
        });
    }

    // Engine, surface, playback glue.
    let sink_ctx = SinkContext::new(handle.clone(), service.clone());

    // OS media controls / media keys (MPRIS, SMTC, Now Playing).
    {
        #[cfg(target_os = "windows")]
        let hwnd = app
            .get_webview_window("main")
            .and_then(|w| w.hwnd().ok())
            .map(|h| h.0 as *mut std::ffi::c_void);
        #[cfg(not(target_os = "windows"))]
        let hwnd: Option<*mut std::ffi::c_void> = None;
        let h = handle.clone();
        let handler: downlink_platform::MediaHandler = Arc::new(move |c| {
            use downlink_platform::MediaCommand as M;
            let Some(st) = h.try_state::<AppState>() else {
                return;
            };
            let Some(pb) = st.playback.clone() else {
                return;
            };
            match c {
                M::Play => drop(pb.command(downlink_player::PlayerCommand::Resume)),
                M::Pause => drop(pb.command(downlink_player::PlayerCommand::Pause)),
                M::Toggle => drop(pb.command(downlink_player::PlayerCommand::TogglePause)),
                M::Stop => {
                    tauri::async_runtime::spawn(async move {
                        let _ = pb.stop().await;
                    });
                }
                M::Raise => {
                    if let Some(w) = h.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                M::Quit => h.exit(0),
                M::Next | M::Previous => {}
            }
        });
        match downlink_platform::MediaSession::new(hwnd, handler) {
            Ok(m) => {
                let _ = sink_ctx.extras.media.set(m);
            }
            Err(e) => tracing::warn!("media controls unavailable: {e}"),
        }
    }

    if let Err(e) = tray::build(app) {
        tracing::warn!("system tray unavailable: {e}");
    }
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let dl = handle.deep_link();
        // Registers the scheme for dev and portable runs; installers register it themselves.
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Err(e) = dl.register_all() {
            tracing::debug!("deep link registration skipped: {e}");
        }
        let h = handle.clone();
        dl.on_open_url(move |e| {
            let urls: Vec<String> = e.urls().iter().map(ToString::to_string).collect();
            deeplink::forward(&h, urls.iter().map(String::as_str));
        });
        if let Ok(Some(urls)) = dl.get_current() {
            let urls: Vec<String> = urls.iter().map(ToString::to_string).collect();
            deeplink::forward(&handle, urls.iter().map(String::as_str));
        }
    }
    let mut player = None;
    let mut playback = None;
    let mut surf = None;
    let mut engine_error = None;
    match downlink_player::load_from_env(selftest::bundled_mpv_dir(&handle).as_deref()) {
        Ok(loaded) => {
            tracing::info!(source = ?loaded.source, api = ?loaded.client_api_version, "libmpv loaded");
            let prefs = dto::preferences_from_settings(&settings);
            match Player::new(
                &loaded,
                prefs,
                RenderBackend::platform_default(),
                sink_ctx.sink(),
            ) {
                Ok(p) => {
                    let p = Arc::new(p);
                    let dh = handle.clone();
                    let dispatch: MainThreadDispatch = Arc::new(move |f| {
                        let _ = dh.run_on_main_thread(f);
                    });
                    match surface::attach(&handle, &p, dispatch) {
                        Ok(s) => surf = Some(Arc::new(s)),
                        Err(e) => {
                            tracing::error!("video surface unavailable: {e}");
                            engine_error = Some(e);
                        }
                    }
                    let _ = sink_ctx.extras.player.set(p.clone());
                    playback = Some(Playback::new(service.clone(), p.clone(), &sink_ctx));
                    player = Some(p);
                }
                Err(e) => engine_error = Some(e.to_string()),
            }
        }
        Err(e) => {
            tracing::error!("{e}");
            engine_error = Some(e.to_string());
        }
    }

    // Rule 6: sleep pauses and releases the stream.
    if let Some(pb) = playback.clone() {
        let cb: downlink_platform::PowerCallback = Arc::new(move |e| {
            if e == downlink_platform::PowerEvent::Sleep {
                tauri::async_runtime::block_on(pb.release_all());
            }
        });
        if let Err(e) = downlink_platform::watch_power(cb) {
            tracing::warn!("sleep notifications unavailable: {e}");
        }
    }

    let updater = updates::UpdaterState::new(
        &version,
        settings
            .get("updateChannel")
            .and_then(serde_json::Value::as_str),
    );
    if std::env::var_os("DOWNLINK_NO_UPDATE_CHECK").is_none() {
        updater.spawn_loop(handle.clone());
    }

    #[cfg(feature = "spike")]
    if let (Some(p), Ok(urls)) = (player.clone(), std::env::var("DOWNLINK_SPIKE_URLS")) {
        spike::spawn_autorun(handle.clone(), p, surf.clone(), urls);
    }

    #[cfg(feature = "e2e")]
    if let Ok(mode) = std::env::var("DOWNLINK_E2E") {
        e2e::spawn(handle.clone(), mode);
    }

    let resume_enabled = settings
        .get("resumeLastChannel")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    app.manage(AppState {
        service,
        player,
        playback,
        surface: surf,
        log,
        updater,
        engine_error,
    });
    resume::mark_setup_done();
    if resume_enabled {
        resume::spawn(handle.clone());
    }
    Ok(())
}

macro_rules! handlers {
    ($($extra:path),* $(,)?) => {
        tauri::generate_handler![
            commands::catalog::list_sources,
            commands::catalog::add_source,
            commands::catalog::test_source,
            commands::catalog::remove_source,
            commands::catalog::sync_source,
            commands::catalog::list_groups,
            commands::catalog::list_channels,
            commands::catalog::get_now_next,
            commands::catalog::get_guide,
            commands::catalog::get_programmes,
            commands::catalog::get_home,
            commands::catalog::search,
            commands::catalog::set_favourite,
            commands::catalog::get_counts,
            commands::player::play_channel,
            commands::player::play_catchup,
            commands::player::play_vod,
            commands::player::play_recording,
            commands::media::list_vod_categories,
            commands::media::list_movies,
            commands::media::get_movie,
            commands::media::list_series,
            commands::media::get_series,
            commands::media::list_recordings,
            commands::media::schedule_recording,
            commands::media::cancel_recording,
            commands::media::stop_recording,
            commands::media::extend_recording,
            commands::media::delete_recording,
            commands::media::list_recording_rules,
            commands::media::delete_recording_rule,
            commands::player::restart_programme,
            commands::player::get_player_state,
            commands::player::player_command,
            commands::player::set_video_bounds,
            commands::player::set_fullscreen,
            commands::player::set_stats_enabled,
            commands::player::list_audio_devices,
            commands::settings::get_settings,
            commands::settings::set_settings,
            commands::diagnostics::get_diagnostics,
            commands::diagnostics::set_debug_logging,
            commands::diagnostics::export_diagnostics,
            commands::diagnostics::reveal_path,
            commands::catalog::get_channel_by_uuid,
            commands::updates::get_update_status,
            commands::updates::set_update_channel,
            commands::updates::check_for_updates,
            $($extra),*
        ]
    };
}

/// `--tv`: the UI reads `window.__DL_TV__` before it starts (10-foot mode). The init script runs
/// in every page before any page script.
fn tv_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let tv = std::env::args().any(|a| a == "--tv");
    let mut b = tauri::plugin::Builder::new("dl-tv");
    if tv {
        b = b.js_init_script("window.__DL_TV__ = true;".to_owned());
    }
    b.build()
}

/// Process start, for the startup budgets (PLAN §8.8) and the perf harness.
pub static START: std::sync::OnceLock<(std::time::Instant, std::time::SystemTime)> =
    std::sync::OnceLock::new();

#[cfg(feature = "e2e")]
fn perf_plugin_if_enabled() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    e2e::perf_plugin()
}
#[cfg(not(feature = "e2e"))]
fn perf_plugin_if_enabled() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("dl-noop").build()
}

fn main() {
    let _ = START.set((std::time::Instant::now(), std::time::SystemTime::now()));
    if std::env::args().any(|a| a == "--self-test") {
        std::process::exit(selftest::run());
    }
    let builder = tauri::Builder::default();
    // Single instance must be registered first. Dev drivers (spike, e2e) and DOWNLINK_NO_SINGLE_INSTANCE skip it.
    #[cfg(not(any(feature = "spike", feature = "e2e")))]
    let builder = if std::env::var_os("DOWNLINK_NO_SINGLE_INSTANCE").is_some() {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            deeplink::forward(
                app,
                argv.iter()
                    .map(String::as_str)
                    .filter(|a| a.starts_with("downlink://")),
            );
        }))
    };
    let builder = builder
        .plugin(tv_plugin())
        .plugin(perf_plugin_if_enabled())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.eval(KEYS_JS);
            }
        })
        .setup(setup);
    #[cfg(feature = "spike")]
    let builder = builder.invoke_handler(handlers!(
        spike::play_url,
        spike::player_snapshot,
        spike::player_diagnostics,
        spike::zap_bench
    ));
    #[cfg(all(not(feature = "spike"), not(feature = "e2e")))]
    let builder = builder.invoke_handler(handlers!());
    #[cfg(all(not(feature = "spike"), feature = "e2e"))]
    let builder = builder.invoke_handler(handlers!(e2e::perf_report));
    // The service is created in `setup`, so the protocol handler looks it up lazily.
    let builder = protocol::register(builder);
    match builder.build(tauri::generate_context!()) {
        Ok(app) => app.run(|handle, event| {
            if let tauri::RunEvent::Exit = event
                && let Some(st) = handle.try_state::<AppState>()
            {
                if let Some(pb) = &st.playback {
                    tauri::async_runtime::block_on(pb.release_all());
                }
                st.updater.install_on_quit();
            }
        }),
        Err(e) => {
            eprintln!("tauri failed: {e}");
            std::process::exit(1);
        }
    }
}
