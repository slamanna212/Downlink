//! "Resume last channel on launch" (setting `resumeLastChannel`, off by default) and the startup
//! timestamps the perf harness reads.

use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::START;
use crate::state::AppState;

fn since_start() -> Duration {
    START.get().map(|(i, _)| i.elapsed()).unwrap_or_default()
}

/// When `setup` finished (offset from process start).
pub static SETUP_DONE: OnceLock<Duration> = OnceLock::new();
/// When the resume `play_channel` was issued (offset from process start).
pub static RESUME_ISSUED: OnceLock<Duration> = OnceLock::new();

/// When `setup` returned and the window could start loading (offset from process start).
pub static SETUP_RETURNED: OnceLock<Duration> = OnceLock::new();
/// When the service and logging were ready.
pub static CORE_READY: OnceLock<Duration> = OnceLock::new();

pub fn mark_setup_returned() {
    let _ = SETUP_RETURNED.set(since_start());
}

pub fn mark_core_ready() {
    let _ = CORE_READY.set(since_start());
}

/// When the engine stage finished (despite the name: "setup" in the original harness).
pub fn mark_setup_done() {
    let _ = SETUP_DONE.set(since_start());
}

/// Play the most recently watched channel as soon as the engine is up. Never blocks the UI.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(st) = app.try_state::<AppState>() else {
            return;
        };
        let Ok(pb) = st.playback().await else { return };
        let Ok(svc) = st.svc().await else { return };
        let Ok(home) = svc.get_home().await else {
            return;
        };
        let Some(ch) = home.recents.first() else {
            return;
        };
        let _ = RESUME_ISSUED.set(since_start());
        match pb.play_channel(ch.id).await {
            Ok(()) => tracing::info!("resumed the last channel"),
            Err(e) => tracing::warn!("could not resume the last channel: {}", e.message),
        }
    });
}
