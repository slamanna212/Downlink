//! Needs a real libmpv. Skipped (passes) when none can be loaded, unless `DOWNLINK_REQUIRE_MPV=1`
//! (CI sets it, with `DOWNLINK_MPV_PATH` pointing at the same staged libmpv the release bundles).
//! Set DOWNLINK_MPV_PATH in dev.

use std::sync::Arc;

use downlink_player::{LoadedMpv, Player, PlayerEvent, Preferences, RenderBackend, load};

/// The libmpv under test, or `None` to skip. Panics instead of skipping when CI requires it.
fn engine() -> Option<LoadedMpv> {
    match load(&[]) {
        Ok(l) => Some(l),
        Err(e) if std::env::var("DOWNLINK_REQUIRE_MPV").is_ok_and(|v| v == "1") => {
            panic!("DOWNLINK_REQUIRE_MPV=1 but libmpv failed to load: {e}")
        }
        Err(_) => {
            eprintln!("skipping: no libmpv available");
            None
        }
    }
}

#[test]
fn engine_initialises_and_reports_version() {
    let Some(loaded) = engine() else {
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

mod failover {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use downlink_player::{
        PlayMode, PlayRequest, Player, PlayerEvent, Preferences, Profile, RenderBackend,
        SessionState,
    };

    fn run(failover: bool) -> Option<(SessionState, Vec<String>, u32)> {
        let loaded = super::engine()?;
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let ev = events.clone();
        let sink: downlink_player::EventSink = Arc::new(move |e| {
            if let PlayerEvent::FailoverNeeded { last_error, .. } = e
                && let Ok(mut v) = ev.lock()
            {
                v.push(last_error);
            }
        });
        let p = match Player::new(
            &loaded,
            Preferences::default(),
            RenderBackend::RenderApi,
            sink,
        ) {
            Ok(p) => p,
            Err(e) => panic!("player init failed: {e}"),
        };
        // Nothing listens on port 1: connection refused, fast and offline.
        p.play(PlayRequest {
            url: "http://127.0.0.1:1/x.ts".into(),
            headers: vec![],
            profile: Profile::Live,
            start_secs: None,
            failover_capable: failover,
            mode: PlayMode::Live,
            source_index: 0,
        })
        .unwrap_or_else(|e| panic!("play failed: {e}"));
        let snap = p.wait_for(|s| s == SessionState::Failed, Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(300));
        let v = events.lock().ok()?.clone();
        Some((snap.state?, v, snap.attempt))
    }

    #[test]
    fn failover_source_retries_once_then_asks_for_stream_list() {
        let Some((state, asks, attempts)) = run(true) else {
            return;
        };
        assert_eq!(state, SessionState::Failed);
        assert_eq!(asks.len(), 1, "exactly one FailoverNeeded");
        assert_eq!(attempts, 2, "initial + one reconnect");
    }

    #[test]
    fn plain_source_gives_up_after_two_attempts_without_asking() {
        let Some((state, asks, attempts)) = run(false) else {
            return;
        };
        assert_eq!(state, SessionState::Failed);
        assert!(asks.is_empty());
        assert_eq!(attempts, 2);
    }
}

mod supervisor {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use downlink_player::{
        FailureKind, PlayMode, PlayRequest, Player, Preferences, Profile, RenderBackend,
        SessionState,
    };

    fn wait(mut f: impl FnMut() -> bool, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn engine_death_restarts_once_then_fails_with_a_crash_id() {
        let Some(loaded) = super::engine() else {
            return;
        };
        let sink: downlink_player::EventSink = Arc::new(|_| {});
        let p = match Player::new(
            &loaded,
            Preferences::default(),
            RenderBackend::RenderApi,
            sink,
        ) {
            Ok(p) => p,
            Err(e) => panic!("player init failed: {e}"),
        };
        let replaced = Arc::new(AtomicU32::new(0));
        let r2 = replaced.clone();
        p.on_engine_replaced(move |_| {
            r2.fetch_add(1, Ordering::SeqCst);
        });
        let req = PlayRequest {
            url: "http://127.0.0.1:1/x.ts".into(),
            headers: vec![],
            profile: Profile::Live,
            start_secs: None,
            failover_capable: false,
            mode: PlayMode::Live,
            source_index: 0,
        };
        if let Err(e) = p.play(req) {
            panic!("play failed: {e}");
        }
        // First death: the supervisor restarts the engine and tells the surface.
        let _ = p.engine().command(&["quit"]);
        assert!(
            wait(|| replaced.load(Ordering::SeqCst) == 1, 10),
            "engine was not replaced"
        );
        assert!(p.notes().iter().any(|n| n.contains("engine restarted")));
        // Let the replayed (refused) request settle, then kill the new engine: second death.
        assert!(wait(
            || p.snapshot().state == Some(SessionState::Failed),
            15
        ));
        let _ = p.engine().command(&["quit"]);
        assert!(wait(
            || p.snapshot()
                .failure
                .as_ref()
                .is_some_and(|f| f.kind == FailureKind::Engine),
            10
        ));
        let f = p.snapshot().failure;
        assert!(
            f.as_ref()
                .and_then(|f| f.crash_id.as_ref())
                .is_some_and(|c| c.len() == 8)
        );
        assert_eq!(
            replaced.load(Ordering::SeqCst),
            1,
            "only one restart per session"
        );
    }
}
