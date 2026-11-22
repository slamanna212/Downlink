use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::store::NewSource;

struct Stub {
    calls: AtomicUsize,
    bytes: Vec<u8>,
}

#[async_trait]
impl ImageFetcher for Stub {
    async fn fetch(&self, _s: i64, _u: &str) -> Result<RawImage> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(RawImage {
            bytes: self.bytes.clone(),
            content_type: Some("image/png".into()),
        })
    }
}

fn png(w: u32, h: u32, alpha: bool) -> Vec<u8> {
    let img = if alpha {
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            w,
            h,
            image::Rgba([200, 10, 10, 128]),
        ))
    } else {
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([10, 200, 10])))
    };
    let mut b = Cursor::new(Vec::new());
    img.write_to(&mut b, ImageFormat::Png).unwrap();
    b.into_inner()
}

async fn setup(max: u64) -> (ImageCache, i64, tempfile::TempDir) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "T".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    (
        ImageCache::new(dir.path(), store).with_max_bytes(max),
        src.id,
        dir,
    )
}

#[test]
fn dlimg_roundtrip_and_validation() {
    let h = hash_url("https://d.test/api/channels/logos/1/cache");
    assert_eq!(h.len(), 64);
    assert_eq!(h, hash_url("https://d.test/api/channels/logos/1/cache"));
    let u = dlimg_url(&h, ImageSize::Grid);
    assert_eq!(u, format!("dlimg://{h}/160"));
    assert_eq!(parse_dlimg(&u), Some((h.clone(), ImageSize::Grid)));
    assert_eq!(
        parse_dlimg(&format!("dlimg://{h}/512/")),
        Some((h.clone(), ImageSize::Detail))
    );
    assert!(parse_dlimg(&format!("dlimg://{h}/300")).is_none());
    assert!(parse_dlimg("dlimg://zzz/160").is_none());
    assert!(parse_dlimg("https://x/160").is_none());
}

#[tokio::test]
async fn miss_fetches_once_resizes_both_sizes_then_hits_disk() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let url = "https://d.test/api/channels/logos/9/cache";
    let dl = cache.register(sid, url, ImageSize::Grid).await.unwrap();
    let (hash, size) = parse_dlimg(&dl).unwrap();
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(800, 400, true),
    };

    let g = cache.get(&hash, size, &stub).await.unwrap();
    assert_eq!(g.mime, "image/png");
    let decoded = image::load_from_memory(&g.bytes).unwrap();
    assert_eq!(
        (decoded.width(), decoded.height()),
        (160, 80),
        "aspect kept, fits 160"
    );

    let d = cache.get(&hash, ImageSize::Detail, &stub).await.unwrap();
    assert_eq!(image::load_from_memory(&d.bytes).unwrap().width(), 512);
    assert_eq!(
        stub.calls.load(Ordering::SeqCst),
        1,
        "second size came from the same fetch"
    );

    let path: String = sqlx::query_scalar("SELECT path FROM image_cache WHERE key = ?")
        .bind(ImageCache::key(&hash, size))
        .fetch_one(cache.store.pool())
        .await
        .unwrap();
    assert!(cache.dir().join(path).exists());
    assert_eq!(cache.get(&hash, size, &stub).await.unwrap(), g);
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn simultaneous_requests_for_two_sizes_share_the_download() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let url = "https://d.test/shared";
    let dl = cache.register(sid, url, ImageSize::Grid).await.unwrap();
    let (hash, _) = parse_dlimg(&dl).unwrap();
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(600, 400, true),
    };
    let (grid, detail) = tokio::join!(
        cache.get(&hash, ImageSize::Grid, &stub),
        cache.get(&hash, ImageSize::Detail, &stub)
    );
    let grid = grid.unwrap();
    let detail = detail.unwrap();
    assert_eq!(stub.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        cache.total_bytes().await.unwrap(),
        (grid.bytes.len() + detail.bytes.len()) as u64
    );
}

#[tokio::test]
async fn small_images_are_not_upscaled_and_opaque_becomes_jpeg() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let dl = cache
        .register(sid, "https://d.test/a", ImageSize::Grid)
        .await
        .unwrap();
    let (hash, _) = parse_dlimg(&dl).unwrap();
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(100, 50, false),
    };
    let r = cache.get(&hash, ImageSize::Detail, &stub).await.unwrap();
    assert_eq!(r.mime, "image/jpeg");
    let d = image::load_from_memory(&r.bytes).unwrap();
    assert_eq!((d.width(), d.height()), (100, 50));
}

#[tokio::test]
async fn unknown_hash_and_bad_image_are_errors() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: b"not an image".to_vec(),
    };
    let e = cache
        .get(&hash_url("nope"), ImageSize::Grid, &stub)
        .await
        .unwrap_err();
    assert_eq!(e.code, crate::ErrorCode::Local);
    let dl = cache
        .register(sid, "https://d.test/bad", ImageSize::Grid)
        .await
        .unwrap();
    let (hash, _) = parse_dlimg(&dl).unwrap();
    assert_eq!(
        cache
            .get(&hash, ImageSize::Grid, &stub)
            .await
            .unwrap_err()
            .code,
        crate::ErrorCode::Media
    );
}

#[tokio::test]
async fn lru_eviction_drops_oldest_first() {
    let (cache, sid, _d) = setup(1).await; // absurdly small: everything but the newest goes
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(300, 300, true),
    };
    let mut hashes = Vec::new();
    for i in 0..3 {
        let dl = cache
            .register(sid, &format!("https://d.test/{i}"), ImageSize::Grid)
            .await
            .unwrap();
        hashes.push(parse_dlimg(&dl).unwrap().0);
    }
    // Pin distinct last_used so ordering is deterministic.
    let big = ImageCache::new(cache.dir(), cache.store.clone()).with_max_bytes(u64::MAX);
    for (i, h) in hashes.iter().enumerate() {
        big.get(h, ImageSize::Grid, &stub).await.unwrap();
        sqlx::query("UPDATE image_cache SET last_used = ? WHERE key LIKE ?")
            .bind(100 + i as i64)
            .bind(format!("{h}:%"))
            .execute(cache.store.pool())
            .await
            .unwrap();
    }
    let total = big.total_bytes().await.unwrap();
    let per_hash = total / 3;
    let capped =
        ImageCache::new(cache.dir(), cache.store.clone()).with_max_bytes(per_hash + per_hash / 2);
    capped.evict().await.unwrap();
    let left: Vec<String> = sqlx::query_scalar("SELECT key FROM image_cache ORDER BY last_used")
        .fetch_all(cache.store.pool())
        .await
        .unwrap();
    assert!(
        left.iter().all(|k| k.starts_with(&hashes[2])),
        "only the most recent hash survives: {left:?}"
    );
    assert!(capped.total_bytes().await.unwrap() <= per_hash + per_hash / 2);
    let remaining = std::fs::read_dir(cache.dir())
        .unwrap()
        .map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap().count())
        .sum::<usize>();
    assert_eq!(remaining, left.len());
}

struct Failing;

#[async_trait]
impl ImageFetcher for Failing {
    async fn fetch(&self, _s: i64, _u: &str) -> Result<RawImage> {
        Err(AppError::network("offline"))
    }
}

async fn age_entries(cache: &ImageCache, secs: i64) {
    sqlx::query("UPDATE image_cache SET fetched_at = ?")
        .bind(now_utc() - secs)
        .execute(cache.store.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn old_images_are_served_stale_then_rechecked_once() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let dl = cache
        .register(sid, "https://d.test/logo.png", ImageSize::Grid)
        .await
        .unwrap();
    let (hash, size) = parse_dlimg(&dl).unwrap();
    let logo = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(64, 64, true),
    };
    assert!(!cache.get(&hash, size, &logo).await.unwrap().stale);
    assert!(!cache.get(&hash, size, &logo).await.unwrap().stale);

    age_entries(&cache, REVALIDATE_AFTER_SECS + 60).await;
    let old = cache.get(&hash, size, &logo).await.unwrap();
    assert!(
        old.stale && old.mime == "image/png",
        "old copy still served"
    );
    assert_eq!(
        logo.calls.load(Ordering::SeqCst),
        1,
        "serving stale fetches nothing"
    );

    // The picture changed behind the same URL (now opaque: re-encoded as JPEG).
    let replaced = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(64, 64, false),
    };
    cache.refresh(&hash, &replaced).await.unwrap();
    cache.refresh(&hash, &replaced).await.unwrap();
    assert_eq!(
        replaced.calls.load(Ordering::SeqCst),
        1,
        "fresh after one re-check"
    );
    let new = cache.get(&hash, size, &replaced).await.unwrap();
    assert!(!new.stale && new.mime == "image/jpeg");
    let pngs = std::fs::read_dir(cache.dir().join(&hash[..2]))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "png")
        })
        .count();
    assert_eq!(pngs, 0, "the replaced files are removed");
}

#[tokio::test]
async fn a_failed_recheck_keeps_the_copy_and_waits_a_day() {
    let (cache, sid, _d) = setup(DEFAULT_MAX_BYTES).await;
    let dl = cache
        .register(sid, "https://d.test/logo2.png", ImageSize::Grid)
        .await
        .unwrap();
    let (hash, size) = parse_dlimg(&dl).unwrap();
    let logo = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(64, 64, true),
    };
    cache.get(&hash, size, &logo).await.unwrap();
    age_entries(&cache, REVALIDATE_AFTER_SECS + 60).await;
    assert!(cache.refresh(&hash, &Failing).await.is_err());
    let hit = cache.get(&hash, size, &logo).await.unwrap();
    assert!(!hit.stale, "not retried on every request");
    assert_eq!(hit.mime, "image/png", "cached copy kept");
}

#[tokio::test]
async fn identical_artwork_urls_are_scoped_to_the_account_uuid() {
    let (cache, first, _dir) = setup(DEFAULT_MAX_BYTES).await;
    let second = cache
        .store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "Second account".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap()
        .id;
    let url = "https://source.test/image?token={dl:token}";
    let page = cache
        .register_page(&[(first, url), (second, url)])
        .await
        .unwrap();
    assert_ne!(page[&first][url], page[&second][url]);
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(80, 40, true),
    };
    let old_hash = parse_dlimg(&page[&second][url]).unwrap().0;
    cache.get(&old_hash, ImageSize::Grid, &stub).await.unwrap();
    cache.store.delete_source(second).await.unwrap();
    assert!(cache.get(&old_hash, ImageSize::Grid, &stub).await.is_err());

    let replacement = cache
        .store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "Replacement account".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap()
        .id;
    assert_eq!(second, replacement);
    let icon = cache
        .register(replacement, url, ImageSize::Grid)
        .await
        .unwrap();
    assert_ne!(parse_dlimg(&icon).unwrap().0, old_hash);
}

#[tokio::test]
async fn eviction_keeps_failed_removals_accounted_until_retry() {
    let (cache, sid, _dir) = setup(DEFAULT_MAX_BYTES).await;
    let dl = cache
        .register(sid, "https://source.test/art", ImageSize::Grid)
        .await
        .unwrap();
    let (hash, size) = parse_dlimg(&dl).unwrap();
    let stub = Stub {
        calls: AtomicUsize::new(0),
        bytes: png(64, 64, true),
    };
    let image = cache.get(&hash, size, &stub).await.unwrap();
    let path: String = sqlx::query_scalar("SELECT path FROM image_cache WHERE key = ?")
        .bind(ImageCache::key(&hash, size))
        .fetch_one(cache.store.pool())
        .await
        .unwrap();
    let path = cache.dir().join(path);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let cache = cache.with_max_bytes(0);
    assert!(cache.evict().await.is_err());
    assert_eq!(cache.total_bytes().await.unwrap(), image.bytes.len() as u64);
    std::fs::remove_dir(path).unwrap();
    assert_eq!(cache.evict().await.unwrap(), 1);
    assert_eq!(cache.total_bytes().await.unwrap(), 0);
}

#[tokio::test]
async fn reconciliation_removes_old_orphaned_files() {
    let (cache, _sid, _dir) = setup(DEFAULT_MAX_BYTES).await;
    let path = cache.dir().join("ab");
    std::fs::create_dir_all(&path).unwrap();
    let orphan = path.join("old-or-interrupted.part");
    std::fs::write(&orphan, b"orphaned").unwrap();
    sqlx::query("UPDATE image_cache_size SET bytes = 100")
        .execute(cache.store.pool())
        .await
        .unwrap();
    cache.reconcile().await.unwrap();
    assert!(!orphan.exists());
    assert_eq!(cache.total_bytes().await.unwrap(), 0);
}

#[test]
fn rejects_oversized_images_before_resizing() {
    let image = image::RgbImage::new(MAX_IMAGE_DIMENSION + 1, 1);
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    let error = encode_sizes(bytes.get_ref()).unwrap_err();
    assert_eq!(error.message, "That image is too large.");
}
