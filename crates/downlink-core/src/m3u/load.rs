//! Fetching and loading playlists: local file or HTTP(S), downloaded to a temp file (bounded
//! memory), parsed streaming, `type="playlist"` includes merged to depth 2 with cycle detection.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;

use super::{M3uEntry, ParseBudget, ParseStats, Playlist, parse_with_budget};
use crate::error::{AppError, Result};

/// Hard cap on a downloaded playlist.
pub const MAX_DOWNLOAD: u64 = crate::util::MAX_IMPORT_FILE_BYTES;
/// Include depth: the top-level playlist is depth 0.
pub const MAX_INCLUDE_DEPTH: u32 = 2;
const MAX_INCLUDE_FETCHES: usize = 32;
const LOAD_TIMEOUT: Duration = Duration::from_secs(300);

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

    /// Charge a playlist download against the import's remaining wire-byte budget.
    async fn fetch_playlist(&self, url: &str, remaining: &mut u64) -> Result<Fetched> {
        let fetched = self.fetch(url).await?;
        let len = tokio::fs::metadata(fetched.path()).await?.len();
        charge_download(remaining, len)?;
        Ok(fetched)
    }

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
                .timeout(LOAD_TIMEOUT)
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

fn charge_download(remaining: &mut u64, len: u64) -> Result<()> {
    if len > *remaining {
        *remaining = 0;
        return Err(AppError::from(crate::util::import_limit()));
    }
    *remaining -= len;
    Ok(())
}

/// Stream a successful response body to a temporary file.
async fn download(mut resp: reqwest::Response, remaining: &mut u64) -> Result<Fetched> {
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::from_status(
            status.as_u16(),
            None,
            "that playlist",
        ));
    }
    if let Some(len) = resp.content_length()
        && len > *remaining
    {
        charge_download(remaining, len)?;
    }
    let tmp = tokio::task::spawn_blocking(NamedTempFile::new)
        .await
        .map_err(|_| AppError::local("Creating the download file was interrupted."))??;
    let (file, path) = tmp.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    while let Some(chunk) = resp.chunk().await? {
        charge_download(remaining, chunk.len() as u64)?;
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
        let mut remaining = MAX_DOWNLOAD;
        self.fetch_playlist(url, &mut remaining).await
    }

    async fn fetch_playlist(&self, url: &str, remaining: &mut u64) -> Result<Fetched> {
        download(self.get(url)?.send().await?, remaining).await
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
        let mut remaining = MAX_DOWNLOAD;
        Ok(Conditional::Fetched(
            download(resp, &mut remaining).await?,
            validators,
        ))
    }
}

/// Parse a file (gzip/xz sniffed) on the blocking pool.
pub async fn parse_file(path: PathBuf) -> Result<Playlist> {
    Ok(
        parse_file_with_budget(Fetched::Local(path), ParseBudget::default())
            .await?
            .0,
    )
}

async fn parse_file_with_budget(
    fetched: Fetched,
    mut budget: ParseBudget,
) -> Result<(Playlist, ParseBudget)> {
    tokio::task::spawn_blocking(move || -> Result<_> {
        let r = crate::util::open_decompressed(fetched.path())?;
        let playlist = parse_with_budget(r, &mut budget)?;
        Ok((playlist, budget))
    })
    .await
    .map_err(|_| AppError::local("Reading the playlist was interrupted."))?
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
    load_with_deadline(location, fetcher, false).await
}

pub(crate) async fn load_complete(
    location: &Location,
    fetcher: &dyn Fetcher,
) -> Result<(Playlist, LoadStats)> {
    load_with_deadline(location, fetcher, true).await
}

async fn load_with_deadline(
    location: &Location,
    fetcher: &dyn Fetcher,
    require_complete: bool,
) -> Result<(Playlist, LoadStats)> {
    tokio::time::timeout(
        LOAD_TIMEOUT,
        load_with_policy(location, fetcher, require_complete),
    )
    .await
    .map_err(|_| AppError::network("The playlist took too long to load."))?
}

async fn load_with_policy(
    location: &Location,
    fetcher: &dyn Fetcher,
    require_complete: bool,
) -> Result<(Playlist, LoadStats)> {
    let mut import = Import {
        visited: HashSet::new(),
        stats: LoadStats::default(),
        parse_budget: ParseBudget::default(),
        downloads_left: MAX_DOWNLOAD,
        includes_left: MAX_INCLUDE_FETCHES,
        require_complete,
    };
    let top = match location {
        Location::Url(url) => {
            import.visited.insert(url.clone());
            fetcher
                .fetch_playlist(url, &mut import.downloads_left)
                .await?
        }
        Location::File(path) => Fetched::Local(path.clone()),
    };
    let pl = load_rec(top, fetcher, 0, &mut import).await?;
    import.stats.parse = pl.stats.clone();
    Ok((pl, import.stats))
}

struct Import {
    visited: HashSet<String>,
    stats: LoadStats,
    parse_budget: ParseBudget,
    downloads_left: u64,
    includes_left: usize,
    require_complete: bool,
}

fn load_rec<'a>(
    fetched: Fetched,
    fetcher: &'a dyn Fetcher,
    depth: u32,
    import: &'a mut Import,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Playlist>> + Send + 'a>> {
    Box::pin(async move {
        let (mut pl, budget) = parse_file_with_budget(fetched, import.parse_budget).await?;
        import.parse_budget = budget;
        let mut merged: Vec<M3uEntry> = Vec::with_capacity(pl.entries.len());
        for e in std::mem::take(&mut pl.entries) {
            if !e.is_include() {
                merged.push(e);
                continue;
            }
            let url = e.url;
            if !(url.starts_with("http://") || url.starts_with("https://"))
                || depth >= MAX_INCLUDE_DEPTH
                || import.visited.contains(&url)
            {
                import.stats.includes_skipped += 1;
                continue;
            }
            if import.includes_left == 0 {
                return Err(AppError::server(
                    "That playlist includes too many other playlists.",
                ));
            }
            import.includes_left -= 1;
            import.visited.insert(url.clone());
            let fetched = match fetcher
                .fetch_playlist(&url, &mut import.downloads_left)
                .await
            {
                Ok(file) => file,
                Err(_) if !import.require_complete && import.downloads_left > 0 => {
                    import.stats.includes_skipped += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let sub = load_rec(fetched, fetcher, depth + 1, import).await?;
            import.stats.includes_followed += 1;
            pl.stats.entries += sub.stats.entries;
            pl.stats.dropped += sub.stats.dropped;
            merged.extend(sub.entries);
        }
        pl.entries = merged;
        Ok(pl)
    })
}
