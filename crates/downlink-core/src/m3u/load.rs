//! Fetching and loading playlists: local file or HTTP(S), downloaded to a temp file (bounded
//! memory), parsed streaming, `type="playlist"` includes merged to depth 2 with cycle detection.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

use super::{M3uEntry, ParseStats, Playlist, parse};
use crate::error::{AppError, Result};

/// Hard cap on a downloaded playlist.
pub const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
/// Include depth: the top-level playlist is depth 0.
pub const MAX_INCLUDE_DEPTH: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Url(String),
    File(PathBuf),
}

impl Location {
    /// `file:///path`, a plain absolute path, or an http(s) URL.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some(p) = s.strip_prefix("file://") {
            return Ok(Self::File(PathBuf::from(p)));
        }
        if s.starts_with("http://") || s.starts_with("https://") {
            return Ok(Self::Url(s.to_owned()));
        }
        if s.starts_with('/') || s.chars().nth(1) == Some(':') {
            return Ok(Self::File(PathBuf::from(s)));
        }
        Err(AppError::local(
            "Enter an http(s) address or the path of a local playlist file.",
        ))
    }
}

/// A downloaded (or local) file; the temp file is deleted on drop.
pub enum Fetched {
    Temp(NamedTempFile),
    Local(PathBuf),
}

impl Fetched {
    pub fn path(&self) -> &Path {
        match self {
            Self::Temp(t) => t.path(),
            Self::Local(p) => p,
        }
    }
}

/// HTTP cache validators from a previous download, to ask "has it changed?" next time. Keyed by a
/// hash of the URL, never the URL itself (feed URLs can carry credentials and this is persisted).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Validators {
    pub url_hash: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validators {
    /// Validators with nothing to send: the next request downloads unconditionally.
    pub fn for_url(url: &str) -> Self {
        Self {
            url_hash: url_hash(url),
            ..Default::default()
        }
    }
}

/// Hex SHA-256 of a URL: identifies it without storing it.
pub fn url_hash(url: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The outcome of a conditional download.
pub enum Conditional {
    /// The server confirmed the copy we already have is current (HTTP 304).
    NotModified,
    Fetched(Fetched, Validators),
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Fetched>;

    /// Download unless `prev` (for this same URL) shows the server's copy is unchanged. The
    /// default always downloads.
    async fn fetch_if_changed(&self, url: &str, _prev: Option<&Validators>) -> Result<Conditional> {
        Ok(Conditional::Fetched(
            self.fetch(url).await?,
            Validators::for_url(url),
        ))
    }
}

/// Plain HTTP(S) download. No auth headers: playlist/EPG URLs carry their own credentials, and
/// they are never logged (errors are URL-free).
pub struct HttpFetcher {
    client: reqwest::Client,
    pinned: Option<(crate::tls::Pin, reqwest::Client)>,
}

impl HttpFetcher {
    pub fn new(pin: Option<&crate::tls::Pin>) -> Result<Self> {
        let builder = || {
            reqwest::Client::builder()
                .referer(false)
                .timeout(Duration::from_secs(300))
                .connect_timeout(Duration::from_secs(20))
                .user_agent(crate::user_agent())
        };
        let pinned = match pin {
            Some(pin) => Some((pin.clone(), pin.apply(builder())?.build()?)),
            None => None,
        };
        Ok(Self {
            client: builder().build()?,
            pinned,
        })
    }

    fn get(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        let url = reqwest::Url::parse(url)?;
        let client = match &self.pinned {
            Some((pin, client)) if pin.matches(&url) => client,
            _ => &self.client,
        };
        Ok(client.get(url))
    }
}

/// Stream a successful response body to a temp file, bounded by `MAX_DOWNLOAD`.
async fn download(mut resp: reqwest::Response) -> Result<Fetched> {
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::from_status(
            status.as_u16(),
            None,
            "that playlist",
        ));
    }
    let too_large = || AppError::server("That file is too large to import.");
    if resp.content_length().is_some_and(|len| len > MAX_DOWNLOAD) {
        return Err(too_large());
    }
    let tmp = tokio::task::spawn_blocking(NamedTempFile::new)
        .await
        .map_err(|_| AppError::local("Creating the download file was interrupted."))??;
    let (file, path) = tmp.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut remaining = MAX_DOWNLOAD;
    while let Some(chunk) = resp.chunk().await? {
        let len = chunk.len() as u64;
        if len > remaining {
            return Err(too_large());
        }
        remaining -= len;
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    Ok(Fetched::Temp(NamedTempFile::from_parts(
        file.into_std().await,
        path,
    )))
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn fetch(&self, url: &str) -> Result<Fetched> {
        download(self.get(url)?.send().await?).await
    }

    async fn fetch_if_changed(&self, url: &str, prev: Option<&Validators>) -> Result<Conditional> {
        use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
        let hash = url_hash(url);
        let mut req = self.get(url)?;
        if let Some(p) = prev.filter(|p| p.url_hash == hash) {
            if let Some(e) = &p.etag {
                req = req.header(IF_NONE_MATCH, e);
            }
            if let Some(m) = &p.last_modified {
                req = req.header(IF_MODIFIED_SINCE, m);
            }
        }
        let resp = req.send().await?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Conditional::NotModified);
        }
        let header = |n| {
            resp.headers()
                .get(n)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let validators = Validators {
            url_hash: hash,
            etag: header(ETAG),
            last_modified: header(LAST_MODIFIED),
        };
        Ok(Conditional::Fetched(download(resp).await?, validators))
    }
}

/// Parse a file (gzip/xz sniffed) on the blocking pool.
pub async fn parse_file(path: PathBuf) -> Result<Playlist> {
    tokio::task::spawn_blocking(move || {
        let r = crate::util::open_decompressed(&path)?;
        parse(r)
    })
    .await
    .map_err(|_| AppError::local("Reading the playlist was interrupted."))?
    .map_err(AppError::from)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadStats {
    pub parse: ParseStats,
    pub includes_followed: u32,
    /// Includes skipped: cycles, too deep, or unreachable.
    pub includes_skipped: u32,
}

/// Load a playlist and merge its includes.
pub async fn load(location: &Location, fetcher: &dyn Fetcher) -> Result<(Playlist, LoadStats)> {
    let mut visited = HashSet::new();
    let mut stats = LoadStats::default();
    let pl = load_rec(location, fetcher, 0, &mut visited, &mut stats).await?;
    stats.parse = pl.stats.clone();
    Ok((pl, stats))
}

fn load_rec<'a>(
    location: &'a Location,
    fetcher: &'a dyn Fetcher,
    depth: u32,
    visited: &'a mut HashSet<String>,
    stats: &'a mut LoadStats,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Playlist>> + Send + 'a>> {
    Box::pin(async move {
        let key = match location {
            Location::Url(u) => u.clone(),
            Location::File(p) => format!("file://{}", p.display()),
        };
        visited.insert(key);
        let fetched;
        let path = match location {
            Location::Url(u) => {
                fetched = fetcher.fetch(u).await?;
                fetched.path().to_path_buf()
            }
            Location::File(p) => p.clone(),
        };
        let mut pl = parse_file(path).await?;
        let mut merged: Vec<M3uEntry> = Vec::with_capacity(pl.entries.len());
        for e in std::mem::take(&mut pl.entries) {
            if !e.is_include() {
                merged.push(e);
                continue;
            }
            let target = Location::Url(e.url.clone());
            let Location::Url(u) = &target else { continue };
            if !(u.starts_with("http://") || u.starts_with("https://"))
                || depth >= MAX_INCLUDE_DEPTH
                || visited.contains(u)
            {
                stats.includes_skipped += 1;
                continue;
            }
            match load_rec(&target, fetcher, depth + 1, visited, stats).await {
                Ok(sub) => {
                    stats.includes_followed += 1;
                    pl.stats.entries += sub.stats.entries;
                    pl.stats.dropped += sub.stats.dropped;
                    merged.extend(sub.entries);
                }
                Err(_) => stats.includes_skipped += 1, // one bad include never aborts the import
            }
        }
        pl.entries = merged;
        Ok(pl)
    })
}
