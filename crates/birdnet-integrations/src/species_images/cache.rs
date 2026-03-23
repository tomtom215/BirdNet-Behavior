//! On-disk species image cache.
//!
//! `DiskCache` stores downloaded species images as `{cache_dir}/{key}.jpg`
//! and maintains an in-memory index to avoid repeated filesystem lookups.
//! It is intentionally separated from the HTTP fetching logic so that
//! provider implementations remain independently testable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::types::{ImageError, SpeciesImage};

/// On-disk image cache with an in-memory lookup index.
#[derive(Debug)]
pub struct DiskCache {
    /// Root directory for cached images.
    cache_dir: PathBuf,
    /// `{cache_key} → SpeciesImage` index (populated on construction and on write).
    index: Mutex<HashMap<String, SpeciesImage>>,
    /// Thumbnail width recorded for newly created entries.
    thumb_width: u32,
}

impl DiskCache {
    /// Create (or re-open) a cache rooted at `cache_dir`.
    ///
    /// Scans the directory for existing `.jpg`/`.jpeg`/`.png`/`.webp` files
    /// and populates the in-memory index so the first `get` is always fast.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the directory cannot be created or read.
    pub fn new(cache_dir: &Path, thumb_width: u32) -> Result<Self, ImageError> {
        std::fs::create_dir_all(cache_dir)?;
        let cache = Self {
            cache_dir: cache_dir.to_path_buf(),
            index: Mutex::new(HashMap::new()),
            thumb_width,
        };
        cache.scan()?;
        Ok(cache)
    }

    /// Look up a species image by cache key (already lowercased/normalised).
    ///
    /// Returns `None` when not present in the index or on disk.
    pub fn get(&self, cache_key: &str) -> Option<SpeciesImage> {
        // Check index first.
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(img) = index.get(cache_key) {
                return Some(img.clone());
            }
        }
        // Fall back to disk check.
        let path = self.path_for(cache_key);
        if path.exists() {
            let img = SpeciesImage {
                url: String::new(),
                cached_path: Some(path),
                width: self.thumb_width,
                description: None,
                wiki_url: None,
            };
            self.index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(cache_key.to_string(), img.clone());
            return Some(img);
        }
        None
    }

    /// Return `true` if a cached image file exists for `cache_key`.
    pub fn contains(&self, cache_key: &str) -> bool {
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if index.contains_key(cache_key) {
                return true;
            }
        }
        self.path_for(cache_key).exists()
    }

    /// Write raw image bytes to disk and update the index.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::Io` on write failure.
    #[allow(clippy::significant_drop_tightening)]
    pub fn store(&self, cache_key: &str, bytes: &[u8]) -> Result<PathBuf, ImageError> {
        let path = self.path_for(cache_key);
        std::fs::write(&path, bytes)?;
        {
            let mut index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = index
                .entry(cache_key.to_string())
                .or_insert_with(|| SpeciesImage {
                    url: String::new(),
                    cached_path: None,
                    width: self.thumb_width,
                    description: None,
                    wiki_url: None,
                });
            entry.cached_path = Some(path.clone());
        }
        Ok(path)
    }

    /// Update the remote URL and metadata for an entry already in the index.
    ///
    /// Called after a successful fetch to persist the URL alongside the path.
    #[allow(clippy::significant_drop_tightening)]
    pub fn update_metadata(&self, cache_key: &str, img: &SpeciesImage) {
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = index
            .entry(cache_key.to_string())
            .or_insert_with(|| img.clone());
        entry.url.clone_from(&img.url);
        entry.description.clone_from(&img.description);
        entry.wiki_url.clone_from(&img.wiki_url);
        if entry.cached_path.is_none() {
            entry.cached_path.clone_from(&img.cached_path);
        }
    }

    /// Number of cached species images in the index.
    pub fn len(&self) -> usize {
        self.index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// `true` if no images are cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Root cache directory.
    pub fn dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Full path for a given cache key.
    fn path_for(&self, cache_key: &str) -> PathBuf {
        self.cache_dir.join(format!("{cache_key}.jpg"))
    }

    /// Scan the cache directory and populate the index.
    #[allow(clippy::significant_drop_tightening)]
    fn scan(&self) -> Result<(), ImageError> {
        let entries = match std::fs::read_dir(&self.cache_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(ImageError::Io(e)),
        };

        let mut count = 0_u32;
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let is_image = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("jpg")
                        || ext.eq_ignore_ascii_case("jpeg")
                        || ext.eq_ignore_ascii_case("png")
                        || ext.eq_ignore_ascii_case("webp")
                });
            if !is_image {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            index.insert(
                stem,
                SpeciesImage {
                    url: String::new(),
                    cached_path: Some(path),
                    width: self.thumb_width,
                    description: None,
                    wiki_url: None,
                },
            );
            count += 1;
        }
        if count > 0 {
            tracing::info!(count, "loaded cached species images");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_directory() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_new");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        assert!(dir.exists());
        assert!(cache.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn contains_false_for_missing() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_miss");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        assert!(!cache.contains("turdus_merula"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_and_get_round_trip() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_store");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        let path = cache.store("turdus_merula", b"fake-jpeg").unwrap();
        assert!(path.exists());
        let img = cache.get("turdus_merula").unwrap();
        assert!(img.cached_path.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_finds_existing_files() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_scan");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("turdus_merula.jpg"), b"data").unwrap();
        std::fs::write(dir.join("parus_major.jpg"), b"data").unwrap();
        let cache = DiskCache::new(&dir, 300).unwrap();
        assert_eq!(cache.len(), 2);
        assert!(cache.contains("turdus_merula"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
