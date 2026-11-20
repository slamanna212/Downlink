//! Hand-written minimal libmpv FFI, resolved at runtime with `libloading`.
//!
//! There is no link-time dependency on libmpv: every symbol is looked up from the loaded library
//! (PLAN §4.10). Only the client API and the render API (OpenGL) are declared.

use std::ffi::{c_char, c_double, c_int, c_void};

use libloading::Library;

#[repr(C)]
pub struct MpvHandle {
    _private: [u8; 0],
}

#[repr(C)]
pub struct MpvRenderCtx {
    _private: [u8; 0],
}

// --- formats ---------------------------------------------------------------------------------
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

// --- event ids -------------------------------------------------------------------------------
pub const EVENT_NONE: c_int = 0;
pub const EVENT_SHUTDOWN: c_int = 1;
pub const EVENT_LOG_MESSAGE: c_int = 2;
pub const EVENT_START_FILE: c_int = 6;
pub const EVENT_END_FILE: c_int = 7;
pub const EVENT_FILE_LOADED: c_int = 8;
pub const EVENT_VIDEO_RECONFIG: c_int = 17;
pub const EVENT_SEEK: c_int = 20;
pub const EVENT_PLAYBACK_RESTART: c_int = 21;
pub const EVENT_PROPERTY_CHANGE: c_int = 22;

// --- end-file reasons ------------------------------------------------------------------------
pub const END_FILE_EOF: c_int = 0;
pub const END_FILE_STOP: c_int = 2;
pub const END_FILE_QUIT: c_int = 3;
pub const END_FILE_ERROR: c_int = 4;
pub const END_FILE_REDIRECT: c_int = 5;

// --- render params ---------------------------------------------------------------------------
pub const RENDER_PARAM_API_TYPE: c_int = 1;
pub const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
pub const RENDER_PARAM_OPENGL_FBO: c_int = 3;
pub const RENDER_PARAM_FLIP_Y: c_int = 4;
pub const RENDER_PARAM_X11_DISPLAY: c_int = 8;
pub const RENDER_PARAM_WL_DISPLAY: c_int = 9;
pub const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub const RENDER_UPDATE_FRAME: u64 = 1;

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct MpvEventEndFile {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[repr(C)]
pub struct MpvEventLogMessage {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
    pub log_level: c_int,
}

#[repr(C)]
pub struct MpvRenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

pub type GetProcAddressFn =
    unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void;

#[repr(C)]
pub struct MpvOpenGlInitParams {
    pub get_proc_address: GetProcAddressFn,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct MpvOpenGlFbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

pub type UpdateCallbackFn = unsafe extern "C" fn(ctx: *mut c_void);

macro_rules! api {
    ($( $field:ident : $name:literal => fn($($arg:ty),*) $(-> $ret:ty)? ),* $(,)?) => {
        /// Resolved libmpv entry points. Holds the [`Library`] so the pointers stay valid.
        pub struct MpvApi {
            _lib: Library,
            $( pub $field: unsafe extern "C" fn($($arg),*) $(-> $ret)?, )*
        }

        impl MpvApi {
            /// Resolve every required symbol from `lib`.
            pub(crate) fn from_library(lib: Library) -> Result<Self, libloading::Error> {
                $(
                    // SAFETY: the declared signature matches the libmpv C header for this symbol;
                    // the pointer is copied out and the Library is kept alive in `_lib`.
                    let $field: unsafe extern "C" fn($($arg),*) $(-> $ret)? =
                        *unsafe { lib.get(concat!($name, "\0").as_bytes())? };
                )*
                Ok(Self { _lib: lib, $( $field, )* })
            }
        }
    };
}

api! {
    client_api_version: "mpv_client_api_version" => fn() -> u64,
    create: "mpv_create" => fn() -> *mut MpvHandle,
    initialize: "mpv_initialize" => fn(*mut MpvHandle) -> c_int,
    terminate_destroy: "mpv_terminate_destroy" => fn(*mut MpvHandle),
    set_option_string: "mpv_set_option_string" => fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    set_property_string: "mpv_set_property_string" => fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    get_property_string: "mpv_get_property_string" => fn(*mut MpvHandle, *const c_char) -> *mut c_char,
    command: "mpv_command" => fn(*mut MpvHandle, *mut *const c_char) -> c_int,
    observe_property: "mpv_observe_property" => fn(*mut MpvHandle, u64, *const c_char, c_int) -> c_int,
    request_log_messages: "mpv_request_log_messages" => fn(*mut MpvHandle, *const c_char) -> c_int,
    wait_event: "mpv_wait_event" => fn(*mut MpvHandle, c_double) -> *mut MpvEvent,
    wakeup: "mpv_wakeup" => fn(*mut MpvHandle),
    error_string: "mpv_error_string" => fn(c_int) -> *const c_char,
    free: "mpv_free" => fn(*mut c_void),
    render_context_create: "mpv_render_context_create" => fn(*mut *mut MpvRenderCtx, *mut MpvHandle, *mut MpvRenderParam) -> c_int,
    render_context_set_update_callback: "mpv_render_context_set_update_callback" => fn(*mut MpvRenderCtx, Option<UpdateCallbackFn>, *mut c_void),
    render_context_update: "mpv_render_context_update" => fn(*mut MpvRenderCtx) -> u64,
    render_context_render: "mpv_render_context_render" => fn(*mut MpvRenderCtx, *mut MpvRenderParam) -> c_int,
    render_context_report_swap: "mpv_render_context_report_swap" => fn(*mut MpvRenderCtx),
    render_context_free: "mpv_render_context_free" => fn(*mut MpvRenderCtx),
}
