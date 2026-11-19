use thiserror::Error;

/// Errors from the player crate. Messages never contain URLs or credentials.
#[derive(Debug, Error)]
pub enum PlayerError {
    #[error("libmpv could not be loaded: {0}")]
    Load(String),
    #[error("libmpv is too old (client API {0}.{1}); 2.0 or newer is required")]
    TooOld(u32, u32),
    #[error("mpv call `{call}` failed: {message}")]
    Mpv { call: &'static str, message: String },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("no media is playing")]
    NotPlaying,
    #[error("video surface error: {0}")]
    Surface(String),
}
