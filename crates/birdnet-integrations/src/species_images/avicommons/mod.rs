//! AviCommons image provider.
//!
//! [AviCommons](https://avicommons.org) is a library of hand-picked, cropped,
//! oriented bird photographs, each under a Creative Commons licence, built to be
//! embedded by other bird software. Where Wikipedia has a museum skin, an egg or
//! a range map for a species, AviCommons usually has a living bird.
//!
//! # What was checked before this was written (2026-10-21)
//!
//! * **No key, no account, no sign-up.** The data is two public things: one JSON
//!   index and an image host (`static.avicommons.org`, Cloudflare R2). Nothing
//!   to configure.
//! * **The terms for the index** are in the project's README: download the JSON
//!   *once*, never "on each page load", so that it costs them little. This
//!   provider downloads it at most once in [`INDEX_MAX_AGE`] and keeps it on
//!   disk, so a Raspberry Pi that is power-cycled daily does not fetch it daily.
//!   The repository has no licence file (the raw URL answers 404); the images'
//!   licences are per image, in the index.
//! * **Per-image licences, mixed.** The 10,185 images in `latest.json` carry
//!   about forty different licence strings (`cc-by-nc`, `CC BY-NC 2.0`,
//!   `CC BY-SA 3.0-de`, …): 2,620 permit commercial use (CC0, CC BY, CC BY-SA)
//!   and 7,565 are NonCommercial or NonCommercial-ShareAlike.
//! * **Coverage of what a station can hear.** Against the 6,522 species of the
//!   BirdNET V2.4 label set: 6,059 have some AviCommons image, 1,628 have one
//!   under a commercial-use licence.
//!
//! # Licensing decision
//!
//! This project is CC BY-NC-SA 4.0, so a NonCommercial image is not
//! incompatible with *distributing the software*. The question this provider
//! has to answer is a different one, and the Flickr provider answered it
//! already: a station's page may be published by a reserve, a visitor centre or
//! a school, and the station cannot know whether that is commercial. So the same
//! rule applies here: **only CC0, CC BY and CC BY-SA images are used**
//! ([`licence_permits_commercial_use`]). That is 1,628 of the 6,522 species,
//! 25 %, which is why this is a provider *in front of* Wikipedia (see
//! [`super::chain`]) and not instead of it: a species with no permissive
//! AviCommons image falls through to Wikipedia unchanged.
//!
//! Attribution is required by every licence in the set, so an image whose
//! photographer the index does not name is not used, and each one is shown with
//! the photographer, the licence and a link to AviCommons, through the same
//! credit line the species page already renders for Flickr.
//!
//! # Matching
//!
//! The index is keyed by eBird species code, with the scientific name in the
//! Clements/eBird taxonomy. The station asks by scientific name (the classifier's
//! label), matched case-insensitively. A species whose name differs between
//! the two taxonomies (a recent split or a lump) is a miss, which falls through
//! to Wikipedia.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use tokio::sync::Mutex;

use super::provider::ImageProvider;
use super::types::{ImageError, SpeciesImage};

/// The index's address, per the project's README ("Getting Started").
pub const INDEX_URL: &str = "https://avicommons.org/latest.json";

/// Where the images are, per the README: `{code}-{key}-{size}.jpg`.
const IMAGE_HOST: &str = "https://static.avicommons.org";

/// The widths the host serves, smallest first.
const SIZES: [u32; 5] = [160, 240, 320, 480, 900];

/// How old the on-disk copy of the index may be before it is fetched again.
///
/// The collection is updated about once a year (new taxonomy each October), so a
/// month is far more often than it changes and far less often than a page load.
pub const INDEX_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

/// After a failed download, how long before the next attempt.
const RETRY_AFTER: Duration = Duration::from_secs(15 * 60);

/// The index is 1.6 MB; this is the most that is accepted.
const MAX_INDEX_BYTES: usize = 8 * 1024 * 1024;

/// The file the index is kept in, inside the image cache directory.
pub const INDEX_FILE: &str = "avicommons-index.json";

/// One image's entry in the index, as published.
#[derive(Debug, Deserialize)]
struct Record {
    code: String,
    #[serde(rename = "sciName")]
    sci_name: String,
    license: String,
    key: String,
    by: String,
}

/// An image this provider may use.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    code: String,
    key: String,
    by: String,
    licence: String,
}

/// Whether an AviCommons licence string permits commercial use.
///
/// The index spells licences about forty ways: lower-case slugs (`cc-by-nc`),
/// versioned names (`CC BY-NC-SA 4.0`) and regional ports (`CC BY 3.0-de`).
/// Only CC0, CC BY and CC BY-SA are accepted; anything with `NC` or `ND`, and
/// anything not recognised, is not. Unknown is refused, not allowed: a new
/// spelling costs coverage, never a licence breach.
#[must_use]
pub fn licence_permits_commercial_use(licence: &str) -> bool {
    let lower = licence.trim().to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split([' ', '-']).filter(|t| !t.is_empty()).collect();
    match tokens.as_slice() {
        ["cc0", rest @ ..] => rest.iter().all(|t| is_version_or_region(t)),
        ["cc", "by", rest @ ..] => rest.iter().all(|t| *t == "sa" || is_version_or_region(t)),
        _ => false,
    }
}

/// `2.0`, `4.0`, `1.0`, or a two-letter regional port such as `de` or `us`.
/// `nc` and `nd` are two letters too: they are the point of the filter.
fn is_version_or_region(token: &str) -> bool {
    if matches!(token, "nc" | "nd" | "sa") {
        return false;
    }
    token.starts_with(|c: char| c.is_ascii_digit())
        || (token.len() == 2 && token.chars().all(|c| c.is_ascii_lowercase()))
}

/// How a licence is written in a credit line: `CC BY 4.0`, `CC BY-SA`, `CC0`.
fn licence_label(licence: &str) -> String {
    let trimmed = licence.trim();
    if trimmed.starts_with("CC") {
        return trimmed.to_owned();
    }
    // The lower-case slugs: `cc-by`, `cc-by-sa`, `cc0`.
    match trimmed.to_ascii_lowercase().as_str() {
        "cc0" => "CC0".to_owned(),
        "cc-by" => "CC BY".to_owned(),
        "cc-by-sa" => "CC BY-SA".to_owned(),
        other => other.to_ascii_uppercase(),
    }
}

/// The usable images of an index, by lower-case scientific name.
#[derive(Debug, Default)]
struct Index {
    by_species: HashMap<String, Entry>,
    /// Every record in the file, usable or not, for the log line.
    total: usize,
}

impl Index {
    /// Parse the published JSON, keeping what may be used.
    ///
    /// # Errors
    ///
    /// [`ImageError::Api`] when the text is not the expected array: a changed
    /// format must read as a fault, not as "no images for any species".
    fn parse(json: &str) -> Result<Self, ImageError> {
        let records: Vec<Record> = serde_json::from_str(json).map_err(|e| {
            ImageError::Api(format!("AviCommons index is not the expected JSON: {e}"))
        })?;
        if records.is_empty() {
            return Err(ImageError::Api("AviCommons index is empty".into()));
        }
        let total = records.len();
        let mut by_species = HashMap::new();
        for r in records {
            if !licence_permits_commercial_use(&r.license) || r.by.trim().is_empty() {
                continue;
            }
            by_species
                .entry(r.sci_name.trim().to_ascii_lowercase())
                .or_insert_with(|| Entry {
                    code: r.code,
                    key: r.key,
                    by: r.by.trim().to_owned(),
                    licence: licence_label(&r.license),
                });
        }
        Ok(Self { by_species, total })
    }
}

enum State {
    Empty,
    Ready(Arc<Index>),
    /// The last download failed at this time.
    Failed(std::time::Instant),
}

/// AviCommons image provider.
pub struct AviCommonsClient {
    http: reqwest::Client,
    index_path: PathBuf,
    thumb_width: u32,
    state: Mutex<State>,
}

impl std::fmt::Debug for AviCommonsClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AviCommonsClient")
            .field("index_path", &self.index_path)
            .field("thumb_width", &self.thumb_width)
            .finish_non_exhaustive()
    }
}

impl AviCommonsClient {
    /// A client that keeps its copy of the index in `cache_dir`.
    ///
    /// # Errors
    ///
    /// [`ImageError::Http`] if the HTTP client cannot be built.
    pub fn new(cache_dir: &Path, thumb_width: u32) -> Result<Self, ImageError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(super::IMAGE_DOWNLOAD_USER_AGENT)
            .build()
            .map_err(|e| ImageError::Http(crate::http_error_text(e)))?;
        Ok(Self {
            http,
            index_path: cache_dir.join(INDEX_FILE),
            thumb_width,
            state: Mutex::new(State::Empty),
        })
    }

    /// The image address for an entry: the smallest size that is at least the
    /// thumbnail width, or the largest there is.
    fn image_url(&self, entry: &Entry) -> String {
        let size = SIZES
            .iter()
            .copied()
            .find(|s| *s >= self.thumb_width)
            .unwrap_or(SIZES[SIZES.len() - 1]);
        format!("{IMAGE_HOST}/{}-{}-{size}.jpg", entry.code, entry.key)
    }

    fn species_image(&self, entry: &Entry) -> SpeciesImage {
        SpeciesImage {
            url: self.image_url(entry),
            cached_path: None,
            width: self.thumb_width,
            description: Some(format!(
                "Photograph by {}, {}, via Avicommons.",
                entry.by, entry.licence
            )),
            wiki_url: Some(format!("https://avicommons.org/species/{}", entry.code)),
        }
    }

    /// The index: from memory, else from a fresh enough file, else downloaded.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "the lock is held across the download on purpose: it makes the fetch single-flight, so a page of species asks the host once, not once each"
    )]
    async fn index(&self) -> Result<Arc<Index>, ImageError> {
        let mut state = self.state.lock().await;
        match &*state {
            State::Ready(index) => return Ok(Arc::clone(index)),
            State::Failed(at) if at.elapsed() < RETRY_AFTER => {
                return Err(ImageError::Http(
                    "the AviCommons index could not be downloaded; trying again later".into(),
                ));
            }
            _ => {}
        }
        // A copy on disk that is young enough is the index: no request at all.
        let path = self.index_path.clone();
        let fresh = tokio::task::spawn_blocking(move || read_if_fresh(&path, INDEX_MAX_AGE))
            .await
            .ok()
            .flatten();
        if let Some(text) = fresh
            && let Ok(index) = Index::parse(&text)
        {
            let index = Arc::new(index);
            *state = State::Ready(Arc::clone(&index));
            return Ok(index);
        }
        match self.download().await {
            Ok(text) => {
                let index = Arc::new(Index::parse(&text)?);
                let (path, bytes) = (self.index_path.clone(), text.into_bytes());
                let kept = tokio::task::spawn_blocking(move || write_atomically(&path, &bytes))
                    .await
                    .map_err(std::io::Error::other)
                    .and_then(|written| written);
                if let Err(e) = kept {
                    // Not fatal: the index works from memory; it is fetched
                    // again at the next start.
                    tracing::warn!(error = %e, "AviCommons index not kept on disk");
                }
                tracing::info!(
                    images = index.total,
                    usable = index.by_species.len(),
                    "AviCommons index loaded (CC0, CC BY and CC BY-SA images only)"
                );
                *state = State::Ready(Arc::clone(&index));
                Ok(index)
            }
            Err(e) => {
                // An old copy is better than none: the collection changes once
                // a year, and a station that is offline must not lose pictures
                // it could still show.
                if let Ok(text) = tokio::fs::read_to_string(&self.index_path).await
                    && let Ok(index) = Index::parse(&text)
                {
                    tracing::warn!(error = %e, "AviCommons index could not be refreshed; using the copy on disk");
                    let index = Arc::new(index);
                    *state = State::Ready(Arc::clone(&index));
                    return Ok(index);
                }
                *state = State::Failed(std::time::Instant::now());
                Err(e)
            }
        }
    }

    async fn download(&self) -> Result<String, ImageError> {
        let mut resp = self
            .http
            .get(INDEX_URL)
            .send()
            .await
            .map_err(|e| ImageError::Http(crate::http_error_text(e)))?
            .error_for_status()
            .map_err(|e| ImageError::Http(crate::http_error_text(e)))?;
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_INDEX_BYTES as u64)
        {
            return Err(ImageError::Http(
                "AviCommons index is larger than expected".into(),
            ));
        }
        let mut body = Vec::with_capacity(1_700_000);
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| ImageError::Http(crate::http_error_text(e)))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_INDEX_BYTES {
                return Err(ImageError::Http(
                    "AviCommons index is larger than expected".into(),
                ));
            }
            body.extend_from_slice(&chunk);
        }
        String::from_utf8(body).map_err(|_| ImageError::Api("AviCommons index is not UTF-8".into()))
    }
}

/// The file's text when it exists and was written less than `max_age` ago.
fn read_if_fresh(path: &Path, max_age: Duration) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    if age < max_age {
        std::fs::read_to_string(path).ok()
    } else {
        None
    }
}

/// Write beside the file and rename into place, so a power cut leaves the old
/// copy or the new one and never half of one.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("json.part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}

impl ImageProvider for AviCommonsClient {
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
        Box::pin(async move {
            let index = self.index().await?;
            index
                .by_species
                .get(&scientific_name.trim().to_ascii_lowercase())
                .map(|entry| self.species_image(entry))
                .ok_or_else(|| ImageError::NotFound(scientific_name.to_owned()))
        })
    }
}

#[cfg(test)]
mod tests;
