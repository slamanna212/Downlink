use downlink_core::AppError;
use downlink_player::{PlayerCommand, SurfaceBounds, VideoSurface};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::CmdResult;
use crate::dto::PlayerStateDto;
use crate::state::AppState;

#[tauri::command]
pub async fn play_channel(state: State<'_, AppState>, channel_id: i64) -> CmdResult<()> {
    state.playback()?.play_channel(channel_id).await
}

#[tauri::command]
pub async fn play_catchup(
    state: State<'_, AppState>,
    programme_id: String,
    resume: Option<bool>,
) -> CmdResult<()> {
    state
        .playback()?
        .play_catchup(&programme_id, resume.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn restart_programme(state: State<'_, AppState>, channel_id: i64) -> CmdResult<()> {
    state.playback()?.restart_programme(channel_id).await
}

#[tauri::command]
pub async fn get_player_state(state: State<'_, AppState>) -> CmdResult<PlayerStateDto> {
    Ok(state.playback()?.snapshot_dto())
}

/// `PlayerCommand` of types.ts (plus `togglePause` for the spike page).
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UiCommand {
    Pause,
    Resume,
    TogglePause,
    Stop,
    Seek { seconds: f64 },
    GoLive,
    SetVolume { volume: f64 },
    SetMuted { muted: bool },
    Retry,
}

#[tauri::command]
pub async fn player_command(state: State<'_, AppState>, command: UiCommand) -> CmdResult<()> {
    let pb = state.playback()?;
    match command {
        UiCommand::Pause => pb.command(PlayerCommand::Pause),
        UiCommand::Resume => pb.command(PlayerCommand::Resume),
        UiCommand::TogglePause => pb.command(PlayerCommand::TogglePause),
        UiCommand::Stop => pb.stop().await,
        UiCommand::Seek { seconds } => pb.command(PlayerCommand::SeekAbsolute { secs: seconds }),
        UiCommand::GoLive => pb.go_live().await,
        UiCommand::SetVolume { volume } => pb.command(PlayerCommand::Volume {
            percent: volume.clamp(0.0, 130.0).round() as u8,
        }),
        UiCommand::SetMuted { muted } => pb.command(PlayerCommand::Mute { muted }),
        UiCommand::Retry => pb.retry().await,
    }
}

/// Video rect in physical pixels (window client coordinates). `visible` defaults to true.
#[tauri::command]
pub async fn set_video_bounds(
    state: State<'_, AppState>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    visible: Option<bool>,
) -> CmdResult<()> {
    if let Some(s) = &state.surface {
        s.set_bounds(SurfaceBounds { x, y, w, h, scale });
        s.set_visible(visible.unwrap_or(true));
    }
    Ok(())
}

#[tauri::command]
pub async fn set_fullscreen(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::local("The window is not available."))?;
    win.set_fullscreen(enabled)
        .map_err(|e| AppError::local(format!("Could not change fullscreen: {e}")))
}

#[tauri::command]
pub async fn set_stats_enabled(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    if let Some(p) = &state.player {
        p.set_stats_enabled(enabled);
    }
    Ok(())
}

#[derive(Serialize)]
pub struct AudioDevice {
    id: String,
    name: String,
}

#[tauri::command]
pub async fn list_audio_devices(state: State<'_, AppState>) -> CmdResult<Vec<AudioDevice>> {
    Ok(state
        .player
        .as_ref()
        .map(|p| {
            p.audio_devices()
                .into_iter()
                .map(|d| AudioDevice {
                    id: d.name,
                    name: d.description,
                })
                .collect()
        })
        .unwrap_or_default())
}

#[tauri::command]
pub async fn play_vod(
    state: State<'_, AppState>,
    kind: String,
    id: i64,
    resume: Option<bool>,
) -> CmdResult<()> {
    state
        .playback()?
        .play_vod(&kind, id, resume.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn play_recording(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.playback()?.play_recording(id).await
}
