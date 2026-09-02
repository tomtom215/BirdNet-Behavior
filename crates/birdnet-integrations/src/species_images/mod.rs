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
pub mod chain;
pub mod flickr;
pub mod provider;
pub mod types;
pub mod wikipedia;

pub use cache::DiskCache;
pub use chain::FallbackProvider;
pub use flickr::FlickrClient;
pub use provider::ImageProvider;
pub use types::{ImageError, SpeciesImage};
pub use wikipedia::WikipediaClient;

use std::fmt;
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// Maximum number of bytes accepted for a single image download.
///
/// `bytes()` reads the whole response body with no cap, so a poisoned or
/// runaway upstream (or a Wikipedia thumbnail URL someone replaced with a huge
/// asset) could OOM the Pi. A few MB is more than enough for any thumbnail —
/// `Special:FilePath` thumbnails are typically under 200 KB.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Download a response body with a hard byte cap so a poisoned image URL can't
/// exhaust memory. Honours `Content-Length` up front when present and bounds
/// the streamed read regardless (the header can lie). Uses `Response::chunk`
/// to avoid pulling `futures_util` for a Stream wrapper.
pub(super) async fn read_capped_image_bytes(
    mut resp: reqwest::Response,
) -> Result<Vec<u8>, ImageError> {
    if let Some(len) = resp.content_length()
        && len > MAX_IMAGE_BYTES as u64
    {
        return Err(ImageError::Http(format!(
            "image download exceeds {MAX_IMAGE_BYTES}-byte cap (Content-Length: {len})"
        )));
    }
    let mut buf = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| ImageError::Http(e.to_string()))?
    {
        if buf.len().saturating_add(chunk.len()) > MAX_IMAGE_BYTES {
            return Err(ImageError::Http(format!(
                "image download exceeds {MAX_IMAGE_BYTES}-byte cap"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// User-Agent for image-byte downloads. Wikimedia rejects requests without a
/// descriptive User-Agent (returning a short policy notice instead of the
/// image — see <https://phabricator.wikimedia.org/T400119>), so the download
/// client must identify the application, matching the provider's API client.
const IMAGE_DOWNLOAD_USER_AGENT: &str =
    "BirdNet-Behavior/0.2 (+https://github.com/tomtom215/BirdNet-Behavior)";

/// Shared, lazily-built HTTP client for downloading image bytes. Carries the
/// User-Agent Wikimedia requires and a bounded timeout.
fn image_download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(IMAGE_DOWNLOAD_USER_AGENT)
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default()
    })
}

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
        Self::new(cache_dir, Arc::new(client), wikipedia::DEFAULT_THUMB_WIDTH)
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

    /// Build the provider a configuration asks for, and the cache around it.
    ///
    /// `provider` is `"flickr"` or anything else, which means Wikipedia — the
    /// default has to be the one that needs no key, so a station with a typo in
    /// this setting still shows photographs.
    ///
    /// Choosing Flickr gives a *chain*, not a replacement: Flickr first,
    /// Wikipedia behind it. See [`chain`] for why "choose one" is the wrong
    /// shape, and note that the cache key is the species name alone, so a
    /// station that switches provider keeps every image it has already
    /// downloaded rather than re-fetching nine thousand thumbnails.
    ///
    /// # Errors
    ///
    /// [`ImageError::Api`] when Flickr is selected without a usable key — a
    /// failure the operator can act on, rather than a station that silently
    /// shows nothing. [`ImageError::Http`] or [`ImageError::CacheDir`] if the
    /// HTTP client or the cache directory cannot be created.
    pub fn from_settings(
        cache_dir: &Path,
        provider: &str,
        flickr_api_key: Option<&str>,
        flickr_filter_email: Option<&str>,
        thumb_width: u32,
    ) -> Result<Self, ImageError> {
        if !provider.trim().eq_ignore_ascii_case("flickr") {
            return Self::new(
                cache_dir,
                Arc::new(WikipediaClient::with_thumb_width(thumb_width)?),
                thumb_width,
            );
        }
        let key = flickr_api_key.unwrap_or_default();
        let mut flickr = FlickrClient::new(key)?.with_thumb_width(thumb_width);
        if let Some(email) = flickr_filter_email {
            flickr = flickr.with_filter_email(email);
        }
        let chained = FallbackProvider::new(
            Box::new(flickr),
            Box::new(WikipediaClient::with_thumb_width(thumb_width)?),
        );
        Self::new(cache_dir, Arc::new(chained), thumb_width)
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

        // Download and store the image bytes. Uses a User-Agent'd client
        // (Wikimedia rejects anonymous requests) and only caches a genuine
        // image so an error page can't poison the on-disk cache.
        if !img.url.is_empty() {
            let resp = image_download_client()
                .get(&img.url)
                .send()
                .await
                .map_err(|e| ImageError::Http(e.to_string()))?
                .error_for_status()
                .map_err(|e| ImageError::Http(e.to_string()))?;
            let is_image = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.starts_with("image/"));
            if !is_image {
                return Err(ImageError::Http(format!(
                    "image download for '{scientific_name}' did not return an image"
                )));
            }
            let bytes = read_capped_image_bytes(resp).await?;
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

    /// Evict a species image from the cache (disk file + in-memory entry).
    ///
    /// Returns `true` if a cached file was deleted. Used when an image is
    /// blacklisted so it is no longer served and is re-fetched on next request.
    pub fn remove(&self, scientific_name: &str) -> bool {
        self.disk.remove(&Self::cache_key(scientific_name))
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

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("birdnet_imagecache_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The default has to be the source that needs no key, so a station with a
    /// typo in `IMAGE_PROVIDER` still shows photographs instead of nothing.
    #[test]
    fn an_unrecognised_provider_falls_back_to_wikipedia() {
        for name in ["wikipedia", "", "  ", "flickerr", "WIKIPEDIA"] {
            assert!(
                ImageCache::from_settings(&tmpdir("prov"), name, None, None, 300).is_ok(),
                "{name:?} should build a working cache"
            );
        }
    }

    /// Flickr selected without a key is an error the operator can act on, not
    /// a station that quietly shows nothing on every species page.
    #[test]
    fn flickr_without_a_key_is_refused_rather_than_silently_empty() {
        let err = ImageCache::from_settings(&tmpdir("nokey"), "flickr", None, None, 300)
            .expect_err("no key must be refused");
        assert!(
            err.to_string().contains("FLICKR_API_KEY"),
            "and name the setting to fix: {err}"
        );
        assert!(
            ImageCache::from_settings(&tmpdir("key"), "flickr", Some("k"), None, 300).is_ok(),
            "the same request with a key builds"
        );
    }

    /// Case and surrounding whitespace in a hand-edited config file must not
    /// decide whether the operator gets the provider they asked for.
    #[test]
    fn the_provider_name_is_read_leniently() {
        for name in ["flickr", "Flickr", "FLICKR", " flickr "] {
            assert!(
                ImageCache::from_settings(&tmpdir("case"), name, Some("k"), None, 300).is_ok(),
                "{name:?} should select Flickr"
            );
            // ...and it really did select Flickr: without a key the same name
            // is refused, which only the Flickr branch does.
            assert!(
                ImageCache::from_settings(&tmpdir("case2"), name, None, None, 300).is_err(),
                "{name:?} did not reach the Flickr branch"
            );
        }
    }

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
