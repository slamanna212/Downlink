//! downlink-core: pure domain logic. Never depends on Tauri, wry, React or mpv.

pub mod auth;
pub mod error;
pub mod redact;
pub mod source;
pub mod store;

pub use error::{AppError, ErrorCode, Result};
