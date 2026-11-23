//! macOS video surface: a layer-backed OpenGL `NSView` inserted **below** the `WKWebView`
//! (`addSubview:positioned:NSWindowBelow relativeTo:`), mpv rendering through the OpenGL render
//! API (PLAN §2.2). mpv's macOS backend has no window-embedding path, so `--wid` is not used.
//!
//! Several surfaces per process (multiview, PLAN §4.11): slot 0 is the main player, other slots
//! are tiles. Each slot has its own `NSOpenGLView`, GL context and mpv render context, all below
//! the webview in the same container. The AppKit state lives in a main-thread thread-local keyed
//! by slot, as on Linux.
//!
//! UNVERIFIED: written without access to a Mac and only type-checked (`cargo check --target
//! aarch64-apple-darwin`) where the toolchain allows. Needs testing by macOS testers: Space
//! switches, fullscreen, Retina scale, z-order, several tiles at once, the deprecated-GL path
//! versus a Metal layer.
//!
//! Design:
//! - `attach(parent)`: `parent` is the `NSWindow*`. Find the `WKWebView` among the content view's
//!   subviews and insert an `NSOpenGLView` (core 3.2, best-resolution surface) below it.
//! - Rendering is driven by mpv's update callback: hop to the main thread, make the slot's context
//!   current, `mpv_render_context_render` into the bound FBO, `flushBuffer`. While more than one
//!   surface is attached, renders do not block for the frame's target time (one blocking render
//!   on the main thread starves the other tiles).
//! - `set_bounds`: physical pixels -> points (`scale`), y flipped for AppKit's bottom-left origin.
//! - Screen change / resize: `NSWindowDidChangeScreenNotification` (this window) and
//!   `NSViewFrameDidChangeNotification` (this slot's view) call `-[NSOpenGLContext update]` and
//!   redraw.
//! - `rebind`: the supervisor replaced a dead mpv instance; the slot's render context is recreated
//!   on the new engine.

use std::cell::RefCell;
use std::collections::HashMap;
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

impl GlLoader {
    fn as_ctx(&self) -> *mut c_void {
        (self as *const GlLoader).cast_mut().cast::<c_void>()
    }
}

/// One slot's AppKit and GL objects.
struct State {
    gl_view: Retained<AnyObject>,
    gl_context: Retained<AnyObject>,
    superview: Retained<AnyObject>,
    render: RefCell<Option<RenderContext>>,
    loader: Box<GlLoader>,
    gl_get: Option<GlGetIntegerv>,
    observers: Vec<Retained<AnyObject>>,
}

/// More than one surface is attached (multiview). Renders then must not block for the frame's
/// target time: all surfaces render on the main thread, and one blocking render starves the rest.
static SURFACES: AtomicU64 = AtomicU64::new(0);

fn multi_active() -> bool {
    SURFACES.load(Ordering::Relaxed) > 1
}

thread_local! {
    static STATE: RefCell<HashMap<u32, State>> = RefCell::new(HashMap::new());
}

/// Run `f` on the slot's state, if attached. Main thread only.
fn with_slot_state(slot: u32, f: impl FnOnce(&State)) {
    STATE.with(|s| {
        if let Some(st) = s.borrow().get(&slot) {
            f(st);
        }
    });
}

/// macOS surface; see the module docs.
pub struct MacSurface {
    engine: Arc<Engine>,
    dispatch: MainThreadDispatch,
    frames: Arc<AtomicU64>,
    slot: u32,
}

impl MacSurface {
    /// The main player's surface (slot 0).
    pub fn new(engine: Arc<Engine>, dispatch: MainThreadDispatch) -> Self {
        Self::with_slot(engine, dispatch, 0)
    }

    /// A multiview tile's surface: its own view and render context, also below the webview.
    pub fn with_slot(engine: Arc<Engine>, dispatch: MainThreadDispatch, slot: u32) -> Self {
        Self {
            engine,
            dispatch,
            frames: Arc::new(AtomicU64::new(0)),
            slot,
        }
    }

    fn with_state(&self, f: impl FnOnce(&State) + Send + 'static) {
        let slot = self.slot;
        (self.dispatch)(Box::new(move || with_slot_state(slot, f)));
    }

    /// Number of frames rendered so far (for FPS measurement).
    pub fn render_count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

fn class(name: &CStr) -> Result<&'static AnyClass, PlayerError> {
    AnyClass::get(name).ok_or_else(|| PlayerError::Surface(format!("missing class {name:?}")))
}

/// Create an mpv render context on `engine` whose update callback redraws `slot`. The slot's GL
/// context must be current. Main thread only.
fn create_render(
    engine: &Arc<Engine>,
    loader: &GlLoader,
    dispatch: &MainThreadDispatch,
    frames: &Arc<AtomicU64>,
    slot: u32,
) -> Result<RenderContext, PlayerError> {
    // SAFETY: the caller made the slot's GL context current; `loader` lives in the slot's `State`
    // (or is moved there right after) for at least as long as the render context.
    let mut rc = unsafe {
        engine.create_render_context(
            get_proc_address,
            loader.as_ctx(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )?
    };
    let dispatch = dispatch.clone();
    let frames = frames.clone();
    rc.set_update_callback(move || {
        let frames = frames.clone();
        dispatch(Box::new(move || draw(slot, &frames)));
    });
    Ok(rc)
}

/// Draw one frame for `slot`. Main thread only.
fn draw(slot: u32, frames: &AtomicU64) {
    with_slot_state(slot, |st| {
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
            if let Err(e) = rc.render(fbo, w, h, true, !multi_active()) {
                tracing::warn!("mpv render failed: {e}");
            }
            rc.report_swap();
            let _: () = msg_send![&*st.gl_context, flushBuffer];
        }
        frames.fetch_add(1, Ordering::Relaxed);
    });
}

fn update_context(slot: u32) {
    with_slot_state(slot, |st| {
        // SAFETY: message to the retained NSOpenGLContext on the main thread.
        unsafe {
            let _: () = msg_send![&*st.gl_context, update];
        }
    });
}

impl VideoSurface for MacSurface {
    /// Pointer to the Tauri window's `NSWindow` (`WebviewWindow::ns_window()`).
    type Parent = *mut c_void;

    fn attach(&mut self, parent: &Self::Parent) -> Result<(), PlayerError> {
        let window = NonNull::new((*parent).cast::<AnyObject>())
            .ok_or_else(|| PlayerError::Surface("null NSWindow".into()))?;
        let slot = self.slot;
        // SAFETY: `window` is the live NSWindow pointer handed over by Tauri; all messages below
        // are sent on the main thread (attach is called on it).
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
            let _: () = msg_send![&*gl_view, setPostsFrameChangedNotifications: Bool::YES];
            let gl_context: Retained<AnyObject> = msg_send![&*gl_view, openGLContext];
            let _: () = msg_send![&*gl_context, makeCurrentContext];
            let swap_interval: i32 = 1;
            let _: () = msg_send![&*gl_context, setValues: &swap_interval, forParameter: 222isize];

            // Below the webview, same container. Tiles never overlap each other.
            let superview: Retained<AnyObject> = msg_send![&*webview, superview];
            let _: () = msg_send![
                &*superview,
                addSubview: &*gl_view,
                positioned: NS_WINDOW_BELOW,
                relativeTo: &*webview
            ];

            let lib = match Library::new("/System/Library/Frameworks/OpenGL.framework/OpenGL") {
                Ok(lib) => lib,
                Err(e) => {
                    let _: () = msg_send![&*gl_view, removeFromSuperview];
                    return Err(PlayerError::Surface(format!("OpenGL framework: {e}")));
                }
            };
            let loader = Box::new(GlLoader { lib });
            let rc = match create_render(&self.engine, &loader, &self.dispatch, &self.frames, slot)
            {
                Ok(rc) => rc,
                Err(e) => {
                    let _: () = msg_send![&*gl_view, removeFromSuperview];
                    return Err(e);
                }
            };
            let gl_get = {
                let p = get_proc_address(loader.as_ctx(), c"glGetIntegerv".as_ptr());
                // SAFETY: glGetIntegerv has the signature `void (GLenum, GLint *)`.
                (!p.is_null()).then(|| std::mem::transmute::<*mut c_void, GlGetIntegerv>(p))
            };

            // This window's screen change and this view's frame change: refresh the context's
            // drawable and redraw this slot.
            let mut observers = Vec::new();
            let center: Retained<AnyObject> =
                msg_send![class(c"NSNotificationCenter")?, defaultCenter];
            let sources: [(&str, &AnyObject); 2] = [
                ("NSWindowDidChangeScreenNotification", window),
                ("NSViewFrameDidChangeNotification", &*gl_view),
            ];
            for (name, object) in sources {
                let ns_name = NSString::from_str(name);
                let frames = self.frames.clone();
                let block = RcBlock::new(move |_n: *mut AnyObject| {
                    update_context(slot);
                    draw(slot, &frames);
                });
                let obs: Retained<AnyObject> = msg_send![
                    &*center,
                    addObserverForName: &*ns_name,
                    object: object,
                    queue: std::ptr::null::<AnyObject>(),
                    usingBlock: &*block
                ];
                observers.push(obs);
            }

            SURFACES.fetch_add(1, Ordering::Relaxed);
            STATE.with(|s| {
                s.borrow_mut().insert(
                    slot,
                    State {
                        gl_view,
                        gl_context,
                        superview,
                        render: RefCell::new(Some(rc)),
                        loader,
                        gl_get,
                        observers,
                    },
                );
            });
        }
        Ok(())
    }

    /// The supervisor replaced a dead mpv instance: free the old render context and create one
    /// on the new engine (GL context current), then redraw.
    fn rebind(&self, engine: Arc<Engine>) {
        let slot = self.slot;
        let dispatch = self.dispatch.clone();
        let frames = self.frames.clone();
        (self.dispatch)(Box::new(move || {
            with_slot_state(slot, |st| {
                // SAFETY: message to the retained NSOpenGLContext on the main thread.
                unsafe {
                    let _: () = msg_send![&*st.gl_context, makeCurrentContext];
                }
                // Dropping frees the old render context with the GL context current.
                *st.render.borrow_mut() = None;
                match create_render(&engine, &st.loader, &dispatch, &frames, slot) {
                    Ok(rc) => *st.render.borrow_mut() = Some(rc),
                    Err(e) => tracing::error!("mpv_render_context_create failed: {e}"),
                }
            });
            draw(slot, &frames);
        }));
    }

    fn set_bounds(&self, b: SurfaceBounds) {
        self.with_state(move |st| {
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
                let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(1.0), h.max(1.0)));
                let _: () = msg_send![&*st.gl_view, setFrame: frame];
                let _: () = msg_send![&*st.gl_context, update];
            }
        });
    }

    fn set_visible(&self, visible: bool) {
        self.with_state(move |st| {
            // SAFETY: message to a retained view on the main thread. Hiding keeps the render
            // context alive.
            unsafe {
                let _: () = msg_send![&*st.gl_view, setHidden: Bool::new(!visible)];
            }
        });
    }

    /// Must run on the main thread.
    fn detach(&mut self) {
        // Take the slot out first so nothing AppKit calls back into during teardown sees it.
        let Some(st) = STATE.with(|s| s.borrow_mut().remove(&self.slot)) else {
            return;
        };
        SURFACES.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: main-thread messages to retained objects; the render context is freed with the
        // GL context current, then the view leaves the hierarchy.
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
}
