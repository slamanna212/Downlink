//! One feed GET: conditional headers, a timeout, a bounded number of redirects and a bounded body.
//! Feed URLs are never logged and errors are URL-free (`AppError: From<reqwest::Error>` drops it).

use std::time::Duration;

use reqwest::header::{
    ETAG, HeaderMap, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, RETRY_AFTER,
};
use reqwest::{RequestBuilder, StatusCode};

use super::FeedUrl;
use crate::error::{AppError, ErrorCode, Result};

pub const TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_BODY_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_REDIRECTS: usize = 5;
/// Validators longer than this are ignored (a misbehaving server cannot bloat the row).
const MAX_VALIDATOR_LEN: usize = 512;

/// Conditional-GET validators from the last 200.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug)]
pub enum Fetched {
    NotModified,
    Body {
        body: Vec<u8>,
        validators: Validators,
    },
}

/// The shared feed client: our User-Agent, no Referer, 15 s timeout, at most 5 redirects.
pub fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .referer(false)
        .user_agent(crate::user_agent())
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()?)
}

/// The GET with `If-None-Match` / `If-Modified-Since` when known.
pub fn request(client: &reqwest::Client, url: &FeedUrl, v: &Validators) -> RequestBuilder {
    let mut req = client.get(url.full()).header(
        reqwest::header::ACCEPT,
        "application/rss+xml, application/atom+xml, application/xml;q=0.9, text/xml;q=0.9, */*;q=0.5",
    );
    if let Some(etag) = v.etag.as_deref().filter(|s| !s.is_empty()) {
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = v.last_modified.as_deref().filter(|s| !s.is_empty()) {
        req = req.header(IF_MODIFIED_SINCE, lm);
    }
    req
}

fn header(h: &HeaderMap, name: reqwest::header::HeaderName) -> Option<String> {
    h.get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= MAX_VALIDATOR_LEN)
        .map(str::to_owned)
}

/// Seconds from a numeric `Retry-After` (dates are ignored).
fn retry_after(h: &HeaderMap) -> Option<u64> {
    h.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()
}

/// What a status means for a feed. 401/403 are not blocking app-level auth failures here.
pub fn status_error(status: StatusCode, retry: Option<u64>) -> AppError {
    match status.as_u16() {
        401 | 403 => AppError::new(
            ErrorCode::Server,
            "The feed refused access. Check that the address is right.",
        ),
        404 | 410 => AppError::server("The feed was not found at that address."),
        s => AppError::from_status(s, retry, "that feed"),
    }
    .with_retry_after(retry)
}

/// Classify the response head: 304, an error status, or a body to read (with new validators).
/// A 200 without validators clears the old ones (the server stopped sending them).
pub fn classify(status: StatusCode, headers: &HeaderMap) -> Result<Option<Validators>> {
    if status == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(status_error(status, retry_after(headers)));
    }
    Ok(Some(Validators {
        etag: header(headers, ETAG),
        last_modified: header(headers, LAST_MODIFIED),
    }))
}

pub async fn fetch(client: &reqwest::Client, url: &FeedUrl, v: &Validators) -> Result<Fetched> {
    let response = request(client, url, v).send().await?;
    let Some(validators) = classify(response.status(), response.headers())? else {
        return Ok(Fetched::NotModified);
    };
    let body = crate::http::read_body(response, MAX_BODY_BYTES, AppError::from)
        .await
        .map_err(|e| {
            if crate::http::is_too_large(&e) {
                AppError::server("The feed is too large.")
            } else {
                e
            }
        })?;
    Ok(Fetched::Body { body, validators })
}
