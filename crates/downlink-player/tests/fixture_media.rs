//! Player tests against the synthetic fixture streams (tools/fixtures/streams/make-media.sh), served
//! over a tiny in-test HTTP server. They assert the state machine and decoded frames (mpv `vo=null`,
//! so no surface is needed). Each test skips (passes) when libmpv or the fixture clip is missing.
//!
//! Run: cargo test -p downlink-player --test fixture_media   (about 15 s total)
//! libmpv in dev: DOWNLINK_MPV_PATH=/usr/lib/x86_64-linux-gnu/libmpv.so.2
//! Clips: tools/fixtures/streams/make-media.sh (needs ffmpeg).

#![allow(clippy::expect_used)] // test helpers

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use downlink_player::{
    FailureKind, PlayMode, PlayRequest, Player, Preferences, Profile, RenderBackend, SessionState,
    load,
};

struct Server {
    port: u16,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Serves `clip` on `/s.ts`, looped, at `speed` x real time (clip bytes / `secs`). With
/// `drop_after`, the first connection is cut after that many bytes and the origin then stays down
/// (later connections are closed without a response).
fn serve(clip: Vec<u8>, secs: f64, speed: f64, drop_after: Option<usize>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let (st, rq) = (stop.clone(), requests.clone());
    let clip = Arc::new(clip);
    let rate = clip.len() as f64 / secs * speed; // bytes per second
    std::thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        while !st.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut conn, _)) => {
                    let n = rq.fetch_add(1, Ordering::SeqCst);
                    if drop_after.is_some() && n > 0 {
                        continue; // the origin stays down after the first cut: close at once
                    }
                    let (clip, st) = (clip.clone(), st.clone());
                    std::thread::spawn(move || {
                        conn.set_nonblocking(false).ok();
                        let mut buf = [0u8; 2048];
                        let _ = conn.read(&mut buf);
                        let head = "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nConnection: close\r\n\r\n";
                        if conn.write_all(head.as_bytes()).is_err() {
                            return;
                        }
                        send_loop(&mut conn, &clip, rate, drop_after, &st);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    Server {
        port,
        stop,
        requests,
    }
}

fn send_loop(
    conn: &mut TcpStream,
    clip: &[u8],
    rate: f64,
    drop_after: Option<usize>,
    stop: &AtomicBool,
) {
    const CHUNK: usize = 16 * 1024;
    let started = Instant::now();
    let mut sent = 0usize;
    let mut i = 0usize;
    while !stop.load(Ordering::SeqCst) {
        let end = (i + CHUNK).min(clip.len());
        if conn.write_all(&clip[i..end]).is_err() {
            return;
        }
        sent += end - i;
        i = if end == clip.len() { 0 } else { end };
        if drop_after.is_some_and(|n| sent >= n) {
            return; // close mid-stream
        }
        let due = Duration::from_secs_f64(sent as f64 / rate);
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
}

fn clip(name: &str) -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../tools/fixtures/media/{name}.ts"));
    std::fs::read(p).ok()
}

fn player() -> Option<Player> {
    let loaded = load(None).ok()?;
    let p = Player::new(
        &loaded,
        Preferences::default(),
        RenderBackend::RenderApi,
        Arc::new(|_| {}),
    )
    .ok()?;
    // No render context in tests: decode and "display" into nothing.
    p.engine().set_property("vo", "null").ok()?;
    Some(p)
}

fn req(port: u16, failover: bool) -> PlayRequest {
    PlayRequest {
        url: format!("http://127.0.0.1:{port}/s.ts"),
        headers: vec![],
        profile: Profile::Live,
        start_secs: None,
        failover_capable: failover,
        mode: PlayMode::Live,
        source_index: 0,
    }
}

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

fn prop_f64(p: &Player, name: &str) -> f64 {
    p.engine()
        .get_property(name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0)
}

/// Plays `name` until it is Playing and ~1.5 s of video decoded; returns the player (still playing).
fn play_clip(name: &str, secs: f64) -> Option<(Player, Server)> {
    let data = clip(name).or_else(|| {
        eprintln!("skipping: fixture {name}.ts missing (run tools/fixtures/streams/make-media.sh)");
        None
    })?;
    let p = player().or_else(|| {
        eprintln!("skipping: no libmpv");
        None
    })?;
    let srv = serve(data, secs, 2.0, None);
    p.play(req(srv.port, false)).ok()?;
    let snap = p.wait_for(|s| s == SessionState::Playing, Duration::from_secs(20));
    assert_eq!(
        snap.state,
        Some(SessionState::Playing),
        "never reached Playing: {:?}",
        snap.error
    );
    assert!(
        wait(|| prop_f64(&p, "estimated-frame-number") > 30.0, 15),
        "no frames decoded"
    );
    Some((p, srv))
}

fn assert_stream(name: &str, codec: &str, w: i64, h: i64) -> Option<Player> {
    let (p, _srv) = play_clip(name, 20.0)?;
    assert!(wait(|| p.snapshot().height.is_some(), 5));
    let s = p.snapshot();
    assert_eq!(
        (s.width, s.height),
        (Some(w), Some(h)),
        "resolution of {name}"
    );
    assert!(
        s.video_codec
            .as_deref()
            .is_some_and(|c| c.to_lowercase().contains(codec)),
        "codec {:?} should contain {codec}",
        s.video_codec
    );
    assert!(prop_f64(&p, "estimated-frame-number") > 0.0);
    Some(p)
}

#[test]
fn h264_aac_decodes_frames_at_fixture_resolution() {
    assert_stream("h264_aac_1080p30", "264", 1920, 1080);
}

#[test]
fn hevc_ac3_decodes_frames_at_fixture_resolution() {
    assert_stream("hevc_ac3_1080p30", "hevc", 1920, 1080);
}

#[test]
fn interlaced_1080i_is_deinterlaced_with_auto() {
    let Some(p) = assert_stream("h264_1080i_tff", "264", 1920, 1080) else {
        return;
    };
    // Report what mpv exposes, then assert on the effective state.
    let di = p.engine().get_property("deinterlace");
    let vf = p.engine().get_property("vf");
    let active = p.engine().get_property("deinterlace-active");
    eprintln!("deinterlace={di:?} deinterlace-active={active:?} vf={vf:?}");
    let on = di.as_deref() == Some("yes")
        || active.as_deref() == Some("yes")
        || vf
            .as_deref()
            .is_some_and(|v| v.contains("yadif") || v.contains("bwdif"));
    assert!(
        on,
        "interlaced stream not deinterlaced (deinterlace={di:?} vf={vf:?})"
    );
}

#[test]
fn broken_timestamps_keep_playing() {
    let Some((p, _srv)) = play_clip("broken_timestamps", 20.0) else {
        return;
    };
    // The clip has a backwards jump after ~5 s and a 5 s forward gap after ~15 s. Keep decoding
    // across both: frames keep increasing and the session never leaves Playing for Failed.
    let start = prop_f64(&p, "estimated-frame-number");
    std::thread::sleep(Duration::from_secs(12));
    let later = prop_f64(&p, "estimated-frame-number");
    assert!(
        later > start + 100.0,
        "decoding stalled: {start} -> {later}"
    );
    assert_ne!(p.snapshot().state, Some(SessionState::Failed));
}

#[test]
fn mid_stream_drop_reconnects_then_fails_as_a_stream_failure() {
    let (Some(data), Some(p)) = (clip("h264_aac_1080p30"), player()) else {
        eprintln!("skipping: no libmpv or fixture");
        return;
    };
    // The first connection serves ~1 MB (a second or two of video) and is cut; the origin then dies.
    let srv = serve(data, 20.0, 4.0, Some(1_000_000));
    p.play(req(srv.port, false)).expect("play");
    let snap = p.wait_for(|s| s == SessionState::Playing, Duration::from_secs(20));
    assert_eq!(
        snap.state,
        Some(SessionState::Playing),
        "should play before the drop"
    );
    let snap = p.wait_for(|s| s == SessionState::Failed, Duration::from_secs(50));
    assert_eq!(snap.state, Some(SessionState::Failed));
    assert!(
        snap.attempt >= 2,
        "should have reconnected: attempt {}",
        snap.attempt
    );
    assert!(
        srv.requests.load(Ordering::SeqCst) >= 2,
        "no reconnect request"
    );
    let f = snap.failure.expect("failure classification");
    assert_eq!(f.kind, FailureKind::Stream);
}
