use downlink_core::AppError;
use downlink_core::epg;
use downlink_core::service::{
    AddSourceInput, ChannelDto, GroupDto, GuideArgs, GuideDto, ListChannelsArgs, NowNextDto,
    ProgrammeDto, SourceDto, TestSourceResult,
};
use downlink_core::source::TimeWindow;
use serde::Serialize;
use serde_json::Value;
use tauri::State;

use super::CmdResult;
use crate::dto;
use crate::state::AppState;

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> CmdResult<Vec<SourceDto>> {
    state.service.list_sources().await
}

#[allow(clippy::too_many_arguments)]
fn input(
    kind: String,
    name: String,
    url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    xmltv_url: Option<String>,
    allow_self_signed: Option<bool>,
) -> AddSourceInput {
    AddSourceInput {
        kind,
        name,
        url,
        api_key,
        username,
        password,
        xmltv_url,
        allow_self_signed: allow_self_signed.unwrap_or(false),
    }
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn add_source(
    state: State<'_, AppState>,
    kind: String,
    name: String,
    url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    xmltv_url: Option<String>,
    allow_self_signed: Option<bool>,
) -> CmdResult<SourceDto> {
    state
        .service
        .add_source(input(
            kind,
            name,
            url,
            api_key,
            username,
            password,
            xmltv_url,
            allow_self_signed,
        ))
        .await
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn test_source(
    state: State<'_, AppState>,
    kind: String,
    name: String,
    url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    xmltv_url: Option<String>,
    allow_self_signed: Option<bool>,
) -> CmdResult<TestSourceResult> {
    state
        .service
        .test_source(input(
            kind,
            name,
            url,
            api_key,
            username,
            password,
            xmltv_url,
            allow_self_signed,
        ))
        .await
}

#[tauri::command]
pub async fn remove_source(state: State<'_, AppState>, source_id: i64) -> CmdResult<()> {
    state.service.remove_source(source_id).await
}

#[tauri::command]
pub async fn sync_source(state: State<'_, AppState>, source_id: i64) -> CmdResult<()> {
    state.service.sync_source(source_id).await
}

#[tauri::command]
pub async fn list_groups(
    state: State<'_, AppState>,
    source_id: Option<i64>,
) -> CmdResult<Vec<GroupDto>> {
    state.service.list_groups(source_id).await
}

#[tauri::command]
pub async fn list_channels(
    state: State<'_, AppState>,
    source_id: Option<i64>,
    group_id: Option<i64>,
    query: Option<String>,
    favourites_only: Option<bool>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> CmdResult<Vec<ChannelDto>> {
    state
        .service
        .list_channels(ListChannelsArgs {
            source_id,
            group_id,
            query,
            favourites_only: favourites_only.unwrap_or(false),
            offset,
            limit,
        })
        .await
}

#[tauri::command]
pub async fn get_now_next(
    state: State<'_, AppState>,
    channel_ids: Vec<i64>,
) -> CmdResult<Vec<NowNextDto>> {
    state.service.get_now_next(&channel_ids).await
}

#[tauri::command]
pub async fn get_guide(
    state: State<'_, AppState>,
    start: i64,
    end: i64,
    channel_offset: Option<i64>,
    channel_limit: Option<i64>,
    group_id: Option<i64>,
) -> CmdResult<GuideDto> {
    state
        .service
        .get_guide(GuideArgs {
            start,
            end,
            channel_offset: channel_offset.unwrap_or(0),
            channel_limit,
            group_id,
        })
        .await
}

#[tauri::command]
pub async fn get_programmes(
    state: State<'_, AppState>,
    channel_id: i64,
    start: i64,
    end: i64,
) -> CmdResult<Vec<ProgrammeDto>> {
    let rows = epg::window(
        state.service.store(),
        &[channel_id],
        TimeWindow {
            start_utc: start,
            end_utc: end,
        },
    )
    .await?;
    Ok(rows.iter().map(ProgrammeDto::from).collect())
}

#[tauri::command]
pub async fn get_home(state: State<'_, AppState>) -> CmdResult<Value> {
    let h = state.service.get_home().await?;
    Ok(dto::home_to_ui(&state.service, h).await)
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: i64) -> CmdResult<Value> {
    let s = state.service.search(&query, limit).await?;
    Ok(dto::search_to_ui(&state.service, s).await)
}

#[tauri::command]
pub async fn set_favourite(
    state: State<'_, AppState>,
    channel_id: i64,
    favourite: bool,
) -> CmdResult<()> {
    state.service.set_favourite(channel_id, favourite).await
}

#[derive(Serialize)]
pub struct Counts {
    all: i64,
    favourites: i64,
}

#[tauri::command]
pub async fn get_counts(state: State<'_, AppState>) -> CmdResult<Counts> {
    let favs = state
        .service
        .list_channels(ListChannelsArgs {
            favourites_only: true,
            ..Default::default()
        })
        .await?;
    Ok(Counts {
        all: dto::all_count(&state.service).await,
        favourites: favs.len() as i64,
    })
}

#[tauri::command]
pub async fn schedule_recording(_programme_id: String, _kind: String) -> CmdResult<()> {
    Err(AppError::local("Recording is not available yet."))
}
