use std::collections::{HashMap, HashSet};

use sqlx::FromRow;

use super::Store;
use crate::error::Result;

/// A channel as written by sync. `group_ext_id` is resolved to a row id by the store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelUpsert {
    pub ext_id: String,
    pub uuid: Option<String>,
    pub number: Option<f64>,
    pub name: String,
    pub group_ext_id: Option<String>,
    pub tvg_id: Option<String>,
    pub logo_key: Option<String>,
    pub quality: Option<String>,
    pub is_adult: bool,
    pub is_radio: bool,
    pub is_drm: bool,
    pub catchup_days: i64,
    pub catchup_template: Option<String>,
    pub stream_redirect: bool,
    pub play_url: Option<String>,
    pub play_headers: Option<String>,
    pub epg_match: Option<String>,
    pub sort_key: Option<String>,
    /// Failover order. Never reordered.
    pub streams: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct ChannelRow {
    pub id: i64,
    pub source_id: i64,
    pub ext_id: String,
    pub uuid: Option<String>,
    pub number: Option<f64>,
    pub name: String,
    pub group_id: Option<i64>,
    pub tvg_id: Option<String>,
    pub logo_key: Option<String>,
    pub quality: Option<String>,
    pub is_adult: bool,
    pub is_radio: bool,
    pub is_drm: bool,
    pub catchup_days: i64,
    pub catchup_template: Option<String>,
    pub stream_redirect: bool,
    pub play_url: Option<String>,
    pub play_headers: Option<String>,
    pub epg_match: Option<String>,
    pub sort_key: Option<String>,
}

pub const CHANNEL_COLUMNS: &str = "id, source_id, ext_id, uuid, number, name, group_id, tvg_id, \
    logo_key, quality, is_adult, is_radio, is_drm, catchup_days, catchup_template, stream_redirect, \
    play_url, play_headers, epg_match, sort_key";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpsertCounts {
    pub inserted: usize,
    pub updated: usize,
    pub removed: usize,
}

impl Store {
    /// Upsert groups and remove ones no longer present. Returns `ext_id -> row id`.
    pub async fn sync_groups(
        &self,
        source_id: i64,
        groups: &[(String, String)],
    ) -> Result<HashMap<String, i64>> {
        let mut tx = self.pool().begin().await?;
        let mut map = HashMap::new();
        for (ext_id, name) in groups {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO channel_group (source_id, ext_id, name) VALUES (?, ?, ?)
                 ON CONFLICT(source_id, ext_id) DO UPDATE SET name = excluded.name RETURNING id",
            )
            .bind(source_id)
            .bind(ext_id)
            .bind(name)
            .fetch_one(&mut *tx)
            .await?;
            map.insert(ext_id.clone(), id);
        }
        let keep: HashSet<i64> = map.values().copied().collect();
        let existing: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM channel_group WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(&mut *tx)
                .await?;
        for id in existing.into_iter().filter(|i| !keep.contains(i)) {
            sqlx::query("DELETE FROM channel_group WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(map)
    }

    /// Diffed, batched channel upsert in one transaction. Rows are keyed by `(source_id, ext_id)`
    /// so ids are stable across refreshes; channels absent from `channels` are removed.
    pub async fn sync_channels(
        &self,
        source_id: i64,
        channels: &[ChannelUpsert],
        group_ids: &HashMap<String, i64>,
    ) -> Result<UpsertCounts> {
        let mut tx = self.pool().begin().await?;
        let existing: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, ext_id FROM channel WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(&mut *tx)
                .await?;
        let existing: HashMap<String, i64> = existing.into_iter().map(|(i, e)| (e, i)).collect();
        let mut counts = UpsertCounts::default();
        let mut seen: HashSet<i64> = HashSet::with_capacity(channels.len());

        for c in channels {
            let group_id = c
                .group_ext_id
                .as_ref()
                .and_then(|g| group_ids.get(g))
                .copied();
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO channel (source_id, ext_id, uuid, number, name, group_id, tvg_id, logo_key,
                   quality, is_adult, is_radio, is_drm, catchup_days, catchup_template, stream_redirect,
                   play_url, play_headers, epg_match, sort_key)
                 VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(source_id, ext_id) DO UPDATE SET
                   uuid=excluded.uuid, number=excluded.number, name=excluded.name,
                   group_id=excluded.group_id, tvg_id=excluded.tvg_id, logo_key=excluded.logo_key,
                   quality=excluded.quality, is_adult=excluded.is_adult, is_radio=excluded.is_radio,
                   is_drm=excluded.is_drm, catchup_days=excluded.catchup_days,
                   catchup_template=excluded.catchup_template, stream_redirect=excluded.stream_redirect,
                   play_url=excluded.play_url, play_headers=excluded.play_headers,
                   epg_match=excluded.epg_match, sort_key=excluded.sort_key
                 RETURNING id",
            )
            .bind(source_id)
            .bind(&c.ext_id)
            .bind(&c.uuid)
            .bind(c.number)
            .bind(&c.name)
            .bind(group_id)
            .bind(&c.tvg_id)
            .bind(&c.logo_key)
            .bind(&c.quality)
            .bind(c.is_adult)
            .bind(c.is_radio)
            .bind(c.is_drm)
            .bind(c.catchup_days)
            .bind(&c.catchup_template)
            .bind(c.stream_redirect)
            .bind(&c.play_url)
            .bind(&c.play_headers)
            .bind(&c.epg_match)
            .bind(&c.sort_key)
            .fetch_one(&mut *tx)
            .await?;
            if existing.contains_key(&c.ext_id) {
                counts.updated += 1;
            } else {
                counts.inserted += 1;
            }
            seen.insert(id);

            sqlx::query("DELETE FROM channel_stream WHERE channel_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            for (pos, sid) in c.streams.iter().enumerate() {
                sqlx::query(
                    "INSERT OR IGNORE INTO channel_stream (channel_id, stream_id, position) VALUES (?, ?, ?)",
                )
                .bind(id)
                .bind(sid)
                .bind(pos as i64)
                .execute(&mut *tx)
                .await?;
            }
        }

        for id in existing.values().filter(|i| !seen.contains(i)) {
            sqlx::query("DELETE FROM channel WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            counts.removed += 1;
        }
        tx.commit().await?;
        Ok(counts)
    }

    pub async fn channel(&self, id: i64) -> Result<Option<ChannelRow>> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM channel WHERE id = ?"
        ))
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn channel_by_ext(&self, source_id: i64, ext_id: &str) -> Result<Option<ChannelRow>> {
        Ok(sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM channel WHERE source_id = ? AND ext_id = ?"
        ))
        .bind(source_id)
        .bind(ext_id)
        .fetch_optional(self.pool())
        .await?)
    }

    /// Stream ids in failover order.
    pub async fn channel_stream_ids(&self, channel_id: i64) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar(
            "SELECT stream_id FROM channel_stream WHERE channel_id = ? ORDER BY position",
        )
        .bind(channel_id)
        .fetch_all(self.pool())
        .await?)
    }

    /// `tvg_id -> channel row ids` for a source (several channels may share an EPG id).
    pub async fn channels_by_tvg(&self, source_id: i64) -> Result<HashMap<String, Vec<i64>>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT tvg_id, id FROM channel WHERE source_id = ? AND tvg_id IS NOT NULL AND tvg_id != ''",
        )
        .bind(source_id)
        .fetch_all(self.pool())
        .await?;
        let mut m: HashMap<String, Vec<i64>> = HashMap::new();
        for (t, i) in rows {
            m.entry(t).or_default().push(i);
        }
        Ok(m)
    }

    /// `uuid -> channel row id` for a source.
    pub async fn channels_by_uuid(&self, source_id: i64) -> Result<HashMap<String, i64>> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT uuid, id FROM channel WHERE source_id = ? AND uuid IS NOT NULL")
                .bind(source_id)
                .fetch_all(self.pool())
                .await?;
        Ok(rows.into_iter().collect())
    }
}

impl Store {
    /// `ext_id -> stream_redirect` as currently stored (used to keep the last known value when
    /// profile metadata cannot be fetched).
    pub async fn redirect_flags(&self, source_id: i64) -> Result<HashMap<String, bool>> {
        let rows: Vec<(String, bool)> =
            sqlx::query_as("SELECT ext_id, stream_redirect FROM channel WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(self.pool())
                .await?;
        Ok(rows.into_iter().collect())
    }
}
