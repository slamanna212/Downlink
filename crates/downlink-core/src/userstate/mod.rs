//! Favourites / recents / resume sync via Dispatcharr app-data (PLAN §7.5).
//!
//! One JSON document under [`APP_DATA_KEY`] (128 KiB max). Every item is last-writer-wins by its
//! `u` timestamp; deleted favourites stay as `d: true` for 30 days so deletes do not resurrect.
//! Local state is authoritative; sync is best-effort. Only Dispatcharr sources participate.
//!
//! Items for sources or channels this machine does not know are carried through the merge
//! untouched, so one machine never erases another's state.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::Result;
use crate::store::{ResumeRow, Store};

/// 21 characters: within Dispatcharr's 8-64 key rule.
pub const APP_DATA_KEY: &str = "com.downlink.state.v1";
pub const MAX_BYTES: usize = 128 * 1024;
pub const MAX_FAVOURITES: usize = 500;
pub const MAX_RECENTS: usize = 200;
pub const MAX_RESUME: usize = 500;
pub const TOMBSTONE_SECS: i64 = 30 * 86_400;
const PREFS_KV: &str = "synced_prefs";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavItem {
    pub s: String,
    pub c: String,
    #[serde(default)]
    pub p: i64,
    pub u: i64,
    #[serde(default)]
    pub d: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeItem {
    pub s: String,
    pub k: String,
    pub i: String,
    pub ms: i64,
    #[serde(default)]
    pub dur: Option<i64>,
    pub u: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentItem {
    pub s: String,
    pub c: String,
    pub u: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateDoc {
    pub v: u32,
    #[serde(default)]
    pub favourites: Vec<FavItem>,
    #[serde(default)]
    pub resume: Vec<ResumeItem>,
    #[serde(default)]
    pub recents: Vec<RecentItem>,
    /// Free-form; the whole object is last-writer-wins by its own `u`.
    #[serde(default)]
    pub prefs: Map<String, Value>,
}

impl Default for StateDoc {
    fn default() -> Self {
        Self {
            v: 1,
            favourites: vec![],
            resume: vec![],
            recents: vec![],
            prefs: Map::new(),
        }
    }
}

fn prefs_u(p: &Map<String, Value>) -> i64 {
    p.get("u").and_then(Value::as_i64).unwrap_or(0)
}

/// Pure merge: per-item last-writer-wins, tombstone expiry, caps. Symmetric (`merge(a,b) ==
/// merge(b,a)`) and idempotent.
pub fn merge(a: &StateDoc, b: &StateDoc, now: i64) -> StateDoc {
    // favourites: key (s, c); tie-break (u, d, p)
    let mut fav: HashMap<(String, String), FavItem> = HashMap::new();
    for it in a.favourites.iter().chain(&b.favourites) {
        let k = (it.s.clone(), it.c.clone());
        match fav.get(&k) {
            Some(cur) if (cur.u, cur.d, cur.p) >= (it.u, it.d, it.p) => {}
            _ => {
                fav.insert(k, it.clone());
            }
        }
    }
    let mut favourites: Vec<FavItem> = fav
        .into_values()
        .filter(|f| !(f.d && f.u < now - TOMBSTONE_SECS))
        .collect();
    favourites.sort_by(|x, y| (x.p, &x.s, &x.c).cmp(&(y.p, &y.s, &y.c)));
    if favourites.len() > MAX_FAVOURITES {
        // Drop the oldest tombstones first, then the oldest-updated live items.
        favourites.sort_by_key(|f| (!f.d, std::cmp::Reverse(f.u)));
        favourites.truncate(MAX_FAVOURITES);
        favourites.sort_by(|x, y| (x.p, &x.s, &x.c).cmp(&(y.p, &y.s, &y.c)));
    }

    let mut res: HashMap<(String, String, String), ResumeItem> = HashMap::new();
    for it in a.resume.iter().chain(&b.resume) {
        let k = (it.s.clone(), it.k.clone(), it.i.clone());
        match res.get(&k) {
            Some(cur) if (cur.u, cur.ms) >= (it.u, it.ms) => {}
            _ => {
                res.insert(k, it.clone());
            }
        }
    }
    let mut resume: Vec<ResumeItem> = res.into_values().collect();
    resume.sort_by(|x, y| {
        y.u.cmp(&x.u)
            .then_with(|| (&x.s, &x.k, &x.i).cmp(&(&y.s, &y.k, &y.i)))
    });
    resume.truncate(MAX_RESUME);

    let mut rec: HashMap<(String, String), RecentItem> = HashMap::new();
    for it in a.recents.iter().chain(&b.recents) {
        let k = (it.s.clone(), it.c.clone());
        if rec.get(&k).is_none_or(|cur| cur.u < it.u) {
            rec.insert(k, it.clone());
        }
    }
    let mut recents: Vec<RecentItem> = rec.into_values().collect();
    recents.sort_by(|x, y| y.u.cmp(&x.u).then_with(|| (&x.s, &x.c).cmp(&(&y.s, &y.c))));
    recents.truncate(MAX_RECENTS);

    let prefs = if prefs_u(&b.prefs) > prefs_u(&a.prefs) {
        b.prefs.clone()
    } else {
        a.prefs.clone()
    };
    StateDoc {
        v: 1,
        favourites,
        resume,
        recents,
        prefs,
    }
}

/// Trim until the serialised document fits the server's per-value limit: oldest resume first,
/// then oldest recents, then oldest tombstones.
pub fn fit_to_limit(doc: &mut StateDoc) -> Result<()> {
    doc.resume.sort_by_key(|r| std::cmp::Reverse(r.u));
    doc.recents.sort_by_key(|r| std::cmp::Reverse(r.u));
    loop {
        if serde_json::to_vec(doc)?.len() <= MAX_BYTES {
            return Ok(());
        }
        if !doc.resume.is_empty() {
            let n = doc.resume.len();
            doc.resume.truncate(n - n.div_ceil(10));
        } else if !doc.recents.is_empty() {
            let n = doc.recents.len();
            doc.recents.truncate(n - n.div_ceil(10));
        } else if let Some(i) = doc.favourites.iter().position(|f| f.d) {
            doc.favourites.remove(i);
        } else {
            return Ok(());
        }
    }
}

/// `(source uuid, channel ext id)` for every channel of a Dispatcharr source, keyed by row id,
/// and the reverse.
struct ChannelIndex {
    by_id: HashMap<i64, (String, String)>,
    by_key: HashMap<(String, String), i64>,
    uuid_of_source: HashMap<i64, String>,
    source_of_uuid: HashMap<String, i64>,
}

async fn index(store: &Store) -> Result<ChannelIndex> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT c.id, s.uuid, c.ext_id FROM channel c JOIN source s ON s.id = c.source_id
         WHERE s.kind = 'dispatcharr'",
    )
    .fetch_all(store.pool())
    .await?;
    let srcs: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, uuid FROM source WHERE kind = 'dispatcharr'")
            .fetch_all(store.pool())
            .await?;
    Ok(ChannelIndex {
        by_id: rows
            .iter()
            .map(|(i, s, c)| (*i, (s.clone(), c.clone())))
            .collect(),
        by_key: rows.into_iter().map(|(i, s, c)| ((s, c), i)).collect(),
        source_of_uuid: srcs.iter().map(|(i, u)| (u.clone(), *i)).collect(),
        uuid_of_source: srcs.into_iter().collect(),
    })
}

/// This machine's state as a document.
pub async fn local_doc(store: &Store) -> Result<StateDoc> {
    let ix = index(store).await?;
    let favourites = store
        .all_favourites()
        .await?
        .into_iter()
        .filter_map(|f| {
            let (s, c) = ix.by_id.get(&f.channel_id)?.clone();
            Some(FavItem {
                s,
                c,
                p: f.position.unwrap_or(0),
                u: f.updated_at,
                d: f.deleted,
            })
        })
        .collect();
    let recents = store
        .recents(MAX_RECENTS as i64)
        .await?
        .into_iter()
        .filter_map(|r| {
            let (s, c) = ix.by_id.get(&r.channel_id)?.clone();
            Some(RecentItem {
                s,
                c,
                u: r.last_played_at,
            })
        })
        .collect();
    let resume = store
        .all_resume()
        .await?
        .into_iter()
        .filter_map(|r| {
            Some(ResumeItem {
                s: ix.uuid_of_source.get(&r.source_id)?.clone(),
                k: r.kind,
                i: r.item_key,
                ms: r.position_ms,
                dur: r.duration_ms,
                u: r.updated_at,
            })
        })
        .collect();
    let prefs = match store.kv_get(PREFS_KV).await? {
        Some((v, _)) => serde_json::from_str(&v).unwrap_or_default(),
        None => Map::new(),
    };
    Ok(StateDoc {
        v: 1,
        favourites,
        resume,
        recents,
        prefs,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyCounts {
    pub favourites: usize,
    pub recents: usize,
    pub resume: usize,
}

/// Write a merged document into local tables (LWW against what is already there). Items for
/// sources/channels not present locally are skipped here (they stay in the document).
pub async fn apply_doc(store: &Store, doc: &StateDoc, now: i64) -> Result<ApplyCounts> {
    let ix = index(store).await?;
    let mut n = ApplyCounts::default();
    for f in &doc.favourites {
        if let Some(&id) = ix.by_key.get(&(f.s.clone(), f.c.clone()))
            && store.apply_favourite(id, f.p, f.u, f.d).await?
        {
            n.favourites += 1;
        }
    }
    for r in &doc.recents {
        if let Some(&id) = ix.by_key.get(&(r.s.clone(), r.c.clone()))
            && store.apply_recent(id, r.u).await?
        {
            n.recents += 1;
        }
    }
    for r in &doc.resume {
        if let Some(&sid) = ix.source_of_uuid.get(&r.s)
            && store
                .apply_resume(&ResumeRow {
                    source_id: sid,
                    kind: r.k.clone(),
                    item_key: r.i.clone(),
                    position_ms: r.ms,
                    duration_ms: r.dur,
                    updated_at: r.u,
                })
                .await?
        {
            n.resume += 1;
        }
    }
    if !doc.prefs.is_empty() {
        let local = store.kv_get(PREFS_KV).await?;
        let local_u = local
            .and_then(|(v, _)| serde_json::from_str::<Map<String, Value>>(&v).ok())
            .map_or(0, |m| prefs_u(&m));
        if prefs_u(&doc.prefs) > local_u {
            store
                .kv_set(PREFS_KV, &serde_json::to_string(&doc.prefs)?, now)
                .await?;
        }
    }
    store
        .prune_favourite_tombstones(now - TOMBSTONE_SECS)
        .await?;
    store.trim_recents(MAX_RECENTS as i64).await?;
    store.trim_resume(MAX_RESUME as i64).await?;
    Ok(n)
}

/// Remote app-data endpoint (Dispatcharr implements it; tests use memory).
#[async_trait]
pub trait AppDataRemote: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<Value>>;
    async fn put(&self, key: &str, value: &Value) -> Result<()>;
}

#[async_trait]
impl AppDataRemote for crate::source::dispatcharr::DispatcharrClient {
    async fn get(&self, key: &str) -> Result<Option<Value>> {
        self.app_data_get(key).await
    }
    async fn put(&self, key: &str, value: &Value) -> Result<()> {
        self.app_data_put(key, value).await
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateSyncReport {
    pub applied: ApplyCounts,
    pub pushed: bool,
}

/// Pull, merge, apply locally, push back when the merged document differs from the remote.
/// Best-effort: callers retry with backoff on error; local state is never lost.
pub async fn sync_state(
    store: &Store,
    remote: &dyn AppDataRemote,
    now: i64,
) -> Result<StateSyncReport> {
    let remote_doc: StateDoc = match remote.get(APP_DATA_KEY).await? {
        // An unreadable remote document is replaced rather than blocking sync forever.
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => StateDoc::default(),
    };
    let local = local_doc(store).await?;
    let mut merged = merge(&local, &remote_doc, now);
    fit_to_limit(&mut merged)?;
    let applied = apply_doc(store, &merged, now).await?;
    let pushed = merged != remote_doc;
    if pushed {
        remote
            .put(APP_DATA_KEY, &serde_json::to_value(&merged)?)
            .await?;
    }
    Ok(StateSyncReport { applied, pushed })
}

#[cfg(test)]
mod tests;
