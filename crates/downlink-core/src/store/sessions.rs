use sqlx::FromRow;

use super::Store;
use crate::error::Result;

/// A catch-up / HLS / TS session that must be deleted (also after a crash).
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct OpenSession {
    pub source_id: i64,
    pub kind: String,
    pub session_id: String,
}

impl Store {
    pub async fn add_open_session(
        &self,
        source_id: i64,
        kind: &str,
        session_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO open_session (source_id, kind, session_id) VALUES (?, ?, ?)",
        )
        .bind(source_id)
        .bind(kind)
        .bind(session_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn remove_open_session(
        &self,
        source_id: i64,
        kind: &str,
        session_id: &str,
    ) -> Result<()> {
        sqlx::query("DELETE FROM open_session WHERE source_id = ? AND kind = ? AND session_id = ?")
            .bind(source_id)
            .bind(kind)
            .bind(session_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn list_open_sessions(&self) -> Result<Vec<OpenSession>> {
        Ok(
            sqlx::query_as("SELECT source_id, kind, session_id FROM open_session ORDER BY rowid")
                .fetch_all(self.pool())
                .await?,
        )
    }
}
