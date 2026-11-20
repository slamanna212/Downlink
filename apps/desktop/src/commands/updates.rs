use tauri::{AppHandle, State};

use super::CmdResult;
use crate::state::AppState;
use crate::updates::UpdateStatus;

#[tauri::command]
pub async fn get_update_status(state: State<'_, AppState>) -> CmdResult<UpdateStatus> {
    Ok(state.updater.status())
}

#[tauri::command]
pub async fn set_update_channel(
    app: AppHandle,
    state: State<'_, AppState>,
    channel: String,
) -> CmdResult<UpdateStatus> {
    state
        .updater
        .set_channel(&app, &*state.svc().await?, &channel)
        .await
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<UpdateStatus> {
    Ok(state.updater.check(&app).await)
}

/// Install the downloaded update and relaunch. Every playback session is stopped first (rule 6).
#[tauri::command]
pub async fn install_update(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(pb) = state.try_playback() {
        pb.release_all().await;
    }
    state.updater.install_now()?;
    app.restart()
}
