//! VOD library and DVR commands (PLAN M4); thin wrappers over the core `Service`.

use downlink_core::service::{
    MovieDto, RecordKindArg, RecordingRuleDto, RecordingsDto, SeriesDetailDto, SeriesDto,
    VodCategoryDto, VodKindArg, VodListArgs, VodPageDto,
};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

#[tauri::command]
pub async fn list_vod_categories(
    state: State<'_, AppState>,
    kind: VodKindArg,
    with_counts: Option<bool>,
) -> CmdResult<Vec<VodCategoryDto>> {
    state
        .svc()
        .await?
        .vod_categories(kind, with_counts.unwrap_or(true))
        .await
}

#[tauri::command]
pub async fn list_movies(
    state: State<'_, AppState>,
    category_id: Option<i64>,
    query: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
    compact: Option<bool>,
) -> CmdResult<VodPageDto<MovieDto>> {
    state
        .svc()
        .await?
        .movie_page(
            VodListArgs {
                category_id,
                query,
                offset,
                limit,
            },
            compact.unwrap_or(false),
        )
        .await
}

#[tauri::command]
pub async fn get_movie(state: State<'_, AppState>, id: i64) -> CmdResult<MovieDto> {
    state.svc().await?.get_movie(id).await
}

#[tauri::command]
pub async fn list_series(
    state: State<'_, AppState>,
    category_id: Option<i64>,
    query: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
    compact: Option<bool>,
) -> CmdResult<VodPageDto<SeriesDto>> {
    state
        .svc()
        .await?
        .series_page(
            VodListArgs {
                category_id,
                query,
                offset,
                limit,
            },
            compact.unwrap_or(false),
        )
        .await
}

#[tauri::command]
pub async fn get_series(state: State<'_, AppState>, id: i64) -> CmdResult<SeriesDetailDto> {
    state.svc().await?.get_series(id).await
}

#[tauri::command]
pub async fn list_recordings(state: State<'_, AppState>) -> CmdResult<RecordingsDto> {
    state.svc().await?.list_recordings().await
}

#[tauri::command]
pub async fn schedule_recording(
    state: State<'_, AppState>,
    programme_id: String,
    kind: RecordKindArg,
) -> CmdResult<()> {
    state
        .svc()
        .await?
        .schedule_recording(&programme_id, kind)
        .await
}

#[tauri::command]
pub async fn cancel_recording(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.cancel_recording(id).await
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.stop_recording(id).await
}

#[tauri::command]
pub async fn extend_recording(state: State<'_, AppState>, id: i64, minutes: i64) -> CmdResult<()> {
    state.svc().await?.extend_recording(id, minutes).await
}

#[tauri::command]
pub async fn delete_recording(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.delete_recording(id).await
}

#[tauri::command]
pub async fn list_recording_rules(state: State<'_, AppState>) -> CmdResult<Vec<RecordingRuleDto>> {
    state.svc().await?.list_recording_rules().await
}

#[tauri::command]
pub async fn delete_recording_rule(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    state.svc().await?.delete_recording_rule(id).await
}
