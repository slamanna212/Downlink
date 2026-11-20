//! Platform video surface selection and attachment to the main window.

use std::sync::Arc;

use downlink_player::{MainThreadDispatch, Player, VideoSurface};
use tauri::{AppHandle, Manager};

#[cfg(target_os = "linux")]
pub type Surface = downlink_player::surface::linux::LinuxSurface;
#[cfg(target_os = "windows")]
pub type Surface = downlink_player::surface::windows::WindowsSurface;
#[cfg(target_os = "macos")]
pub type Surface = downlink_player::surface::macos::MacSurface;

#[cfg(target_os = "linux")]
pub fn attach(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    attach_slot(app, p.engine(), dispatch, 0)
}

/// Attach a surface for `slot` (0 = main player, 1.. = multiview tiles). Main thread only.
#[cfg(target_os = "linux")]
pub fn attach_slot(
    app: &AppHandle,
    engine: Arc<downlink_player::Engine>,
    dispatch: MainThreadDispatch,
    slot: u32,
) -> Result<Surface, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let vbox = win.default_vbox().map_err(|e| e.to_string())?;
    let mut s = Surface::with_slot(engine, dispatch, slot);
    s.attach(&vbox).map_err(|e| e.to_string())?;
    Ok(s)
}

#[cfg(target_os = "windows")]
pub fn attach(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    attach_slot(app, p.engine(), dispatch, 0)
}

/// Attach a surface for `slot` (0 = main player, 1.. = multiview tiles). Main thread only.
#[cfg(target_os = "windows")]
pub fn attach_slot(
    app: &AppHandle,
    engine: Arc<downlink_player::Engine>,
    dispatch: MainThreadDispatch,
    slot: u32,
) -> Result<Surface, String> {
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let hwnd = win.hwnd().map_err(|e| e.to_string())?;
    let mut s = Surface::with_slot(engine, dispatch, slot);
    s.attach(&(hwnd.0 as isize)).map_err(|e| e.to_string())?;
    Ok(s)
}

#[cfg(target_os = "macos")]
pub fn attach(
    app: &AppHandle,
    p: &Arc<Player>,
    dispatch: MainThreadDispatch,
) -> Result<Surface, String> {
    attach_slot(app, p.engine(), dispatch, 0)
}

/// Attach a surface for `slot`. macOS supports slot 0 only (`MAX_SURFACES` is 1). Main thread only.
#[cfg(target_os = "macos")]
pub fn attach_slot(
    app: &AppHandle,
    engine: Arc<downlink_player::Engine>,
    dispatch: MainThreadDispatch,
    slot: u32,
) -> Result<Surface, String> {
    if slot != 0 {
        return Err("multiview is not available on macOS yet".into());
    }
    let win = app.get_webview_window("main").ok_or("no main window")?;
    let view = win.ns_window().map_err(|e| e.to_string())?;
    let mut s = Surface::with_slot(engine, dispatch, slot);
    s.attach(&view).map_err(|e| e.to_string())?;
    Ok(s)
}
