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
//!
//! It also tracks whether the monitor the main window is on has HDR turned on
//! ([`super::set_display_hdr`]), so RTX HDR only runs where it can be shown.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BLACK_BRUSH, ClientToScreen, GetMonitorInfoW, GetStockObject, HBRUSH, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MONITORINFOEXW, MonitorFromWindow,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsIconic, IsWindowVisible, MA_NOACTIVATE,
    RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SetWindowPos,
    ShowWindow, WM_DISPLAYCHANGE, WM_MOUSEACTIVATE, WM_MOVE, WM_SETTINGCHANGE, WM_SHOWWINDOW,
    WM_SIZE, WM_WINDOWPOSCHANGED, WNDCLASSW, WS_CLIPCHILDREN, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
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
/// The monitor the main window was last seen on, or 0.
static LAST_MONITOR: AtomicIsize = AtomicIsize::new(0);

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
    refresh_display_hdr(false);
}

/// Re-check whether the main window's monitor has HDR on: only when the window changed monitor,
/// or always with `force` (display settings changed, e.g. HDR toggled). Main thread only.
fn refresh_display_hdr(force: bool) {
    let main = MAIN.load(Ordering::Relaxed);
    if main == 0 {
        return;
    }
    // SAFETY: `main` is the live top-level HWND we subclassed.
    let mon = unsafe { MonitorFromWindow(main as HWND, MONITOR_DEFAULTTONEAREST) };
    if mon.is_null() {
        return;
    }
    if LAST_MONITOR.swap(mon as isize, Ordering::Relaxed) == mon as isize && !force {
        return;
    }
    let hdr = monitor_hdr(mon);
    if super::set_display_hdr(hdr) {
        tracing::info!(hdr, "video monitor HDR state");
    }
}

fn wide(buf: &[u16]) -> &[u16] {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    &buf[..end]
}

/// Whether Windows HDR is on for `mon`. Advanced colour on an SDR panel (Windows 11 Auto Color
/// Management) reports `wideColorEnforced` and is not HDR. Any failure counts as SDR.
fn monitor_hdr(mon: HMONITOR) -> bool {
    // SAFETY: MONITORINFOEXW is plain data; `cbSize` tells the API which variant it got.
    let mut mi: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: `mon` came from MonitorFromWindow; `mi` is a valid, sized MONITORINFOEXW.
    if unsafe { GetMonitorInfoW(mon, (&raw mut mi).cast::<MONITORINFO>()) } == 0 {
        return false;
    }
    let gdi_name = wide(&mi.szDevice);

    let (mut n_paths, mut n_modes) = (0u32, 0u32);
    // SAFETY: valid out-pointers for the two counts.
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut n_paths, &mut n_modes) }
        != 0
    {
        return false;
    }
    // SAFETY: both are plain data; zeroed is valid before QueryDisplayConfig fills them.
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> =
        vec![unsafe { std::mem::zeroed() }; n_paths as usize];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> =
        vec![unsafe { std::mem::zeroed() }; n_modes as usize];
    // SAFETY: the buffers hold exactly the counts passed; the topology id is optional here.
    let r = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut n_paths,
            paths.as_mut_ptr(),
            &mut n_modes,
            modes.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    if r != 0 {
        return false;
    }
    paths.truncate(n_paths as usize);

    for p in &paths {
        // SAFETY: plain data; the header says which request this is and how big the buffer is.
        let mut src: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { std::mem::zeroed() };
        src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        src.header.adapterId = p.sourceInfo.adapterId;
        src.header.id = p.sourceInfo.id;
        // SAFETY: `src` is a correctly sized and typed request packet.
        if unsafe { DisplayConfigGetDeviceInfo(&mut src.header) } != 0
            || wide(&src.viewGdiDeviceName) != gdi_name
        {
            continue;
        }
        // SAFETY: as above.
        let mut color: DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO = unsafe { std::mem::zeroed() };
        color.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
        color.header.size = std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32;
        color.header.adapterId = p.targetInfo.adapterId;
        color.header.id = p.targetInfo.id;
        // SAFETY: `color` is a correctly sized and typed request packet.
        if unsafe { DisplayConfigGetDeviceInfo(&mut color.header) } != 0 {
            return false;
        }
        // SAFETY: the union is a bitfield over one u32; reading `value` is always valid.
        let bits = unsafe { color.Anonymous.value };
        return advanced_color_is_hdr(bits);
    }
    false
}

/// `DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO` bits: 0 supported, 1 enabled, 2 wide colour enforced
/// (Auto Color Management on SDR), 3 force disabled.
fn advanced_color_is_hdr(bits: u32) -> bool {
    let supported = bits & 0b0001 != 0;
    let enabled = bits & 0b0010 != 0;
    let wide_enforced = bits & 0b0100 != 0;
    supported && enabled && !wide_enforced
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
    // Resolution, monitor layout or HDR toggled in Windows settings.
    if matches!(msg, WM_DISPLAYCHANGE | WM_SETTINGCHANGE) {
        refresh_display_hdr(true);
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
        refresh_display_hdr(true);
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

#[cfg(test)]
mod tests {
    use super::advanced_color_is_hdr;

    #[test]
    fn advanced_color_bits() {
        assert!(advanced_color_is_hdr(0b0011));
        // Auto Color Management on an SDR panel.
        assert!(!advanced_color_is_hdr(0b0111));
        // HDR capable but off.
        assert!(!advanced_color_is_hdr(0b0001));
        assert!(!advanced_color_is_hdr(0));
    }
}
