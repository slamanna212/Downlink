//! Saved command center commands (docs/COMMAND_CENTER.md §6). Typed DTOs only; validation and
//! storage live in core.

use downlink_core::service::{CommandCenterDto, CommandCenterSaveDto, CommandCenterSummaryDto};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

#[tauri::command]
pub async fn command_centers_list(
    state: State<'_, AppState>,
) -> CmdResult<Vec<CommandCenterSummaryDto>> {
    state.svc().await?.command_centers_list().await
}

#[tauri::command]
pub async fn command_center_get(
    state: State<'_, AppState>,
    id: i64,
) -> CmdResult<CommandCenterDto> {
    state.svc().await?.command_center_get(id).await
}

/// Create (`center.id` null) or replace a center; returns its id.
#[tauri::command]
pub async fn command_center_save(
    state: State<'_, AppState>,
    center: CommandCenterSaveDto,
) -> CmdResult<i64> {
    state.svc().await?.command_center_save(center).await
}

#[tauri::command]
pub async fn command_center_delete(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.command_center_delete(id).await
}
