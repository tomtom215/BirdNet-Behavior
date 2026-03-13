//! Error and data types for species image operations.

use std::fmt;
use std::path::PathBuf;

/// Errors from species image operations.
#[derive(Debug)]
pub enum ImageError {
    /// HTTP request failed.
    Http(String),
    /// Remote API returned an error or unexpected response.
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
            Self::Api(msg) => write!(f, "image API error: {msg}"),
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
    /// URL of the image on the remote source.
    pub url: String,
    /// Local path to the cached image file (`None` if not yet downloaded).
    pub cached_path: Option<PathBuf>,
    /// Image width in pixels (of the thumbnail).
    pub width: u32,
    /// Brief description/extract from the source (first paragraph).
    pub description: Option<String>,
    /// Source page URL for the species.
    pub wiki_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ImageError::NotFound("Turdus merula".into()).to_string(),
            "no image found for: Turdus merula"
        );
        assert_eq!(
            ImageError::Http("timeout".into()).to_string(),
            "image HTTP error: timeout"
        );
        assert_eq!(
            ImageError::Api("bad response".into()).to_string(),
            "image API error: bad response"
        );
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
}
