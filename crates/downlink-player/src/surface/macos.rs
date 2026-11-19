//! macOS video surface: a layer-backed `NSView` inserted **below** the `WKWebView`
//! (`addSubview:positioned:NSWindowBelow`), mpv rendering through the render API (PLAN §2.2).
//! mpv's macOS backend has no window-embedding path, so `--wid` is not used.
//!
//! UNVERIFIED: written without a Mac. Needs M0 testing by macOS testers (Space switches,
//! fullscreen, Retina scale, GL deprecation path versus a Metal layer).
//!
//! Sketch of the intended implementation, to be completed on a Mac:
//! 1. `attach`: take the window's content `NSView`, create an `NSOpenGLView` (layer-backed,
//!    `wantsBestResolutionOpenGLSurface = YES`), insert it below the webview.
//! 2. Make its `NSOpenGLContext` current, create the mpv render context with
//!    `get_proc_address` backed by `dlsym` on the OpenGL framework.
//! 3. Render from the mpv update callback on the main thread: `makeCurrentContext`,
//!    `mpv_render_context_render` to FBO 0, `flushBuffer`.
//! 4. `set_bounds`: `setFrame:` converting physical pixels to points using `scale`.

use std::sync::Arc;

use super::{MainThreadDispatch, SurfaceBounds, VideoSurface};
use crate::engine::Engine;
use crate::error::PlayerError;

pub struct MacSurface {
    _engine: Arc<Engine>,
    _dispatch: MainThreadDispatch,
}

impl MacSurface {
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self {
            _engine: engine,
            _dispatch: dispatch,
        }
    }
}

impl VideoSurface for MacSurface {
    /// Pointer to the window's content `NSView` (`WebviewWindow::ns_view()`).
    type Parent = *mut std::ffi::c_void;

    fn attach(&mut self, _parent: &Self::Parent) -> Result<(), PlayerError> {
        Err(PlayerError::Surface(
            "macOS surface is not implemented yet (unverified skeleton, see module docs)".into(),
        ))
    }

    fn set_bounds(&self, _bounds: SurfaceBounds) {}

    fn set_visible(&self, _visible: bool) {}

    fn detach(&mut self) {}
}
