//! OS media controls and media keys: MPRIS on Linux, SMTC on Windows, MPNowPlayingInfoCenter on
//! macOS, through `souvlaki`. The Windows and macOS paths are UNVERIFIED (no machines here).

use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
};

use crate::PlatformError;

/// Transport requests coming from media keys, the shell's media widget or the lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Raise / show the main window.
    Raise,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    Playing,
    Paused,
    Stopped,
}

pub type MediaHandler = Arc<dyn Fn(MediaCommand) + Send + Sync>;

/// Map a souvlaki event to our command, ignoring the ones we do not support (seeking is handled
/// by the UI, not by the OS widget, for live TV).
pub fn map_event(e: &MediaControlEvent) -> Option<MediaCommand> {
    Some(match e {
        MediaControlEvent::Play => MediaCommand::Play,
        MediaControlEvent::Pause => MediaCommand::Pause,
        MediaControlEvent::Toggle => MediaCommand::Toggle,
        MediaControlEvent::Stop => MediaCommand::Stop,
        MediaControlEvent::Next => MediaCommand::Next,
        MediaControlEvent::Previous => MediaCommand::Previous,
        MediaControlEvent::Raise => MediaCommand::Raise,
        MediaControlEvent::Quit => MediaCommand::Quit,
        _ => return None,
    })
}

pub struct MediaSession {
    controls: Mutex<MediaControls>,
}

impl MediaSession {
    /// `hwnd` is required on Windows (the main window's HWND) and ignored elsewhere.
    pub fn new(hwnd: Option<*mut c_void>, handler: MediaHandler) -> Result<Self, PlatformError> {
        let mut controls = MediaControls::new(PlatformConfig {
            dbus_name: "downlink",
            display_name: "Downlink",
            hwnd,
        })
        .map_err(|e| PlatformError::Unavailable(format!("{e:?}")))?;
        controls
            .attach(move |e: MediaControlEvent| {
                if let Some(c) = map_event(&e) {
                    handler(c);
                }
            })
            .map_err(|e| PlatformError::Unavailable(format!("{e:?}")))?;
        Ok(Self {
            controls: Mutex::new(controls),
        })
    }

    pub fn set_now_playing(&self, title: &str, subtitle: Option<&str>, duration: Option<Duration>) {
        if let Ok(mut c) = self.controls.lock() {
            let _ = c.set_metadata(MediaMetadata {
                title: Some(title),
                artist: subtitle,
                album: None,
                cover_url: None,
                duration,
            });
        }
    }

    pub fn set_state(&self, state: MediaState, position: Option<Duration>) {
        let progress = position.map(MediaPosition);
        let pb = match state {
            MediaState::Playing => MediaPlayback::Playing { progress },
            MediaState::Paused => MediaPlayback::Paused { progress },
            MediaState::Stopped => MediaPlayback::Stopped,
        };
        if let Ok(mut c) = self.controls.lock() {
            let _ = c.set_playback(pb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_transport_events() {
        assert_eq!(
            map_event(&MediaControlEvent::Toggle),
            Some(MediaCommand::Toggle)
        );
        assert_eq!(
            map_event(&MediaControlEvent::Raise),
            Some(MediaCommand::Raise)
        );
        assert_eq!(map_event(&MediaControlEvent::SetVolume(0.5)), None);
    }
}
