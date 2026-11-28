//! Feed polling, only for feeds a view currently watches.
//!
//! A view calls `watch(key, feed_ids)` (replacing whatever that key watched) and `unwatch(key)` on
//! close. A feed is polled while at least one key watches it: every interval (default 3 min,
//! minimum 1 min), at once when first watched and stale, and with doubling backoff after errors
//! (up to 30 min, or longer if the server sent `Retry-After`). [`Scheduler`] is the pure
//! schedule; [`FeedPoller`] runs it on a task with an injected clock and refresher.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;

use super::{DEFAULT_INTERVAL_SECS, MAX_BACKOFF_SECS, MAX_INTERVAL_SECS, MIN_INTERVAL_SECS};
use crate::error::Result;

/// Feeds fetched at the same time.
const CONCURRENCY: usize = 4;
/// Longest sleep with nothing due (a watch/unwatch wakes the task anyway).
const IDLE_SECS: i64 = 3600;

/// Backoff after `failures` consecutive errors: the interval doubled per failure, capped.
pub fn backoff_secs(interval: i64, failures: u32) -> i64 {
    if failures == 0 {
        return interval;
    }
    let shift = failures.min(16);
    interval
        .saturating_mul(1i64 << shift)
        .min(MAX_BACKOFF_SECS.max(interval))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    next_due: i64,
    failures: u32,
    in_flight: bool,
}

/// The pure schedule: which feeds are watched, when each is next due.
#[derive(Debug)]
pub struct Scheduler {
    interval: i64,
    watches: HashMap<String, HashSet<i64>>,
    slots: HashMap<i64, Slot>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new(DEFAULT_INTERVAL_SECS)
    }
}

impl Scheduler {
    pub fn new(interval: i64) -> Self {
        Self {
            interval: interval.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS),
            watches: HashMap::new(),
            slots: HashMap::new(),
        }
    }

    pub fn interval(&self) -> i64 {
        self.interval
    }

    /// Change the interval (clamped to 1..=60 min). Applies from each feed's next fetch.
    pub fn set_interval(&mut self, secs: i64) {
        self.interval = secs.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
    }

    /// Replace what `key` watches. `feeds` pairs each id with its last fetch time; a feed not
    /// already scheduled is due at once when it was never fetched or is older than the interval.
    pub fn watch(&mut self, key: &str, feeds: &[(i64, Option<i64>)], now: i64) {
        if feeds.is_empty() {
            self.unwatch(key);
            return;
        }
        let interval = self.interval;
        for &(id, last) in feeds {
            self.slots.entry(id).or_insert_with(|| Slot {
                next_due: last.map_or(now, |t| (t + interval).min(now + interval)),
                failures: 0,
                in_flight: false,
            });
        }
        self.watches
            .insert(key.to_owned(), feeds.iter().map(|(id, _)| *id).collect());
        self.gc();
    }

    pub fn unwatch(&mut self, key: &str) {
        self.watches.remove(key);
        self.gc();
    }

    /// Stop polling a feed everywhere (it was removed).
    pub fn forget(&mut self, id: i64) {
        for set in self.watches.values_mut() {
            set.remove(&id);
        }
        self.watches.retain(|_, s| !s.is_empty());
        self.slots.remove(&id);
    }

    fn gc(&mut self) {
        let watched = self.watched();
        self.slots.retain(|id, _| watched.contains(id));
    }

    /// Every feed at least one key watches.
    pub fn watched(&self) -> HashSet<i64> {
        self.watches.values().flatten().copied().collect()
    }

    /// Feeds due now, marked in flight so they are not handed out twice.
    pub fn take_due(&mut self, now: i64) -> Vec<i64> {
        let mut due: Vec<i64> = self
            .slots
            .iter_mut()
            .filter(|(_, s)| !s.in_flight && s.next_due <= now)
            .map(|(id, s)| {
                s.in_flight = true;
                *id
            })
            .collect();
        due.sort_unstable();
        due
    }

    /// Record a fetch result. `Err(retry_after)` backs off.
    pub fn record(&mut self, id: i64, outcome: std::result::Result<(), Option<u64>>, now: i64) {
        let interval = self.interval;
        let Some(s) = self.slots.get_mut(&id) else {
            return; // unwatched meanwhile
        };
        s.in_flight = false;
        match outcome {
            Ok(()) => {
                s.failures = 0;
                s.next_due = now + interval;
            }
            Err(retry_after) => {
                s.failures = s.failures.saturating_add(1);
                let wait = backoff_secs(interval, s.failures)
                    .max(retry_after.map_or(0, |r| i64::try_from(r).unwrap_or(i64::MAX)))
                    .min(24 * 3600);
                s.next_due = now + wait;
            }
        }
    }

    /// When the next not-in-flight feed is due.
    pub fn next_wake(&self) -> Option<i64> {
        self.slots
            .values()
            .filter(|s| !s.in_flight)
            .map(|s| s.next_due)
            .min()
    }

    #[cfg(test)]
    pub(crate) fn failures(&self, id: i64) -> Option<u32> {
        self.slots.get(&id).map(|s| s.failures)
    }

    #[cfg(test)]
    pub(crate) fn next_due(&self, id: i64) -> Option<i64> {
        self.slots.get(&id).map(|s| s.next_due)
    }
}

/// Fetches one feed. `Ok(true)` when items were added or changed.
#[async_trait]
pub trait Refresher: Send + Sync {
    async fn refresh(&self, feed_id: i64) -> Result<bool>;
}

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
pub type OnUpdate = Arc<dyn Fn(Vec<i64>) + Send + Sync>;

struct Inner {
    sched: Mutex<Scheduler>,
    wake: tokio::sync::Notify,
    stopped: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// The polling task and its schedule. Cheap to clone.
#[derive(Clone)]
pub struct FeedPoller {
    inner: Arc<Inner>,
}

impl Default for FeedPoller {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedPoller {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                sched: Mutex::new(Scheduler::default()),
                wake: tokio::sync::Notify::new(),
                stopped: AtomicBool::new(false),
                task: Mutex::new(None),
            }),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Scheduler) -> R) -> Option<R> {
        let r = self.inner.sched.lock().ok().map(|mut s| f(&mut s));
        self.inner.wake.notify_one();
        r
    }

    pub fn watch(&self, key: &str, feeds: &[(i64, Option<i64>)], now: i64) {
        self.with(|s| s.watch(key, feeds, now));
    }

    pub fn unwatch(&self, key: &str) {
        self.with(|s| s.unwatch(key));
    }

    pub fn forget(&self, id: i64) {
        self.with(|s| s.forget(id));
    }

    pub fn set_interval(&self, secs: i64) {
        self.with(|s| s.set_interval(secs));
    }

    /// A manual refresh finished: restart that feed's schedule from now.
    pub fn record(&self, id: i64, outcome: std::result::Result<(), Option<u64>>, now: i64) {
        self.with(|s| s.record(id, outcome, now));
    }

    pub fn watched(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .inner
            .sched
            .lock()
            .map(|s| s.watched().into_iter().collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    pub fn is_running(&self) -> bool {
        self.inner
            .task
            .lock()
            .is_ok_and(|t| t.as_ref().is_some_and(|h| !h.is_finished()))
    }

    /// Start the task (once; later calls are no-ops while it runs). Must be called inside a Tokio
    /// runtime.
    pub fn start(&self, refresher: Arc<dyn Refresher>, clock: Clock, on_update: OnUpdate) {
        let Ok(mut task) = self.inner.task.lock() else {
            return;
        };
        if task.as_ref().is_some_and(|h| !h.is_finished()) {
            return;
        }
        self.inner.stopped.store(false, Ordering::SeqCst);
        let inner = self.inner.clone();
        *task = Some(tokio::spawn(run(inner, refresher, clock, on_update)));
    }

    /// Stop polling (quit). Watches are kept; `start` resumes them.
    pub fn stop(&self) {
        self.inner.stopped.store(true, Ordering::SeqCst);
        self.inner.wake.notify_one();
        if let Ok(mut t) = self.inner.task.lock()
            && let Some(h) = t.take()
        {
            h.abort();
        }
    }
}

async fn run(inner: Arc<Inner>, refresher: Arc<dyn Refresher>, clock: Clock, on_update: OnUpdate) {
    while !inner.stopped.load(Ordering::SeqCst) {
        let now = clock();
        let due = inner
            .sched
            .lock()
            .map(|mut s| s.take_due(now))
            .unwrap_or_default();
        if !due.is_empty() {
            let results: Vec<(i64, Result<bool>)> = futures_util::stream::iter(due)
                .map(|id| {
                    let r = refresher.clone();
                    async move { (id, r.refresh(id).await) }
                })
                .buffer_unordered(CONCURRENCY)
                .collect()
                .await;
            let now = clock();
            let mut changed = Vec::new();
            if let Ok(mut s) = inner.sched.lock() {
                for (id, r) in &results {
                    match r {
                        Ok(c) => {
                            s.record(*id, Ok(()), now);
                            if *c {
                                changed.push(*id);
                            }
                        }
                        Err(e) => {
                            tracing::debug!(feed = id, error = %e.message, "feed refresh failed");
                            s.record(*id, Err(e.retry_after), now);
                        }
                    }
                }
            }
            if !changed.is_empty() {
                changed.sort_unstable();
                on_update(changed);
            }
            continue;
        }
        let wait = inner
            .sched
            .lock()
            .ok()
            .and_then(|s| s.next_wake())
            .map_or(IDLE_SECS, |t| (t - now).clamp(1, IDLE_SECS));
        tokio::select! {
            () = inner.wake.notified() => {}
            () = tokio::time::sleep(Duration::from_secs(wait.unsigned_abs())) => {}
        }
    }
}
