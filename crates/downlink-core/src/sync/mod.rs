//! Channel/group sync: fetch -> normalise -> filter (via `catalog`) -> one transaction per entity
//! -> counts (PLAN §6.1, §7.3).

use crate::catalog::{self, DropReason};
use crate::error::Result;
use crate::normalise;
use crate::source::{AccountInfo, Source, StreamMode, UserLevel, all_channels};
use crate::store::{SourceUpdate, Store};

pub fn now_utc() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncCounts {
    pub groups: usize,
    /// Channels written.
    pub channels: usize,
    pub inserted: usize,
    pub updated: usize,
    pub removed: usize,
    pub dropped_hidden: usize,
    pub dropped_level: usize,
}

/// Connect and record what was learned (level, version, timezone, caps) on the source row.
pub async fn connect_and_record(
    src: &dyn Source,
    store: &Store,
    source_id: i64,
) -> Result<AccountInfo> {
    let info = src.connect().await?;
    store
        .update_source(
            source_id,
            &SourceUpdate {
                user_level: info.user_level.map(|l| l.as_str().to_owned()),
                server_version: info.server_version.clone(),
                server_tz: info.timezone.clone(),
                caps: src.caps().to_db(),
            },
        )
        .await?;
    Ok(info)
}

/// Sync channels and groups. On any fetch error the cache is left as it was and the error is
/// recorded in `sync_state`.
pub async fn sync_channels(
    src: &dyn Source,
    store: &Store,
    source_id: i64,
    account_level: Option<UserLevel>,
) -> Result<SyncCounts> {
    match run(src, store, source_id, account_level).await {
        Ok(c) => {
            let now = now_utc();
            store.set_sync_ok(source_id, "channels", now).await?;
            store.touch_source_sync(source_id, now).await?;
            Ok(c)
        }
        Err(e) => {
            store
                .set_sync_error(source_id, "channels", &e.message)
                .await?;
            Err(e)
        }
    }
}

async fn run(
    src: &dyn Source,
    store: &Store,
    source_id: i64,
    account_level: Option<UserLevel>,
) -> Result<SyncCounts> {
    let groups = src.groups().await?;
    let (channels, _count) = all_channels(src, 500).await?;
    // Effective stream profile (PLAN §3.5): metadata only. If unavailable (e.g. a low-privilege
    // key), keep the last stored flag rather than guessing.
    let profiles = match src.profile_info().await {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(error = %e, "stream profiles unavailable; keeping stored redirect flags");
            None
        }
    };
    let previous = store.redirect_flags(source_id).await?;

    let mut counts = SyncCounts::default();
    let mut rows = Vec::with_capacity(channels.len());
    for c in &channels {
        match catalog::sync_filter(c, account_level) {
            Some(DropReason::Hidden) => {
                counts.dropped_hidden += 1;
                continue;
            }
            Some(DropReason::AboveUserLevel) => {
                counts.dropped_level += 1;
                continue;
            }
            None => {}
        }
        let redirect = match &profiles {
            Some(p) => p.mode(c.stream_profile_id),
            None => previous
                .get(&c.ext_id)
                .copied()
                .unwrap_or(StreamMode::Unknown),
        };
        rows.push(normalise::channel_row(c, redirect));
    }

    let group_rows: Vec<(String, String)> = groups
        .into_iter()
        .map(|g| (g.ext_id, g.name.trim().to_owned()))
        .collect();
    let ids = store.sync_groups(source_id, &group_rows).await?;
    let up = store.sync_channels(source_id, &rows, &ids).await?;
    counts.groups = group_rows.len();
    counts.channels = rows.len();
    counts.inserted = up.inserted;
    counts.updated = up.updated;
    counts.removed = up.removed;
    Ok(counts)
}
