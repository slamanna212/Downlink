use std::sync::Arc;

use downlink_core::AppError;
use downlink_player::{PlayerCommand, SurfaceBounds, VideoSurface};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use super::CmdResult;
use crate::dto::PlayerStateDto;
use crate::state::AppState;

#[tauri::command]
pub async fn play_channel(state: State<'_, AppState>, channel_id: i64) -> CmdResult<()> {
    state.playback().await?.play_channel(channel_id).await
}

#[tauri::command]
pub async fn play_catchup(
    state: State<'_, AppState>,
    programme_id: String,
    resume: Option<bool>,
) -> CmdResult<()> {
    state
        .playback()
        .await?
        .play_catchup(&programme_id, resume.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn restart_programme(state: State<'_, AppState>, channel_id: i64) -> CmdResult<()> {
    state.playback().await?.restart_programme(channel_id).await
}

#[tauri::command]
pub async fn get_player_state(state: State<'_, AppState>) -> CmdResult<PlayerStateDto> {
    Ok(state.playback().await?.snapshot_dto())
}

/// `PlayerCommand` of types.ts (plus `togglePause` for the spike page).
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UiCommand {
    Pause,
    Resume,
    TogglePause,
    Stop,
    Seek {
        seconds: f64,
    },
    GoLive,
    SetVolume {
        volume: f64,
    },
    SetMuted {
        muted: bool,
    },
    SetAudioTrack {
        id: i64,
    },
    /// `null` turns subtitles off.
    SetSubtitleTrack {
        id: Option<i64>,
    },
    Retry,
}

#[tauri::command]
pub async fn player_command(state: State<'_, AppState>, command: UiCommand) -> CmdResult<()> {
    let pb = state.playback().await?;
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
        UiCommand::SetAudioTrack { id } => pb.command(PlayerCommand::AudioTrack { id }),
        UiCommand::SetSubtitleTrack { id } => pb.command(PlayerCommand::SubtitleTrack { id }),
        UiCommand::Retry => pb.retry().await,
    }
}

/// The channel's provider streams in failover order, for the stream picker.
#[tauri::command]
pub async fn get_channel_streams(
    state: State<'_, AppState>,
    channel_id: i64,
) -> CmdResult<downlink_core::service::StreamListDto> {
    state.svc().await?.channel_streams(channel_id).await
}

/// Stream picker: switch the playing live channel to the stream at `index` (0-based). This moves
/// the server's shared channel session (PLAN §3.4, D18).
#[tauri::command]
pub async fn switch_stream(
    state: State<'_, AppState>,
    channel_id: i64,
    index: usize,
) -> CmdResult<()> {
    state
        .playback()
        .await?
        .switch_stream(channel_id, index)
        .await
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
    if let Some(s) = state.surface().await {
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

fn multiview(pb: &crate::player_glue::Playback) -> CmdResult<Arc<crate::multiview::Multiview>> {
    pb.multiview
        .get()
        .cloned()
        .ok_or_else(|| AppError::local("Multiview is not available."))
}

#[tauri::command]
pub async fn get_multiview(
    state: State<'_, AppState>,
) -> CmdResult<crate::multiview::MultiviewDto> {
    let pb = state.playback().await?;
    Ok(multiview(&pb)?.state(pb.channel_id()).await)
}

/// Put a live channel in a multiview slot (1..). Slot 0 is the main player (`play_channel`).
#[tauri::command]
pub async fn multiview_set_tile(
    state: State<'_, AppState>,
    slot: u32,
    channel_id: i64,
) -> CmdResult<()> {
    let pb = state.playback().await?;
    multiview(&pb)?
        .set_tile(slot, channel_id, pb.channel_id())
        .await
}

#[tauri::command]
pub async fn multiview_remove_tile(state: State<'_, AppState>, slot: u32) -> CmdResult<()> {
    let pb = state.playback().await?;
    multiview(&pb)?.remove_tile(slot, pb.channel_id()).await;
    Ok(())
}

#[tauri::command]
pub async fn multiview_set_audio(state: State<'_, AppState>, slot: u32) -> CmdResult<()> {
    let pb = state.playback().await?;
    multiview(&pb)?.set_audio(slot, pb.channel_id()).await;
    Ok(())
}

/// "Make main": the tile's channel moves to the main player and the main channel to the tile.
#[tauri::command]
pub async fn multiview_make_main(state: State<'_, AppState>, slot: u32) -> CmdResult<()> {
    let pb = state.playback().await?;
    let mv = multiview(&pb)?;
    let Some(tile_ch) = mv.tile_channel(slot).await else {
        return Ok(());
    };
    let main_ch = pb.channel_id();
    pb.play_channel(tile_ch).await?;
    match main_ch {
        Some(c) => mv.set_tile(slot, c, Some(tile_ch)).await,
        None => {
            mv.remove_tile(slot, Some(tile_ch)).await;
            Ok(())
        }
    }
}

/// Tile rect in physical pixels (window client coordinates). Slot 0 uses `set_video_bounds`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn multiview_set_bounds(
    state: State<'_, AppState>,
    slot: u32,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    visible: Option<bool>,
) -> CmdResult<()> {
    let pb = state.playback().await?;
    multiview(&pb)?
        .set_bounds(
            slot,
            SurfaceBounds { x, y, w, h, scale },
            visible.unwrap_or(true),
        )
        .await;
    Ok(())
}

/// Leave multiview: every tile's stream stops (rule 6); the main player keeps playing.
#[tauri::command]
pub async fn multiview_exit(state: State<'_, AppState>) -> CmdResult<()> {
    let pb = state.playback().await?;
    multiview(&pb)?.clear().await;
    Ok(())
}

/// Window geometry saved when entering the mini player, restored on leaving it.
struct SavedWindow {
    position: tauri::PhysicalPosition<i32>,
    size: tauri::PhysicalSize<u32>,
    maximized: bool,
}

static SAVED_WINDOW: std::sync::Mutex<Option<SavedWindow>> = std::sync::Mutex::new(None);

const MINI_W: f64 = 480.0;
const MINI_H: f64 = 270.0;
const MINI_MARGIN: f64 = 24.0;
/// `minWidth` / `minHeight` of the main window in tauri.conf.json.
const MIN_W: f64 = 640.0;
const MIN_H: f64 = 400.0;

/// Mini player (PLAN §8.2, D19): the main window becomes a compact, borderless, always-on-top video
/// window in the bottom-right corner of its monitor; leaving restores size, position and maximised
/// state. One surface, no second window.
#[tauri::command]
pub async fn set_mini_player(app: AppHandle, enabled: bool) -> CmdResult<()> {
    use tauri::{LogicalSize, PhysicalPosition};
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| AppError::local("The window is not available."))?;
    let err = |e: tauri::Error| AppError::local(format!("Could not change the window: {e}"));
    let mut saved = match SAVED_WINDOW.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if enabled {
        if saved.is_some() {
            return Ok(());
        }
        if win.is_fullscreen().unwrap_or(false) {
            win.set_fullscreen(false).map_err(err)?;
        }
        let maximized = win.is_maximized().unwrap_or(false);
        if maximized {
            win.unmaximize().map_err(err)?;
        }
        *saved = Some(SavedWindow {
            position: win.outer_position().map_err(err)?,
            size: win.inner_size().map_err(err)?,
            maximized,
        });
        win.set_min_size(Some(LogicalSize::new(320.0, 180.0)))
            .map_err(err)?;
        win.set_decorations(false).map_err(err)?;
        win.set_size(LogicalSize::new(MINI_W, MINI_H))
            .map_err(err)?;
        win.set_always_on_top(true).map_err(err)?;
        if let Ok(Some(m)) = win.current_monitor() {
            let scale = m.scale_factor();
            let area = m.work_area();
            let (w, h) = (
                (MINI_W * scale).round() as i32,
                (MINI_H * scale).round() as i32,
            );
            let margin = (MINI_MARGIN * scale).round() as i32;
            let x = area.position.x + area.size.width as i32 - w - margin;
            let y = area.position.y + area.size.height as i32 - h - margin;
            win.set_position(PhysicalPosition::new(x, y)).map_err(err)?;
        }
    } else {
        let Some(prev) = saved.take() else {
            return Ok(());
        };
        win.set_always_on_top(false).map_err(err)?;
        win.set_decorations(true).map_err(err)?;
        win.set_min_size(Some(LogicalSize::new(MIN_W, MIN_H)))
            .map_err(err)?;
        win.set_size(prev.size).map_err(err)?;
        win.set_position(prev.position).map_err(err)?;
        if prev.maximized {
            win.maximize().map_err(err)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_stats_enabled(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    if let Some(p) = state.player().await {
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
        .player()
        .await
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
        .playback()
        .await?
        .play_vod(&kind, id, resume.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn play_recording(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.playback().await?.play_recording(id).await
}
