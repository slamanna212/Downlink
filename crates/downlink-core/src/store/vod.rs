//! VOD cache, recordings and DVR rules. Local row ids are what the UI sees.

use std::collections::HashSet;

use sqlx::FromRow;

use super::Store;
use crate::error::Result;
use crate::source::{RecStatus, RecordingInfo, RuleInfo, VodItem};

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct VodRow {
    pub id: i64,
    pub source_id: i64,
    pub kind: String,
    pub ext_id: String,
    pub parent_ext: Option<String>,
    pub name: String,
    pub year: Option<i64>,
    pub rating: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub duration_secs: Option<i64>,
    /// Poster image URL (templated for credentials; see `vault`).
    pub art_key: Option<String>,
    pub backdrop_key: Option<String>,
    pub uuid: Option<String>,
    pub container: Option<String>,
    pub category_ext: Option<String>,
}

const VOD_COLS: &str = "id, source_id, kind, ext_id, parent_ext, name, year, rating, genre, description, \
    season, episode, duration_secs, art_key, backdrop_key, uuid, container, category_ext";

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct RecordingRow {
    pub id: i64,
    pub source_id: i64,
    pub ext_id: String,
    pub channel_id: Option<i64>,
    pub start_utc: i64,
    pub end_utc: i64,
    pub state: String,
    pub title: Option<String>,
    pub rule_label: Option<String>,
    pub size_bytes: Option<i64>,
    pub error: Option<String>,
    pub file_url: Option<String>,
}

const REC_COLS: &str = "id, source_id, ext_id, channel_id, start_utc, end_utc, state, title, rule_label, size_bytes, error, file_url";

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RuleRow {
    pub id: i64,
    pub source_id: i64,
    pub kind: String,
    pub ext_id: String,
    pub title: String,
    pub channel_name: String,
    pub schedule: String,
}

impl Store {
    // ---- VOD --------------------------------------------------------------------------------

    /// Upsert fetched items (keyed by `(source, kind, ext_id)`) and return them as rows, in order.
    pub async fn upsert_vod_items(
        &self,
        source_id: i64,
        items: &[VodItem],
        now: i64,
    ) -> Result<Vec<VodRow>> {
        let mut tx = self.pool().begin().await?;
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let Some(kind) = it.kind else { continue };
            let row: VodRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "INSERT INTO vod_item (source_id, kind, ext_id, parent_ext, name, year, rating, genre, description,
                   season, episode, duration_secs, art_key, backdrop_key, uuid, container, category_ext, updated_at)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(source_id, kind, ext_id) DO UPDATE SET
                   parent_ext = coalesce(excluded.parent_ext, vod_item.parent_ext), name = excluded.name,
                   year = coalesce(excluded.year, vod_item.year), rating = coalesce(excluded.rating, vod_item.rating),
                   genre = coalesce(excluded.genre, vod_item.genre),
                   description = coalesce(excluded.description, vod_item.description),
                   season = coalesce(excluded.season, vod_item.season), episode = coalesce(excluded.episode, vod_item.episode),
                   duration_secs = coalesce(excluded.duration_secs, vod_item.duration_secs),
                   art_key = coalesce(excluded.art_key, vod_item.art_key),
                   backdrop_key = coalesce(excluded.backdrop_key, vod_item.backdrop_key),
                   uuid = coalesce(excluded.uuid, vod_item.uuid), container = coalesce(excluded.container, vod_item.container),
                   category_ext = coalesce(excluded.category_ext, vod_item.category_ext), updated_at = excluded.updated_at
                 RETURNING {VOD_COLS}"
            )))
            .bind(source_id)
            .bind(kind.as_str())
            .bind(&it.ext_id)
            .bind(&it.parent_ext)
            .bind(&it.title)
            .bind(it.year)
            .bind(&it.rating)
            .bind(&it.genre)
            .bind(&it.plot)
            .bind(it.season)
            .bind(it.episode)
            .bind(it.duration_secs)
            .bind(&it.poster)
            .bind(&it.backdrop)
            .bind(&it.uuid)
            .bind(&it.container)
            .bind(&it.category_ext)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?;
            out.push(row);
        }
        tx.commit().await?;
        Ok(out)
    }

    pub async fn vod_item(&self, id: i64) -> Result<Option<VodRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {VOD_COLS} FROM vod_item WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn vod_by_ext(
        &self,
        source_id: i64,
        kind: &str,
        ext_id: &str,
    ) -> Result<Option<VodRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {VOD_COLS} FROM vod_item WHERE source_id = ? AND kind = ? AND ext_id = ?"
        )))
        .bind(source_id)
        .bind(kind)
        .bind(ext_id)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Cached episodes of a series, in watching order (season-less specials first).
    pub async fn vod_episodes(&self, source_id: i64, series_ext: &str) -> Result<Vec<VodRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {VOD_COLS} FROM vod_item WHERE source_id = ? AND kind = 'episode' AND parent_ext = ?
             ORDER BY coalesce(season, 0), coalesce(episode, 0), id"
        )))
        .bind(source_id)
        .bind(series_ext)
        .fetch_all(self.pool())
        .await?)
    }

    // ---- recordings -------------------------------------------------------------------------

    /// Replace a source's recordings with the server's list (the server is the source of truth).
    pub async fn sync_recordings(
        &self,
        source_id: i64,
        recs: &[RecordingInfo],
    ) -> Result<Vec<RecordingRow>> {
        let mut tx = self.pool().begin().await?;
        let mut ids = Vec::new();
        for r in recs {
            let channel_id: Option<i64> =
                sqlx::query_scalar("SELECT id FROM channel WHERE source_id = ? AND ext_id = ?")
                    .bind(source_id)
                    .bind(&r.channel_ext_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO recording (source_id, ext_id, channel_id, start_utc, end_utc, state, title,
                   rule_label, size_bytes, error, file_url)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(source_id, ext_id) DO UPDATE SET channel_id = excluded.channel_id,
                   start_utc = excluded.start_utc, end_utc = excluded.end_utc, state = excluded.state,
                   title = excluded.title, rule_label = excluded.rule_label, size_bytes = excluded.size_bytes,
                   error = excluded.error, file_url = excluded.file_url
                 RETURNING id",
            )
            .bind(source_id)
            .bind(&r.ext_id)
            .bind(channel_id)
            .bind(r.start_utc)
            .bind(r.end_utc)
            .bind(r.status.as_str())
            .bind(&r.title)
            .bind(&r.rule_label)
            .bind(r.size_bytes)
            .bind(&r.error)
            .bind(&r.file_url)
            .fetch_one(&mut *tx)
            .await?;
            ids.push(id);
        }
        let keep: HashSet<i64> = ids.iter().copied().collect();
        let existing: Vec<i64> = sqlx::query_scalar("SELECT id FROM recording WHERE source_id = ?")
            .bind(source_id)
            .fetch_all(&mut *tx)
            .await?;
        for id in existing.into_iter().filter(|i| !keep.contains(i)) {
            sqlx::query("DELETE FROM recording WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.recording(id).await?);
        }
        Ok(out)
    }

    pub async fn recording_rows(&self, source_id: i64) -> Result<Vec<RecordingRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REC_COLS} FROM recording WHERE source_id = ? ORDER BY start_utc"
        )))
        .bind(source_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn rule_rows(&self, source_id: i64) -> Result<Vec<RuleRow>> {
        Ok(sqlx::query_as(
            "SELECT id, source_id, kind, ext_id, title, channel_name, schedule FROM recording_rule WHERE source_id = ? ORDER BY id",
        )
        .bind(source_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn recording(&self, id: i64) -> Result<Option<RecordingRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {REC_COLS} FROM recording WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }

    /// `(channel_id, start_utc)` of programmes that are scheduled or recording (the guide's flag).
    pub async fn scheduled_programmes(&self) -> Result<HashSet<(i64, i64)>> {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT channel_id, start_utc FROM recording
             WHERE channel_id IS NOT NULL AND state IN ('scheduled', 'recording')",
        )
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub async fn sync_rules(&self, source_id: i64, rules: &[RuleInfo]) -> Result<Vec<RuleRow>> {
        let mut tx = self.pool().begin().await?;
        let mut ids = Vec::new();
        for r in rules {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO recording_rule (source_id, kind, ext_id, title, channel_name, schedule)
                 VALUES (?,?,?,?,?,?)
                 ON CONFLICT(source_id, kind, ext_id) DO UPDATE SET title = excluded.title,
                   channel_name = excluded.channel_name, schedule = excluded.schedule
                 RETURNING id",
            )
            .bind(source_id)
            .bind(r.kind.as_str())
            .bind(&r.ext_id)
            .bind(&r.title)
            .bind(&r.channel_name)
            .bind(&r.schedule)
            .fetch_one(&mut *tx)
            .await?;
            ids.push(id);
        }
        let keep: HashSet<i64> = ids.iter().copied().collect();
        let existing: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM recording_rule WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(&mut *tx)
                .await?;
        for id in existing.into_iter().filter(|i| !keep.contains(i)) {
            sqlx::query("DELETE FROM recording_rule WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.rule(id).await?);
        }
        Ok(out)
    }

    pub async fn rule(&self, id: i64) -> Result<Option<RuleRow>> {
        Ok(sqlx::query_as(
            "SELECT id, source_id, kind, ext_id, title, channel_name, schedule FROM recording_rule WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }
}

impl RecordingRow {
    pub fn status(&self) -> RecStatus {
        match self.state.as_str() {
            "scheduled" => RecStatus::Scheduled,
            "recording" => RecStatus::Recording,
            "failed" => RecStatus::Failed,
            _ => RecStatus::Completed,
        }
    }
}
