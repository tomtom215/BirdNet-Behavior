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

use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Maximum number of bytes accepted for a single image download.
///
/// `bytes()` reads the whole response body with no cap, so a poisoned or
/// runaway upstream (or a Wikipedia thumbnail URL someone replaced with a huge
/// asset) could OOM the Pi. A few MB is more than enough for any thumbnail —
/// `Special:FilePath` thumbnails are typically under 200 KB.
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// How long a failed lookup is remembered before the provider is asked again.
///
/// Without this, a species the provider has no picture for costs a live API
/// round trip *every time its image URL is requested*, because nothing is
/// written to disk on a miss and `DiskCache::get` therefore keeps returning
/// `None`. That was tolerable while the only `<img>` tags were on the species
/// gallery and the detail page, which a reader opens deliberately. It is not
/// tolerable now the avatar in every detection row carries a photo: the live
/// feed re-renders on a timer, and a station with thirty photo-less species
/// would have asked Wikipedia about all thirty of them on every poll, for as
/// long as it was switched on.
///
/// Fifteen minutes bounds a permanent miss to four lookups an hour per
/// species. Only a provider that *answered*, saying it has no picture, is
/// remembered this long — see [`TRANSIENT_MISS_TTL`].
const MISS_TTL: Duration = Duration::from_secs(15 * 60);

/// How long a *transient* failure is remembered: a timeout, a rate limit, a
/// station that was briefly offline.
///
/// Distinguishing the two matters, and the visual-QA sweep is what showed it.
/// With one TTL for both, five species whose fetch happened to fail during a
/// CI run were locked out of every page for the rest of it, and the sweep
/// reported thirty-six pages of broken images — for birds whose photographs
/// were perfectly available a minute later. A provider saying "no image on
/// this species page" is a durable fact; an HTTP error is not, and pretending
/// otherwise turns a blip into a quarter of an hour of letter tiles.
///
/// A minute is still long enough to stop a polling feed re-asking on every
/// render, which is the whole reason either of these exists.
const TRANSIENT_MISS_TTL: Duration = Duration::from_secs(60);

/// Maximum number of remembered failed lookups.
///
/// The key comes from a URL path segment (`/api/v2/species/image/{name}/file`)
/// and nothing upstream checks it against the label set, so this map's key
/// space is whatever a caller asks for rather than whatever the station has
/// heard. A real station only ever populates it with species on its own life
/// list that have no picture, which is a handful; the cap is for the other
/// case. Over it, expired entries go first and the write is dropped only if
/// that was not enough.
const MAX_MISSES: usize = 4096;

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
    /// Species the provider has already failed to supply, and when it said so.
    /// See [`MISS_TTL`] for why this exists.
    ///
    /// Behind an `Arc` like the two fields above it, and for the same reason:
    /// the type derives `Clone`, and a clone with its own map would forget
    /// every miss the original had recorded.
    misses: Arc<Mutex<HashMap<String, (Instant, Duration)>>>,
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
            misses: Arc::new(Mutex::new(HashMap::new())),
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

        // Second fast path: the provider has already said it has nothing for
        // this species, recently enough to believe. A miss writes no file, so
        // without this the disk check above can never short-circuit it.
        if self.miss_is_fresh(&key) {
            return Err(ImageError::NotFound(scientific_name.to_string()));
        }

        // Slow path: fetch from provider.
        let mut img = match self.provider.fetch(scientific_name).await {
            Ok(img) => img,
            Err(e) => {
                self.record_miss(&key, &e);
                return Err(e);
            }
        };

        // Download and store the image bytes. Uses a User-Agent'd client
        // (Wikimedia rejects anonymous requests) and only caches a genuine
        // image so an error page can't poison the on-disk cache.
        if !img.url.is_empty() {
            let download = async {
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
                read_capped_image_bytes(resp).await
            };
            // A download that fails is as much a miss as a lookup that finds
            // nothing: it leaves no file on disk, so the next request would
            // otherwise repeat the whole round trip.
            let bytes = match download.await {
                Ok(bytes) => bytes,
                Err(e) => {
                    self.record_miss(&key, &e);
                    return Err(e);
                }
            };
            let path = self.disk.store(&key, &bytes)?;
            img.cached_path = Some(path);
        }

        self.disk.update_metadata(&key, &img);
        Ok(img)
    }

    /// `true` when this species was recently looked up and came back empty.
    fn miss_is_fresh(&self, key: &str) -> bool {
        self.misses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .is_some_and(|(at, ttl)| at.elapsed() < *ttl)
    }

    /// How long `error` is worth remembering.
    ///
    /// [`ImageError::NotFound`] is the provider answering: this species has no
    /// picture, and it will not have one a minute from now. Everything else is
    /// the network.
    const fn miss_ttl(error: &ImageError) -> Duration {
        match error {
            ImageError::NotFound(_) => MISS_TTL,
            _ => TRANSIENT_MISS_TTL,
        }
    }

    /// Remember that this species has no image, so the provider is not asked
    /// again for [`MISS_TTL`] or [`TRANSIENT_MISS_TTL`].
    fn record_miss(&self, key: &str, error: &ImageError) {
        let ttl = Self::miss_ttl(error);
        let mut misses = self
            .misses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if misses.len() >= MAX_MISSES && !misses.contains_key(key) {
            misses.retain(|_, (at, ttl)| at.elapsed() < *ttl);
            if misses.len() >= MAX_MISSES {
                return;
            }
        }
        misses.insert(key.to_string(), (Instant::now(), ttl));
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
        let key = Self::cache_key(scientific_name);
        self.misses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key);
        self.disk.remove(&key)
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
            misses: Arc::new(Mutex::new(HashMap::new())),
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
            misses: Arc::new(Mutex::new(HashMap::new())),
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

    /// `NullProvider` that counts how many times it was asked.
    struct CountingProvider(Arc<std::sync::atomic::AtomicUsize>);
    impl ImageProvider for CountingProvider {
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
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let name = scientific_name.to_string();
            Box::pin(async move { Err(ImageError::NotFound(name)) })
        }
    }

    fn counting_cache(dir: &std::path::Path) -> (ImageCache, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cache = ImageCache {
            provider: Arc::new(CountingProvider(Arc::clone(&calls))),
            disk: Arc::new(DiskCache::new(dir, 300).unwrap()),
            misses: Arc::new(Mutex::new(HashMap::new())),
        };
        (cache, calls)
    }

    /// A species the provider has nothing for writes no file, so nothing in
    /// `DiskCache` can ever short-circuit the next request for it. Before
    /// [`MISS_TTL`] existed, that meant one live API round trip per `<img>`
    /// render — and the avatar in every detection row now carries an `<img>`
    /// that the live feed re-renders on a timer.
    #[tokio::test]
    async fn a_species_with_no_photo_is_asked_about_once() {
        let dir = tmpdir("miss_once");
        let (cache, calls) = counting_cache(&dir);

        for _ in 0..5 {
            assert!(cache.get_image("Turdus merula").await.is_err());
        }

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "five requests for the same photo-less species must reach the \
             provider once"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The counterpart: remembering a miss must not turn into never asking.
    /// Without this, `get_image` returning `Err` unconditionally would pass
    /// the test above.
    #[tokio::test]
    async fn a_species_not_yet_asked_about_still_reaches_the_provider() {
        let dir = tmpdir("miss_per_species");
        let (cache, calls) = counting_cache(&dir);

        for sci in ["Turdus merula", "Parus major", "Pica pica"] {
            assert!(cache.get_image(sci).await.is_err());
        }

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "each new species must be looked up on its own"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A provider that answered "no picture" is remembered fifteen minutes; a
    /// network failure, one. Checked at the unit that decides, because the
    /// difference is only observable fourteen minutes later otherwise.
    #[test]
    fn a_network_failure_is_forgotten_sooner_than_a_missing_photo() {
        assert_eq!(
            ImageCache::miss_ttl(&ImageError::NotFound("Turdus merula".into())),
            MISS_TTL,
            "a provider saying the species has no picture is a durable fact"
        );
        for transient in [
            ImageError::Http("timed out".into()),
            ImageError::Api("429".into()),
        ] {
            assert_eq!(
                ImageCache::miss_ttl(&transient),
                TRANSIENT_MISS_TTL,
                "a blip must not cost a quarter of an hour of letter tiles"
            );
        }
        assert!(
            TRANSIENT_MISS_TTL < MISS_TTL,
            "the whole point is that one is shorter"
        );
        assert!(
            TRANSIENT_MISS_TTL >= Duration::from_secs(30),
            "still long enough that a polling feed does not re-ask per render"
        );
    }

    /// Blacklisting a species evicts it so the next request re-resolves. A
    /// remembered miss would defeat that, because the re-fetch never happens.
    #[tokio::test]
    async fn evicting_a_species_forgets_that_it_had_no_photo() {
        let dir = tmpdir("miss_forgotten_on_remove");
        let (cache, calls) = counting_cache(&dir);

        assert!(cache.get_image("Turdus merula").await.is_err());
        cache.remove("Turdus merula");
        assert!(cache.get_image("Turdus merula").await.is_err());

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "an eviction must clear the remembered miss as well as the file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
