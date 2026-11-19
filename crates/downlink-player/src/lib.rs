//! downlink-player: runtime-loaded libmpv, session state machine, failover policy and the native
//! video surfaces that sit under the transparent webview (PLAN §2.2, §4).
//!
//! `unsafe` lives here (FFI and platform shims) and every block carries a `// SAFETY:` comment.

pub mod engine;
pub mod error;
pub mod ffi;
pub mod loader;
pub mod options;
pub mod player;
pub mod retry;
pub mod state;
pub mod surface;

pub use engine::{Engine, RenderContext};
pub use error::PlayerError;
pub use loader::{ENV_MPV_PATH, LoadSource, LoadedMpv, load};
pub use options::{AudioMode, Deinterlace, Preferences, Profile, RenderBackend, SubtitleStyle};
pub use player::{
    AudioDevice, EventSink, PlayMode, PlayRequest, Player, PlayerCommand, PlayerEvent,
    PlayerSnapshot, StatsSnapshot, Track, WaitingOn,
};
pub use retry::{FailureCause, RetryDecision, RetryPolicy};
pub use state::{SessionState, StateMachine, Transition};
pub use surface::{MainThreadDispatch, SurfaceBounds, VideoSurface};
