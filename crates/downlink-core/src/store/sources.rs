use sqlx::FromRow;

use super::Store;
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct NewSource {
    pub kind: String,
    pub name: String,
    pub base_url: Option<String>,
    /// Keychain entry name. Never the secret itself.
    pub secret_ref: Option<String>,
    pub username: Option<String>,
    pub tls_insecure: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct SourceRow {
    pub id: i64,
    pub uuid: String,
    pub kind: String,
    pub name: String,
    pub base_url: Option<String>,
    pub secret_ref: Option<String>,
    pub username: Option<String>,
    pub user_level: Option<String>,
    pub server_version: Option<String>,
    pub server_tz: Option<String>,
    pub caps: i64,
    pub tls_insecure: bool,
    /// SHA-256 of the pinned leaf certificate (hex), for sources that allow self-signed.
    pub tls_pin: Option<String>,
    /// Opt-in: live channels on this (Dispatcharr) source play through its HLS output.
    pub hls_transport: bool,
    /// Concurrent streams the account may hold (NULL = unknown or unlimited).
    pub stream_limit: Option<i64>,
    pub last_sync_at: Option<i64>,
    pub created_at: i64,
}

/// Facts learned at connect time.
#[derive(Debug, Clone, Default)]
pub struct SourceUpdate {
    pub user_level: Option<String>,
    pub server_version: Option<String>,
    pub server_tz: Option<String>,
    pub caps: i64,
    pub stream_limit: Option<i64>,
}

impl Store {
    pub async fn insert_source(&self, s: &NewSource) -> Result<SourceRow> {
        let uuid = uuid::Uuid::new_v4().to_string();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO source (uuid, kind, name, base_url, secret_ref, username, tls_insecure, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&uuid)
        .bind(&s.kind)
        .bind(&s.name)
        .bind(&s.base_url)
        .bind(&s.secret_ref)
        .bind(&s.username)
        .bind(s.tls_insecure)
        .bind(s.created_at)
        .fetch_one(self.pool())
        .await?;
        self.source(id).await
    }

    pub async fn source(&self, id: i64) -> Result<SourceRow> {
        Ok(sqlx::query_as("SELECT * FROM source WHERE id = ?")
            .bind(id)
            .fetch_one(self.pool())
            .await?)
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceRow>> {
        Ok(sqlx::query_as("SELECT * FROM source ORDER BY id")
            .fetch_all(self.pool())
            .await?)
    }

    pub async fn update_source(&self, id: i64, u: &SourceUpdate) -> Result<()> {
        sqlx::query(
            "UPDATE source SET user_level = ?, server_version = ?, server_tz = ?, caps = ?, stream_limit = ? WHERE id = ?",
        )
        .bind(&u.user_level)
        .bind(&u.server_version)
        .bind(&u.server_tz)
        .bind(u.caps)
        .bind(u.stream_limit)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn set_source_hls_transport(&self, id: i64, on: bool) -> Result<()> {
        sqlx::query("UPDATE source SET hls_transport = ? WHERE id = ?")
            .bind(on)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn set_source_tls_pin(&self, id: i64, pin: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE source SET tls_pin = ? WHERE id = ?")
            .bind(pin)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn set_source_secret_ref(&self, id: i64, secret_ref: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE source SET secret_ref = ? WHERE id = ?")
            .bind(secret_ref)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn touch_source_sync(&self, id: i64, now: i64) -> Result<()> {
        sqlx::query("UPDATE source SET last_sync_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_source(&self, id: i64) -> Result<()> {
        sqlx::query("DELETE FROM source WHERE id = ?")
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
