use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};

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
    /// Artwork URL from the guide (XMLTV `<icon>`), if any.
    pub icon: Option<String>,
}

impl Store {
    pub async fn programme_row(
        &self,
        channel_id: i64,
        start_utc: i64,
    ) -> Result<Option<ProgrammeRow>> {
        Ok(sqlx::query_as(
            "SELECT channel_id, start_utc, end_utc, ext_id, title, subtitle, description, season, episode, flags, icon
             FROM programme WHERE channel_id = ? AND start_utc = ?",
        )
        .bind(channel_id)
        .bind(start_utc)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Upsert programmes in one transaction. Returns rows written.
    pub async fn upsert_programmes(&self, rows: &[ProgrammeRow]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool().begin().await?;
        Self::write_programmes(&mut tx, rows).await?;
        tx.commit().await?;
        Ok(rows.len())
    }

    async fn write_programmes(db: &mut SqliteConnection, rows: &[ProgrammeRow]) -> Result<()> {
        let mut changed = 0;
        for chunk in rows.chunks(90) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "INSERT INTO programme (channel_id, start_utc, end_utc, ext_id, title, subtitle,
                                        description, season, episode, flags, icon) ",
            );
            query.push_values(chunk, |mut b, p| {
                b.push_bind(p.channel_id)
                    .push_bind(p.start_utc)
                    .push_bind(p.end_utc)
                    .push_bind(&p.ext_id)
                    .push_bind(&p.title)
                    .push_bind(&p.subtitle)
                    .push_bind(&p.description)
                    .push_bind(p.season)
                    .push_bind(p.episode)
                    .push_bind(p.flags)
                    .push_bind(&p.icon);
            });
            query.push(
                " ON CONFLICT(channel_id, start_utc) DO UPDATE SET
                   end_utc = excluded.end_utc, ext_id = excluded.ext_id, title = excluded.title,
                   subtitle = excluded.subtitle, description = excluded.description,
                   season = excluded.season, episode = excluded.episode, flags = excluded.flags,
                   icon = excluded.icon
                 WHERE (programme.end_utc, programme.ext_id, programme.title, programme.subtitle,
                        programme.description, programme.season, programme.episode, programme.flags,
                        programme.icon)
                    IS NOT (excluded.end_utc, excluded.ext_id, excluded.title, excluded.subtitle,
                            excluded.description, excluded.season, excluded.episode, excluded.flags,
                            excluded.icon)",
            );
            changed += query
                .build()
                .persistent(chunk.len() == 90)
                .execute(&mut *db)
                .await?
                .rows_affected();
        }
        if changed >= 1_000 {
            // Bulk inserts leave more FTS segments behind. A bounded merge keeps prefix
            // searches fast without rebuilding the index or delaying small refreshes.
            sqlx::query("INSERT INTO programme_fts(programme_fts, rank) VALUES ('merge', 64)")
                .execute(db)
                .await?;
        }
        Ok(())
    }

    /// Reconcile one authoritative source window in a single transaction.
    pub async fn replace_programme_window(
        &self,
        source_id: i64,
        window: crate::source::TimeWindow,
        rows: &[ProgrammeRow],
    ) -> Result<usize> {
        let keys: std::collections::HashSet<_> =
            rows.iter().map(|p| (p.channel_id, p.start_utc)).collect();
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let previous: Vec<(i64, i64, i64)> = sqlx::query_as(
            "SELECT rowid, channel_id, start_utc FROM programme WHERE start_utc < ? AND end_utc > ?
              AND channel_id IN (SELECT id FROM channel WHERE source_id = ?)",
        )
        .bind(window.end_utc)
        .bind(window.start_utc)
        .bind(source_id)
        .fetch_all(&mut *tx)
        .await?;
        let obsolete: Vec<_> = previous
            .into_iter()
            .filter_map(|(rowid, channel, start)| {
                (!keys.contains(&(channel, start))).then_some(rowid)
            })
            .collect();
        Self::delete_programme_rows(&mut tx, &obsolete).await?;
        Self::write_programmes(&mut tx, rows).await?;
        tx.commit().await?;
        Ok(rows.len())
    }

    pub(crate) async fn delete_programme_rows(
        db: &mut SqliteConnection,
        rowids: &[i64],
    ) -> Result<u64> {
        if rowids.is_empty() {
            return Ok(0);
        }
        let result =
            sqlx::query("DELETE FROM programme WHERE rowid IN (SELECT value FROM json_each(?))")
                .bind(serde_json::to_string(rowids)?)
                .execute(db)
                .await?;
        Ok(result.rows_affected())
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

    /// Drop one source's programmes that ended before `before_utc`.
    pub async fn prune_source_programmes(&self, source_id: i64, before_utc: i64) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM programme WHERE end_utc < ?
               AND channel_id IN (SELECT id FROM channel WHERE source_id = ?)",
        )
        .bind(before_utc)
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
