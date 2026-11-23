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

/// Directories (in order) where a packaged install may keep the shipped engine. Pure path logic:
/// `resource_dir` is Tauri's resource dir (`/usr/lib/<productName>` for a deb, which is NOT where
/// `bundle.linux.deb.files` puts the engine), `exe` the running executable.
pub fn bundled_search_dirs(resource_dir: Option<&Path>, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(r) = resource_dir {
        v.push(r.join("mpv"));
        v.push(r.to_path_buf());
    }
    if let Some(dir) = exe.and_then(Path::parent) {
        v.push(dir.join("mpv"));
        v.push(dir.join("resources").join("mpv"));
        // deb/rpm: /usr/bin/<exe> -> /usr/lib/downlink/mpv
        v.push(dir.join("../lib/downlink/mpv"));
        // AppImage: <appdir>/usr/bin/<exe> -> <appdir>/usr/lib
        v.push(dir.join("../lib"));
        // macOS bundle
        v.push(dir.join("../Frameworks"));
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

/// Load libmpv following the PLAN §4.10 order.
///
/// `bundled_dirs` are the candidate directories that may hold the shipped engine (see
/// [`bundled_search_dirs`]); empty in dev builds that have not staged one.
pub fn load(bundled_dirs: &[PathBuf]) -> Result<LoadedMpv, PlayerError> {
    load_with(bundled_dirs, false)
}

/// Env var that makes [`load_from_env`] refuse everything but the bundled engine (CI smoke test).
pub const ENV_REQUIRE_BUNDLED: &str = "DOWNLINK_REQUIRE_BUNDLED";

/// [`load`], with the escape hatch and the system fallback off in release builds (or when
/// `DOWNLINK_REQUIRE_BUNDLED=1`).
pub fn load_from_env(bundled_dirs: &[PathBuf]) -> Result<LoadedMpv, PlayerError> {
    load_with(bundled_dirs, strict_by_default())
}

/// Release builds never fall back to the env override or a system libmpv (CLAUDE.md rule 14);
/// debug builds do unless `DOWNLINK_REQUIRE_BUNDLED=1`.
pub fn strict_by_default() -> bool {
    !cfg!(debug_assertions) || std::env::var(ENV_REQUIRE_BUNDLED).is_ok_and(|v| v == "1")
}

/// `require_bundled`: only the bundled directory is tried; no env override, no system library.
pub fn load_with(
    bundled_dirs: &[PathBuf],
    require_bundled: bool,
) -> Result<LoadedMpv, PlayerError> {
    let mut attempts: Vec<String> = Vec::new();

    for dir in bundled_dirs {
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
    if bundled_dirs.is_empty() {
        attempts.push("no bundled directory to search".to_owned());
    } else if attempts.is_empty() {
        let list: Vec<String> = bundled_dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect();
        attempts.push(format!(
            "no libmpv in any searched directory: {}",
            list.join(", ")
        ));
    }

    if require_bundled {
        attempts.push("bundled engine required (no env override, no system fallback)".to_owned());
        return Err(PlayerError::Load(attempts.join("; ")));
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
    fn strict_mode_never_falls_back() {
        // No bundled dir: strict mode must fail without trying the env override or the system.
        let r = load_with(&[], true);
        assert!(matches!(r, Err(PlayerError::Load(m)) if m.contains("bundled engine required")));
    }

    #[test]
    fn release_builds_are_strict() {
        assert_eq!(
            strict_by_default(),
            !cfg!(debug_assertions) || std::env::var(ENV_REQUIRE_BUNDLED).is_ok_and(|v| v == "1")
        );
    }

    #[test]
    fn file_names_are_not_empty() {
        assert!(!library_file_names().is_empty());
    }

    #[test]
    fn deb_install_is_searched() {
        let dirs = bundled_search_dirs(
            Some(Path::new("/usr/lib/Downlink")),
            Some(Path::new("/usr/bin/downlink")),
        );
        assert!(dirs.contains(&PathBuf::from("/usr/bin/../lib/downlink/mpv")));
        assert_eq!(dirs[0], PathBuf::from("/usr/lib/Downlink/mpv"));
    }

    #[test]
    fn appimage_install_is_searched() {
        let dirs = bundled_search_dirs(None, Some(Path::new("/tmp/.mount_x/usr/bin/downlink")));
        assert!(dirs.contains(&PathBuf::from("/tmp/.mount_x/usr/bin/../lib")));
    }

    #[test]
    fn missing_dirs_are_listed_in_the_error() {
        let r = load_with(&[PathBuf::from("/nonexistent/mpv")], true);
        assert!(matches!(r, Err(PlayerError::Load(m)) if m.contains("/nonexistent/mpv")));
    }
}
