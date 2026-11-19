//! downlink-core: pure domain logic. Never depends on Tauri, wry, React or mpv.

pub mod auth;
pub mod catalog;
pub mod catchup;
pub mod epg;
pub mod error;
pub mod images;
pub mod m3u;
pub mod normalise;
pub mod redact;
pub mod service;
pub mod sessions;
pub mod source;
pub mod store;
pub mod sync;
pub mod userstate;
pub mod util;
pub mod xmltv;

pub use error::{AppError, ErrorCode, Result};
