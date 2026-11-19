//! The pre-warmed player: one libmpv instance reused across zaps, an event thread driving the
//! session state machine, the failover/retry policy and the throttled `player://state` stream.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::engine::{EndReason, Engine, EngineEvent, PropValue};
use crate::error::PlayerError;
use crate::ffi;
use crate::loader::{LoadSource, LoadedMpv};
use crate::options::{self, Preferences, Profile, RenderBackend};
use crate::retry::{FailureCause, NO_FRAME_TIMEOUT, RetryDecision, RetryPolicy};
use crate::state::{SessionState, StateMachine, Throttle, Transition};

/// What to play. The URL and headers are secrets: `Debug` never prints them.
#[derive(Clone)]
pub struct PlayRequest {
    pub url: String,
    /// Sent to mpv as per-file `http-header-fields`, never globally (CLAUDE.md rule 5).
    pub headers: Vec<(String, String)>,
    pub profile: Profile,
    pub start_secs: Option<f64>,
    /// `Caps::FAILOVER` present on the source.
    pub failover_capable: bool,
    pub mode: PlayMode,
    /// Index of the stream within the channel's failover list (0 = primary), for the info panel.
    pub source_index: u32,
}

/// What kind of session this is (drives UI chrome, not mpv options; those follow [`Profile`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PlayMode {
    #[default]
    Live,
    Catchup,
    Vod,
}

/// What a long buffering state is waiting on (shown after 3 s, PLAN §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitingOn {
    Connection,
    FirstData,
    FirstKeyframe,
}

pub const WAITING_AFTER: Duration = Duration::from_secs(3);

/// Pure rule for [`WaitingOn`]: only after `WAITING_AFTER` in a connecting/buffering state.
pub fn waiting_on(
    state: SessionState,
    in_state: Duration,
    file_loaded: bool,
    first_frame_seen: bool,
) -> Option<WaitingOn> {
    if in_state < WAITING_AFTER {
        return None;
    }
    match state {
        SessionState::Connecting => Some(WaitingOn::Connection),
        SessionState::Buffering if !first_frame_seen && file_loaded => {
            Some(WaitingOn::FirstKeyframe)
        }
        SessionState::Buffering if !first_frame_seen => Some(WaitingOn::Connection),
        SessionState::Buffering => Some(WaitingOn::FirstData),
        _ => None,
    }
}

/// One audio/video/subtitle track.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
}

/// Parse mpv's `track-list` JSON.
pub fn parse_tracks(json: &str) -> Vec<Track> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|t| {
            let str_of = |k: &str| t.get(k).and_then(|x| x.as_str()).map(str::to_owned);
            Some(Track {
                id: t.get("id")?.as_i64()?,
                kind: str_of("type")?,
                lang: str_of("lang"),
                title: str_of("title"),
                codec: str_of("codec"),
                selected: t.get("selected").and_then(|x| x.as_bool()).unwrap_or(false),
            })
        })
        .collect()
}

/// An output device from `audio-device-list`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
    pub supports_passthrough: bool,
}

pub fn parse_audio_devices(json: &str) -> Vec<AudioDevice> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|d| {
            let name = d.get("name")?.as_str()?.to_owned();
            let description = d
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or(&name)
                .to_owned();
            Some(AudioDevice {
                supports_passthrough: crate::options::device_supports_passthrough(&name),
                name,
                description,
            })
        })
        .collect()
}

/// Payload of `session://stats` (1 Hz, only while the info panel is open).
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    pub resolution: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub dropped_frames: Option<i64>,
    pub buffer_secs: Option<f64>,
    pub deinterlace: Option<String>,
    pub hwdec_current: Option<String>,
    pub source_index: u32,
}

impl fmt::Debug for PlayRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayRequest")
            .field("url", &"<redacted>")
            .field(
                "headers",
                &format_args!("<{} redacted>", self.headers.len()),
            )
            .field("profile", &self.profile)
            .field("start_secs", &self.start_secs)
            .field("failover_capable", &self.failover_capable)
            .finish()
    }
}

/// Typed transport commands. The UI never sends mpv strings (CLAUDE.md rule 3).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PlayerCommand {
    Pause,
    Resume,
    TogglePause,
    SeekRelative { secs: f64 },
    SeekAbsolute { secs: f64 },
    Volume { percent: u8 },
    Mute { muted: bool },
    AudioTrack { id: i64 },
    SubtitleTrack { id: Option<i64> },
}

/// Payload of `player://state`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub state: Option<SessionState>,
    pub session: u64,
    pub attempt: u32,
    pub position_secs: Option<f64>,
    pub duration_secs: Option<f64>,
    pub buffering_on_cache: bool,
    pub hwdec_current: Option<String>,
    pub video_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub fps: Option<f64>,
    pub dropped_frames: Option<i64>,
    /// Time from `loadfile` to the first frame of the current session, in milliseconds.
    pub first_frame_ms: Option<u64>,
    pub error: Option<String>,
    pub mode: Option<PlayMode>,
    /// Seconds of media buffered ahead (`demuxer-cache-time` minus position).
    pub buffered_secs: Option<f64>,
    pub volume: Option<f64>,
    pub muted: bool,
    pub tracks: Vec<Track>,
    /// Set after 3 s of connecting/buffering.
    pub waiting_on: Option<WaitingOn>,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    /// 1 Hz while enabled with [`Player::set_stats_enabled`].
    Stats(StatsSnapshot),
    /// Throttled to <= 10 Hz.
    State(PlayerSnapshot),
    /// The retry policy wants the channel's stream list (then call [`Player::play_alternate`]).
    FailoverNeeded { session: u64, last_error: String },
    /// An mpv log line, already scrubbed of URLs.
    Log { level: String, text: String },
}

pub type EventSink = Arc<dyn Fn(PlayerEvent) + Send + Sync>;

#[derive(Default)]
struct Stats {
    cache_time: Option<f64>,
    volume: Option<f64>,
    muted: bool,
    tracks: Vec<Track>,
    position: Option<f64>,
    duration: Option<f64>,
    hwdec: Option<String>,
    codec: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
    fps: Option<f64>,
    dropped: Option<i64>,
}

struct Shared {
    sm: StateMachine,
    request: Option<PlayRequest>,
    policy: RetryPolicy,
    session: u64,
    stats: Stats,
    load_started: Option<Instant>,
    retry_at: Option<Instant>,
    first_frame_ms: Option<u64>,
    error: Option<String>,
    cache_buffering: bool,
    paused: bool,
    throttle: Throttle,
    outbox: Vec<PlayerEvent>,
    state_since: Instant,
    file_loaded: bool,
    last_stats: Option<Instant>,
    audio_fell_back: bool,
}

struct Inner {
    engine: Arc<Engine>,
    prefs: Mutex<Preferences>,
    shared: Mutex<Shared>,
    cv: Condvar,
    stop: AtomicBool,
    stats_enabled: AtomicBool,
    sink: EventSink,
    epoch: Instant,
    source: LoadSource,
}

pub struct Player {
    inner: Arc<Inner>,
    thread: Option<JoinHandle<()>>,
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Replace anything URL-shaped in an mpv log line. Rule 8: nothing credential-bearing may reach
/// logs; the app layer additionally runs `redact/` over whatever this crate emits.
pub fn scrub_urls(text: &str) -> String {
    text.split(' ')
        .map(|w| if w.contains("://") { "<url>" } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Expected hardware-decode probe failures: mpv tries several hwdec backends and logs an error for
/// each one the machine lacks. They are noise unless every backend fails (`hwdec-current=no`).
pub fn is_expected_probe_noise(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "hwdec",
        "hwaccel",
        "libcuda",
        "CUDA",
        "libva",
        "vaapi",
        "VK_KHR_video_decode",
        "vdpau",
        "VDPAU",
        "Unable to set hw-parameters",
        "Suspected software renderer",
    ];
    MARKERS.iter().any(|m| text.contains(m))
}

/// mpv's message when no audio output could be opened; playback continues silently.
pub fn is_audio_open_failure(text: &str) -> bool {
    text.contains("Could not open/initialize audio device")
        || text.contains("Failed to initialize audio driver")
}

const OBSERVED: &[(&str, i32)] = &[
    ("pause", ffi::FORMAT_FLAG),
    ("paused-for-cache", ffi::FORMAT_FLAG),
    ("time-pos", ffi::FORMAT_DOUBLE),
    ("duration", ffi::FORMAT_DOUBLE),
    ("hwdec-current", ffi::FORMAT_STRING),
    ("video-codec", ffi::FORMAT_STRING),
    ("dwidth", ffi::FORMAT_INT64),
    ("dheight", ffi::FORMAT_INT64),
    ("estimated-vf-fps", ffi::FORMAT_DOUBLE),
    ("vo-drop-frame-count", ffi::FORMAT_INT64),
    ("demuxer-cache-time", ffi::FORMAT_DOUBLE),
    ("volume", ffi::FORMAT_DOUBLE),
    ("mute", ffi::FORMAT_FLAG),
    ("track-list", ffi::FORMAT_STRING),
];

impl Player {
    /// Create and pre-warm the single mpv instance.
    pub fn new(
        loaded: &LoadedMpv,
        prefs: Preferences,
        backend: RenderBackend,
        sink: EventSink,
    ) -> Result<Self, PlayerError> {
        let mut opts = options::base_options(&prefs, backend);
        // Developer-only override used by the M0 spike to probe `gpu-next` via the render API.
        if let Ok(vo) = std::env::var("DOWNLINK_MPV_VO") {
            opts.retain(|(k, _)| k != "vo");
            opts.push(("vo".to_owned(), vo));
        }
        let engine = Arc::new(Engine::new(loaded, &opts)?);
        if let Err(e) = engine.request_log_messages("warn") {
            tracing::warn!("could not request mpv log messages: {e}");
        }
        for (i, (name, fmt)) in OBSERVED.iter().enumerate() {
            engine.observe(i as u64 + 1, name, *fmt)?;
        }
        let inner = Arc::new(Inner {
            engine,
            prefs: Mutex::new(prefs),
            shared: Mutex::new(Shared {
                sm: StateMachine::new(),
                request: None,
                policy: RetryPolicy::new(false),
                session: 0,
                stats: Stats::default(),
                load_started: None,
                retry_at: None,
                first_frame_ms: None,
                error: None,
                cache_buffering: false,
                paused: false,
                throttle: Throttle::new(Duration::from_millis(100)),
                outbox: Vec::new(),
                state_since: Instant::now(),
                file_loaded: false,
                last_stats: None,
                audio_fell_back: false,
            }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            stats_enabled: AtomicBool::new(false),
            sink,
            epoch: Instant::now(),
            source: loaded.source.clone(),
        });
        let t_inner = inner.clone();
        let thread = std::thread::Builder::new()
            .name("downlink-mpv-events".into())
            .spawn(move || event_loop(&t_inner))
            .map_err(|e| PlayerError::Load(format!("event thread: {e}")))?;
        Ok(Self {
            inner,
            thread: Some(thread),
        })
    }

    /// The engine, for attaching a video surface.
    pub fn engine(&self) -> Arc<Engine> {
        self.inner.engine.clone()
    }

    pub fn load_source(&self) -> &LoadSource {
        &self.inner.source
    }

    /// Update preferences. Everything that can change on a running instance is applied now
    /// (hwdec, deinterlace, languages, audio device and mode, subtitle style); the live buffer
    /// applies from the next `play`.
    pub fn set_preferences(&self, prefs: Preferences) {
        for (k, v) in prefs.live_options() {
            if let Err(e) = self.inner.engine.set_property(&k, &v) {
                tracing::warn!("preference `{k}` not applied: {e}");
            }
        }
        if let Ok(mut p) = self.inner.prefs.lock() {
            *p = prefs;
        }
    }

    pub fn preferences(&self) -> Preferences {
        match self.inner.prefs.lock() {
            Ok(p) => p.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// Output devices (`audio-device-list`).
    pub fn audio_devices(&self) -> Vec<AudioDevice> {
        self.inner
            .engine
            .get_property("audio-device-list")
            .map(|j| parse_audio_devices(&j))
            .unwrap_or_default()
    }

    /// Choose an output device (`None` = auto) and re-evaluate the audio mode for it. Returns
    /// the mode actually in effect (passthrough falls back to surround on unsupported devices).
    pub fn set_audio_device(&self, name: Option<String>) -> crate::options::AudioMode {
        let mut prefs = self.preferences();
        prefs.audio_device = name;
        let mode = prefs.audio_mode_effective();
        self.set_preferences(prefs);
        mode
    }

    /// Turn the 1 Hz `session://stats` stream on or off (only on while the info panel is open).
    pub fn set_stats_enabled(&self, on: bool) {
        self.inner.stats_enabled.store(on, Ordering::SeqCst);
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        lock(&self.inner)
    }

    /// Enter `Resolving` (the caller is resolving a play plan). Optional before [`Player::play`].
    pub fn begin_resolving(&self) {
        let mut s = self.lock();
        go(&self.inner, &mut s, SessionState::Resolving, None);
    }

    /// Zap: `loadfile <url> replace` on the pre-warmed instance. Starts a new session.
    pub fn play(&self, req: PlayRequest) -> Result<u64, PlayerError> {
        let mut s = self.lock();
        s.session += 1;
        s.policy = RetryPolicy::new(req.failover_capable);
        s.request = Some(req);
        s.error = None;
        s.first_frame_ms = None;
        s.retry_at = None;
        s.cache_buffering = false;
        s.paused = false;
        s.file_loaded = false;
        s.stats = Stats::default();
        go(
            &self.inner,
            &mut s,
            SessionState::Connecting,
            Some("loadfile".into()),
        );
        let session = s.session;
        match load_current(&self.inner, &mut s) {
            Ok(()) => Ok(session),
            Err(e) => {
                s.error = Some(e.to_string());
                go(
                    &self.inner,
                    &mut s,
                    SessionState::Failed,
                    Some(e.to_string()),
                );
                Err(e)
            }
        }
    }

    /// Load an alternate stream after [`PlayerEvent::FailoverNeeded`]; counts as the final attempt.
    pub fn play_alternate(&self, req: PlayRequest) -> Result<(), PlayerError> {
        let mut s = self.lock();
        s.policy.on_escalated();
        s.request = Some(req);
        s.error = None;
        s.retry_at = None;
        go(
            &self.inner,
            &mut s,
            SessionState::Connecting,
            Some("alternate stream".into()),
        );
        load_current(&self.inner, &mut s)
    }

    /// Stop and release the stream (zap away, navigate away, sleep, quit).
    pub fn stop(&self) -> Result<(), PlayerError> {
        let mut s = self.lock();
        s.request = None;
        s.retry_at = None;
        s.load_started = None;
        let r = self.inner.engine.command(&["stop"]);
        go(&self.inner, &mut s, SessionState::Stopped, None);
        r
    }

    pub fn command(&self, cmd: PlayerCommand) -> Result<(), PlayerError> {
        let e = &self.inner.engine;
        match cmd {
            PlayerCommand::Pause => e.set_property("pause", "yes"),
            PlayerCommand::Resume => e.set_property("pause", "no"),
            PlayerCommand::TogglePause => e.command(&["cycle", "pause"]),
            PlayerCommand::SeekRelative { secs } => seek(e, secs, "relative"),
            PlayerCommand::SeekAbsolute { secs } => seek(e, secs, "absolute"),
            PlayerCommand::Volume { percent } => {
                e.set_property("volume", &percent.min(130).to_string())
            }
            PlayerCommand::Mute { muted } => {
                e.set_property("mute", if muted { "yes" } else { "no" })
            }
            PlayerCommand::AudioTrack { id } => e.set_property("aid", &id.to_string()),
            PlayerCommand::SubtitleTrack { id } => {
                e.set_property("sid", &id.map_or("no".to_owned(), |i| i.to_string()))
            }
        }
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        snapshot(&self.lock())
    }

    /// Last 50 timestamped transitions (debug panel).
    pub fn history(&self) -> Vec<Transition> {
        self.lock().sm.history().cloned().collect()
    }

    /// Block until `pred(state)` holds or `timeout` elapses. Returns the final snapshot.
    pub fn wait_for(
        &self,
        pred: impl Fn(SessionState) -> bool,
        timeout: Duration,
    ) -> PlayerSnapshot {
        let deadline = Instant::now() + timeout;
        let mut s = self.lock();
        while !pred(s.sm.state()) {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            s = match self.inner.cv.wait_timeout(s, deadline - now) {
                Ok((g, _)) => g,
                Err(p) => p.into_inner().0,
            };
        }
        snapshot(&s)
    }

    /// A few mpv properties for the info panel and bug reports (`hwdec-current` etc.).
    pub fn diagnostics(&self) -> Vec<(String, String)> {
        const NAMES: &[&str] = &[
            "mpv-version",
            "ffmpeg-version",
            "current-vo",
            "hwdec-current",
            "hwdec",
            "video-codec",
            "video-format",
            "hw-pixelformat",
            "video-params/pixelformat",
            "video-params/colormatrix",
            "video-params/gamma",
            "video-params/primaries",
            "estimated-vf-fps",
            "container-fps",
            "vo-drop-frame-count",
            "decoder-frame-drop-count",
            "avsync",
            "audio-codec-name",
            "cache-secs",
            "demuxer-readahead-secs",
            "demuxer-max-bytes",
        ];
        NAMES
            .iter()
            .filter_map(|n| {
                self.inner
                    .engine
                    .get_property(n)
                    .map(|v| ((*n).to_owned(), v))
            })
            .collect()
    }
}

fn seek(e: &Engine, secs: f64, mode: &str) -> Result<(), PlayerError> {
    if !secs.is_finite() {
        return Err(PlayerError::InvalidArgument("seek position".into()));
    }
    e.command(&["seek", &format!("{secs:.3}"), mode])
}

impl Drop for Player {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::SeqCst);
        self.inner.engine.wakeup();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn lock(inner: &Inner) -> MutexGuard<'_, Shared> {
    match inner.shared.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn snapshot(s: &Shared) -> PlayerSnapshot {
    PlayerSnapshot {
        state: Some(s.sm.state()),
        session: s.session,
        attempt: s.policy.attempts(),
        position_secs: s.stats.position,
        duration_secs: s.stats.duration,
        buffering_on_cache: s.cache_buffering,
        hwdec_current: s.stats.hwdec.clone(),
        video_codec: s.stats.codec.clone(),
        width: s.stats.width,
        height: s.stats.height,
        fps: s.stats.fps,
        dropped_frames: s.stats.dropped,
        first_frame_ms: s.first_frame_ms,
        error: s.error.clone(),
        mode: s.request.as_ref().map(|r| r.mode),
        buffered_secs: match (s.stats.cache_time, s.stats.position) {
            (Some(c), Some(p)) => Some((c - p).max(0.0)),
            _ => None,
        },
        volume: s.stats.volume,
        muted: s.stats.muted,
        tracks: s.stats.tracks.clone(),
        waiting_on: waiting_on(
            s.sm.state(),
            s.state_since.elapsed(),
            s.file_loaded,
            s.first_frame_ms.is_some(),
        ),
    }
}

fn go(inner: &Inner, s: &mut Shared, to: SessionState, note: Option<String>) -> bool {
    let ok = s.sm.transition(to, note, unix_ms()).is_ok();
    if ok {
        s.state_since = Instant::now();
        s.throttle.mark_dirty();
        inner.cv.notify_all();
    }
    ok
}

/// Issue `loadfile` for the current request (per-file headers and profile options only).
fn load_current(inner: &Inner, s: &mut Shared) -> Result<(), PlayerError> {
    let Some(req) = s.request.clone() else {
        return Err(PlayerError::NotPlaying);
    };
    let prefs = match inner.prefs.lock() {
        Ok(p) => p.clone(),
        Err(p) => p.into_inner().clone(),
    };
    let opts = options::loadfile_options(req.profile, &prefs, &req.headers, req.start_secs);
    s.load_started = Some(Instant::now());
    s.first_frame_ms = None;
    inner
        .engine
        .command(&["loadfile", &req.url, "replace", "-1", &opts])
}

fn on_failure(inner: &Inner, s: &mut Shared, cause: FailureCause) {
    s.load_started = None;
    match s.policy.on_failure(&cause) {
        RetryDecision::RetrySame { after, attempt } => {
            go(
                inner,
                s,
                SessionState::Connecting,
                Some(format!("retry {attempt}: {}", cause.describe())),
            );
            s.retry_at = Some(Instant::now() + after);
        }
        RetryDecision::AskForStreamList { last_error } => {
            s.error = Some(last_error.clone());
            go(inner, s, SessionState::Failed, Some(last_error.clone()));
            s.outbox.push(PlayerEvent::FailoverNeeded {
                session: s.session,
                last_error,
            });
        }
        RetryDecision::GiveUp { last_error } => {
            s.error = Some(last_error.clone());
            go(inner, s, SessionState::Failed, Some(last_error));
        }
    }
}

fn handle_event(inner: &Inner, ev: EngineEvent) {
    let mut s = lock(inner);
    match ev {
        EngineEvent::StartFile => {
            if s.request.is_some() {
                s.file_loaded = false;
                go(inner, &mut s, SessionState::Connecting, None);
            }
        }
        EngineEvent::FileLoaded => {
            if s.request.is_some() {
                s.file_loaded = true;
                go(
                    inner,
                    &mut s,
                    SessionState::Buffering,
                    Some("first data".into()),
                );
            }
        }
        EngineEvent::PlaybackRestart => {
            if s.request.is_some()
                && matches!(
                    s.sm.state(),
                    SessionState::Connecting | SessionState::Buffering
                )
            {
                if let Some(t) = s.load_started.take() {
                    s.first_frame_ms = Some(t.elapsed().as_millis() as u64);
                }
                s.policy.on_playing();
                let target = if s.paused {
                    SessionState::Paused
                } else {
                    SessionState::Playing
                };
                go(inner, &mut s, target, Some("first frame".into()));
            }
        }
        EngineEvent::EndFile { reason, error } => match reason {
            EndReason::Error if s.request.is_some() => {
                let e = if error.is_empty() {
                    "playback failed".to_owned()
                } else {
                    error
                };
                on_failure(inner, &mut s, FailureCause::EndFileError(e));
            }
            EndReason::Eof if s.request.is_some() => {
                if s.request.as_ref().map(|r| r.profile) == Some(Profile::Live) {
                    on_failure(inner, &mut s, FailureCause::UnexpectedEof);
                } else {
                    s.request = None;
                    go(
                        inner,
                        &mut s,
                        SessionState::Stopped,
                        Some("end of media".into()),
                    );
                }
            }
            // Stop/redirect/quit are our own `loadfile replace` and `stop`; nothing to do.
            _ => {}
        },
        EngineEvent::Property { name, value } => handle_property(inner, &mut s, &name, value),
        EngineEvent::Log { level, text, .. } => {
            let text = scrub_urls(text.trim_end());
            if is_audio_open_failure(&text) && !s.audio_fell_back {
                // No usable audio device (VM, headless): stop retrying it on every loadfile.
                s.audio_fell_back = true;
                match inner.engine.set_property("ao", "null") {
                    Ok(()) => tracing::warn!(
                        "no audio device available; continuing without sound (ao=null)"
                    ),
                    Err(e) => tracing::warn!("could not switch to ao=null: {e}"),
                }
            }
            let level = if is_expected_probe_noise(&text)
                || (s.audio_fell_back && level == "error") && is_audio_open_failure(&text)
            {
                "debug".to_owned()
            } else {
                level
            };
            s.outbox.push(PlayerEvent::Log { level, text });
        }
        _ => {}
    }
}

fn handle_property(inner: &Inner, s: &mut Shared, name: &str, value: PropValue) {
    match (name, value) {
        ("pause", PropValue::Flag(p)) => {
            s.paused = p;
            if p && matches!(
                s.sm.state(),
                SessionState::Playing | SessionState::Buffering
            ) {
                go(inner, s, SessionState::Paused, None);
            } else if !p && s.sm.state() == SessionState::Paused {
                go(inner, s, SessionState::Playing, None);
            }
        }
        ("paused-for-cache", PropValue::Flag(b)) => {
            s.cache_buffering = b;
            if b && s.sm.state() == SessionState::Playing {
                go(
                    inner,
                    s,
                    SessionState::Buffering,
                    Some("cache underrun".into()),
                );
            } else if !b && s.sm.state() == SessionState::Buffering && s.load_started.is_none() {
                go(inner, s, SessionState::Playing, None);
            }
            s.throttle.mark_dirty();
        }
        ("time-pos", PropValue::Double(v)) => {
            s.stats.position = Some(v);
            s.throttle.mark_dirty();
        }
        ("duration", v) => {
            s.stats.duration = if let PropValue::Double(d) = v {
                Some(d)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("demuxer-cache-time", v) => {
            s.stats.cache_time = if let PropValue::Double(d) = v {
                Some(d)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("volume", PropValue::Double(v)) => {
            s.stats.volume = Some(v);
            s.throttle.mark_dirty();
        }
        ("mute", PropValue::Flag(m)) => {
            s.stats.muted = m;
            s.throttle.mark_dirty();
        }
        ("track-list", v) => {
            s.stats.tracks = if let PropValue::Str(j) = v {
                parse_tracks(&j)
            } else {
                Vec::new()
            };
            s.throttle.mark_dirty();
        }
        ("hwdec-current", v) => {
            s.stats.hwdec = if let PropValue::Str(t) = v {
                Some(t)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("video-codec", v) => {
            s.stats.codec = if let PropValue::Str(t) = v {
                Some(t)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("dwidth", v) => {
            s.stats.width = if let PropValue::Int(i) = v {
                Some(i)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("dheight", v) => {
            s.stats.height = if let PropValue::Int(i) = v {
                Some(i)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("estimated-vf-fps", v) => {
            s.stats.fps = if let PropValue::Double(d) = v {
                Some(d)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        ("vo-drop-frame-count", v) => {
            s.stats.dropped = if let PropValue::Int(i) = v {
                Some(i)
            } else {
                None
            };
            s.throttle.mark_dirty();
        }
        _ => {}
    }
}

fn tick(inner: &Inner) {
    let mut s = lock(inner);
    let now = Instant::now();
    if let Some(t) = s.retry_at
        && now >= t
    {
        s.retry_at = None;
        if let Err(e) = load_current(inner, &mut s) {
            let msg = e.to_string();
            s.error = Some(msg.clone());
            go(inner, &mut s, SessionState::Failed, Some(msg));
        }
    }
    if let Some(t) = s.load_started
        && now.duration_since(t) >= NO_FRAME_TIMEOUT
        && matches!(
            s.sm.state(),
            SessionState::Connecting | SessionState::Buffering
        )
    {
        on_failure(inner, &mut s, FailureCause::NoFrameTimeout);
    }
    if matches!(
        s.sm.state(),
        SessionState::Connecting | SessionState::Buffering
    ) && s.state_since.elapsed() >= WAITING_AFTER
        && s.state_since.elapsed() < WAITING_AFTER + Duration::from_millis(250)
    {
        // Crossed the 3 s mark: let the UI say what it is waiting on.
        s.throttle.mark_dirty();
    }
    if inner.stats_enabled.load(Ordering::SeqCst)
        && s.last_stats
            .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
    {
        s.last_stats = Some(now);
        let stats = collect_stats(inner, &s);
        s.outbox.push(PlayerEvent::Stats(stats));
    }
    if s.throttle.poll(now.duration_since(inner.epoch)) {
        let snap = snapshot(&s);
        s.outbox.push(PlayerEvent::State(snap));
    }
}

fn collect_stats(inner: &Inner, s: &Shared) -> StatsSnapshot {
    let e = &inner.engine;
    let num = |name: &str| e.get_property(name).and_then(|v| v.parse::<f64>().ok());
    let bitrate = match (num("video-bitrate"), num("audio-bitrate")) {
        (None, None) => None,
        (v, a) => Some(((v.unwrap_or(0.0) + a.unwrap_or(0.0)) / 1000.0).round() as u32),
    };
    StatsSnapshot {
        resolution: match (s.stats.width, s.stats.height) {
            (Some(w), Some(h)) => Some(format!("{w}x{h}")),
            _ => None,
        },
        video_codec: s.stats.codec.clone(),
        audio_codec: e.get_property("audio-codec-name"),
        bitrate_kbps: bitrate,
        dropped_frames: s.stats.dropped,
        buffer_secs: num("demuxer-cache-duration"),
        deinterlace: e.get_property("deinterlace"),
        hwdec_current: s.stats.hwdec.clone(),
        source_index: s.request.as_ref().map_or(0, |r| r.source_index),
    }
}

fn flush(inner: &Inner) {
    let events = std::mem::take(&mut lock(inner).outbox);
    for e in events {
        (inner.sink)(e);
    }
}

fn event_loop(inner: &Arc<Inner>) {
    while !inner.stop.load(Ordering::SeqCst) {
        let ev = inner.engine.wait_event(0.05);
        if ev == EngineEvent::Shutdown {
            break;
        }
        handle_event(inner, ev);
        tick(inner);
        flush(inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_url_or_headers() {
        let r = PlayRequest {
            url: "http://example.invalid/live/user/pass/1.ts".into(),
            headers: vec![("X-API-Key".into(), "sekrit".into())],
            profile: Profile::Live,
            start_secs: None,
            failover_capable: true,
            mode: PlayMode::Live,
            source_index: 0,
        };
        let d = format!("{r:?}");
        assert!(
            !d.contains("example")
                && !d.contains("pass")
                && !d.contains("sekrit")
                && !d.contains("X-API-Key")
        );
    }

    #[test]
    fn log_scrubbing_removes_urls() {
        let s = scrub_urls("Opening http://h/live/u/p/1.ts failed: 403");
        assert_eq!(s, "Opening <url> failed: 403");
    }

    #[test]
    fn classifies_log_noise() {
        assert!(is_expected_probe_noise(
            "AVHWDeviceContext: Cannot load libcuda.so.1"
        ));
        assert!(is_expected_probe_noise(
            "h264: Device does not support the VK_KHR_video_decode_queue extension!"
        ));
        assert!(!is_expected_probe_noise("http: HTTP error 403 Forbidden"));
        assert!(is_audio_open_failure(
            "Could not open/initialize audio device -> no sound."
        ));
        assert!(!is_audio_open_failure("Opening audio decoder"));
    }

    #[test]
    fn waiting_on_rules() {
        let t = Duration::from_secs;
        assert_eq!(
            waiting_on(SessionState::Connecting, t(2), false, false),
            None
        );
        assert_eq!(
            waiting_on(SessionState::Connecting, t(3), false, false),
            Some(WaitingOn::Connection)
        );
        assert_eq!(
            waiting_on(SessionState::Buffering, t(4), true, false),
            Some(WaitingOn::FirstKeyframe)
        );
        assert_eq!(
            waiting_on(SessionState::Buffering, t(4), false, false),
            Some(WaitingOn::Connection)
        );
        assert_eq!(
            waiting_on(SessionState::Buffering, t(4), true, true),
            Some(WaitingOn::FirstData)
        );
        assert_eq!(waiting_on(SessionState::Playing, t(10), true, true), None);
    }

    #[test]
    fn parses_track_list() {
        let j = r#"[{"id":1,"type":"video","codec":"h264","selected":true},
                    {"id":2,"type":"audio","lang":"eng","title":"Main","codec":"ac3","selected":true},
                    {"id":3,"type":"sub","lang":"fra","selected":false}]"#;
        let t = parse_tracks(j);
        assert_eq!(t.len(), 3);
        assert_eq!(t[1].lang.as_deref(), Some("eng"));
        assert!(!t[2].selected);
        assert!(parse_tracks("not json").is_empty());
    }

    #[test]
    fn parses_audio_devices() {
        let j = r#"[{"name":"auto","description":"Autoselect device"},
                    {"name":"alsa/hdmi:CARD=PCH,DEV=0","description":"HDMI"}]"#;
        let d = parse_audio_devices(j);
        assert_eq!(d.len(), 2);
        assert!(!d[0].supports_passthrough);
        assert!(d[1].supports_passthrough);
    }

    #[test]
    fn command_json_shape() {
        let c: PlayerCommand = serde_json::from_str(r#"{"kind":"seekRelative","secs":10}"#)
            .unwrap_or(PlayerCommand::Pause);
        assert_eq!(c, PlayerCommand::SeekRelative { secs: 10.0 });
    }
}
