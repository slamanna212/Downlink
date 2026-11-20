//! Windows video surface: mpv's own `--wid` child HWND of the Tauri window, z-ordered **beneath**
//! the WebView2 child; `vo=gpu-next`, `gpu-api=d3d11` (PLAN §2.2).
//!
//! UNVERIFIED: written without access to a Windows machine. Needs M0 testing on real hardware
//! (z-order versus the WebView2 host window, DPI handling, taskbar auto-hide, fullscreen).

use std::sync::Arc;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_BOTTOM, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SetWindowPos,
    ShowWindow, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

use super::{MainThreadDispatch, SurfaceBounds, VideoSurface};
use crate::engine::Engine;
use crate::error::PlayerError;

pub struct WindowsSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    /// Child HWND, stored as `isize` so the struct stays `Send`.
    child: isize,
}

impl WindowsSurface {
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self {
            engine,
            dispatch,
            child: 0,
        }
    }

    /// A multiview tile's surface: its own child HWND, also beneath the webview.
    pub fn with_slot(engine: Arc<Engine>, dispatch: MainThreadDispatch, _slot: u32) -> Self {
        Self::new(engine, dispatch)
    }
}

impl VideoSurface for WindowsSurface {
    /// The Tauri window's HWND (`WebviewWindow::hwnd()`), as `isize`.
    type Parent = isize;

    fn attach(&mut self, parent: &isize) -> Result<(), PlayerError> {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: `parent` is the live top-level HWND; the class name is NUL-terminated UTF-16.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                *parent as HWND,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(PlayerError::Surface("CreateWindowExW failed".into()));
        }
        // SAFETY: `hwnd` was just created; HWND_BOTTOM orders it beneath its siblings (WebView2).
        unsafe { SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 1, 1, SWP_NOACTIVATE) };
        // mpv creates its VO lazily on the first file (`idle=yes`), so setting `wid` now is early enough.
        self.engine
            .set_option("wid", &(hwnd as isize).to_string())?;
        self.child = hwnd as isize;
        Ok(())
    }

    fn rebind(&self, engine: Arc<Engine>) {
        // The new mpv instance must render into the same child window.
        if self.child != 0 {
            let _ = engine.set_option("wid", &self.child.to_string());
        }
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        let child = self.child;
        (self.dispatch)(Box::new(move || {
            if child != 0 {
                // SAFETY: `child` is our live child HWND; bounds are physical pixels, which is
                // what child windows use in a per-monitor-DPI-aware process.
                unsafe {
                    SetWindowPos(
                        child as HWND,
                        HWND_BOTTOM,
                        b.x,
                        b.y,
                        b.w as i32,
                        b.h as i32,
                        SWP_NOACTIVATE,
                    )
                };
            }
        }));
    }

    fn set_visible(&self, visible: bool) {
        let child = self.child;
        (self.dispatch)(Box::new(move || {
            if child != 0 {
                // SAFETY: `child` is our live child HWND.
                unsafe { ShowWindow(child as HWND, if visible { SW_SHOWNA } else { SW_HIDE }) };
            }
        }));
    }

    fn detach(&mut self) {
        if self.child != 0 {
            // SAFETY: destroying the child window we created, exactly once.
            unsafe { DestroyWindow(self.child as HWND) };
            self.child = 0;
        }
    }
}
