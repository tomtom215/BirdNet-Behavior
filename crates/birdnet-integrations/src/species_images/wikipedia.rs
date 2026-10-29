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
//!
//! # Credit
//!
//! A page image is a file, and a file has an author and a licence. The provider asks the
//! file's own metadata for them and builds the picture's [`ImageCredit`] from the answer. A file
//! marked non-free (Wikipedia, unlike Commons, hosts some under fair use), or under a licence that
//! forbids adaptation or commercial use, is refused as if the page had no picture: it is not ours
//! to show, and the kiosk engraves what it is given.

use std::time::Duration;

use super::provider::ImageProvider;
use super::types::{ImageCredit, ImageError, SpeciesImage, licence_is_restricted, strip_html};

/// Default request timeout for Wikipedia API.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Total request attempts (initial + retries) before a fetch is abandoned.
const MAX_ATTEMPTS: u32 = 2;

/// Default thumbnail width in pixels.
pub const DEFAULT_THUMB_WIDTH: u32 = 300;

/// Wikipedia API endpoint (English).
const WIKIPEDIA_API: &str = "https://en.wikipedia.org/w/api.php";

const HEX_CHARS: [u8; 16] = *b"0123456789ABCDEF";

/// The `extmetadata` fields a credit is built from.
const META_FIELDS: &str = "Artist%7CObjectName%7CLicenseShortName%7CLicenseUrl%7CNonFree";

/// What the page query found: the thumbnail, the file it is a thumbnail of, and the extract.
#[derive(Debug)]
pub(super) struct PageHit {
    /// The thumbnail's URL.
    pub(super) image_url: String,
    /// The page's introductory extract.
    pub(super) description: Option<String>,
    /// The page's own URL.
    pub(super) wiki_url: Option<String>,
    /// The file the page's picture is, without the `File:` prefix.
    pub(super) file_name: Option<String>,
}

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

    /// GET `url` and parse the JSON body, retrying a failed request once.
    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ImageError> {
        let mut last_error = ImageError::Http("no attempts made".into());
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
            }
            match self.http.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| ImageError::Http(e.to_string()))?;
                    return serde_json::from_str(&body).map_err(|e| ImageError::Api(e.to_string()));
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

    /// Query the Wikipedia API for a page image and extract.
    async fn query_page(&self, scientific_name: &str) -> Result<PageHit, ImageError> {
        let encoded = url_encode(scientific_name);
        let url = format!(
            "{WIKIPEDIA_API}?action=query&format=json&formatversion=2\
             &prop=pageimages%7Cextracts%7Cinfo&titles={encoded}\
             &piprop=name%7Cthumbnail&pithumbsize={}&exintro=1&explaintext=1&exsentences=3\
             &inprop=url&redirects=1",
            self.thumb_width
        );
        let json = self.get_json(&url).await?;
        Self::parse_response(&json, scientific_name)
    }

    /// Ask for a file's author and licence.
    async fn query_credit(&self, file_name: &str) -> Result<ImageCredit, ImageError> {
        let url = format!(
            "{WIKIPEDIA_API}?action=query&format=json&formatversion=2&prop=imageinfo\
             &iiprop=extmetadata%7Curl&iiextmetadatafilter={META_FIELDS}&titles=File%3A{}",
            url_encode(file_name)
        );
        let json = self.get_json(&url).await?;
        Self::parse_credit(&json, file_name)
    }

    /// Build a file's [`ImageCredit`] from the `imageinfo` answer, or refuse it.
    ///
    /// # Errors
    ///
    /// [`ImageError::NotFound`] for a file marked non-free, under a licence that forbids
    /// adaptation or commercial use, or hosted on Wikipedia itself with no licence at all.
    /// [`ImageError::Api`] when the answer is not the expected shape.
    pub(super) fn parse_credit(
        json: &serde_json::Value,
        file_name: &str,
    ) -> Result<ImageCredit, ImageError> {
        let page = json
            .get("query")
            .and_then(|q| q.get("pages"))
            .and_then(serde_json::Value::as_array)
            .and_then(|pages| pages.first())
            .ok_or_else(|| ImageError::Api("unexpected imageinfo response structure".into()))?;
        let info = page
            .get("imageinfo")
            .and_then(serde_json::Value::as_array)
            .and_then(|infos| infos.first())
            .ok_or_else(|| ImageError::Api(format!("no file information for '{file_name}'")))?;
        let meta = |key: &str| -> String {
            info.get("extmetadata")
                .and_then(|m| m.get(key))
                .and_then(|v| v.get("value"))
                .and_then(serde_json::Value::as_str)
                .map(strip_html)
                .unwrap_or_default()
        };
        let refuse = |why: &str| ImageError::NotFound(format!("{file_name}: {why}"));

        if meta("NonFree").eq_ignore_ascii_case("true") {
            return Err(refuse("marked non-free"));
        }
        let licence = meta("LicenseShortName");
        if licence_is_restricted(&licence) {
            return Err(refuse("its licence forbids adaptation or commercial use"));
        }
        let shared = page
            .get("imagerepository")
            .and_then(serde_json::Value::as_str)
            == Some("shared");
        if !shared && licence.is_empty() {
            return Err(refuse("hosted on Wikipedia with no licence"));
        }

        let title = {
            let name = meta("ObjectName");
            if name.is_empty() {
                file_name.replace('_', " ")
            } else {
                name
            }
        };
        let source_url = info
            .get("descriptionurl")
            .and_then(serde_json::Value::as_str)
            .filter(|u| !u.is_empty())
            .map_or_else(
                || {
                    format!(
                        "https://commons.wikimedia.org/wiki/File:{}",
                        url_encode(file_name)
                    )
                },
                str::to_owned,
            );
        Ok(ImageCredit {
            title,
            author: meta("Artist"),
            source: if shared {
                "Wikimedia Commons"
            } else {
                "Wikipedia"
            }
            .to_owned(),
            source_url,
            licence,
            licence_url: meta("LicenseUrl"),
        })
    }

    /// Parse the Wikipedia API response.
    pub(super) fn parse_response(
        json: &serde_json::Value,
        scientific_name: &str,
    ) -> Result<PageHit, ImageError> {
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

        let file_name = page
            .get("pageimage")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(String::from);

        Ok(PageHit {
            image_url,
            description,
            wiki_url,
            file_name,
        })
    }

    /// Download bytes from a URL with retry.
    ///
    /// # Errors
    ///
    /// Returns `ImageError::Http` on network failures or non-success HTTP responses.
    pub async fn download_bytes(&self, url: &str) -> Result<Vec<u8>, ImageError> {
        let mut last_error = ImageError::Http("no attempts made".into());
        for attempt in 0..MAX_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2_u64.pow(attempt))).await;
            }
            match self.http.get(url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return super::read_capped_image_bytes(resp).await;
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
        let width = self.thumb_width;
        Box::pin(async move {
            let hit = self.query_page(&name).await?;
            // A picture nobody can be credited for is not shown. The page names its image's
            // file; without that there is no way to ask who made it.
            let file_name = hit.file_name.ok_or_else(|| {
                ImageError::NotFound(format!("{name}: the page's picture names no file"))
            })?;
            let credit = self.query_credit(&file_name).await?;
            Ok(SpeciesImage {
                url: hit.image_url,
                cached_path: None,
                width,
                description: hit.description,
                wiki_url: hit.wiki_url,
                credit: Some(credit),
            })
        })
    }
}

/// Minimal percent-encoding for URL query parameter values.
///
/// `pub(super)` so the Flickr provider shares one encoder rather than carrying
/// a second copy of the same table. It lives here because this is where it was
/// written and moving it would make the diff about the move.
pub(super) fn url_encode(s: &str) -> String {
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
                    "pageimage": "Common_blackbird.jpg",
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
        let hit = WikipediaClient::parse_response(&json, "Turdus merula").unwrap();
        assert!(hit.image_url.contains("wikimedia.org"));
        assert_eq!(
            hit.description.unwrap(),
            "The common blackbird is a species of true thrush."
        );
        assert!(hit.wiki_url.unwrap().contains("wikipedia.org"));
        assert_eq!(hit.file_name.as_deref(), Some("Common_blackbird.jpg"));
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

    fn imageinfo(repo: &str, meta: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "query": { "pages": [{
            "title": "File:Common blackbird.jpg",
            "imagerepository": repo,
            "imageinfo": [{
                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Common_blackbird.jpg",
                "extmetadata": meta
            }]
        }]}})
    }

    fn value(v: &str) -> serde_json::Value {
        serde_json::json!({ "value": v, "source": "commons-desc-page" })
    }

    #[test]
    fn a_commons_file_becomes_a_credit_with_author_source_and_licence() {
        let json = imageinfo(
            "shared",
            &serde_json::json!({
                "Artist": value(r#"<a href="//commons.wikimedia.org/wiki/User:Jo">Jo Smith</a>"#),
                "ObjectName": value("Common blackbird"),
                "LicenseShortName": value("CC BY-SA 4.0"),
                "LicenseUrl": value("https://creativecommons.org/licenses/by-sa/4.0"),
                "NonFree": value("false"),
            }),
        );
        let credit = WikipediaClient::parse_credit(&json, "Common_blackbird.jpg").unwrap();
        assert_eq!(credit.title, "Common blackbird");
        assert_eq!(credit.author, "Jo Smith");
        assert_eq!(credit.source, "Wikimedia Commons");
        assert_eq!(credit.licence, "CC BY-SA 4.0");
        assert_eq!(
            credit.source_url,
            "https://commons.wikimedia.org/wiki/File:Common_blackbird.jpg"
        );
        assert!(credit.is_attributable());
    }

    #[test]
    fn a_non_free_file_is_refused_not_shown_uncredited() {
        let json = imageinfo(
            "local",
            &serde_json::json!({
                "Artist": value("A. Photographer"),
                "LicenseShortName": value("Fair use"),
                "NonFree": value("true"),
            }),
        );
        assert!(matches!(
            WikipediaClient::parse_credit(&json, "X.jpg"),
            Err(ImageError::NotFound(_))
        ));
    }

    /// The counterpart that keeps the refusal honest: the same metadata without the marker is
    /// shown, so the gate is on `NonFree`, not on "local" or "fair".
    #[test]
    fn a_free_file_hosted_on_wikipedia_with_a_licence_is_credited() {
        let json = imageinfo(
            "local",
            &serde_json::json!({
                "Artist": value("A. Photographer"),
                "LicenseShortName": value("CC BY 3.0"),
                "NonFree": value("false"),
            }),
        );
        let credit = WikipediaClient::parse_credit(&json, "X.jpg").unwrap();
        assert_eq!(credit.source, "Wikipedia");
        assert_eq!(credit.licence, "CC BY 3.0");
    }

    #[test]
    fn a_local_file_with_no_licence_is_refused() {
        let json = imageinfo("local", &serde_json::json!({ "Artist": value("A") }));
        assert!(matches!(
            WikipediaClient::parse_credit(&json, "X.jpg"),
            Err(ImageError::NotFound(_))
        ));
    }

    #[test]
    fn a_licence_that_forbids_adaptation_is_refused() {
        for licence in ["CC BY-ND 2.0", "CC BY-NC 4.0"] {
            let json = imageinfo(
                "shared",
                &serde_json::json!({ "LicenseShortName": value(licence) }),
            );
            assert!(
                matches!(
                    WikipediaClient::parse_credit(&json, "X.jpg"),
                    Err(ImageError::NotFound(_))
                ),
                "{licence}"
            );
        }
    }

    #[test]
    fn a_file_with_no_author_is_still_credited_through_its_page() {
        let json = imageinfo(
            "shared",
            &serde_json::json!({ "LicenseShortName": value("Public domain") }),
        );
        let credit = WikipediaClient::parse_credit(&json, "Common_blackbird.jpg").unwrap();
        assert_eq!(credit.author, "");
        assert_eq!(credit.title, "Common blackbird.jpg");
        assert!(
            credit.is_attributable(),
            "the page is where the author is listed"
        );
    }

    #[test]
    fn a_page_whose_picture_names_no_file_cannot_be_credited() {
        let json: serde_json::Value = serde_json::json!({ "query": { "pages": [{
            "pageid": 1,
            "thumbnail": { "source": "https://upload.wikimedia.org/x.jpg" }
        }]}});
        let hit = WikipediaClient::parse_response(&json, "Some bird").unwrap();
        assert!(hit.file_name.is_none());
    }
}
