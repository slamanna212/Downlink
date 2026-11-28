//! News feed verbs (docs/COMMAND_CENTER.md §6.2): add / list / remove / items / refresh, and the
//! watch API that drives polling while a command center with feeds is open.

use std::sync::Arc;

use serde_json::Value;

use super::{Event, FeedDto, FeedItemDto, FeedsUpdated, Service};
use crate::error::{AppError, Result};
use crate::feeds::store::{FeedItemRow, FeedRow, MAX_ITEMS_QUERY};
use crate::feeds::{self, FeedEngine, FeedUrl};
use crate::sync::now_utc;

/// Setting: minutes between polls of a watched feed (default 3, 1..=60).
pub const SET_FEED_REFRESH_MINUTES: &str = "feedRefreshMinutes";
/// Most feeds one view may watch.
const MAX_WATCHED: usize = 50;
const MAX_WATCH_KEY_LEN: usize = 128;

fn feed_dto(r: FeedRow) -> FeedDto {
    FeedDto {
        id: r.id,
        has_secret: r.secret_ref.is_some(),
        url: r.url,
        title: r.title,
        site_url: r.site_url,
        last_fetch_utc: r.last_fetch_utc,
        last_ok_utc: r.last_ok_utc,
        last_error: r.last_error.map(|e| crate::redact::redact(&e)),
        item_count: r.item_count,
    }
}

fn item_dto(r: FeedItemRow) -> FeedItemDto {
    FeedItemDto {
        feed_id: r.feed_id,
        guid: r.guid,
        title: r.title,
        link: r.link,
        summary: r.summary,
        published_utc: r.published_utc.unwrap_or(r.fetched_utc),
    }
}

/// Poll interval from the settings object.
fn interval_secs(settings: &Value) -> i64 {
    settings
        .get(SET_FEED_REFRESH_MINUTES)
        .and_then(Value::as_i64)
        .map_or(feeds::DEFAULT_INTERVAL_SECS, |m| m.saturating_mul(60))
        .clamp(feeds::MIN_INTERVAL_SECS, feeds::MAX_INTERVAL_SECS)
}

impl Service {
    fn feed_engine(&self) -> Result<FeedEngine> {
        FeedEngine::new(self.store.clone(), self.secrets.clone())
    }

    /// Start the feed poller (once, from the app's boot). It idles until a view watches feeds.
    pub fn start_feed_poller(&self) -> Result<()> {
        let engine = Arc::new(self.feed_engine()?);
        let events = self.events.clone();
        self.feeds.start(
            engine,
            Arc::new(now_utc),
            Arc::new(move |feed_ids| {
                let _ = events.send(Event::FeedsUpdated(FeedsUpdated { feed_ids }));
            }),
        );
        Ok(())
    }

    /// Stop polling (quit). In-flight fetches are dropped.
    pub fn stop_feed_poller(&self) {
        self.feeds.stop();
    }

    /// Validate, fetch once, and save. A feed whose first fetch fails is not saved: the redacted
    /// error comes back so the user can fix the address.
    pub async fn feed_add(&self, url: &str) -> Result<FeedDto> {
        let url = FeedUrl::parse(url)?;
        if self.store.feed_by_url(url.display()).await?.is_some() {
            return Err(AppError::local("That feed is already added."));
        }
        let engine = self.feed_engine()?;
        let (parsed, validators) = engine.probe(&url).await?;
        let id = engine.insert(&url, &parsed, &validators).await?;
        self.emit(Event::FeedsUpdated(FeedsUpdated { feed_ids: vec![id] }));
        Ok(feed_dto(self.store.feed(id).await?))
    }

    pub async fn feeds_list(&self) -> Result<Vec<FeedDto>> {
        Ok(self
            .store
            .feeds()
            .await?
            .into_iter()
            .map(feed_dto)
            .collect())
    }

    /// Remove a feed, its items and its keychain entry; it stops being polled.
    pub async fn feed_remove(&self, id: i64) -> Result<()> {
        self.feeds.forget(id);
        self.feed_engine()?.remove(id).await
    }

    /// Items of the given feeds, newest first, at most `limit` (1..=500).
    pub async fn feed_items(&self, feed_ids: &[i64], limit: i64) -> Result<Vec<FeedItemDto>> {
        Ok(self
            .store
            .feed_items(feed_ids, limit.clamp(1, MAX_ITEMS_QUERY))
            .await?
            .into_iter()
            .map(item_dto)
            .collect())
    }

    /// Manual refresh. Emits `feeds://updated` when items changed; the poll schedule restarts.
    pub async fn feed_refresh(&self, id: i64) -> Result<FeedDto> {
        let result = self.feed_engine()?.refresh(id).await;
        let now = now_utc();
        match &result {
            Ok(o) => {
                self.feeds.record(id, Ok(()), now);
                if o.changed > 0 {
                    self.emit(Event::FeedsUpdated(FeedsUpdated { feed_ids: vec![id] }));
                }
            }
            Err(e) => self.feeds.record(id, Err(e.retry_after), now),
        }
        result?;
        Ok(feed_dto(self.store.feed(id).await?))
    }

    /// Replace the feeds `key` watches (a view-chosen id, one per open view). An empty list stops
    /// watching. Feeds never fetched, or older than the interval, are fetched at once.
    pub async fn feeds_watch(&self, key: &str, feed_ids: &[i64]) -> Result<()> {
        let key = key.trim();
        if key.is_empty() || key.len() > MAX_WATCH_KEY_LEN {
            return Err(AppError::local("Invalid feed watch key."));
        }
        let mut ids: Vec<i64> = feed_ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(MAX_WATCHED);
        self.feeds
            .set_interval(interval_secs(&self.get_settings().await?));
        let known = self.store.feed_last_fetch(&ids).await?;
        self.feeds.watch(key, &known, now_utc());
        Ok(())
    }

    pub fn feeds_unwatch(&self, key: &str) {
        self.feeds.unwatch(key.trim());
    }

    /// Feeds currently polled (diagnostics and tests).
    pub fn feeds_watched(&self) -> Vec<i64> {
        self.feeds.watched()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::auth::MemorySecretStore;
    use crate::store::Store;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RSS: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Test wire</title>
        <link>https://news.example/</link>
        <item><title>First</title><guid>g1</guid></item>
        <item><title>Second</title><guid>g2</guid></item></channel></rss>"#;

    async fn service() -> (Arc<Service>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let svc = Service::with_store(
            Store::open_memory().await.unwrap(),
            Arc::new(MemorySecretStore::new()),
            dir.path().join("img"),
            "0.0.0-test".into(),
        );
        (svc, dir)
    }

    #[tokio::test]
    async fn add_list_items_watch_remove() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rss"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let (svc, _dir) = service().await;
        let mut rx = svc.subscribe();

        let bad = svc
            .feed_add(&format!("{}/missing?token=abc12345", server.uri()))
            .await
            .unwrap_err();
        assert!(!bad.message.contains("abc12345"));
        assert!(
            svc.feeds_list().await.unwrap().is_empty(),
            "a failed first fetch saves nothing"
        );

        let feed = svc
            .feed_add(&format!("{}/rss", server.uri()))
            .await
            .unwrap();
        assert_eq!(feed.title.as_deref(), Some("Test wire"));
        assert_eq!(feed.item_count, 2);
        assert!(!feed.has_secret);
        assert!(
            svc.feed_add(&format!("{}/rss", server.uri()))
                .await
                .is_err()
        );
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.name(), "feeds://updated");
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({"feedIds": [feed.id]})
        );

        let items = svc.feed_items(&[feed.id], 10).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.published_utc > 0));

        svc.feeds_watch("view-1", &[feed.id, feed.id, 9999])
            .await
            .unwrap();
        assert_eq!(svc.feeds_watched(), vec![feed.id]);
        assert!(svc.feeds_watch("", &[feed.id]).await.is_err());
        svc.feeds_unwatch("view-1");
        assert!(svc.feeds_watched().is_empty());

        let refreshed = svc.feed_refresh(feed.id).await.unwrap();
        assert!(refreshed.last_error.is_none());

        svc.feeds_watch("view-2", &[feed.id]).await.unwrap();
        svc.feed_remove(feed.id).await.unwrap();
        assert!(svc.feeds_watched().is_empty());
        assert!(svc.feeds_list().await.unwrap().is_empty());
    }

    #[test]
    fn interval_setting_is_clamped() {
        assert_eq!(interval_secs(&serde_json::json!({})), 180);
        assert_eq!(
            interval_secs(&serde_json::json!({"feedRefreshMinutes": 0})),
            60
        );
        assert_eq!(
            interval_secs(&serde_json::json!({"feedRefreshMinutes": 10})),
            600
        );
        assert_eq!(
            interval_secs(&serde_json::json!({"feedRefreshMinutes": 100000})),
            3600
        );
    }
}
