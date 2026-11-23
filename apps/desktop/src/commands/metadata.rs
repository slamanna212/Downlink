//! Title details and the TMDB key: thin wrappers over the core `Service`.

use downlink_core::auth::SecretString;
use downlink_core::service::{MediaDetailsDto, MediaTargetArg, TmdbStatusDto};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

/// Rich details for a programme, recording or VOD title (source first, then TMDB).
#[tauri::command]
pub async fn media_details(
    state: State<'_, AppState>,
    target: MediaTargetArg,
) -> CmdResult<MediaDetailsDto> {
    state.svc().await?.media_details(target).await
}

#[tauri::command]
pub async fn tmdb_status(state: State<'_, AppState>) -> CmdResult<TmdbStatusDto> {
    state.svc().await?.tmdb_status().await
}

/// Check a TMDB key and keep it in the keychain. The key never comes back to the UI.
#[tauri::command]
pub async fn tmdb_set_key(state: State<'_, AppState>, key: String) -> CmdResult<TmdbStatusDto> {
    state
        .svc()
        .await?
        .tmdb_set_key(SecretString::new(key))
        .await
}

#[tauri::command]
pub async fn tmdb_clear_key(state: State<'_, AppState>) -> CmdResult<TmdbStatusDto> {
    state.svc().await?.tmdb_clear_key().await
}
