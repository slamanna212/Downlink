//! Guide refresh policy (PLAN §7.3). Pure planning plus a cancellable runner. Windows are UTC
//! seconds; day-sized chunks are aligned to local midnight in the server's timezone so a chunk
//! is a calendar day as the user sees it (DST-correct: a day may be 23 or 25 hours).

use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{LocalResult, NaiveDate, TimeZone};
use chrono_tz::Tz;

use crate::error::Result;
use crate::source::{Source, TimeWindow};
use crate::store::Store;

use super::{GuideCounts, sync_guide, visible_window};

/// Default cap for catch-up history.
pub const DEFAULT_MAX_CATCHUP_DAYS: i64 = 7;
/// How far ahead the background fill goes.
pub const AHEAD_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// now - 2 h .. now + 12 h; every full sync, and hourly in between (`refresh_visible_guides`).
    Visible,
    /// Back to the longest `catchup_days`, newest day first, after the visible window.
    CatchupHistory,
    /// Forward to 7 days, day by day, idle/background every 6 h.
    Ahead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedWindow {
    pub kind: WindowKind,
    pub window: TimeWindow,
}

pub fn parse_tz(name: Option<&str>) -> Tz {
    name.and_then(|n| n.parse().ok()).unwrap_or(Tz::UTC)
}

/// UTC seconds of the start of the local calendar day containing `t`.
pub fn local_day_start(t: i64, tz: Tz) -> i64 {
    let Some(dt) = tz.timestamp_opt(t, 0).single() else {
        return t - t.rem_euclid(86_400);
    };
    let date = dt.date_naive();
    local_midnight(date, tz).unwrap_or_else(|| t - t.rem_euclid(86_400))
}

fn local_midnight(date: NaiveDate, tz: Tz) -> Option<i64> {
    let naive = date.and_hms_opt(0, 0, 0)?;
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(d) => Some(d.timestamp()),
        LocalResult::Ambiguous(first, _) => Some(first.timestamp()),
        // Midnight skipped by a DST jump: the day starts at the first valid instant after it.
        LocalResult::None => {
            let shifted = date.and_hms_opt(1, 0, 0)?;
            tz.from_local_datetime(&shifted)
                .earliest()
                .map(|d| d.timestamp())
        }
    }
}

fn next_day_start(day_start: i64, tz: Tz) -> i64 {
    let Some(dt) = tz.timestamp_opt(day_start, 0).single() else {
        return day_start + 86_400;
    };
    dt.date_naive()
        .succ_opt()
        .and_then(|d| local_midnight(d, tz))
        .unwrap_or(day_start + 86_400)
}

/// Full plan in priority order: visible, then catch-up history (newest day first), then ahead.
pub fn plan_windows(now: i64, max_catchup_days: i64, tz: Tz) -> Vec<PlannedWindow> {
    let vis = visible_window(now);
    let mut out = vec![PlannedWindow {
        kind: WindowKind::Visible,
        window: vis,
    }];

    let days = max_catchup_days.clamp(0, DEFAULT_MAX_CATCHUP_DAYS);
    if days > 0 {
        let floor = local_day_start(now - days * 86_400, tz);
        let mut end = vis.start_utc;
        while end > floor {
            let start = local_day_start(end - 1, tz).max(floor);
            out.push(PlannedWindow {
                kind: WindowKind::CatchupHistory,
                window: TimeWindow {
                    start_utc: start,
                    end_utc: end,
                },
            });
            end = start;
        }
    }

    let limit = now + AHEAD_DAYS * 86_400;
    let mut start = vis.end_utc;
    while start < limit {
        let end = next_day_start(local_day_start(start, tz), tz).min(limit);
        let end = if end <= start { start + 86_400 } else { end };
        out.push(PlannedWindow {
            kind: WindowKind::Ahead,
            window: TimeWindow {
                start_utc: start,
                end_utc: end,
            },
        });
        start = end;
    }
    out
}

/// The longest `catchup_days` across stored channels of a source (0 when none).
pub async fn max_catchup_days(store: &Store, source_id: i64) -> Result<i64> {
    let d: Option<i64> =
        sqlx::query_scalar("SELECT max(catchup_days) FROM channel WHERE source_id = ?")
            .bind(source_id)
            .fetch_one(store.pool())
            .await?;
    Ok(d.unwrap_or(0))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub windows_done: usize,
    pub windows_total: usize,
    pub programmes: usize,
    pub cancelled: bool,
}

/// Fill the given kinds in plan order. Cancellable between windows (the cache is consistent after
/// every window); a failing window stops the run and returns the error.
pub async fn run_plan(
    src: &dyn Source,
    store: &Store,
    source_id: i64,
    plan: &[PlannedWindow],
    kinds: &[WindowKind],
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize) + Send,
) -> Result<RefreshReport> {
    let todo: Vec<&PlannedWindow> = plan.iter().filter(|w| kinds.contains(&w.kind)).collect();
    let mut report = RefreshReport {
        windows_total: todo.len(),
        ..Default::default()
    };
    for w in todo {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let GuideCounts { programmes, .. } = sync_guide(src, store, source_id, w.window).await?;
        report.windows_done += 1;
        report.programmes += programmes;
        progress(report.windows_done, report.windows_total);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(tz: Tz, y: i32, m: u32, d: u32, h: u32) -> i64 {
        tz.with_ymd_and_hms(y, m, d, h, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn plan_order_and_coverage() {
        let now = 1_735_725_600 + 1800;
        let p = plan_windows(now, 3, Tz::UTC);
        assert_eq!(p[0].kind, WindowKind::Visible);
        assert_eq!(p[0].window, visible_window(now));
        let hist: Vec<_> = p
            .iter()
            .filter(|w| w.kind == WindowKind::CatchupHistory)
            .collect();
        let ahead: Vec<_> = p.iter().filter(|w| w.kind == WindowKind::Ahead).collect();
        // History is contiguous, newest first, ends at the visible start and reaches 3 days back.
        assert_eq!(hist[0].window.end_utc, p[0].window.start_utc);
        for pair in hist.windows(2) {
            assert_eq!(pair[1].window.end_utc, pair[0].window.start_utc);
        }
        assert!(hist.last().unwrap().window.start_utc <= now - 3 * 86_400);
        // Ahead is contiguous from the visible end to +7 d.
        assert_eq!(ahead[0].window.start_utc, p[0].window.end_utc);
        for pair in ahead.windows(2) {
            assert_eq!(pair[1].window.start_utc, pair[0].window.end_utc);
        }
        assert_eq!(ahead.last().unwrap().window.end_utc, now + 7 * 86_400);
        // Priority order: visible, history, ahead.
        let kinds: Vec<_> = p.iter().map(|w| w.kind).collect();
        let first_ahead = kinds.iter().position(|k| *k == WindowKind::Ahead).unwrap();
        assert!(kinds[..first_ahead].iter().all(|k| *k != WindowKind::Ahead));
    }

    #[test]
    fn no_history_when_no_catchup_and_capped_at_seven() {
        let now = 2_000_000_000;
        assert!(
            plan_windows(now, 0, Tz::UTC)
                .iter()
                .all(|w| w.kind != WindowKind::CatchupHistory)
        );
        let long = plan_windows(now, 30, Tz::UTC);
        let oldest = long
            .iter()
            .filter(|w| w.kind == WindowKind::CatchupHistory)
            .map(|w| w.window.start_utc)
            .min()
            .unwrap();
        assert!(oldest >= now - 8 * 86_400);
    }

    #[test]
    fn day_chunks_follow_local_midnight_with_dst() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        // 2025-03-30 is 23 h long in Berlin (spring forward); 2025-10-26 is 25 h (fall back).
        let t = ts(tz, 2025, 3, 30, 12);
        let start = local_day_start(t, tz);
        assert_eq!(start, ts(tz, 2025, 3, 30, 0));
        assert_eq!(next_day_start(start, tz) - start, 23 * 3600);
        let t = ts(tz, 2025, 10, 26, 12);
        let start = local_day_start(t, tz);
        assert_eq!(next_day_start(start, tz) - start, 25 * 3600);
        // Different zones disagree on where "today" starts for the same instant.
        let ny: Tz = "America/New_York".parse().unwrap();
        let t = ts(Tz::UTC, 2025, 6, 1, 3);
        assert_ne!(local_day_start(t, tz), local_day_start(t, ny));
        assert_eq!(parse_tz(Some("Nope/Zone")), Tz::UTC);
        assert_eq!(parse_tz(None), Tz::UTC);
    }

    #[test]
    fn history_chunks_are_local_days_in_berlin() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        let now = ts(tz, 2025, 3, 31, 9);
        let p = plan_windows(now, 3, tz);
        let hist: Vec<_> = p
            .iter()
            .filter(|w| w.kind == WindowKind::CatchupHistory)
            .collect();
        // Interior chunk boundaries sit on local midnights.
        for w in &hist[..hist.len() - 1] {
            assert_eq!(local_day_start(w.window.start_utc, tz), w.window.start_utc);
        }
    }
}
