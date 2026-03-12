//! Species image caching via Wikipedia/Wikimedia Commons.
//!
//! Fetches species thumbnail images from Wikipedia using the `MediaWiki` API.
//! Images are cached on disk to avoid repeated network requests and support
//! air-gapped/offline operation after initial population.
//!
//! Wikipedia is preferred over Flickr because:
//! - No API key required
//! - CC-licensed images
//! - Reliable availability for bird species
//! - Single API for both image URL and species description

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Default request timeout for Wikipedia API.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum retry attempts for failed requests.
const MAX_RETRIES: u32 = 2;

/// Default thumbnail width in pixels.
const DEFAULT_THUMB_WIDTH: u32 = 300;

/// Wikipedia API endpoint (English).
const WIKIPEDIA_API: &str = "https://en.wikipedia.org/w/api.php";

/// Errors from species image operations.
#[derive(Debug)]
pub enum ImageError {
    /// HTTP request failed.
    Http(String),
    /// Wikipedia API returned an error or unexpected response.
    Api(String),
    /// Image not found for species.
    NotFound(String),
    /// I/O error reading/writing cache.
    Io(std::io::Error),
    /// Cache directory not accessible.
    CacheDir(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(msg) => write!(f, "image HTTP error: {msg}"),
            Self::Api(msg) => write!(f, "Wikipedia API error: {msg}"),
            Self::NotFound(species) => write!(f, "no image found for: {species}"),
            Self::Io(e) => write!(f, "image I/O error: {e}"),
            Self::CacheDir(msg) => write!(f, "cache directory error: {msg}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Http(_) | Self::Api(_) | Self::NotFound(_) | Self::CacheDir(_) => None,
        }
    }
}

impl From<std::io::Error> for ImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Metadata about a species image.
#[derive(Debug, Clone)]
pub struct SpeciesImage {
    /// URL of the image on Wikipedia/Wikimedia Commons.
    pub url: String,
    /// Local path to the cached image file (None if not yet cached).
    pub cached_path: Option<PathBuf>,
    /// Image width in pixels (of the thumbnail).
    pub width: u32,
    /// Brief description/extract from Wikipedia (first paragraph).
    pub description: Option<String>,
    /// Wikipedia page URL for the species.
    pub wiki_url: Option<String>,
}

/// Species image cache client.
///
/// Fetches and caches species images from Wikipedia. The cache directory
/// is organized as `{cache_dir}/{scientific_name_safe}.jpg`.
pub struct ImageCache {
    /// HTTP client for Wikipedia API requests.
    http: reqwest::Client,
    /// Path to the on-disk image cache directory.
    cache_dir: PathBuf,
    /// In-memory index of cached species → image metadata.
    /// Avoids filesystem lookups on every request.
    index: Mutex<HashMap<String, SpeciesImage>>,
    /// Thumbnail width to request.
    thumb_width: u32,
}

impl fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageCache")
            .field("cache_dir", &self.cache_dir)
            .field("thumb_width", &self.thumb_width)
            .field(
                "cached_count",
                &self.index.lock().map(|idx| idx.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl ImageCache {
    /// Create a new image cache.
    ///
    /// Creates the cache directory if it doesn't exist. Scans for existing
    /// cached images to build the in-memory index.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the cache directory cannot be created.
    pub fn new(cache_dir: &Path) -> Result<Self, ImageError> {
        std::fs::create_dir_all(cache_dir)?;

        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent("BirdNet-Behavior/0.1 (bird classification system)")
            .build()
            .map_err(|e| ImageError::Http(e.to_string()))?;

        let cache = Self {
            http,
            cache_dir: cache_dir.to_path_buf(),
            index: Mutex::new(HashMap::new()),
            thumb_width: DEFAULT_THUMB_WIDTH,
        };

        // Scan existing cache for pre-populated images
        cache.scan_cache_dir()?;

        Ok(cache)
    }

    /// Create an image cache with a custom thumbnail width.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the cache directory cannot be created.
    pub fn with_thumb_width(cache_dir: &Path, width: u32) -> Result<Self, ImageError> {
        let mut cache = Self::new(cache_dir)?;
        cache.thumb_width = width;
        Ok(cache)
    }

    /// Get the image for a species, fetching from Wikipedia if not cached.
    ///
    /// Uses the scientific name for the Wikipedia lookup (more reliable than
    /// common names which vary by locale).
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the fetch fails and no cached version exists.
    pub async fn get_image(&self, scientific_name: &str) -> Result<SpeciesImage, ImageError> {
        let cache_key = Self::cache_key(scientific_name);

        // Check in-memory index first
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(image) = index.get(&cache_key) {
                return Ok(image.clone());
            }
        }

        // Check disk cache
        let cache_path = self.cache_path(&cache_key);
        if cache_path.exists() {
            let image = SpeciesImage {
                url: String::new(),
                cached_path: Some(cache_path),
                width: self.thumb_width,
                description: None,
                wiki_url: None,
            };
            self.index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(cache_key, image.clone());
            return Ok(image);
        }

        // Fetch from Wikipedia
        let image = self.fetch_from_wikipedia(scientific_name).await?;

        // Update in-memory index
        {
            let mut index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            index.insert(cache_key, image.clone());
        }

        Ok(image)
    }

    /// Check if a species image is cached (no network request).
    pub fn is_cached(&self, scientific_name: &str) -> bool {
        let cache_key = Self::cache_key(scientific_name);

        // Check in-memory index
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if index.contains_key(&cache_key) {
                return true;
            }
        }

        // Check disk
        self.cache_path(&cache_key).exists()
    }

    /// Get cached image metadata without network access.
    ///
    /// Returns `None` if the species is not cached.
    pub fn get_cached(&self, scientific_name: &str) -> Option<SpeciesImage> {
        let cache_key = Self::cache_key(scientific_name);

        let index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        index.get(&cache_key).cloned()
    }

    /// Get the number of cached species images.
    pub fn cached_count(&self) -> usize {
        let index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        index.len()
    }

    /// Get the cache directory path.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Download and cache the image bytes to disk.
    ///
    /// # Errors
    ///
    /// Returns `ImageError` if the download or file write fails.
    pub async fn download_image(
        &self,
        scientific_name: &str,
        image_url: &str,
    ) -> Result<PathBuf, ImageError> {
        let cache_key = Self::cache_key(scientific_name);
        let cache_path = self.cache_path(&cache_key);

        // Download with retry
        let bytes = self.download_with_retry(image_url).await?;

        // Write to cache
        std::fs::write(&cache_path, &bytes)?;

        tracing::debug!(
            species = scientific_name,
            path = %cache_path.display(),
            bytes = bytes.len(),
            "cached species image"
        );

        // Update index with cached path
        {
            let mut index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(entry) = index.get_mut(&cache_key) {
                entry.cached_path = Some(cache_path.clone());
            }
        }

        Ok(cache_path)
    }

    /// Fetch species image metadata from Wikipedia.
    async fn fetch_from_wikipedia(
        &self,
        scientific_name: &str,
    ) -> Result<SpeciesImage, ImageError> {
        // Step 1: Get the page image and extract for the scientific name
        let (image_url, description, wiki_url) = self.query_wikipedia_page(scientific_name).await?;

        let image = SpeciesImage {
            url: image_url,
            cached_path: None,
            width: self.thumb_width,
            description,
            wiki_url,
        };

        Ok(image)
    }

    /// Query the Wikipedia API for page image and extract.
    ///
    /// Uses the `pageimages` and `extracts` properties of the `MediaWiki` API.
    async fn query_wikipedia_page(
        &self,
        scientific_name: &str,
    ) -> Result<(String, Option<String>, Option<String>), ImageError> {
        // Build URL with query parameters manually (reqwest 0.13 query builder
        // requires serde's Serialize on the params type)
        let encoded_title = url_encode(scientific_name);
        let url = format!(
            "{WIKIPEDIA_API}?action=query&format=json&formatversion=2\
             &prop=pageimages%7Cextracts%7Cinfo&titles={encoded_title}\
             &pithumbsize={}&exintro=1&explaintext=1&exsentences=3\
             &inprop=url&redirects=1",
            self.thumb_width
        );

        let mut last_error = ImageError::Http("no attempts made".into());

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(2_u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }

            match self.http.get(&url).send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        last_error = ImageError::Api(format!("HTTP {status}"));
                        continue;
                    }

                    let body = resp
                        .text()
                        .await
                        .map_err(|e| ImageError::Http(e.to_string()))?;

                    let json: serde_json::Value =
                        serde_json::from_str(&body).map_err(|e| ImageError::Api(e.to_string()))?;

                    return Self::parse_wikipedia_response(&json, scientific_name);
                }
                Err(e) => {
                    last_error = ImageError::Http(e.to_string());
                }
            }
        }

        Err(last_error)
    }

    /// Parse the Wikipedia API response to extract image URL, description, and page URL.
    fn parse_wikipedia_response(
        json: &serde_json::Value,
        scientific_name: &str,
    ) -> Result<(String, Option<String>, Option<String>), ImageError> {
        let pages = json
            .get("query")
            .and_then(|q| q.get("pages"))
            .and_then(|p| p.as_array())
            .ok_or_else(|| ImageError::Api("unexpected API response structure".into()))?;

        let page = pages
            .first()
            .ok_or_else(|| ImageError::NotFound(scientific_name.to_string()))?;

        // Check if the page exists (missing field present means no article)
        if page.get("missing").is_some() {
            return Err(ImageError::NotFound(scientific_name.to_string()));
        }

        // Extract thumbnail URL
        let image_url = page
            .get("thumbnail")
            .and_then(|t| t.get("source"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| ImageError::NotFound(scientific_name.to_string()))?
            .to_string();

        // Extract description (first paragraph)
        let description = page
            .get("extract")
            .and_then(|e| e.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Extract canonical URL
        let wiki_url = page
            .get("fullurl")
            .and_then(|u| u.as_str())
            .map(String::from);

        Ok((image_url, description, wiki_url))
    }

    /// Download bytes from a URL with retry.
    async fn download_with_retry(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        let mut last_error = ImageError::Http("no attempts made".into());

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = Duration::from_secs(2_u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }

            match self.http.get(url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        return resp
                            .bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| ImageError::Http(e.to_string()));
                    }
                    let status = resp.status();
                    last_error = ImageError::Http(format!("HTTP {status}"));
                }
                Err(e) => {
                    last_error = ImageError::Http(e.to_string());
                }
            }
        }

        Err(last_error)
    }

    /// Scan the cache directory for existing images and populate the index.
    #[allow(clippy::significant_drop_tightening)]
    fn scan_cache_dir(&self) -> Result<(), ImageError> {
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

            // Reconstruct scientific name from filename
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();

            let image = SpeciesImage {
                url: String::new(),
                cached_path: Some(path),
                width: self.thumb_width,
                description: None,
                wiki_url: None,
            };

            index.insert(stem, image);
            count += 1;
        }

        if count > 0 {
            tracing::info!(count, "loaded cached species images");
        }

        Ok(())
    }

    /// Convert a scientific name to a cache-safe filename key.
    ///
    /// `"Turdus merula"` → `"turdus_merula"`
    fn cache_key(scientific_name: &str) -> String {
        scientific_name.to_lowercase().replace([' ', '/'], "_")
    }

    /// Get the full cache path for a species.
    fn cache_path(&self, cache_key: &str) -> PathBuf {
        self.cache_dir.join(format!("{cache_key}.jpg"))
    }
}

/// Minimal percent-encoding for URL query parameter values.
///
/// Encodes spaces as `%20` and other special characters as needed
/// for safe inclusion in a URL query string.
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push_str("%20"),
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX_CHARS[(byte >> 4) as usize]));
                encoded.push(char::from(HEX_CHARS[(byte & 0x0F) as usize]));
            }
        }
    }
    encoded
}

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_from_scientific_name() {
        assert_eq!(ImageCache::cache_key("Turdus merula"), "turdus_merula");
        assert_eq!(
            ImageCache::cache_key("Erithacus rubecula"),
            "erithacus_rubecula"
        );
        assert_eq!(ImageCache::cache_key("Parus major"), "parus_major");
    }

    #[test]
    fn cache_key_handles_special_chars() {
        assert_eq!(
            ImageCache::cache_key("Corvus corone/cornix"),
            "corvus_corone_cornix"
        );
    }

    #[test]
    fn new_creates_cache_directory() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = ImageCache::new(&dir).unwrap();
        assert!(dir.exists());
        assert_eq!(cache.cached_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_cached_returns_false_for_missing() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache_miss");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = ImageCache::new(&dir).unwrap();
        assert!(!cache.is_cached("Turdus merula"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_finds_existing_images() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache_scan");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a fake cached image
        std::fs::write(dir.join("turdus_merula.jpg"), b"fake-jpeg-data").unwrap();
        std::fs::write(dir.join("parus_major.jpg"), b"fake-jpeg-data").unwrap();

        let cache = ImageCache::new(&dir).unwrap();
        assert_eq!(cache.cached_count(), 2);
        assert!(cache.is_cached("Turdus merula"));
        assert!(cache.is_cached("Parus major"));
        assert!(!cache.is_cached("Erithacus rubecula"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_cached_returns_none_for_missing() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache_get");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = ImageCache::new(&dir).unwrap();
        assert!(cache.get_cached("Turdus merula").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_cached_returns_data_for_existing() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache_get_hit");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join("turdus_merula.jpg"), b"fake-jpeg-data").unwrap();

        let cache = ImageCache::new(&dir).unwrap();
        let image = cache.get_cached("Turdus merula").unwrap();
        assert!(image.cached_path.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_wikipedia_response_success() {
        let json: serde_json::Value = serde_json::json!({
            "query": {
                "pages": [{
                    "pageid": 12345,
                    "title": "Turdus merula",
                    "thumbnail": {
                        "source": "https://upload.wikimedia.org/wikipedia/commons/thumb/a/test.jpg",
                        "width": 300,
                        "height": 200
                    },
                    "extract": "The common blackbird is a species of true thrush.",
                    "fullurl": "https://en.wikipedia.org/wiki/Common_blackbird"
                }]
            }
        });

        let (url, desc, wiki_url) =
            ImageCache::parse_wikipedia_response(&json, "Turdus merula").unwrap();
        assert!(url.contains("wikimedia.org"));
        assert_eq!(
            desc.unwrap(),
            "The common blackbird is a species of true thrush."
        );
        assert_eq!(
            wiki_url.unwrap(),
            "https://en.wikipedia.org/wiki/Common_blackbird"
        );
    }

    #[test]
    fn parse_wikipedia_response_missing_page() {
        let json: serde_json::Value = serde_json::json!({
            "query": {
                "pages": [{
                    "title": "Nonexistent species",
                    "missing": true
                }]
            }
        });

        let result = ImageCache::parse_wikipedia_response(&json, "Nonexistent species");
        assert!(matches!(result, Err(ImageError::NotFound(_))));
    }

    #[test]
    fn parse_wikipedia_response_no_image() {
        let json: serde_json::Value = serde_json::json!({
            "query": {
                "pages": [{
                    "pageid": 12345,
                    "title": "Some page",
                    "extract": "A description without an image."
                }]
            }
        });

        let result = ImageCache::parse_wikipedia_response(&json, "Some species");
        assert!(matches!(result, Err(ImageError::NotFound(_))));
    }

    #[test]
    fn species_image_debug() {
        let image = SpeciesImage {
            url: "https://example.com/test.jpg".into(),
            cached_path: None,
            width: 300,
            description: Some("Test description".into()),
            wiki_url: Some("https://en.wikipedia.org/wiki/Test".into()),
        };
        let debug = format!("{image:?}");
        assert!(debug.contains("SpeciesImage"));
    }

    #[test]
    fn image_error_display() {
        let err = ImageError::NotFound("Turdus merula".into());
        assert_eq!(err.to_string(), "no image found for: Turdus merula");

        let err = ImageError::Http("timeout".into());
        assert_eq!(err.to_string(), "image HTTP error: timeout");

        let err = ImageError::Api("bad response".into());
        assert_eq!(err.to_string(), "Wikipedia API error: bad response");
    }

    #[test]
    fn with_thumb_width() {
        let dir = std::env::temp_dir().join("birdnet_test_image_cache_width");
        let _ = std::fs::remove_dir_all(&dir);

        let cache = ImageCache::with_thumb_width(&dir, 500).unwrap();
        assert_eq!(cache.thumb_width, 500);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_encode_plain_text() {
        assert_eq!(url_encode("hello"), "hello");
    }

    #[test]
    fn url_encode_spaces() {
        assert_eq!(url_encode("Turdus merula"), "Turdus%20merula");
    }

    #[test]
    fn url_encode_special_chars() {
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }
}
