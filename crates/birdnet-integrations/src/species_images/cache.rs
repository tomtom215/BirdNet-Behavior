//! On-disk species image cache.
//!
//! `DiskCache` stores downloaded species images as `{cache_dir}/{key}.jpg`
//! and maintains an in-memory index to avoid repeated filesystem lookups.
//! It is intentionally separated from the HTTP fetching logic so that
//! provider implementations remain independently testable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::types::{ImageCredit, ImageError, SpeciesImage};

/// Maximum number of cached species images (disk files + index entries).
///
/// Prevents unbounded disk growth during long field deployments. With ~50 KB
/// per thumbnail, 2000 entries ≈ 100 MB — well within SD card budgets.
const DEFAULT_MAX_ENTRIES: usize = 2000;

/// Whether `key` is one file name, so that `{cache_dir}/{key}.jpg` and
/// `{cache_dir}/{key}.credit.json` can only be files directly in the cache directory.
///
/// A key is derived from the species name in a request path, and the cache is public API:
/// it checks the key itself rather than trusting whoever built it. Punctuation is allowed
/// (`larus_sp.`, `anas_(domestic)`); a separator, `..` anywhere in the key, `.` and the empty
/// name are not.
///
/// Every public method that takes a key also starts with `key.contains("..")` written out in
/// place. That is the one check of a path's text that CodeQL's `rust/path-injection` query
/// recognises as a guard (it reads the receiver of a `contains` with a literal `".."`, and does
/// not look through a helper such as this one), so the scanner sees the check this function
/// makes.
fn is_plain_name(key: &str) -> bool {
    !key.contains('\0')
        && !key.contains("..")
        && Path::new(key).file_name().is_some_and(|name| name == key)
}

/// The error for a key that is not a single file name.
fn not_a_name(key: &str) -> ImageError {
    ImageError::CacheDir(format!("{key:?} is not a single file name"))
}

/// On-disk image cache with an in-memory lookup index.
#[derive(Debug)]
pub struct DiskCache {
    /// Root directory for cached images.
    cache_dir: PathBuf,
    /// `{cache_key} → SpeciesImage` index (populated on construction and on write).
    index: Mutex<HashMap<String, SpeciesImage>>,
    /// Thumbnail width recorded for newly created entries.
    thumb_width: u32,
    /// Maximum number of cached entries.
    max_entries: usize,
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
            max_entries: DEFAULT_MAX_ENTRIES,
        };
        cache.scan()?;
        Ok(cache)
    }

    /// Look up a species image by cache key (already lowercased/normalised).
    ///
    /// Returns `None` when not present in the index or on disk.
    pub fn get(&self, cache_key: &str) -> Option<SpeciesImage> {
        if cache_key.contains("..") {
            return None;
        }
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
        let path = self.path_for(cache_key)?;
        if path.exists() {
            let img = SpeciesImage {
                url: String::new(),
                cached_path: Some(path),
                width: self.thumb_width,
                description: None,
                wiki_url: None,
                credit: self.read_credit(cache_key),
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
        if cache_key.contains("..") {
            return false;
        }
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if index.contains_key(cache_key) {
                return true;
            }
        }
        self.path_for(cache_key).is_some_and(|path| path.exists())
    }

    /// Write raw image bytes to disk and update the index.
    ///
    /// A cache at capacity (`max_entries`) refuses a new entry and writes
    /// nothing. It used to write the file and leave it out of the index, which
    /// bounded the index and not the disk — the disk the recordings live on.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::Io` on write failure, and `ImageError::CacheDir`
    /// when the cache is full or `cache_key` is not a single file name.
    #[allow(clippy::significant_drop_tightening)]
    pub fn store(&self, cache_key: &str, bytes: &[u8]) -> Result<PathBuf, ImageError> {
        if cache_key.contains("..") {
            return Err(not_a_name(cache_key));
        }
        let path = self
            .path_for(cache_key)
            .ok_or_else(|| not_a_name(cache_key))?;

        // Check capacity before writing (don't grow the index beyond max_entries).
        {
            let index = self
                .index
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if index.len() >= self.max_entries && !index.contains_key(cache_key) {
                tracing::debug!(
                    max = self.max_entries,
                    key = cache_key,
                    "image cache at capacity, not storing"
                );
                return Err(ImageError::CacheDir(format!(
                    "image cache is full ({} entries)",
                    self.max_entries
                )));
            }
        }

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
                    credit: None,
                });
            entry.cached_path = Some(path.clone());
        }
        Ok(path)
    }

    /// Remove a cached image: delete the file from disk and drop the index
    /// entry. Used when an admin blacklists a species image so the next request
    /// re-fetches (and is then refused if the resolved URL is still blacklisted).
    ///
    /// Returns `true` if a file was deleted. A missing file is not an error.
    pub fn remove(&self, cache_key: &str) -> bool {
        if cache_key.contains("..") {
            return false;
        }
        self.index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(cache_key);
        // The credit goes with the picture: a credit left behind would be put on the next
        // picture stored under this name, which is somebody else's.
        if let Some(credit) = self.existing(&format!("{cache_key}.credit.json")) {
            let _ = std::fs::remove_file(credit);
        }
        let Some(path) = self.existing(&format!("{cache_key}.jpg")) else {
            return false;
        };
        match std::fs::remove_file(&path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                tracing::warn!(key = cache_key, error = %e, "failed to remove cached image");
                false
            }
        }
    }

    /// Update the remote URL and metadata for an entry already in the index.
    ///
    /// Called after a successful fetch to persist the URL alongside the path.
    #[allow(clippy::significant_drop_tightening)]
    pub fn update_metadata(&self, cache_key: &str, img: &SpeciesImage) {
        if cache_key.contains("..") {
            return;
        }
        if !is_plain_name(cache_key) {
            return;
        }
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
        entry.credit.clone_from(&img.credit);
        if entry.cached_path.is_none() {
            entry.cached_path.clone_from(&img.cached_path);
        }
        drop(index);
        match &img.credit {
            Some(credit) => self.write_credit(cache_key, credit),
            None => {
                if let Some(path) = self.credit_path_for(cache_key) {
                    let _ = std::fs::remove_file(path);
                }
            }
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

    /// Full path for a given cache key, or `None` when the key is not a single file name.
    fn path_for(&self, cache_key: &str) -> Option<PathBuf> {
        is_plain_name(cache_key).then(|| self.cache_dir.join(format!("{cache_key}.jpg")))
    }

    /// The file called `name` in the cache directory, if there is one, found by listing the
    /// directory and comparing names rather than by building a path from `name`: what `remove`
    /// deletes is a file that is already in this directory whatever `name` holds.
    fn existing(&self, name: &str) -> Option<PathBuf> {
        std::fs::read_dir(&self.cache_dir)
            .ok()?
            .flatten()
            .find(|entry| entry.file_name() == name)
            .map(|entry| entry.path())
    }

    /// Where the credit for `cache_key` is kept: beside the picture, as `{key}.credit.json`.
    fn credit_path_for(&self, cache_key: &str) -> Option<PathBuf> {
        is_plain_name(cache_key).then(|| self.cache_dir.join(format!("{cache_key}.credit.json")))
    }

    /// The credit kept beside a picture. `None` when there is none, or it is not readable: a
    /// picture whose credit cannot be read is treated as one that has none, and is replaced.
    fn read_credit(&self, cache_key: &str) -> Option<ImageCredit> {
        let text = std::fs::read_to_string(self.credit_path_for(cache_key)?).ok()?;
        let credit: ImageCredit = serde_json::from_str(&text).ok()?;
        credit.is_attributable().then_some(credit)
    }

    /// Keep `credit` beside the picture, written to a temporary name and renamed into place so
    /// a power cut leaves the old credit or the new one and never half of one.
    fn write_credit(&self, cache_key: &str, credit: &ImageCredit) {
        let Some(path) = self.credit_path_for(cache_key) else {
            return;
        };
        let part = path.with_extension("json.part");
        let result = serde_json::to_vec_pretty(credit)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&part, bytes))
            .and_then(|()| std::fs::rename(&part, &path));
        if let Err(e) = result {
            tracing::warn!(key = cache_key, error = %e, "could not keep the picture's credit");
        }
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
            let credit = self.read_credit(&stem);
            index.insert(
                stem,
                SpeciesImage {
                    url: String::new(),
                    cached_path: Some(path),
                    width: self.thumb_width,
                    description: None,
                    wiki_url: None,
                    credit,
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

    /// A full cache writes nothing. It used to write the file anyway and
    /// leave it out of the index, so capacity bounded the index and nothing
    /// bounded the disk.
    #[test]
    fn a_full_cache_writes_nothing_to_disk() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_full");
        let _ = std::fs::remove_dir_all(&dir);
        let mut cache = DiskCache::new(&dir, 300).unwrap();
        cache.max_entries = 1;
        cache.store("turdus_merula", b"fake-jpeg").unwrap();
        assert!(cache.store("erithacus_rubecula", b"fake-jpeg").is_err());
        assert!(
            !cache
                .path_for("erithacus_rubecula")
                .is_some_and(|path| path.exists()),
            "no file past capacity"
        );
        // Replacing an entry already held is not growth.
        cache.store("turdus_merula", b"fake-jpeg-2").unwrap();
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
    fn remove_deletes_file_and_index_entry() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_remove");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        let path = cache.store("turdus_merula", b"fake-jpeg").unwrap();
        assert!(path.exists() && cache.contains("turdus_merula"));

        assert!(
            cache.remove("turdus_merula"),
            "reports the file was deleted"
        );
        assert!(!path.exists(), "file removed from disk");
        assert!(!cache.contains("turdus_merula"), "index entry dropped");
        assert!(
            !cache.remove("turdus_merula"),
            "removing an absent entry is a no-op, not an error"
        );
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

    fn a_credit() -> ImageCredit {
        ImageCredit {
            title: "Common blackbird".into(),
            author: "J. Smith".into(),
            source: "Wikimedia Commons".into(),
            source_url: "https://commons.wikimedia.org/wiki/File:X.jpg".into(),
            licence: "CC BY-SA 4.0".into(),
            licence_url: "https://creativecommons.org/licenses/by-sa/4.0/".into(),
        }
    }

    fn an_image(credit: Option<ImageCredit>) -> SpeciesImage {
        SpeciesImage {
            url: "https://upload.wikimedia.org/x.jpg".into(),
            cached_path: None,
            width: 300,
            description: None,
            wiki_url: None,
            credit,
        }
    }

    /// The credit is what makes the picture lawful to show, so it has to outlive the process:
    /// before it was written to disk, a restart turned every cached picture into one nobody
    /// could be credited for.
    #[test]
    fn a_credit_is_kept_beside_the_picture_and_survives_a_restart() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_credit_restart");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let cache = DiskCache::new(&dir, 300).unwrap();
            cache.store("turdus_merula", b"fake-jpeg").unwrap();
            cache.update_metadata("turdus_merula", &an_image(Some(a_credit())));
        }
        assert!(dir.join("turdus_merula.credit.json").exists());
        let reopened = DiskCache::new(&dir, 300).unwrap();
        assert_eq!(
            reopened.get("turdus_merula").unwrap().credit,
            Some(a_credit()),
            "read back by the scan"
        );
        assert_eq!(reopened.len(), 1, "the credit file is not a picture");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A credit left behind would be put on the next picture stored under the same name, which
    /// is somebody else's.
    #[test]
    fn removing_a_picture_removes_its_credit() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_credit_remove");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        cache.store("turdus_merula", b"fake-jpeg").unwrap();
        cache.update_metadata("turdus_merula", &an_image(Some(a_credit())));
        assert!(dir.join("turdus_merula.credit.json").exists());
        cache.remove("turdus_merula");
        assert!(!dir.join("turdus_merula.credit.json").exists());
        cache.store("turdus_merula", b"another-bird").unwrap();
        assert!(
            DiskCache::new(&dir, 300)
                .unwrap()
                .get("turdus_merula")
                .unwrap()
                .credit
                .is_none(),
            "the new picture does not inherit the old one's credit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A credit that cannot be read, or that names no source to follow, is no credit: the
    /// picture is treated as uncredited and replaced, never shown with a half of one.
    #[test]
    fn a_credit_that_is_unreadable_or_names_no_source_reads_as_none() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_credit_bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        std::fs::write(dir.join("a.credit.json"), b"{not json").unwrap();
        std::fs::write(dir.join("b.jpg"), b"x").unwrap();
        std::fs::write(
            dir.join("b.credit.json"),
            serde_json::to_vec(&ImageCredit {
                author: "Somebody".into(),
                ..ImageCredit::default()
            })
            .unwrap(),
        )
        .unwrap();
        let cache = DiskCache::new(&dir, 300).unwrap();
        assert!(cache.get("a").unwrap().credit.is_none(), "unreadable");
        assert!(cache.get("b").unwrap().credit.is_none(), "no source page");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A cache under `{name}/cache` with files named `victim` beside it, where a key that
    /// leaves the cache directory (`../victim`) would land.
    fn cache_beside_files(name: &str) -> (PathBuf, DiskCache) {
        let parent = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&parent);
        let cache = DiskCache::new(&parent.join("cache"), 300).unwrap();
        for file in ["victim.jpg", "victim.credit.json"] {
            std::fs::write(parent.join(file), b"not the cache's").unwrap();
        }
        (parent, cache)
    }

    /// The key comes from the name in a request path, so the cache refuses one that is not a
    /// single file name rather than trusting whoever built it. `remove` is where a request
    /// can aim it: an admin's blacklist, and a picture whose provider has nothing to offer.
    #[test]
    fn removing_a_key_that_leaves_the_directory_deletes_nothing_outside_it() {
        let (parent, cache) = cache_beside_files("birdnet_diskcache_escape_remove");
        assert!(!cache.remove("../victim"), "no cached picture was removed");
        // An absolute key holds no `..`: `Path::join` would let it replace the cache directory.
        let absolute = parent.join("victim").to_string_lossy().into_owned();
        assert!(!cache.remove(&absolute), "nor one named by its full path");
        for file in ["victim.jpg", "victim.credit.json"] {
            assert!(
                parent.join(file).exists(),
                "{file} beside the cache survives"
            );
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_key_that_leaves_the_directory_is_not_stored_and_not_read() {
        let (parent, cache) = cache_beside_files("birdnet_diskcache_escape_store");
        assert!(cache.store("../escape", b"x").is_err());
        assert!(
            !parent.join("escape.jpg").exists(),
            "nothing written beside the cache"
        );
        assert!(
            cache.get("../victim").is_none(),
            "victim.jpg is not a cached picture"
        );
        assert!(!cache.contains("../victim"));
        for key in ["", ".", "..", "a/b", "/etc/passwd", "a..b", "..a", "a.."] {
            assert!(
                cache.store(key, b"x").is_err(),
                "{key:?} is not a file name, or holds `..`"
            );
            assert!(!cache.contains(key), "{key:?}");
            assert!(cache.get(key).is_none(), "{key:?}");
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn a_credit_is_not_kept_or_read_under_a_key_that_leaves_the_directory() {
        let (parent, cache) = cache_beside_files("birdnet_diskcache_escape_credit");
        cache.update_metadata("../escape", &an_image(Some(a_credit())));
        assert!(
            !parent.join("escape.credit.json").exists(),
            "no credit written beside the cache"
        );
        cache.update_metadata("../victim", &an_image(None));
        assert!(
            parent.join("victim.credit.json").exists(),
            "and none removed from beside it"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The counterpart: scientific names carry punctuation, and a check that refused them would
    /// stop a species being cached at all. Only a key that is more than one file name, or that
    /// holds `..` anywhere, is refused.
    #[test]
    fn a_key_with_punctuation_in_it_is_still_a_file_name() {
        let dir = std::env::temp_dir().join("birdnet_diskcache_punctuation");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(&dir, 300).unwrap();
        for key in [
            "larus_sp.",
            "passer_domesticus_×_hispaniolensis",
            "anas_(domestic)",
            "a.b",
        ] {
            cache.store(key, b"fake-jpeg").unwrap();
            cache.update_metadata(key, &an_image(Some(a_credit())));
            assert!(cache.contains(key), "{key:?}");
            assert!(cache.get(key).unwrap().credit.is_some(), "{key:?}");
            assert!(cache.remove(key), "{key:?}");
            assert!(!dir.join(format!("{key}.credit.json")).exists(), "{key:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
