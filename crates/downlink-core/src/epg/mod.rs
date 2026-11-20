//! Guide: windowing, now/next and sync. All times are UTC seconds.

pub mod refresh;

use std::collections::HashMap;

use crate::error::Result;
use crate::source::{Programme, Source, TimeWindow};
use crate::store::{ProgrammeRow, Store};

/// Visible window: now - 2 h .. now + 12 h (PLAN §7.3).
pub fn visible_window(now: i64) -> TimeWindow {
    TimeWindow {
        start_utc: now - 2 * 3600,
        end_utc: now + 12 * 3600,
    }
}

/// Programmes kept from `now - max_catchup_days` (default cap 7) to `now + 7 d` (PLAN §7.4).
pub fn retention_cutoff(now: i64, max_catchup_days: i64) -> i64 {
    now - max_catchup_days.clamp(0, 7) * 86_400
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NowNext {
    pub now: Option<ProgrammeRow>,
    pub next: Option<ProgrammeRow>,
}

const PCOLS: &str =
    "channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags";

/// Now/next for one channel: `start <= now < end`, and the first programme starting at/after that
/// one ends (or after `now` when nothing is airing).
pub async fn now_next(store: &Store, channel_id: i64, now: i64) -> Result<NowNext> {
    let mut m = now_next_many(store, &[channel_id], now).await?;
    Ok(m.remove(&channel_id).unwrap_or_default())
}

pub async fn now_next_many(
    store: &Store,
    channel_ids: &[i64],
    now: i64,
) -> Result<HashMap<i64, NowNext>> {
    let mut out: HashMap<i64, NowNext> = HashMap::new();
    for &id in channel_ids {
        let cur: Option<ProgrammeRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PCOLS} FROM programme WHERE channel_id = ? AND start_utc <= ? AND end_utc > ?
             ORDER BY start_utc DESC LIMIT 1"
        )))
        .bind(id)
        .bind(now)
        .bind(now)
        .fetch_optional(store.pool())
        .await?;
        let after = cur.as_ref().map_or(now, |c| c.end_utc);
        let next: Option<ProgrammeRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PCOLS} FROM programme WHERE channel_id = ? AND start_utc >= ?
             ORDER BY start_utc LIMIT 1"
        )))
        .bind(id)
        .bind(after)
        .fetch_optional(store.pool())
        .await?;
        out.insert(id, NowNext { now: cur, next });
    }
    Ok(out)
}

/// Programmes overlapping `[start, end)` for the given channels: one range query on the
/// `(channel_id, start_utc)` key for the whole set, returned in the order the channels were given,
/// then by start time.
pub async fn window(
    store: &Store,
    channel_ids: &[i64],
    w: TimeWindow,
) -> Result<Vec<ProgrammeRow>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids_json = serde_json::to_string(channel_ids)?;
    let mut rows: Vec<ProgrammeRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PCOLS} FROM programme
         WHERE channel_id IN (SELECT value FROM json_each(?1)) AND start_utc < ?2 AND end_utc > ?3
         ORDER BY channel_id, start_utc"
    )))
    .bind(ids_json)
    .bind(w.end_utc)
    .bind(w.start_utc)
    .fetch_all(store.pool())
    .await?;
    // Caller order (e.g. channel number), not id order. Stable, so start order is kept per channel.
    let mut pos: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for (i, id) in channel_ids.iter().enumerate() {
        pos.entry(*id).or_insert(i);
    }
    rows.sort_by_key(|r| pos.get(&r.channel_id).copied().unwrap_or(usize::MAX));
    Ok(rows)
}

fn to_row(channel_id: i64, p: &Programme) -> ProgrammeRow {
    ProgrammeRow {
        channel_id,
        start_utc: p.start_utc,
        end_utc: p.end_utc,
        ext_id: p.ext_id.clone(),
        title: p.title.clone(),
        subtitle: p.subtitle.clone(),
        description: p.description.clone(),
        season: p.season,
        episode: p.episode,
        flags: p.flags,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuideCounts {
    pub programmes: usize,
    /// Rows whose `tvg_id` matches no channel (dropped).
    pub unmatched: usize,
}

/// Fetch a window and replace it in SQLite. The fetch happens first, so a failed request leaves
/// the cache untouched. Rows are joined to channels on `tvg_id` (several channels may share one),
/// falling back to the channel uuid.
pub async fn sync_guide(
    src: &dyn Source,
    store: &Store,
    source_id: i64,
    w: TimeWindow,
) -> Result<GuideCounts> {
    let fetched = match src.guide(w).await {
        Ok(f) => f,
        Err(e) => {
            store.set_sync_error(source_id, "guide", &e.message).await?;
            return Err(e);
        }
    };
    let by_tvg = store.channels_by_tvg(source_id).await?;
    // Dispatcharr's dummy-EPG rows carry the channel uuid in place of a tvg_id.
    let by_uuid = store.channels_by_uuid(source_id).await?;
    let mut rows = Vec::with_capacity(fetched.len());
    let mut unmatched = 0;
    for p in &fetched {
        if let Some(ids) = by_tvg.get(&p.channel_ref) {
            rows.extend(ids.iter().map(|id| to_row(*id, p)));
        } else if let Some(id) = by_uuid.get(&p.channel_ref) {
            rows.push(to_row(*id, p));
        } else {
            unmatched += 1;
        }
    }
    if unmatched > 0 {
        tracing::debug!(unmatched, "guide rows matched no channel");
    }
    store
        .delete_programmes_overlapping(source_id, w.start_utc, w.end_utc)
        .await?;
    let n = store.upsert_programmes(&rows).await?;
    let now = crate::sync::now_utc();
    store.set_sync_ok(source_id, "guide", now).await?;
    Ok(GuideCounts {
        programmes: n,
        unmatched,
    })
}

/// Quick now-playing fill via the source's current-programmes call (no window delete).
pub async fn sync_current(src: &dyn Source, store: &Store, source_id: i64) -> Result<usize> {
    let by_uuid = store.channels_by_uuid(source_id).await?;
    let uuids: Vec<String> = by_uuid.keys().cloned().collect();
    let cur = src.current_programmes(&uuids).await?;
    let rows: Vec<ProgrammeRow> = cur
        .iter()
        .filter_map(|(u, p)| by_uuid.get(u).map(|id| to_row(*id, p)))
        .collect();
    store.upsert_programmes(&rows).await
}

/// Prune programmes older than the retention cutoff.
pub async fn prune(store: &Store, now: i64) -> Result<u64> {
    let max_days: Option<i64> = sqlx::query_scalar("SELECT max(catchup_days) FROM channel")
        .fetch_one(store.pool())
        .await?;
    store
        .prune_programmes(retention_cutoff(now, max_days.unwrap_or(0)))
        .await
}

#[cfg(test)]
mod tests;
