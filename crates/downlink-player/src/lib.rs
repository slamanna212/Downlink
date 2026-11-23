//! downlink-player: runtime-loaded libmpv, session state machine, failover policy and the native
//! video surfaces that sit under the transparent webview (PLAN §2.2, §4).
//!
//! `unsafe` lives here (FFI and platform shims) and every block carries a `// SAFETY:` comment.

pub mod engine;
pub mod error;
pub mod ffi;
pub mod gpu;
pub mod loader;
pub mod options;
pub mod player;
pub mod retry;
pub mod state;
pub mod surface;
pub mod timeshift;

pub use engine::{Engine, RenderContext};
pub use error::PlayerError;
pub use gpu::{VideoCaps, video_caps};
pub use loader::{
    ENV_MPV_PATH, ENV_REQUIRE_BUNDLED, LoadSource, LoadedMpv, load, load_from_env, load_with,
};
pub use options::{
    AudioMode, Deinterlace, HdrOutput, Preferences, Profile, RenderBackend, SubtitleStyle,
};
pub use player::{
    AudioDevice, EventSink, FailureInfo, FailureKind, PlayMode, PlayRequest, Player, PlayerCommand,
    PlayerEvent, PlayerSnapshot, StatsSnapshot, Track, WaitingOn,
};
pub use retry::{FailureCause, RetryDecision, RetryPolicy};
pub use state::{SessionState, StateMachine, Transition};
pub use surface::{MAX_SURFACES, MainThreadDispatch, SurfaceBounds, VideoSurface};
pub use timeshift::LiveAnchor;
