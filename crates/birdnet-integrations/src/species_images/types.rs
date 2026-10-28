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

/// What a licence asks a reader to be told about a picture: its title, its author, where it came
/// from and the terms it is under (the "TASL" of Creative Commons' attribution guidance).
///
/// Captured when the picture is fetched, kept beside the cached file, and shown wherever the
/// picture is. A provider that cannot supply one refuses the picture rather than showing it
/// uncredited.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ImageCredit {
    /// The work's title: a Commons file name, a Flickr title, a plate's caption.
    pub title: String,
    /// The author, as plain text. Empty when the source names none, in which case
    /// [`source_url`](Self::source_url) is where the author is listed.
    pub author: String,
    /// The place the picture is from, as a name: `Wikimedia Commons`, `Flickr`, `AviCommons`.
    pub source: String,
    /// The page that shows the picture at its source, where its author and terms are given.
    pub source_url: String,
    /// The licence, as its short name: `CC BY-SA 4.0`, `CC0`, `Public domain`.
    pub licence: String,
    /// The licence text, or empty when the source gives none.
    pub licence_url: String,
}

impl ImageCredit {
    /// The credit as one plain line: `Title by Author, Source, Licence`, leaving out what is
    /// empty.
    #[must_use]
    pub fn line(&self) -> String {
        let who = match (self.title.is_empty(), self.author.is_empty()) {
            (false, false) => format!("{} by {}", self.title, self.author),
            (false, true) => self.title.clone(),
            (true, false) => format!("By {}", self.author),
            (true, true) => String::new(),
        };
        [who.as_str(), self.source.as_str(), self.licence.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether a reader can find out who made the picture and on what terms: it names a source
    /// page, which is where an author and a licence are listed.
    #[must_use]
    pub fn is_attributable(&self) -> bool {
        !self.source_url.trim().is_empty() && !self.source.trim().is_empty()
    }

    /// Whether the licence forbids adaptation (`ND`) or commercial use (`NC`), by its name. The
    /// kiosk engraves photographs, which is an adaptation, so such a picture must not reach it.
    #[must_use]
    pub fn licence_is_restricted(&self) -> bool {
        licence_is_restricted(&self.licence)
    }
}

/// Whether a licence's short name carries a `NC` or `ND` term (`CC BY-NC 2.0`, `CC BY-ND 2.0`).
#[must_use]
pub fn licence_is_restricted(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("cc")
        && lower
            .split([' ', '-', '/'])
            .any(|token| matches!(token, "nc" | "nd"))
}

/// Plain text from the HTML fragment Wikimedia's metadata gives for an author or title: tags
/// removed, the five common entities decoded, whitespace folded.
#[must_use]
pub fn strip_html(fragment: &str) -> String {
    let mut text = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for ch in fragment.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
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
    /// Who made the picture and on what terms. `None` for a picture cached before credits were
    /// kept; it is replaced, with its credit, the next time it is asked for.
    pub credit: Option<ImageCredit>,
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
            credit: None,
        };
        let debug = format!("{image:?}");
        assert!(debug.contains("SpeciesImage"));
    }

    #[test]
    fn a_credit_reads_as_one_line_and_leaves_out_what_is_missing() {
        let full = ImageCredit {
            title: "Common blackbird".into(),
            author: "J. Smith".into(),
            source: "Wikimedia Commons".into(),
            source_url: "https://commons.wikimedia.org/wiki/File:X.jpg".into(),
            licence: "CC BY-SA 4.0".into(),
            licence_url: String::new(),
        };
        assert_eq!(
            full.line(),
            "Common blackbird by J. Smith, Wikimedia Commons, CC BY-SA 4.0"
        );
        let anonymous = ImageCredit {
            author: String::new(),
            ..full.clone()
        };
        assert_eq!(
            anonymous.line(),
            "Common blackbird, Wikimedia Commons, CC BY-SA 4.0"
        );
        let untitled = ImageCredit {
            title: String::new(),
            ..full
        };
        assert_eq!(
            untitled.line(),
            "By J. Smith, Wikimedia Commons, CC BY-SA 4.0"
        );
    }

    #[test]
    fn a_credit_is_attributable_only_with_a_source_page() {
        let mut c = ImageCredit::default();
        assert!(!c.is_attributable());
        c.source = "Flickr".into();
        assert!(!c.is_attributable(), "a name without a page is not enough");
        c.source_url = "https://www.flickr.com/photos/a/1/".into();
        assert!(c.is_attributable());
    }

    #[test]
    fn no_derivatives_and_non_commercial_licences_are_restricted() {
        for restricted in [
            "CC BY-ND 2.0",
            "CC BY-NC 4.0",
            "CC BY-NC-SA 3.0",
            "cc-by-nd",
        ] {
            assert!(licence_is_restricted(restricted), "{restricted}");
        }
        for free in [
            "CC BY 4.0",
            "CC BY-SA 3.0",
            "CC0 1.0",
            "Public domain",
            "Public Domain Mark 1.0",
            "PD-old-70",
        ] {
            assert!(!licence_is_restricted(free), "{free}");
        }
    }

    #[test]
    fn html_from_wikimedia_metadata_becomes_plain_text() {
        assert_eq!(
            strip_html(
                r#"<a href="//commons.wikimedia.org/wiki/User:X" title="User:X">J. R. Smith</a> &amp; <b>Co</b>"#
            ),
            "J. R. Smith & Co"
        );
        assert_eq!(strip_html("  Ren&#39;e\n\t d&quot;Or "), "Ren'e d\"Or");
        assert_eq!(strip_html(""), "");
    }
}
