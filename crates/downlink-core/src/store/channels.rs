use std::collections::{HashMap, HashSet};

use sqlx::FromRow;

use super::Store;
use crate::error::Result;
use crate::source::StreamMode;

/// A channel as written by sync. `group_ext_id` is resolved to a row id by the store.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelUpsert {
    pub ext_id: String,
    pub uuid: Option<String>,
    pub number: Option<f64>,
    pub name: String,
    pub group_ext_id: Option<String>,
    pub tvg_id: Option<String>,
    pub tvg_name: Option<String>,
    pub logo_key: Option<String>,
    pub quality: Option<String>,
    pub is_adult: bool,
    pub is_radio: bool,
    pub is_drm: bool,
    pub epg_shift_secs: i64,
    pub catchup_days: i64,
    pub catchup_template: Option<String>,
    pub stream_redirect: StreamMode,
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
    pub tvg_name: Option<String>,
    pub logo_key: Option<String>,
    pub quality: Option<String>,
    pub is_adult: bool,
    pub is_radio: bool,
    pub is_drm: bool,
    pub catchup_days: i64,
    pub catchup_template: Option<String>,
    pub stream_redirect: StreamMode,
    pub play_url: Option<String>,
    pub play_headers: Option<String>,
    pub epg_match: Option<String>,
    pub sort_key: Option<String>,
}

pub const CHANNEL_COLUMNS: &str = "id, source_id, ext_id, uuid, number, name, group_id, tvg_id, \
    tvg_name, logo_key, quality, is_adult, is_radio, is_drm, catchup_days, catchup_template, stream_redirect, \
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
        for (position, (ext_id, name)) in groups.iter().enumerate() {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO channel_group (source_id, ext_id, name, position) VALUES (?, ?, ?, ?)
                 ON CONFLICT(source_id, ext_id) DO UPDATE SET
                     name = excluded.name, position = excluded.position RETURNING id",
            )
            .bind(source_id)
            .bind(ext_id)
            .bind(name)
            .bind(position as i64)
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
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, ext_id FROM channel WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(&mut *tx)
                .await?;
        let mut existing: HashMap<String, i64> =
            existing.into_iter().map(|(i, e)| (e, i)).collect();
        let prior_streams: Vec<(i64, String, i64)> = sqlx::query_as(
            "SELECT s.channel_id, s.stream_id, s.position FROM channel_stream s
             JOIN channel c ON c.id = s.channel_id WHERE c.source_id = ?
             ORDER BY s.channel_id, s.position",
        )
        .bind(source_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut stored_streams: HashMap<i64, Vec<(String, i64)>> = HashMap::new();
        for (id, stream, position) in prior_streams {
            stored_streams
                .entry(id)
                .or_default()
                .push((stream, position));
        }
        let existing_ids: HashSet<i64> = existing.values().copied().collect();
        let mut stream_changes = HashMap::new();
        let mut counts = UpsertCounts::default();
        let mut seen: HashSet<i64> = HashSet::with_capacity(channels.len());

        for batch in channels.chunks(40) {
            let changed = Self::write_channels(&mut tx, source_id, batch, group_ids).await?;
            existing.extend(changed.into_iter().map(|(id, ext_id)| (ext_id, id)));
        }

        for c in channels {
            let id = *existing.get(&c.ext_id).ok_or_else(|| {
                crate::error::AppError::local("A synced channel could not be found.")
            })?;
            if existing_ids.contains(&id) {
                counts.updated += 1;
            } else {
                counts.inserted += 1;
            }
            seen.insert(id);

            let mut unique = HashSet::new();
            let streams: Vec<_> = c
                .streams
                .iter()
                .enumerate()
                .filter_map(|(pos, stream)| {
                    unique
                        .insert(stream)
                        .then_some((stream.as_str(), pos as i64))
                })
                .collect();
            let same = stored_streams
                .get(&id)
                .into_iter()
                .flatten()
                .map(|(stream, pos)| (stream.as_str(), *pos))
                .eq(streams.iter().copied());
            if same {
                stream_changes.remove(&id);
            } else {
                stream_changes.insert(id, streams);
            }
        }

        if !stream_changes.is_empty() {
            let ids: Vec<_> = stream_changes.keys().copied().collect();
            sqlx::query(
                "DELETE FROM channel_stream WHERE channel_id IN (SELECT value FROM json_each(?))",
            )
            .bind(serde_json::to_string(&ids)?)
            .execute(&mut *tx)
            .await?;
            let entries: Vec<_> = stream_changes
                .iter()
                .flat_map(|(&id, streams)| {
                    streams.iter().map(move |&(stream, pos)| (id, stream, pos))
                })
                .collect();
            for batch in entries.chunks(300) {
                let mut query = sqlx::QueryBuilder::new(
                    "INSERT INTO channel_stream (channel_id, stream_id, position) ",
                );
                query.push_values(batch, |mut values, &(id, stream, pos)| {
                    values.push_bind(id).push_bind(stream).push_bind(pos);
                });
                query
                    .build()
                    .persistent(batch.len() == 300)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        let removed: Vec<_> = existing_ids.difference(&seen).copied().collect();
        if !removed.is_empty() {
            sqlx::query("DELETE FROM channel WHERE id IN (SELECT value FROM json_each(?))")
                .bind(serde_json::to_string(&removed)?)
                .execute(&mut *tx)
                .await?;
            counts.removed = removed.len();
        }
        tx.commit().await?;
        Ok(counts)
    }

    async fn write_channels(
        tx: &mut sqlx::SqliteConnection,
        source_id: i64,
        channels: &[ChannelUpsert],
        group_ids: &HashMap<String, i64>,
    ) -> Result<Vec<(i64, String)>> {
        let mut query = sqlx::QueryBuilder::new(
            "INSERT INTO channel (source_id, ext_id, uuid, number, name, group_id, tvg_id, tvg_name, logo_key,
                   quality, is_adult, is_radio, is_drm, catchup_days, catchup_template, stream_redirect,
                   play_url, play_headers, epg_match, sort_key, epg_shift_secs) ",
        );
        query.push_values(channels, |mut b, c| {
            let group_id = c
                .group_ext_id
                .as_ref()
                .and_then(|key| group_ids.get(key))
                .copied();
            b.push_bind(source_id)
                .push_bind(&c.ext_id)
                .push_bind(&c.uuid)
                .push_bind(c.number)
                .push_bind(&c.name)
                .push_bind(group_id)
                .push_bind(&c.tvg_id)
                .push_bind(&c.tvg_name)
                .push_bind(&c.logo_key)
                .push_bind(&c.quality)
                .push_bind(c.is_adult)
                .push_bind(c.is_radio)
                .push_bind(c.is_drm)
                .push_bind(c.catchup_days)
                .push_bind(&c.catchup_template)
                .push_bind(c.stream_redirect)
                .push_bind(&c.play_url)
                .push_bind(&c.play_headers)
                .push_bind(&c.epg_match)
                .push_bind(&c.sort_key)
                .push_bind(c.epg_shift_secs);
        });
        query.push(
            " ON CONFLICT(source_id, ext_id) DO UPDATE SET
                   uuid=excluded.uuid, number=excluded.number, name=excluded.name,
                   group_id=excluded.group_id, tvg_id=excluded.tvg_id, tvg_name=excluded.tvg_name, logo_key=excluded.logo_key,
                   quality=excluded.quality, is_adult=excluded.is_adult, is_radio=excluded.is_radio,
                   is_drm=excluded.is_drm, catchup_days=excluded.catchup_days,
                   catchup_template=excluded.catchup_template, stream_redirect=excluded.stream_redirect,
                   play_url=excluded.play_url, play_headers=excluded.play_headers,
                   epg_match=CASE WHEN channel.epg_match IN ('name','display','manual') THEN channel.epg_match ELSE excluded.epg_match END,
                   sort_key=excluded.sort_key, epg_shift_secs=excluded.epg_shift_secs
                 WHERE (channel.uuid, channel.number, channel.name, channel.group_id, channel.tvg_id,
                        channel.tvg_name, channel.logo_key, channel.quality, channel.is_adult,
                        channel.is_radio, channel.is_drm, channel.catchup_days, channel.catchup_template,
                        channel.stream_redirect, channel.play_url, channel.play_headers, channel.epg_match,
                        channel.sort_key, channel.epg_shift_secs)
                    IS NOT (excluded.uuid, excluded.number, excluded.name, excluded.group_id, excluded.tvg_id,
                            excluded.tvg_name, excluded.logo_key, excluded.quality, excluded.is_adult,
                            excluded.is_radio, excluded.is_drm, excluded.catchup_days, excluded.catchup_template,
                            excluded.stream_redirect, excluded.play_url, excluded.play_headers,
                            CASE WHEN channel.epg_match IN ('name','display','manual') THEN channel.epg_match ELSE excluded.epg_match END,
                            excluded.sort_key, excluded.epg_shift_secs)
                 RETURNING id, ext_id",
        );
        Ok(query
            .build_query_as()
            .persistent(channels.len() == 40)
            .fetch_all(tx)
            .await?)
    }

    pub async fn channel(&self, id: i64) -> Result<Option<ChannelRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CHANNEL_COLUMNS} FROM channel WHERE id = ?"
        )))
        .bind(id)
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn channel_by_ext(&self, source_id: i64, ext_id: &str) -> Result<Option<ChannelRow>> {
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {CHANNEL_COLUMNS} FROM channel WHERE source_id = ? AND ext_id = ?"
        )))
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
    pub async fn redirect_flags(&self, source_id: i64) -> Result<HashMap<String, StreamMode>> {
        let rows: Vec<(String, StreamMode)> = sqlx::query_as::<_, (String, StreamMode)>(
            "SELECT ext_id, stream_redirect FROM channel WHERE source_id = ?",
        )
        .bind(source_id)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().collect())
    }
}
