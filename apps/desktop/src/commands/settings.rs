use serde_json::Value;
use tauri::State;

use super::CmdResult;
use crate::dto::preferences_from_settings;
use crate::state::AppState;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Value> {
    state.service.get_settings().await
}

/// Persist the patch, then apply it: player preferences and the log level.
#[tauri::command]
pub async fn set_settings(state: State<'_, AppState>, patch: Value) -> CmdResult<Value> {
    let v = state.service.set_settings(patch).await?;
    apply(&state, &v);
    Ok(v)
}

pub fn apply(state: &AppState, settings: &Value) {
    if let Some(p) = &state.player {
        p.set_preferences(preferences_from_settings(settings));
    }
    state.log.set_debug(
        settings
            .get("debugLogging")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
}
