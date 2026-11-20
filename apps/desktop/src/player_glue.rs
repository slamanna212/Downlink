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
use downlink_player::{
    PlayMode, PlayRequest, Player, PlayerCommand, PlayerEvent, PlayerSnapshot, Profile,
    SessionState,
};
use tauri::{AppHandle, Emitter};

use crate::dto::{SessionInfo, player_state};

#[derive(Debug, Clone)]
pub enum LastPlay {
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
}

pub struct Playback {
    service: Arc<Service>,
    player: Arc<Player>,
    active: Arc<Mutex<Active>>,
}

fn lock(a: &Mutex<Active>) -> MutexGuard<'_, Active> {
    match a.lock() {
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
#[derive(Clone)]
pub struct SinkContext {
    active: Arc<Mutex<Active>>,
    app: AppHandle,
}

impl SinkContext {
    pub fn new(app: AppHandle) -> Self {
        Self {
            active: Arc::new(Mutex::new(Active::default())),
            app,
        }
    }

    /// The event sink: `player://state` (already throttled by the player), `session://stats`.
    pub fn sink(&self) -> downlink_player::EventSink {
        let ctx = self.clone();
        Arc::new(move |e| match e {
            PlayerEvent::State(snap) => {
                let dto = {
                    let mut a = lock(&ctx.active);
                    if snap.state != a.last_phase {
                        a.last_phase = snap.state;
                        a.info.transitioned_at = now_ms();
                    }
                    player_state(&snap, &a.info)
                };
                let _ = ctx.app.emit("player://state", dto);
            }
            PlayerEvent::Stats(s) => {
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
                        "sourceIndex": s.source_index + 1,
                        "sourceCount": null,
                    }),
                );
            }
            PlayerEvent::FailoverNeeded { last_error, .. } => {
                // PLAN §4.5: the channel's alternate streams are a server-side concern for
                // Dispatcharr; the client surfaces the failure with the last error.
                tracing::warn!("playback failed after a reconnect: {last_error}");
                let mut a = lock(&ctx.active);
                a.info.failure = Some(AppError::new(
                    ErrorCode::Stream,
                    "Source unavailable. Try again.",
                ));
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

    pub fn transitions(&self) -> Vec<downlink_player::Transition> {
        self.player.history()
    }

    fn to_request(&self, plan: &PlayPlan, mode: PlayMode, failover: bool) -> PlayRequest {
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

    fn begin(
        &self,
        channel_id: i64,
        mode: PlayMode,
        programme: Option<ProgrammeDto>,
        last: LastPlay,
    ) {
        self.stop_previous_catchup();
        let mut a = lock(&self.active);
        a.generation += 1;
        a.info = SessionInfo {
            channel_id: Some(channel_id),
            mode: Some(mode),
            programme,
            resume_at: None,
            failure: None,
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
            channel_id,
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
        let programme = self
            .service
            .get_now_next(&[channel_id])
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
            .and_then(|nn| nn.now);
        lock(&self.active).info.programme = programme;
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
        Ok(())
    }

    async fn start_catchup(
        &self,
        play: CatchupPlay,
        programme: Option<ProgrammeDto>,
        last: LastPlay,
    ) -> Result<(), AppError> {
        self.begin(play.channel_id, PlayMode::Catchup, programme, last);
        let failover = self.failover_capable(play.source_id).await;
        let req = self.to_request(&play.plan, PlayMode::Catchup, failover);
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

    pub async fn play_catchup(&self, programme_id: &str) -> Result<(), AppError> {
        let play = self.service.play_catchup(programme_id).await?;
        let programme = self.programme_dto(play.channel_id, play.start_utc).await;
        self.start_catchup(play, programme, LastPlay::Catchup(programme_id.to_owned()))
            .await
    }

    pub async fn restart_programme(&self, channel_id: i64) -> Result<(), AppError> {
        let play = self.service.restart_programme(channel_id).await?;
        let programme = self.programme_dto(play.channel_id, play.start_utc).await;
        self.start_catchup(play, programme, LastPlay::Restart(channel_id))
            .await
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

    pub async fn retry(&self) -> Result<(), AppError> {
        let last = lock(&self.active)
            .last
            .clone()
            .ok_or_else(|| AppError::local("Nothing to retry."))?;
        match last {
            LastPlay::Channel(id) => self.play_channel(id).await,
            LastPlay::Catchup(p) => self.play_catchup(&p).await,
            LastPlay::Restart(id) => self.restart_programme(id).await,
        }
    }

    /// Navigate away / stop button: stop playback and any open session.
    pub async fn stop(&self) -> Result<(), AppError> {
        {
            let mut a = lock(&self.active);
            a.generation += 1;
        }
        let r = self.player.stop().map_err(player_err);
        let prev = lock(&self.active).catchup.take();
        if let Some(play) = prev
            && let Err(e) = self.service.stop_catchup(&play).await
        {
            tracing::warn!("could not stop the catch-up session: {}", e.message);
        }
        r
    }

    /// Sleep / quit: pause and release the stream, delete every open session.
    pub async fn release_all(&self) {
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
