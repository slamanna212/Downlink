//! Images (PLAN §3.6). Logos and VOD art come from authenticated endpoints; the UI never sees the
//! server URL. Flow: [`ImageCache::register`] turns a source URL into `dlimg://<hash>/<size>`; the
//! custom protocol handler calls [`ImageCache::get`], which serves from the content-addressed
//! disk cache (`sha256(url)`) or fetches with auth, resizes to 160/512 px, stores both sizes and
//! evicts least-recently-used files beyond the cap (512 MB by default).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex as AsyncMutex;

use async_trait::async_trait;
use image::{DynamicImage, ImageFormat};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};
use crate::store::Store;
use crate::sync::now_utc;

pub const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageSize {
    /// Grid thumbnails.
    Grid = 160,
    /// Detail view.
    Detail = 512,
}

impl ImageSize {
    pub fn px(self) -> u32 {
        self as u32
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "160" => Some(Self::Grid),
            "512" => Some(Self::Detail),
            _ => None,
        }
    }
}

/// Raw bytes as fetched from a source.
#[derive(Debug, Clone)]
pub struct RawImage {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

/// Fetches an image URL with the owning source's credentials. Implementations must refuse URLs
/// outside the source's own origin (the key must never go elsewhere).
#[async_trait]
pub trait ImageFetcher: Send + Sync {
    async fn fetch(&self, source_id: i64, url: &str) -> Result<RawImage>;
}

/// What the protocol handler serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBytes {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    /// Served from a copy older than [`REVALIDATE_AFTER_SECS`]: the caller should call
    /// [`ImageCache::refresh`] in the background (the cached copy is still shown meanwhile).
    pub stale: bool,
}

/// Cached images are downloaded again after this long, in case the picture changed behind the
/// same URL.
pub const REVALIDATE_AFTER_SECS: i64 = 30 * 86_400;
/// After a failed re-check, try again this much later rather than on every request.
const REVALIDATE_RETRY_SECS: i64 = 86_400;

/// Unauthenticated fetch of a public image URL (M3U/Xtream logos). No credentials are sent.
pub async fn fetch_public(url: &str, pin: Option<&crate::tls::Pin>) -> Result<RawImage> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::local("That image address is not supported."));
    }
    let url = reqwest::Url::parse(url)?;
    let client = crate::tls::apply(reqwest::Client::builder().referer(false), &url, pin)?
        .user_agent(crate::user_agent())
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let r = client.get(url).send().await?;
    if !r.status().is_success() {
        return Err(AppError::from_status(
            r.status().as_u16(),
            None,
            "that image",
        ));
    }
    let content_type = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = r.bytes().await?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(AppError::media("That image is too large."));
    }
    Ok(RawImage {
        bytes: bytes.to_vec(),
        content_type,
    })
}

pub fn hash_url(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn dlimg_url(hash: &str, size: ImageSize) -> String {
    format!("dlimg://{hash}/{}", size.px())
}

/// Parse `dlimg://<hash>/<size>` (the host may be lower-cased by the webview; hashes are hex).
pub fn parse_dlimg(u: &str) -> Option<(String, ImageSize)> {
    let rest = u.strip_prefix("dlimg://")?;
    let (hash, size) = rest.split_once('/')?;
    let size = ImageSize::parse(size.trim_end_matches('/'))?;
    if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some((hash.to_ascii_lowercase(), size))
    } else {
        None
    }
}

const TOUCH_INTERVAL_SECS: i64 = 60;
const EVICTION_BATCH: i64 = 64;

struct CacheLocks {
    // A bounded set of locks coalesces duplicate downloads, including requests for different sizes.
    fetches: [AsyncMutex<()>; 64],
    writes: AsyncMutex<()>,
    /// Hashes with a background re-check in flight, so a grid of stale logos refreshes each once.
    refreshing: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl CacheLocks {
    fn new() -> Self {
        Self {
            fetches: std::array::from_fn(|_| AsyncMutex::new(())),
            writes: AsyncMutex::new(()),
            refreshing: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    fn fetch(&self, hash: &str) -> &AsyncMutex<()> {
        let mut hasher = DefaultHasher::new();
        hash.hash(&mut hasher);
        &self.fetches[(hasher.finish() as usize) % self.fetches.len()]
    }
}

#[derive(Clone)]
pub struct ImageCache {
    dir: PathBuf,
    store: Store,
    max_bytes: u64,
    locks: Arc<CacheLocks>,
}

impl ImageCache {
    pub fn new(dir: impl Into<PathBuf>, store: Store) -> Self {
        Self {
            dir: dir.into(),
            store,
            max_bytes: DEFAULT_MAX_BYTES,
            locks: Arc::new(CacheLocks::new()),
        }
    }

    pub fn with_max_bytes(mut self, max: u64) -> Self {
        self.max_bytes = max;
        self
    }

    /// Remember where `url` lives and return its `dlimg://` URL at `size`.
    pub async fn register(&self, source_id: i64, url: &str, size: ImageSize) -> Result<String> {
        let hash = hash_url(url);
        sqlx::query("INSERT OR IGNORE INTO image_origin (hash, source_id, url) VALUES (?, ?, ?)")
            .bind(&hash)
            .bind(source_id)
            .bind(url)
            .execute(self.store.pool())
            .await?;
        Ok(dlimg_url(&hash, size))
    }

    /// Register one page of artwork in a single write.
    pub(crate) async fn register_page(
        &self,
        images: &[(i64, &str)],
    ) -> Result<HashMap<String, String>> {
        if images.is_empty() {
            return Ok(HashMap::new());
        }
        let entries: Vec<_> = images
            .iter()
            .map(|&(source, url)| (hash_url(url), source, url))
            .collect();
        sqlx::query(
            "INSERT OR IGNORE INTO image_origin (hash, source_id, url)
             SELECT json_extract(value, '$[0]'), json_extract(value, '$[1]'), json_extract(value, '$[2]')
             FROM json_each(?)",
        )
        .bind(serde_json::to_string(&entries)?)
        .execute(self.store.pool())
        .await?;
        Ok(entries
            .into_iter()
            .map(|(hash, _, url)| (url.to_owned(), dlimg_url(&hash, ImageSize::Detail)))
            .collect())
    }

    fn key(hash: &str, size: ImageSize) -> String {
        format!("{hash}:{}", size.px())
    }

    fn mime_of(path: &str) -> &'static str {
        if path.ends_with(".jpg") {
            "image/jpeg"
        } else {
            "image/png"
        }
    }

    async fn cached(&self, hash: &str, size: ImageSize) -> Result<Option<ImageBytes>> {
        let key = Self::key(hash, size);
        let row: Option<(String, i64, i64)> =
            sqlx::query_as("SELECT path, last_used, fetched_at FROM image_cache WHERE key = ?")
                .bind(&key)
                .fetch_optional(self.store.pool())
                .await?;
        let Some((rel, last_used, fetched_at)) = row else {
            return Ok(None);
        };
        let Ok(bytes) = tokio::fs::read(self.dir.join(&rel)).await else {
            // A miss will replace the stale entry after the image is downloaded.
            return Ok(None);
        };
        let now = now_utc();
        if last_used <= now - TOUCH_INTERVAL_SECS {
            sqlx::query("UPDATE image_cache SET last_used = ? WHERE key = ? AND last_used <= ?")
                .bind(now)
                .bind(&key)
                .bind(now - TOUCH_INTERVAL_SECS)
                .execute(self.store.pool())
                .await?;
        }
        Ok(Some(ImageBytes {
            bytes,
            mime: Self::mime_of(&rel),
            stale: fetched_at <= now - REVALIDATE_AFTER_SECS,
        }))
    }

    /// Serve `dlimg://<hash>/<size>`.
    pub async fn get(
        &self,
        hash: &str,
        size: ImageSize,
        fetcher: &dyn ImageFetcher,
    ) -> Result<ImageBytes> {
        if let Some(hit) = self.cached(hash, size).await? {
            return Ok(hit);
        }
        let _fetch = self.locks.fetch(hash).lock().await;
        if let Some(hit) = self.cached(hash, size).await? {
            return Ok(hit);
        }
        self.download(hash, size, fetcher).await
    }

    /// Download `hash` again (it was served [`ImageBytes::stale`]) and replace both sizes. A
    /// failure keeps the cached copy and waits [`REVALIDATE_RETRY_SECS`] before trying again.
    /// Concurrent calls for the same image do the work once.
    pub async fn refresh(&self, hash: &str, fetcher: &dyn ImageFetcher) -> Result<()> {
        let claimed = self
            .locks
            .refreshing
            .lock()
            .map(|mut g| g.insert(hash.to_owned()))
            .unwrap_or(false);
        if !claimed {
            return Ok(());
        }
        let r = async {
            let _fetch = self.locks.fetch(hash).lock().await;
            let fresh: Option<i64> =
                sqlx::query_scalar("SELECT min(fetched_at) FROM image_cache WHERE key IN (?, ?)")
                    .bind(Self::key(hash, ImageSize::Grid))
                    .bind(Self::key(hash, ImageSize::Detail))
                    .fetch_one(self.store.pool())
                    .await?;
            if fresh.is_some_and(|t| t > now_utc() - REVALIDATE_AFTER_SECS) {
                return Ok(()); // refreshed by someone else meanwhile
            }
            if let Err(e) = self.download(hash, ImageSize::Grid, fetcher).await {
                let retry_at = now_utc() - REVALIDATE_AFTER_SECS + REVALIDATE_RETRY_SECS;
                sqlx::query("UPDATE image_cache SET fetched_at = ? WHERE key IN (?, ?)")
                    .bind(retry_at)
                    .bind(Self::key(hash, ImageSize::Grid))
                    .bind(Self::key(hash, ImageSize::Detail))
                    .execute(self.store.pool())
                    .await?;
                return Err(e);
            }
            Ok(())
        }
        .await;
        if let Ok(mut g) = self.locks.refreshing.lock() {
            g.remove(hash);
        }
        r
    }

    /// Fetch the image behind `hash`, store both sizes and return `size`. The caller holds the
    /// hash's fetch lock.
    async fn download(
        &self,
        hash: &str,
        size: ImageSize,
        fetcher: &dyn ImageFetcher,
    ) -> Result<ImageBytes> {
        let origin: Option<(i64, String)> =
            sqlx::query_as("SELECT source_id, url FROM image_origin WHERE hash = ?")
                .bind(hash)
                .fetch_optional(self.store.pool())
                .await?;
        let (source_id, url) =
            origin.ok_or_else(|| AppError::local("That image is no longer available."))?;
        let raw = fetcher.fetch(source_id, &url).await?;
        let encoded = tokio::task::spawn_blocking(move || encode_sizes(&raw.bytes))
            .await
            .map_err(|_| AppError::local("Image processing was interrupted."))??;
        let _writes = self.locks.writes.lock().await;
        let mut wanted = None;
        let mut entries = Vec::with_capacity(encoded.len());
        for (sz, bytes, ext) in encoded {
            let rel = format!("{}/{hash}-{}.{ext}", &hash[..2], sz.px());
            let abs = self.dir.join(&rel);
            if let Some(parent) = abs.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            // Write then rename, so a reader never sees half of a replaced (re-checked) file.
            let tmp = abs.with_extension(format!("{ext}.part"));
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &abs).await?;
            entries.push((
                Self::key(hash, sz),
                rel.clone(),
                i64::try_from(bytes.len()).unwrap_or(i64::MAX),
            ));
            if sz == size {
                wanted = Some(ImageBytes {
                    bytes,
                    mime: Self::mime_of(&rel),
                    stale: false,
                });
            }
        }
        // A re-download may change the format (png <-> jpg): the old file is removed below.
        let old_paths: Vec<String> =
            sqlx::query_scalar("SELECT path FROM image_cache WHERE key IN (?, ?)")
                .bind(Self::key(hash, ImageSize::Grid))
                .bind(Self::key(hash, ImageSize::Detail))
                .fetch_all(self.store.pool())
                .await?;
        let mut tx = self.store.pool().begin().await?;
        for (key, rel, bytes) in &entries {
            sqlx::query(
                "INSERT INTO image_cache (key, path, bytes, last_used, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(key) DO UPDATE SET path = excluded.path, bytes = excluded.bytes,
                                                last_used = excluded.last_used,
                                                fetched_at = excluded.fetched_at",
            )
            .bind(key)
            .bind(rel)
            .bind(bytes)
            .bind(now_utc())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        for old in old_paths
            .iter()
            .filter(|p| !entries.iter().any(|(_, rel, _)| rel == *p))
        {
            let _ = tokio::fs::remove_file(self.dir.join(old)).await;
        }
        self.evict_locked().await?;
        wanted.ok_or_else(|| AppError::media("The image could not be prepared."))
    }

    /// Total cached bytes.
    pub async fn total_bytes(&self) -> Result<u64> {
        let bytes: i64 = sqlx::query_scalar("SELECT bytes FROM image_cache_size WHERE id = 1")
            .fetch_one(self.store.pool())
            .await?;
        Ok(u64::try_from(bytes).unwrap_or(0))
    }

    /// Delete least-recently-used files in batches until under the cap.
    pub async fn evict(&self) -> Result<usize> {
        let _writes = self.locks.writes.lock().await;
        self.evict_locked().await
    }

    async fn evict_locked(&self) -> Result<usize> {
        let mut total = self.total_bytes().await?;
        let mut removed = 0;
        while total > self.max_bytes {
            let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await?;
            let rows: Vec<(String, String, i64)> = sqlx::query_as(
                "SELECT key, path, bytes FROM image_cache ORDER BY last_used, key LIMIT ?",
            )
            .bind(EVICTION_BATCH)
            .fetch_all(&mut *tx)
            .await?;
            if rows.is_empty() {
                break;
            }
            let mut evicted = Vec::new();
            for (key, rel, bytes) in rows {
                evicted.push((key, rel));
                total = total.saturating_sub(u64::try_from(bytes).unwrap_or(0));
                if total <= self.max_bytes {
                    break;
                }
            }
            let keys: Vec<_> = evicted.iter().map(|(key, _)| key).collect();
            sqlx::query("DELETE FROM image_cache WHERE key IN (SELECT value FROM json_each(?))")
                .bind(serde_json::to_string(&keys)?)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            for (_, rel) in evicted {
                let _ = tokio::fs::remove_file(self.dir.join(rel)).await;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Decode once, produce both sizes. Never upscales. PNG when the image has transparency (logos),
/// JPEG otherwise.
fn encode_sizes(bytes: &[u8]) -> Result<Vec<(ImageSize, Vec<u8>, &'static str)>> {
    let img = image::load_from_memory(bytes)
        .map_err(|_| AppError::media("That image is not in a format Downlink can show."))?;
    let alpha = img.color().has_alpha();
    let mut out = Vec::new();
    for size in [ImageSize::Grid, ImageSize::Detail] {
        let px = size.px();
        let scaled: DynamicImage = if img.width() > px || img.height() > px {
            img.thumbnail(px, px)
        } else {
            img.clone()
        };
        let mut buf = Cursor::new(Vec::new());
        let ext = if alpha {
            scaled
                .write_to(&mut buf, ImageFormat::Png)
                .map_err(|_| AppError::media("The image could not be prepared."))?;
            "png"
        } else {
            DynamicImage::ImageRgb8(scaled.to_rgb8())
                .write_to(&mut buf, ImageFormat::Jpeg)
                .map_err(|_| AppError::media("The image could not be prepared."))?;
            "jpg"
        };
        out.push((size, buf.into_inner(), ext));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
