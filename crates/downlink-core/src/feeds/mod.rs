//! News feeds for the command center rail (docs/COMMAND_CENTER.md §6.2): user-added RSS/Atom
//! feeds, fetched with conditional GETs, parsed with `feed-rs`, kept for 48 h, and polled only
//! while a view watches them.
//!
//! Secrets (rule 7): a feed URL with userinfo or a credential query parameter is stored in the
//! OS keychain; SQLite keeps only its masked display form. Errors are URL-free and redacted.

pub mod fetch;
pub mod parse;
pub mod poller;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::{SecretStore, SecretString};
use crate::error::{AppError, Result};
use crate::redact::{self, MASK, RedactionSet};
use crate::store::Store;

pub use fetch::{Fetched, Validators};
pub use parse::{ParsedFeed, ParsedItem};
pub use poller::{FeedPoller, Refresher, Scheduler};

/// Longest feed URL accepted.
pub const MAX_URL_LEN: usize = 2048;
/// Default and minimum poll interval.
pub const DEFAULT_INTERVAL_SECS: i64 = 3 * 60;
pub const MIN_INTERVAL_SECS: i64 = 60;
pub const MAX_INTERVAL_SECS: i64 = 60 * 60;
/// Error backoff doubles from the interval up to this.
pub const MAX_BACKOFF_SECS: i64 = 30 * 60;
/// Items older than this are dropped; at most `MAX_ITEMS_PER_FEED` are kept per feed.
pub const RETENTION_SECS: i64 = 48 * 3600;
pub const MAX_ITEMS_PER_FEED: i64 = 200;

/// A validated feed URL: the full URL (which may hold a credential) and what may be stored and
/// shown in its place.
#[derive(Clone)]
pub struct FeedUrl {
    full: String,
    display: String,
    secret: bool,
}

impl std::fmt::Debug for FeedUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FeedUrl({})", self.display)
    }
}

impl FeedUrl {
    /// Trim and validate user input: http/https, a host, bounded length, no fragment.
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(AppError::local("Enter a feed address."));
        }
        if trimmed.len() > MAX_URL_LEN {
            return Err(AppError::local("That feed address is too long."));
        }
        let mut url = url::Url::parse(trimmed)
            .map_err(|_| AppError::local("That feed address is not valid."))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
            return Err(AppError::local(
                "Feed addresses must start with http:// or https://.",
            ));
        }
        url.set_fragment(None);
        let full = url.to_string();
        let display = display_url(&url);
        Ok(Self {
            secret: display != full,
            full,
            display,
        })
    }

    pub fn full(&self) -> &str {
        &self.full
    }

    /// What SQLite stores and the UI shows.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// True when the URL carries a credential and must live in the keychain.
    pub fn is_secret(&self) -> bool {
        self.secret
    }

    /// Literal secrets to scrub from any message about this feed.
    pub fn secrets(&self) -> RedactionSet {
        let mut set = RedactionSet::new();
        if self.secret {
            set.add(self.full.clone());
            if let Ok(u) = url::Url::parse(&self.full) {
                set.add(u.username().to_owned());
                if let Some(p) = u.password() {
                    set.add(p.to_owned());
                }
                for (k, v) in u.query_pairs() {
                    if is_credential_param(&k) {
                        set.add(v.into_owned());
                    }
                }
            }
        }
        set
    }
}

/// A query parameter name that carries a credential (the same list `redact` masks).
fn is_credential_param(name: &str) -> bool {
    redact::QUERY_CRED.is_match(&format!("?{name}="))
}

/// The URL without userinfo and with credential query values masked.
fn display_url(url: &url::Url) -> String {
    let mut u = url.clone();
    let _ = u.set_username("");
    let _ = u.set_password(None);
    if u.query().is_some() {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| {
                let v = if is_credential_param(&k) {
                    MASK.to_owned()
                } else {
                    v.into_owned()
                };
                (k.into_owned(), v)
            })
            .collect();
        if pairs.iter().any(|(_, v)| v == MASK) {
            u.query_pairs_mut().clear().extend_pairs(pairs);
        }
    }
    u.to_string()
}

/// Keychain reference for a credential-bearing feed URL.
pub fn secret_ref(uuid: &str) -> String {
    format!("downlink/feed/{uuid}/url")
}

/// Map a feed-specific failure for storage and display: never the URL, always redacted.
pub fn error_text(e: &AppError, url: &FeedUrl) -> String {
    url.secrets().redact(&e.message)
}

/// What a refresh did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshOutcome {
    /// Items inserted or changed.
    pub changed: usize,
    pub not_modified: bool,
}

/// Fetch, parse and store feeds. Holds no Tauri or Service state.
pub struct FeedEngine {
    store: Store,
    secrets: Arc<dyn SecretStore>,
    client: reqwest::Client,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl FeedEngine {
    pub fn new(store: Store, secrets: Arc<dyn SecretStore>) -> Result<Self> {
        Ok(Self {
            store,
            secrets,
            client: fetch::client()?,
            now: Arc::new(crate::sync::now_utc),
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The full URL of a stored feed (from the keychain when it carries a credential).
    pub async fn resolve_url(&self, row: &store::FeedRow) -> Result<FeedUrl> {
        let raw = match row.secret_ref.clone() {
            Some(r) => {
                let secrets = self.secrets.clone();
                tokio::task::spawn_blocking(move || secrets.get(&r))
                    .await
                    .map_err(|_| AppError::local("The system keychain is unavailable."))??
                    .map(|s| s.expose().to_owned())
                    .ok_or_else(|| {
                        AppError::local(
                            "The saved address of this feed was not found. Add it again.",
                        )
                    })?
            }
            None => row.url.clone(),
        };
        FeedUrl::parse(&raw)
    }

    /// Fetch and parse a URL that is not stored yet (the first fetch of `feed_add`).
    pub async fn probe(&self, url: &FeedUrl) -> Result<(ParsedFeed, Validators)> {
        let now = (self.now)();
        match fetch::fetch(&self.client, url, &Validators::default()).await {
            Ok(Fetched::Body { body, validators }) => {
                Ok((parse::parse_blocking(body, now).await?, validators))
            }
            Ok(Fetched::NotModified) => Err(AppError::server("The feed sent no content.")),
            Err(e) => Err(AppError::with_secrets(e.code, &e.message, &url.secrets())
                .with_retry_after(e.retry_after)),
        }
    }

    /// Store a new feed with its first items. Credential-bearing URLs go to the keychain first.
    pub async fn insert(
        &self,
        url: &FeedUrl,
        parsed: &ParsedFeed,
        validators: &Validators,
    ) -> Result<i64> {
        let now = (self.now)();
        if self.store.feed_by_url(url.display()).await?.is_some() {
            return Err(AppError::local("That feed is already added."));
        }
        let secret_ref = if url.is_secret() {
            let r = secret_ref(&uuid::Uuid::new_v4().to_string());
            let secrets = self.secrets.clone();
            let (r2, value) = (r.clone(), SecretString::new(url.full()));
            tokio::task::spawn_blocking(move || secrets.set(&r2, &value))
                .await
                .map_err(|_| AppError::local("The system keychain is unavailable."))??;
            Some(r)
        } else {
            None
        };
        let inserted = self
            .store
            .insert_feed(&store::NewFeed {
                url: url.display(),
                secret_ref: secret_ref.as_deref(),
                title: parsed.title.as_deref(),
                site_url: parsed.site_url.as_deref(),
                validators,
                now,
            })
            .await;
        let id = match inserted {
            Ok(id) => id,
            Err(e) => {
                if let Some(r) = secret_ref {
                    let _ = self.secrets.delete(&r);
                }
                return Err(e);
            }
        };
        self.store.upsert_feed_items(id, &parsed.items, now).await?;
        Ok(id)
    }

    /// Remove a feed, its items and its keychain entry.
    pub async fn remove(&self, id: i64) -> Result<()> {
        if let Some(r) = self.store.delete_feed(id).await? {
            let secrets = self.secrets.clone();
            tokio::task::spawn_blocking(move || secrets.delete(&r))
                .await
                .map_err(|_| AppError::local("The system keychain is unavailable."))??;
        }
        Ok(())
    }

    /// Conditional GET, parse, upsert, prune. Failures are recorded on the row (redacted) and
    /// returned.
    pub async fn refresh(&self, id: i64) -> Result<RefreshOutcome> {
        let row = self.store.feed(id).await?;
        let url = match self.resolve_url(&row).await {
            Ok(u) => u,
            Err(e) => {
                self.store
                    .record_feed_error(id, (self.now)(), &e.message)
                    .await?;
                return Err(e);
            }
        };
        let validators = Validators {
            etag: row.etag.clone(),
            last_modified: row.last_modified.clone(),
        };
        let result: Result<(Option<ParsedFeed>, Validators)> = async {
            match fetch::fetch(&self.client, &url, &validators).await? {
                Fetched::NotModified => Ok((None, validators.clone())),
                Fetched::Body { body, validators } => Ok((
                    Some(parse::parse_blocking(body, (self.now)()).await?),
                    validators,
                )),
            }
        }
        .await;
        let now = (self.now)();
        match result {
            Ok((parsed, validators)) => {
                let changed = match &parsed {
                    Some(p) => self.store.upsert_feed_items(id, &p.items, now).await?,
                    None => 0,
                };
                self.store
                    .record_feed_ok(id, now, parsed.as_ref(), &validators)
                    .await?;
                self.store.prune_feed_items(id, now).await?;
                Ok(RefreshOutcome {
                    changed,
                    not_modified: parsed.is_none(),
                })
            }
            Err(e) => {
                let e = AppError::with_secrets(e.code, &e.message, &url.secrets())
                    .with_retry_after(e.retry_after);
                self.store.record_feed_error(id, now, &e.message).await?;
                Err(e)
            }
        }
    }
}

#[async_trait]
impl Refresher for FeedEngine {
    async fn refresh(&self, feed_id: i64) -> Result<bool> {
        FeedEngine::refresh(self, feed_id)
            .await
            .map(|o| o.changed > 0)
    }
}

#[cfg(test)]
mod tests;
