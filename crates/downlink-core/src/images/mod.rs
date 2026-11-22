//! Images (PLAN §3.6). Logos and VOD art come from authenticated endpoints; the UI never sees the
//! server URL. Flow: [`ImageCache::register`] turns a source URL into `dlimg://<hash>/<size>`; the
//! custom protocol handler calls [`ImageCache::get`], which serves from the content-addressed
//! disk cache keyed by source UUID and URL or fetches with auth, resizes to 160/512 px, stores both sizes and
//! evicts least-recently-used files beyond the cap (512 MB by default).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Cursor, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use tokio::sync::{Mutex as AsyncMutex, Semaphore};

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

    async fn fetch_for(&self, source_id: i64, _source_uuid: &str, url: &str) -> Result<RawImage> {
        self.fetch(source_id, url).await
    }
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

pub(crate) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
static PUBLIC_IMAGE_FETCHES: Semaphore = Semaphore::const_new(8);
static PUBLIC_IMAGE_CLIENT: LazyLock<Result<reqwest::Client>> =
    LazyLock::new(|| Ok(public_client_builder().build()?));

fn public_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .referer(false)
        .user_agent(crate::user_agent())
        .pool_max_idle_per_host(2)
        .timeout(std::time::Duration::from_secs(20))
}

/// Fetch public artwork without sending source authentication headers.
pub async fn fetch_public(url: &str, pin: Option<&crate::tls::Pin>) -> Result<RawImage> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::local("That image address is not supported."));
    }
    let url = reqwest::Url::parse(url)?;
    let _permit = PUBLIC_IMAGE_FETCHES
        .acquire()
        .await
        .map_err(|_| AppError::local("Image fetching is unavailable."))?;
    let client = match pin.filter(|pin| pin.matches(&url)) {
        Some(pin) => pin.apply(public_client_builder())?.build()?,
        None => PUBLIC_IMAGE_CLIENT.as_ref().map_err(Clone::clone)?.clone(),
    };
    let mut r = client.get(url).send().await?;
    if !r.status().is_success() {
        return Err(AppError::from_status(
            r.status().as_u16(),
            None,
            "that image",
        ));
    }
    if r.content_length()
        .is_some_and(|bytes| bytes > MAX_IMAGE_BYTES as u64)
    {
        return Err(AppError::media("That image is too large."));
    }
    let content_type = r
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    while let Some(chunk) = r.chunk().await? {
        if chunk.len() > MAX_IMAGE_BYTES - bytes.len() {
            return Err(AppError::media("That image is too large."));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(RawImage {
        bytes,
        content_type,
    })
}

pub fn hash_url(url: &str) -> String {
    Sha256::digest(url.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn account_hash(source_uuid: &str, url: &str) -> String {
    hash_url(&format!("downlink-artwork-v2\0{source_uuid}\0{url}"))
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

struct RefreshClaim<'a> {
    locks: &'a CacheLocks,
    hash: &'a str,
}

impl<'a> RefreshClaim<'a> {
    fn new(locks: &'a CacheLocks, hash: &'a str) -> Option<Self> {
        let mut refreshing = locks.refreshing.lock().ok()?;
        refreshing
            .insert(hash.to_owned())
            .then(|| Self { locks, hash })
    }
}

impl Drop for RefreshClaim<'_> {
    fn drop(&mut self) {
        if let Ok(mut refreshing) = self.locks.refreshing.lock() {
            refreshing.remove(self.hash);
        }
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
        let source = self.store.source(source_id).await?;
        self.register_scoped(source_id, &source.uuid, url, size)
            .await
    }

    pub(crate) async fn register_scoped(
        &self,
        source_id: i64,
        source_uuid: &str,
        url: &str,
        size: ImageSize,
    ) -> Result<String> {
        let hash = account_hash(source_uuid, url);
        sqlx::query(
            "INSERT OR IGNORE INTO image_origin (hash, source_id, url)
             SELECT ?, id, ? FROM source WHERE id = ? AND uuid = ?",
        )
        .bind(&hash)
        .bind(url)
        .bind(source_id)
        .bind(source_uuid)
        .execute(self.store.pool())
        .await?;
        Ok(dlimg_url(&hash, size))
    }

    /// Register one page of artwork in a single write.
    pub(crate) async fn register_page(
        &self,
        images: &[(i64, &str)],
    ) -> Result<HashMap<i64, HashMap<String, String>>> {
        if images.is_empty() {
            return Ok(HashMap::new());
        }
        let sources: HashMap<_, _> = self
            .store
            .list_sources()
            .await?
            .into_iter()
            .map(|source| (source.id, source.uuid))
            .collect();
        let entries: Vec<_> = images
            .iter()
            .filter_map(|&(source, url)| {
                let uuid = sources.get(&source)?;
                Some((account_hash(uuid, url), source, url, uuid))
            })
            .collect();
        sqlx::query(
            "INSERT OR IGNORE INTO image_origin (hash, source_id, url)
             SELECT json_extract(value, '$[0]'), json_extract(value, '$[1]'), json_extract(value, '$[2]')
             FROM json_each(?) JOIN source ON source.id = json_extract(value, '$[1]')
                                  AND source.uuid = json_extract(value, '$[3]')",
        )
        .bind(serde_json::to_string(&entries)?)
        .execute(self.store.pool())
        .await?;
        let mut artwork: HashMap<i64, HashMap<String, String>> = HashMap::new();
        for (hash, source, url, _) in entries {
            artwork
                .entry(source)
                .or_default()
                .insert(url.to_owned(), dlimg_url(&hash, ImageSize::Detail));
        }
        Ok(artwork)
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
        let row: Option<(String, i64, i64)> = sqlx::query_as(
            "SELECT path, last_used, fetched_at FROM image_cache WHERE key = ?
                 AND EXISTS (SELECT 1 FROM image_origin WHERE hash = ?)",
        )
        .bind(&key)
        .bind(hash)
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
        let Some(_refresh) = RefreshClaim::new(&self.locks, hash) else {
            return Ok(());
        };
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

    /// Fetch the image behind `hash`, store both sizes and return `size`. The caller holds the
    /// hash's fetch lock.
    async fn download(
        &self,
        hash: &str,
        size: ImageSize,
        fetcher: &dyn ImageFetcher,
    ) -> Result<ImageBytes> {
        let origin: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT source_id, source.uuid, url FROM image_origin
             JOIN source ON source.id = image_origin.source_id WHERE hash = ?",
        )
        .bind(hash)
        .fetch_optional(self.store.pool())
        .await?;
        let (source_id, source_uuid, url) =
            origin.ok_or_else(|| AppError::local("That image is no longer available."))?;
        let raw = fetcher.fetch_for(source_id, &source_uuid, &url).await?;
        let encoded = tokio::task::spawn_blocking(move || encode_sizes(&raw.bytes))
            .await
            .map_err(|_| AppError::local("Image processing was interrupted."))??;
        let cache = self.clone();
        let hash = hash.to_owned();
        // Finish publication even when the requesting view cancels. A committed row must
        // keep its file, while an unsuccessful transaction drops its temporary files.
        tokio::spawn(async move { cache.publish(&hash, size, encoded).await })
            .await
            .map_err(|_| AppError::local("Saving the artwork was interrupted."))?
    }

    async fn publish(
        &self,
        hash: &str,
        size: ImageSize,
        encoded: Vec<(ImageSize, Vec<u8>, &'static str)>,
    ) -> Result<ImageBytes> {
        let shard = hash[..2].to_owned();
        let dir = self.dir.join(&shard);
        let (entries, wanted) = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&dir)?;
            let mut wanted = None;
            let mut entries = Vec::with_capacity(encoded.len());
            for (sz, bytes, ext) in encoded {
                let mut file = tempfile::Builder::new()
                    .prefix("image-")
                    .suffix(&format!(".{ext}"))
                    .tempfile_in(&dir)?;
                file.write_all(&bytes)?;
                let path = file
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| AppError::local("The image cache path is invalid."))?;
                let rel = format!("{shard}/{path}");
                entries.push((
                    sz,
                    rel.clone(),
                    i64::try_from(bytes.len()).unwrap_or(i64::MAX),
                    file.into_temp_path(),
                ));
                if sz == size {
                    wanted = Some(ImageBytes {
                        bytes,
                        mime: Self::mime_of(&rel),
                        stale: false,
                    });
                }
            }
            Ok::<_, AppError>((entries, wanted))
        })
        .await
        .map_err(|_| AppError::local("Saving the artwork was interrupted."))??;

        // The staged paths are private to this request. Only publication and pruning need
        // to run against a consistent view of the cache index.
        let writes = self.locks.writes.lock().await;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await?;
        let active: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM image_origin WHERE hash = ?)")
                .bind(hash)
                .fetch_one(&mut *tx)
                .await?;
        if !active {
            return Err(AppError::local("That image is no longer available."));
        }
        for (size, rel, bytes, _) in &entries {
            let key = Self::key(hash, *size);
            // Obsolete files stay accounted for until removal succeeds, including after a crash.
            sqlx::query(
                "UPDATE image_cache SET key = 'garbage:' || path, last_used = ? WHERE key = ?",
            )
            .bind(i64::MIN)
            .bind(&key)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO image_cache (key, path, bytes, last_used, fetched_at)
                         VALUES (?1, ?2, ?3, ?4, ?4)",
            )
            .bind(key)
            .bind(rel)
            .bind(bytes)
            .bind(now_utc())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        for (_, _, _, path) in entries {
            path.keep().map_err(|e| e.error)?;
        }
        drop(writes);
        if let Err(error) = self.evict().await {
            tracing::warn!(%error, "artwork eviction will be retried");
        }
        wanted.ok_or_else(|| AppError::media("The image could not be prepared."))
    }

    /// Account for files left behind after interruption and remove stale cache entries.
    pub(crate) async fn reconcile(&self) -> Result<()> {
        let _writes = self.locks.writes.lock().await;
        let dir = self.dir.clone();
        let disk = tokio::task::spawn_blocking(move || -> std::io::Result<HashMap<String, i64>> {
            let mut disk = HashMap::new();
            let shards = match std::fs::read_dir(&dir) {
                Ok(shards) => shards,
                Err(e) if e.kind() == ErrorKind::NotFound => return Ok(disk),
                Err(e) => return Err(e),
            };
            for shard in shards {
                let shard = shard?;
                let name = shard.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.len() != 2
                    || !name.bytes().all(|b| b.is_ascii_hexdigit())
                    || !shard.file_type()?.is_dir()
                {
                    continue;
                }
                for entry in std::fs::read_dir(shard.path())? {
                    let entry = entry?;
                    if entry.file_type()?.is_file()
                        && let Some(file) = entry.file_name().to_str()
                    {
                        let bytes = i64::try_from(entry.metadata()?.len()).unwrap_or(i64::MAX);
                        disk.insert(format!("{name}/{file}"), bytes);
                    }
                }
            }
            Ok(disk)
        })
        .await
        .map_err(|_| AppError::local("Checking the artwork cache was interrupted."))??;
        let mut disk = disk;
        let mut tx = self.store.pool().begin_with("BEGIN IMMEDIATE").await?;
        let rows: Vec<(String, String, i64, bool)> = sqlx::query_as(
            "SELECT key, path, bytes, EXISTS (
                 SELECT 1 FROM image_origin WHERE hash = substr(image_cache.key, 1, 64)
             ) FROM image_cache",
        )
        .fetch_all(&mut *tx)
        .await?;
        for (key, path, bytes, active) in rows {
            match disk.remove(&path) {
                None => {
                    sqlx::query("DELETE FROM image_cache WHERE key = ?")
                        .bind(key)
                        .execute(&mut *tx)
                        .await?;
                }
                Some(actual) if !active || actual != bytes => {
                    let obsolete = !active;
                    let next_key = if obsolete {
                        format!("garbage:{path}")
                    } else {
                        key.clone()
                    };
                    sqlx::query("UPDATE image_cache SET key = ?, bytes = ?, last_used = CASE WHEN ? THEN ? ELSE last_used END WHERE key = ?")
                        .bind(next_key).bind(actual).bind(obsolete).bind(i64::MIN).bind(key).execute(&mut *tx).await?;
                }
                _ => {}
            }
        }
        for (path, bytes) in disk {
            sqlx::query("INSERT INTO image_cache (key, path, bytes, last_used, fetched_at) VALUES (?, ?, ?, ?, 0)")
                .bind(format!("garbage:{path}")).bind(path).bind(bytes).bind(i64::MIN)
                .execute(&mut *tx).await?;
        }
        // Repair the total even if a previous version or an old recovery backup had drifted.
        sqlx::query("UPDATE image_cache_size SET bytes = (SELECT coalesce(sum(bytes), 0) FROM image_cache) WHERE id = 1")
            .execute(&mut *tx).await?;
        tx.commit().await?;
        self.evict_locked().await?;
        Ok(())
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
        let mut cursor = (i64::MIN, String::new());
        let mut failure = None;
        loop {
            let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
                "SELECT key, path, bytes, last_used FROM image_cache
                 WHERE (last_used, key) > (?, ?) AND (? OR last_used = ?)
                 ORDER BY last_used, key LIMIT ?",
            )
            .bind(cursor.0)
            .bind(&cursor.1)
            .bind(total > self.max_bytes)
            .bind(i64::MIN)
            .bind(EVICTION_BATCH)
            .fetch_all(self.store.pool())
            .await?;
            if rows.is_empty() {
                break;
            }
            for (key, rel, bytes, last_used) in rows {
                cursor = (last_used, key);
                if total <= self.max_bytes && last_used != i64::MIN {
                    break;
                }
                let path = Path::new(&rel);
                if path.components().count() != 2
                    || !path
                        .components()
                        .all(|part| matches!(part, std::path::Component::Normal(_)))
                {
                    failure.get_or_insert_with(|| {
                        std::io::Error::new(ErrorKind::InvalidData, "Invalid cache path")
                    });
                    continue;
                }
                let result = tokio::fs::remove_file(self.dir.join(path)).await;
                if let Err(error) = result
                    && error.kind() != ErrorKind::NotFound
                {
                    failure.get_or_insert(error);
                    continue;
                }
                sqlx::query("DELETE FROM image_cache WHERE key = ?")
                    .bind(&cursor.1)
                    .execute(self.store.pool())
                    .await?;
                total = total.saturating_sub(u64::try_from(bytes).unwrap_or(0));
                removed += 1;
            }
        }
        if let Some(error) = failure {
            return Err(error.into());
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
