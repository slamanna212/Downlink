//! Tauri-managed application state.

use std::sync::Arc;

use downlink_core::service::Service;
use downlink_player::Player;

use crate::logging::LogControl;
use crate::player_glue::Playback;
use crate::surface::Surface;
use crate::updates::UpdaterState;

pub struct AppState {
    pub service: Arc<Service>,
    pub player: Option<Arc<Player>>,
    pub playback: Option<Arc<Playback>>,
    pub surface: Option<Arc<Surface>>,
    pub log: LogControl,
    pub updater: Arc<UpdaterState>,
    /// Why the engine is unavailable, if it is.
    pub engine_error: Option<String>,
}

impl AppState {
    pub fn playback(&self) -> Result<Arc<Playback>, downlink_core::AppError> {
        self.playback.clone().ok_or_else(|| {
            downlink_core::AppError::new(
                downlink_core::ErrorCode::Media,
                self.engine_error
                    .as_deref()
                    .unwrap_or("The playback engine is not available."),
            )
        })
    }
}
