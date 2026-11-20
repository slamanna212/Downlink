//! Keep the screen and the machine awake while a stream is playing. Linux: the session-bus
//! `org.freedesktop.ScreenSaver` inhibitor, falling back to GNOME's `SessionManager`. Windows:
//! `SetThreadExecutionState` on a dedicated thread. macOS: stub (IOPMAssertion not wired yet).
//! The Windows and macOS paths are UNVERIFIED.

use std::sync::Mutex;

pub struct PlaybackInhibitor {
    inner: Mutex<imp::State>,
}

impl Default for PlaybackInhibitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackInhibitor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(imp::State::default()),
        }
    }

    /// Idempotent: `true` while playing, `false` when paused/stopped. Failures are logged at debug.
    pub fn set_active(&self, active: bool) {
        if let Ok(mut s) = self.inner.lock() {
            imp::set(&mut s, active);
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use zbus::blocking::Connection;

    #[derive(Default)]
    pub struct State {
        conn: Option<Connection>,
        /// (destination, path, interface, cookie)
        cookie: Option<(&'static str, &'static str, &'static str, u32)>,
    }

    const TARGETS: [(&str, &str, &str); 2] = [
        (
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
        ),
        (
            "org.gnome.SessionManager",
            "/org/gnome/SessionManager",
            "org.gnome.SessionManager",
        ),
    ];

    pub fn set(s: &mut State, active: bool) {
        if active == s.cookie.is_some() {
            return;
        }
        if s.conn.is_none() {
            s.conn = Connection::session().ok();
        }
        let Some(conn) = s.conn.as_ref() else { return };
        if active {
            for (i, (dest, path, iface)) in TARGETS.iter().enumerate() {
                let reply = if i == 0 {
                    conn.call_method(
                        Some(*dest),
                        *path,
                        Some(*iface),
                        "Inhibit",
                        &("Downlink", "Playing video"),
                    )
                } else {
                    // app_id, toplevel xid, reason, flags (8 = inhibit idle)
                    conn.call_method(
                        Some(*dest),
                        *path,
                        Some(*iface),
                        "Inhibit",
                        &("downlink", 0u32, "Playing video", 8u32),
                    )
                };
                if let Ok(m) = reply
                    && let Ok(cookie) = m.body().deserialize::<u32>()
                {
                    s.cookie = Some((dest, path, iface, cookie));
                    return;
                }
            }
            tracing::debug!("no screensaver inhibitor available");
        } else if let Some((dest, path, iface, cookie)) = s.cookie.take() {
            let _ = conn.call_method(Some(dest), path, Some(iface), "UnInhibit", &(cookie,));
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::mpsc::{Sender, channel};

    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    #[derive(Default)]
    pub struct State {
        tx: Option<Sender<bool>>,
        active: bool,
    }

    pub fn set(s: &mut State, active: bool) {
        if s.active == active {
            return;
        }
        s.active = active;
        // ES_CONTINUOUS is per thread, so a dedicated thread owns the state.
        let tx = s.tx.get_or_insert_with(|| {
            let (tx, rx) = channel::<bool>();
            std::thread::spawn(move || {
                while let Ok(on) = rx.recv() {
                    let flags = if on {
                        ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED
                    } else {
                        ES_CONTINUOUS
                    };
                    // SAFETY: plain Win32 call with documented flag combinations.
                    unsafe { SetThreadExecutionState(flags) };
                }
            });
            tx
        });
        let _ = tx.send(active);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    #[derive(Default)]
    pub struct State;

    /// UNIMPLEMENTED on macOS: `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleDisplaySleep)`.
    pub fn set(_s: &mut State, _active: bool) {}
}
