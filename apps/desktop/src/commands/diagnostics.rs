use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

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

/// Write the diagnostics zip into the downloads dir and return its path.
#[tauri::command]
pub async fn export_diagnostics(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    use downlink_core::AppError;
    let d = state.service.diagnostics().await?;
    let (player_state, hwdec, transitions, mpv) = match &state.playback {
        Some(pb) => (
            serde_json::to_value(pb.snapshot_dto()).unwrap_or(Value::Null),
            pb.snapshot().hwdec_current,
            pb.transitions()
                .iter()
                .rev()
                .take(50)
                .rev()
                .map(|t| json!({ "at": t.at_unix_ms, "from": phase_name(t.from), "to": phase_name(t.to), "note": t.note }))
                .collect::<Vec<_>>(),
            state.player.as_ref().map(|p| p.diagnostics()).unwrap_or_default(),
        ),
        None => (Value::Null, None, Vec::new(), Vec::new()),
    };
    let mut entries: Vec<(String, String)> = vec![
        (
            "app.json".into(),
            serde_json::to_string_pretty(&json!({
                "appVersion": d.app_version,
                "serverVersions": d.server_versions,
                "os": d.os,
                "arch": std::env::consts::ARCH,
                "engineError": state.engine_error,
                "update": state.updater.status(),
            }))
            .unwrap_or_default(),
        ),
        (
            "player.json".into(),
            serde_json::to_string_pretty(&json!({
                "state": player_state,
                "hwdecCurrent": hwdec,
                "transitions": transitions,
                // GPU / renderer / decoder facts as reported by mpv.
                "mpv": mpv.iter().map(|(k, v)| json!({ k: v })).collect::<Vec<_>>(),
            }))
            .unwrap_or_default(),
        ),
        ("logs/tail.txt".into(), d.log_tail.join("\n")),
    ];
    if let Ok(dir) = app.path().app_log_dir()
        && let Ok(rd) = std::fs::read_dir(&dir)
    {
        let mut files: Vec<_> = rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("downlink.log"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        for f in files {
            if let Ok(text) = std::fs::read_to_string(f.path()) {
                entries.push((
                    format!("logs/{}", f.file_name().to_string_lossy()),
                    crate::bundle::tail(&text, crate::bundle::LOG_TAIL_BYTES).to_owned(),
                ));
            }
        }
    }
    let zip = crate::bundle::build_zip(&entries)
        .map_err(|e| AppError::local(format!("Could not build the bundle: {e}")))?;
    let dir = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("downlink-diagnostics-{secs}.zip"));
    std::fs::write(&path, zip)
        .map_err(|e| AppError::local(format!("Could not save the bundle: {e}")))?;
    Ok(path.display().to_string())
}
