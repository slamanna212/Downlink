//! `--self-test`: see `docs/release.md` (smoke job contract).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

pub fn bundled_mpv_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().resource_dir().ok().map(|d| d.join("mpv"))
}

/// Where the engine lives relative to the executable when there is no Tauri resource dir yet
/// (`--self-test` runs before the app is built): Windows `mpv\`, AppImage `usr/lib`, deb/rpm
/// private dir, or `DOWNLINK_MPV_DIR` for CI staging.
fn exe_relative_mpv_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DOWNLINK_MPV_DIR") {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("mpv"),
        dir.join("resources").join("mpv"),
        dir.join("../lib/downlink/mpv"),
        dir.join("../lib"),
        dir.join("../Frameworks"),
        dir.join("../../Frameworks"),
    ];
    let names = downlink_player::loader::library_file_names();
    candidates
        .into_iter()
        .find(|c| names.iter().any(|n| c.join(n).is_file()))
}

fn run_inner() -> Result<(String, String), String> {
    use downlink_player::LoadSource;
    use downlink_player::engine::{Engine, EngineEvent};
    let loaded = downlink_player::load_from_env(exe_relative_mpv_dir().as_deref())
        .map_err(|e| e.to_string())?;
    let path = match &loaded.source {
        LoadSource::Bundled(p) => p.display().to_string(),
        other if std::env::var("DOWNLINK_SELF_TEST_ALLOW_DEV").is_ok_and(|v| v == "1") => {
            format!("{other:?}")
        }
        other => return Err(format!("libmpv did not come from the bundle: {other:?}")),
    };
    let opts: Vec<(String, String)> = [
        ("vo", "null"),
        ("ao", "null"),
        ("idle", "yes"),
        ("terminal", "no"),
        ("ytdl", "no"),
        ("load-scripts", "no"),
        ("input-default-bindings", "no"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    let engine = Engine::new(&loaded, &opts).map_err(|e| e.to_string())?;
    let version = engine.get_property("mpv-version").unwrap_or_default();
    engine
        .command(&[
            "loadfile",
            "av://lavfi:testsrc=size=320x240:rate=25",
            "replace",
        ])
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut first_frame = None;
    while Instant::now() < deadline {
        match engine.wait_event(0.1) {
            EngineEvent::PlaybackRestart if first_frame.is_none() => {
                first_frame = Some(Instant::now())
            }
            EngineEvent::EndFile { error, .. } if !error.is_empty() => {
                return Err(format!("playback failed: {error}"));
            }
            _ => {}
        }
        if first_frame.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
            break;
        }
    }
    if first_frame.is_none() {
        return Err("no first frame within 30 s".into());
    }
    let pos: f64 = engine
        .get_property("time-pos")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    if pos < 1.0 {
        return Err(format!("playback did not advance (time-pos {pos:.2})"));
    }
    Ok((path, version))
}

/// Runs before any window exists. Prints one JSON line (and writes it to `DOWNLINK_SELF_TEST_OUT`
/// when set, for GUI-subsystem executables). Returns the process exit code.
pub fn run() -> i32 {
    let (ok, json) = match run_inner() {
        Ok((libmpv, mpv_version)) => (
            true,
            serde_json::json!({ "ok": true, "libmpv": libmpv, "mpv_version": mpv_version }),
        ),
        Err(e) => (false, serde_json::json!({ "ok": false, "error": e })),
    };
    let line = downlink_core::redact::redact(&json.to_string());
    println!("{line}");
    if let Some(p) = std::env::var_os("DOWNLINK_SELF_TEST_OUT") {
        let _ = std::fs::write(p, &line);
    }
    i32::from(!ok)
}
