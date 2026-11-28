#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Invented fixtures only (rule 16). HTTP tests use a local wiremock server.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fetch::{self, Fetched, Validators};
use super::parse::{self, MAX_SUMMARY_CHARS};
use super::poller::{FeedPoller, Refresher, Scheduler, backoff_secs};
use super::*;
use crate::auth::MemorySecretStore;
use crate::error::ErrorCode;

/// 2026-11-28T12:00:00Z
const NOW: i64 = 1_795_867_200;

const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
<channel>
  <title>Example &amp; Daily</title>
  <link>https://news.example/</link>
  <description>Invented headlines</description>
  <item>
    <title>Harbor bridge reopens</title>
    <link>https://news.example/a/1</link>
    <guid isPermaLink="false">item-1</guid>
    <pubDate>Sat, 28 Nov 2026 10:00:00 GMT</pubDate>
    <description><![CDATA[<p>The <b>bridge</b> reopened&nbsp;today.</p><script>alert(1)</script>]]></description>
  </item>
  <item>
    <title>No guid here</title>
    <link>https://news.example/a/2</link>
    <pubDate>Sat, 28 Nov 2026 09:30:00 +0100</pubDate>
    <description>Plain &lt;i&gt;escaped&lt;/i&gt; html &amp;amp; more</description>
  </item>
  <item>
    <title>Neither guid nor link</title>
    <pubDate>Fri, 27 Nov 2026 23:00:00 EST</pubDate>
  </item>
  <item>
    <title>Odd date</title>
    <guid>item-odd</guid>
    <pubDate>not a date at all</pubDate>
  </item>
  <item>
    <title>Too old</title>
    <guid>item-old</guid>
    <pubDate>Mon, 02 Nov 2026 10:00:00 GMT</pubDate>
  </item>
  <item>
    <title>From the future</title>
    <guid>item-future</guid>
    <pubDate>Fri, 01 Jan 2027 00:00:00 GMT</pubDate>
  </item>
</channel>
</rss>"#;

const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="html">Sample &lt;em&gt;Wire&lt;/em&gt;</title>
  <link rel="self" href="https://wire.example/feed.atom"/>
  <link rel="alternate" href="https://wire.example/"/>
  <id>urn:uuid:00000000-0000-0000-0000-000000000001</id>
  <updated>2026-11-28T11:00:00Z</updated>
  <entry>
    <title>Storm passes coast</title>
    <link rel="alternate" href="https://wire.example/s/1"/>
    <id>urn:wire:1</id>
    <updated>2026-11-28T11:00:00+02:00</updated>
    <summary type="html">&lt;div&gt;Winds   eased
      overnight.&lt;/div&gt;</summary>
  </entry>
  <entry>
    <title type="xhtml"><div xmlns="http://www.w3.org/1999/xhtml">Markets <b>rise</b></div></title>
    <link href="https://wire.example/s/2"/>
    <id>urn:wire:2</id>
    <published>2026-11-28T08:15:30.123Z</published>
    <updated>2026-11-28T09:00:00Z</updated>
    <content type="html">&lt;p&gt;Long content&lt;/p&gt;</content>
  </entry>
</feed>"#;

fn by_title<'a>(f: &'a ParsedFeed, t: &str) -> &'a ParsedItem {
    f.items.iter().find(|i| i.title == t).unwrap()
}

// ---- parsing ---------------------------------------------------------------------------------

#[test]
fn rss_items_are_normalised() {
    let f = parse::parse(RSS.as_bytes(), NOW).unwrap();
    assert_eq!(f.title.as_deref(), Some("Example & Daily"));
    assert_eq!(f.site_url.as_deref(), Some("https://news.example/"));

    let a = by_title(&f, "Harbor bridge reopens");
    assert_eq!(a.guid, "item-1");
    assert_eq!(a.link.as_deref(), Some("https://news.example/a/1"));
    assert_eq!(a.summary.as_deref(), Some("The bridge reopened today."));
    assert_eq!(a.published_utc, Some(NOW - 2 * 3600));

    // Missing guid: the link stands in.
    let b = by_title(&f, "No guid here");
    assert_eq!(b.guid, "https://news.example/a/2");
    assert_eq!(b.published_utc, Some(NOW - 3 * 3600 - 30 * 60));
    assert_eq!(b.summary.as_deref(), Some("Plain escaped html & more"));

    // Neither: a stable hash of title + date.
    let c = by_title(&f, "Neither guid nor link");
    assert!(c.guid.starts_with("h:"));
    let again = parse::parse(RSS.as_bytes(), NOW).unwrap();
    assert_eq!(by_title(&again, "Neither guid nor link").guid, c.guid);

    // Unparseable date: kept, undated.
    assert_eq!(by_title(&f, "Odd date").published_utc, None);
    // Older than 48 h: dropped. Future: clamped to now.
    assert!(f.items.iter().all(|i| i.title != "Too old"));
    assert_eq!(by_title(&f, "From the future").published_utc, Some(NOW));
}

#[test]
fn atom_items_are_normalised() {
    let f = parse::parse(ATOM.as_bytes(), NOW).unwrap();
    assert_eq!(f.title.as_deref(), Some("Sample Wire"));
    assert_eq!(f.site_url.as_deref(), Some("https://wire.example/"));
    let a = by_title(&f, "Storm passes coast");
    assert_eq!(a.guid, "urn:wire:1");
    assert_eq!(a.published_utc, Some(NOW - 3 * 3600));
    assert_eq!(a.summary.as_deref(), Some("Winds eased overnight."));
    let b = by_title(&f, "Markets rise");
    assert_eq!(b.link.as_deref(), Some("https://wire.example/s/2"));
    assert_eq!(b.published_utc, Some(NOW - 3 * 3600 - 45 * 60 + 30));
    assert_eq!(b.summary.as_deref(), Some("Long content"));
    // Newest first.
    assert_eq!(f.items[0].guid, "urn:wire:1");
}

#[test]
fn non_feeds_are_rejected() {
    let e = parse::parse(b"<html><body>hello</body></html>", NOW).unwrap_err();
    assert_eq!(e.code, ErrorCode::Server);
    assert!(parse::parse(b"", NOW).is_err());
    assert!(parse::parse(b"\x00\x01garbage", NOW).is_err());
}

#[test]
fn text_is_cleaned_and_truncated() {
    assert_eq!(parse::clean_text("  a\n\tb\u{0}c  ", 50), "a b c");
    assert_eq!(
        parse::clean_text("<!-- x --><style>p{}</style>Hi &#233;&#x41;", 50),
        "Hi éA"
    );
    assert_eq!(parse::clean_text("&bogus; &amp;", 50), "&bogus; &");
    let long = "word ".repeat(200);
    let s = parse::clean_text(&long, MAX_SUMMARY_CHARS);
    assert!(s.chars().count() <= MAX_SUMMARY_CHARS);
    assert!(s.ends_with('\u{2026}'));
    assert!(s.trim_end_matches('\u{2026}').ends_with("word"));
    // Multi-byte characters never split.
    let s = parse::truncate_chars(&"é".repeat(10), 5);
    assert_eq!(s.chars().count(), 5);
}

// ---- URLs and secrets ------------------------------------------------------------------------

#[test]
fn urls_are_validated() {
    assert!(FeedUrl::parse("").is_err());
    assert!(FeedUrl::parse("   ").is_err());
    assert!(FeedUrl::parse("ftp://news.example/rss").is_err());
    assert!(FeedUrl::parse("file:///etc/passwd").is_err());
    assert!(FeedUrl::parse("javascript:alert(1)").is_err());
    assert!(FeedUrl::parse("news.example/rss").is_err());
    assert!(FeedUrl::parse(&format!("https://news.example/{}", "a".repeat(MAX_URL_LEN))).is_err());
    let u = FeedUrl::parse("  https://news.example/rss.xml#top ").unwrap();
    assert_eq!(u.full(), "https://news.example/rss.xml");
    assert_eq!(u.display(), u.full());
    assert!(!u.is_secret());
    // Ordinary query parameters are not secrets.
    let u = FeedUrl::parse("https://news.example/rss?section=world&lang=en").unwrap();
    assert!(!u.is_secret());
}

#[test]
fn credential_urls_are_masked_for_storage() {
    let u = FeedUrl::parse("https://news.example/rss?section=world&token=s3cr3tvalue").unwrap();
    assert!(u.is_secret());
    assert!(!u.display().contains("s3cr3tvalue"));
    assert!(u.display().contains("section=world"));
    assert!(u.display().contains("token="));

    let u = FeedUrl::parse("https://reader:hunter22@news.example/private.xml").unwrap();
    assert!(u.is_secret());
    assert_eq!(u.display(), "https://news.example/private.xml");

    for q in ["key", "apikey", "api_key", "access_token", "auth", "sig"] {
        let u = FeedUrl::parse(&format!("https://news.example/f?{q}=abcd1234xyz")).unwrap();
        assert!(u.is_secret(), "{q}");
        assert!(!u.display().contains("abcd1234xyz"), "{q}");
    }
}

#[test]
fn error_text_never_carries_the_url_secret() {
    let u = FeedUrl::parse("https://reader:hunter22@news.example/f?apikey=zzzz9999").unwrap();
    let e = AppError::network(format!("failed {} (hunter22, zzzz9999)", u.full()));
    let t = error_text(&e, &u);
    assert!(!t.contains("hunter22") && !t.contains("zzzz9999"), "{t}");
}

// ---- store -----------------------------------------------------------------------------------

fn item(guid: &str, title: &str, published: Option<i64>) -> ParsedItem {
    ParsedItem {
        guid: guid.into(),
        title: title.into(),
        link: None,
        summary: None,
        published_utc: published,
    }
}

async fn feed_row(store: &Store) -> i64 {
    store
        .insert_feed(&store::NewFeed {
            url: "https://news.example/rss",
            secret_ref: None,
            title: Some("Example"),
            site_url: None,
            validators: &Validators::default(),
            now: NOW,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn upsert_counts_new_and_changed_items() {
    let store = Store::open_memory().await.unwrap();
    let id = feed_row(&store).await;
    let items = vec![item("a", "A", Some(NOW - 60)), item("b", "B", None)];
    assert_eq!(store.upsert_feed_items(id, &items, NOW).await.unwrap(), 2);
    assert_eq!(
        store.upsert_feed_items(id, &items, NOW + 60).await.unwrap(),
        0
    );
    let edited = vec![
        item("a", "A (updated)", Some(NOW - 60)),
        item("b", "B", None),
    ];
    assert_eq!(
        store
            .upsert_feed_items(id, &edited, NOW + 120)
            .await
            .unwrap(),
        1
    );

    let rows = store.feed_items(&[id], 50).await.unwrap();
    assert_eq!(rows.len(), 2);
    // b has no date: sorted by first seen (NOW) which is newer than a's NOW - 60.
    assert_eq!(rows[0].guid, "b");
    assert_eq!(rows[1].title, "A (updated)");
    assert_eq!(rows[1].fetched_utc, NOW, "first-seen time is kept");
    assert_eq!(store.feed(id).await.unwrap().item_count, 2);

    assert!(
        store
            .insert_feed(&store::NewFeed {
                url: "https://news.example/rss",
                secret_ref: None,
                title: None,
                site_url: None,
                validators: &Validators::default(),
                now: NOW,
            })
            .await
            .is_err(),
        "duplicate URL"
    );
}

#[tokio::test]
async fn retention_keeps_48h_and_200_items() {
    let store = Store::open_memory().await.unwrap();
    let id = feed_row(&store).await;
    let mut items: Vec<ParsedItem> = (0..250)
        .map(|n| item(&format!("g{n}"), &format!("T{n}"), Some(NOW - n * 60)))
        .collect();
    items.push(item("stale", "Stale", Some(NOW - RETENTION_SECS - 1)));
    store.upsert_feed_items(id, &items, NOW).await.unwrap();
    store.prune_feed_items(id, NOW).await.unwrap();
    let rows = store.feed_items(&[id], 500).await.unwrap();
    assert_eq!(rows.len() as i64, MAX_ITEMS_PER_FEED);
    assert_eq!(rows[0].guid, "g0");
    assert!(rows.iter().all(|r| r.guid != "stale" && r.guid != "g249"));

    // Undated items age out by first-seen time.
    store
        .upsert_feed_items(id, &[item("undated", "U", None)], NOW - RETENTION_SECS - 5)
        .await
        .unwrap();
    store.prune_feed_items(id, NOW).await.unwrap();
    assert!(
        store
            .feed_items(&[id], 500)
            .await
            .unwrap()
            .iter()
            .all(|r| r.guid != "undated")
    );

    // Delete cascades.
    assert_eq!(store.delete_feed(id).await.unwrap(), None);
    assert!(store.feed_items(&[id], 500).await.unwrap().is_empty());
}

#[tokio::test]
async fn errors_are_stored_redacted() {
    let store = Store::open_memory().await.unwrap();
    let id = feed_row(&store).await;
    store
        .record_feed_error(id, NOW, "boom https://u:pw1234@news.example/f?token=abc123")
        .await
        .unwrap();
    let row = store.feed(id).await.unwrap();
    let err = row.last_error.unwrap();
    assert!(!err.contains("pw1234") && !err.contains("abc123"), "{err}");
    assert_eq!(row.last_fetch_utc, Some(NOW));
}

// ---- conditional GET -------------------------------------------------------------------------

#[test]
fn request_sends_validators() {
    let client = fetch::client().unwrap();
    let url = FeedUrl::parse("https://news.example/rss").unwrap();
    let req = fetch::request(&client, &url, &Validators::default())
        .build()
        .unwrap();
    assert!(req.headers().get("if-none-match").is_none());
    assert!(req.headers().get("if-modified-since").is_none());
    let v = Validators {
        etag: Some("\"v1\"".into()),
        last_modified: Some("Sat, 28 Nov 2026 10:00:00 GMT".into()),
    };
    let req = fetch::request(&client, &url, &v).build().unwrap();
    assert_eq!(req.headers()["if-none-match"], "\"v1\"");
    assert_eq!(
        req.headers()["if-modified-since"],
        "Sat, 28 Nov 2026 10:00:00 GMT"
    );
}

#[test]
fn responses_are_classified() {
    let mut h = HeaderMap::new();
    assert_eq!(fetch::classify(StatusCode::NOT_MODIFIED, &h).unwrap(), None);
    h.insert("etag", HeaderValue::from_static("\"v2\""));
    h.insert(
        "last-modified",
        HeaderValue::from_static("Sat, 28 Nov 2026 11:00:00 GMT"),
    );
    let v = fetch::classify(StatusCode::OK, &h).unwrap().unwrap();
    assert_eq!(v.etag.as_deref(), Some("\"v2\""));
    assert!(v.last_modified.is_some());

    let mut h = HeaderMap::new();
    h.insert("retry-after", HeaderValue::from_static("120"));
    let e = fetch::classify(StatusCode::TOO_MANY_REQUESTS, &h).unwrap_err();
    assert_eq!(e.retry_after, Some(120));
    let e = fetch::classify(StatusCode::UNAUTHORIZED, &HeaderMap::new()).unwrap_err();
    assert!(!e.blocking, "a feed 401 never blocks the app");
    assert_eq!(e.code, ErrorCode::Server);
}

fn engine(store: Store) -> FeedEngine {
    FeedEngine::new(store, Arc::new(MemorySecretStore::new())).unwrap()
}

#[tokio::test]
async fn refresh_uses_conditional_get_and_honours_304() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rss"))
        .and(header("if-none-match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rss"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_string(RSS),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let store = Store::open_memory().await.unwrap();
    let eng = engine(store.clone());
    let url = FeedUrl::parse(&format!("{}/rss", server.uri())).unwrap();
    let (parsed, v) = eng.probe(&url).await.unwrap();
    assert_eq!(v.etag.as_deref(), Some("\"v1\""));
    let id = eng.insert(&url, &parsed, &v).await.unwrap();
    let row = store.feed(id).await.unwrap();
    assert_eq!(row.title.as_deref(), Some("Example & Daily"));
    assert!(row.item_count >= 4);

    let out = eng.refresh(id).await.unwrap();
    assert!(out.not_modified);
    assert_eq!(out.changed, 0);
    let row = store.feed(id).await.unwrap();
    assert!(row.last_error.is_none());
    assert_eq!(row.etag.as_deref(), Some("\"v1\""));
    assert!(matches!(
        fetch::fetch(&fetch::client().unwrap(), &url, &Validators::default())
            .await
            .unwrap(),
        Fetched::Body { .. }
    ));
}

#[tokio::test]
async fn failures_are_recorded_and_secret_urls_use_the_keychain() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ATOM))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let store = Store::open_memory().await.unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let eng = FeedEngine::new(store.clone(), secrets.clone()).unwrap();
    let url = FeedUrl::parse(&format!("{}/f?token=tok3n-value", server.uri())).unwrap();
    let (parsed, v) = eng.probe(&url).await.unwrap();
    let id = eng.insert(&url, &parsed, &v).await.unwrap();

    let row = store.feed(id).await.unwrap();
    assert!(
        !row.url.contains("tok3n-value"),
        "SQLite holds the masked URL"
    );
    let r = row.secret_ref.clone().unwrap();
    assert_eq!(
        crate::auth::SecretStore::get(secrets.as_ref(), &r)
            .unwrap()
            .unwrap()
            .expose(),
        url.full()
    );
    let e = eng.refresh(id).await.unwrap_err();
    assert_eq!(e.code, ErrorCode::Server);
    let row = store.feed(id).await.unwrap();
    assert!(row.last_error.is_some());
    assert!(!row.last_error.unwrap().contains("tok3n-value"));

    eng.remove(id).await.unwrap();
    assert!(
        crate::auth::SecretStore::get(secrets.as_ref(), &r)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn oversized_bodies_are_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(vec![b' '; fetch::MAX_BODY_BYTES + 1]),
        )
        .mount(&server)
        .await;
    let url = FeedUrl::parse(&format!("{}/big", server.uri())).unwrap();
    let e = fetch::fetch(&fetch::client().unwrap(), &url, &Validators::default())
        .await
        .unwrap_err();
    assert_eq!(e.message, "The feed is too large.");
}

// ---- schedule --------------------------------------------------------------------------------

#[test]
fn backoff_doubles_and_caps() {
    assert_eq!(backoff_secs(180, 0), 180);
    assert_eq!(backoff_secs(180, 1), 360);
    assert_eq!(backoff_secs(180, 2), 720);
    assert_eq!(backoff_secs(180, 3), 1440);
    assert_eq!(backoff_secs(180, 4), MAX_BACKOFF_SECS);
    assert_eq!(backoff_secs(180, 40), MAX_BACKOFF_SECS);
    assert_eq!(
        backoff_secs(3600, 2),
        3600,
        "an interval above the cap is not shortened"
    );
}

#[test]
fn scheduler_polls_watched_feeds_only() {
    let mut s = Scheduler::new(180);
    assert!(s.take_due(NOW).is_empty());
    // 1 never fetched, 2 stale, 3 fresh.
    s.watch(
        "a",
        &[(1, None), (2, Some(NOW - 600)), (3, Some(NOW - 60))],
        NOW,
    );
    assert_eq!(s.take_due(NOW), vec![1, 2]);
    assert!(
        s.take_due(NOW).is_empty(),
        "in flight feeds are not handed out twice"
    );
    assert_eq!(s.next_wake(), Some(NOW + 120));
    s.record(1, Ok(()), NOW);
    s.record(2, Err(None), NOW);
    assert_eq!(s.next_due(1), Some(NOW + 180));
    assert_eq!(s.next_due(2), Some(NOW + 360));
    s.record(2, Err(None), NOW);
    assert_eq!(s.next_due(2), Some(NOW + 720));
    assert_eq!(s.failures(2), Some(2));
    s.record(2, Err(Some(5000)), NOW);
    assert_eq!(
        s.next_due(2),
        Some(NOW + 5000),
        "Retry-After wins when longer"
    );
    s.record(2, Ok(()), NOW);
    assert_eq!(s.failures(2), Some(0));

    // A second view sharing feed 3 keeps its schedule when the first closes.
    s.watch("b", &[(3, Some(NOW - 60))], NOW);
    s.unwatch("a");
    let mut w: Vec<i64> = s.watched().into_iter().collect();
    w.sort_unstable();
    assert_eq!(w, vec![3]);
    assert_eq!(s.next_due(1), None);
    assert_eq!(s.take_due(NOW + 120), vec![3]);

    // Replacing a key's set; an empty set unwatches.
    s.watch("b", &[(4, None)], NOW);
    assert_eq!(s.watched().into_iter().collect::<Vec<_>>(), vec![4]);
    s.watch("b", &[], NOW);
    assert!(s.watched().is_empty());
    assert_eq!(s.next_wake(), None);

    s.watch("c", &[(5, None)], NOW);
    s.forget(5);
    assert!(s.watched().is_empty());
}

#[test]
fn interval_is_clamped() {
    let mut s = Scheduler::new(5);
    assert_eq!(s.interval(), MIN_INTERVAL_SECS);
    s.set_interval(10 * 3600);
    assert_eq!(s.interval(), MAX_INTERVAL_SECS);
}

struct FakeRefresher {
    calls: AtomicUsize,
    fail: bool,
}

#[async_trait::async_trait]
impl Refresher for FakeRefresher {
    async fn refresh(&self, _feed_id: i64) -> crate::Result<bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(AppError::network("down"))
        } else {
            Ok(true)
        }
    }
}

#[tokio::test]
async fn poller_fetches_stale_feeds_on_watch_and_reports_updates() {
    let poller = FeedPoller::new();
    let refresher = Arc::new(FakeRefresher {
        calls: AtomicUsize::new(0),
        fail: false,
    });
    let clock = Arc::new(AtomicI64::new(NOW));
    let c = clock.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    poller.start(
        refresher.clone(),
        Arc::new(move || c.load(Ordering::SeqCst)),
        Arc::new(move |ids| {
            let _ = tx.send(ids);
        }),
    );
    assert!(poller.is_running());
    poller.watch("view", &[(7, None), (8, Some(NOW - 10))], NOW);
    let ids = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ids, vec![7], "the fresh feed waits for its interval");
    assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    assert_eq!(poller.watched(), vec![7, 8]);
    poller.unwatch("view");
    assert!(poller.watched().is_empty());
    poller.stop();
    tokio::task::yield_now().await;
    assert!(!poller.is_running());
}

#[tokio::test]
async fn poller_backs_off_on_errors() {
    let poller = FeedPoller::new();
    let refresher = Arc::new(FakeRefresher {
        calls: AtomicUsize::new(0),
        fail: true,
    });
    poller.start(refresher.clone(), Arc::new(|| NOW), Arc::new(|_| {}));
    poller.watch("v", &[(1, None)], NOW);
    for _ in 0..50 {
        if refresher.calls.load(Ordering::SeqCst) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    // The clock does not move, so the backed-off feed is not retried.
    assert_eq!(refresher.calls.load(Ordering::SeqCst), 1);
    poller.stop();
}
