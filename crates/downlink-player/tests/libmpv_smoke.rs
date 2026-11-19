//! Needs a real libmpv; skipped (passes) when none can be loaded. Set DOWNLINK_MPV_PATH in dev.

use std::sync::Arc;

use downlink_player::{Player, PlayerEvent, Preferences, RenderBackend, load};

#[test]
fn engine_initialises_and_reports_version() {
    let Ok(loaded) = load(None) else {
        eprintln!("skipping: no libmpv available");
        return;
    };
    let sink: downlink_player::EventSink = Arc::new(|_e: PlayerEvent| {});
    let player = Player::new(
        &loaded,
        Preferences::default(),
        RenderBackend::RenderApi,
        sink,
    );
    let player = match player {
        Ok(p) => p,
        Err(e) => panic!("player init failed: {e}"),
    };
    let diag = player.diagnostics();
    assert!(diag.iter().any(|(k, _)| k == "mpv-version"));
}
