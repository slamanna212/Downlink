//! Windows video surface: mpv renders (`--wid`, `vo=gpu-next`, `gpu-api=d3d11`) into a borderless
//! top-level popup kept **directly behind** the Tauri window in z-order and aligned to the video
//! rect of its client area.
//!
//! Why not a child HWND: a transparent WebView2 only blends with the top-level window's DWM
//! backdrop, not with sibling child windows, so a child placed beneath the webview is never seen
//! (the video rect shows the desktop instead). DWM does composite *top-level* windows with
//! per-pixel alpha, so a window directly behind the transparent main window shows through it.
//!
//! The main window is subclassed so every move/resize/z-order/minimise change re-syncs the
//! popups: same position (client → screen), inserted right below the main window, hidden while
//! the main window is minimised or hidden (tray).

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, ClientToScreen, GetStockObject, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsIconic, IsWindowVisible, MA_NOACTIVATE,
    RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SetWindowPos,
    ShowWindow, WM_MOUSEACTIVATE, WM_MOVE, WM_SHOWWINDOW, WM_SIZE, WM_WINDOWPOSCHANGED, WNDCLASSW,
    WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};

use super::{MainThreadDispatch, SurfaceBounds, VideoSurface};
use crate::engine::Engine;
use crate::error::PlayerError;

const CLASS_NAME: &str = "DownlinkVideo\0";
const SUBCLASS_ID: usize = 0x444C_5644; // "DLVD"

/// One video popup and where it should sit relative to the main window's client area.
#[derive(Clone, Copy)]
struct Entry {
    hwnd: isize,
    bounds: Option<SurfaceBounds>,
    visible: bool,
}

/// All live video popups (main player + multiview tiles). Touched on the main thread only; the
/// mutex just keeps the static safe.
static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
/// The subclassed main window, or 0.
static MAIN: AtomicIsize = AtomicIsize::new(0);
static CLASS_REGISTERED: AtomicBool = AtomicBool::new(false);
static SYNCING: AtomicBool = AtomicBool::new(false);

fn entries() -> std::sync::MutexGuard<'static, Vec<Entry>> {
    ENTRIES.lock().unwrap_or_else(|p| p.into_inner())
}

fn update(hwnd: isize, f: impl FnOnce(&mut Entry)) {
    if let Some(e) = entries().iter_mut().find(|e| e.hwnd == hwnd) {
        f(e);
    }
}

/// Re-position every popup behind the main window. Main thread only.
fn sync() {
    let main = MAIN.load(Ordering::Relaxed);
    if main == 0 || SYNCING.swap(true, Ordering::Relaxed) {
        return;
    }
    let list: Vec<Entry> = entries().clone();
    // SAFETY: `main` is the live top-level HWND we subclassed; every `e.hwnd` is a popup we own.
    unsafe {
        let shown = IsWindowVisible(main as HWND) != 0 && IsIconic(main as HWND) == 0;
        for e in list {
            match e.bounds {
                Some(b) if shown && e.visible && b.w > 0 && b.h > 0 => {
                    let mut pt = POINT { x: b.x, y: b.y };
                    ClientToScreen(main as HWND, &mut pt);
                    // hWndInsertAfter = main: the popup goes immediately below the main window.
                    SetWindowPos(
                        e.hwnd as HWND,
                        main as HWND,
                        pt.x,
                        pt.y,
                        b.w as i32,
                        b.h as i32,
                        SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                    );
                }
                _ => {
                    ShowWindow(e.hwnd as HWND, SW_HIDE);
                }
            }
        }
    }
    SYNCING.store(false, Ordering::Relaxed);
}

unsafe extern "system" fn main_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // SAFETY: forwarding the message we were given to the next handler in the chain.
    let r = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    if matches!(msg, WM_WINDOWPOSCHANGED | WM_MOVE | WM_SIZE | WM_SHOWWINDOW) {
        sync();
    }
    r
}

unsafe extern "system" fn video_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as LRESULT;
    }
    // SAFETY: default handling for our own window.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn class_name() -> Vec<u16> {
    CLASS_NAME.encode_utf16().collect()
}

/// Register the popup class once. Black background so the rect is black, not the desktop,
/// until mpv draws its first frame.
fn register_class() {
    if CLASS_REGISTERED.swap(true, Ordering::Relaxed) {
        return;
    }
    let name = class_name();
    // SAFETY: plain class registration with a NUL-terminated name and a valid window procedure.
    unsafe {
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(video_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: name.as_ptr(),
        };
        // Zero means it already exists (e.g. after a hot restart), which is fine.
        RegisterClassW(&wc);
    }
}

pub struct WindowsSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    /// Video popup HWND, stored as `isize` so the struct stays `Send`.
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

    /// A multiview tile's surface: its own popup, also behind the main window.
    pub fn with_slot(engine: Arc<Engine>, dispatch: MainThreadDispatch, _slot: u32) -> Self {
        Self::new(engine, dispatch)
    }
}

impl VideoSurface for WindowsSurface {
    /// The Tauri window's HWND (`WebviewWindow::hwnd()`), as `isize`.
    type Parent = isize;

    fn attach(&mut self, parent: &isize) -> Result<(), PlayerError> {
        register_class();
        let class = class_name();
        // SAFETY: the class is registered above; the name is NUL-terminated UTF-16. Unowned popup
        // (an owned window would always sit *above* its owner); tool window keeps it out of the
        // taskbar and Alt-Tab, no-activate keeps focus on the main window.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(PlayerError::Surface("CreateWindowExW failed".into()));
        }
        if MAIN.swap(*parent, Ordering::Relaxed) != *parent {
            // SAFETY: `parent` is the live top-level HWND and we are on its thread (main).
            unsafe { SetWindowSubclass(*parent as HWND, Some(main_subclass), SUBCLASS_ID, 0) };
        }
        entries().push(Entry {
            hwnd: hwnd as isize,
            bounds: None,
            visible: true,
        });
        // mpv creates its VO lazily on the first file (`idle=yes`), so setting `wid` now is early enough.
        if let Err(e) = self.engine.set_option("wid", &(hwnd as isize).to_string()) {
            entries().retain(|x| x.hwnd != hwnd as isize);
            // SAFETY: destroying the popup we just created.
            unsafe { DestroyWindow(hwnd) };
            return Err(e);
        }
        self.child = hwnd as isize;
        Ok(())
    }

    fn rebind(&self, engine: Arc<Engine>) {
        // The new mpv instance must render into the same popup.
        if self.child != 0 {
            let _ = engine.set_option("wid", &self.child.to_string());
        }
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        let child = self.child;
        (self.dispatch)(Box::new(move || {
            if child != 0 {
                update(child, |e| e.bounds = Some(b));
                sync();
            }
        }));
    }

    fn set_visible(&self, visible: bool) {
        let child = self.child;
        (self.dispatch)(Box::new(move || {
            if child != 0 {
                update(child, |e| e.visible = visible);
                sync();
            }
        }));
    }

    fn detach(&mut self) {
        if self.child != 0 {
            entries().retain(|e| e.hwnd != self.child);
            // SAFETY: destroying the popup we created, exactly once.
            unsafe { DestroyWindow(self.child as HWND) };
            self.child = 0;
        }
    }
}
