//! HDR video surface for Linux on Wayland.
//!
//! GTK3 cannot tag its own surface with a colour space, so the video gets its **own** `wl_surface`:
//! a subsurface of the window placed *below* it, seen through the webview's transparent video rect.
//! A render thread draws mpv into it through EGL (10-bit, OpenGL render API) and tags it with
//! `wp_color_management_v1`:
//!
//! - HDR content (PQ/HLG) on an output in HDR mode: mpv renders BT.2020 PQ tone-mapped to the
//!   output's peak, and the surface carries a matching PQ image description.
//! - Anything else: mpv renders SDR as usual and the surface carries no description (sRGB).
//!
//! The output's mode is tracked live (`image_description_changed`), so toggling HDR in the desktop
//! settings switches the video without a restart. When the compositor lacks the protocol (or the
//! features we need), or no 10-bit EGL config exists, [`HdrSurface::new`] returns `None` and the
//! caller keeps the GtkGLArea path.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use libloading::Library;
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_output::WlOutput, wl_region::WlRegion, wl_registry::WlRegistry,
    wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::color_management::v1::client::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_management_surface_v1::WpColorManagementSurfaceV1,
    wp_color_manager_v1::{
        self, Feature, Primaries, RenderIntent, TransferFunction, WpColorManagerV1,
    },
    wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1,
    wp_image_description_info_v1::{self, WpImageDescriptionInfoV1},
    wp_image_description_v1::{self, WpImageDescriptionV1},
};

use crate::engine::{Engine, RenderContext};

/// SDR reference white in an HDR (PQ) signal, ITU-R BT.2408.
const REFERENCE_WHITE_NITS: u32 = 203;
/// Display peak assumed when the compositor does not report one.
const DEFAULT_PEAK_NITS: u32 = 1000;
const GL_RGB10_A2: c_int = 0x8059;

// ---------------------------------------------------------------------------------------------
// EGL / wayland-egl, loaded at run time (GTK has already loaded both on Wayland).

type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;

const EGL_NONE: i32 = 0x3038;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_BIT: i32 = 0x0008;
const EGL_OPENGL_API: u32 = 0x30A2;

struct Egl {
    _egl: Library,
    _wl_egl: Library,
    get_display: unsafe extern "C" fn(*mut c_void) -> EglDisplay,
    initialize: unsafe extern "C" fn(EglDisplay, *mut i32, *mut i32) -> u32,
    bind_api: unsafe extern "C" fn(u32) -> u32,
    choose_config:
        unsafe extern "C" fn(EglDisplay, *const i32, *mut EglConfig, i32, *mut i32) -> u32,
    get_config_attrib: unsafe extern "C" fn(EglDisplay, EglConfig, i32, *mut i32) -> u32,
    create_context:
        unsafe extern "C" fn(EglDisplay, EglConfig, EglContext, *const i32) -> EglContext,
    create_window_surface:
        unsafe extern "C" fn(EglDisplay, EglConfig, *mut c_void, *const i32) -> EglSurface,
    make_current: unsafe extern "C" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> u32,
    swap_buffers: unsafe extern "C" fn(EglDisplay, EglSurface) -> u32,
    swap_interval: unsafe extern "C" fn(EglDisplay, i32) -> u32,
    destroy_surface: unsafe extern "C" fn(EglDisplay, EglSurface) -> u32,
    destroy_context: unsafe extern "C" fn(EglDisplay, EglContext) -> u32,
    get_proc_address: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    wl_egl_window_create: unsafe extern "C" fn(*mut c_void, c_int, c_int) -> *mut c_void,
    wl_egl_window_resize: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int),
    wl_egl_window_destroy: unsafe extern "C" fn(*mut c_void),
}

impl Egl {
    fn load() -> Result<Self, String> {
        // SAFETY: system EGL / wayland-egl libraries, already loaded into the process by GTK.
        let egl = unsafe { Library::new("libEGL.so.1") }.map_err(|e| e.to_string())?;
        // SAFETY: as above.
        let wl_egl = unsafe { Library::new("libwayland-egl.so.1") }.map_err(|e| e.to_string())?;
        macro_rules! sym {
            ($lib:expr, $name:literal) => {
                // SAFETY: the field types match the EGL 1.4 / wayland-egl C prototypes.
                *unsafe { $lib.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| format!("{}: {e}", $name))?
            };
        }
        Ok(Self {
            get_display: sym!(egl, "eglGetDisplay"),
            initialize: sym!(egl, "eglInitialize"),
            bind_api: sym!(egl, "eglBindAPI"),
            choose_config: sym!(egl, "eglChooseConfig"),
            get_config_attrib: sym!(egl, "eglGetConfigAttrib"),
            create_context: sym!(egl, "eglCreateContext"),
            create_window_surface: sym!(egl, "eglCreateWindowSurface"),
            make_current: sym!(egl, "eglMakeCurrent"),
            swap_buffers: sym!(egl, "eglSwapBuffers"),
            swap_interval: sym!(egl, "eglSwapInterval"),
            destroy_surface: sym!(egl, "eglDestroySurface"),
            destroy_context: sym!(egl, "eglDestroyContext"),
            get_proc_address: sym!(egl, "eglGetProcAddress"),
            wl_egl_window_create: sym!(wl_egl, "wl_egl_window_create"),
            wl_egl_window_resize: sym!(wl_egl, "wl_egl_window_resize"),
            wl_egl_window_destroy: sym!(wl_egl, "wl_egl_window_destroy"),
            _egl: egl,
            _wl_egl: wl_egl,
        })
    }
}

unsafe extern "C" fn egl_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `ctx` is the render thread's `Egl`, alive for as long as the mpv render context.
    let egl = unsafe { &*(ctx as *const Egl) };
    // SAFETY: `name` is a NUL-terminated string from mpv.
    unsafe { (egl.get_proc_address)(name) }
}

// ---------------------------------------------------------------------------------------------
// Wayland state (dispatched on our own event queue, sharing GTK's connection).

#[derive(Default)]
struct Caps {
    parametric: bool,
    luminances: bool,
    mastering: bool,
    pq: bool,
    bt2020: bool,
    perceptual: bool,
    done: bool,
}

impl Caps {
    fn usable(&self) -> bool {
        self.parametric && self.pq && self.bt2020 && self.perceptual
    }
}

/// What the compositor says an output is targeting (from its image description).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct OutputInfo {
    /// The output is in an HDR mode (PQ or HLG signalling).
    hdr: bool,
    /// Target peak luminance, nits (0 = not reported).
    peak: u32,
}

#[derive(Default)]
struct WlState {
    caps: Caps,
    /// Our `wl_output` proxies by protocol id -> latest known info.
    outputs: HashMap<ObjectId, OutputInfo>,
    /// Info being collected per output (between `get_information` and `done`).
    pending_info: HashMap<ObjectId, OutputInfo>,
    /// Outputs the video surface is on, most recent last.
    entered: Vec<ObjectId>,
    /// Output whose image description should be (re)queried.
    requery: Vec<ObjectId>,
    /// Our PQ image descriptions: Some(true) ready, Some(false) failed.
    desc_ready: HashMap<ObjectId, bool>,
}

impl WlState {
    /// HDR target of the output the video is on, if it is in an HDR mode.
    fn current_output(&self) -> Option<OutputInfo> {
        self.entered
            .last()
            .and_then(|o| self.outputs.get(o))
            .copied()
    }
}

/// User data for per-output protocol objects: which `wl_output` they describe.
#[derive(Clone)]
struct OutputKey(ObjectId);

impl Dispatch<WlRegistry, GlobalListContents> for WlState {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for WlState {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(
    WlCompositor,
    WlSubcompositor,
    WlSubsurface,
    WlRegion,
    WlOutput,
    WpColorManagementSurfaceV1,
    WpImageDescriptionCreatorParamsV1
);

impl Dispatch<WlSurface, ()> for WlState {
    fn event(
        st: &mut Self,
        _: &WlSurface,
        ev: <WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_surface::Event;
        match ev {
            Event::Enter { output } => {
                let id = output.id();
                // Enter can name GTK's own wl_output proxies too; only ours carry our state.
                if st.outputs.contains_key(&id) {
                    st.entered.retain(|o| o != &id);
                    st.entered.push(id);
                }
            }
            Event::Leave { output } => {
                let id = output.id();
                st.entered.retain(|o| o != &id);
            }
            _ => {}
        }
    }
}

impl Dispatch<WpColorManagerV1, ()> for WlState {
    fn event(
        st: &mut Self,
        _: &WpColorManagerV1,
        ev: wp_color_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wp_color_manager_v1::Event;
        match ev {
            Event::SupportedIntent {
                render_intent: WEnum::Value(RenderIntent::Perceptual),
            } => st.caps.perceptual = true,
            Event::SupportedFeature {
                feature: WEnum::Value(f),
            } => match f {
                Feature::Parametric => st.caps.parametric = true,
                Feature::SetLuminances => st.caps.luminances = true,
                Feature::SetMasteringDisplayPrimaries => st.caps.mastering = true,
                _ => {}
            },
            Event::SupportedTfNamed {
                tf: WEnum::Value(TransferFunction::St2084Pq),
            } => st.caps.pq = true,
            Event::SupportedPrimariesNamed {
                primaries: WEnum::Value(Primaries::Bt2020),
            } => st.caps.bt2020 = true,
            Event::Done => st.caps.done = true,
            _ => {}
        }
    }
}

impl Dispatch<WpColorManagementOutputV1, OutputKey> for WlState {
    fn event(
        st: &mut Self,
        _: &WpColorManagementOutputV1,
        ev: wp_color_management_output_v1::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_color_management_output_v1::Event::ImageDescriptionChanged = ev {
            st.requery.push(key.0.clone());
        }
    }
}

/// Image descriptions are created for two reasons: an output's (user data `Some(output)`), to read
/// its target, and our own PQ description (`None`), to tag the surface.
impl Dispatch<WpImageDescriptionV1, Option<OutputKey>> for WlState {
    fn event(
        st: &mut Self,
        desc: &WpImageDescriptionV1,
        ev: wp_image_description_v1::Event,
        key: &Option<OutputKey>,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wp_image_description_v1::Event;
        let ok = match ev {
            Event::Ready { .. } | Event::Ready2 { .. } => true,
            Event::Failed { .. } => false,
            _ => return,
        };
        match key {
            Some(out) => {
                if ok {
                    st.pending_info.insert(out.0.clone(), OutputInfo::default());
                    desc.get_information(qh, out.clone());
                }
                desc.destroy();
            }
            None => {
                st.desc_ready.insert(desc.id(), ok);
            }
        }
    }
}

impl Dispatch<WpImageDescriptionInfoV1, OutputKey> for WlState {
    fn event(
        st: &mut Self,
        _: &WpImageDescriptionInfoV1,
        ev: wp_image_description_info_v1::Event,
        key: &OutputKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wp_image_description_info_v1::Event;
        let Some(info) = st.pending_info.get_mut(&key.0) else {
            return;
        };
        match ev {
            Event::TfNamed { tf } => {
                info.hdr = matches!(
                    tf,
                    WEnum::Value(TransferFunction::St2084Pq | TransferFunction::Hlg)
                );
            }
            Event::TargetLuminance { max_lum, .. } => info.peak = max_lum,
            Event::Luminances { max_lum, .. } if info.peak == 0 => info.peak = max_lum,
            Event::Done => {
                if let Some(done) = st.pending_info.remove(&key.0) {
                    tracing::info!(
                        "HDR surface: output mode hdr={} peak={} nits",
                        done.hdr,
                        done.peak
                    );
                    st.outputs.insert(key.0.clone(), done);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The surface.

struct Wl {
    conn: Connection,
    queue: EventQueue<WlState>,
    qh: QueueHandle<WlState>,
    state: WlState,
    manager: WpColorManagerV1,
    surface: WlSurface,
    subsurface: WlSubsurface,
    cm_surface: WpColorManagementSurfaceV1,
    /// Per-output colour-management objects, kept alive for `image_description_changed`.
    cm_outputs: Vec<(WlOutput, WpColorManagementOutputV1)>,
}

impl Wl {
    fn query_output(&mut self, id: &ObjectId) {
        let Some((_, cm)) = self.cm_outputs.iter().find(|(o, _)| &o.id() == id) else {
            return;
        };
        cm.get_image_description(&self.qh, Some(OutputKey(id.clone())));
    }
}

/// Geometry in surface-local logical pixels plus the integer buffer scale.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Geometry {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: i32,
}

struct Shared {
    geometry: Geometry,
    visible: bool,
    engine: Arc<Engine>,
    /// mpv asked for a redraw, or something changed that needs one.
    dirty: bool,
    quit: bool,
}

struct Sync {
    m: Mutex<Shared>,
    cv: Condvar,
}

impl Sync {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        match self.m.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn poke(&self, f: impl FnOnce(&mut Shared)) {
        let mut s = self.lock();
        f(&mut s);
        s.dirty = true;
        self.cv.notify_all();
    }
}

/// Main-thread handle to one HDR video subsurface and its render thread.
pub(crate) struct HdrSurface {
    sync: Arc<Sync>,
    thread: Option<JoinHandle<()>>,
    subsurface: WlSubsurface,
}

impl HdrSurface {
    /// Set up the subsurface under `parent` (the window's `wl_surface`) on GTK's `wl_display`.
    /// Returns `None` (after logging why) when HDR output is not possible here; the caller then
    /// uses the GtkGLArea path.
    ///
    /// # Safety
    /// `wl_display` and `parent` must be GTK's live `wl_display` / toplevel `wl_surface`, and stay
    /// valid until this surface is dropped.
    pub(crate) unsafe fn new(
        wl_display: *mut c_void,
        parent: *mut c_void,
        engine: Arc<Engine>,
        frames: Arc<std::sync::atomic::AtomicU64>,
    ) -> Option<Self> {
        // SAFETY: caller guarantees both pointers (see above).
        match unsafe { Self::try_new(wl_display, parent, engine, frames) } {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::info!("HDR video surface not used: {e}");
                None
            }
        }
    }

    unsafe fn try_new(
        wl_display: *mut c_void,
        parent: *mut c_void,
        engine: Arc<Engine>,
        frames: Arc<std::sync::atomic::AtomicU64>,
    ) -> Result<Self, String> {
        // SAFETY: GTK's display outlives us (caller contract); guest mode never closes it.
        let backend = unsafe { Backend::from_foreign_display(wl_display.cast()) };
        let conn = Connection::from_backend(backend);
        let (globals, mut queue) =
            registry_queue_init::<WlState>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let manager: WpColorManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "compositor has no wp_color_manager_v1".to_owned())?;
        let compositor: WlCompositor = globals
            .bind(&qh, 4..=4, ())
            .map_err(|e| format!("wl_compositor: {e}"))?;
        let subcompositor: WlSubcompositor = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("wl_subcompositor: {e}"))?;
        let outputs: Vec<WlOutput> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == WlOutput::interface().name)
                .map(|g| {
                    globals
                        .registry()
                        .bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, ())
                })
                .collect()
        });
        let mut state = WlState::default();
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("roundtrip: {e}"))?;
        if !state.caps.usable() {
            return Err(format!(
                "colour management lacks what HDR needs (parametric={} pq={} bt2020={} perceptual={})",
                state.caps.parametric, state.caps.pq, state.caps.bt2020, state.caps.perceptual
            ));
        }

        // SAFETY: `parent` is GTK's live toplevel wl_surface (caller contract).
        let parent_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), parent.cast()) }
            .map_err(|_| "parent is not a wl_surface".to_owned())?;
        let parent = WlSurface::from_id(&conn, parent_id).map_err(|e| e.to_string())?;

        let surface = compositor.create_surface(&qh, ());
        let region = compositor.create_region(&qh, ());
        surface.set_input_region(Some(&region)); // empty: clicks go to the webview
        region.destroy();
        let subsurface = subcompositor.get_subsurface(&surface, &parent, &qh, ());
        subsurface.place_below(&parent);
        subsurface.set_desync();
        let cm_surface = manager.get_surface(&surface, &qh, ());

        let mut cm_outputs = Vec::new();
        for o in outputs {
            state.outputs.insert(o.id(), OutputInfo::default());
            let cm = manager.get_output(&o, &qh, OutputKey(o.id()));
            cm_outputs.push((o, cm));
        }
        let mut wl = Wl {
            conn,
            queue,
            qh,
            state,
            manager,
            surface,
            subsurface: subsurface.clone(),
            cm_surface,
            cm_outputs,
        };
        let ids: Vec<ObjectId> = wl.cm_outputs.iter().map(|(o, _)| o.id()).collect();
        for id in &ids {
            wl.query_output(id);
        }
        let _ = wl.conn.flush();

        let sync = Arc::new(Sync {
            m: Mutex::new(Shared {
                geometry: Geometry::default(),
                visible: true,
                engine,
                dirty: true,
                quit: false,
            }),
            cv: Condvar::new(),
        });
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let display_addr = wl_display as usize; // raw pointers are not Send
        let thread = {
            let sync = sync.clone();
            std::thread::Builder::new()
                .name("hdr-video".into())
                .spawn(move || render_thread(wl, display_addr, sync, frames, ready_tx))
                .map_err(|e| e.to_string())?
        };
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                sync,
                thread: Some(thread),
                subsurface,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                sync.poke(|s| s.quit = true);
                Err("render thread did not start".into())
            }
        }
    }

    /// Position (surface-local logical px of the window) and size (logical px) at `scale`.
    pub(crate) fn set_geometry(&self, x: i32, y: i32, w: i32, h: i32, scale: i32) {
        let g = Geometry {
            x,
            y,
            w: w.max(1),
            h: h.max(1),
            scale: scale.max(1),
        };
        // Position is parent state: it applies on GTK's next commit of the window.
        self.subsurface.set_position(g.x, g.y);
        self.sync.poke(|s| s.geometry = g);
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.sync.poke(|s| s.visible = visible);
    }

    /// Replace the engine after the supervisor restarted mpv; the thread rebuilds its context.
    pub(crate) fn rebind(&self, engine: Arc<Engine>) {
        self.sync.poke(|s| s.engine = engine);
    }
}

impl Drop for HdrSurface {
    fn drop(&mut self) {
        self.sync.poke(|s| s.quit = true);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// HDR state mpv is currently rendering for, so properties are only touched on change.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Sdr,
    Hdr { peak: u32 },
}

fn content_is_hdr(engine: &Engine) -> bool {
    matches!(
        engine.get_property("video-params/gamma").as_deref(),
        Some("pq" | "hlg")
    )
}

/// Pick the render mode. The user's setting wins over everything: with HDR output off the
/// surface always renders SDR (mpv tone-maps HDR sources) and carries no image description.
fn choose_mode(user_auto: bool, output: Option<OutputInfo>, content_hdr: bool) -> Mode {
    match output {
        Some(out) if user_auto && out.hdr && content_hdr => Mode::Hdr {
            peak: if out.peak > 0 {
                out.peak
            } else {
                DEFAULT_PEAK_NITS
            },
        },
        _ => Mode::Sdr,
    }
}

fn apply_mode(engine: &Engine, mode: Mode) {
    let set = |k: &str, v: &str| {
        if let Err(e) = engine.set_property(k, v) {
            tracing::warn!("HDR surface: {k}={v} not applied: {e}");
        }
    };
    match mode {
        Mode::Sdr => {
            // Explicit SDR target: mpv tone-maps PQ/HLG sources instead of passing them through.
            set("target-colorspace-hint", "no");
            set("target-trc", "auto");
            set("target-prim", "auto");
            set("target-peak", "auto");
        }
        Mode::Hdr { peak } => {
            set("target-trc", "pq");
            set("target-prim", "bt.2020");
            set("target-peak", &peak.to_string());
        }
    }
}

/// Create our PQ image description for an output peaking at `peak` nits and wait until it is ready.
fn create_pq_description(wl: &mut Wl, peak: u32) -> Option<WpImageDescriptionV1> {
    let creator = wl.manager.create_parametric_creator(&wl.qh, ());
    creator.set_tf_named(TransferFunction::St2084Pq);
    creator.set_primaries_named(Primaries::Bt2020);
    if wl.state.caps.luminances {
        // min 0.005 nits; max is ignored for PQ (min + 10000).
        creator.set_luminances(50, 10_000, REFERENCE_WHITE_NITS);
    }
    if wl.state.caps.mastering {
        // mpv tone-maps to the output's peak, so that is the content's target volume.
        creator.set_mastering_luminance(50, peak);
    }
    let desc = creator.create(&wl.qh, None);
    for _ in 0..50 {
        if let Some(ok) = wl.state.desc_ready.get(&desc.id()).copied() {
            wl.state.desc_ready.remove(&desc.id());
            if ok {
                return Some(desc);
            }
            desc.destroy();
            tracing::warn!("HDR surface: compositor rejected the PQ image description");
            return None;
        }
        if wl.queue.roundtrip(&mut wl.state).is_err() {
            return None;
        }
    }
    None
}

struct Gl {
    egl: Box<Egl>,
    dpy: EglDisplay,
    ctx: EglContext,
    surf: EglSurface,
    window: *mut c_void,
    size: (i32, i32),
}

impl Gl {
    unsafe fn new(wl_display: *mut c_void, surface: *mut c_void) -> Result<Self, String> {
        let egl = Box::new(Egl::load()?);
        // SAFETY: all calls below follow the EGL 1.4 / wayland-egl contracts with valid handles;
        // `wl_display`/`surface` are live (GTK's display, our wl_surface).
        unsafe {
            let dpy = (egl.get_display)(wl_display);
            if dpy.is_null() {
                return Err("eglGetDisplay failed".into());
            }
            // GTK initialised this display already; this just takes another reference.
            if (egl.initialize)(dpy, std::ptr::null_mut(), std::ptr::null_mut()) == 0 {
                return Err("eglInitialize failed".into());
            }
            if (egl.bind_api)(EGL_OPENGL_API) == 0 {
                return Err("eglBindAPI(OpenGL) failed".into());
            }
            let attrs = [
                EGL_SURFACE_TYPE,
                EGL_WINDOW_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_BIT,
                EGL_RED_SIZE,
                10,
                EGL_GREEN_SIZE,
                10,
                EGL_BLUE_SIZE,
                10,
                EGL_ALPHA_SIZE,
                0,
                EGL_NONE,
            ];
            let mut configs = [std::ptr::null_mut(); 32];
            let mut n = 0;
            if (egl.choose_config)(dpy, attrs.as_ptr(), configs.as_mut_ptr(), 32, &mut n) == 0 {
                return Err("eglChooseConfig failed".into());
            }
            // Exactly 10 bits per channel (the list also holds deeper/float configs).
            let config = configs[..n.max(0) as usize]
                .iter()
                .copied()
                .find(|&c| {
                    let mut r = 0;
                    (egl.get_config_attrib)(dpy, c, EGL_RED_SIZE, &mut r);
                    r == 10
                })
                .ok_or("no 10-bit EGL window config")?;
            let ctx = (egl.create_context)(dpy, config, std::ptr::null_mut(), [EGL_NONE].as_ptr());
            if ctx.is_null() {
                return Err("eglCreateContext failed".into());
            }
            let window = (egl.wl_egl_window_create)(surface, 1, 1);
            if window.is_null() {
                (egl.destroy_context)(dpy, ctx);
                return Err("wl_egl_window_create failed".into());
            }
            let surf = (egl.create_window_surface)(dpy, config, window, [EGL_NONE].as_ptr());
            if surf.is_null() {
                (egl.wl_egl_window_destroy)(window);
                (egl.destroy_context)(dpy, ctx);
                return Err("eglCreateWindowSurface failed".into());
            }
            if (egl.make_current)(dpy, surf, surf, ctx) == 0 {
                (egl.destroy_surface)(dpy, surf);
                (egl.wl_egl_window_destroy)(window);
                (egl.destroy_context)(dpy, ctx);
                return Err("eglMakeCurrent failed".into());
            }
            // mpv paces frames (block_for_target_time); never block on frame callbacks, which stop
            // while the window is hidden and would wedge shutdown.
            (egl.swap_interval)(dpy, 0);
            Ok(Self {
                egl,
                dpy,
                ctx,
                surf,
                window,
                size: (1, 1),
            })
        }
    }

    fn resize(&mut self, w: i32, h: i32) {
        if self.size != (w, h) {
            // SAFETY: window is our live wl_egl_window.
            unsafe { (self.egl.wl_egl_window_resize)(self.window, w, h, 0, 0) };
            self.size = (w, h);
        }
    }

    fn swap(&self) {
        // SAFETY: display/surface valid and current on this thread.
        unsafe { (self.egl.swap_buffers)(self.dpy, self.surf) };
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        // SAFETY: handles are ours and destroyed once; the display stays initialised for GTK.
        unsafe {
            (self.egl.make_current)(
                self.dpy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            (self.egl.destroy_surface)(self.dpy, self.surf);
            (self.egl.wl_egl_window_destroy)(self.window);
            (self.egl.destroy_context)(self.dpy, self.ctx);
        }
    }
}

fn make_render_context(
    gl: &Gl,
    engine: &Arc<Engine>,
    wl_display: *mut c_void,
    sync: &Arc<Sync>,
) -> Option<RenderContext> {
    let ctx = (&*gl.egl as *const Egl).cast_mut().cast::<c_void>();
    // SAFETY: the EGL context is current on this thread and `gl.egl` outlives the render context
    // (both owned by the render thread, context dropped first).
    match unsafe {
        engine.create_render_context(egl_proc_address, ctx, wl_display, std::ptr::null_mut())
    } {
        Ok(mut rc) => {
            let sync = sync.clone();
            rc.set_update_callback(move || sync.poke(|_| {}));
            Some(rc)
        }
        Err(e) => {
            tracing::error!("HDR surface: mpv_render_context_create failed: {e}");
            None
        }
    }
}

fn render_thread(
    mut wl: Wl,
    wl_display: usize,
    sync: Arc<Sync>,
    frames: Arc<std::sync::atomic::AtomicU64>,
    ready: mpsc::Sender<Result<(), String>>,
) {
    let wl_display = wl_display as *mut c_void;
    let surface_ptr = wl.surface.id().as_ptr().cast::<c_void>();
    // SAFETY: GTK's display and our surface are live for the whole thread.
    let mut gl = match unsafe { Gl::new(wl_display, surface_ptr) } {
        Ok(gl) => gl,
        Err(e) => {
            let _ = ready.send(Err(e));
            teardown(&wl);
            return;
        }
    };
    let mut engine = sync.lock().engine.clone();
    let mut rc = make_render_context(&gl, &engine, wl_display, &sync);
    if rc.is_none() {
        let _ = ready.send(Err("could not create the mpv render context".into()));
        drop(gl);
        teardown(&wl);
        return;
    }
    tracing::info!("HDR video surface active (Wayland subsurface, 10-bit EGL)");
    let _ = ready.send(Ok(()));

    let mut mode = Mode::Sdr;
    let mut tagged: Option<(u32, WpImageDescriptionV1)> = None;
    let mut shown = true;
    loop {
        let (geometry, visible, eng, quit) = {
            let mut s = sync.lock();
            if !s.dirty && !s.quit {
                s = match sync.cv.wait_timeout(s, Duration::from_millis(100)) {
                    Ok((g, _)) => g,
                    Err(p) => p.into_inner().0,
                };
            }
            s.dirty = false;
            (s.geometry, s.visible, s.engine.clone(), s.quit)
        };
        if quit {
            break;
        }
        // Output mode changes (HDR toggled in the desktop settings) arrive on our queue.
        let _ = wl.queue.dispatch_pending(&mut wl.state);
        for id in std::mem::take(&mut wl.state.requery) {
            wl.query_output(&id);
        }
        let _ = wl.conn.flush();

        if !Arc::ptr_eq(&eng, &engine) {
            // Supervisor replaced mpv: free the old context (EGL current) and bind the new engine.
            drop(rc.take());
            engine = eng;
            rc = make_render_context(&gl, &engine, wl_display, &sync);
            mode = Mode::Sdr;
        }
        let Some(ctx) = rc.as_ref() else { continue };

        if !visible {
            if shown {
                // Unmap: a null buffer hides the subsurface until the next swap.
                wl.surface.attach(None, 0, 0);
                wl.surface.commit();
                let _ = wl.conn.flush();
                shown = false;
            }
            ctx.update();
            continue;
        }
        shown = true;

        let want = choose_mode(
            super::hdr_output_auto(),
            wl.state.current_output(),
            content_is_hdr(&engine),
        );
        if want != mode {
            tracing::info!("HDR surface: rendering {want:?}");
            apply_mode(&engine, want);
            mode = want;
        }
        // Surface tag for this frame (double-buffered, applied by the swap's commit).
        match mode {
            Mode::Hdr { peak } if tagged.as_ref().map(|(p, _)| *p) != Some(peak) => {
                if let Some((_, old)) = tagged.take() {
                    old.destroy();
                }
                if let Some(desc) = create_pq_description(&mut wl, peak) {
                    wl.cm_surface
                        .set_image_description(&desc, RenderIntent::Perceptual);
                    tagged = Some((peak, desc));
                }
            }
            Mode::Sdr if tagged.is_some() => {
                wl.cm_surface.unset_image_description();
                if let Some((_, old)) = tagged.take() {
                    old.destroy();
                }
            }
            _ => {}
        }

        let scale = geometry.scale.max(1);
        let (pw, ph) = (geometry.w.max(1) * scale, geometry.h.max(1) * scale);
        gl.resize(pw, ph);
        wl.surface.set_buffer_scale(scale);
        ctx.update();
        if let Err(e) = ctx.render_with_format(0, pw, ph, GL_RGB10_A2, true, true) {
            tracing::warn!("HDR surface: mpv render failed: {e}");
        }
        gl.swap();
        ctx.report_swap();
        frames.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    if let Some((_, d)) = tagged.take() {
        d.destroy();
    }
    if mode != Mode::Sdr {
        // The engine outlives this surface; do not leave it targeting PQ for an SDR surface.
        apply_mode(&engine, Mode::Sdr);
    }
    drop(rc); // with the EGL context still current
    drop(gl);
    teardown(&wl);
}

fn teardown(wl: &Wl) {
    for (_, cm) in &wl.cm_outputs {
        cm.destroy();
    }
    for (o, _) in &wl.cm_outputs {
        if o.version() >= 3 {
            o.release();
        }
    }
    wl.cm_surface.destroy();
    wl.subsurface.destroy();
    wl.surface.destroy();
    wl.manager.destroy();
    let _ = wl.conn.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(hdr: bool, peak: u32) -> Option<OutputInfo> {
        Some(OutputInfo { hdr, peak })
    }

    #[test]
    fn hdr_only_when_setting_output_and_content_agree() {
        assert_eq!(
            choose_mode(true, out(true, 800), true),
            Mode::Hdr { peak: 800 }
        );
        assert_eq!(
            choose_mode(true, out(true, 0), true),
            Mode::Hdr {
                peak: DEFAULT_PEAK_NITS
            }
        );
        assert_eq!(choose_mode(true, out(true, 800), false), Mode::Sdr);
        assert_eq!(choose_mode(true, out(false, 800), true), Mode::Sdr);
        assert_eq!(choose_mode(true, None, true), Mode::Sdr);
    }

    #[test]
    fn setting_off_always_tone_maps() {
        assert_eq!(choose_mode(false, out(true, 1000), true), Mode::Sdr);
    }
}
