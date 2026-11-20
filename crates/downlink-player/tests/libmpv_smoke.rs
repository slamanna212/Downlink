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

mod failover {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use downlink_player::{
        PlayMode, PlayRequest, Player, PlayerEvent, Preferences, Profile, RenderBackend,
        SessionState, load,
    };

    fn run(failover: bool) -> Option<(SessionState, Vec<String>, u32)> {
        let loaded = load(None).ok()?;
        let events: Arc<Mutex<Vec<String>>> = Arc::default();
        let ev = events.clone();
        let sink: downlink_player::EventSink = Arc::new(move |e| {
            if let PlayerEvent::FailoverNeeded { last_error, .. } = e
                && let Ok(mut v) = ev.lock()
            {
                v.push(last_error);
            }
        });
        let p = Player::new(
            &loaded,
            Preferences::default(),
            RenderBackend::RenderApi,
            sink,
        )
        .ok()?;
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
        .ok()?;
        let snap = p.wait_for(|s| s == SessionState::Failed, Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(300));
        let v = events.lock().ok()?.clone();
        Some((snap.state?, v, snap.attempt))
    }

    #[test]
    fn failover_source_retries_once_then_asks_for_stream_list() {
        let Some((state, asks, attempts)) = run(true) else {
            eprintln!("skipping: no libmpv");
            return;
        };
        assert_eq!(state, SessionState::Failed);
        assert_eq!(asks.len(), 1, "exactly one FailoverNeeded");
        assert_eq!(attempts, 2, "initial + one reconnect");
    }

    #[test]
    fn plain_source_gives_up_after_two_attempts_without_asking() {
        let Some((state, asks, attempts)) = run(false) else {
            eprintln!("skipping: no libmpv");
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
        SessionState, load,
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
        let Ok(loaded) = load(None) else {
            eprintln!("skipping: no libmpv");
            return;
        };
        let sink: downlink_player::EventSink = Arc::new(|_| {});
        let Ok(p) = Player::new(
            &loaded,
            Preferences::default(),
            RenderBackend::RenderApi,
            sink,
        ) else {
            eprintln!("skipping: player init failed");
            return;
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
        assert!(p.play(req).is_ok());
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
