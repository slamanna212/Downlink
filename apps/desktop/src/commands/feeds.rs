//! News feed commands (docs/COMMAND_CENTER.md §6.2). Fetching, parsing and polling live in core;
//! the UI never talks HTTP (rule 2). Item links open through `open_external_link`.

use downlink_core::service::{FeedDto, FeedItemDto};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

#[tauri::command]
pub async fn feeds_list(state: State<'_, AppState>) -> CmdResult<Vec<FeedDto>> {
    state.svc().await?.feeds_list().await
}

/// Validate, fetch once and save; a failed first fetch saves nothing.
#[tauri::command]
pub async fn feed_add(state: State<'_, AppState>, url: String) -> CmdResult<FeedDto> {
    state.svc().await?.feed_add(&url).await
}

#[tauri::command]
pub async fn feed_remove(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.feed_remove(id).await
}

/// Newest first across `feedIds`, at most `limit` (1..=500).
#[tauri::command]
pub async fn feed_items(
    state: State<'_, AppState>,
    feed_ids: Vec<i64>,
    limit: i64,
) -> CmdResult<Vec<FeedItemDto>> {
    state.svc().await?.feed_items(&feed_ids, limit).await
}

#[tauri::command]
pub async fn feed_refresh(state: State<'_, AppState>, id: i64) -> CmdResult<FeedDto> {
    state.svc().await?.feed_refresh(id).await
}

/// Replace what view `key` watches (polled while watched). An empty list stops watching.
#[tauri::command]
pub async fn feeds_watch(
    state: State<'_, AppState>,
    key: String,
    feed_ids: Vec<i64>,
) -> CmdResult<()> {
    state.svc().await?.feeds_watch(&key, &feed_ids).await
}

#[tauri::command]
pub async fn feeds_unwatch(state: State<'_, AppState>, key: String) -> CmdResult<()> {
    state.svc().await?.feeds_unwatch(&key);
    Ok(())
}
