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

/// How many of a source's channels are shown (those not hidden by the server).
pub fn visible_count(channels: &[SourceChannel]) -> i64 {
    channels
        .iter()
        .filter(|c| sync_filter(c, None).is_none())
        .count() as i64
}

/// Heuristic classification for sources that do not flag adult/radio themselves (M3U, Xtream
/// categories). Classification lives here with the filtering; sync calls it. Only sets flags,
/// never clears ones the source provided.
pub fn classify(c: &mut SourceChannel, group_name: Option<&str>) {
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '+'))
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let g = group_name.map(words).unwrap_or_default();
    let n = words(&c.name);
    if g.iter()
        .any(|w| matches!(w.as_str(), "adult" | "adults" | "xxx" | "18+" | "porn"))
        || n.iter().any(|w| w == "xxx")
    {
        c.is_adult = true;
    }
    if g.iter().any(|w| matches!(w.as_str(), "radio" | "radios")) {
        c.is_radio = true;
    }
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
    if search.is_none() {
        let mut sql = sqlx::QueryBuilder::new(format!("SELECT {COLS} FROM channel c"));
        if f.favourites_only {
            sql.push(" JOIN favourite f ON f.channel_id = c.id AND f.deleted = 0");
        }
        sql.push(format!(" WHERE {vis}"));
        if let Some(id) = f.source_id {
            sql.push(" AND c.source_id = ").push_bind(id);
        }
        if let Some(id) = f.group_id {
            sql.push(" AND c.group_id = ").push_bind(id);
        }
        sql.push(" ORDER BY ");
        if f.favourites_only {
            sql.push("f.position, ");
        }
        sql.push("c.number IS NULL, c.number, c.sort_key, c.id LIMIT ")
            .push_bind(limit)
            .push(" OFFSET ")
            .push_bind(offset);
        return Ok(sql.build_query_as().fetch_all(store.pool()).await?);
    }
    let eligible = format!(
        "{vis} AND (?2 IS NULL OR c.source_id = ?2)
        AND (?3 IS NULL OR c.group_id = ?3)
        AND (?7 = 0 OR c.id IN (SELECT channel_id FROM favourite WHERE deleted = 0))"
    );
    let sql = {
        format!(
            "SELECT {COLS} FROM (
                 SELECT rowid AS id, min(rank) AS rank FROM (
                   SELECT rowid, rank FROM (
                     SELECT channel_fts.rowid AS rowid, rank FROM channel_fts
                       CROSS JOIN channel c ON c.id = channel_fts.rowid
                       WHERE channel_fts MATCH ?1 AND {eligible}
                       LIMIT max({CANDIDATES}, ?5 + ?6))
                   UNION ALL
                   SELECT id, -1e9 FROM channel WHERE ?4 IS NOT NULL AND number = ?4
                 ) GROUP BY rowid
               ) m JOIN channel c ON c.id = m.id
             WHERE {eligible}
             ORDER BY CASE WHEN ?4 IS NOT NULL AND c.number = ?4 THEN 0 ELSE 1 END,
                      CASE WHEN ?4 IS NULL THEN 0 ELSE abs(coalesce(c.number, 1e9) - ?4) END,
                      m.rank, c.number, c.sort_key, c.id
             LIMIT ?5 OFFSET ?6"
        )
    };
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
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
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
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
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    // One query for the whole set (was one per id).
    let ids_json = serde_json::to_string(ids)?;
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLS} FROM channel c WHERE c.id IN (SELECT value FROM json_each(?1)) AND {vis}"
    )))
    .bind(ids_json)
    .fetch_all(store.pool())
    .await?)
}

/// Rank a bounded set of eligible FTS hits by start time. Scan past expired or
/// hidden hits so broad terms can still fill the requested result window.
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
    // FTS5 can expand a common prefix even when another word has no matches.
    if let Some((first, _)) = q.split_once(' ') {
        let exists: bool = sqlx::query_scalar(
            "SELECT CASE WHEN EXISTS(SELECT 1 FROM programme_fts WHERE programme_fts MATCH ?1)
             THEN 1 ELSE EXISTS(SELECT 1 FROM programme_fts WHERE programme_fts MATCH ?2) END",
        )
        .bind(first.trim_end_matches('*'))
        .bind(first)
        .fetch_one(store.pool())
        .await?;
        if !exists {
            return Ok(Vec::new());
        }
    }
    let vis = visibility_sql(f);
    let sql = format!(
        "SELECT p.channel_id, p.start_utc, p.end_utc, p.ext_id, p.title, p.subtitle, p.description,
                p.season, p.episode, p.flags
         FROM (SELECT rowid FROM (
             SELECT candidate.rowid, candidate.start_utc, candidate.channel_id FROM programme_fts
             CROSS JOIN programme candidate ON candidate.rowid = programme_fts.rowid
             CROSS JOIN channel c ON c.id = candidate.channel_id
             WHERE programme_fts MATCH ?1 AND {vis}
               AND candidate.end_utc >= ?2 AND candidate.start_utc <= ?6
               AND (?3 IS NULL OR c.source_id = ?3)
               AND (?7 IS NULL OR c.group_id = ?7)
               AND (?8 = 0 OR c.id IN (SELECT channel_id FROM favourite WHERE deleted = 0))
             LIMIT ?5
         ) ORDER BY start_utc, channel_id LIMIT ?4) f JOIN programme p ON p.rowid = f.rowid
         ORDER BY p.start_utc, p.channel_id LIMIT ?4"
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(q)
        .bind(from_utc)
        .bind(f.source_id)
        .bind(limit)
        .bind(limit.max(5000))
        .bind(from_utc.saturating_add(7 * 86_400))
        .bind(f.group_id)
        .bind(f.favourites_only)
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
    let mut query = sqlx::QueryBuilder::new(format!(
        "SELECT {COLS} FROM channel c WHERE {vis} AND (c.number IS NULL) = 0 AND c.number >= "
    ));
    query.push_bind(number);
    if let Some(id) = f.source_id {
        query.push(" AND c.source_id = ").push_bind(id);
    }
    query.push(" ORDER BY c.number, c.sort_key, c.id LIMIT 1");
    Ok(query.build_query_as().fetch_optional(store.pool()).await?)
}

/// Total visible channel count for a filter (ignores `search`; honours `favourites_only`).
pub async fn count_channels(store: &Store, f: &ChannelFilter) -> Result<i64> {
    let vis = visibility_sql(f);
    Ok(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM channel c WHERE {vis} AND (?1 IS NULL OR c.source_id = ?1)
           AND (?2 = 0 OR c.id IN (SELECT channel_id FROM favourite WHERE deleted = 0))"
    )))
    .bind(f.source_id)
    .bind(f.favourites_only)
    .fetch_one(store.pool())
    .await?)
}

#[cfg(test)]
mod tests;
