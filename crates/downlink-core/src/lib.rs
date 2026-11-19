//! downlink-core: pure domain logic. Never depends on Tauri, wry, React or mpv.

pub mod auth;
pub mod catalog;
pub mod epg;
pub mod error;
pub mod images;
pub mod normalise;
pub mod redact;
pub mod source;
pub mod store;
pub mod sync;

pub use error::{AppError, ErrorCode, Result};
