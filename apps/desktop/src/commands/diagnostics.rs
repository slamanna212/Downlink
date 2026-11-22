use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

use super::CmdResult;
use crate::dto::phase_name;
use crate::state::AppState;

#[tauri::command]
pub async fn get_diagnostics(state: State<'_, AppState>) -> CmdResult<Value> {
    let d = state.svc().await?.diagnostics().await?;
    let (player_state, hwdec, transitions) = match state.playback().await.ok() {
        Some(pb) => (
            serde_json::to_value(pb.snapshot_dto()).unwrap_or(Value::Null),
            pb.snapshot().hwdec_current,
            pb.transitions()
                .iter()
                .map(|t| json!({ "at": t.at_unix_ms, "from": phase_name(t.from), "to": phase_name(t.to), "note": t.note }))
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
        "playerNotes": state.playback().await.ok().map(|p| p.notes()).unwrap_or_default(),
        "logTail": d.log_tail,
    }))
}

#[tauri::command]
pub async fn set_debug_logging(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    state
        .svc()
        .await?
        .set_settings(json!({ "debugLogging": enabled }))
        .await?;
    state.log().await.set_debug(enabled);
    Ok(())
}

/// Write the diagnostics zip into the downloads dir and return its path.
#[tauri::command]
pub async fn export_diagnostics(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    use downlink_core::AppError;
    let d = state.svc().await?.diagnostics().await?;
    let (player_state, hwdec, transitions, mpv) = match state.playback().await.ok() {
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
            state.player().await.map(|p| p.diagnostics()).unwrap_or_default(),
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
                "engineError": state.engine.now().and_then(|e| e.error.clone()),
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
                "notes": state.playback().await.ok().map(|p| p.notes()).unwrap_or_default(),
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
    let mut out = tempfile::Builder::new()
        .prefix("downlink-diagnostics-")
        .suffix(".zip")
        .tempfile_in(dir)
        .map_err(|e| AppError::local(format!("Could not create the bundle: {e}")))?;
    std::io::Write::write_all(out.as_file_mut(), &zip)
        .map_err(|e| AppError::local(format!("Could not save the bundle: {e}")))?;
    let (_, path) = out
        .keep()
        .map_err(|e| AppError::local(format!("Could not keep the bundle: {e}")))?;
    Ok(path.display().to_string())
}

/// True when `path` (canonicalised) lies inside one of `roots` (canonicalised).
pub fn path_allowed(path: &std::path::Path, roots: &[std::path::PathBuf]) -> bool {
    let Ok(p) = path.canonicalize() else {
        return false;
    };
    roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| p.starts_with(r))
}

/// Open the folder containing `path` in the OS file manager. Only paths inside the downloads,
/// app data or log directories are accepted (the UI passes the diagnostics bundle path).
#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: String) -> CmdResult<()> {
    use downlink_core::AppError;
    use tauri_plugin_opener::OpenerExt;
    let p = std::path::PathBuf::from(&path);
    let roots: Vec<std::path::PathBuf> = [
        app.path().download_dir().ok(),
        app.path().app_data_dir().ok(),
        app.path().app_log_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !path_allowed(&p, &roots) {
        return Err(AppError::local("That location cannot be opened."));
    }
    app.opener()
        .reveal_item_in_dir(&p)
        .map_err(|e| AppError::local(format!("Could not open the folder: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_roots_are_allowed() {
        let base = std::env::temp_dir().join(format!("dl-reveal-{}", std::process::id()));
        let inside = base.join("dl");
        let _ = std::fs::create_dir_all(&inside);
        let f = inside.join("x.zip");
        let _ = std::fs::write(&f, b"x");
        assert!(path_allowed(&f, std::slice::from_ref(&inside)));
        assert!(!path_allowed(
            &std::env::temp_dir(),
            std::slice::from_ref(&inside)
        ));
        assert!(!path_allowed(
            &inside.join("../../etc/passwd"),
            std::slice::from_ref(&inside)
        ));
        assert!(!path_allowed(
            &inside.join("missing"),
            std::slice::from_ref(&inside)
        ));
        let _ = std::fs::remove_dir_all(&base);
    }
}
