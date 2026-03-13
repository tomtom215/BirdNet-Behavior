//! Wikipedia/Wikimedia Commons image provider.
//!
//! Fetches species thumbnail images from Wikipedia using the `MediaWiki` API.
//! Uses the scientific name for lookups (more reliable than locale-sensitive
//! common names) and prefers the `pageimages` + `extracts` properties.
//!
//! Wikipedia is preferred because:
//! - No API key required
//! - CC-licensed images
//! - Reliable coverage for bird species
//! - Single API for both image URL and description

use std::time::Duration;

use super::provider::ImageProvider;
use super::types::{ImageError, SpeciesImage};

/// Default request timeout for Wikipedia API.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Maximum retry attempts for failed requests.
const MAX_RETRIES: u32 = 2;

/// Default thumbnail width in pixels.
pub const DEFAULT_THUMB_WIDTH: u32 = 300;

/// Wikipedia API endpoint (English).
const WIKIPEDIA_API: &str = "https://en.wikipedia.org/w/api.php";

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

/// Wikipedia image provider.
///
/// Queries the Wikipedia `MediaWiki` API for a species image and description,
/// falling back gracefully when no image exists.
#[derive(Debug)]
pub struct WikipediaClient {
    http: reqwest::Client,
    thumb_width: u32,
}

impl WikipediaClient {
    /// Create a new client with the default thumbnail width.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::Http` if the HTTP client cannot be constructed.
    pub fn new() -> Result<Self, ImageError> {
        Self::with_thumb_width(DEFAULT_THUMB_WIDTH)
    }

    /// Create a new client with a specific thumbnail width.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::Http` if the HTTP client cannot be constructed.
    pub fn with_thumb_width(thumb_width: u32) -> Result<Self, ImageError> {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent("BirdNet-Behavior/0.1 (bird classification system)")
            .build()
            .map_err(|e| ImageError::Http(e.to_string()))?;
        Ok(Self { http, thumb_width })
    }

    /// Query the Wikipedia API for a page image and extract.
    async fn query_page(
        &self,
        scientific_name: &str,
    ) -> Result<(String, Option<String>, Option<String>), ImageError> {
        let encoded = url_encode(scientific_name);
        let url = format!(
            "{WIKIPEDIA_API}?action=query&format=json&formatversion=2\
             &prop=pageimages%7Cextracts%7Cinfo&titles={encoded}\
             &pithumbsize={}&exintro=1&explaintext=1&exsentences=3\
             &inprop=url&redirects=1",
            self.thumb_width
        );

        let mut last_error = ImageError::Http("no attempts made".into());
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
            }
            match self.http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| ImageError::Http(e.to_string()))?;
                    let json: serde_json::Value =
                        serde_json::from_str(&body).map_err(|e| ImageError::Api(e.to_string()))?;
                    return Self::parse_response(&json, scientific_name);
                }
                Ok(resp) => {
                    last_error = ImageError::Api(format!("HTTP {}", resp.status()));
                }
                Err(e) => {
                    last_error = ImageError::Http(e.to_string());
                }
            }
        }
        Err(last_error)
    }

    /// Parse the Wikipedia API response.
    pub(super) fn parse_response(
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

        if page.get("missing").is_some() {
            return Err(ImageError::NotFound(scientific_name.to_string()));
        }

        let image_url = page
            .get("thumbnail")
            .and_then(|t| t.get("source"))
            .and_then(|s| s.as_str())
            .ok_or_else(|| ImageError::NotFound(scientific_name.to_string()))?
            .to_string();

        let description = page
            .get("extract")
            .and_then(|e| e.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let wiki_url = page
            .get("fullurl")
            .and_then(|u| u.as_str())
            .map(String::from);

        Ok((image_url, description, wiki_url))
    }

    /// Download bytes from a URL with retry.
    pub async fn download_bytes(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        let mut last_error = ImageError::Http("no attempts made".into());
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
            }
            match self.http.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return resp
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| ImageError::Http(e.to_string()));
                }
                Ok(resp) => last_error = ImageError::Http(format!("HTTP {}", resp.status())),
                Err(e) => last_error = ImageError::Http(e.to_string()),
            }
        }
        Err(last_error)
    }
}

impl ImageProvider for WikipediaClient {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<SpeciesImage, ImageError>> + Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let name = scientific_name.to_string();
        let width = self.thumb_width;
        Box::pin(async move {
            let (image_url, description, wiki_url) = self.query_page(&name).await?;
            Ok(SpeciesImage {
                url: image_url,
                cached_path: None,
                width,
                description,
                wiki_url,
            })
        })
    }
}

/// Minimal percent-encoding for URL query parameter values.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_plain() {
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

    #[test]
    fn parse_response_success() {
        let json: serde_json::Value = serde_json::json!({
            "query": {
                "pages": [{
                    "pageid": 12345,
                    "title": "Turdus merula",
                    "thumbnail": {
                        "source": "https://upload.wikimedia.org/test.jpg",
                        "width": 300,
                        "height": 200
                    },
                    "extract": "The common blackbird is a species of true thrush.",
                    "fullurl": "https://en.wikipedia.org/wiki/Common_blackbird"
                }]
            }
        });
        let (url, desc, wiki) =
            WikipediaClient::parse_response(&json, "Turdus merula").unwrap();
        assert!(url.contains("wikimedia.org"));
        assert_eq!(desc.unwrap(), "The common blackbird is a species of true thrush.");
        assert!(wiki.unwrap().contains("wikipedia.org"));
    }

    #[test]
    fn parse_response_missing_page() {
        let json: serde_json::Value = serde_json::json!({
            "query": { "pages": [{ "title": "Nonexistent", "missing": true }] }
        });
        assert!(matches!(
            WikipediaClient::parse_response(&json, "Nonexistent"),
            Err(ImageError::NotFound(_))
        ));
    }

    #[test]
    fn parse_response_no_thumbnail() {
        let json: serde_json::Value = serde_json::json!({
            "query": { "pages": [{ "pageid": 1, "extract": "Some text." }] }
        });
        assert!(matches!(
            WikipediaClient::parse_response(&json, "Some species"),
            Err(ImageError::NotFound(_))
        ));
    }
}
