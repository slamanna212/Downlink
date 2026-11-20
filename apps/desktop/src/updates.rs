//! Updates with `tauri-plugin-updater`: a check every 6 h in the background, silent download,
//! install on quit. Nothing here ever touches playback. The channel (stable/dev) picks the
//! manifest endpoint at runtime (docs/release.md).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use downlink_core::AppError;
use downlink_core::service::Service;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const STABLE_ENDPOINT: &str =
    "https://github.com/slamanna212/Downlink/releases/latest/download/latest.json";
pub const DEV_ENDPOINT: &str =
    "https://github.com/slamanna212/Downlink/releases/download/dev/latest.json";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub channel: String,
    pub current: String,
    pub available: Option<String>,
    pub last_checked: Option<i64>,
}

#[derive(Default)]
struct Pending {
    update: Option<Update>,
    bytes: Option<Vec<u8>>,
}

pub struct UpdaterState {
    status: Mutex<UpdateStatus>,
    pending: Mutex<Pending>,
}

pub fn endpoint(channel: &str) -> &'static str {
    if channel == "dev" {
        DEV_ENDPOINT
    } else {
        STABLE_ENDPOINT
    }
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl UpdaterState {
    pub fn new(current: &str, saved_channel: Option<&str>) -> Arc<Self> {
        let channel = match saved_channel {
            Some(c @ ("stable" | "dev")) => c.to_owned(),
            _ if current.contains("-dev") => "dev".to_owned(),
            _ => "stable".to_owned(),
        };
        Arc::new(Self {
            status: Mutex::new(UpdateStatus {
                channel,
                current: current.to_owned(),
                available: None,
                last_checked: None,
            }),
            pending: Mutex::new(Pending::default()),
        })
    }

    /// The channel saved in settings, applied once the service is up.
    pub fn apply_saved_channel(&self, channel: &str) {
        if channel == "stable" || channel == "dev" {
            self.update_status(|s| s.channel = channel.to_owned());
        }
    }

    pub fn status(&self) -> UpdateStatus {
        match self.status.lock() {
            Ok(s) => s.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    fn update_status(&self, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
        let mut g = match self.status.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g);
        g.clone()
    }

    pub async fn set_channel(
        &self,
        app: &AppHandle,
        service: &Service,
        channel: &str,
    ) -> Result<UpdateStatus, AppError> {
        if channel != "stable" && channel != "dev" {
            return Err(AppError::local("Unknown update channel."));
        }
        service
            .set_settings(serde_json::json!({ "updateChannel": channel }))
            .await?;
        let st = self.update_status(|s| {
            s.channel = channel.to_owned();
            s.available = None;
        });
        if let Ok(mut p) = self.pending.lock() {
            *p = Pending::default();
        }
        let _ = app.emit("update://status", &st);
        Ok(st)
    }

    /// Check the channel's manifest. Failures (offline, bad signature setup) leave the previous
    /// status and are logged at debug; they never interrupt anything.
    pub async fn check(self: &Arc<Self>, app: &AppHandle) -> UpdateStatus {
        let channel = self.status().channel;
        let found = async {
            let url = endpoint(&channel).parse().map_err(|e| format!("{e}"))?;
            let updater = app
                .updater_builder()
                .endpoints(vec![url])
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            updater.check().await.map_err(|e| e.to_string())
        }
        .await;
        let st = match found {
            Ok(Some(update)) => {
                let version = update.version.clone();
                tracing::info!("update available: {version}");
                if let Ok(mut p) = self.pending.lock() {
                    *p = Pending {
                        update: Some(update.clone()),
                        bytes: None,
                    };
                }
                self.spawn_download(update);
                self.update_status(|s| {
                    s.available = Some(version);
                    s.last_checked = Some(now_s());
                })
            }
            Ok(None) => self.update_status(|s| {
                s.available = None;
                s.last_checked = Some(now_s());
            }),
            Err(e) => {
                tracing::debug!("update check failed: {e}");
                self.update_status(|s| s.last_checked = Some(now_s()))
            }
        };
        let _ = app.emit("update://status", &st);
        st
    }

    fn spawn_download(self: &Arc<Self>, update: Update) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            match update.download(|_, _| {}, || {}).await {
                Ok(bytes) => {
                    if let Ok(mut p) = me.pending.lock() {
                        p.bytes = Some(bytes);
                    }
                    tracing::info!("update downloaded; it installs when the app quits");
                }
                Err(e) => tracing::debug!("update download failed: {e}"),
            }
        });
    }

    /// Called on quit: install a downloaded update.
    pub fn install_on_quit(&self) {
        let taken = match self.pending.lock() {
            Ok(mut p) => (p.update.take(), p.bytes.take()),
            Err(_) => (None, None),
        };
        if let (Some(u), Some(b)) = taken
            && let Err(e) = u.install(b)
        {
            tracing::warn!("update install failed: {e}");
        }
    }

    pub fn spawn_loop(self: &Arc<Self>, app: AppHandle) {
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            loop {
                me.check(&app).await;
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_defaults_follow_version() {
        assert_eq!(UpdaterState::new("0.1.0", None).status().channel, "stable");
        assert_eq!(
            UpdaterState::new("0.1.0-dev.abc1234", None)
                .status()
                .channel,
            "dev"
        );
        assert_eq!(
            UpdaterState::new("0.1.0", Some("dev")).status().channel,
            "dev"
        );
    }

    #[test]
    fn endpoints_differ_by_channel() {
        assert!(endpoint("dev").contains("/download/dev/"));
        assert!(endpoint("stable").contains("/latest/download/"));
    }
}
