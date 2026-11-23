//! Native video surfaces ordered **under** the transparent webview (PLAN §2.2).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::PlayerError;
use crate::options::HdrOutput;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "linux")]
mod wayland_hdr;
#[cfg(target_os = "windows")]
pub mod windows;

/// How many video surfaces (main player + multiview tiles) can be shown at once (PLAN §4.11).
/// The same on every platform.
pub const MAX_SURFACES: u32 = 9;

/// Video rectangle in **physical pixels**, relative to the window's client area, plus the
/// window's scale factor (as reported by the UI's ResizeObserver).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceBounds {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

/// Runs a closure on the UI/main thread (Tauri's `run_on_main_thread`). Platform widgets are
/// not `Send`, so surfaces are driven through this.
pub type MainThreadDispatch = Arc<dyn Fn(Box<dyn FnOnce() + Send + 'static>) + Send + Sync>;

/// A native surface that mpv renders into. All methods may be called from any thread; the
/// implementation hops to the main thread itself (except `attach`, which must run on it).
pub trait VideoSurface {
    /// The platform window/container the surface is inserted into.
    type Parent;

    /// Create the surface under the webview. Must be called on the main thread.
    fn attach(&mut self, parent: &Self::Parent) -> Result<(), PlayerError>;
    /// Re-bind to a replacement mpv engine after the supervisor restarted a dead one. Default:
    /// not supported (the surface keeps rendering nothing until the app restarts).
    fn rebind(&self, _engine: Arc<crate::engine::Engine>) {}
    fn set_bounds(&self, bounds: SurfaceBounds);
    fn set_visible(&self, visible: bool);
    /// Tear down the surface and free the render context.
    fn detach(&mut self);
}

/// The user's HDR output setting, for surfaces that pick their rendering path when attached (Linux).
/// Updated by [`crate::Player::new`] and [`crate::Player::set_preferences`]; read at attach time, so a
/// change applies the next time a video surface is created.
static HDR_AUTO: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_hdr_output(h: HdrOutput) {
    HDR_AUTO.store(h == HdrOutput::Auto, Ordering::Relaxed);
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn hdr_output_auto() -> bool {
    HDR_AUTO.load(Ordering::Relaxed)
}
