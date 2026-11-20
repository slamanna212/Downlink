//! SQLite read model (ADR-0003, PLAN §7). One pool, WAL, `busy_timeout=5s`, forward-only
//! migrations. Nothing secret is ever written here: sources hold only a keychain reference.

mod channels;
mod programmes;
mod sessions;
mod sources;
mod userstate;
mod vod;

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::Result;

pub use channels::{ChannelRow, ChannelUpsert, UpsertCounts};
pub use programmes::ProgrammeRow;
pub use sessions::OpenSession;
pub use sources::{NewSource, SourceRow, SourceUpdate};
pub use userstate::{FavouriteRow, RecentRow, ResumeRow};
pub use vod::{RecordingRow, RuleRow, VodRow};

/// Handle to the local database. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (creating if needed) a file database and run migrations.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path.as_ref())
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await?;
        backup_before_migrate(&pool, path.as_ref()).await?;
        Self::from_pool(pool).await
    }

    /// In-memory database (single connection so every query sees the same data).
    pub async fn open_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Self::from_pool(pool).await
    }

    async fn from_pool(pool: SqlitePool) -> Result<Self> {
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    /// The underlying pool, for `catalog/` and `epg/` read queries.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Record the outcome of syncing one entity (`channels`, `guide`, …).
    pub async fn set_sync_ok(&self, source_id: i64, entity: &str, now: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO sync_state (source_id, entity, last_ok, last_error) VALUES (?, ?, ?, NULL)
             ON CONFLICT(source_id, entity) DO UPDATE SET last_ok = excluded.last_ok, last_error = NULL",
        )
        .bind(source_id)
        .bind(entity)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `error` must already be redacted (it is when it comes from an `AppError`).
    pub async fn set_sync_error(&self, source_id: i64, entity: &str, error: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO sync_state (source_id, entity, last_ok, last_error) VALUES (?, ?, NULL, ?)
             ON CONFLICT(source_id, entity) DO UPDATE SET last_error = excluded.last_error",
        )
        .bind(source_id)
        .bind(entity)
        .bind(crate::redact::redact(error))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `(last_ok, last_error)` for an entity.
    pub async fn sync_state(
        &self,
        source_id: i64,
        entity: &str,
    ) -> Result<Option<(Option<i64>, Option<String>)>> {
        let row: Option<(Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT last_ok, last_error FROM sync_state WHERE source_id = ? AND entity = ?",
        )
        .bind(source_id)
        .bind(entity)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }
}

/// Number of backups kept next to the database (`<db>.bak1` newest, `<db>.bak2` older).
pub const BACKUPS_KEPT: usize = 2;

fn backup_path(db: &Path, n: usize) -> std::path::PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(format!(".bak{n}"));
    std::path::PathBuf::from(name)
}

/// PLAN §7.6: before applying pending migrations to a database that already holds data, copy it
/// (consistent, WAL-aware, via `VACUUM INTO`) and keep the last two copies. Fresh databases and
/// up-to-date ones are not backed up.
async fn backup_before_migrate(pool: &SqlitePool, db: &Path) -> Result<()> {
    let has_table: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_optional(pool)
    .await?;
    if has_table.is_none() {
        return Ok(());
    }
    let applied: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await?;
    let Some(applied) = applied else {
        return Ok(());
    };
    let pending = sqlx::migrate!("./migrations")
        .iter()
        .any(|m| m.version > applied && !m.migration_type.is_down_migration());
    if !pending {
        return Ok(());
    }
    // Rotate: drop the oldest, shift the rest up.
    for n in (1..BACKUPS_KEPT).rev() {
        let from = backup_path(db, n);
        if from.exists() {
            std::fs::rename(&from, backup_path(db, n + 1))?;
        }
    }
    let target = backup_path(db, 1);
    let _ = std::fs::remove_file(&target);
    let target_str = target
        .to_str()
        .ok_or_else(|| crate::AppError::local("The database path is not valid text."))?
        .replace('\'', "''");
    sqlx::query(sqlx::AssertSqlSafe(format!("VACUUM INTO '{target_str}'")))
        .execute(pool)
        .await?;
    tracing::info!("database backed up before migration");
    Ok(())
}

#[cfg(test)]
mod tests;
