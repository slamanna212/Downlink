//! macOS video surface: a layer-backed OpenGL `NSView` inserted **below** the `WKWebView`
//! (`addSubview:positioned:NSWindowBelow relativeTo:`), mpv rendering through the OpenGL render
//! API (PLAN §2.2). mpv's macOS backend has no window-embedding path, so `--wid` is not used.
//!
//! UNVERIFIED: written without access to a Mac and only type-checked (`cargo check --target
//! aarch64-apple-darwin`) where the toolchain allows. Needs M0 testing by macOS testers: Space
//! switches, fullscreen, Retina scale, z-order, the deprecated-GL path versus a Metal layer.
//!
//! Design:
//! - `attach(parent)`: `parent` is the `NSWindow*`. Find the `WKWebView` among the content view's
//!   subviews and insert an `NSOpenGLView` (core 3.2, best-resolution surface) below it.
//! - Rendering is driven by mpv's update callback: hop to the main thread, make the context
//!   current, `mpv_render_context_render` into FBO 0, `flushBuffer`. (A `CVDisplayLink` would give
//!   steadier pacing; left for M0 results because `video-sync=audio` already paces frames.)
//! - `set_bounds`: physical pixels -> points (`scale`), y flipped for AppKit's bottom-left origin.
//! - Screen change / resize: `NSWindowDidChangeScreenNotification` and `NSViewFrameDidChange`
//!   observers call `-[NSOpenGLContext update]` and redraw.

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use libloading::Library;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::{MainThreadDispatch, SurfaceBounds, VideoSurface};
use crate::engine::{Engine, RenderContext};
use crate::error::PlayerError;

const NS_WINDOW_BELOW: isize = -1;
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
type GlGetIntegerv = unsafe extern "C" fn(u32, *mut i32);

/// Loads GL entry points from the system OpenGL framework.
struct GlLoader {
    lib: Library,
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` is the `GlLoader` kept alive in `State` for the render context's lifetime;
    // `name` is a NUL-terminated string from mpv.
    let loader = unsafe { &*(ctx as *const GlLoader) };
    // SAFETY: symbol lookup by NUL-terminated name; the pointer is used only as a function address.
    let cname = unsafe { CStr::from_ptr(name) };
    match unsafe { loader.lib.get::<*mut c_void>(cname.to_bytes_with_nul()) } {
        Ok(sym) => *sym,
        Err(_) => std::ptr::null_mut(),
    }
}

struct State {
    gl_view: Retained<AnyObject>,
    gl_context: Retained<AnyObject>,
    superview: Retained<AnyObject>,
    render: RefCell<Option<RenderContext>>,
    loader: Box<GlLoader>,
    gl_get: Option<GlGetIntegerv>,
    observers: Vec<Retained<AnyObject>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// macOS surface; see the module docs. One surface per process.
pub struct MacSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    frames: Arc<AtomicU64>,
}

impl MacSurface {
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self {
            engine,
            dispatch,
            frames: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn render_count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

fn class(name: &CStr) -> Result<&'static AnyClass, PlayerError> {
    AnyClass::get(name).ok_or_else(|| PlayerError::Surface(format!("missing class {name:?}")))
}

/// Draw one frame. Main thread only.
fn draw(frames: &AtomicU64) {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(st) = s.as_ref() else { return };
        let render = st.render.borrow();
        let Some(rc) = render.as_ref() else { return };
        // SAFETY: Objective-C messages to live objects retained in `State`, on the main thread.
        unsafe {
            let _: () = msg_send![&*st.gl_context, makeCurrentContext];
            let bounds: NSRect = msg_send![&*st.gl_view, bounds];
            let backing: NSRect = msg_send![&*st.gl_view, convertRectToBacking: bounds];
            let (w, h) = (backing.size.width as i32, backing.size.height as i32);
            rc.update();
            let mut fbo: i32 = 0;
            if let Some(f) = st.gl_get {
                f(GL_FRAMEBUFFER_BINDING, &mut fbo);
            }
            if let Err(e) = rc.render(fbo, w, h, true) {
                tracing::warn!("mpv render failed: {e}");
            }
            rc.report_swap();
            let _: () = msg_send![&*st.gl_context, flushBuffer];
        }
        frames.fetch_add(1, Ordering::Relaxed);
    });
}

fn update_context() {
    STATE.with(|s| {
        if let Some(st) = s.borrow().as_ref() {
            // SAFETY: message to the retained NSOpenGLContext on the main thread.
            unsafe {
                let _: () = msg_send![&*st.gl_context, update];
            }
        }
    });
}

impl VideoSurface for MacSurface {
    /// Pointer to the Tauri window's `NSWindow` (`WebviewWindow::ns_window()`).
    type Parent = *mut c_void;

    fn attach(&mut self, parent: &Self::Parent) -> Result<(), PlayerError> {
        let window = NonNull::new((*parent).cast::<AnyObject>())
            .ok_or_else(|| PlayerError::Surface("null NSWindow".into()))?;
        // SAFETY: `window` is the live NSWindow pointer handed over by Tauri; all messages below
        // are sent on the main thread (attach is called from setup).
        unsafe {
            let window: &AnyObject = window.as_ref();
            let content: Option<Retained<AnyObject>> = msg_send![window, contentView];
            let content =
                content.ok_or_else(|| PlayerError::Surface("window has no content view".into()))?;
            let subviews: Retained<AnyObject> = msg_send![&*content, subviews];
            let count: usize = msg_send![&*subviews, count];
            let mut webview: Option<Retained<AnyObject>> = None;
            for i in 0..count {
                let v: Retained<AnyObject> = msg_send![&*subviews, objectAtIndex: i];
                let cls_name: Retained<NSString> = msg_send![&*v, className];
                if cls_name.to_string().contains("WKWebView") {
                    webview = Some(v);
                    break;
                }
            }
            let webview =
                webview.ok_or_else(|| PlayerError::Surface("no WKWebView found".into()))?;

            // Core 3.2, double buffered, accelerated.
            let attrs: [u32; 7] = [99, 0x3200, 5, 73, 0, 0, 0];
            let pf_alloc: *mut AnyObject = msg_send![class(c"NSOpenGLPixelFormat")?, alloc];
            let pf_raw: *mut AnyObject = msg_send![pf_alloc, initWithAttributes: attrs.as_ptr()];
            let pf = Retained::from_raw(pf_raw)
                .ok_or_else(|| PlayerError::Surface("no OpenGL pixel format".into()))?;

            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
            let view_alloc: *mut AnyObject = msg_send![class(c"NSOpenGLView")?, alloc];
            let view_raw: *mut AnyObject =
                msg_send![view_alloc, initWithFrame: frame, pixelFormat: &*pf];
            let gl_view = Retained::from_raw(view_raw)
                .ok_or_else(|| PlayerError::Surface("NSOpenGLView init failed".into()))?;
            let _: () = msg_send![&*gl_view, setWantsBestResolutionOpenGLSurface: Bool::YES];
            let gl_context: Retained<AnyObject> = msg_send![&*gl_view, openGLContext];
            let _: () = msg_send![&*gl_context, makeCurrentContext];
            let swap_interval: i32 = 1;
            let _: () = msg_send![&*gl_context, setValues: &swap_interval, forParameter: 222isize];

            // Below the webview, same container.
            let superview: Retained<AnyObject> = msg_send![&*webview, superview];
            let _: () = msg_send![
                &*superview,
                addSubview: &*gl_view,
                positioned: NS_WINDOW_BELOW,
                relativeTo: &*webview
            ];

            let lib = Library::new("/System/Library/Frameworks/OpenGL.framework/OpenGL")
                .map_err(|e| PlayerError::Surface(format!("OpenGL framework: {e}")))?;
            let loader = Box::new(GlLoader { lib });
            let loader_ptr = (&*loader as *const GlLoader).cast_mut().cast::<c_void>();
            let mut rc = self.engine.create_render_context(
                get_proc_address,
                loader_ptr,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )?;
            let dispatch = self.dispatch.clone();
            let frames = self.frames.clone();
            rc.set_update_callback(move || {
                let frames = frames.clone();
                dispatch(Box::new(move || draw(&frames)));
            });
            let gl_get = {
                let p = get_proc_address(loader_ptr, c"glGetIntegerv".as_ptr());
                // SAFETY: glGetIntegerv has the signature `void (GLenum, GLint *)`.
                (!p.is_null()).then(|| std::mem::transmute::<*mut c_void, GlGetIntegerv>(p))
            };

            // Screen change and frame change: refresh the context's drawable.
            let mut observers = Vec::new();
            let center: Retained<AnyObject> =
                msg_send![class(c"NSNotificationCenter")?, defaultCenter];
            for name in [
                "NSWindowDidChangeScreenNotification",
                "NSViewFrameDidChangeNotification",
            ] {
                let ns_name = NSString::from_str(name);
                let frames = self.frames.clone();
                let block = RcBlock::new(move |_n: *mut AnyObject| {
                    update_context();
                    draw(&frames);
                });
                let obs: Retained<AnyObject> = msg_send![
                    &*center,
                    addObserverForName: &*ns_name,
                    object: std::ptr::null::<AnyObject>(),
                    queue: std::ptr::null::<AnyObject>(),
                    usingBlock: &*block
                ];
                observers.push(obs);
            }

            STATE.with(|s| {
                *s.borrow_mut() = Some(State {
                    gl_view,
                    gl_context,
                    superview,
                    render: RefCell::new(Some(rc)),
                    loader,
                    gl_get,
                    observers,
                });
            });
        }
        Ok(())
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        (self.dispatch)(Box::new(move || {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    let scale = if b.scale > 0.0 { b.scale } else { 1.0 };
                    // SAFETY: messages to retained views on the main thread.
                    unsafe {
                        let sup: NSRect = msg_send![&*st.superview, bounds];
                        let (x, w, h) = (
                            f64::from(b.x) / scale,
                            f64::from(b.w) / scale,
                            f64::from(b.h) / scale,
                        );
                        // AppKit's origin is bottom-left; the UI reports top-left.
                        let y = sup.size.height - f64::from(b.y) / scale - h;
                        let frame =
                            NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(1.0), h.max(1.0)));
                        let _: () = msg_send![&*st.gl_view, setFrame: frame];
                        let _: () = msg_send![&*st.gl_context, update];
                    }
                }
            });
        }));
    }

    fn set_visible(&self, visible: bool) {
        (self.dispatch)(Box::new(move || {
            STATE.with(|s| {
                if let Some(st) = s.borrow().as_ref() {
                    // SAFETY: message to a retained view on the main thread.
                    unsafe {
                        let _: () = msg_send![&*st.gl_view, setHidden: Bool::new(!visible)];
                    }
                }
            });
        }));
    }

    fn detach(&mut self) {
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().take() {
                // SAFETY: main-thread messages to retained objects; the render context is freed
                // with the GL context current, then the view leaves the hierarchy.
                unsafe {
                    let _: () = msg_send![&*st.gl_context, makeCurrentContext];
                    *st.render.borrow_mut() = None;
                    let center: Option<Retained<AnyObject>> =
                        AnyClass::get(c"NSNotificationCenter").map(|c| msg_send![c, defaultCenter]);
                    if let Some(center) = center {
                        for o in &st.observers {
                            let _: () = msg_send![&*center, removeObserver: &**o];
                        }
                    }
                    let _: () = msg_send![&*st.gl_view, removeFromSuperview];
                }
                drop(st.loader);
            }
        });
    }
}
