//! Try one provider, then another.
//!
//! # Why a chain rather than a choice
//!
//! Neither source covers every bird. Wikipedia has no photograph at all for a
//! long tail of species, and for many others has a museum skin or a range map.
//! Flickr's coverage is broader but its commercially-licensed subset is not,
//! and a `FLICKR_FILTER_EMAIL` narrowing the search to one photographer's own
//! photostream is *deliberately* sparse — an operator showing their own
//! pictures has pictures of a few dozen species, not nine thousand.
//!
//! So "choose a provider" is the wrong shape. A station that picks Flickr and
//! then shows nothing for three quarters of its species has been made worse by
//! the setting. Falling back means the operator's own photograph appears where
//! they have one and Wikipedia's fills in everywhere else, which is what they
//! wanted from the setting in the first place.
//!
//! # What falls through and what does not
//!
//! Only [`ImageError::NotFound`]. A network failure or an API error stops the
//! chain and is reported, because those are conditions an operator can fix and
//! silently papering over them with the other provider's image is how a broken
//! API key stays broken for a year.

use super::provider::ImageProvider;
use super::types::{ImageError, SpeciesImage};

/// Two providers in order: the second is consulted only when the first has
/// nothing for this species.
pub struct FallbackProvider {
    primary: Box<dyn ImageProvider>,
    secondary: Box<dyn ImageProvider>,
}

impl std::fmt::Debug for FallbackProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackProvider").finish_non_exhaustive()
    }
}

impl FallbackProvider {
    /// Ask `primary` first, `secondary` only on a miss.
    #[must_use]
    pub fn new(primary: Box<dyn ImageProvider>, secondary: Box<dyn ImageProvider>) -> Self {
        Self { primary, secondary }
    }
}

impl ImageProvider for FallbackProvider {
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
            match self.primary.fetch(scientific_name).await {
                Err(ImageError::NotFound(_)) => {
                    tracing::debug!(
                        species = scientific_name,
                        "primary image provider has nothing; trying the fallback"
                    );
                    self.secondary.fetch(scientific_name).await
                }
                other => other,
            }
        })
    }
}

#[cfg(test)]
mod tests;
