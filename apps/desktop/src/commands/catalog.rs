use downlink_core::AppError;
use downlink_core::service::{
    AddSourceInput, ChannelDto, CountsDto, GroupDto, GuideArgs, GuideDto, HomeDto,
    ListChannelsArgs, NowNextDto, ProgrammeDto, SearchDto, SourceDto, TestSourceResult,
};
use tauri::State;

use super::CmdResult;
use crate::state::AppState;

#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> CmdResult<Vec<SourceDto>> {
    state.svc().await?.list_sources().await
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
        .svc()
        .await?
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
        .svc()
        .await?
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
    state.svc().await?.remove_source(source_id).await
}

#[tauri::command]
pub async fn sync_source(state: State<'_, AppState>, source_id: i64) -> CmdResult<()> {
    state.svc().await?.sync_source(source_id).await
}

#[tauri::command]
pub async fn list_groups(
    state: State<'_, AppState>,
    source_id: Option<i64>,
) -> CmdResult<Vec<GroupDto>> {
    state.svc().await?.list_groups(source_id).await
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
        .svc()
        .await?
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
    state.svc().await?.get_now_next(&channel_ids).await
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
        .svc()
        .await?
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
    state
        .svc()
        .await?
        .get_programmes(channel_id, start, end)
        .await
}

#[tauri::command]
pub async fn get_home(state: State<'_, AppState>) -> CmdResult<HomeDto> {
    state.svc().await?.get_home().await
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: i64) -> CmdResult<SearchDto> {
    state.svc().await?.search(&query, limit).await
}

#[tauri::command]
pub async fn set_favourite(
    state: State<'_, AppState>,
    channel_id: i64,
    favourite: bool,
) -> CmdResult<()> {
    state
        .svc()
        .await?
        .set_favourite(channel_id, favourite)
        .await
}

#[tauri::command]
pub async fn get_counts(state: State<'_, AppState>) -> CmdResult<CountsDto> {
    state.svc().await?.get_counts().await
}

/// Channel DTO by Dispatcharr channel uuid (deep links and the UI use uuids). Core has no uuid
/// lookup, so this walks the sources' `uuid -> id` maps in the store.
#[tauri::command]
pub async fn get_channel_by_uuid(
    state: State<'_, AppState>,
    uuid: String,
) -> CmdResult<ChannelDto> {
    let uuid = uuid.to_ascii_lowercase();
    let svc = state.svc().await?;
    for s in svc.list_sources().await? {
        let map = svc.store().channels_by_uuid(s.id).await?;
        if let Some((_, id)) = map.iter().find(|(u, _)| u.to_ascii_lowercase() == uuid) {
            return svc.get_channel(*id).await;
        }
    }
    Err(AppError::local("That channel is not in your list."))
}
