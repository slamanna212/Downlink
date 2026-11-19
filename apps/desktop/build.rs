use std::path::Path;

fn main() {
    // `tauri::generate_context!` requires `frontendDist` to exist at compile time. On a fresh
    // checkout the UI has not been built yet; leave a (gitignored) placeholder so the workspace
    // still builds. Real UI builds overwrite it.
    let dist = Path::new("../ui/dist");
    if std::env::var_os("TAURI_CONFIG").is_none() && !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
        let _ = std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>Downlink</title><p>UI not built. Run pnpm -C apps/ui build.</p>",
        );
    }
    tauri_build::build();
}
