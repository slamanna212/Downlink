//! Safe wrapper over one libmpv handle and its render context.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::Arc;

use crate::error::PlayerError;
use crate::ffi::{self, MpvApi, MpvHandle, MpvRenderCtx};
use crate::loader::LoadedMpv;

/// Owned copy of the property value in a `PROPERTY_CHANGE` event.
#[derive(Debug, Clone, PartialEq)]
pub enum PropValue {
    None,
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(String),
}

/// Owned mpv event (only the ones Downlink handles).
#[derive(Debug, Clone, PartialEq)]
pub enum EngineEvent {
    None,
    Shutdown,
    StartFile,
    FileLoaded,
    PlaybackRestart,
    VideoReconfig,
    Seek,
    EndFile {
        reason: EndReason,
        error: String,
    },
    Property {
        name: String,
        value: PropValue,
    },
    Log {
        level: String,
        prefix: String,
        text: String,
    },
    Other(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
    Unknown,
}

/// One libmpv instance. Cheap to share: libmpv's client API is thread-safe, except that
/// `wait_event` must only be called from a single thread (the player's event thread).
pub struct Engine {
    api: Arc<MpvApi>,
    handle: *mut MpvHandle,
}

// SAFETY: libmpv documents its client API as thread-safe; the raw handle is only used through
// `MpvApi` functions, and `wait_event` is confined to one thread by `Player`.
unsafe impl Send for Engine {}
// SAFETY: see above.
unsafe impl Sync for Engine {}

fn cstr(s: &str) -> Result<CString, PlayerError> {
    CString::new(s).map_err(|_| PlayerError::InvalidArgument("string contains NUL".into()))
}

impl Engine {
    /// Create, configure (`options` before init) and initialise an mpv instance.
    pub fn new(loaded: &LoadedMpv, options: &[(String, String)]) -> Result<Self, PlayerError> {
        let api = loaded.api.clone();
        force_c_numeric_locale();
        // SAFETY: `mpv_create` has no preconditions; a null return is handled.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(PlayerError::Mpv {
                call: "mpv_create",
                message: "returned null".into(),
            });
        }
        let engine = Self { api, handle };
        for (k, v) in options {
            if let Err(e) = engine.set_option(k, v) {
                // A bad option must not stop playback entirely (version differences).
                tracing::warn!("mpv option rejected: {e}");
            }
        }
        // SAFETY: `handle` is a valid, not yet initialised mpv handle.
        let rc = unsafe { (engine.api.initialize)(engine.handle) };
        engine.check("mpv_initialize", rc)?;
        Ok(engine)
    }

    fn err_text(&self, rc: c_int) -> String {
        // SAFETY: `mpv_error_string` returns a pointer to a static NUL-terminated string.
        let p = unsafe { (self.api.error_string)(rc) };
        if p.is_null() {
            return format!("error {rc}");
        }
        // SAFETY: non-null and NUL-terminated static string.
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    fn check(&self, call: &'static str, rc: c_int) -> Result<(), PlayerError> {
        if rc >= 0 {
            Ok(())
        } else {
            Err(PlayerError::Mpv {
                call,
                message: self.err_text(rc),
            })
        }
    }

    pub fn api(&self) -> &Arc<MpvApi> {
        &self.api
    }

    pub fn raw_handle(&self) -> *mut MpvHandle {
        self.handle
    }

    /// Set an option (valid before and, for most options, after init).
    pub fn set_option(&self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: handle is valid; both strings are NUL-terminated and outlive the call.
        let rc = unsafe { (self.api.set_option_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.check("mpv_set_option_string", rc)
            .map_err(|e| PlayerError::InvalidArgument(format!("option `{name}`: {e}")))
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), PlayerError> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: as in `set_option`.
        let rc = unsafe { (self.api.set_property_string)(self.handle, n.as_ptr(), v.as_ptr()) };
        self.check("mpv_set_property_string", rc)
    }

    pub fn get_property(&self, name: &str) -> Option<String> {
        let n = cstr(name).ok()?;
        // SAFETY: handle valid; `n` is NUL-terminated. A non-null result must be freed with
        // `mpv_free`, which we do after copying.
        let p = unsafe { (self.api.get_property_string)(self.handle, n.as_ptr()) };
        if p.is_null() {
            return None;
        }
        // SAFETY: non-null NUL-terminated string owned by mpv until `mpv_free`.
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        // SAFETY: `p` was returned by mpv_get_property_string and is freed exactly once.
        unsafe { (self.api.free)(p.cast::<c_void>()) };
        Some(s)
    }

    /// `mpv_command` with string arguments.
    pub fn command(&self, args: &[&str]) -> Result<(), PlayerError> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        // SAFETY: handle valid; `ptrs` is a NULL-terminated array of NUL-terminated strings kept
        // alive by `owned` for the duration of the call.
        let rc = unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) };
        self.check("mpv_command", rc)
    }

    pub fn observe(&self, id: u64, name: &str, format: c_int) -> Result<(), PlayerError> {
        let n = cstr(name)?;
        // SAFETY: handle valid; `n` NUL-terminated and copied by mpv.
        let rc = unsafe { (self.api.observe_property)(self.handle, id, n.as_ptr(), format) };
        self.check("mpv_observe_property", rc)
    }

    pub fn request_log_messages(&self, level: &str) -> Result<(), PlayerError> {
        let l = cstr(level)?;
        // SAFETY: handle valid; `l` NUL-terminated and copied.
        let rc = unsafe { (self.api.request_log_messages)(self.handle, l.as_ptr()) };
        self.check("mpv_request_log_messages", rc)
    }

    /// Wake a blocked `wait_event` (used to stop the event thread).
    pub fn wakeup(&self) {
        // SAFETY: handle valid; `mpv_wakeup` is thread-safe.
        unsafe { (self.api.wakeup)(self.handle) };
    }

    /// Wait up to `timeout_secs` for an event. Call from one thread only.
    pub fn wait_event(&self, timeout_secs: f64) -> EngineEvent {
        // SAFETY: handle valid. The returned event pointer is valid until the next
        // `mpv_wait_event` call on this handle, and we copy everything out before returning.
        let ev = unsafe { (self.api.wait_event)(self.handle, timeout_secs) };
        if ev.is_null() {
            return EngineEvent::None;
        }
        // SAFETY: non-null per above.
        let ev = unsafe { &*ev };
        match ev.event_id {
            ffi::EVENT_NONE => EngineEvent::None,
            ffi::EVENT_SHUTDOWN => EngineEvent::Shutdown,
            ffi::EVENT_START_FILE => EngineEvent::StartFile,
            ffi::EVENT_FILE_LOADED => EngineEvent::FileLoaded,
            ffi::EVENT_PLAYBACK_RESTART => EngineEvent::PlaybackRestart,
            ffi::EVENT_VIDEO_RECONFIG => EngineEvent::VideoReconfig,
            ffi::EVENT_SEEK => EngineEvent::Seek,
            ffi::EVENT_END_FILE if !ev.data.is_null() => {
                // SAFETY: for END_FILE, `data` points to an `mpv_event_end_file`.
                let e = unsafe { &*(ev.data as *const ffi::MpvEventEndFile) };
                let reason = match e.reason {
                    ffi::END_FILE_EOF => EndReason::Eof,
                    ffi::END_FILE_STOP => EndReason::Stop,
                    ffi::END_FILE_QUIT => EndReason::Quit,
                    ffi::END_FILE_ERROR => EndReason::Error,
                    ffi::END_FILE_REDIRECT => EndReason::Redirect,
                    _ => EndReason::Unknown,
                };
                let error = if e.error < 0 {
                    self.err_text(e.error)
                } else {
                    String::new()
                };
                EngineEvent::EndFile { reason, error }
            }
            ffi::EVENT_PROPERTY_CHANGE if !ev.data.is_null() => {
                // SAFETY: for PROPERTY_CHANGE, `data` points to an `mpv_event_property`.
                let p = unsafe { &*(ev.data as *const ffi::MpvEventProperty) };
                // SAFETY: `name` is a NUL-terminated string valid for this event.
                let name = unsafe { CStr::from_ptr(p.name) }
                    .to_string_lossy()
                    .into_owned();
                let value = if p.data.is_null() {
                    PropValue::None
                } else {
                    match p.format {
                        // SAFETY: format tells us the pointee type of `data`.
                        ffi::FORMAT_FLAG => {
                            PropValue::Flag(unsafe { *(p.data as *const c_int) } != 0)
                        }
                        // SAFETY: as above.
                        ffi::FORMAT_INT64 => PropValue::Int(unsafe { *(p.data as *const i64) }),
                        // SAFETY: as above.
                        ffi::FORMAT_DOUBLE => PropValue::Double(unsafe { *(p.data as *const f64) }),
                        ffi::FORMAT_STRING => {
                            // SAFETY: `data` points to a `char*` that may be null.
                            let sp = unsafe { *(p.data as *const *const c_char) };
                            if sp.is_null() {
                                PropValue::None
                            } else {
                                // SAFETY: non-null NUL-terminated string.
                                PropValue::Str(
                                    unsafe { CStr::from_ptr(sp) }.to_string_lossy().into_owned(),
                                )
                            }
                        }
                        _ => PropValue::None,
                    }
                };
                EngineEvent::Property { name, value }
            }
            ffi::EVENT_LOG_MESSAGE if !ev.data.is_null() => {
                // SAFETY: for LOG_MESSAGE, `data` points to an `mpv_event_log_message`.
                let m = unsafe { &*(ev.data as *const ffi::MpvEventLogMessage) };
                let s = |p: *const c_char| {
                    if p.is_null() {
                        String::new()
                    } else {
                        // SAFETY: non-null NUL-terminated string valid for this event.
                        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
                    }
                };
                EngineEvent::Log {
                    level: s(m.level),
                    prefix: s(m.prefix),
                    text: s(m.text),
                }
            }
            other => EngineEvent::Other(other),
        }
    }

    /// Create an OpenGL render context. Must be called with the GL context current.
    ///
    /// # Safety
    /// `get_proc_address` must be a valid loader for the current GL context for as long as the
    /// returned [`RenderContext`] lives; `wl_display`/`x11_display` must be null or valid.
    pub unsafe fn create_render_context(
        self: &Arc<Self>,
        get_proc_address: ffi::GetProcAddressFn,
        get_proc_address_ctx: *mut c_void,
        wl_display: *mut c_void,
        x11_display: *mut c_void,
    ) -> Result<RenderContext, PlayerError> {
        let mut init = ffi::MpvOpenGlInitParams {
            get_proc_address,
            get_proc_address_ctx,
        };
        let api_type = c"opengl";
        let mut params: Vec<ffi::MpvRenderParam> = vec![
            ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_API_TYPE,
                data: api_type.as_ptr() as *mut c_void,
            },
            ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: (&raw mut init).cast::<c_void>(),
            },
        ];
        if !wl_display.is_null() {
            params.push(ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_WL_DISPLAY,
                data: wl_display,
            });
        }
        if !x11_display.is_null() {
            params.push(ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_X11_DISPLAY,
                data: x11_display,
            });
        }
        params.push(ffi::MpvRenderParam {
            kind: 0,
            data: ptr::null_mut(),
        });
        let mut ctx: *mut MpvRenderCtx = ptr::null_mut();
        // SAFETY: handle is valid; `params` is an INVALID-terminated array whose pointers
        // (`init`, `api_type`) outlive this call; mpv copies what it needs.
        let rc =
            unsafe { (self.api.render_context_create)(&mut ctx, self.handle, params.as_mut_ptr()) };
        self.check("mpv_render_context_create", rc)?;
        Ok(RenderContext {
            engine: self.clone(),
            ctx,
            callback: None,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: handle is valid and is destroyed exactly once, here. Render contexts hold an
        // `Arc<Engine>`, so none outlives this call.
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}

type UpdateCb = Box<dyn Fn() + Send + Sync>;

/// mpv render API context bound to a GL context. Free it with the GL context current.
pub struct RenderContext {
    engine: Arc<Engine>,
    ctx: *mut MpvRenderCtx,
    callback: Option<Box<UpdateCb>>,
}

// SAFETY: mpv's render API requires render/update calls from the GL thread but the context
// object itself may be moved between threads.
unsafe impl Send for RenderContext {}

unsafe extern "C" fn update_trampoline(data: *mut c_void) {
    // SAFETY: `data` is the `Box<UpdateCb>` we registered and keep alive until the callback is
    // cleared in `Drop`.
    let cb = unsafe { &*(data as *const UpdateCb) };
    cb();
}

impl RenderContext {
    /// Register the "a new frame is ready" callback (called from arbitrary mpv threads).
    pub fn set_update_callback(&mut self, cb: impl Fn() + Send + Sync + 'static) {
        let boxed: Box<UpdateCb> = Box::new(Box::new(cb));
        let data = (&*boxed as *const UpdateCb).cast_mut().cast::<c_void>();
        // SAFETY: ctx valid; `data` stays alive in `self.callback` until cleared in Drop.
        unsafe {
            (self.engine.api.render_context_set_update_callback)(
                self.ctx,
                Some(update_trampoline),
                data,
            );
        }
        self.callback = Some(boxed);
    }

    /// Returns true if a new frame should be rendered.
    pub fn update(&self) -> bool {
        // SAFETY: ctx valid.
        let flags = unsafe { (self.engine.api.render_context_update)(self.ctx) };
        flags & ffi::RENDER_UPDATE_FRAME != 0
    }

    /// Render into framebuffer `fbo` of size `w` x `h`. Call with the GL context current.
    pub fn render(&self, fbo: i32, w: i32, h: i32, flip_y: bool) -> Result<(), PlayerError> {
        let mut f = ffi::MpvOpenGlFbo {
            fbo,
            w,
            h,
            internal_format: 0x8058, // GL_RGBA8: GtkGLArea FBOs are RGBA8
        };
        let mut flip: c_int = c_int::from(flip_y);
        let mut params = [
            ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_OPENGL_FBO,
                data: (&raw mut f).cast::<c_void>(),
            },
            ffi::MpvRenderParam {
                kind: ffi::RENDER_PARAM_FLIP_Y,
                data: (&raw mut flip).cast::<c_void>(),
            },
            ffi::MpvRenderParam {
                kind: 0,
                data: ptr::null_mut(),
            },
        ];
        // SAFETY: ctx valid; params are INVALID-terminated and point at locals alive for the call.
        let rc = unsafe { (self.engine.api.render_context_render)(self.ctx, params.as_mut_ptr()) };
        self.engine.check("mpv_render_context_render", rc)
    }

    pub fn report_swap(&self) {
        // SAFETY: ctx valid.
        unsafe { (self.engine.api.render_context_report_swap)(self.ctx) };
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: ctx valid. The callback is cleared first so mpv no longer references the
        // boxed closure, then the context is freed exactly once.
        unsafe {
            (self.engine.api.render_context_set_update_callback)(self.ctx, None, ptr::null_mut());
            (self.engine.api.render_context_free)(self.ctx);
        }
        self.callback = None;
    }
}

/// libmpv refuses to run unless `LC_NUMERIC` is "C"; GTK and friends reset it from the environment.
fn force_c_numeric_locale() {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
        }
        const LC_NUMERIC: c_int = if cfg!(target_os = "macos") { 4 } else { 1 };
        // SAFETY: setlocale with a valid category and a NUL-terminated string; called during
        // setup on the main thread before other threads read locale state.
        unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
    }
}
