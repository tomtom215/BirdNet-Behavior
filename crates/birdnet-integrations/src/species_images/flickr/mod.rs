//! Flickr image provider.
//!
//! # Why a second provider
//!
//! Wikipedia has no photograph at all for a long tail of species, and for many
//! others has a museum skin, an egg, or a range map. A station whose most
//! interesting visitor renders as a distribution map is a worse station.
//!
//! Flickr also allows something Wikipedia cannot: `FLICKR_FILTER_EMAIL`
//! restricts the search to one photographer's photostream, so an operator can
//! show *their own* photographs of the birds their own station heard.
//!
//! # Licensing
//!
//! Only the licences that permit commercial use are requested
//! ([`ALLOWED_LICENSES`]). A station's web UI is usually not commercial, but
//! "usually" is not a licence, and the station cannot know: a reserve, a
//! visitor centre or a school may all publish the page. Requesting the
//! permissive set is the choice that cannot go wrong, and Flickr filters
//! server-side so it costs nothing.
//!
//! Attribution is not optional under CC BY / CC BY-SA. Every image carries the
//! photographer's name in [`SpeciesImage::description`] and a link to the
//! Flickr photo page in [`SpeciesImage::wiki_url`], which is what the species
//! page already renders as its credit line.

use std::time::Duration;

use tokio::sync::OnceCell;

use super::provider::ImageProvider;
use super::types::{ImageError, SpeciesImage};
use super::wikipedia::url_encode;

/// Flickr REST endpoint.
const FLICKR_API: &str = "https://api.flickr.com/services/rest/";

/// Request timeout, matching the Wikipedia provider.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Total attempts (initial + retries) before a fetch is abandoned.
const MAX_ATTEMPTS: u32 = 2;

/// Flickr licence IDs that permit commercial use, as the API's `license`
/// parameter wants them: comma-separated.
///
/// | ID | Licence |
/// |----|---------|
/// | 4 | CC BY |
/// | 5 | CC BY-SA |
/// | 6 | CC BY-ND |
/// | 7 | No known copyright restrictions (Flickr Commons) |
/// | 8 | United States Government Work |
/// | 9 | CC0 |
/// | 10 | Public Domain Mark |
///
/// The non-commercial licences (1, 2, 3) and All Rights Reserved (0) are
/// excluded. That loses coverage, which is the point: an image the station
/// cannot lawfully publish is not coverage.
pub const ALLOWED_LICENSES: &str = "4,5,6,7,8,9,10";

/// Default thumbnail width, matching the Wikipedia provider so a station that
/// switches provider keeps one cache-key space and one rendered size.
pub const DEFAULT_THUMB_WIDTH: u32 = super::wikipedia::DEFAULT_THUMB_WIDTH;

/// Flickr image provider.
pub struct FlickrClient {
    http: reqwest::Client,
    api_key: String,
    /// The photographer to restrict the search to, as an email address. Held
    /// un-resolved because resolving it costs an API call, and a station
    /// whose species page is never opened should not make one.
    filter_email: Option<String>,
    /// The resolved Flickr NSID for [`Self::filter_email`]. A `OnceCell`
    /// rather than a field set at construction: it needs a network round trip,
    /// construction is synchronous, and the answer never changes.
    filter_user_id: OnceCell<Option<String>>,
    thumb_width: u32,
}

#[expect(
    clippy::missing_fields_in_debug,
    reason = "the omitted fields are the API key and the operator's address;               omitting them is the entire reason this impl is hand-written"
)]
impl std::fmt::Debug for FlickrClient {
    /// Hand-written so the API key cannot reach a log through `{:?}`.
    ///
    /// The derived implementation would print it in full, and this struct is
    /// exactly the kind of thing that ends up inside a `tracing::debug!` of
    /// some enclosing config during an unrelated investigation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlickrClient")
            .field("api_key", &"<redacted>")
            .field("filter_email", &self.filter_email.as_ref().map(|_| "<set>"))
            .field("thumb_width", &self.thumb_width)
            .finish()
    }
}

/// The sizes `extras=url_s,url_m,url_z,url_c` returns, smallest first, with the
/// long edge each one guarantees.
const SIZES: [(&str, u32); 4] = [
    ("url_s", 240),
    ("url_m", 500),
    ("url_z", 640),
    ("url_c", 800),
];
impl FlickrClient {
    /// A client for the given API key.
    ///
    /// # Errors
    ///
    /// [`ImageError::Http`] if the HTTP client cannot be built, and
    /// [`ImageError::Api`] for an empty key — which is worth refusing at
    /// construction rather than sending: Flickr answers a keyless request with
    /// a 200 and a JSON error body, so the failure would otherwise surface as
    /// "no image for this species" on every species, for ever.
    pub fn new(api_key: impl Into<String>) -> Result<Self, ImageError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ImageError::Api(
                "FLICKR_API_KEY is empty; set a key or use the wikipedia provider".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(super::IMAGE_DOWNLOAD_USER_AGENT)
            .build()
            .map_err(|e| ImageError::Http(e.to_string()))?;
        Ok(Self {
            http,
            api_key: api_key.trim().to_string(),
            filter_email: None,
            filter_user_id: OnceCell::new(),
            thumb_width: DEFAULT_THUMB_WIDTH,
        })
    }

    /// Restrict searches to one photographer's photostream.
    ///
    /// An empty or whitespace-only value clears the filter rather than
    /// searching for a photographer with no address: `FLICKR_FILTER_EMAIL=` in
    /// a config file means "not set", the same as omitting the line.
    #[must_use]
    pub fn with_filter_email(mut self, email: impl Into<String>) -> Self {
        let email = email.into();
        self.filter_email = (!email.trim().is_empty()).then(|| email.trim().to_string());
        self
    }

    /// Set the thumbnail width.
    #[must_use]
    pub const fn with_thumb_width(mut self, width: u32) -> Self {
        self.thumb_width = width;
        self
    }

    /// Resolve [`Self::filter_email`] to a Flickr NSID, once.
    ///
    /// `Ok(None)` when no filter is configured, and also when the address does
    /// not resolve — an unknown address means "search everyone" rather than
    /// "find nothing", because the alternative is a station that silently
    /// shows no photographs at all because of a typo in one config line.
    /// The miss is logged.
    async fn resolve_filter_user(&self) -> Option<String> {
        let email = self.filter_email.as_ref()?;
        self.filter_user_id
            .get_or_init(|| async {
                let url = format!(
                    "{FLICKR_API}?method=flickr.people.findByEmail&api_key={}\
                     &find_email={}&format=json&nojsoncallback=1",
                    url_encode(&self.api_key),
                    url_encode(email)
                );
                match self.get_json(&url).await {
                    Ok(json) => {
                        let id = json
                            .get("user")
                            .and_then(|u| u.get("nsid"))
                            .and_then(serde_json::Value::as_str)
                            .map(String::from);
                        if id.is_none() {
                            tracing::warn!(
                                "FLICKR_FILTER_EMAIL did not resolve to a Flickr account; \
                                 searching all of Flickr instead"
                            );
                        }
                        id
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "could not resolve FLICKR_FILTER_EMAIL; searching all of Flickr"
                        );
                        None
                    }
                }
            })
            .await
            .clone()
    }

    /// One GET returning parsed JSON, with the same retry shape as the
    /// Wikipedia provider.
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
                    let json: serde_json::Value =
                        serde_json::from_str(&body).map_err(|e| ImageError::Api(e.to_string()))?;
                    check_api_error(&json)?;
                    return Ok(json);
                }
                Ok(resp) => last_error = ImageError::Api(format!("HTTP {}", resp.status())),
                Err(e) => last_error = ImageError::Http(e.to_string()),
            }
        }
        Err(last_error)
    }

    /// The search URL for one species.
    ///
    /// Separated from the request so the query can be asserted without a
    /// network: the parameters *are* the behaviour of this provider, and a
    /// dropped `license` or `safe_search` is invisible in any test that only
    /// checks the parsing.
    pub(super) fn search_url(&self, scientific_name: &str, user_id: Option<&str>) -> String {
        let mut url = format!(
            "{FLICKR_API}?method=flickr.photos.search&api_key={}\
             &text={}&license={ALLOWED_LICENSES}&sort=relevance&media=photos\
             &content_types=0&safe_search=1&per_page=1&page=1\
             &extras=owner_name,license,url_s,url_m,url_z,url_c\
             &format=json&nojsoncallback=1",
            url_encode(&self.api_key),
            url_encode(scientific_name)
        );
        if let Some(id) = user_id {
            url.push_str("&user_id=");
            url.push_str(&url_encode(id));
        }
        url
    }

    /// Pull the first usable photo out of a `flickr.photos.search` response.
    ///
    /// Prefers the size closest to `thumb_width` from above, so the rendered
    /// thumbnail is downscaled rather than upscaled. Flickr's suffixes:
    /// `url_s` 240 px, `url_m` 500, `url_z` 640, `url_c` 800 on the long edge.
    pub(super) fn parse_search(
        json: &serde_json::Value,
        scientific_name: &str,
        thumb_width: u32,
    ) -> Result<SpeciesImage, ImageError> {
        let photo = json
            .get("photos")
            .and_then(|p| p.get("photo"))
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ImageError::Api("unexpected Flickr response structure".into()))?
            .first()
            .ok_or_else(|| ImageError::NotFound(scientific_name.to_string()))?;

        let str_field = |k: &str| photo.get(k).and_then(serde_json::Value::as_str);

        // Smallest-first, then take the first that is still at least the
        // target; fall back to the widest available when every size is
        // smaller.
        let url = SIZES
            .iter()
            .find(|(k, w)| *w >= thumb_width && str_field(k).is_some())
            .or_else(|| SIZES.iter().rev().find(|(k, _)| str_field(k).is_some()))
            .and_then(|(k, _)| str_field(k))
            .ok_or_else(|| ImageError::NotFound(scientific_name.to_string()))?
            .to_string();

        // The credit line. CC BY and CC BY-SA both require attribution, so an
        // image whose photographer Flickr did not name is refused rather than
        // shown uncredited.
        let owner = str_field("ownername")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ImageError::Api(format!(
                    "Flickr returned a photo for '{scientific_name}' with no photographer to \
                     credit; refusing to show it uncredited"
                ))
            })?;

        // The photo page, which is where the licence terms live and what an
        // attribution has to link to.
        let page_url = match (str_field("id"), str_field("owner")) {
            (Some(id), Some(nsid)) => Some(format!("https://www.flickr.com/photos/{nsid}/{id}/")),
            _ => None,
        };

        Ok(SpeciesImage {
            url,
            cached_path: None,
            width: thumb_width,
            description: Some(format!("Photograph by {owner}, via Flickr.")),
            wiki_url: page_url,
        })
    }
}

/// Turn Flickr's in-band error into a real one.
///
/// Flickr answers an API error with **HTTP 200** and
/// `{"stat":"fail","code":100,"message":"Invalid API Key"}`. Treated as
/// success, a bad or revoked key reads as "no image for this species" — on
/// every species, for ever, with nothing in the log. That is the single
/// most likely way this provider is misconfigured, so it is the one thing
/// worth pulling out where it can be tested without a network.
pub(super) fn check_api_error(json: &serde_json::Value) -> Result<(), ImageError> {
    if json.get("stat").and_then(serde_json::Value::as_str) == Some("fail") {
        let msg = json
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown Flickr API error");
        return Err(ImageError::Api(format!("Flickr: {msg}")));
    }
    Ok(())
}

impl ImageProvider for FlickrClient {
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
        Box::pin(async move {
            let user = self.resolve_filter_user().await;
            let url = self.search_url(&name, user.as_deref());
            let json = self.get_json(&url).await?;
            Self::parse_search(&json, &name, self.thumb_width)
        })
    }
}

#[cfg(test)]
mod tests;
