//! Runtime loading of libmpv. Load order (PLAN §4.10): bundled -> `DOWNLINK_MPV_PATH` -> system
//! library, the last logged loudly as unsupported. No code path ever asks the user to install
//! anything.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::Library;

use crate::error::PlayerError;
use crate::ffi::MpvApi;

/// Environment variable naming a libmpv to load (developer escape hatch; never set in release).
pub const ENV_MPV_PATH: &str = "DOWNLINK_MPV_PATH";

/// Where the loaded libmpv came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadSource {
    /// The libmpv shipped inside the installer.
    Bundled(PathBuf),
    /// `DOWNLINK_MPV_PATH` (developer escape hatch).
    EnvOverride(PathBuf),
    /// The system's libmpv. Unsupported: release builds must never rely on this.
    SystemUnsupported(String),
}

/// A loaded libmpv.
#[derive(Clone)]
pub struct LoadedMpv {
    pub api: Arc<MpvApi>,
    pub source: LoadSource,
    pub client_api_version: (u32, u32),
}

/// File names of libmpv per platform, in preference order.
pub fn library_file_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["libmpv-2.dll", "mpv-2.dll"]
    } else if cfg!(target_os = "macos") {
        &["libmpv.2.dylib", "libmpv.dylib"]
    } else {
        &["libmpv.so.2", "libmpv.so"]
    }
}

/// Load libmpv following the PLAN §4.10 order.
///
/// `bundled_dir` is the app-relative directory holding the shipped engine (`None` in dev builds
/// that have not staged one).
pub fn load(bundled_dir: Option<&Path>) -> Result<LoadedMpv, PlayerError> {
    let mut attempts: Vec<String> = Vec::new();

    if let Some(dir) = bundled_dir {
        for name in library_file_names() {
            let path = dir.join(name);
            if path.is_file() {
                match open(&path) {
                    Ok(api) => return finish(api, LoadSource::Bundled(path)),
                    Err(e) => attempts.push(format!("bundled {}: {e}", path.display())),
                }
            }
        }
    }

    if let Some(raw) = std::env::var_os(ENV_MPV_PATH) {
        let path = PathBuf::from(raw);
        tracing::warn!(
            "loading libmpv from {ENV_MPV_PATH} ({}); this is a developer escape hatch",
            path.display()
        );
        match open(&path) {
            Ok(api) => return finish(api, LoadSource::EnvOverride(path)),
            Err(e) => attempts.push(format!("{ENV_MPV_PATH} {}: {e}", path.display())),
        }
    }

    for name in library_file_names() {
        if let Ok(api) = open(Path::new(name)) {
            tracing::error!(
                "UNSUPPORTED: no bundled libmpv was found; falling back to the system library `{name}`. \
                 Release builds must ship their own engine (PLAN §4.10)."
            );
            return finish(api, LoadSource::SystemUnsupported((*name).to_owned()));
        }
    }
    attempts.push("system library: not found".to_owned());
    Err(PlayerError::Load(attempts.join("; ")))
}

fn open(path: &Path) -> Result<MpvApi, String> {
    // SAFETY: loading a shared library runs its initialisers. The library is libmpv, which is
    // designed to be dlopen'ed; the path comes from our bundle, a developer env var, or the
    // system loader.
    let lib = unsafe { Library::new(path) }.map_err(|e| e.to_string())?;
    MpvApi::from_library(lib).map_err(|e| e.to_string())
}

fn finish(api: MpvApi, source: LoadSource) -> Result<LoadedMpv, PlayerError> {
    // SAFETY: `mpv_client_api_version` takes no arguments and has no preconditions.
    let v = unsafe { (api.client_api_version)() };
    let (major, minor) = ((v >> 16) as u32, (v & 0xffff) as u32);
    if major < 2 {
        return Err(PlayerError::TooOld(major, minor));
    }
    Ok(LoadedMpv {
        api: Arc::new(api),
        source,
        client_api_version: (major, minor),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_path_is_an_error_not_a_panic() {
        let r = open(Path::new("/nonexistent/libmpv.so.2"));
        assert!(r.is_err());
    }

    #[test]
    fn file_names_are_not_empty() {
        assert!(!library_file_names().is_empty());
    }
}
