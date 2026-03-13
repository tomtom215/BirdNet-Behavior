//! Species image caching via Wikipedia/Wikimedia Commons.
//!
//! Downloads and caches bird species thumbnail images, supporting offline
//! operation after initial population. The design is provider-agnostic:
//! `ImageCache` delegates fetching to any `ImageProvider` implementation
//! so that Wikipedia can be replaced with Flickr, eBird, or a custom source
//! without touching cache logic.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use birdnet_integrations::species_images::{ImageCache, WikipediaClient};
//! use std::path::Path;
//!
//! # async fn example() {
//! let cache = ImageCache::with_wikipedia(Path::new("/var/cache/birdnet/images")).unwrap();
//! let img = cache.get_image("Turdus merula").await.unwrap();
//! println!("image URL: {}", img.url);
//! # }
//! ```
//!
//! # Module layout
//!
//! | Sub-module   | Contents                                             |
//! |--------------|------------------------------------------------------|
//! | `types`      | `ImageError`, `SpeciesImage`                         |
//! | `provider`   | `ImageProvider` trait                                |
//! | `wikipedia`  | `WikipediaClient` implementing `ImageProvider`       |
//! | `cache`      | `DiskCache` — on-disk image storage and indexing     |

pub mod cache;
pub mod provider;
pub mod types;
pub mod wikipedia;

pub use cache::DiskCache;
pub use provider::ImageProvider;
pub use types::{ImageError, SpeciesImage};
pub use wikipedia::WikipediaClient;

use std::fmt;
use std::path::Path;
use std::sync::Arc;

/// Coordinating cache: fetches from a remote `ImageProvider` and stores
/// images locally via `DiskCache`.
///
/// `ImageCache` is `Clone + Send + Sync` because it stores its state behind
/// an `Arc`. A single instance is shared across all request handlers.
#[derive(Clone)]
pub struct ImageCache {
    provider: Arc<dyn ImageProvider>,
    disk: Arc<DiskCache>,
}

impl fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageCache")
            .field("cached_count", &self.disk.len())
            .finish_non_exhaustive()
    }
}

impl ImageCache {
    /// Create a new `ImageCache` backed by the given `provider`.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the cache directory cannot be created.
    pub fn new(
        cache_dir: &Path,
        provider: Arc<dyn ImageProvider>,
        thumb_width: u32,
    ) -> Result<Self, ImageError> {
        let disk = DiskCache::new(cache_dir, thumb_width)?;
        Ok(Self {
            provider,
            disk: Arc::new(disk),
        })
    }

    /// Create a new `ImageCache` using the default `WikipediaClient`.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the HTTP client or cache directory cannot be created.
    pub fn with_wikipedia(cache_dir: &Path) -> Result<Self, ImageError> {
        let client = WikipediaClient::new()?;
        Self::new(
            cache_dir,
            Arc::new(client),
            wikipedia::DEFAULT_THUMB_WIDTH,
        )
    }

    /// Create a `WikipediaClient`-backed cache with a custom thumbnail width.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the HTTP client or cache directory cannot be created.
    pub fn with_wikipedia_and_width(cache_dir: &Path, width: u32) -> Result<Self, ImageError> {
        let client = WikipediaClient::with_thumb_width(width)?;
        Self::new(cache_dir, Arc::new(client), width)
    }

    /// Get the image for a species, fetching from the provider if not cached.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the fetch fails and no cached version exists.
    pub async fn get_image(&self, scientific_name: &str) -> Result<SpeciesImage, ImageError> {
        let key = Self::cache_key(scientific_name);

        // Fast path: in-memory index / disk hit.
        if let Some(img) = self.disk.get(&key) {
            return Ok(img);
        }

        // Slow path: fetch from provider.
        let mut img = self.provider.fetch(scientific_name).await?;

        // Download and store the image bytes.
        if !img.url.is_empty() {
            // Re-use the provider's HTTP client for the download if it's Wikipedia.
            // For a generic provider we'd need a separate client; use a simple reqwest call.
            let bytes = reqwest::get(&img.url)
                .await
                .map_err(|e| ImageError::Http(e.to_string()))?
                .bytes()
                .await
                .map_err(|e| ImageError::Http(e.to_string()))?;

            let path = self.disk.store(&key, &bytes)?;
            img.cached_path = Some(path);
        }

        self.disk.update_metadata(&key, &img);
        Ok(img)
    }

    /// Return `true` if the species image is already cached on disk.
    pub fn is_cached(&self, scientific_name: &str) -> bool {
        self.disk.contains(&Self::cache_key(scientific_name))
    }

    /// Return cached metadata without making a network request.
    ///
    /// Returns `None` if the species is not cached.
    pub fn get_cached(&self, scientific_name: &str) -> Option<SpeciesImage> {
        self.disk.get(&Self::cache_key(scientific_name))
    }

    /// Number of cached species images.
    pub fn cached_count(&self) -> usize {
        self.disk.len()
    }

    /// Root cache directory.
    pub fn cache_dir(&self) -> &Path {
        self.disk.dir()
    }

    /// Compute the cache key for a scientific name.
    ///
    /// `"Turdus merula"` → `"turdus_merula"`
    pub fn cache_key(scientific_name: &str) -> String {
        scientific_name.to_lowercase().replace([' ', '/'], "_")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_lowercases_and_normalises() {
        assert_eq!(ImageCache::cache_key("Turdus merula"), "turdus_merula");
        assert_eq!(
            ImageCache::cache_key("Corvus corone/cornix"),
            "corvus_corone_cornix"
        );
    }

    #[test]
    fn is_cached_false_for_new_cache() {
        let dir = std::env::temp_dir().join("birdnet_imagecache_new");
        let _ = std::fs::remove_dir_all(&dir);
        // Construct a test-only cache using a dummy DiskCache (no network).
        let disk = DiskCache::new(&dir, 300).unwrap();
        let cache = ImageCache {
            provider: Arc::new(NullProvider),
            disk: Arc::new(disk),
        };
        assert!(!cache.is_cached("Turdus merula"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_cached_true_after_pre_populating() {
        let dir = std::env::temp_dir().join("birdnet_imagecache_populated");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("turdus_merula.jpg"), b"data").unwrap();
        let disk = DiskCache::new(&dir, 300).unwrap();
        let cache = ImageCache {
            provider: Arc::new(NullProvider),
            disk: Arc::new(disk),
        };
        assert!(cache.is_cached("Turdus merula"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A no-op provider for unit tests that must never hit the network.
    struct NullProvider;
    impl ImageProvider for NullProvider {
        fn fetch<'life0, 'life1, 'async_trait>(
            &'life0 self,
            scientific_name: &'life1 str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<SpeciesImage, ImageError>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            let name = scientific_name.to_string();
            Box::pin(async move { Err(ImageError::NotFound(name)) })
        }
    }
}
