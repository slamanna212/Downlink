//! Channel catalogue: the **only** place adult / radio / hidden / DRM filtering happens
//! (CLAUDE.md rule 10). Sync drops hidden and over-level channels; listing and search apply the
//! visibility predicate below, so components never filter ad hoc.

mod play;

use sqlx::FromRow;

use crate::error::Result;
use crate::source::{SourceChannel, UserLevel};
use crate::store::{ChannelRow, ProgrammeRow, Store};

pub use play::resolve_play_plan;

/// Numeric Dispatcharr level for a user level.
fn level_value(l: UserLevel) -> i64 {
    match l {
        UserLevel::Streamer => 0,
        UserLevel::Standard => 1,
        UserLevel::Admin => 10,
    }
}

/// Why a channel is dropped at sync time (never stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    Hidden,
    AboveUserLevel,
}

/// Sync-time filter. `hidden_from_output` channels are never shown; channels above the account's
/// level cannot be played, so they are not shown either (hidden, not shown-and-erroring).
pub fn sync_filter(c: &SourceChannel, account_level: Option<UserLevel>) -> Option<DropReason> {
    if c.hidden {
        return Some(DropReason::Hidden);
    }
    if let Some(l) = account_level
        && c.user_level > level_value(l)
    {
        return Some(DropReason::AboveUserLevel);
    }
    None
}

/// What the user has chosen to see.
#[derive(Debug, Clone, Default)]
pub struct ChannelFilter {
    pub source_id: Option<i64>,
    pub group_id: Option<i64>,
    pub search: Option<String>,
    pub include_adult: bool,
    pub include_radio: bool,
    pub include_drm: bool,
    /// Only live favourites, in the user's order.
    pub favourites_only: bool,
}

/// SQL fragment (table alias `c`) implementing the visibility rules.
fn visibility_sql(f: &ChannelFilter) -> String {
    let mut s = String::from("1 = 1");
    if !f.include_adult {
        s.push_str(" AND c.is_adult = 0");
    }
    if !f.include_radio {
        s.push_str(" AND c.is_radio = 0");
    }
    if !f.include_drm {
        s.push_str(" AND c.is_drm = 0");
    }
    s
}

/// FTS candidates ranked per search. A narrow query has fewer matches than this, so its ranking
/// is exact; a broad prefix is ranked within its first candidates (by channel order).
const CANDIDATES: u32 = 2000;

const COLS: &str = "c.id, c.source_id, c.ext_id, c.uuid, c.number, c.name, c.group_id, c.tvg_id, \
    c.tvg_name, c.logo_key, c.quality, c.is_adult, c.is_radio, c.is_drm, c.catchup_days, c.catchup_template, \
    c.stream_redirect, c.play_url, c.play_headers, c.epg_match, c.sort_key";

/// FTS5 prefix query from free text: every token quoted and starred.
fn fts_query(q: &str) -> Option<String> {
    let toks: Vec<String> = q
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '.'))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect();
    if toks.is_empty() {
        None
    } else {
        Some(toks.join(" "))
    }
}

/// List visible channels, by number then name; or search ranked by number proximity, then bm25.
pub async fn list_channels(
    store: &Store,
    f: &ChannelFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<ChannelRow>> {
    let vis = visibility_sql(f);
    let search = f.search.as_deref().and_then(fts_query);
    let numeric: Option<f64> = f.search.as_deref().and_then(|s| s.trim().parse().ok());
    let sql = if search.is_some() {
        // Two stages so a broad prefix ("ne") never sorts every match: FTS yields up to CANDIDATES
        // matches (bm25 `rank` is evaluated only for those, never for every match of a broad prefix); exact number matches are added via the number index; the
        // outer query then orders by number proximity, then rank.
        format!(
            "SELECT {COLS} FROM (
                 SELECT rowid AS id, min(rank) AS rank FROM (
                   SELECT rowid, rank FROM (
                     SELECT rowid, rank FROM channel_fts WHERE channel_fts MATCH ?1
                       LIMIT {CANDIDATES})
                   UNION ALL
                   SELECT id, -1e9 FROM channel WHERE ?4 IS NOT NULL AND number = ?4
                 ) GROUP BY rowid
               ) m JOIN channel c ON c.id = m.id
             WHERE {vis}
               AND (?2 IS NULL OR c.source_id = ?2) AND (?3 IS NULL OR c.group_id = ?3)
               AND (?7 = 0 OR c.id IN (SELECT channel_id FROM favourite WHERE deleted = 0))
             ORDER BY CASE WHEN ?4 IS NOT NULL AND c.number = ?4 THEN 0 ELSE 1 END,
                      CASE WHEN ?4 IS NULL THEN 0 ELSE abs(coalesce(c.number, 1e9) - ?4) END,
                      m.rank, c.number, c.sort_key
             LIMIT ?5 OFFSET ?6"
        )
    } else {
        format!(
            "SELECT {COLS} FROM channel c
             WHERE {vis} AND (?2 IS NULL OR c.source_id = ?2) AND (?3 IS NULL OR c.group_id = ?3)
               AND (?1 IS NULL OR 1 = 1) AND (?4 IS NULL OR 1 = 1)
               AND (?7 = 0 OR c.id IN (SELECT channel_id FROM favourite WHERE deleted = 0))
             ORDER BY CASE WHEN ?7 = 0 THEN 0
                           ELSE (SELECT position FROM favourite f WHERE f.channel_id = c.id) END,
                      c.number IS NULL, c.number, c.sort_key
             LIMIT ?5 OFFSET ?6"
        )
    };
    Ok(sqlx::query_as(&sql)
        .bind(search)
        .bind(f.source_id)
        .bind(f.group_id)
        .bind(numeric)
        .bind(limit)
        .bind(offset)
        .bind(f.favourites_only)
        .fetch_all(store.pool())
        .await?)
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct GroupRow {
    pub id: i64,
    pub source_id: i64,
    pub name: String,
    /// Visible channels in the group under the same filter.
    pub channel_count: i64,
}

/// Groups with at least one visible channel.
pub async fn list_groups(store: &Store, f: &ChannelFilter) -> Result<Vec<GroupRow>> {
    let vis = visibility_sql(f);
    let sql = format!(
        "SELECT g.id, g.source_id, g.name, count(c.id) AS channel_count
         FROM channel_group g JOIN channel c ON c.group_id = g.id AND {vis}
         WHERE (?1 IS NULL OR g.source_id = ?1)
         GROUP BY g.id ORDER BY g.name COLLATE NOCASE"
    );
    Ok(sqlx::query_as(&sql)
        .bind(f.source_id)
        .fetch_all(store.pool())
        .await?)
}

/// Visible channels for the given row ids, in the order given (unknown/hidden ids are skipped).
pub async fn channels_by_ids(
    store: &Store,
    f: &ChannelFilter,
    ids: &[i64],
) -> Result<Vec<ChannelRow>> {
    let vis = visibility_sql(f);
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let row: Option<ChannelRow> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM channel c WHERE c.id = ?1 AND {vis}"
        ))
        .bind(id)
        .fetch_optional(store.pool())
        .await?;
        out.extend(row);
    }
    Ok(out)
}

/// Local guide search (FTS5 over title/subtitle/description), restricted to visible channels and
/// to programmes that have not ended before `from_utc`. Ranked by bm25, then start time.
pub async fn search_programmes(
    store: &Store,
    f: &ChannelFilter,
    query: &str,
    from_utc: i64,
    limit: i64,
) -> Result<Vec<ProgrammeRow>> {
    let Some(q) = fts_query(query) else {
        return Ok(Vec::new());
    };
    let vis = visibility_sql(f);
    let sql = format!(
        "SELECT p.channel_id, p.start_utc, p.end_utc, p.ext_id, p.title, p.subtitle, p.description,
                p.season, p.episode, p.flags
         FROM programme_fts JOIN programme p ON p.rowid = programme_fts.rowid
         JOIN channel c ON c.id = p.channel_id
         WHERE programme_fts MATCH ?1 AND {vis} AND p.end_utc >= ?2
           AND (?3 IS NULL OR c.source_id = ?3)
         ORDER BY bm25(programme_fts), p.start_utc LIMIT ?4"
    );
    Ok(sqlx::query_as(&sql)
        .bind(q)
        .bind(from_utc)
        .bind(f.source_id)
        .bind(limit)
        .fetch_all(store.pool())
        .await?)
}

/// Number entry (PLAN §3.8.1): an exact match wins, else the lowest channel >= the number.
pub async fn channel_by_number(
    store: &Store,
    f: &ChannelFilter,
    number: f64,
) -> Result<Option<ChannelRow>> {
    let vis = visibility_sql(f);
    let sql = format!(
        "SELECT {COLS} FROM channel c WHERE {vis} AND (?1 IS NULL OR c.source_id = ?1)
           AND c.number >= ?2 ORDER BY c.number, c.sort_key LIMIT 1"
    );
    Ok(sqlx::query_as(&sql)
        .bind(f.source_id)
        .bind(number)
        .fetch_optional(store.pool())
        .await?)
}

/// Total visible channel count for a filter (ignores `search`).
pub async fn count_channels(store: &Store, f: &ChannelFilter) -> Result<i64> {
    let vis = visibility_sql(f);
    Ok(sqlx::query_scalar(&format!(
        "SELECT count(*) FROM channel c WHERE {vis} AND (?1 IS NULL OR c.source_id = ?1)"
    ))
    .bind(f.source_id)
    .fetch_one(store.pool())
    .await?)
}

#[cfg(test)]
mod tests;
