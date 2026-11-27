//! Move credentials created by earlier credential templaters into the source vault.
use sqlx::{SqliteConnection, SqlitePool};
use std::{collections::HashMap, path::Path};

use crate::auth::{SecretKind, SecretStore, SecretString, secret_ref};
use crate::error::{AppError, Result};
use crate::vault::Vault;

const SANITIZED_VERSION: i64 = 1;

pub(super) async fn migrate(
    pool: &SqlitePool,
    path: &Path,
    secrets: &dyn SecretStore,
) -> Result<()> {
    let sanitized: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;
    if sanitized >= SANITIZED_VERSION {
        return Ok(());
    }
    for index in 1..=super::BACKUPS_KEPT {
        let backup = super::backup_path(path, index);
        if !backup.try_exists()? {
            continue;
        }
        let backup_pool = super::open_pool(&backup).await?;
        let result = scrub(&backup_pool, secrets).await;
        backup_pool.close().await;
        result?;
    }
    scrub(pool, secrets).await
}

async fn checkpoint(conn: &mut SqliteConnection) -> Result<()> {
    let (busy, _, _): (i64, i64, i64) = sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(conn)
        .await?;
    if busy != 0 {
        return Err(AppError::local(
            "The library is busy. Close the other instance and try again.",
        ));
    }
    Ok(())
}

async fn scrub(pool: &SqlitePool, secrets: &dyn SecretStore) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    // The pool runs with `secure_delete = FAST`; this one-time scrub overwrites rows that held
    // credentials, so zero every freed page on its connection too (the VACUUM below rewrites
    // the file either way).
    sqlx::query("PRAGMA secure_delete = ON")
        .execute(&mut *tx)
        .await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *tx)
        .await?;
    if version >= SANITIZED_VERSION {
        return Ok(());
    }
    let has_source = has_table(&mut tx, "source").await?;
    if has_source {
        let sources: Vec<(i64, String)> = sqlx::query_as("SELECT id, uuid FROM source")
            .fetch_all(&mut *tx)
            .await?;
        let has_image_origin = has_table(&mut tx, "image_origin").await?;
        let has_meta = has_table(&mut tx, "meta_cache").await?;
        let has_backdrop: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('vod_item') WHERE name = 'backdrop_key')",
        )
        .fetch_one(&mut *tx)
        .await?;
        for (source, uuid) in sources {
            let vault = Vault::new();
            let changed = scrub_source(
                &mut tx,
                source,
                &vault,
                secrets,
                &uuid,
                has_image_origin,
                has_meta,
                has_backdrop,
            )
            .await?;
            if changed {
                let reference = secret_ref(&uuid, SecretKind::Credentials);
                secrets.set(
                    &reference,
                    &SecretString::new(serde_json::to_string(&vault.export())?),
                )?;
            }
        }
    }
    tx.commit().await?;
    let mut conn = pool.acquire().await?;
    if has_source {
        // Secure-delete updates active records. Rebuild to clear old free pages as well,
        // then discard the WAL that formerly contained the unsanitized versions.
        sqlx::query("VACUUM").execute(&mut *conn).await?;
        if has_table(&mut conn, "programme_fts").await? {
            sqlx::query("INSERT INTO programme_fts(programme_fts) VALUES ('rebuild')")
                .execute(&mut *conn)
                .await?;
        }
        checkpoint(&mut conn).await?;
    }
    sqlx::query("PRAGMA user_version = 1")
        .execute(&mut *conn)
        .await?;
    checkpoint(&mut conn).await
}

async fn has_table(conn: &mut SqliteConnection, table: &str) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name = ?)")
            .bind(table)
            .fetch_one(conn)
            .await?,
    )
}

#[derive(Clone, Copy)]
enum Field {
    Url,
    Headers,
    Catchup,
    VodList,
}

fn template(vault: &Vault, raw: &str, field: Field) -> Result<String> {
    let keys: &[&str] = match field {
        Field::Url => return Ok(vault.template_url(raw)),
        Field::Headers => &[],
        Field::Catchup => &["source"],
        Field::VodList => &["stream_icon", "icon", "cover", "backdrop_path"],
    };
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Ok(vault.template_url(raw));
    };
    let mut changed = false;
    let mut apply = |key: &str, value: &mut serde_json::Value| {
        if let Some(raw) = value.as_str() {
            let cleaned = if matches!(field, Field::Headers) {
                vault.template_header(key, raw)
            } else {
                vault.template_url(raw)
            };
            if cleaned != raw {
                *value = cleaned.into();
                changed = true;
            }
        } else if let Some(values) = value.as_array_mut() {
            for value in values {
                if let Some(raw) = value.as_str() {
                    let cleaned = vault.template_url(raw);
                    if cleaned != raw {
                        *value = cleaned.into();
                        changed = true;
                    }
                }
            }
        }
    };
    let mut apply_fields = |row: &mut serde_json::Value| {
        if let Some(map) = row.as_object_mut() {
            for (key, value) in map {
                if keys.is_empty() || keys.contains(&key.as_str()) {
                    apply(key, value);
                }
            }
        }
    };
    if let Some(rows) = json.as_array_mut() {
        for row in rows {
            apply_fields(row);
        }
    } else {
        apply_fields(&mut json);
    }
    if changed {
        Ok(serde_json::to_string(&json)?)
    } else {
        Ok(raw.to_owned())
    }
}

#[allow(clippy::too_many_arguments)]
async fn scrub_source(
    conn: &mut SqliteConnection,
    source: i64,
    vault: &Vault,
    secrets: &dyn SecretStore,
    uuid: &str,
    has_image_origin: bool,
    has_meta: bool,
    has_backdrop: bool,
) -> Result<bool> {
    let mut fields = vec![
        ("channel", "logo_key", Field::Url),
        ("channel", "play_url", Field::Url),
        ("channel", "play_headers", Field::Headers),
        ("channel", "catchup_template", Field::Catchup),
        ("vod_item", "art_key", Field::Url),
    ];
    if has_backdrop {
        fields.push(("vod_item", "backdrop_key", Field::Url));
    }
    if has_image_origin {
        fields.push(("image_origin", "url", Field::Url));
    }
    if has_meta {
        fields.push(("meta_cache", "body", Field::VodList));
    }

    let mut original: Option<HashMap<String, String>> = None;
    for (table, column, field) in fields {
        let filter = if table == "meta_cache" {
            "key LIKE ?"
        } else {
            "source_id = ?"
        };
        let mut last = i64::MIN;
        loop {
            let sql = sqlx::AssertSqlSafe(format!(
                "SELECT rowid, {column} FROM {table} WHERE {filter} AND {column} IS NOT NULL
                 AND rowid > ? ORDER BY rowid LIMIT 128"
            ));
            let query = sqlx::query_as(sql);
            let rows: Vec<(i64, String)> = if table == "meta_cache" {
                query
                    .bind(format!(
                        "{}%",
                        crate::source::xtream::vod_cache_prefix(source)
                    ))
                    .bind(last)
                    .fetch_all(&mut *conn)
                    .await?
            } else {
                query.bind(source).bind(last).fetch_all(&mut *conn).await?
            };
            if rows.is_empty() {
                break;
            }
            for (rowid, raw) in rows {
                last = rowid;
                if original.is_none() {
                    // Don't open the keychain for sources that already contain templates.
                    if template(vault, &raw, field)? == raw {
                        continue;
                    }
                    let map: HashMap<String, String> = secrets
                        .get(&secret_ref(uuid, SecretKind::Credentials))?
                        .map(|value| serde_json::from_str(value.expose()))
                        .transpose()?
                        .unwrap_or_default();
                    vault.load(map.clone());
                    original = Some(map);
                }
                let cleaned = template(vault, &raw, field)?;
                if cleaned != raw {
                    sqlx::query(sqlx::AssertSqlSafe(format!(
                        "UPDATE {table} SET {column} = ? WHERE rowid = ?"
                    )))
                    .bind(cleaned)
                    .bind(rowid)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
    }
    Ok(original.is_some_and(|entries| entries != vault.export()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MemorySecretStore;
    use crate::store::{NewSource, Store};

    #[tokio::test]
    async fn scrubs_database_and_rotating_backup_before_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let store = Store::open(&path).await.unwrap();
        let source = store
            .insert_source(&NewSource {
                kind: "m3u".into(),
                name: "Test".into(),
                base_url: None,
                secret_ref: None,
                username: None,
                tls_insecure: false,
                created_at: 1,
            })
            .await
            .unwrap();
        let url = "https://iptv.test/proxy/hls/legacy-bearer/index.m3u8";
        sqlx::query(
            "INSERT INTO channel (source_id, ext_id, name, play_url, play_headers)
                     VALUES (?, 'one', 'One', ?, ?)",
        )
        .bind(source.id)
        .bind(url)
        .bind(serde_json::json!({"X-Plex-Token": "legacy-header"}).to_string())
        .execute(store.pool())
        .await
        .unwrap();
        let backup = super::super::backup_path(&path, 1);
        sqlx::query("VACUUM INTO ?")
            .bind(backup.to_str().unwrap())
            .execute(store.pool())
            .await
            .unwrap();
        store.close().await;
        let secrets = MemorySecretStore::new();
        let store = Store::open_with_secrets(&path, &secrets).await.unwrap();
        let (play, headers): (String, String) =
            sqlx::query_as("SELECT play_url, play_headers FROM channel")
                .fetch_one(store.pool())
                .await
                .unwrap();
        let vault = Vault::new();
        let stored = secrets
            .get(&secret_ref(&source.uuid, SecretKind::Credentials))
            .unwrap()
            .unwrap();
        vault.load_secret_json(stored.expose());
        assert_eq!(vault.resolve(&play).unwrap(), url);
        let headers: HashMap<String, String> = serde_json::from_str(&headers).unwrap();
        assert_eq!(
            vault.resolve(&headers["X-Plex-Token"]).unwrap(),
            "legacy-header"
        );
        store.close().await;
        for file in [path, backup] {
            let bytes = std::fs::read(file).unwrap();
            for secret in ["legacy-bearer", "legacy-header"] {
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|slice| slice == secret.as_bytes())
                );
            }
        }
    }
}
