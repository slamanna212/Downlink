//! Catch-up flow (PLAN §5.2): create session -> play plan (relative `playback_url` joined to the
//! base) -> throttled position reports -> delete on stop. Every time comes from the **programme
//! row** in SQLite (CLAUDE.md rule 12): one clock for guide and archive; nothing is inferred.

pub mod template;

use std::time::Duration;

use crate::epg;
use crate::error::{AppError, Result};
use crate::sessions::{self, KIND_CATCHUP, SessionStopper};
use crate::source::{ChannelKey, PlayPlan, Programme, SessionId, SourceClient};
use crate::store::{ProgrammeRow, Store};

/// Position reports: every 10 s, and immediately on pause/seek (PLAN §5.2).
pub const REPORT_INTERVAL: Duration = Duration::from_secs(10);

/// A started catch-up playback.
#[derive(Debug, Clone)]
pub struct CatchupPlay {
    pub plan: PlayPlan,
    pub session: SessionId,
    pub channel_id: i64,
    pub source_id: i64,
    /// Programme row times (UTC seconds), for the UI's timeline.
    pub start_utc: i64,
    pub end_utc: i64,
}

fn to_programme(row: &ProgrammeRow, tvg: &str) -> Programme {
    Programme {
        channel_ref: tvg.to_owned(),
        ext_id: row.ext_id.clone(),
        start_utc: row.start_utc,
        end_utc: row.end_utc,
        title: row.title.clone(),
        subtitle: row.subtitle.clone(),
        description: row.description.clone(),
        season: row.season,
        episode: row.episode,
        flags: row.flags,
    }
}

/// Key under which catch-up watch positions are stored in `resume`.
pub fn resume_key(channel_ext_id: &str, start_utc: i64) -> String {
    format!("{channel_ext_id}:{start_utc}")
}

async fn programme_row(store: &Store, channel_id: i64, start_utc: i64) -> Result<ProgrammeRow> {
    sqlx::query_as(
        "SELECT channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags
         FROM programme WHERE channel_id = ? AND start_utc = ?",
    )
    .bind(channel_id)
    .bind(start_utc)
    .fetch_optional(store.pool())
    .await?
    .ok_or_else(|| AppError::local("That programme is no longer in the guide."))
}

/// Play a past programme from the archive. `now` is injected for testability.
///
/// `is_catchup`/`catchup_days` only say a channel *may* have an archive; the server's session
/// endpoint is the authority, so the window is not second-guessed here. A programme that has not
/// started cannot be in the archive and is refused without a request.
pub async fn play_catchup(
    store: &Store,
    client: &dyn SourceClient,
    channel_id: i64,
    programme_start_utc: i64,
    now: i64,
) -> Result<CatchupPlay> {
    let ch = store
        .channel(channel_id)
        .await?
        .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
    if ch.catchup_days <= 0 {
        return Err(AppError::stream("This channel does not keep an archive."));
    }
    let row = programme_row(store, channel_id, programme_start_utc).await?;
    if row.start_utc > now {
        return Err(AppError::stream("This programme has not started yet."));
    }
    let key = ChannelKey::from_row(&ch);
    let plan = client
        .catchup(key, &to_programme(&row, ch.tvg_id.as_deref().unwrap_or("")))
        .await?;
    let session = plan
        .session
        .clone()
        .ok_or_else(|| AppError::server("The server did not start an archive session."))?;
    // Persist immediately so a crash cannot orphan the session (rule 6).
    sessions::track(store, ch.source_id, KIND_CATCHUP, &session.0).await?;
    Ok(CatchupPlay {
        plan,
        session,
        channel_id,
        source_id: ch.source_id,
        start_utc: row.start_utc,
        end_utc: row.end_utc,
    })
}

/// "Restart" the programme airing now on a channel: archive playback from its start (taken from
/// the programme row).
pub async fn restart_programme(
    store: &Store,
    client: &dyn SourceClient,
    channel_id: i64,
    now: i64,
) -> Result<CatchupPlay> {
    let nn = epg::now_next(store, channel_id, now).await?;
    let cur = nn
        .now
        .ok_or_else(|| AppError::stream("Nothing is airing on this channel to restart."))?;
    play_catchup(store, client, channel_id, cur.start_utc, now).await
}

/// Stop one catch-up playback (zap, navigate-away, go live).
pub async fn stop(store: &Store, stopper: &dyn SessionStopper, play: &CatchupPlay) -> Result<()> {
    sessions::release(
        store,
        stopper,
        play.source_id,
        KIND_CATCHUP,
        &play.session.0,
    )
    .await
}

/// Throttled position reporter.
#[derive(Debug)]
pub struct PositionReporter {
    session: SessionId,
    last_report_ms: Option<u64>,
}

impl PositionReporter {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            last_report_ms: None,
        }
    }

    /// True when a report is due: first report, `force` (pause/seek), or 10 s since the last.
    pub fn due(&self, now_ms: u64, force: bool) -> bool {
        force
            || self
                .last_report_ms
                .is_none_or(|l| now_ms.saturating_sub(l) >= REPORT_INTERVAL.as_millis() as u64)
    }

    /// Report if due. Returns whether a request was made. Errors are non-fatal for playback;
    /// the caller logs them.
    pub async fn report(
        &mut self,
        client: &dyn SourceClient,
        now_ms: u64,
        position_secs: u64,
        force: bool,
    ) -> Result<bool> {
        if !self.due(now_ms, force) {
            return Ok(false);
        }
        self.last_report_ms = Some(now_ms);
        client
            .catchup_position(&self.session, position_secs)
            .await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttle_logic() {
        let mut r = PositionReporter::new(SessionId("s".into()));
        assert!(r.due(0, false), "first report is due");
        r.last_report_ms = Some(1_000);
        assert!(!r.due(5_000, false));
        assert!(r.due(5_000, true), "pause/seek forces");
        assert!(r.due(11_000, false));
    }

    #[test]
    fn resume_keys() {
        assert_eq!(resume_key("7", 1700000000), "7:1700000000");
    }
}
