//! VOD cache, recordings and DVR rules. Local row ids are what the UI sees.

use std::collections::{HashMap, HashSet};

use sqlx::FromRow;

use super::Store;
use crate::error::Result;
use crate::source::{RecStatus, RecordingInfo, RuleInfo, VodCategoryInfo, VodItem, VodKind};

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
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    /// JSON [`crate::source::VodExtra`], when the source reported any.
    pub meta: Option<String>,
}

impl VodRow {
    /// The extra fields the source reported (empty when none or unreadable).
    pub fn extra(&self) -> crate::source::VodExtra {
        self.meta
            .as_deref()
            .and_then(|m| serde_json::from_str(m).ok())
            .unwrap_or_default()
    }
}

const VOD_COLS: &str = "id, source_id, kind, ext_id, parent_ext, name, year, rating, genre, description, \
    season, episode, duration_secs, art_key, backdrop_key, uuid, container, category_ext, tmdb_id, imdb_id, meta";

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
    pub rule_ext: Option<String>,
    pub tvg_id: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub programme_ext: Option<String>,
}

const REC_COLS: &str = "id, source_id, ext_id, channel_id, start_utc, end_utc, state, title, rule_label, size_bytes, error, file_url, rule_ext, tvg_id, subtitle, description, season, episode, programme_ext";

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
        let items: Vec<_> = items.iter().filter_map(|it| Some((it.kind?, it))).collect();
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool().begin().await?;
        let mut out = Vec::with_capacity(items.len());
        for batch in items.chunks(50) {
            let unique: HashSet<_> = batch.iter().map(|(kind, it)| (*kind, &it.ext_id)).collect();
            if unique.len() == batch.len() {
                out.extend(Self::write_vod_items(&mut tx, source_id, batch, now).await?);
            } else {
                // Preserve the intermediate row returned by each occurrence of a duplicate.
                for item in batch {
                    out.extend(
                        Self::write_vod_items(&mut tx, source_id, std::slice::from_ref(item), now)
                            .await?,
                    );
                }
            }
        }
        tx.commit().await?;
        Ok(out)
    }

    async fn write_vod_items(
        db: &mut sqlx::SqliteConnection,
        source_id: i64,
        items: &[(VodKind, &VodItem)],
        now: i64,
    ) -> Result<Vec<VodRow>> {
        let mut query = sqlx::QueryBuilder::new(
            "INSERT INTO vod_item (source_id, kind, ext_id, parent_ext, name, year, rating, genre, description,
                   season, episode, duration_secs, art_key, backdrop_key, uuid, container, category_ext, tmdb_id, imdb_id, meta, updated_at) ",
        );
        query.push_values(items, |mut b, &(kind, it)| {
            b.push_bind(source_id)
                .push_bind(kind.as_str())
                .push_bind(&it.ext_id)
                .push_bind(&it.parent_ext)
                .push_bind(&it.title)
                .push_bind(it.year)
                .push_bind(&it.rating)
                .push_bind(&it.genre)
                .push_bind(&it.plot)
                .push_bind(it.season)
                .push_bind(it.episode)
                .push_bind(it.duration_secs)
                .push_bind(&it.poster)
                .push_bind(&it.backdrop)
                .push_bind(&it.uuid)
                .push_bind(&it.container)
                .push_bind(&it.category_ext)
                .push_bind(&it.tmdb_id)
                .push_bind(&it.imdb_id)
                .push_bind(
                    it.extra
                        .as_ref()
                        .filter(|x| !x.is_empty())
                        .and_then(|x| serde_json::to_string(x).ok()),
                )
                .push_bind(now);
        });
        query.push(format!(
            " ON CONFLICT(source_id, kind, ext_id) DO UPDATE SET
                   parent_ext = coalesce(excluded.parent_ext, vod_item.parent_ext), name = excluded.name,
                   year = coalesce(excluded.year, vod_item.year), rating = coalesce(excluded.rating, vod_item.rating),
                   genre = coalesce(excluded.genre, vod_item.genre),
                   description = coalesce(excluded.description, vod_item.description),
                   season = coalesce(excluded.season, vod_item.season), episode = coalesce(excluded.episode, vod_item.episode),
                   duration_secs = coalesce(excluded.duration_secs, vod_item.duration_secs),
                   art_key = coalesce(excluded.art_key, vod_item.art_key),
                   backdrop_key = coalesce(excluded.backdrop_key, vod_item.backdrop_key),
                   uuid = coalesce(excluded.uuid, vod_item.uuid), container = coalesce(excluded.container, vod_item.container),
                   category_ext = coalesce(excluded.category_ext, vod_item.category_ext),
                   tmdb_id = coalesce(excluded.tmdb_id, vod_item.tmdb_id), imdb_id = coalesce(excluded.imdb_id, vod_item.imdb_id),
                   meta = coalesce(excluded.meta, vod_item.meta),
                   updated_at = excluded.updated_at

                 RETURNING {VOD_COLS}",
        ));
        let rows: Vec<VodRow> = query
            .build_query_as()
            .persistent(matches!(items.len(), 1 | 50))
            .fetch_all(db)
            .await?;
        let order: HashMap<_, _> = items
            .iter()
            .enumerate()
            .map(|(index, (kind, it))| ((kind.as_str(), it.ext_id.as_str()), index))
            .collect();
        let mut rows = rows;
        rows.sort_by_key(|r| order.get(&(r.kind.as_str(), r.ext_id.as_str())).copied());
        Ok(rows)
    }

    pub(crate) async fn vod_resumes(&self, ids: &[i64]) -> Result<HashMap<i64, super::ResumeRow>> {
        let rows = sqlx::query(
            "SELECT v.id, r.source_id, r.kind, r.item_key, r.position_ms, r.duration_ms, r.updated_at
             FROM vod_item v JOIN resume r
               ON r.source_id = v.source_id AND r.kind = v.kind AND r.item_key = v.ext_id
             WHERE v.id IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(ids)?)
        .try_map(|row| {
            use sqlx::Row;
            Ok((row.try_get("id")?, super::ResumeRow::from_row(&row)?))
        })
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().collect())
    }

    pub(crate) async fn vod_season_counts(&self, ids: &[i64]) -> Result<HashMap<i64, i64>> {
        let rows: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT v.id, (SELECT count(DISTINCT coalesce(e.season, 0)) FROM vod_item e
               WHERE e.source_id = v.source_id AND e.kind = 'episode' AND e.parent_ext = v.ext_id)
             FROM vod_item v WHERE v.id IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(ids)?)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Replace one source's cached VOD categories of a kind (one transaction).
    pub async fn replace_vod_categories(
        &self,
        source_id: i64,
        kind: VodKind,
        categories: &[VodCategoryInfo],
    ) -> Result<()> {
        let kind = kind.as_str();
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM vod_category WHERE source_id = ? AND kind = ?")
            .bind(source_id)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
        for c in categories {
            sqlx::query(
                "INSERT OR REPLACE INTO vod_category (source_id, kind, ext_id, name, item_count)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(source_id)
            .bind(kind)
            .bind(&c.ext_id)
            .bind(&c.name)
            .bind(c.count)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Cached VOD categories of a kind, in provider order.
    pub async fn vod_categories(
        &self,
        source_id: i64,
        kind: VodKind,
    ) -> Result<Vec<VodCategoryInfo>> {
        let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
            "SELECT ext_id, name, item_count FROM vod_category
             WHERE source_id = ? AND kind = ? ORDER BY rowid",
        )
        .bind(source_id)
        .bind(kind.as_str())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(ext_id, name, count)| VodCategoryInfo {
                ext_id,
                name,
                count,
            })
            .collect())
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
                   rule_label, size_bytes, error, file_url, rule_ext, tvg_id,
                   subtitle, description, season, episode, programme_ext)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(source_id, ext_id) DO UPDATE SET channel_id = excluded.channel_id,
                   start_utc = excluded.start_utc, end_utc = excluded.end_utc, state = excluded.state,
                   title = excluded.title, rule_label = excluded.rule_label, size_bytes = excluded.size_bytes,
                   error = excluded.error, file_url = excluded.file_url,
                   rule_ext = excluded.rule_ext, tvg_id = excluded.tvg_id,
                   subtitle = excluded.subtitle, description = excluded.description,
                   season = excluded.season, episode = excluded.episode,
                   programme_ext = excluded.programme_ext
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
            .bind(&r.rule_ext)
            .bind(&r.tvg_id)
            .bind(&r.subtitle)
            .bind(&r.description)
            .bind(r.season)
            .bind(r.episode)
            .bind(&r.programme_ext)
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
