//! Media server (Plex) commands: thin wrappers over the core `Service`, plus opening pages in the
//! user's browser.

use downlink_core::AppError;
use downlink_core::service::{
    MediaMatchDto, MediaServerStatusDto, MediaTargetArg, PlexPinDto, PlexServerDto,
};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use super::CmdResult;
use crate::state::AppState;

/// Pages Downlink opens for media servers. Anything else is refused.
fn allowed_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https" && u.host_str() == Some("app.plex.tv") && u.port().is_none()
    })
}

fn open(app: &AppHandle, url: &str) -> CmdResult<()> {
    if !allowed_url(url) {
        return Err(AppError::local("That page cannot be opened."));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::local(format!("Could not open the browser: {e}")))
}

#[tauri::command]
pub async fn media_server_status(state: State<'_, AppState>) -> CmdResult<MediaServerStatusDto> {
    state.svc().await?.media_server_status().await
}

/// Start Plex sign-in and open the approval page in the browser.
#[tauri::command]
pub async fn plex_sign_in_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<PlexPinDto> {
    let pin = state.svc().await?.plex_sign_in_start().await?;
    open(&app, &pin.auth_url)?;
    Ok(pin)
}

#[tauri::command]
pub async fn plex_sign_in_check(state: State<'_, AppState>, pin_id: i64) -> CmdResult<bool> {
    state.svc().await?.plex_sign_in_check(pin_id).await
}

#[tauri::command]
pub async fn plex_servers(state: State<'_, AppState>) -> CmdResult<Vec<PlexServerDto>> {
    state.svc().await?.plex_servers().await
}

#[tauri::command]
pub async fn plex_select_server(
    state: State<'_, AppState>,
    server_id: String,
) -> CmdResult<MediaServerStatusDto> {
    state.svc().await?.plex_select_server(&server_id).await
}

#[tauri::command]
pub async fn media_server_sign_out(state: State<'_, AppState>) -> CmdResult<()> {
    state.svc().await?.media_server_sign_out().await
}

#[tauri::command]
pub async fn media_server_match(
    state: State<'_, AppState>,
    target: MediaTargetArg,
) -> CmdResult<Option<MediaMatchDto>> {
    state.svc().await?.media_server_match(target).await
}

/// Open a media-server page (an `app.plex.tv` URL from `media_server_match`) in the browser.
#[tauri::command]
pub async fn media_server_open(app: AppHandle, url: String) -> CmdResult<()> {
    open(&app, &url)
}

#[cfg(test)]
mod tests {
    use super::allowed_url;

    #[test]
    fn only_plex_web_pages_open() {
        assert!(allowed_url(
            "https://app.plex.tv/desktop/#!/server/m1/details?key=%2Flibrary%2Fmetadata%2F1"
        ));
        assert!(allowed_url("https://app.plex.tv/auth#?clientID=x&code=y"));
        assert!(!allowed_url("http://app.plex.tv/desktop"));
        assert!(!allowed_url("https://app.plex.tv.evil.test/desktop"));
        assert!(!allowed_url("https://app.plex.tv:8443/desktop"));
        assert!(!allowed_url("file:///etc/passwd"));
        assert!(!allowed_url("not a url"));
    }
}
