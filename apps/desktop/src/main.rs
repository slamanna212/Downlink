//! Downlink desktop shell: Tauri 2 commands over the core `Service`, the libmpv `Player`, the
//! native video surface under the transparent webview, logging, updates and the `dlimg://` protocol.

mod boot;
mod bundle;
mod commands;
mod deeplink;
mod dto;
#[cfg(feature = "e2e")]
mod e2e;
mod hls_proxy;
mod logging;
mod multiview;
mod player_glue;
mod protocol;
mod release_notes;
mod resume;
mod selftest;
#[cfg(feature = "spike")]
mod spike;
mod state;
mod surface;
mod tray;
mod updates;

use std::path::PathBuf;

use tauri::Manager;

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

    let data_dir = data_dir(app);
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let updater = updates::UpdaterState::new(&version, None);
    if std::env::var_os("DOWNLINK_NO_UPDATE_CHECK").is_none() {
        updater.spawn_loop(handle.clone());
    }
    // Deep links need no engine: register now so a launch URL is not lost.
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let dl = handle.deep_link();
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
    app.manage(AppState::new(updater, data_dir.clone()));
    resume::mark_setup_returned();
    // Everything heavy happens after the window is up; commands wait for it (see `state`).
    tauri::async_runtime::spawn(boot::run(handle, version, data_dir, log_dir));
    Ok(())
}

macro_rules! handlers {
    ($($extra:path),* $(,)?) => {
        tauri::generate_handler![
            commands::catalog::list_sources,
            commands::catalog::add_source,
            commands::catalog::test_source,
            commands::catalog::remove_source,
            commands::catalog::set_source_options,
            commands::catalog::sync_source,
            commands::catalog::list_groups,
            commands::catalog::list_channels,
            commands::catalog::get_channel,
            commands::catalog::find_channel_number,
            commands::catalog::get_programme_window,
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
            commands::player::set_mini_player,
            commands::player::set_stats_enabled,
            commands::player::list_audio_devices,
            commands::player::get_video_caps,
            commands::player::get_channel_streams,
            commands::player::switch_stream,
            commands::player::get_multiview,
            commands::player::multiview_set_tile,
            commands::player::multiview_remove_tile,
            commands::player::multiview_set_audio,
            commands::player::multiview_make_main,
            commands::player::multiview_set_bounds,
            commands::player::multiview_exit,
            commands::settings::get_settings,
            commands::settings::set_settings,
            commands::diagnostics::get_diagnostics,
            commands::diagnostics::set_debug_logging,
            commands::diagnostics::export_diagnostics,
            commands::diagnostics::reveal_path,
            commands::app::report_ui_timing,
            commands::app::archive_newer_database,
            commands::app::trust_new_certificate,
            commands::app::quit_app,
            commands::catalog::get_channel_by_uuid,
            commands::updates::get_update_status,
            commands::updates::set_update_channel,
            commands::updates::check_for_updates,
            commands::updates::install_update,
            commands::updates::get_release_notes,
            commands::updates::open_external_link,
            commands::mediaserver::media_server_status,
            commands::mediaserver::plex_sign_in_start,
            commands::mediaserver::plex_sign_in_check,
            commands::mediaserver::plex_servers,
            commands::mediaserver::plex_select_server,
            commands::mediaserver::media_server_sign_out,
            commands::mediaserver::media_server_match,
            commands::mediaserver::media_server_open,
            commands::metadata::media_details,
            commands::metadata::tmdb_status,
            commands::metadata::tmdb_set_key,
            commands::metadata::tmdb_clear_key,
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
                if let Some(pb) = st.try_playback() {
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
