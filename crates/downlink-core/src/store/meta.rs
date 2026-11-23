//! `meta_cache`: lookups that are not catalog rows (server programme details, TMDB replies),
//! stored as our own normalised JSON with the time they were fetched. Callers own freshness.

use super::Store;
use crate::error::Result;

impl Store {
    /// `(body, fetched_at)` for `key`.
    pub async fn meta_get(&self, key: &str) -> Result<Option<(String, i64)>> {
        Ok(
            sqlx::query_as("SELECT body, fetched_at FROM meta_cache WHERE key = ?")
                .bind(key)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn meta_put(&self, key: &str, body: &str, fetched_at: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO meta_cache (key, body, fetched_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET body = ?2, fetched_at = ?3",
        )
        .bind(key)
        .bind(body)
        .bind(fetched_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Forget every entry whose key starts with `prefix` (a changed TMDB key or language).
    pub async fn meta_clear_prefix(&self, prefix: &str) -> Result<u64> {
        let pattern = format!(
            "{}%",
            prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        Ok(
            sqlx::query("DELETE FROM meta_cache WHERE key LIKE ? ESCAPE '\\'")
                .bind(pattern)
                .execute(self.pool())
                .await?
                .rows_affected(),
        )
    }

    /// Drop entries fetched before `before` (startup housekeeping).
    pub async fn meta_prune(&self, before: i64) -> Result<u64> {
        Ok(sqlx::query("DELETE FROM meta_cache WHERE fetched_at < ?")
            .bind(before)
            .execute(self.pool())
            .await?
            .rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[tokio::test]
    async fn put_get_clear_prune() {
        let s = Store::open_memory().await.unwrap();
        s.meta_put("tmdb:tv:1", "{}", 10).await.unwrap();
        s.meta_put("tmdb:tv:1", "{\"a\":1}", 20).await.unwrap();
        s.meta_put("tmdb_x", "x", 5).await.unwrap();
        s.meta_put("dpg:1:2", "y", 30).await.unwrap();
        assert_eq!(
            s.meta_get("tmdb:tv:1").await.unwrap(),
            Some(("{\"a\":1}".into(), 20))
        );
        // `_` in the prefix is literal, so `tmdb_x` survives a `tmdb:` clear and vice versa.
        assert_eq!(s.meta_clear_prefix("tmdb:").await.unwrap(), 1);
        assert!(s.meta_get("tmdb_x").await.unwrap().is_some());
        assert_eq!(s.meta_prune(25).await.unwrap(), 1);
        assert!(s.meta_get("dpg:1:2").await.unwrap().is_some());
    }
}
