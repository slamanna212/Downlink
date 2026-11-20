use sqlx::FromRow;

use super::Store;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct ProgrammeRow {
    pub channel_id: i64,
    pub start_utc: i64,
    pub end_utc: i64,
    pub ext_id: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub flags: i64,
}

impl Store {
    pub async fn programme_row(
        &self,
        channel_id: i64,
        start_utc: i64,
    ) -> Result<Option<ProgrammeRow>> {
        Ok(sqlx::query_as(
            "SELECT channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags
             FROM programme WHERE channel_id = ? AND start_utc = ?",
        )
        .bind(channel_id)
        .bind(start_utc)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Upsert programmes in one transaction. Returns rows written.
    pub async fn upsert_programmes(&self, rows: &[ProgrammeRow]) -> Result<usize> {
        let mut tx = self.pool().begin().await?;
        for p in rows {
            sqlx::query(
                "INSERT INTO programme (channel_id, start_utc, end_utc, ext_id, title, subtitle,
                                        description, season, episode, flags)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(channel_id, start_utc) DO UPDATE SET
                   end_utc = excluded.end_utc, ext_id = excluded.ext_id, title = excluded.title,
                   subtitle = excluded.subtitle, description = excluded.description,
                   season = excluded.season, episode = excluded.episode, flags = excluded.flags",
            )
            .bind(p.channel_id)
            .bind(p.start_utc)
            .bind(p.end_utc)
            .bind(&p.ext_id)
            .bind(&p.title)
            .bind(&p.subtitle)
            .bind(&p.description)
            .bind(p.season)
            .bind(p.episode)
            .bind(p.flags)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(rows.len())
    }

    /// Delete a source's programmes that overlap `[start, end)`; used before re-filling a window
    /// so programmes the server dropped do not linger.
    pub async fn delete_programmes_overlapping(
        &self,
        source_id: i64,
        start: i64,
        end: i64,
    ) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM programme WHERE start_utc < ? AND end_utc > ?
               AND channel_id IN (SELECT id FROM channel WHERE source_id = ?)",
        )
        .bind(end)
        .bind(start)
        .bind(source_id)
        .execute(self.pool())
        .await?;
        Ok(r.rows_affected())
    }

    /// Drop programmes that ended before `before_utc` (PLAN §7.4).
    pub async fn prune_programmes(&self, before_utc: i64) -> Result<u64> {
        let r = sqlx::query("DELETE FROM programme WHERE end_utc < ?")
            .bind(before_utc)
            .execute(self.pool())
            .await?;
        Ok(r.rows_affected())
    }
}
