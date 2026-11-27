//! Linux video surface: a `GtkOverlay` replaces the Tauri window's default child; a `GtkGLArea`
//! sits **under** the transparent WebKitGTK webview; mpv renders into it through the OpenGL
//! render API (PLAN §2.2, ADR-0001). GTK-level, so it works on Wayland and X11.
//!
//! Several surfaces per process (multiview, PLAN §4.11): slot 0 is the main player and owns the
//! overlay; other slots add their own `GtkGLArea` to the same overlay, also under the webview. The
//! GTK state lives in a main-thread thread-local keyed by slot.
//!
//! On Wayland a slot first tries the video subsurface (`wayland_video`): our own `wl_surface` under
//! the window, drawn on its own thread, so video frames bypass GTK and WebKit entirely (and are
//! tagged HDR when the compositor supports `wp_color_management_v1`). The GtkGLArea is the X11 path
//! and the fallback when no subsurface can be made. `DOWNLINK_VIDEO_SURFACE=glarea` forces the
//! GtkGLArea on Wayland too (troubleshooting).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gtk::glib;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use libloading::Library;

use super::wayland_video::SubsurfaceVideo;
use super::{MainThreadDispatch, SurfaceBounds, VideoSurface};
use crate::engine::{Engine, RenderContext};
use crate::error::PlayerError;

const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;

type GetProcFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type GlGetIntegerv = unsafe extern "C" fn(u32, *mut i32);

/// Resolves GL entry points through EGL (Wayland, and X11 when GTK uses EGL) and GLX.
struct GlLoader {
    // Libraries must outlive the function pointers below.
    _libs: Vec<Library>,
    procs: Vec<GetProcFn>,
}

impl GlLoader {
    fn new(wayland: bool) -> Self {
        let mut libs = Vec::new();
        let mut procs: Vec<GetProcFn> = Vec::new();
        let candidates: &[(&str, &[u8])] = if wayland {
            &[
                ("libEGL.so.1", b"eglGetProcAddress\0"),
                ("libGL.so.1", b"glXGetProcAddressARB\0"),
            ]
        } else {
            &[
                ("libGL.so.1", b"glXGetProcAddressARB\0"),
                ("libGLX.so.0", b"glXGetProcAddressARB\0"),
                ("libEGL.so.1", b"eglGetProcAddress\0"),
            ]
        };
        for (lib, sym) in candidates {
            // SAFETY: loading the system GL/EGL libraries, which GTK itself has already loaded.
            if let Ok(l) = unsafe { Library::new(lib) } {
                // SAFETY: both symbols have the signature `void *(*)(const char *)`.
                if let Ok(f) = unsafe { l.get::<GetProcFn>(sym) } {
                    procs.push(*f);
                    libs.push(l);
                }
            }
        }
        Self { _libs: libs, procs }
    }

    fn proc_address(&self, name: *const c_char) -> *mut c_void {
        for p in &self.procs {
            // SAFETY: `name` is a valid NUL-terminated string supplied by mpv or us.
            let r = unsafe { p(name) };
            if !r.is_null() {
                return r;
            }
        }
        std::ptr::null_mut()
    }
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` is the `GlLoader` we passed to `create_render_context`, kept alive in an
    // `Rc` captured by the GLArea signal closures for the life of the render context.
    let loader = unsafe { &*(ctx as *const GlLoader) };
    loader.proc_address(name)
}

/// Dev-only: log a centre-pixel readback of the GLArea every 60 frames (`DOWNLINK_DEBUG_READBACK`).
fn debug_readback() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("DOWNLINK_DEBUG_READBACK").is_some())
}

type CreateFn = Rc<dyn Fn(&gtk::GLArea)>;
type CreateSlot = Rc<RefCell<Option<CreateFn>>>;

struct GtkState {
    overlay: gtk::Overlay,
    video: Video,
    /// The newest bounds the UI asked for. Re-applied whenever the overlay is re-allocated: the
    /// webview-relative rect does not change when the window goes fullscreen, but the overlay's offset in
    /// the toplevel (CSD shadow) and the toplevel's scale do, and those are read at apply time.
    bounds: std::cell::Cell<Option<SurfaceBounds>>,
}

/// Place the slot's surface for `b` using the overlay/toplevel geometry as it is right now.
fn apply_bounds(st: &GtkState, b: SurfaceBounds) {
    let scale = if b.scale > 0.0 { b.scale } else { 1.0 };
    let l = |v: f64| (v / scale).round() as i32;
    let (x, y) = (l(f64::from(b.x)).max(0), l(f64::from(b.y)).max(0));
    let (w, h) = (l(f64::from(b.w)).max(1), l(f64::from(b.h)).max(1));
    match &st.video {
        Video::GlArea { glarea, .. } => {
            glarea.set_margin_start(x);
            glarea.set_margin_top(y);
            glarea.set_size_request(w, h);
            glarea.queue_render();
        }
        Video::Subsurface { surface, toplevel } => {
            // Bounds are relative to the webview, which fills the overlay; the subsurface is
            // positioned in the window surface's coordinates (CSD shadow included).
            let (ox, oy) = st
                .overlay
                .translate_coordinates(toplevel, 0, 0)
                .unwrap_or((0, 0));
            surface.set_geometry(ox + x, oy + y, w, h, toplevel.scale_factor());
            // The position is applied on the window's next commit.
            toplevel.queue_draw();
        }
    }
}

/// Re-apply every slot's last bounds (overlay re-allocated, e.g. fullscreen finished).
fn reapply_all() {
    STATE.with(|s| {
        if let Ok(map) = s.try_borrow() {
            for st in map.values() {
                if let Some(b) = st.bounds.get() {
                    apply_bounds(st, b);
                }
            }
        }
    });
}

/// How this slot shows video.
enum Video {
    /// mpv renders into a GtkGLArea under the webview (SDR; X11, or no Wayland subsurface).
    GlArea {
        glarea: gtk::GLArea,
        render: Rc<RefCell<Option<RenderContext>>>,
        engine_slot: Rc<RefCell<Arc<Engine>>>,
        create: CreateSlot,
    },
    /// mpv renders into our own Wayland subsurface under the window on its own thread, tagged HDR
    /// when the content and the output are (see `wayland_video`).
    Subsurface {
        surface: SubsurfaceVideo,
        toplevel: gtk::Widget,
    },
}

thread_local! {
    static STATE: RefCell<HashMap<u32, GtkState>> = RefCell::new(HashMap::new());
}

/// Linux surface; see the module docs.
pub struct LinuxSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    frames: Arc<AtomicU64>,
    slot: u32,
}

impl LinuxSurface {
    /// The main player's surface (slot 0).
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self::with_slot(engine, dispatch, 0)
    }

    /// A multiview tile's surface. Slot 0 must already be attached.
    pub fn with_slot(engine: Arc<Engine>, dispatch: MainThreadDispatch, slot: u32) -> Self {
        Self {
            engine,
            dispatch,
            frames: Arc::new(AtomicU64::new(0)),
            slot,
        }
    }

    fn with_state(&self, f: impl FnOnce(&GtkState) + Send + 'static) {
        let slot = self.slot;
        (self.dispatch)(Box::new(move || {
            STATE.with(|s| {
                if let Some(st) = s.borrow().get(&slot) {
                    f(st);
                }
            });
        }));
    }

    /// Number of frames rendered so far (for FPS measurement).
    pub fn render_count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

/// The window's own `wl_surface` (the subsurface parent), when GTK runs on Wayland.
fn toplevel_wl_surface(toplevel: &gtk::Widget) -> Option<*mut c_void> {
    let gdk_window = toplevel.window()?;
    let raw: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
    let this = libloading::os::unix::Library::this();
    // SAFETY: symbol from libgdk-3 with this exact signature; `raw` is a live GdkWindow.
    let f = unsafe {
        this.get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
            b"gdk_wayland_window_get_wl_surface\0",
        )
    }
    .ok()?;
    // SAFETY: as above.
    let p = unsafe { f(raw.cast()) };
    (!p.is_null()).then_some(p)
}

/// `(wl_display, x11_display)` pointers for mpv's hwdec interop; one of them is null.
fn display_pointers() -> (*mut c_void, *mut c_void, bool) {
    let Some(display) = gtk::gdk::Display::default() else {
        return (std::ptr::null_mut(), std::ptr::null_mut(), false);
    };
    let ty = display.type_().name();
    let raw: *mut gtk::gdk::ffi::GdkDisplay = display.to_glib_none().0;
    let this = libloading::os::unix::Library::this();
    if ty.contains("Wayland") {
        // SAFETY: symbol from libgdk-3 with this exact signature; `raw` is a live GdkDisplay.
        if let Ok(f) = unsafe {
            this.get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                b"gdk_wayland_display_get_wl_display\0",
            )
        } {
            // SAFETY: as above.
            return (unsafe { f(raw.cast()) }, std::ptr::null_mut(), true);
        }
    } else if ty.contains("X11") {
        // SAFETY: as above, for the X11 backend.
        if let Ok(f) = unsafe {
            this.get::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                b"gdk_x11_display_get_xdisplay\0",
            )
        } {
            // SAFETY: as above.
            return (std::ptr::null_mut(), unsafe { f(raw.cast()) }, false);
        }
    }
    (
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        ty.contains("Wayland"),
    )
}

impl VideoSurface for LinuxSurface {
    /// `parent` is the Tauri window's default vbox (`WebviewWindow::default_vbox()`).
    type Parent = gtk::Box;

    fn attach(&mut self, vbox: &gtk::Box) -> Result<(), PlayerError> {
        // Tiles join the overlay slot 0 created; only slot 0 re-parents the webview.
        let existing = STATE.with(|s| s.borrow().get(&0).map(|st| st.overlay.clone()));
        let (overlay, webview) = if self.slot == 0 {
            let webview = vbox
                .children()
                .into_iter()
                .find(|c| c.type_().name().contains("WebView"))
                .ok_or_else(|| PlayerError::Surface("no webview found in the window".into()))?;
            let overlay = gtk::Overlay::new();
            let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
            overlay.add(&base);
            overlay.connect_size_allocate(|_, _| reapply_all());
            (overlay, Some(webview))
        } else {
            let overlay = existing.ok_or_else(|| {
                PlayerError::Surface("the main video surface is not attached".into())
            })?;
            (overlay, None)
        };

        let (wl, x11, wayland) = display_pointers();
        let force_glarea = std::env::var("DOWNLINK_VIDEO_SURFACE").is_ok_and(|v| v == "glarea");
        tracing::info!(
            wayland,
            hdr_auto = super::hdr_output_auto(),
            force_glarea,
            "video surface: choosing path"
        );
        if wayland && !wl.is_null() && !force_glarea {
            let toplevel = vbox.toplevel();
            if let Some(parent) = toplevel.as_ref().and_then(toplevel_wl_surface) {
                // SAFETY: GTK's wl_display and the window's wl_surface live as long as the window,
                // and the surface is dropped (detach) before the window goes away.
                let sub = unsafe {
                    SubsurfaceVideo::new(wl, parent, self.engine.clone(), self.frames.clone())
                };
                if let (Some(surface), Some(toplevel)) = (sub, toplevel) {
                    if self.slot == 0 {
                        toplevel.connect_scale_factor_notify(|_| reapply_all());
                    }
                    if let Some(webview) = webview {
                        vbox.remove(&webview);
                        overlay.add_overlay(&webview);
                        webview.set_hexpand(true);
                        webview.set_vexpand(true);
                        vbox.pack_start(&overlay, true, true, 0);
                        overlay.show_all();
                    }
                    let slot = self.slot;
                    STATE.with(|s| {
                        s.borrow_mut().insert(
                            slot,
                            GtkState {
                                overlay,
                                video: Video::Subsurface { surface, toplevel },
                                bounds: std::cell::Cell::new(None),
                            },
                        );
                    });
                    return Ok(());
                }
            }
        }

        tracing::info!("video surface: GtkGLArea (SDR, tone-mapped)");
        let glarea = gtk::GLArea::new();
        glarea.set_halign(gtk::Align::Start);
        glarea.set_valign(gtk::Align::Start);
        glarea.set_size_request(1, 1);
        glarea.set_has_depth_buffer(false);
        glarea.set_has_alpha(false);
        glarea.set_auto_render(false);

        let render: Rc<RefCell<Option<RenderContext>>> = Rc::new(RefCell::new(None));
        let loader = Rc::new(GlLoader::new(wayland));
        let gl_get_integerv: Rc<RefCell<Option<GlGetIntegerv>>> = Rc::new(RefCell::new(None));
        let engine_slot: Rc<RefCell<Arc<Engine>>> = Rc::new(RefCell::new(self.engine.clone()));
        let create_cell: CreateSlot = Rc::new(RefCell::new(None));
        // Size of the last frame drawn into the area; `None` forces the next render (new context).
        let drawn_size: Rc<std::cell::Cell<Option<(i32, i32)>>> =
            Rc::new(std::cell::Cell::new(None));

        {
            let render = render.clone();
            let loader = loader.clone();
            let engine_slot = engine_slot.clone();
            let gl_get = gl_get_integerv.clone();
            let create_slot = create_cell.clone();
            let drawn_size = drawn_size.clone();
            let create: Rc<dyn Fn(&gtk::GLArea)> = Rc::new(move |area: &gtk::GLArea| {
                drawn_size.set(None);
                let engine = engine_slot.borrow().clone();
                area.make_current();
                if let Some(err) = area.error() {
                    tracing::error!("GtkGLArea could not create a GL context: {err}");
                    return;
                }
                // SAFETY: the GL context is current; the loader lives in `loader` (captured by
                // this and the render closure) for at least as long as the render context.
                let created = unsafe {
                    engine.create_render_context(
                        get_proc_address,
                        Rc::as_ptr(&loader).cast_mut().cast::<c_void>(),
                        wl,
                        x11,
                    )
                };
                match created {
                    Ok(mut rc) => {
                        let weak = glib::SendWeakRef::from(area.downgrade());
                        rc.set_update_callback(move || {
                            let weak = weak.clone();
                            glib::MainContext::default().invoke(move || {
                                if let Some(a) = weak.upgrade() {
                                    a.queue_render();
                                }
                            });
                        });
                        let name = c"glGetIntegerv";
                        let p = loader.proc_address(name.as_ptr());
                        if !p.is_null() {
                            // SAFETY: glGetIntegerv has the signature `void (GLenum, GLint *)`.
                            *gl_get.borrow_mut() = Some(unsafe {
                                std::mem::transmute::<*mut c_void, GlGetIntegerv>(p)
                            });
                        }
                        *render.borrow_mut() = Some(rc);
                        tracing::info!("mpv render context created (OpenGL, wayland={wayland})");
                    }
                    Err(e) => tracing::error!("mpv_render_context_create failed: {e}"),
                }
            });
            *create_slot.borrow_mut() = Some(create.clone());
            glarea.connect_realize(move |a| create(a));
        }
        {
            let render = render.clone();
            glarea.connect_unrealize(move |area| {
                area.make_current();
                // Dropping frees the mpv render context with the GL context current.
                *render.borrow_mut() = None;
            });
        }
        {
            let render = render.clone();
            let loader_keepalive = loader.clone();
            let gl_get = gl_get_integerv.clone();
            let frames = self.frames.clone();
            let drawn_size = drawn_size.clone();
            glarea.connect_render(move |area, _ctx| {
                let _keep = &loader_keepalive;
                if let Some(rc) = render.borrow().as_ref() {
                    let scale = area.scale_factor();
                    let (w, h) = (
                        area.allocated_width() * scale,
                        area.allocated_height() * scale,
                    );
                    // GTK keeps the area's framebuffer between draws, so a render signal with no
                    // new mpv frame and an unchanged size (bounds re-applied, a stray redraw) can
                    // reuse it. `update()` must still run every time to consume mpv's flags.
                    let fresh = rc.update();
                    if !fresh && drawn_size.get() == Some((w, h)) {
                        return glib::Propagation::Stop;
                    }
                    drawn_size.set(Some((w, h)));
                    let mut fbo: i32 = 0;
                    if let Some(f) = *gl_get.borrow() {
                        // SAFETY: GL context is current inside the render signal; `fbo` is a
                        // valid out pointer for one GLint.
                        unsafe { f(GL_FRAMEBUFFER_BINDING, &mut fbo) };
                    }
                    // Never block for the frame's target time: this runs on the GTK main thread,
                    // which also drives the webview, IPC and input (and the other multiview tiles).
                    // mpv has already queued the frame for its display time; GTK's frame clock
                    // presents it on the next vsync.
                    if let Err(e) = rc.render(fbo, w, h, true, false) {
                        tracing::warn!("mpv render failed: {e}");
                    }
                    rc.report_swap();
                    if debug_readback() {
                        let n = frames.load(Ordering::Relaxed);
                        if n.is_multiple_of(60) {
                            let name = c"glReadPixels";
                            let p = loader_keepalive.proc_address(name.as_ptr());
                            if !p.is_null() {
                                type ReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
                                // SAFETY: glReadPixels(x, y, w, h, format=GL_RGBA, type=GL_UNSIGNED_BYTE, data).
                                let f = unsafe { std::mem::transmute::<*mut c_void, ReadPixels>(p) };
                                let bind = loader_keepalive.proc_address(c"glBindFramebuffer".as_ptr());
                                if !bind.is_null() {
                                    // SAFETY: glBindFramebuffer(GL_FRAMEBUFFER, fbo) with the context current.
                                    let b = unsafe {
                                        std::mem::transmute::<*mut c_void, unsafe extern "C" fn(u32, u32)>(bind)
                                    };
                                    // SAFETY: as above.
                                    unsafe { b(0x8D40, fbo as u32) };
                                }
                                let (rw, rh) = (w.max(1), h.max(1));
                                let mut px = vec![0u8; (rw * rh * 4) as usize];
                                // SAFETY: GL context current; `px` holds one RGBA pixel.
                                unsafe { f(0, 0, rw, rh, 0x1908, 0x1401, px.as_mut_ptr().cast()) };
                                let sum = px.chunks(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count();
                                tracing::info!("GLArea fbo {fbo} {w}x{h} frame {n} non-black pixels {sum} of {} (alpha0 px {})", px.len() / 4, px.chunks(4).filter(|p| p[3] == 0).count());
                            }
                        }
                    }
                    frames.fetch_add(1, Ordering::Relaxed);
                }
                glib::Propagation::Stop
            });
        }

        if let Some(webview) = webview {
            vbox.remove(&webview);
            overlay.add_overlay(&glarea);
            overlay.add_overlay(&webview);
            webview.set_hexpand(true);
            webview.set_vexpand(true);
            vbox.pack_start(&overlay, true, true, 0);
            overlay.show_all();
        } else {
            // Under the webview: index 0 is drawn first. Tiles never overlap each other.
            overlay.add_overlay(&glarea);
            overlay.reorder_overlay(&glarea, 0);
            glarea.show();
        }

        let slot = self.slot;
        STATE.with(|s| {
            s.borrow_mut().insert(
                slot,
                GtkState {
                    overlay,
                    video: Video::GlArea {
                        glarea,
                        render,
                        engine_slot,
                        create: create_cell,
                    },
                    bounds: std::cell::Cell::new(None),
                },
            );
        });
        Ok(())
    }

    /// The supervisor replaced a dead mpv instance: free the old render context and create one
    /// on the new engine (GL context current), then redraw.
    fn rebind(&self, engine: Arc<Engine>) {
        self.with_state(move |st| match &st.video {
            Video::GlArea {
                glarea,
                render,
                engine_slot,
                create,
            } => {
                *engine_slot.borrow_mut() = engine;
                glarea.make_current();
                *render.borrow_mut() = None;
                if let Some(create) = create.borrow().as_ref() {
                    create(glarea);
                }
                glarea.queue_render();
            }
            Video::Subsurface { surface, .. } => surface.rebind(engine),
        });
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        self.with_state(move |st| {
            st.bounds.set(Some(b));
            apply_bounds(st, b);
        });
    }

    fn set_visible(&self, visible: bool) {
        self.with_state(move |st| match &st.video {
            // Hiding does not unrealize, so the render context survives.
            Video::GlArea { glarea, .. } => glarea.set_visible(visible),
            Video::Subsurface { surface, .. } => surface.set_visible(visible),
        });
    }

    /// Must run on the main thread.
    fn detach(&mut self) {
        let slot = self.slot;
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().remove(&slot) {
                match st.video {
                    Video::GlArea { glarea, render, .. } => {
                        glarea.make_current();
                        *render.borrow_mut() = None;
                        st.overlay.remove(&glarea);
                    }
                    // Dropping stops the render thread and destroys the subsurface.
                    Video::Subsurface { surface, .. } => drop(surface),
                }
            }
        });
    }
}
