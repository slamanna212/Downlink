//! XMLTV ingestion into SQLite: match channels, stream programmes in 5,000-row batched
//! transactions. A refresh that read the whole feed replaces each channel's programmes inside the
//! span the feed covers for it (so rescheduled programmes disappear); a failed or truncated
//! refresh only upserts and never deletes.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use chrono_tz::Tz;
use tokio::sync::mpsc;

use super::matching::{ChanInfo, Method, match_channels};
use super::{MAX_CHANNELS, XmltvChannel, parse_channels, try_parse_programmes};
use crate::error::{AppError, Result};
use crate::source::TimeWindow;
use crate::store::{ProgrammeRow, Store};
use crate::util::import_limit;

pub const BATCH: usize = 5_000;
const MAX_BATCH_TEXT_BYTES: usize = 8 * 1024 * 1024;
/// Stored rows (inside the retention window, after channel fan-out). 20,000 channels with two
/// weeks of guide is about 7M. Past the cap programmes are skipped and counted, not fatal.
pub(crate) const MAX_STORED_PROGRAMMES: usize = 12_000_000;
/// Stored programme text, same rule as [`MAX_STORED_PROGRAMMES`].
const MAX_PROGRAMME_TEXT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

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
    /// Programmes skipped because the stored-guide cap was reached (the rest of the guide is kept).
    pub capped: u64,
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
        "SELECT id, ext_id, tvg_id, tvg_name, name FROM channel WHERE source_id = ? LIMIT ?",
    )
    .bind(source_id)
    .bind((MAX_CHANNELS + 1) as i64)
    .fetch_all(store.pool())
    .await?;
    if rows.len() > MAX_CHANNELS {
        return Err(import_limit().into());
    }
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
    let matches = prepare_matches(store, source_id, xmltv).await?;
    save_matches(store, &matches.updates).await?;
    Ok((matches.by_xmltv, matches.summary))
}

struct ChannelMatches {
    by_xmltv: HashMap<String, Vec<i64>>,
    summary: MatchSummary,
    updates: Vec<(i64, &'static str)>,
}

async fn prepare_matches(
    store: &Store,
    source_id: i64,
    xmltv: &[XmltvChannel],
) -> Result<ChannelMatches> {
    let chans = chan_infos(store, source_id).await?;
    let remap = store.epg_remaps(source_id).await?;
    let matches = match_channels(&chans, xmltv, &remap);
    let mut summary = MatchSummary::default();
    let mut by_xmltv: HashMap<String, Vec<i64>> = HashMap::new();
    let matched: HashMap<i64, Method> = matches.iter().map(|m| (m.channel_id, m.method)).collect();
    let updates = chans
        .into_iter()
        .map(|c| {
            let method = matched.get(&c.id);
            if method.is_none() {
                summary.unmatched.push(c.name);
            }
            (c.id, method.map_or("none", |m| m.as_str()))
        })
        .collect();
    for m in matches {
        match m.method {
            Method::Id => summary.by_id += 1,
            Method::Name => summary.by_name += 1,
            Method::Display => summary.by_display += 1,
            Method::Manual => summary.manual += 1,
        }
        by_xmltv.entry(m.xmltv_id).or_default().push(m.channel_id);
    }
    Ok(ChannelMatches {
        by_xmltv,
        summary,
        updates,
    })
}

async fn save_matches(store: &Store, updates: &[(i64, &'static str)]) -> Result<()> {
    let mut tx = store.pool().begin().await?;
    for batch in updates.chunks(1000) {
        sqlx::query(
            "WITH matches AS (
               SELECT json_extract(value, '$[0]') AS id, json_extract(value, '$[1]') AS method
               FROM json_each(?)
             )
             UPDATE channel SET epg_match = matches.method FROM matches
             WHERE channel.id = matches.id AND channel.epg_match IS NOT matches.method",
        )
        .bind(serde_json::to_string(batch)?)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// What one channel's feed covers: the stored span and the programme starts seen in it.
struct Coverage {
    first_start: i64,
    last_end: i64,
    /// Sorted and de-duplicated once the feed has been read (a `Vec` keeps millions of starts
    /// at 8 bytes each).
    starts: Vec<i64>,
}

async fn channel_shifts(store: &Store, source_id: i64) -> Result<HashMap<i64, i64>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, epg_shift_secs FROM channel WHERE source_id = ? AND epg_shift_secs != 0",
    )
    .bind(source_id)
    .fetch_all(store.pool())
    .await?;
    Ok(rows.into_iter().collect())
}

/// Remove stored programmes that sit inside a channel's fed span but were not in the feed.
/// One short transaction per batch of channels: with tens of thousands of channels a single
/// transaction held the write lock long enough for unrelated writes (favourites, resume
/// positions) to time out.
async fn replace_within_coverage(store: &Store, cov: &HashMap<i64, Coverage>) -> Result<u64> {
    let mut removed = 0;
    let bounds: Vec<_> = cov
        .iter()
        .map(|(&id, c)| (id, c.first_start, c.last_end))
        .collect();
    for batch in bounds.chunks(128) {
        let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await?;
        let have: Vec<(i64, i64, i64)> = sqlx::query_as(
            "WITH bounds AS (
               SELECT json_extract(value, '$[0]') AS id, json_extract(value, '$[1]') AS first_start,
                      json_extract(value, '$[2]') AS last_end FROM json_each(?)
             )
             SELECT p.rowid, p.channel_id, p.start_utc FROM bounds b JOIN programme p
               ON p.channel_id = b.id AND p.start_utc >= b.first_start AND p.start_utc < b.last_end",
        )
        .bind(serde_json::to_string(batch)?)
        .fetch_all(&mut *tx)
        .await?;
        let obsolete: Vec<_> = have
            .into_iter()
            .filter_map(|(rowid, channel, start)| {
                cov.get(&channel)
                    .is_some_and(|c| c.starts.binary_search(&start).is_err())
                    .then_some(rowid)
            })
            .collect();
        removed += Store::delete_programme_rows(&mut tx, &obsolete).await?;
        tx.commit().await?;
    }
    Ok(removed)
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
    ingest_with_progress(store, source_id, path, default_tz, keep, &|_| {}).await
}

/// [`ingest`] calling `on_batch(programmes written so far)` after every committed batch.
pub async fn ingest_with_progress(
    store: &Store,
    source_id: i64,
    path: PathBuf,
    default_tz: Tz,
    keep: TimeWindow,
    on_batch: &(dyn Fn(usize) + Sync),
) -> Result<IngestReport> {
    let p1 = path.clone();
    let xmltv = tokio::task::spawn_blocking(move || parse_channels(&p1))
        .await
        .map_err(|_| AppError::local("Reading the guide was interrupted."))??;
    let ChannelMatches {
        by_xmltv,
        summary,
        updates,
    } = prepare_matches(store, source_id, &xmltv).await?;
    drop(xmltv);
    let shifts = channel_shifts(store, source_id).await?;

    let (tx, mut rx) = mpsc::channel::<Vec<ProgrammeRow>>(4);
    let parser = tokio::task::spawn_blocking(move || -> io::Result<_> {
        let mut batch: Vec<ProgrammeRow> = Vec::with_capacity(BATCH);
        // De-duplicate by (channel, start): the first one in a batch wins. Duplicates in real
        // feeds are near each other, so a per-batch set bounds memory.
        let mut seen: std::collections::HashSet<(i64, i64)> = std::collections::HashSet::new();
        let (mut unmatched, mut outside, mut capped) = (0u64, 0u64, 0u64);
        let mut cov: HashMap<i64, Coverage> = HashMap::new();
        let mut batch_bytes = 0;
        let mut text_bytes = 0u64;
        let mut stored = 0usize;
        let cancelled = || io::Error::new(io::ErrorKind::BrokenPipe, "The guide import stopped.");
        let stats = try_parse_programmes(&path, default_tz, |p| {
            let Some(ids) = by_xmltv.get(&p.channel) else {
                // XMLTV ids are matched case-insensitively against the channel list.
                unmatched += 1;
                return Ok(());
            };
            let row_bytes = [&p.title, &p.subtitle, &p.description, &p.icon]
                .into_iter()
                .filter_map(|part| part.as_ref())
                .map(String::len)
                .sum::<usize>();
            let mut any_in = false;
            for id in ids {
                // `tvg-shift`: this channel's guide runs that many seconds off the feed's clock.
                let shift = shifts.get(id).copied().unwrap_or(0);
                let (start, end) = (p.start_utc + shift, p.end_utc + shift);
                if end < keep.start_utc || start > keep.end_utc {
                    continue;
                }
                any_in = true;
                if !seen.insert((*id, start)) {
                    continue;
                }
                if stored >= MAX_STORED_PROGRAMMES
                    || text_bytes + row_bytes as u64 > MAX_PROGRAMME_TEXT_BYTES
                {
                    capped += 1;
                    continue;
                }
                stored += 1;
                text_bytes += row_bytes as u64;
                let c = cov.entry(*id).or_insert_with(|| Coverage {
                    first_start: start,
                    last_end: end,
                    starts: Vec::new(),
                });
                c.first_start = c.first_start.min(start);
                c.last_end = c.last_end.max(end);
                c.starts.push(start);
                batch_bytes += row_bytes;
                batch.push(ProgrammeRow {
                    channel_id: *id,
                    start_utc: start,
                    end_utc: end,
                    ext_id: None,
                    title: p.title.clone(),
                    subtitle: p.subtitle.clone(),
                    description: p.description.clone(),
                    season: p.season,
                    episode: p.episode,
                    flags: p.flags,
                    icon: p.icon.clone(),
                });
                if batch.len() >= BATCH || batch_bytes >= MAX_BATCH_TEXT_BYTES {
                    seen.clear();
                    batch_bytes = 0;
                    tx.blocking_send(std::mem::replace(&mut batch, Vec::with_capacity(BATCH)))
                        .map_err(|_| cancelled())?;
                }
            }
            if !any_in {
                outside += 1;
            }
            Ok(())
        })?;
        if !batch.is_empty() {
            tx.blocking_send(batch).map_err(|_| cancelled())?;
        }
        for c in cov.values_mut() {
            c.starts.sort_unstable();
            c.starts.dedup();
        }
        Ok((stats, unmatched, outside, capped, cov))
    });

    let mut written = 0usize;
    while let Some(batch) = rx.recv().await {
        written += store.upsert_programmes(&batch).await?;
        on_batch(written);
    }
    let (stats, unmatched, outside, capped, cov) = parser
        .await
        .map_err(|_| AppError::local("Reading the guide was interrupted."))??;
    // Only a feed read to the end may replace anything: a truncated one proves nothing about
    // what is missing from it.
    if !stats.truncated {
        save_matches(store, &updates).await?;
        if !cov.is_empty() {
            replace_within_coverage(store, &cov).await?;
        }
    }
    Ok(IngestReport {
        programmes: written,
        parsed: stats.programmes,
        dropped: stats.dropped,
        unmatched_programmes: unmatched,
        outside_window: outside,
        capped,
        assumed_tz: stats.assumed_tz,
        truncated: stats.truncated,
        matches: summary,
    })
}
