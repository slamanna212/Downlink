//! App-level commands that must work even when the library could not be opened.

use std::sync::Mutex;

use downlink_core::AppError;
use downlink_core::service::Service;
use tauri::{AppHandle, State};

use super::CmdResult;
use crate::state::AppState;

/// Latest UI timings by name (ms), for the perf harness and the diagnostics tail.
pub static UI_TIMINGS: Mutex<Vec<(String, f64)>> = Mutex::new(Vec::new());

/// The UI reports named timings (for example `interactive`, `guide-open`). Logged at info and
/// kept for the perf harness; the name is restricted so a compromised page cannot spam the log.
#[tauri::command]
pub async fn report_ui_timing(name: String, ms: f64) -> CmdResult<()> {
    let ok = name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
    if ok && ms.is_finite() && ms >= 0.0 {
        tracing::info!(target: "ui_timing", "{name}: {ms:.0} ms");
        if let Ok(mut t) = UI_TIMINGS.lock() {
            if t.len() >= 200 {
                t.remove(0);
            }
            t.push((name, ms));
        }
    }
    Ok(())
}

/// "This library was created by a newer version": move it aside and restart so the app starts
/// with a fresh library. The old file is kept next to it as a `.bak`.
#[tauri::command]
pub async fn archive_newer_database(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    Service::archive_newer_database(&state.data_dir)?;
    tracing::info!("archived the newer library; restarting");
    app.restart()
}

#[tauri::command]
pub async fn trust_new_certificate(state: State<'_, AppState>, source_id: i64) -> CmdResult<()> {
    state.svc().await?.trust_new_certificate(source_id).await
}

#[tauri::command]
pub async fn quit_app(app: AppHandle) -> CmdResult<()> {
    app.exit(0);
    Ok(())
}

#[allow(dead_code)]
fn _assert_error_type(_: AppError) {}
