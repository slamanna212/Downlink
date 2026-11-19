//! Linux video surface: a `GtkOverlay` replaces the Tauri window's default child; a `GtkGLArea`
//! sits **under** the transparent WebKitGTK webview; mpv renders into it through the OpenGL
//! render API (PLAN §2.2, ADR-0001). GTK-level, so it works on Wayland and X11.
//!
//! Only one surface per process (the GTK state lives in a main-thread thread-local).

use std::cell::RefCell;
use std::ffi::{c_char, c_void};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use gtk::glib;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use libloading::Library;

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

struct GtkState {
    overlay: gtk::Overlay,
    glarea: gtk::GLArea,
    render: Rc<RefCell<Option<RenderContext>>>,
}

thread_local! {
    static STATE: RefCell<Option<GtkState>> = const { RefCell::new(None) };
}

/// Linux surface; see the module docs.
pub struct LinuxSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    frames: Arc<AtomicU64>,
}

impl LinuxSurface {
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self {
            engine,
            dispatch,
            frames: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of frames rendered into the GLArea so far (for FPS measurement).
    pub fn render_count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
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
        let webview = vbox
            .children()
            .into_iter()
            .find(|c| c.type_().name().contains("WebView"))
            .ok_or_else(|| PlayerError::Surface("no webview found in the window".into()))?;

        let overlay = gtk::Overlay::new();
        let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
        overlay.add(&base);

        let glarea = gtk::GLArea::new();
        glarea.set_halign(gtk::Align::Start);
        glarea.set_valign(gtk::Align::Start);
        glarea.set_size_request(1, 1);
        glarea.set_has_depth_buffer(false);
        glarea.set_has_alpha(false);
        glarea.set_auto_render(false);

        let render: Rc<RefCell<Option<RenderContext>>> = Rc::new(RefCell::new(None));
        let (wl, x11, wayland) = display_pointers();
        let loader = Rc::new(GlLoader::new(wayland));
        let gl_get_integerv: Rc<RefCell<Option<GlGetIntegerv>>> = Rc::new(RefCell::new(None));

        {
            let render = render.clone();
            let loader = loader.clone();
            let engine = self.engine.clone();
            let gl_get = gl_get_integerv.clone();
            glarea.connect_realize(move |area| {
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
            glarea.connect_render(move |area, _ctx| {
                let _keep = &loader_keepalive;
                if let Some(rc) = render.borrow().as_ref() {
                    rc.update();
                    let scale = area.scale_factor();
                    let (w, h) = (
                        area.allocated_width() * scale,
                        area.allocated_height() * scale,
                    );
                    let mut fbo: i32 = 0;
                    if let Some(f) = *gl_get.borrow() {
                        // SAFETY: GL context is current inside the render signal; `fbo` is a
                        // valid out pointer for one GLint.
                        unsafe { f(GL_FRAMEBUFFER_BINDING, &mut fbo) };
                    }
                    if let Err(e) = rc.render(fbo, w, h, true) {
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

        vbox.remove(&webview);
        overlay.add_overlay(&glarea);
        overlay.add_overlay(&webview);
        webview.set_hexpand(true);
        webview.set_vexpand(true);
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();

        STATE.with(|s| {
            *s.borrow_mut() = Some(GtkState {
                overlay,
                glarea,
                render,
            });
        });
        Ok(())
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        (self.dispatch)(Box::new(move || {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    let scale = if b.scale > 0.0 { b.scale } else { 1.0 };
                    let l = |v: f64| (v / scale).round() as i32;
                    st.glarea.set_margin_start(l(f64::from(b.x)).max(0));
                    st.glarea.set_margin_top(l(f64::from(b.y)).max(0));
                    st.glarea
                        .set_size_request(l(f64::from(b.w)).max(1), l(f64::from(b.h)).max(1));
                    st.glarea.queue_render();
                }
            });
        }));
    }

    fn set_visible(&self, visible: bool) {
        (self.dispatch)(Box::new(move || {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    // Hiding does not unrealize, so the render context survives.
                    st.glarea.set_visible(visible);
                }
            });
        }));
    }

    fn detach(&mut self) {
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().take() {
                *st.render.borrow_mut() = None;
                st.overlay.remove(&st.glarea);
            }
        });
    }
}
