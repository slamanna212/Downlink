//! XMLTV ingestion into SQLite: match channels, stream programmes in 5,000-row batched
//! transactions. Upsert-only: a failed or truncated refresh never deletes existing (future)
//! programmes.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono_tz::Tz;
use tokio::sync::mpsc;

use super::matching::{ChanInfo, Method, match_channels};
use super::{XmltvChannel, parse_channels, parse_programmes};
use crate::error::{AppError, Result};
use crate::source::TimeWindow;
use crate::store::{ProgrammeRow, Store};

pub const BATCH: usize = 5_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchSummary {
    pub by_id: usize,
    pub by_name: usize,
    pub by_display: usize,
    pub manual: usize,
    /// Channel names with no guide.
    pub unmatched: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestReport {
    pub programmes: usize,
    pub parsed: u64,
    pub dropped: u64,
    /// Programmes for XMLTV channels no playlist channel maps to.
    pub unmatched_programmes: u64,
    pub outside_window: u64,
    pub assumed_tz: u64,
    pub truncated: bool,
    pub matches: MatchSummary,
}

impl Store {
    pub async fn set_epg_remap(
        &self,
        source_id: i64,
        channel_ext_id: &str,
        xmltv_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO epg_remap (source_id, channel_ext_id, xmltv_id) VALUES (?, ?, ?)
             ON CONFLICT(source_id, channel_ext_id) DO UPDATE SET xmltv_id = excluded.xmltv_id",
        )
        .bind(source_id)
        .bind(channel_ext_id)
        .bind(xmltv_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn clear_epg_remap(&self, source_id: i64, channel_ext_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM epg_remap WHERE source_id = ? AND channel_ext_id = ?")
            .bind(source_id)
            .bind(channel_ext_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn epg_remaps(&self, source_id: i64) -> Result<HashMap<String, String>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT channel_ext_id, xmltv_id FROM epg_remap WHERE source_id = ?")
                .bind(source_id)
                .fetch_all(self.pool())
                .await?;
        Ok(rows.into_iter().collect())
    }
}

type ChanRow = (i64, String, Option<String>, Option<String>, String);

async fn chan_infos(store: &Store, source_id: i64) -> Result<Vec<ChanInfo>> {
    let rows: Vec<ChanRow> = sqlx::query_as(
        "SELECT id, ext_id, tvg_id, tvg_name, name FROM channel WHERE source_id = ?",
    )
    .bind(source_id)
    .fetch_all(store.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, ext_id, tvg_id, tvg_name, name)| ChanInfo {
            id,
            ext_id,
            tvg_id,
            tvg_name,
            name,
        })
        .collect())
}

/// Match channels to the feed's channels and record the result in `channel.epg_match`.
pub async fn apply_matches(
    store: &Store,
    source_id: i64,
    xmltv: &[XmltvChannel],
) -> Result<(HashMap<String, Vec<i64>>, MatchSummary)> {
    let chans = chan_infos(store, source_id).await?;
    let remap = store.epg_remaps(source_id).await?;
    let matches = match_channels(&chans, xmltv, &remap);
    let mut summary = MatchSummary::default();
    let mut by_xmltv: HashMap<String, Vec<i64>> = HashMap::new();
    let matched: HashMap<i64, Method> = matches.iter().map(|m| (m.channel_id, m.method)).collect();
    let mut tx = store.pool().begin().await?;
    for c in &chans {
        let v = matched.get(&c.id).map_or("none", |m| m.as_str());
        sqlx::query("UPDATE channel SET epg_match = ? WHERE id = ?")
            .bind(v)
            .bind(c.id)
            .execute(&mut *tx)
            .await?;
        if !matched.contains_key(&c.id) {
            summary.unmatched.push(c.name.clone());
        }
    }
    tx.commit().await?;
    for m in matches {
        match m.method {
            Method::Id => summary.by_id += 1,
            Method::Name => summary.by_name += 1,
            Method::Display => summary.by_display += 1,
            Method::Manual => summary.manual += 1,
        }
        by_xmltv.entry(m.xmltv_id).or_default().push(m.channel_id);
    }
    Ok((by_xmltv, summary))
}

/// Ingest a downloaded XMLTV file. `keep` bounds which programmes are stored (retention window);
/// `default_tz` is used for stamps without an offset.
pub async fn ingest(
    store: &Store,
    source_id: i64,
    path: PathBuf,
    default_tz: Tz,
    keep: TimeWindow,
) -> Result<IngestReport> {
    let p1 = path.clone();
    let xmltv = tokio::task::spawn_blocking(move || parse_channels(&p1))
        .await
        .map_err(|_| AppError::local("Reading the guide was interrupted."))??;
    let (by_xmltv, summary) = apply_matches(store, source_id, &xmltv).await?;
    let by_xmltv = std::sync::Arc::new(by_xmltv);

    let (tx, mut rx) = mpsc::channel::<Vec<ProgrammeRow>>(4);
    let map = by_xmltv.clone();
    let parser = tokio::task::spawn_blocking(move || {
        let mut batch: Vec<ProgrammeRow> = Vec::with_capacity(BATCH);
        // De-duplicate by (channel, start): the first one in a batch wins. Duplicates in real
        // feeds are near each other, so a per-batch set bounds memory.
        let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
        let (mut unmatched, mut outside) = (0u64, 0u64);
        let stats = parse_programmes(&path, default_tz, |p| {
            let Some(ids) = map.get(&p.channel) else {
                // XMLTV ids are matched case-insensitively against the channel list.
                unmatched += 1;
                return;
            };
            if p.end_utc < keep.start_utc || p.start_utc > keep.end_utc {
                outside += 1;
                return;
            }
            for id in ids {
                if !seen.insert((*id, p.start_utc)) {
                    continue;
                }
                batch.push(ProgrammeRow {
                    channel_id: *id,
                    start_utc: p.start_utc,
                    end_utc: p.end_utc,
                    ext_id: None,
                    title: p.title.clone(),
                    subtitle: p.subtitle.clone(),
                    description: p.description.clone(),
                    season: p.season,
                    episode: p.episode,
                    flags: p.flags,
                });
            }
            if batch.len() >= BATCH {
                seen.clear();
                let _ = tx.blocking_send(std::mem::replace(&mut batch, Vec::with_capacity(BATCH)));
            }
        });
        if !batch.is_empty() {
            let _ = tx.blocking_send(batch);
        }
        (stats, unmatched, outside)
    });

    let mut written = 0usize;
    while let Some(batch) = rx.recv().await {
        written += store.upsert_programmes(&batch).await?;
    }
    let (stats, unmatched, outside) = parser
        .await
        .map_err(|_| AppError::local("Reading the guide was interrupted."))?;
    let stats = stats?;
    Ok(IngestReport {
        programmes: written,
        parsed: stats.programmes,
        dropped: stats.dropped,
        unmatched_programmes: unmatched,
        outside_window: outside,
        assumed_tz: stats.assumed_tz,
        truncated: stats.truncated,
        matches: summary,
    })
}
