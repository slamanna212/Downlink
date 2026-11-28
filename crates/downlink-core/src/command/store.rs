//! `command_center` / `command_center_tile` persistence. Rows hold channel ids as stored; the
//! service drops tiles whose channel is hidden (`catalog/`), and a deleted channel's tiles go with
//! it (`ON DELETE CASCADE`).

use sqlx::FromRow;

use super::ValidSave;
use crate::error::{AppError, Result};
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct CenterRow {
    pub id: i64,
    pub name: String,
    pub layout_style: String,
    pub audible: i64,
    pub rail_json: String,
    pub created_utc: i64,
    pub updated_utc: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromRow)]
pub struct TileRow {
    pub center_id: i64,
    pub slot: i64,
    pub channel_id: i64,
}

const CENTER_COLS: &str = "id, name, layout_style, audible, rail_json, created_utc, updated_utc";

fn not_found() -> AppError {
    AppError::local("That command center no longer exists.")
}

/// Every center, most recently changed first.
pub async fn list(store: &Store) -> Result<Vec<CenterRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CENTER_COLS} FROM command_center ORDER BY updated_utc DESC, id DESC"
    )))
    .fetch_all(store.pool())
    .await?)
}

pub async fn get(store: &Store, id: i64) -> Result<Option<CenterRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CENTER_COLS} FROM command_center WHERE id = ?"
    )))
    .bind(id)
    .fetch_optional(store.pool())
    .await?)
}

/// Tiles of every center (or one), in slot order.
pub async fn tiles(store: &Store, center_id: Option<i64>) -> Result<Vec<TileRow>> {
    Ok(sqlx::query_as(
        "SELECT center_id, slot, channel_id FROM command_center_tile
         WHERE ?1 IS NULL OR center_id = ?1 ORDER BY center_id, slot",
    )
    .bind(center_id)
    .fetch_all(store.pool())
    .await?)
}

/// Insert (no id) or replace (id) a center and its tiles in one transaction. Tiles on channels that
/// no longer exist are skipped. Returns the center id.
pub async fn save(store: &Store, v: &ValidSave, now: i64) -> Result<i64> {
    let mut tx = store.pool().begin().await?;
    let id =
        match v.id {
            Some(id) => {
                let done = sqlx::query(
                "UPDATE command_center SET name = ?, layout_style = ?, audible = ?, rail_json = ?,
                 updated_utc = ? WHERE id = ?",
            )
            .bind(&v.name)
            .bind(&v.layout_style)
            .bind(v.audible)
            .bind(&v.rail_json)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
                if done.rows_affected() == 0 {
                    return Err(not_found());
                }
                sqlx::query("DELETE FROM command_center_tile WHERE center_id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                id
            }
            None => sqlx::query_scalar(
                "INSERT INTO command_center (name, layout_style, audible, rail_json, created_utc,
             updated_utc) VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
            )
            .bind(&v.name)
            .bind(&v.layout_style)
            .bind(v.audible)
            .bind(&v.rail_json)
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await?,
        };
    for (slot, channel_id) in &v.tiles {
        sqlx::query(
            "INSERT INTO command_center_tile (center_id, slot, channel_id)
             SELECT ?, ?, id FROM channel WHERE id = ?",
        )
        .bind(id)
        .bind(slot)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

/// Delete a center (tiles cascade). Deleting one that is already gone is not an error.
pub async fn delete(store: &Store, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM command_center WHERE id = ?")
        .bind(id)
        .execute(store.pool())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
