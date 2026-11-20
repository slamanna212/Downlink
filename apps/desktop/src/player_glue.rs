//! Glue between the core `Service` (play plans, catch-up sessions) and the `Player`.
//!
//! Rules enforced here: play plans come from SQLite (the one allowed play-time request is the
//! redirect probe inside `Service::play_channel`), auth headers go to mpv per file only, and every
//! catch-up session is stopped on zap, navigate-away, sleep and quit (CLAUDE.md rules 4-6).

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use downlink_core::catchup::{CatchupPlay, PositionReporter};
use downlink_core::service::{ProgrammeDto, Service};
use downlink_core::source::{Caps, PlayPlan, PlayProfile};
use downlink_core::{AppError, ErrorCode};
use downlink_platform::{MediaSession, MediaState, PlaybackInhibitor};
use downlink_player::{
    PlayMode, PlayRequest, Player, PlayerCommand, PlayerEvent, PlayerSnapshot, Profile,
    SessionState,
};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

use crate::dto::{SessionInfo, player_state};

#[derive(Debug, Clone)]
pub enum LastPlay {
    Vod { kind: String, id: i64 },
    Recording(i64),
    Channel(i64),
    Catchup(String),
    Restart(i64),
}

#[derive(Default)]
struct Active {
    info: SessionInfo,
    catchup: Option<CatchupPlay>,
    last: Option<LastPlay>,
    last_phase: Option<SessionState>,
    /// Bumped on every new playback so background reporters of old ones exit.
    generation: u64,
    /// Server-side failover attempts since the user's last play (PLAN §4.5).
    failover_count: u32,
    /// The current live plan is an opt-in HLS session (minted URL, no headers).
    live_hls: bool,
    /// The one allowed re-mint after a 410 / dead session was used.
    hls_reminted: bool,
    /// VOD / recording being played, for position reports.
    media: Option<Media>,
    /// Loopback playlist proxy for the current recording (aborted when dropped).
    hls: Option<crate::hls_proxy::HlsProxy>,
    /// 1-based provider stream in use and how many the channel has (0 = unknown).
    source_index: u32,
    source_count: u32,
}

#[derive(Debug, Clone, Copy)]
enum Media {
    Vod(i64),
    Recording(i64),
}

pub struct Playback {
    service: Arc<Service>,
    player: Arc<Player>,
    active: Arc<Mutex<Active>>,
    /// Multiview tiles (PLAN §4.11); set once at boot.
    pub multiview: OnceLock<Arc<crate::multiview::Multiview>>,
}

fn lock(a: &Mutex<Active>) -> MutexGuard<'_, Active> {
    match a.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

pub(crate) fn plan_request(plan: &PlayPlan, mode: PlayMode, failover: bool) -> PlayRequest {
    PlayRequest {
        url: plan.url.clone(),
        headers: plan
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.expose().to_owned()))
            .collect(),
        profile: match plan.profile {
            PlayProfile::Live => Profile::Live,
            PlayProfile::Seekable => Profile::Seekable,
        },
        start_secs: None,
        failover_capable: failover,
        mode,
        source_index: 0,
    }
}

async fn report_media(svc: &Service, m: Media, pos: f64, duration: Option<f64>) {
    let (pos, dur) = (pos.max(0.0) as i64, duration.map(|d| d as i64));
    let r = match m {
        Media::Vod(id) => svc.report_vod_position(id, pos, dur).await,
        Media::Recording(id) => svc.report_recording_position(id, pos, dur).await,
    };
    if let Err(e) = r {
        tracing::debug!("position report failed: {}", e.message);
    }
}

fn lock_plain<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn player_err(e: downlink_player::PlayerError) -> AppError {
    AppError::new(ErrorCode::Media, e.to_string())
}

/// What `Player::new` needs for its sink; created before the `Player` exists.
/// OS integrations driven by player state: screen-awake inhibitor and the media session.
#[derive(Default)]
pub struct Extras {
    pub inhibitor: PlaybackInhibitor,
    pub media: OnceLock<MediaSession>,
    /// Set once the player exists; the failover loop needs it from the event sink.
    pub player: OnceLock<Arc<Player>>,
    last_title: Mutex<String>,
}

#[derive(Clone)]
pub struct SinkContext {
    active: Arc<Mutex<Active>>,
    app: AppHandle,
    service: Arc<Service>,
    pub extras: Arc<Extras>,
}

impl SinkContext {
    pub fn new(app: AppHandle, service: Arc<Service>) -> Self {
        Self {
            active: Arc::new(Mutex::new(Active::default())),
            app,
            service,
            extras: Arc::new(Extras::default()),
        }
    }

    /// PLAN §4.5, after the player's own same-URL reconnect failed: ask the core for the next
    /// provider stream (attempts 2 and 3), play it, and surface a plain failure after that.
    /// The state stays visible throughout (Resolving, then Connecting).
    async fn run_failover(&self) {
        let Some(player) = self.extras.player.get().cloned() else {
            return;
        };
        // Live HLS: a dead session (410) is fixed by minting a new one, once per play.
        let remint = {
            let mut a = lock(&self.active);
            let r = a.live_hls && !a.hls_reminted && a.info.mode == Some(PlayMode::Live);
            if r {
                a.hls_reminted = true;
                a.info.failure = None;
                a.info.failover_active = true;
            }
            r.then_some(a.info.channel_id).flatten()
        };
        if let Some(channel) = remint {
            player.begin_resolving();
            match self.service.remint_hls(channel).await {
                Ok(plan) => {
                    tracing::info!("live HLS session re-minted after a failure");
                    if let Err(e) = player.play(plan_request(&plan, PlayMode::Live, true)) {
                        self.publish_failure(
                            &player,
                            AppError::new(ErrorCode::Media, e.to_string()),
                        );
                    }
                }
                Err(e) => self.publish_failure(&player, e),
            }
            return;
        }
        let (channel, attempt, live) = {
            let mut a = lock(&self.active);
            a.failover_count += 1;
            (
                a.info.channel_id,
                a.failover_count + 1,
                a.info.mode == Some(PlayMode::Live),
            )
        };
        let Some(channel) = channel.filter(|_| live) else {
            self.publish_failure(
                &player,
                AppError::new(ErrorCode::Stream, "Source unavailable. Try again."),
            );
            return;
        };
        {
            let mut a = lock(&self.active);
            a.info.failure = None;
            a.info.failover_active = true;
        }
        player.begin_resolving();
        match self.service.failover_next(channel, attempt).await {
            Ok(step) => {
                {
                    let mut a = lock(&self.active);
                    a.source_index = u32::try_from(step.source_index).unwrap_or(0);
                    a.source_count = u32::try_from(step.source_count).unwrap_or(0);
                }
                tracing::info!(
                    "failover attempt {attempt}: stream {}/{}{}",
                    step.source_index,
                    step.source_count,
                    if step.switched {
                        " (server switched)"
                    } else {
                        ""
                    }
                );
                if let Err(e) = player.play(plan_request(&step.plan, PlayMode::Live, true)) {
                    self.publish_failure(&player, AppError::new(ErrorCode::Media, e.to_string()));
                }
            }
            Err(e) => self.publish_failure(&player, e),
        }
    }

    fn publish_failure(&self, player: &Player, e: AppError) {
        // Keep a specific reason (limit, codec, engine) over core's generic "no working stream".
        let e = player
            .snapshot()
            .failure
            .filter(|f| f.kind != downlink_player::FailureKind::Stream)
            .map_or(e, |f| crate::dto::failure_error(&f));
        lock(&self.active).info.failover_active = false;
        lock(&self.active).info.failure = Some(e.clone());
        player.mark_failed(e.message);
        let dto = {
            let a = lock(&self.active);
            player_state(&player.snapshot(), &a.info)
        };
        let _ = self.app.emit("player://state", dto);
    }

    /// The event sink: `player://state` (already throttled by the player), `session://stats`.
    pub fn sink(&self) -> downlink_player::EventSink {
        let ctx = self.clone();
        Arc::new(move |e| match e {
            PlayerEvent::State(snap) => {
                let (dto, title) = {
                    let mut a = lock(&ctx.active);
                    if snap.state != a.last_phase {
                        a.last_phase = snap.state;
                        a.info.transitioned_at = now_ms();
                    }
                    let title = a
                        .info
                        .programme
                        .as_ref()
                        .and_then(|p| p.title.clone())
                        .unwrap_or_else(|| "Live TV".to_owned());
                    (player_state(&snap, &a.info), title)
                };
                let phase = snap.state.unwrap_or(SessionState::Idle);
                if phase == SessionState::Playing {
                    let mut a = lock(&ctx.active);
                    a.failover_count = 0;
                    a.info.failover_active = false;
                }
                ctx.extras
                    .inhibitor
                    .set_active(phase == SessionState::Playing);
                if let Some(m) = ctx.extras.media.get() {
                    let mut last = lock_plain(&ctx.extras.last_title);
                    if *last != title && phase != SessionState::Idle {
                        m.set_now_playing(&title, None, None);
                        last.clone_from(&title);
                    }
                    m.set_state(
                        match phase {
                            SessionState::Playing | SessionState::Buffering => MediaState::Playing,
                            SessionState::Paused => MediaState::Paused,
                            _ => MediaState::Stopped,
                        },
                        snap.position_secs
                            .filter(|p| *p >= 0.0)
                            .map(Duration::from_secs_f64),
                    );
                }
                let _ = ctx.app.emit("player://state", dto);
            }
            PlayerEvent::Stats(s) => {
                let (idx, count) = {
                    let a = lock(&ctx.active);
                    let nz = |n: u32| (n > 0).then_some(n);
                    (nz(a.source_index), nz(a.source_count))
                };
                let _ = ctx.app.emit(
                    "session://stats",
                    serde_json::json!({
                        "resolution": s.resolution,
                        "videoCodec": s.video_codec,
                        "audioCodec": s.audio_codec,
                        "bitrateKbps": s.bitrate_kbps,
                        "droppedFrames": s.dropped_frames,
                        "bufferSecs": s.buffer_secs,
                        "deinterlace": s.deinterlace,
                        "hwdecCurrent": s.hwdec_current,
                        "sourceIndex": idx,
                        "sourceCount": count,
                        "fps": s.fps,
                        "hdr": s.hdr,
                        "transfer": s.transfer,
                        "toneMapped": s.tone_mapped,
                        "primaries": s.primaries,
                        "bitDepth": s.bit_depth,
                        "audioChannels": s.audio_channels,
                        "audioSampleRate": s.audio_sample_rate,
                        "downloadKbps": s.download_kbps,
                        "avsyncMs": s.avsync_ms,
                        "container": s.container,
                    }),
                );
            }
            PlayerEvent::FailoverNeeded { last_error, .. } => {
                tracing::warn!("playback failed after a reconnect: {last_error}");
                let c = ctx.clone();
                tauri::async_runtime::spawn(async move { c.run_failover().await });
            }
            PlayerEvent::Log { level, text } => match level.as_str() {
                "debug" | "v" | "trace" => tracing::debug!(target: "mpv", "{text}"),
                "error" | "fatal" => tracing::warn!(target: "mpv", "{text}"),
                _ => tracing::info!(target: "mpv", "{text}"),
            },
        })
    }
}

impl Playback {
    pub fn new(service: Arc<Service>, player: Arc<Player>, ctx: &SinkContext) -> Arc<Self> {
        Arc::new(Self {
            service,
            player,
            active: ctx.active.clone(),
            multiview: OnceLock::new(),
        })
    }

    pub fn snapshot_dto(&self) -> crate::dto::PlayerStateDto {
        let snap = self.player.snapshot();
        let a = lock(&self.active);
        player_state(&snap, &a.info)
    }

    #[cfg(feature = "e2e")]
    pub fn snapshot_wait(
        &self,
        pred: impl Fn(SessionState) -> bool,
        t: Duration,
    ) -> PlayerSnapshot {
        self.player.wait_for(pred, t)
    }

    pub fn snapshot(&self) -> PlayerSnapshot {
        self.player.snapshot()
    }

    /// Notable player events (reconnects, HTTP classifications, decoder and engine failures).
    pub fn notes(&self) -> Vec<String> {
        self.player.notes()
    }

    pub fn transitions(&self) -> Vec<downlink_player::Transition> {
        self.player.history()
    }

    fn to_request(&self, plan: &PlayPlan, mode: PlayMode, failover: bool) -> PlayRequest {
        plan_request(plan, mode, failover)
    }

    async fn failover_capable(&self, source_id: i64) -> bool {
        match self.service.store().source(source_id).await {
            Ok(r) => Caps::from_db(r.caps).contains(Caps::FAILOVER),
            Err(_) => false,
        }
    }

    /// Stop the previous catch-up session in the background (zap must not wait for it).
    fn stop_previous_catchup(&self) {
        let prev = lock(&self.active).catchup.take();
        if let Some(play) = prev {
            let svc = self.service.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = svc.stop_catchup(&play).await {
                    tracing::warn!("could not stop the catch-up session: {}", e.message);
                }
            });
        }
    }

    /// Release the live HLS session in the background (navigate-away, non-live playback).
    fn stop_live_hls_bg(&self) {
        let svc = self.service.clone();
        tauri::async_runtime::spawn(async move { svc.stop_live_hls().await });
    }

    fn begin(
        &self,
        channel_id: Option<i64>,
        mode: PlayMode,
        programme: Option<ProgrammeDto>,
        last: LastPlay,
    ) {
        self.stop_previous_catchup();
        if mode != PlayMode::Live {
            self.stop_live_hls_bg();
        }
        let mut a = lock(&self.active);
        a.generation += 1;
        a.failover_count = 0;
        a.live_hls = false;
        a.hls_reminted = false;
        a.media = None;
        a.hls = None;
        a.source_index = 0;
        a.source_count = 0;
        a.info = SessionInfo {
            channel_id,
            mode: Some(mode),
            programme,
            title: None,
            resume_at: None,
            failure: None,
            failover_active: false,
            transitioned_at: now_ms(),
        };
        a.last = Some(last);
        drop(a);
        self.player.begin_resolving();
    }

    fn fail(&self, e: &AppError) {
        lock(&self.active).info.failure = Some(e.clone());
        self.player.mark_failed(e.message.clone());
    }

    pub async fn play_channel(&self, channel_id: i64) -> Result<(), AppError> {
        self.begin(
            Some(channel_id),
            PlayMode::Live,
            None,
            LastPlay::Channel(channel_id),
        );
        let plan = match self.service.play_channel(channel_id).await {
            Ok(p) => p,
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        lock(&self.active).live_hls = plan.surface == downlink_core::source::Surface::Hls;
        let source_id = self
            .service
            .store()
            .channel(channel_id)
            .await
            .ok()
            .flatten()
            .map(|c| c.source_id);
        let failover = match source_id {
            Some(s) => self.failover_capable(s).await,
            None => false,
        };
        self.player
            .play(self.to_request(&plan, PlayMode::Live, failover))
            .map_err(player_err)
            .inspect_err(|e| self.fail(e))?;
        // Zap path: nothing above awaits the programme lookup; fill it in after loadfile went out.
        let programme = self
            .service
            .get_now_next(&[channel_id])
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
            .and_then(|nn| nn.now);
        lock(&self.active).info.programme = programme;
        if let Ok((i, c)) = self.service.source_position(channel_id).await {
            let mut a = lock(&self.active);
            a.source_index = u32::try_from(i).unwrap_or(0);
            a.source_count = u32::try_from(c).unwrap_or(0);
        }
        Ok(())
    }

    async fn start_catchup(
        &self,
        play: CatchupPlay,
        programme: Option<ProgrammeDto>,
        last: LastPlay,
        resume: bool,
    ) -> Result<(), AppError> {
        let start_secs = if resume {
            self.catchup_resume_secs(&play).await
        } else {
            None
        };
        self.begin(Some(play.channel_id), PlayMode::Catchup, programme, last);
        let failover = self.failover_capable(play.source_id).await;
        let mut req = self.to_request(&play.plan, PlayMode::Catchup, failover);
        req.start_secs = start_secs;
        let generation = {
            let mut a = lock(&self.active);
            a.catchup = Some(play.clone());
            a.generation
        };
        if let Err(e) = self.player.play(req) {
            let e = player_err(e);
            self.fail(&e);
            let _ = self.service.stop_catchup(&play).await;
            lock(&self.active).catchup = None;
            return Err(e);
        }
        self.spawn_reporter(play, generation);
        Ok(())
    }

    /// Reports position to the server (throttled by `PositionReporter`; forced on pause/seek).
    fn spawn_reporter(&self, play: CatchupPlay, generation: u64) {
        let svc = self.service.clone();
        let player = self.player.clone();
        let active = self.active.clone();
        tauri::async_runtime::spawn(async move {
            let mut reporter = PositionReporter::new(play.session.clone());
            let mut last_phase: Option<SessionState> = None;
            let mut last_pos: Option<f64> = None;
            let start = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if lock(&active).generation != generation {
                    break;
                }
                let snap = player.snapshot();
                let (Some(state), Some(pos)) = (snap.state, snap.position_secs) else {
                    continue;
                };
                if !matches!(state, SessionState::Playing | SessionState::Paused) {
                    continue;
                }
                let jumped = last_pos.is_some_and(|p| (pos - p).abs() > 5.0);
                let force = last_phase.is_some_and(|p| p != state) || jumped;
                last_phase = Some(state);
                last_pos = Some(pos);
                let now = start.elapsed().as_millis() as u64;
                if let Err(e) = svc
                    .report_catchup_position(&play, &mut reporter, now, pos.max(0.0) as u64, force)
                    .await
                {
                    tracing::debug!("position report failed: {}", e.message);
                }
            }
        });
    }

    /// Saved catch-up position (seconds) for this programme, when one is resumable.
    async fn catchup_resume_secs(&self, play: &CatchupPlay) -> Option<f64> {
        let ch = self.service.store().channel(play.channel_id).await.ok()??;
        let key = downlink_core::catchup::resume_key(&ch.ext_id, play.start_utc);
        let r = self
            .service
            .store()
            .resume(play.source_id, "catchup", &key)
            .await
            .ok()??;
        r.resumable().then_some((r.position_ms / 1000) as f64)
    }

    /// `resume`: start at the saved position instead of the programme start.
    pub async fn play_catchup(&self, programme_id: &str, resume: bool) -> Result<(), AppError> {
        let play = self.service.play_catchup(programme_id).await?;
        let programme = self.programme_dto(play.channel_id, play.start_utc).await;
        self.start_catchup(
            play,
            programme,
            LastPlay::Catchup(programme_id.to_owned()),
            resume,
        )
        .await
    }

    pub async fn restart_programme(&self, channel_id: i64) -> Result<(), AppError> {
        let play = self.service.restart_programme(channel_id).await?;
        let programme = self.programme_dto(play.channel_id, play.start_utc).await;
        self.start_catchup(play, programme, LastPlay::Restart(channel_id), false)
            .await
    }

    async fn report_media(&self, m: Media, pos: f64, duration: Option<f64>) {
        report_media(&self.service, m, pos, duration).await;
    }

    pub async fn play_vod(&self, kind: &str, id: i64, resume: bool) -> Result<(), AppError> {
        self.begin(
            None,
            PlayMode::Vod,
            None,
            LastPlay::Vod {
                kind: kind.to_owned(),
                id,
            },
        );
        let vp = match self
            .service
            .play_vod(downlink_core::service::PlayVodArgs {
                kind: kind.to_owned(),
                id,
                resume,
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        let start = vp.resume_secs.map(|s| s as f64);
        self.start_media(&vp.plan, PlayMode::Vod, vp.title, start, Media::Vod(vp.id))
    }

    pub async fn play_recording(&self, id: i64) -> Result<(), AppError> {
        self.begin(None, PlayMode::Recording, None, LastPlay::Recording(id));
        let rp = match self.service.play_recording(id).await {
            Ok(v) => v,
            Err(e) => {
                self.fail(&e);
                return Err(e);
            }
        };
        let start = rp.resume_secs.map(|s| s as f64);
        let mut plan = rp.plan.clone();
        if plan.surface == downlink_core::source::Surface::Hls
            && let Some((local, proxy)) = crate::hls_proxy::prepare(&plan).await
        {
            plan.url = local;
            lock(&self.active).hls = Some(proxy);
        }
        self.start_media(
            &plan,
            PlayMode::Recording,
            rp.title,
            start,
            Media::Recording(rp.id),
        )
    }

    fn start_media(
        &self,
        plan: &PlayPlan,
        mode: PlayMode,
        title: String,
        start_secs: Option<f64>,
        media: Media,
    ) -> Result<(), AppError> {
        let generation = {
            let mut a = lock(&self.active);
            a.info.title = Some(title);
            a.info.resume_at = start_secs;
            a.media = Some(media);
            a.generation
        };
        let mut req = self.to_request(plan, mode, false);
        req.start_secs = start_secs;
        self.player
            .play(req)
            .map_err(player_err)
            .inspect_err(|e| self.fail(e))?;
        self.spawn_media_reporter(media, generation);
        Ok(())
    }

    /// Saves the watch position every 10 s and right away on pause, resume or seek.
    fn spawn_media_reporter(&self, media: Media, generation: u64) {
        let svc = self.service.clone();
        let player = self.player.clone();
        let active = self.active.clone();
        tauri::async_runtime::spawn(async move {
            let (mut last_phase, mut last_pos): (Option<SessionState>, Option<f64>) = (None, None);
            let mut last_report = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if lock(&active).generation != generation {
                    break;
                }
                let snap = player.snapshot();
                let (Some(state), Some(pos)) = (snap.state, snap.position_secs) else {
                    continue;
                };
                if !matches!(state, SessionState::Playing | SessionState::Paused) {
                    continue;
                }
                let jumped = last_pos.is_some_and(|p| (pos - p).abs() > 5.0);
                let force = last_phase.is_some_and(|p| p != state) || jumped;
                last_phase = Some(state);
                last_pos = Some(pos);
                if force || last_report.elapsed() >= Duration::from_secs(10) {
                    last_report = std::time::Instant::now();
                    report_media(&svc, media, pos, snap.duration_secs).await;
                }
            }
        });
    }

    async fn programme_dto(&self, channel_id: i64, start_utc: i64) -> Option<ProgrammeDto> {
        let w = downlink_core::source::TimeWindow {
            start_utc,
            end_utc: start_utc + 1,
        };
        let rows = downlink_core::epg::window(self.service.store(), &[channel_id], w)
            .await
            .ok()?;
        rows.iter()
            .find(|r| r.start_utc == start_utc)
            .map(ProgrammeDto::from)
    }

    pub async fn go_live(&self) -> Result<(), AppError> {
        let ch = lock(&self.active)
            .info
            .channel_id
            .ok_or_else(|| AppError::local("Nothing is playing."))?;
        self.stop_previous_catchup();
        self.play_channel(ch).await
    }

    /// Stream picker (Dispatcharr admin). The server swaps the upstream under the same proxy
    /// connection, so mpv keeps its URL; only the info-panel position changes.
    pub async fn switch_stream(&self, channel_id: i64, index: usize) -> Result<(), AppError> {
        let playing = {
            let a = lock(&self.active);
            a.info.channel_id == Some(channel_id) && a.info.mode == Some(PlayMode::Live)
        };
        if !playing {
            return Err(AppError::local(
                "Switch streams while the channel is playing live.",
            ));
        }
        let (i, c) = self.service.switch_stream(channel_id, index).await?;
        let mut a = lock(&self.active);
        a.source_index = u32::try_from(i).unwrap_or(0);
        a.source_count = u32::try_from(c).unwrap_or(0);
        Ok(())
    }

    pub async fn retry(&self) -> Result<(), AppError> {
        let last = lock(&self.active)
            .last
            .clone()
            .ok_or_else(|| AppError::local("Nothing to retry."))?;
        match last {
            LastPlay::Channel(id) => self.play_channel(id).await,
            LastPlay::Catchup(p) => self.play_catchup(&p, true).await,
            LastPlay::Vod { kind, id } => self.play_vod(&kind, id, true).await,
            LastPlay::Recording(id) => self.play_recording(id).await,
            LastPlay::Restart(id) => self.restart_programme(id).await,
        }
    }

    /// Navigate away / stop button: stop playback and any open session.
    pub async fn stop(&self) -> Result<(), AppError> {
        // Save the final position of VOD / recordings before the player forgets it.
        let (media, snap) = (lock(&self.active).media, self.player.snapshot());
        if let (Some(m), Some(pos)) = (media, snap.position_secs) {
            self.report_media(m, pos, snap.duration_secs).await;
        }
        {
            let mut a = lock(&self.active);
            a.generation += 1;
            a.media = None;
            a.hls = None;
        }
        let r = self.player.stop().map_err(player_err);
        self.service.stop_live_hls().await;
        let prev = lock(&self.active).catchup.take();
        if let Some(play) = prev
            && let Err(e) = self.service.stop_catchup(&play).await
        {
            tracing::warn!("could not stop the catch-up session: {}", e.message);
        }
        r
    }

    /// The channel the main player is on (live or catch-up), if any.
    pub fn channel_id(&self) -> Option<i64> {
        lock(&self.active).info.channel_id
    }

    /// Sleep / quit: pause and release the stream, delete every open session.
    pub async fn release_all(&self) {
        if let Some(mv) = self.multiview.get() {
            mv.clear().await;
        }
        let _ = self.stop().await;
        match self.service.stop_all_sessions().await {
            Ok(r) => tracing::info!(
                "sessions released: {} stopped, {} failed",
                r.stopped,
                r.failed
            ),
            Err(e) => tracing::warn!("session cleanup failed: {}", e.message),
        }
    }

    pub fn command(&self, c: PlayerCommand) -> Result<(), AppError> {
        self.player.command(c).map_err(player_err)
    }
}
