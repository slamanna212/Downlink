//! Desktop-layer DTOs and mappings between core types and the UI contract
//! (`apps/ui/src/api/types.ts`). Core owns most shapes; only the ones that differ are mapped here.

use downlink_core::catalog;
use downlink_core::service::{ChannelDto, HomeDto, ProgrammeDto, SearchDto, Service};
use downlink_core::{AppError, ErrorCode};
use downlink_player::options::{SubtitleBackground, SubtitleSize};
use downlink_player::{
    AudioMode, Deinterlace, PlayMode, PlayerSnapshot, Preferences, SessionState, WaitingOn,
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
    pub error: Option<AppError>,
    pub transitioned_at: i64,
}

/// What the player glue knows about the current session beyond mpv's own state.
#[derive(Debug, Clone, Default)]
pub struct SessionInfo {
    pub channel_id: Option<i64>,
    pub mode: Option<PlayMode>,
    pub programme: Option<ProgrammeDto>,
    pub resume_at: Option<f64>,
    pub failure: Option<AppError>,
    pub transitioned_at: i64,
}

pub fn player_state(snap: &PlayerSnapshot, info: &SessionInfo) -> PlayerStateDto {
    let state = snap.state.unwrap_or(SessionState::Idle);
    let mode = info.mode.or(snap.mode);
    let error = info.failure.clone().or_else(|| {
        (state == SessionState::Failed)
            .then(|| AppError::new(ErrorCode::Stream, "Source unavailable. Try again."))
    });
    PlayerStateDto {
        state: phase_name(state),
        mode: mode.map(mode_name),
        channel_id: info.channel_id,
        programme: info.programme.clone(),
        resume_at: info.resume_at,
        position: snap.position_secs,
        duration: snap.duration_secs,
        buffered: snap.buffered_secs,
        seekable: matches!(mode, Some(PlayMode::Catchup | PlayMode::Vod))
            && snap.duration_secs.is_some(),
        volume: snap.volume.unwrap_or(100.0),
        muted: snap.muted,
        hwdec_current: snap.hwdec_current.clone(),
        video_codec: snap.video_codec.clone(),
        width: snap.width,
        height: snap.height,
        waiting_on: snap.waiting_on.map(waiting_name),
        error,
        transitioned_at: info.transitioned_at,
    }
}

/// UI `AppSettings` (types.ts) -> player preferences. Unknown or missing fields keep defaults.
pub fn preferences_from_settings(v: &Value) -> Preferences {
    let mut p = Preferences::default();
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

// ---- Home / search: core DTOs differ from the UI contract -----------------------------------

async fn channel_dto(service: &Service, id: i64) -> Option<ChannelDto> {
    let row = service.store().channel(id).await.ok()??;
    // Logo is not resolved here (core exposes no by-id DTO builder yet).
    Some(ChannelDto::from_row(&row, None, false))
}

pub async fn home_to_ui(service: &Service, h: HomeDto) -> Value {
    let mut cont = Vec::new();
    for c in &h.continue_watching {
        let channel = match c.channel_id {
            Some(id) => channel_dto(service, id).await,
            None => None,
        };
        let kind = match c.kind.as_str() {
            k @ ("live" | "catchup" | "movie" | "episode" | "recording") => k,
            _ => "movie",
        };
        let progress = c
            .duration_ms
            .filter(|d| *d > 0)
            .map(|d| (c.position_ms as f64 / d as f64).clamp(0.0, 1.0));
        let remaining = c
            .duration_ms
            .map(|d| ((d - c.position_ms).max(0) / 1000) as f64);
        cont.push(serde_json::json!({
            "kind": kind,
            "id": c.programme_id.clone().unwrap_or_else(|| c.item_key.clone()),
            "channel": channel,
            "programme": null,
            "title": c.title.clone().unwrap_or_default(),
            "subtitle": null,
            "art": null,
            "progress": progress,
            "remainingSecs": remaining,
        }));
    }
    let on_now: Vec<Value> = h
        .on_now
        .iter()
        .map(|o| serde_json::json!({ "channel": o.channel, "now": o.programme }))
        .collect();
    serde_json::json!({
        "continueWatching": cont,
        "favourites": h.favourites,
        "onNow": on_now,
        "recents": h.recents,
    })
}

pub async fn search_to_ui(service: &Service, s: SearchDto) -> Value {
    let mut hits = Vec::new();
    for p in &s.programmes {
        let ch = match s.channels.iter().find(|c| c.id == p.channel_id) {
            Some(c) => Some(c.clone()),
            None => channel_dto(service, p.channel_id).await,
        };
        if let Some(channel) = ch {
            hits.push(serde_json::json!({ "programme": p, "channel": channel }));
        }
    }
    serde_json::json!({ "channels": s.channels, "programmes": hits })
}

/// Visible-channel count for `get_counts` with core's default filter (adult/radio/DRM hidden).
pub async fn all_count(service: &Service) -> i64 {
    catalog::count_channels(service.store(), &catalog::ChannelFilter::default())
        .await
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_preferences() {
        let v = serde_json::json!({
            "liveBufferSecs": 20, "hwdec": "vaapi", "audioOutput": "passthrough",
            "audioDevice": "alsa/hdmi:X", "deinterlace": "no", "subtitleSize": 150, "subtitleBackground": false
        });
        let p = preferences_from_settings(&v);
        assert_eq!(p.live_buffer_secs, 10);
        assert_eq!(p.hwdec, "vaapi");
        assert_eq!(p.audio_mode, AudioMode::Passthrough);
        assert_eq!(p.deinterlace, Deinterlace::Off);
        assert_eq!(p.subtitle.size, SubtitleSize::Large);
        assert_eq!(p.subtitle.background, SubtitleBackground::None);
        assert_eq!(p.audio_device.as_deref(), Some("alsa/hdmi:X"));
    }

    #[test]
    fn phases_are_pascal_case() {
        assert_eq!(phase_name(SessionState::Buffering), "Buffering");
        assert_eq!(waiting_name(WaitingOn::FirstKeyframe), "first_keyframe");
    }
}
