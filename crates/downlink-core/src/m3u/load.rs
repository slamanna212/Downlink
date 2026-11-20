//! Fetching and loading playlists: local file or HTTP(S), downloaded to a temp file (bounded
//! memory), parsed streaming, `type="playlist"` includes merged to depth 2 with cycle detection.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::NamedTempFile;

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

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Fetched>;
}

/// Plain HTTP(S) download. No auth headers: playlist/EPG URLs carry their own credentials, and
/// they are never logged (errors are URL-free).
pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new(insecure_tls: bool) -> Result<Self> {
        Ok(Self {
            client: crate::tls::apply(reqwest::Client::builder(), insecure_tls)?
                .timeout(Duration::from_secs(300))
                .connect_timeout(Duration::from_secs(20))
                .user_agent(concat!("Downlink/", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }
}

#[async_trait]
impl Fetcher for HttpFetcher {
    async fn fetch(&self, url: &str) -> Result<Fetched> {
        let mut resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AppError::from_status(
                status.as_u16(),
                None,
                "that playlist",
            ));
        }
        let mut tmp = NamedTempFile::new()?;
        let mut total: u64 = 0;
        while let Some(chunk) = resp.chunk().await? {
            total += chunk.len() as u64;
            if total > MAX_DOWNLOAD {
                return Err(AppError::server("That file is too large to import."));
            }
            tmp.write_all(&chunk)?;
        }
        tmp.flush()?;
        Ok(Fetched::Temp(tmp))
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
