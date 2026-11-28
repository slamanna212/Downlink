//! `feed` / `feed_item` persistence (migration 0024). UTC seconds throughout.

use sqlx::FromRow;

use super::fetch::Validators;
use super::parse::{ParsedFeed, ParsedItem};
use super::{MAX_ITEMS_PER_FEED, RETENTION_SECS};
use crate::error::{AppError, Result};
use crate::store::Store;

/// Longest stored error text.
const MAX_ERROR_LEN: usize = 300;
/// Upper bound on items one `feed_items` call returns.
pub const MAX_ITEMS_QUERY: i64 = 500;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FeedRow {
    pub id: i64,
    /// The URL, or its masked form when `secret_ref` is set.
    pub url: String,
    pub secret_ref: Option<String>,
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_fetch_utc: Option<i64>,
    pub last_ok_utc: Option<i64>,
    pub last_error: Option<String>,
    pub created_utc: i64,
    pub item_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct FeedItemRow {
    pub feed_id: i64,
    pub guid: String,
    pub title: String,
    pub link: Option<String>,
    pub summary: Option<String>,
    pub published_utc: Option<i64>,
    pub fetched_utc: i64,
}

pub struct NewFeed<'a> {
    pub url: &'a str,
    pub secret_ref: Option<&'a str>,
    pub title: Option<&'a str>,
    pub site_url: Option<&'a str>,
    pub validators: &'a Validators,
    pub now: i64,
}

const FEED_COLS: &str = "f.id, f.url, f.secret_ref, f.title, f.site_url, f.etag, f.last_modified,
    f.last_fetch_utc, f.last_ok_utc, f.last_error, f.created_utc,
    (SELECT count(*) FROM feed_item i WHERE i.feed_id = f.id) AS item_count";

impl Store {
    pub async fn insert_feed(&self, f: &NewFeed<'_>) -> Result<i64> {
        let r = sqlx::query(
            "INSERT INTO feed (url, secret_ref, title, site_url, etag, last_modified,
                               last_fetch_utc, last_ok_utc, created_utc)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(f.url)
        .bind(f.secret_ref)
        .bind(f.title)
        .bind(f.site_url)
        .bind(f.validators.etag.as_deref())
        .bind(f.validators.last_modified.as_deref())
        .bind(f.now)
        .bind(f.now)
        .bind(f.now)
        .execute(self.pool())
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => {
                AppError::local("That feed is already added.")
            }
            _ => AppError::from(e),
        })?;
        Ok(r.last_insert_rowid())
    }

    pub async fn feeds(&self) -> Result<Vec<FeedRow>> {
        Ok(sqlx::query_as::<_, FeedRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {FEED_COLS} FROM feed f ORDER BY lower(coalesce(f.title, f.url)), f.id"
        )))
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn feed(&self, id: i64) -> Result<FeedRow> {
        sqlx::query_as::<_, FeedRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {FEED_COLS} FROM feed f WHERE f.id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool())
        .await?
        .ok_or_else(|| AppError::local("That feed no longer exists."))
    }

    pub async fn feed_by_url(&self, url: &str) -> Result<Option<FeedRow>> {
        Ok(sqlx::query_as::<_, FeedRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {FEED_COLS} FROM feed f WHERE f.url = ?"
        )))
        .bind(url)
        .fetch_optional(self.pool())
        .await?)
    }

    /// `last_fetch_utc` for each existing id (missing ids are left out).
    pub async fn feed_last_fetch(&self, ids: &[i64]) -> Result<Vec<(i64, Option<i64>)>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let sql = format!("SELECT id, last_fetch_utc FROM feed WHERE id IN ({marks})");
            let mut q = sqlx::query_as::<_, (i64, Option<i64>)>(sqlx::AssertSqlSafe(sql));
            for id in chunk {
                q = q.bind(id);
            }
            out.extend(q.fetch_all(self.pool()).await?);
        }
        Ok(out)
    }

    /// Delete a feed (items cascade). Returns its keychain reference, if any, for the caller to
    /// delete.
    pub async fn delete_feed(&self, id: i64) -> Result<Option<String>> {
        let r: Option<Option<String>> =
            sqlx::query_scalar("DELETE FROM feed WHERE id = ? RETURNING secret_ref")
                .bind(id)
                .fetch_optional(self.pool())
                .await?;
        Ok(r.flatten())
    }

    pub async fn record_feed_ok(
        &self,
        id: i64,
        now: i64,
        parsed: Option<&ParsedFeed>,
        v: &Validators,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE feed SET last_fetch_utc = ?, last_ok_utc = ?, last_error = NULL,
                    etag = ?, last_modified = ?,
                    title = coalesce(?, title), site_url = coalesce(?, site_url)
             WHERE id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(v.etag.as_deref())
        .bind(v.last_modified.as_deref())
        .bind(parsed.and_then(|p| p.title.as_deref()))
        .bind(parsed.and_then(|p| p.site_url.as_deref()))
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Record a failure. `message` must already be redacted (it is an `AppError` message).
    pub async fn record_feed_error(&self, id: i64, now: i64, message: &str) -> Result<()> {
        let message = super::parse::truncate_chars(&crate::redact::redact(message), MAX_ERROR_LEN);
        sqlx::query("UPDATE feed SET last_fetch_utc = ?, last_error = ? WHERE id = ?")
            .bind(now)
            .bind(message)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Insert new items and update changed ones. Returns how many were inserted or changed.
    /// `fetched_utc` keeps the first-seen time.
    pub async fn upsert_feed_items(
        &self,
        feed_id: i64,
        items: &[ParsedItem],
        now: i64,
    ) -> Result<usize> {
        let mut tx = self.pool().begin().await?;
        let mut changed = 0usize;
        for i in items {
            let inserted = sqlx::query(
                "INSERT OR IGNORE INTO feed_item
                   (feed_id, guid, title, link, summary, published_utc, fetched_utc)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(feed_id)
            .bind(&i.guid)
            .bind(&i.title)
            .bind(&i.link)
            .bind(&i.summary)
            .bind(i.published_utc)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted > 0 {
                changed += 1;
                continue;
            }
            let updated = sqlx::query(
                "UPDATE feed_item SET title = ?, link = ?, summary = ?,
                        published_utc = coalesce(?, published_utc)
                 WHERE feed_id = ? AND guid = ?
                   AND (title IS NOT ? OR link IS NOT ? OR summary IS NOT ?
                        OR (? IS NOT NULL AND published_utc IS NOT ?))",
            )
            .bind(&i.title)
            .bind(&i.link)
            .bind(&i.summary)
            .bind(i.published_utc)
            .bind(feed_id)
            .bind(&i.guid)
            .bind(&i.title)
            .bind(&i.link)
            .bind(&i.summary)
            .bind(i.published_utc)
            .bind(i.published_utc)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            changed += usize::from(updated > 0);
        }
        tx.commit().await?;
        Ok(changed)
    }

    /// Retention: drop items older than 48 h (by publish time, else first seen), then keep the
    /// newest `MAX_ITEMS_PER_FEED`.
    pub async fn prune_feed_items(&self, feed_id: i64, now: i64) -> Result<u64> {
        let old = sqlx::query(
            "DELETE FROM feed_item WHERE feed_id = ? AND coalesce(published_utc, fetched_utc) < ?",
        )
        .bind(feed_id)
        .bind(now - RETENTION_SECS)
        .execute(self.pool())
        .await?
        .rows_affected();
        let over = sqlx::query(
            "DELETE FROM feed_item WHERE feed_id = ? AND guid NOT IN (
               SELECT guid FROM feed_item WHERE feed_id = ?
               ORDER BY coalesce(published_utc, fetched_utc) DESC, fetched_utc DESC, guid
               LIMIT ?)",
        )
        .bind(feed_id)
        .bind(feed_id)
        .bind(MAX_ITEMS_PER_FEED)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(old + over)
    }

    /// Items of the given feeds, newest first (publish time, else first seen).
    pub async fn feed_items(&self, feed_ids: &[i64], limit: i64) -> Result<Vec<FeedItemRow>> {
        if feed_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = feed_ids.iter().copied().take(500).collect();
        let marks = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT feed_id, guid, title, link, summary, published_utc, fetched_utc
             FROM feed_item WHERE feed_id IN ({marks})
             ORDER BY coalesce(published_utc, fetched_utc) DESC, fetched_utc DESC, feed_id, guid
             LIMIT ?"
        );
        let mut q = sqlx::query_as::<_, FeedItemRow>(sqlx::AssertSqlSafe(sql));
        for id in &ids {
            q = q.bind(id);
        }
        Ok(q.bind(limit.clamp(1, MAX_ITEMS_QUERY))
            .fetch_all(self.pool())
            .await?)
    }
}
