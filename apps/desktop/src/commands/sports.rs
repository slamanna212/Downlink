//! Sports tracking commands (docs/SPORTS.md §4). Typed DTOs only; matching runs in core.

use downlink_core::service::{SportsCatalogDto, SportsFollowDto, SportsMatchDto};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

#[tauri::command]
pub async fn sports_catalog(state: State<'_, AppState>) -> CmdResult<SportsCatalogDto> {
    state.svc().await?.sports_catalog()
}

#[tauri::command]
pub async fn sports_follow_get(state: State<'_, AppState>) -> CmdResult<SportsFollowDto> {
    state.svc().await?.sports_follow_get().await
}

/// Save, re-match, then `catalog://updated` (scope `sports`).
#[tauri::command]
pub async fn sports_follow_set(
    state: State<'_, AppState>,
    follow: SportsFollowDto,
) -> CmdResult<()> {
    state.svc().await?.sports_follow_set(follow).await
}

#[tauri::command]
pub async fn sports_matches(
    state: State<'_, AppState>,
    start_utc: i64,
    end_utc: i64,
) -> CmdResult<Vec<SportsMatchDto>> {
    state.svc().await?.sports_matches(start_utc, end_utc).await
}
