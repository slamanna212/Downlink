//! Native video surfaces ordered **under** the transparent webview (PLAN §2.2).

use std::sync::Arc;

use crate::error::PlayerError;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

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
