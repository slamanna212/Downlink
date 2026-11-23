use tauri::{AppHandle, State};

use super::CmdResult;
use crate::release_notes::{self, ReleaseNotesDto};
use crate::state::AppState;
use crate::updates::UpdateStatus;
use downlink_core::AppError;

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

/// The GitHub release body (Markdown) for `version`, or the latest.json notes if GitHub can't be reached.
#[tauri::command]
pub async fn get_release_notes(
    state: State<'_, AppState>,
    version: String,
) -> CmdResult<ReleaseNotesDto> {
    if !release_notes::valid_version(&version) {
        return Err(AppError::local("Unknown version."));
    }
    let st = state.updater.status();
    let fallback = st
        .notes
        .filter(|_| st.available.as_deref() == Some(version.as_str()));
    Ok(release_notes::release_notes_default(&version, fallback).await)
}

/// Open a link from release notes in the system browser (web links only).
#[tauri::command]
pub async fn open_external_link(app: AppHandle, url: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !release_notes::external_link_allowed(&url) {
        return Err(AppError::local("That link cannot be opened."));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::local(format!("Could not open the browser: {e}")))
}
