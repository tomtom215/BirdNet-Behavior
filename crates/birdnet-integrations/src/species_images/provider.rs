//! `ImageProvider` trait — the abstraction over image sources.
//!
//! Any image backend (Wikipedia, Flickr, eBird, …) implements this trait
//! so that `ImageCache` is source-agnostic.

use super::types::{ImageError, SpeciesImage};

/// A source that can supply species images by scientific name.
///
/// Implementations are async (network-bound) and must be `Send + Sync`
/// so they can be stored behind an `Arc` in the shared `ImageCache`.
pub trait ImageProvider: Send + Sync {
    /// Fetch image metadata for `scientific_name`.
    ///
    /// Returns a `SpeciesImage` describing the remote image URL and optional
    /// description.  The `cached_path` field is always `None`; caching is
    /// handled by `ImageCache`.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::NotFound` if no image can be found.
    /// Returns `ImageError::Http` or `ImageError::Api` on transient failures.
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
        Self: 'async_trait;
}
