use serde_json::{Value, json};
use tauri::State;

use super::CmdResult;
use crate::dto::phase_name;
use crate::state::AppState;

#[tauri::command]
pub async fn get_diagnostics(state: State<'_, AppState>) -> CmdResult<Value> {
    let d = state.service.diagnostics().await?;
    let (player_state, hwdec, transitions) = match &state.playback {
        Some(pb) => (
            serde_json::to_value(pb.snapshot_dto()).unwrap_or(Value::Null),
            pb.snapshot().hwdec_current,
            pb.transitions()
                .iter()
                .map(|t| json!({ "at": t.at_unix_ms, "from": phase_name(t.from), "to": phase_name(t.to) }))
                .collect::<Vec<_>>(),
        ),
        None => (Value::Null, None, Vec::new()),
    };
    Ok(json!({
        "appVersion": d.app_version,
        "serverVersions": d.server_versions,
        "playerState": player_state,
        "hwdecCurrent": hwdec,
        "os": d.os,
        "transitions": transitions,
        "logTail": d.log_tail,
    }))
}

#[tauri::command]
pub async fn set_debug_logging(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    state
        .service
        .set_settings(json!({ "debugLogging": enabled }))
        .await?;
    state.log.set_debug(enabled);
    Ok(())
}
