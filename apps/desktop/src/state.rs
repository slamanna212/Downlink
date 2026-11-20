//! Tauri-managed application state.
//!
//! The window must appear before the heavy start-up work (SQLite open and migrations, libmpv,
//! surface, tray, media session), so the state exists immediately and fills in two stages that
//! commands wait on: [`Core`] (service + logging) and [`Engine`] (player, playback glue, surface).

use std::path::PathBuf;
use std::sync::Arc;

use downlink_core::service::Service;
use downlink_core::{AppError, ErrorCode};
use downlink_player::Player;
use tokio::sync::watch;

use crate::logging::LogControl;
use crate::player_glue::Playback;
use crate::surface::Surface;
use crate::updates::UpdaterState;

/// A value that is set once, later, and that async code can wait for.
pub struct Slot<T> {
    tx: watch::Sender<Option<Arc<T>>>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            tx: watch::channel(None).0,
        }
    }
}

impl<T> Slot<T> {
    pub fn set(&self, v: T) {
        self.tx.send_replace(Some(Arc::new(v)));
    }

    pub fn now(&self) -> Option<Arc<T>> {
        self.tx.borrow().clone()
    }

    pub async fn wait(&self) -> Arc<T> {
        let mut rx = self.tx.subscribe();
        loop {
            if let Some(v) = rx.borrow_and_update().clone() {
                return v;
            }
            if rx.changed().await.is_err() {
                // The sender lives in this slot, so it cannot close while we hold `&self`.
                std::future::pending::<()>().await;
            }
        }
    }
}

/// Stage 1: the service. `service` is the blocking error when the library cannot be opened
/// (for example "created by a newer version"): every catalog command then returns it.
pub struct Core {
    pub service: Result<Arc<Service>, AppError>,
    pub log: LogControl,
}

/// Stage 2: playback.
pub struct Engine {
    pub player: Option<Arc<Player>>,
    pub playback: Option<Arc<Playback>>,
    pub surface: Option<Arc<Surface>>,
    /// Why the engine is unavailable, if it is.
    pub error: Option<String>,
}

pub struct AppState {
    pub core: Slot<Core>,
    pub engine: Slot<Engine>,
    pub updater: Arc<UpdaterState>,
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(updater: Arc<UpdaterState>, data_dir: PathBuf) -> Self {
        Self {
            core: Slot::default(),
            engine: Slot::default(),
            updater,
            data_dir,
        }
    }

    pub async fn svc(&self) -> Result<Arc<Service>, AppError> {
        self.core.wait().await.service.clone()
    }

    pub async fn playback(&self) -> Result<Arc<Playback>, AppError> {
        let core = self.core.wait().await;
        if let Err(e) = &core.service {
            return Err(e.clone());
        }
        let eng = self.engine.wait().await;
        eng.playback.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::Media,
                eng.error
                    .as_deref()
                    .unwrap_or("The playback engine is not available."),
            )
        })
    }

    pub async fn player(&self) -> Option<Arc<Player>> {
        self.engine.wait().await.player.clone()
    }

    pub async fn surface(&self) -> Option<Arc<Surface>> {
        self.engine.wait().await.surface.clone()
    }

    pub async fn log(&self) -> LogControl {
        self.core.wait().await.log.clone()
    }

    /// Non-waiting accessors for sync callers (tray, media keys, quit): `None` until ready.
    pub fn try_playback(&self) -> Option<Arc<Playback>> {
        self.engine.now().and_then(|e| e.playback.clone())
    }

    #[allow(dead_code)]
    pub fn try_service(&self) -> Option<Arc<Service>> {
        self.core.now().and_then(|c| c.service.clone().ok())
    }

    /// For the dev drivers, after they waited for readiness.
    #[cfg(feature = "e2e")]
    #[allow(clippy::expect_used)] // dev driver only, after readiness
    pub fn service_now(&self) -> Arc<Service> {
        self.try_service().expect("service is ready")
    }
}
