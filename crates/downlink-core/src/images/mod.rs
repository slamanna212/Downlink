//! Images (PLAN §3.6). Logos and VOD art come from authenticated endpoints; the UI never sees the
//! server URL. Flow: [`ImageCache::register`] turns a source URL into `dlimg://<hash>/<size>`; the
//! custom protocol handler calls [`ImageCache::get`], which serves from the content-addressed
//! disk cache (`sha256(url)`) or fetches with auth, resizes to 160/512 px, stores both sizes and
//! evicts least-recently-used files beyond the cap (512 MB by default).

use std::io::Cursor;
use std::path::{Path, PathBuf};

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

#[derive(Clone)]
pub struct ImageCache {
    dir: PathBuf,
    store: Store,
    max_bytes: u64,
}

impl ImageCache {
    pub fn new(dir: impl Into<PathBuf>, store: Store) -> Self {
        Self {
            dir: dir.into(),
            store,
            max_bytes: DEFAULT_MAX_BYTES,
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
        let row: Option<(String,)> = sqlx::query_as("SELECT path FROM image_cache WHERE key = ?")
            .bind(Self::key(hash, size))
            .fetch_optional(self.store.pool())
            .await?;
        let Some((rel,)) = row else { return Ok(None) };
        match tokio::fs::read(self.dir.join(&rel)).await {
            Ok(bytes) => {
                sqlx::query("UPDATE image_cache SET last_used = ? WHERE key = ?")
                    .bind(now_utc())
                    .bind(Self::key(hash, size))
                    .execute(self.store.pool())
                    .await?;
                Ok(Some(ImageBytes {
                    bytes,
                    mime: Self::mime_of(&rel),
                }))
            }
            Err(_) => {
                // File vanished: forget the row and refetch.
                sqlx::query("DELETE FROM image_cache WHERE key = ?")
                    .bind(Self::key(hash, size))
                    .execute(self.store.pool())
                    .await?;
                Ok(None)
            }
        }
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
        let mut wanted = None;
        for (sz, bytes, ext) in encoded {
            let rel = format!("{}/{hash}-{}.{ext}", &hash[..2], sz.px());
            let abs = self.dir.join(&rel);
            if let Some(parent) = abs.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&abs, &bytes).await?;
            sqlx::query(
                "INSERT INTO image_cache (key, path, bytes, last_used) VALUES (?, ?, ?, ?)
                 ON CONFLICT(key) DO UPDATE SET path = excluded.path, bytes = excluded.bytes,
                                                last_used = excluded.last_used",
            )
            .bind(Self::key(hash, sz))
            .bind(&rel)
            .bind(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
            .bind(now_utc())
            .execute(self.store.pool())
            .await?;
            if sz == size {
                wanted = Some(ImageBytes {
                    bytes,
                    mime: Self::mime_of(&rel),
                });
            }
        }
        self.evict().await?;
        wanted.ok_or_else(|| AppError::media("The image could not be prepared."))
    }

    /// Total cached bytes.
    pub async fn total_bytes(&self) -> Result<u64> {
        let n: Option<i64> = sqlx::query_scalar("SELECT sum(bytes) FROM image_cache")
            .fetch_one(self.store.pool())
            .await?;
        Ok(u64::try_from(n.unwrap_or(0)).unwrap_or(0))
    }

    /// Delete least-recently-used files until under the cap.
    pub async fn evict(&self) -> Result<usize> {
        let mut total = self.total_bytes().await?;
        if total <= self.max_bytes {
            return Ok(0);
        }
        let rows: Vec<(String, String, i64)> =
            sqlx::query_as("SELECT key, path, bytes FROM image_cache ORDER BY last_used, key")
                .fetch_all(self.store.pool())
                .await?;
        let mut removed = 0;
        for (key, rel, bytes) in rows {
            if total <= self.max_bytes {
                break;
            }
            let _ = tokio::fs::remove_file(self.dir.join(&rel)).await;
            sqlx::query("DELETE FROM image_cache WHERE key = ?")
                .bind(&key)
                .execute(self.store.pool())
                .await?;
            total = total.saturating_sub(u64::try_from(bytes).unwrap_or(0));
            removed += 1;
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
