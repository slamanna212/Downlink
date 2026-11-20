//! Local user state (PLAN §7.1): favourites, recents, resume positions, small kv. Mirrored to
//! Dispatcharr app-data by `userstate` (PLAN §7.5).

use sqlx::FromRow;

use super::Store;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FavouriteRow {
    pub channel_id: i64,
    pub position: Option<i64>,
    pub updated_at: i64,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct RecentRow {
    pub channel_id: i64,
    pub last_played_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ResumeRow {
    pub source_id: i64,
    pub kind: String,
    pub item_key: String,
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    pub updated_at: i64,
}

/// Positions at or beyond this fraction of the duration count as finished.
const FINISHED_FRACTION_PERCENT: i64 = 95;
/// Positions shorter than this are not worth resuming.
const MIN_RESUME_MS: i64 = 10_000;

impl Store {
    // ---- favourites -------------------------------------------------------------------------

    /// Add or remove a favourite. Removal keeps a tombstone (`deleted = 1`) so it syncs.
    pub async fn set_favourite(&self, channel_id: i64, favourite: bool, now: i64) -> Result<()> {
        if favourite {
            sqlx::query(
                "INSERT INTO favourite (channel_id, position, updated_at, deleted)
                 VALUES (?1, (SELECT coalesce(max(position), -1) + 1 FROM favourite WHERE deleted = 0), ?2, 0)
                 ON CONFLICT(channel_id) DO UPDATE SET
                   position = CASE WHEN favourite.deleted = 1
                                   THEN (SELECT coalesce(max(position), -1) + 1 FROM favourite WHERE deleted = 0)
                                   ELSE favourite.position END,
                   deleted = 0, updated_at = ?2",
            )
            .bind(channel_id)
            .bind(now)
            .execute(self.pool())
            .await?;
        } else {
            sqlx::query("UPDATE favourite SET deleted = 1, updated_at = ? WHERE channel_id = ? AND deleted = 0")
                .bind(now)
                .bind(channel_id)
                .execute(self.pool())
                .await?;
        }
        Ok(())
    }

    /// Live favourites (not tombstones), in order.
    pub async fn favourite_channel_ids(&self, source_id: Option<i64>) -> Result<Vec<i64>> {
        Ok(sqlx::query_scalar(
            "SELECT f.channel_id FROM favourite f JOIN channel c ON c.id = f.channel_id
             WHERE f.deleted = 0 AND (?1 IS NULL OR c.source_id = ?1)
             ORDER BY f.position, f.channel_id",
        )
        .bind(source_id)
        .fetch_all(self.pool())
        .await?)
    }

    /// Everything, tombstones included (for sync).
    pub async fn all_favourites(&self) -> Result<Vec<FavouriteRow>> {
        Ok(
            sqlx::query_as("SELECT channel_id, position, updated_at, deleted FROM favourite")
                .fetch_all(self.pool())
                .await?,
        )
    }

    /// Last-writer-wins write of a synced favourite.
    pub async fn apply_favourite(
        &self,
        channel_id: i64,
        position: i64,
        updated_at: i64,
        deleted: bool,
    ) -> Result<bool> {
        let r = sqlx::query(
            "INSERT INTO favourite (channel_id, position, updated_at, deleted) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(channel_id) DO UPDATE SET position = ?2, updated_at = ?3, deleted = ?4
             WHERE excluded.updated_at > favourite.updated_at",
        )
        .bind(channel_id)
        .bind(position)
        .bind(updated_at)
        .bind(deleted)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn prune_favourite_tombstones(&self, older_than: i64) -> Result<u64> {
        let r = sqlx::query("DELETE FROM favourite WHERE deleted = 1 AND updated_at < ?")
            .bind(older_than)
            .execute(self.pool())
            .await?;
        Ok(r.rows_affected())
    }

    // ---- recents ----------------------------------------------------------------------------

    pub async fn touch_recent(&self, channel_id: i64, now: i64) -> Result<()> {
        self.apply_recent(channel_id, now).await.map(|_| ())
    }

    /// Keeps the later timestamp.
    pub async fn apply_recent(&self, channel_id: i64, at: i64) -> Result<bool> {
        let r = sqlx::query(
            "INSERT INTO recent (channel_id, last_played_at) VALUES (?1, ?2)
             ON CONFLICT(channel_id) DO UPDATE SET last_played_at = ?2
             WHERE excluded.last_played_at > recent.last_played_at",
        )
        .bind(channel_id)
        .bind(at)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn recents(&self, limit: i64) -> Result<Vec<RecentRow>> {
        Ok(sqlx::query_as(
            "SELECT channel_id, last_played_at FROM recent ORDER BY last_played_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    /// Keep only the newest `keep` recents.
    pub async fn trim_recents(&self, keep: i64) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM recent WHERE channel_id NOT IN
               (SELECT channel_id FROM recent ORDER BY last_played_at DESC LIMIT ?)",
        )
        .bind(keep)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected())
    }

    // ---- resume / watch positions -----------------------------------------------------------

    pub async fn set_resume(
        &self,
        source_id: i64,
        kind: &str,
        item_key: &str,
        position_ms: i64,
        duration_ms: Option<i64>,
        now: i64,
    ) -> Result<()> {
        self.apply_resume(&ResumeRow {
            source_id,
            kind: kind.to_owned(),
            item_key: item_key.to_owned(),
            position_ms,
            duration_ms,
            updated_at: now,
        })
        .await
        .map(|_| ())
    }

    /// Last-writer-wins write of a resume row.
    pub async fn apply_resume(&self, r: &ResumeRow) -> Result<bool> {
        let res = sqlx::query(
            "INSERT INTO resume (source_id, kind, item_key, position_ms, duration_ms, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source_id, kind, item_key) DO UPDATE SET
               position_ms = ?4, duration_ms = ?5, updated_at = ?6
             WHERE excluded.updated_at >= resume.updated_at",
        )
        .bind(r.source_id)
        .bind(&r.kind)
        .bind(&r.item_key)
        .bind(r.position_ms)
        .bind(r.duration_ms)
        .bind(r.updated_at)
        .execute(self.pool())
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn resume(
        &self,
        source_id: i64,
        kind: &str,
        item_key: &str,
    ) -> Result<Option<ResumeRow>> {
        Ok(sqlx::query_as(
            "SELECT source_id, kind, item_key, position_ms, duration_ms, updated_at FROM resume
             WHERE source_id = ? AND kind = ? AND item_key = ?",
        )
        .bind(source_id)
        .bind(kind)
        .bind(item_key)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn all_resume(&self) -> Result<Vec<ResumeRow>> {
        Ok(sqlx::query_as(
            "SELECT source_id, kind, item_key, position_ms, duration_ms, updated_at FROM resume",
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn clear_resume(&self, source_id: i64, kind: &str, item_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM resume WHERE source_id = ? AND kind = ? AND item_key = ?")
            .bind(source_id)
            .bind(kind)
            .bind(item_key)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// "Continue watching": unfinished items with a meaningful position, newest first.
    pub async fn continue_watching(&self, limit: i64) -> Result<Vec<ResumeRow>> {
        Ok(sqlx::query_as(
            "SELECT source_id, kind, item_key, position_ms, duration_ms, updated_at FROM resume
             WHERE position_ms >= ?1
               AND (duration_ms IS NULL OR duration_ms <= 0 OR position_ms * 100 < duration_ms * ?2)
             ORDER BY updated_at DESC LIMIT ?3",
        )
        .bind(MIN_RESUME_MS)
        .bind(FINISHED_FRACTION_PERCENT)
        .bind(limit)
        .fetch_all(self.pool())
        .await?)
    }

    /// Keep only the newest `keep` resume rows (oldest dropped first, PLAN §7.5).
    pub async fn trim_resume(&self, keep: i64) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM resume WHERE rowid NOT IN
               (SELECT rowid FROM resume ORDER BY updated_at DESC LIMIT ?)",
        )
        .bind(keep)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected())
    }

    // ---- kv ---------------------------------------------------------------------------------

    pub async fn kv_get(&self, key: &str) -> Result<Option<(String, i64)>> {
        Ok(
            sqlx::query_as("SELECT value, updated_at FROM app_kv WHERE key = ?")
                .bind(key)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn kv_set(&self, key: &str, value: &str, updated_at: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO app_kv (key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        )
        .bind(key)
        .bind(value)
        .bind(updated_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

impl ResumeRow {
    /// Watched to (nearly) the end: not offered for resume, shown as watched.
    pub fn finished(&self) -> bool {
        self.duration_ms
            .is_some_and(|d| d > 0 && self.position_ms * 100 >= d * FINISHED_FRACTION_PERCENT)
    }

    /// Worth resuming: past the first seconds and not finished.
    pub fn resumable(&self) -> bool {
        self.position_ms >= MIN_RESUME_MS && !self.finished()
    }
}
