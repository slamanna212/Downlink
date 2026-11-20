//! Tauri commands. Names and argument shapes follow `apps/ui/src/api/ipc.ts` exactly; arguments
//! arrive camelCase and are spread at the top level (Tauri maps them to snake_case parameters).
//! Failures reject with core's `AppError` (camelCase `code`, `message`, `retryAfter`).

pub mod catalog;
pub mod diagnostics;
pub mod media;
pub mod player;
pub mod settings;
pub mod updates;

pub type CmdResult<T> = Result<T, downlink_core::AppError>;
