//! Sleep / resume notifications. Linux: logind `PrepareForSleep` over the system bus, with a
//! delay inhibitor so the callback finishes (pause + release the stream) before suspend.
//! Windows and macOS are stubs for now (WM_POWERBROADCAST / NSWorkspace notifications later).

use std::sync::Arc;

use crate::PlatformError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The machine is about to sleep (or the lid closed): pause and release the stream.
    Sleep,
    /// The machine woke up.
    Resume,
}

pub type PowerCallback = Arc<dyn Fn(PowerEvent) + Send + Sync>;

/// Keeps the watcher alive; dropping it does not stop the background thread (it exits with the
/// process), but the handle documents ownership for the future per-platform implementations.
pub struct PowerWatcher {
    _private: (),
}

/// Start watching for sleep/resume. Fails soft: the app keeps working without it.
pub fn watch_power(cb: PowerCallback) -> Result<PowerWatcher, PlatformError> {
    imp::watch(cb)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::thread;

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedFd;

    use super::{PowerCallback, PowerEvent, PowerWatcher};
    use crate::PlatformError;

    fn unavailable(e: impl std::fmt::Display) -> PlatformError {
        PlatformError::Unavailable(e.to_string())
    }

    fn inhibit(proxy: &Proxy<'_>) -> Option<OwnedFd> {
        proxy
            .call::<_, _, OwnedFd>(
                "Inhibit",
                &(
                    "sleep",
                    "Downlink",
                    "Releasing the stream before sleep",
                    "delay",
                ),
            )
            .ok()
    }

    pub fn watch(cb: PowerCallback) -> Result<PowerWatcher, PlatformError> {
        let conn = Connection::system().map_err(unavailable)?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(unavailable)?;
        let signals = proxy
            .receive_signal("PrepareForSleep")
            .map_err(unavailable)?;
        thread::Builder::new()
            .name("downlink-power".into())
            .spawn(move || {
                let mut lock = inhibit(&proxy);
                for msg in signals {
                    let Ok(going_to_sleep) = msg.body().deserialize::<bool>() else {
                        continue;
                    };
                    if going_to_sleep {
                        cb(PowerEvent::Sleep);
                        // Dropping the inhibitor fd lets logind proceed with suspend.
                        drop(lock.take());
                    } else {
                        lock = inhibit(&proxy);
                        cb(PowerEvent::Resume);
                    }
                }
            })
            .map_err(unavailable)?;
        Ok(PowerWatcher { _private: () })
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{PowerCallback, PowerWatcher};
    use crate::PlatformError;

    /// UNIMPLEMENTED stub: Windows (`WM_POWERBROADCAST`) and macOS (`NSWorkspaceWillSleepNotification`).
    pub fn watch(_cb: PowerCallback) -> Result<PowerWatcher, PlatformError> {
        Err(PlatformError::Unavailable(
            "power events not implemented on this platform yet".into(),
        ))
    }
}
