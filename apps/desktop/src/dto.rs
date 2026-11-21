//! Desktop-layer DTOs and mappings between core types and the UI contract
//! (`apps/ui/src/api/types.ts`). Core owns most shapes; only the ones that differ are mapped here.

use downlink_core::service::ProgrammeDto;
use downlink_core::{AppError, ErrorCode};
use downlink_player::options::{SubtitleBackground, SubtitleSize};
use downlink_player::{
    AudioMode, Deinterlace, FailureInfo, FailureKind, HdrOutput, PlayMode, PlayerSnapshot,
    Preferences, SessionState, WaitingOn,
};
use serde::Serialize;
use serde_json::Value;

pub fn phase_name(s: SessionState) -> &'static str {
    match s {
        SessionState::Idle => "Idle",
        SessionState::Resolving => "Resolving",
        SessionState::Connecting => "Connecting",
        SessionState::Buffering => "Buffering",
        SessionState::Playing => "Playing",
        SessionState::Paused => "Paused",
        SessionState::Stopped => "Stopped",
        SessionState::Failed => "Failed",
    }
}

fn mode_name(m: PlayMode) -> &'static str {
    match m {
        PlayMode::Live => "live",
        PlayMode::Catchup => "catchup",
        PlayMode::Vod => "vod",
        PlayMode::Recording => "recording",
    }
}

fn waiting_name(w: WaitingOn) -> &'static str {
    match w {
        WaitingOn::Connection => "connection",
        WaitingOn::FirstData => "first_data",
        WaitingOn::FirstKeyframe => "first_keyframe",
    }
}

/// `PlayerState` of types.ts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateDto {
    pub state: &'static str,
    pub mode: Option<&'static str>,
    pub channel_id: Option<i64>,
    pub programme: Option<ProgrammeDto>,
    /// Title of VOD / recording media (null for live and catch-up).
    pub title: Option<String>,
    pub resume_at: Option<f64>,
    pub position: Option<f64>,
    pub duration: Option<f64>,
    pub buffered: Option<f64>,
    pub seekable: bool,
    pub volume: f64,
    pub muted: bool,
    pub hwdec_current: Option<String>,
    pub video_codec: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub waiting_on: Option<&'static str>,
    /// Automatic retry or failover in progress: the UI shows "Reconnecting…".
    pub reconnecting: bool,
    pub error: Option<AppError>,
    pub transitioned_at: i64,
    /// Audio and subtitle tracks of the current file (video tracks are left out).
    pub tracks: Vec<TrackDto>,
}

/// `Track` of types.ts: one audio or subtitle track (teletext and DVB subtitles are `sub`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackDto {
    pub id: i64,
    pub kind: &'static str,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
}

pub fn track_dtos(tracks: &[downlink_player::Track]) -> Vec<TrackDto> {
    tracks
        .iter()
        .filter_map(|t| {
            let kind = match t.kind.as_str() {
                "audio" => "audio",
                "sub" => "sub",
                _ => return None,
            };
            Some(TrackDto {
                id: t.id,
                kind,
                lang: t.lang.clone(),
                title: t.title.clone(),
                codec: t.codec.clone(),
                selected: t.selected,
            })
        })
        .collect()
}

/// What the player glue knows about the current session beyond mpv's own state.
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub channel_id: Option<i64>,
    pub mode: Option<PlayMode>,
    pub programme: Option<ProgrammeDto>,
    pub title: Option<String>,
    pub resume_at: Option<f64>,
    pub failure: Option<AppError>,
    /// A server-side failover attempt is in progress.
    pub failover_active: bool,
    pub transitioned_at: i64,
}

pub fn player_state(snap: &PlayerSnapshot, info: &SessionInfo) -> PlayerStateDto {
    let state = snap.state.unwrap_or(SessionState::Idle);
    let mode = info.mode.or(snap.mode);
    let error = info.failure.clone().or_else(|| {
        (state == SessionState::Failed).then(|| match &snap.failure {
            Some(f) => failure_error(f),
            None => AppError::new(ErrorCode::Stream, "Source unavailable. Try again."),
        })
    });
    PlayerStateDto {
        state: phase_name(state),
        mode: mode.map(mode_name),
        channel_id: info.channel_id,
        programme: info.programme.clone(),
        title: info.title.clone(),
        resume_at: info.resume_at,
        position: snap.position_secs,
        duration: snap.duration_secs,
        buffered: snap.buffered_secs,
        seekable: matches!(
            mode,
            Some(PlayMode::Catchup | PlayMode::Vod | PlayMode::Recording)
        ) && snap.duration_secs.is_some(),
        volume: snap.volume.unwrap_or(100.0),
        muted: snap.muted,
        hwdec_current: snap.hwdec_current.clone(),
        video_codec: snap.video_codec.clone(),
        width: snap.width,
        height: snap.height,
        waiting_on: snap.waiting_on.map(waiting_name),
        reconnecting: snap.reconnecting || (info.failover_active && state != SessionState::Playing),
        error,
        transitioned_at: info.transitioned_at,
        tracks: track_dtos(&snap.tracks),
    }
}

/// PLAN §4.9 copy and detail fields for a classified failure.
pub fn failure_error(f: &FailureInfo) -> AppError {
    match f.kind {
        FailureKind::Limit => {
            let msg = if f.http_status == Some(503) {
                "No stream is available on your server right now."
            } else {
                "Your server is at its stream limit."
            };
            AppError::new(ErrorCode::Stream, msg).with_detail("limit", None, None)
        }
        FailureKind::Codec => {
            let codec = f.codec.clone().unwrap_or_else(|| "unknown".to_owned());
            AppError::new(
                ErrorCode::Media,
                format!("This channel uses a format Downlink can't decode: {codec}"),
            )
            .with_detail("codec", Some(codec), None)
        }
        FailureKind::Engine => AppError::new(
            ErrorCode::Media,
            "The playback engine stopped unexpectedly.",
        )
        .with_detail("engine", None, f.crash_id.clone()),
        FailureKind::Stream => AppError::new(ErrorCode::Stream, "Source unavailable. Try again."),
    }
}

/// UI `AppSettings` (types.ts) -> player preferences. Unknown or missing fields keep defaults.
pub fn preferences_from_settings(v: &Value) -> Preferences {
    let mut p = Preferences {
        user_agent: downlink_core::user_agent().to_owned(),
        ..Preferences::default()
    };
    if let Some(n) = v.get("liveBufferSecs").and_then(Value::as_f64) {
        p.live_buffer_secs = n.round().clamp(1.0, 10.0) as u32;
    }
    if let Some(h) = v.get("hwdec").and_then(Value::as_str) {
        p.hwdec = h.to_owned();
    }
    p.audio_device = v
        .get("audioDevice")
        .and_then(Value::as_str)
        .map(str::to_owned);
    p.audio_mode = match v.get("audioOutput").and_then(Value::as_str) {
        Some("stereo") => AudioMode::Stereo,
        Some("passthrough") => AudioMode::Passthrough,
        _ => AudioMode::Surround,
    };
    p.deinterlace = match v.get("deinterlace").and_then(Value::as_str) {
        Some("yes") => Deinterlace::On,
        Some("no") => Deinterlace::Off,
        _ => Deinterlace::Auto,
    };
    p.hdr_output = match v.get("hdrOutput").and_then(Value::as_str) {
        Some("off") => HdrOutput::Off,
        _ => HdrOutput::Auto,
    };
    if let Some(pct) = v.get("subtitleSize").and_then(Value::as_f64) {
        p.subtitle.size = if pct < 85.0 {
            SubtitleSize::Small
        } else if pct > 120.0 {
            SubtitleSize::Large
        } else {
            SubtitleSize::Medium
        };
    }
    if let Some(bg) = v.get("subtitleBackground").and_then(Value::as_bool) {
        p.subtitle.background = if bg {
            SubtitleBackground::Dim
        } else {
            SubtitleBackground::None
        };
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_preferences() {
        let v = serde_json::json!({
            "liveBufferSecs": 20, "hwdec": "vaapi", "audioOutput": "passthrough",
            "audioDevice": "alsa/hdmi:X", "deinterlace": "no", "subtitleSize": 150, "subtitleBackground": false,
            "hdrOutput": "off"
        });
        let p = preferences_from_settings(&v);
        assert_eq!(p.live_buffer_secs, 10);
        assert_eq!(p.hwdec, "vaapi");
        assert_eq!(p.audio_mode, AudioMode::Passthrough);
        assert_eq!(p.deinterlace, Deinterlace::Off);
        assert_eq!(p.hdr_output, HdrOutput::Off);
        // Settings saved before the option existed keep HDR on Auto.
        assert_eq!(
            preferences_from_settings(&serde_json::json!({})).hdr_output,
            HdrOutput::Auto
        );
        assert_eq!(p.subtitle.size, SubtitleSize::Large);
        assert_eq!(p.subtitle.background, SubtitleBackground::None);
        assert_eq!(p.audio_device.as_deref(), Some("alsa/hdmi:X"));
    }

    #[test]
    fn phases_are_pascal_case() {
        assert_eq!(phase_name(SessionState::Buffering), "Buffering");
        assert_eq!(waiting_name(WaitingOn::FirstKeyframe), "first_keyframe");
    }

    #[test]
    fn recording_statuses_match_the_ui_union() {
        use downlink_core::source::RecStatus;
        for st in [
            RecStatus::Scheduled,
            RecStatus::Recording,
            RecStatus::Completed,
            RecStatus::Failed,
        ] {
            assert!(["scheduled", "recording", "completed", "failed"].contains(&st.as_str()));
        }
    }

    #[test]
    fn vod_and_recording_state_carry_title_mode_and_seekability() {
        let snap = PlayerSnapshot {
            state: Some(SessionState::Playing),
            duration_secs: Some(5400.0),
            position_secs: Some(12.0),
            ..PlayerSnapshot::default()
        };
        for (mode, name) in [(PlayMode::Vod, "vod"), (PlayMode::Recording, "recording")] {
            let info = SessionInfo {
                mode: Some(mode),
                title: Some("A title".into()),
                resume_at: Some(600.0),
                ..SessionInfo::default()
            };
            let v = serde_json::to_value(player_state(&snap, &info)).unwrap_or_default();
            assert_eq!(v["mode"], name);
            assert_eq!(v["title"], "A title");
            assert_eq!(v["seekable"], true);
            assert_eq!(v["resumeAt"], 600.0);
            assert!(v["channelId"].is_null());
        }
        // Live is never seekable and has no title.
        let live = SessionInfo {
            mode: Some(PlayMode::Live),
            ..SessionInfo::default()
        };
        let v = serde_json::to_value(player_state(&snap, &live)).unwrap_or_default();
        assert_eq!(v["seekable"], false);
        assert!(v["title"].is_null());
    }

    #[test]
    fn failures_map_to_plain_copy_and_detail_fields() {
        let f = |kind, http_status, codec: Option<&str>, crash: Option<&str>| FailureInfo {
            kind,
            http_status,
            codec: codec.map(str::to_owned),
            crash_id: crash.map(str::to_owned),
        };
        let e = failure_error(&f(FailureKind::Limit, Some(429), None, None));
        assert_eq!(
            (e.reason.as_deref(), e.code),
            (Some("limit"), ErrorCode::Stream)
        );
        assert!(e.message.contains("stream limit"));
        let e = failure_error(&f(FailureKind::Codec, None, Some("av1"), None));
        assert_eq!(
            (e.reason.as_deref(), e.codec.as_deref(), e.code),
            (Some("codec"), Some("av1"), ErrorCode::Media)
        );
        assert!(e.message.contains("av1"));
        let e = failure_error(&f(FailureKind::Engine, None, None, Some("ab12cd34")));
        assert_eq!(
            (e.reason.as_deref(), e.crash_id.as_deref()),
            (Some("engine"), Some("ab12cd34"))
        );
        assert!(
            failure_error(&f(FailureKind::Stream, None, None, None))
                .reason
                .is_none()
        );
    }

    #[test]
    fn reconnecting_during_retry_or_failover() {
        let retry = PlayerSnapshot {
            state: Some(SessionState::Connecting),
            reconnecting: true,
            ..PlayerSnapshot::default()
        };
        let v =
            serde_json::to_value(player_state(&retry, &SessionInfo::default())).unwrap_or_default();
        assert_eq!(v["reconnecting"], true);
        let fo = PlayerSnapshot {
            state: Some(SessionState::Connecting),
            ..PlayerSnapshot::default()
        };
        let info = SessionInfo {
            failover_active: true,
            ..SessionInfo::default()
        };
        assert_eq!(
            serde_json::to_value(player_state(&fo, &info)).unwrap_or_default()["reconnecting"],
            true
        );
        let playing = PlayerSnapshot {
            state: Some(SessionState::Playing),
            ..PlayerSnapshot::default()
        };
        assert_eq!(
            serde_json::to_value(player_state(&playing, &info)).unwrap_or_default()["reconnecting"],
            false
        );
    }
}
