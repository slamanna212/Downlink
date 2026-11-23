//! Start-up work that runs after the window is up. Order: service + logging (stage 1), then the
//! engine, surface, tray, media session and probes (stage 2), then optional resume.

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::auth::KeyringStore;
use downlink_core::service::{LogTail, Service, ServiceConfig};
use downlink_player::{MainThreadDispatch, Player, RenderBackend, VideoSurface};
use tauri::{AppHandle, Emitter, Manager};

use crate::player_glue::{Playback, SinkContext};
use crate::state::{AppState, Core, Engine};
use crate::{dto, resume, selftest, surface, tray};

fn phase(name: &str) {
    if let Some((t, _)) = crate::START.get() {
        tracing::info!(target: "startup", "{name}: {} ms", t.elapsed().as_millis());
    }
}

/// Run `f` on the UI thread and wait for its result.
pub(crate) async fn on_main<T: Send + 'static>(
    h: &AppHandle,
    f: impl FnOnce(&AppHandle) -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let h2 = h.clone();
    h.run_on_main_thread(move || {
        let _ = tx.send(f(&h2));
    })
    .ok()?;
    rx.await.ok()
}

pub async fn run(handle: AppHandle, version: String, data_dir: PathBuf, log_dir: PathBuf) {
    let st = handle.state::<AppState>();

    // ---- stage 1: service and logging ------------------------------------------------------
    let service = Service::new(ServiceConfig {
        data_dir,
        secrets: Arc::new(KeyringStore::new()),
        app_version: version.clone(),
    })
    .await;
    phase("service opened");
    let (settings, log) = match &service {
        Ok(s) => {
            let settings = s.get_settings().await.unwrap_or_default();
            let debug = settings
                .get("debugLogging")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            (
                settings,
                crate::logging::init(&log_dir, s.log_tail(), debug),
            )
        }
        Err(_) => (
            serde_json::Value::Null,
            crate::logging::init(&log_dir, LogTail::new(), false),
        ),
    };
    tracing::info!("Downlink {version} starting");
    if let Err(e) = &service {
        tracing::error!("the library could not be opened: {}", e.message);
    }
    if let Some(ch) = settings
        .get("updateChannel")
        .and_then(serde_json::Value::as_str)
    {
        st.updater.apply_saved_channel(ch);
    }
    st.core.set(Core {
        service: service.clone(),
        log,
    });
    resume::mark_core_ready();
    phase("core ready");

    let Ok(service) = service else {
        // Degraded mode: the UI shows the blocking prompt; the engine stays off.
        st.engine.set(Engine {
            player: None,
            playback: None,
            surface: None,
            error: None,
        });
        spawn_shell_only(&handle).await;
        #[cfg(feature = "e2e")]
        if std::env::var("DOWNLINK_E2E").is_ok_and(|m| m == "degraded") {
            crate::e2e::spawn(handle.clone(), "degraded".into());
        }
        return;
    };

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
    // Crash recovery (rule 6), the reachability probe, then background refresh.
    {
        let s = service.clone();
        let h_refresh = handle.clone();
        tauri::async_runtime::spawn(async move {
            match s.startup_cleanup().await {
                Ok(r) if r.stopped + r.failed > 0 => {
                    tracing::info!(
                        "startup cleanup: {} stopped, {} failed",
                        r.stopped,
                        r.failed
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("startup cleanup failed: {}", e.message),
            }
            if let Err(e) = s.probe_sources().await {
                tracing::warn!("source probe failed: {}", e.message);
            }
            // Background refresh: a cheap DB check every few minutes; a source is only synced
            // once its data is older than the `autoRefreshHours` setting (or it was offline and
            // may be back). `refreshDuringPlayback` off defers it while something plays.
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(5 * 60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await; // the first tick is immediate; launch already refreshed
            loop {
                tick.tick().await;
                if let Err(e) = s.auto_refresh(is_playing(&h_refresh)).await {
                    tracing::warn!("auto refresh failed: {}", e.message);
                }
                // Between full syncs: the guide's visible hours, once they are an hour old.
                if let Err(e) = s.refresh_visible_guides().await {
                    tracing::warn!("guide refresh failed: {}", e.message);
                }
                if let Err(e) = s.housekeeping().await {
                    tracing::warn!("housekeeping failed: {}", e.message);
                }
            }
        });
    }

    // ---- stage 2: engine, surface, shell integrations --------------------------------------
    let sink_ctx = SinkContext::new(handle.clone(), service.clone());
    let mut player = None;
    let mut playback = None;
    let mut surf = None;
    let mut error = None;
    let h2 = handle.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        downlink_player::load_from_env(&selftest::bundled_mpv_dirs(Some(&h2)))
    })
    .await;
    phase("libmpv loaded");
    match loaded {
        Ok(Ok(loaded)) => {
            tracing::info!(source = ?loaded.source, api = ?loaded.client_api_version, "libmpv loaded");
            // Our own timeshift cache dir only: emptied on every start (mpv unlinks its cache
            // files, but a crash can leave one behind).
            if let Ok(dir) = handle.path().app_cache_dir() {
                let dir = dir.join("timeshift");
                let _ = std::fs::remove_dir_all(&dir);
                match std::fs::create_dir_all(&dir) {
                    Ok(()) => dto::set_timeshift_dir(dir.to_string_lossy().into_owned()),
                    Err(e) => tracing::warn!("timeshift cache dir unavailable: {e}"),
                }
            }
            let prefs = dto::preferences_from_settings(&settings);
            let sink = sink_ctx.sink();
            let loaded_for_tiles = loaded.clone();
            let p = tokio::task::spawn_blocking(move || {
                Player::new(&loaded, prefs, RenderBackend::platform_default(), sink)
            })
            .await;
            phase("engine pre-warmed");
            match p {
                Ok(Ok(p)) => {
                    let p = Arc::new(p);
                    let (pp, ctx) = (p.clone(), sink_ctx.clone());
                    // The GTK/AppKit/Win32 surface, tray and media session live on the UI thread.
                    let r = on_main(&handle, move |h| {
                        let dh = h.clone();
                        let dispatch: MainThreadDispatch = Arc::new(move |f| {
                            let _ = dh.run_on_main_thread(f);
                        });
                        let surf = surface::attach(h, &pp, dispatch);
                        let surf = surf.map(Arc::new);
                        if let Ok(s) = &surf {
                            let for_rebind = s.clone();
                            pp.on_engine_replaced(move |engine| for_rebind.rebind(engine));
                        }
                        let _ = ctx.extras.player.set(pp.clone());
                        surf
                    })
                    .await;
                    phase("surface attached");
                    match r {
                        Some(Ok(s)) => surf = Some(s),
                        Some(Err(e)) => {
                            tracing::error!("video surface unavailable: {e}");
                            error = Some(e);
                        }
                        None => error = Some("the window is not available".into()),
                    }
                    let pb = Playback::new(service.clone(), p.clone(), &sink_ctx);
                    let dh = handle.clone();
                    let dispatch: MainThreadDispatch = Arc::new(move |f| {
                        let _ = dh.run_on_main_thread(f);
                    });
                    let _ = pb.multiview.set(crate::multiview::Multiview::new(
                        handle.clone(),
                        service.clone(),
                        loaded_for_tiles,
                        dispatch,
                        p.clone(),
                    ));
                    playback = Some(pb);
                    player = Some(p);
                }
                Ok(Err(e)) => error = Some(e.to_string()),
                Err(e) => error = Some(e.to_string()),
            }
        }
        Ok(Err(e)) => {
            tracing::error!("{e}");
            error = Some(e.to_string());
        }
        Err(e) => error = Some(e.to_string()),
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

    // Media session needs the sink context's extras; create it with the tray on the UI thread.
    let ctx = sink_ctx.clone();
    on_main(&handle, move |h| {
        media_session(h, &ctx);
        if let Err(e) = tray::build(h) {
            tracing::warn!("system tray unavailable: {e}");
        }
    })
    .await;
    phase("shell integrations");

    st.engine.set(Engine {
        player: player.clone(),
        playback,
        surface: surf.clone(),
        error,
    });
    resume::mark_setup_done();
    phase("engine ready");

    #[cfg(feature = "spike")]
    if let (Some(p), Ok(urls)) = (player.clone(), std::env::var("DOWNLINK_SPIKE_URLS")) {
        crate::spike::spawn_autorun(handle.clone(), p, surf.clone(), urls);
    }
    #[cfg(feature = "e2e")]
    if let Ok(mode) = std::env::var("DOWNLINK_E2E") {
        crate::e2e::spawn(handle.clone(), mode);
    }
    if settings
        .get("resumeLastChannel")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        resume::spawn(handle.clone());
    }
}

/// Degraded mode still gets a tray (so the user can quit).
async fn spawn_shell_only(handle: &AppHandle) {
    on_main(handle, |h| {
        if let Err(e) = tray::build(h) {
            tracing::warn!("system tray unavailable: {e}");
        }
    })
    .await;
}

fn media_session(app: &AppHandle, sink_ctx: &SinkContext) {
    #[cfg(target_os = "windows")]
    let hwnd = app
        .get_webview_window("main")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as *mut std::ffi::c_void);
    #[cfg(not(target_os = "windows"))]
    let hwnd: Option<*mut std::ffi::c_void> = None;
    let h = app.clone();
    let handler: downlink_platform::MediaHandler = Arc::new(move |c| {
        use downlink_platform::MediaCommand as M;
        let Some(st) = h.try_state::<AppState>() else {
            return;
        };
        let Some(pb) = st.try_playback() else {
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

/// Whether the main player is showing something (multiview tiles only exist alongside it).
fn is_playing(handle: &AppHandle) -> bool {
    use downlink_player::SessionState::*;
    handle
        .try_state::<crate::state::AppState>()
        .and_then(|st| st.try_playback())
        .is_some_and(|pb| {
            matches!(
                pb.snapshot().state,
                Some(Resolving | Connecting | Buffering | Playing)
            )
        })
}
