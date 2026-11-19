//! Session hygiene (CLAUDE.md rule 6): every catch-up, HLS and TS session is stopped on zap,
//! navigate-away, sleep and quit. Session ids are persisted in `open_session` the moment they are
//! created, so a crashed run's sessions are deleted at the next start.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::Result;
use crate::store::{OpenSession, Store};

pub const KIND_CATCHUP: &str = "catchup";
pub const KIND_HLS: &str = "hls";

/// How long one stop may take before it is left for the next start (keeps quit/sleep quick).
pub const STOP_TIMEOUT: Duration = Duration::from_secs(3);

/// Stops a session on its server. Must be idempotent: an already-gone session is `Ok`.
#[async_trait]
pub trait SessionStopper: Send + Sync {
    async fn stop(&self, source_id: i64, kind: &str, session_id: &str) -> Result<()>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub stopped: usize,
    /// Left in `open_session` for the next attempt.
    pub failed: usize,
}

/// Record a session as soon as it exists.
pub async fn track(store: &Store, source_id: i64, kind: &str, session_id: &str) -> Result<()> {
    store.add_open_session(source_id, kind, session_id).await
}

/// Stop one session (zap / navigate-away). The row is removed only once the server confirmed, so a
/// failure is retried at the next start.
pub async fn release(
    store: &Store,
    stopper: &dyn SessionStopper,
    source_id: i64,
    kind: &str,
    session_id: &str,
) -> Result<()> {
    match tokio::time::timeout(STOP_TIMEOUT, stopper.stop(source_id, kind, session_id)).await {
        Ok(Ok(())) => store.remove_open_session(source_id, kind, session_id).await,
        Ok(Err(e)) => Err(e),
        Err(_) => Err(crate::AppError::network("Stopping the session timed out.")),
    }
}

async fn stop_these(
    store: &Store,
    stopper: &dyn SessionStopper,
    rows: Vec<OpenSession>,
) -> Result<CleanupReport> {
    let mut report = CleanupReport::default();
    for s in rows {
        match release(store, stopper, s.source_id, &s.kind, &s.session_id).await {
            Ok(()) => report.stopped += 1,
            Err(e) => {
                tracing::warn!(kind = %s.kind, error = %e, "could not stop a session; will retry later");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// Startup: delete every persisted session from a previous (crashed) run.
pub async fn cleanup_on_startup(
    store: &Store,
    stopper: &dyn SessionStopper,
) -> Result<CleanupReport> {
    let rows = store.list_open_sessions().await?;
    stop_these(store, stopper, rows).await
}

/// Sleep / quit: stop everything currently open.
pub async fn stop_all(store: &Store, stopper: &dyn SessionStopper) -> Result<CleanupReport> {
    cleanup_on_startup(store, stopper).await
}

/// Zap / navigate-away: stop all open sessions of one kind (e.g. catch-up when leaving archive
/// playback).
pub async fn stop_kind(
    store: &Store,
    stopper: &dyn SessionStopper,
    kind: &str,
) -> Result<CleanupReport> {
    let rows = store
        .list_open_sessions()
        .await?
        .into_iter()
        .filter(|s| s.kind == kind)
        .collect();
    stop_these(store, stopper, rows).await
}

#[cfg(test)]
mod tests;
