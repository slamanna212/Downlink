//! RSS / Atom parsing (via `feed-rs`) and item normalisation: stable guids, plain-text titles and
//! summaries, UTC seconds.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use super::RETENTION_SECS;
use crate::error::{AppError, Result};

pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_SUMMARY_CHARS: usize = 400;
const MAX_LINK_LEN: usize = 2048;
const MAX_GUID_LEN: usize = 512;
/// Items parsed from one document (the newest are kept).
const MAX_PARSED_ITEMS: usize = 500;
/// Dates further ahead than this are clamped to "now" (bad clocks / typos).
const FUTURE_SLACK_SECS: i64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFeed {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub items: Vec<ParsedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedItem {
    pub guid: String,
    pub title: String,
    pub link: Option<String>,
    pub summary: Option<String>,
    pub published_utc: Option<i64>,
}

/// Parse on a blocking thread (a 5 MiB document is real work).
pub async fn parse_blocking(body: Vec<u8>, now: i64) -> Result<ParsedFeed> {
    tokio::task::spawn_blocking(move || parse(&body, now))
        .await
        .map_err(|_| AppError::local("Reading the feed was interrupted."))?
}

/// Parse an RSS 0.9x/1.0/2.0 or Atom document. Items older than the retention window are dropped.
pub fn parse(body: &[u8], now: i64) -> Result<ParsedFeed> {
    // An empty id from the generator means "none": our own fallback below is stable across fetches
    // (feed-rs's default can be a random UUID).
    let parser = feed_rs::parser::Builder::new()
        .id_generator(|_, _, _| String::new())
        .build();
    let feed = parser
        .parse(body)
        .map_err(|_| AppError::server("That address did not return an RSS or Atom feed."))?;
    let title = feed
        .title
        .as_ref()
        .map(|t| clean_text(&t.content, MAX_TITLE_CHARS))
        .filter(|s| !s.is_empty());
    let site_url = pick_link(&feed.links);
    let cutoff = now - RETENTION_SECS;
    let mut items: Vec<ParsedItem> = feed
        .entries
        .iter()
        .filter_map(|e| item(e, now))
        .filter(|i| i.published_utc.is_none_or(|t| t >= cutoff))
        .collect();
    // Duplicate guids inside one document: keep the first.
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.guid.clone()));
    items.sort_by_key(|i| std::cmp::Reverse(i.published_utc));
    items.truncate(MAX_PARSED_ITEMS);
    Ok(ParsedFeed {
        title,
        site_url,
        items,
    })
}

fn item(e: &feed_rs::model::Entry, now: i64) -> Option<ParsedItem> {
    let summary_raw = e
        .summary
        .as_ref()
        .map(|t| t.content.as_str())
        .or_else(|| e.content.as_ref().and_then(|c| c.body.as_deref()))
        .unwrap_or_default();
    let summary = Some(clean_text(summary_raw, MAX_SUMMARY_CHARS)).filter(|s| !s.is_empty());
    let title = e
        .title
        .as_ref()
        .map(|t| clean_text(&t.content, MAX_TITLE_CHARS))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            summary
                .as_deref()
                .map(|s| truncate_chars(s, 120))
                .filter(|s| !s.is_empty())
        })?;
    let link = pick_link(&e.links);
    let published_utc = e
        .published
        .as_ref()
        .or(e.updated.as_ref())
        .map(|d| d.timestamp().as_second())
        .map(|t| if t > now + FUTURE_SLACK_SECS { now } else { t });
    let guid = Some(e.id.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| link.clone())
        .unwrap_or_else(|| {
            hash(&format!(
                "{title}\u{0}{}",
                published_utc.map(|t| t.to_string()).unwrap_or_default()
            ))
        });
    let guid = if guid.len() > MAX_GUID_LEN {
        hash(&guid)
    } else {
        guid
    };
    Some(ParsedItem {
        guid,
        title,
        link,
        summary,
        published_utc,
    })
}

/// The first alternate (or unqualified) http(s) link.
fn pick_link(links: &[feed_rs::model::Link]) -> Option<String> {
    links
        .iter()
        .filter(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
        .chain(links.iter())
        .map(|l| l.href.trim())
        .find(|h| {
            h.len() <= MAX_LINK_LEN
                && url::Url::parse(h).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
        })
        .map(str::to_owned)
}

fn hash(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    let mut out = String::from("h:");
    for b in &d[..16] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

static DROP_BLOCKS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:script|style|head|template)\b[^>]*>.*?</(?:script|style|head|template)\s*>|<!--.*?-->").ok()
});
static TAG: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?s)</?[a-zA-Z!][^>]*>").ok());
static ENTITY: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[a-zA-Z]{2,8});").ok());

/// HTML (or plain text) to one line of plain text, at most `max` characters (with an ellipsis).
pub fn clean_text(input: &str, max: usize) -> String {
    let mut s = input.to_owned();
    if let Some(re) = DROP_BLOCKS.as_ref() {
        s = re.replace_all(&s, " ").into_owned();
    }
    if let Some(re) = TAG.as_ref() {
        s = re.replace_all(&s, " ").into_owned();
    }
    if let Some(re) = ENTITY.as_ref() {
        s = re
            .replace_all(&s, |c: &regex::Captures| {
                decode_entity(&c[1]).unwrap_or_else(|| c[0].to_owned())
            })
            .into_owned();
    }
    let collapsed = s
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    truncate_chars(&collapsed, max)
}

fn decode_entity(name: &str) -> Option<String> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).map(String::from);
    }
    Some(
        match name {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            "nbsp" => " ",
            "ndash" => "\u{2013}",
            "mdash" => "\u{2014}",
            "hellip" => "\u{2026}",
            "lsquo" => "\u{2018}",
            "rsquo" => "\u{2019}",
            "ldquo" => "\u{201C}",
            "rdquo" => "\u{201D}",
            "copy" => "\u{00A9}",
            "reg" => "\u{00AE}",
            "trade" => "\u{2122}",
            _ => return None,
        }
        .to_owned(),
    )
}

/// At most `max` characters; a cut adds "…" (within the limit) at a word boundary when one is near.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let keep: String = s.chars().take(max.saturating_sub(1)).collect();
    let cut = match keep.rfind(' ') {
        Some(i) if i * 4 >= keep.len() * 3 => &keep[..i],
        _ => keep.as_str(),
    };
    format!("{}\u{2026}", cut.trim_end())
}
