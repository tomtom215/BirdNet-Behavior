//! Gates for the Flickr provider.
//!
//! Everything here runs without a network. The two things worth testing are
//! the request this provider *makes* — its query parameters are its whole
//! behaviour, and a dropped `license` is invisible to any test that only
//! checks parsing — and what it does with a response, including the responses
//! Flickr sends that look like successes.

use super::*;

fn client() -> FlickrClient {
    FlickrClient::new("KEY123").expect("a non-empty key builds")
}

fn search_json(photos: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "photos": { "photo": photos.clone() }, "stat": "ok" })
}

fn photo() -> serde_json::Value {
    serde_json::json!({
        "id": "5432",
        "owner": "12345@N01",
        "ownername": "A Photographer",
        "license": "4",
        "url_s": "https://live.staticflickr.com/1/5432_s.jpg",
        "url_m": "https://live.staticflickr.com/1/5432_m.jpg",
        "url_z": "https://live.staticflickr.com/1/5432_z.jpg",
        "url_c": "https://live.staticflickr.com/1/5432_c.jpg",
    })
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// A keyless request gets HTTP 200 and a JSON error body, so an empty key
/// would read as "no image for this species" on every species rather than as a
/// configuration mistake. Refuse it where the operator can still see why.
#[test]
fn an_empty_api_key_is_refused_at_construction() {
    assert!(matches!(FlickrClient::new(""), Err(ImageError::Api(_))));
    assert!(matches!(FlickrClient::new("   "), Err(ImageError::Api(_))));
    assert!(FlickrClient::new("k").is_ok(), "a real key still builds");
}

/// `FLICKR_FILTER_EMAIL=` in a config file means "not set". Treated as a
/// value, it would send the station looking for a photographer with no address
/// and it would find nothing, on every species.
#[test]
fn an_empty_filter_email_clears_the_filter_rather_than_setting_it() {
    assert!(client().with_filter_email("").filter_email.is_none());
    assert!(client().with_filter_email("  ").filter_email.is_none());
    assert_eq!(
        client().with_filter_email(" me@example.com ").filter_email,
        Some("me@example.com".to_string()),
        "and a real address is kept, trimmed"
    );
}

/// The API key must not reach a log through a `{:?}` of some enclosing struct.
#[test]
fn the_debug_rendering_does_not_carry_the_key() {
    let rendered = format!("{:?}", client().with_filter_email("me@example.com"));
    assert!(
        !rendered.contains("KEY123"),
        "the API key leaked into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("me@example.com"),
        "and so did the operator's address: {rendered}"
    );
    assert!(rendered.contains("FlickrClient"), "still identifiable");
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// The query parameters are this provider's behaviour. Each one here changes
/// what an operator's station shows or what it is allowed to show, so each is
/// asserted rather than left to the reader of the format string.
#[test]
fn the_search_request_carries_every_parameter_that_matters() {
    let url = client().search_url("Turdus merula", None);
    assert!(url.starts_with(FLICKR_API), "{url}");
    assert!(url.contains("method=flickr.photos.search"), "{url}");
    assert!(url.contains("api_key=KEY123"), "{url}");
    assert!(
        url.contains("text=Turdus%20merula"),
        "the species name must be encoded: {url}"
    );
    assert!(
        url.contains("license=4,5,6,7,8,9,10"),
        "without the licence filter the station shows images it may not publish: {url}"
    );
    assert!(url.contains("sort=relevance"), "{url}");
    assert!(
        url.contains("safe_search=1"),
        "a bird station must not surface adult content on a species page: {url}"
    );
    assert!(url.contains("media=photos"), "{url}");
    assert!(
        url.contains("content_types=0"),
        "photographs only, not screenshots or artwork: {url}"
    );
    assert!(
        url.contains("per_page=1"),
        "one result is all that is used: {url}"
    );
    assert!(
        url.contains("extras=owner_name,license,url_s,url_m,url_z,url_c"),
        "the sizes and the photographer come back with the search, \
         so no second call is needed: {url}"
    );
    assert!(
        url.contains("nojsoncallback=1"),
        "raw JSON, not JSONP: {url}"
    );
}

/// The photostream filter is present when configured and absent when not.
/// Both halves: an always-present `user_id` would silently search one empty
/// stream, and an always-absent one would ignore the setting.
#[test]
fn the_photostream_filter_appears_only_when_it_is_set() {
    let c = client();
    assert!(
        !c.search_url("Turdus merula", None).contains("user_id="),
        "no filter configured means no user_id"
    );
    let filtered = c.search_url("Turdus merula", Some("12345@N01"));
    assert!(
        filtered.contains("user_id=12345%40N01"),
        "the NSID must be encoded — it contains an @: {filtered}"
    );
}

// ---------------------------------------------------------------------------
// The response
// ---------------------------------------------------------------------------

/// Flickr's error is an HTTP 200 with `stat: fail`. This is the single most
/// likely misconfiguration — a wrong or revoked key — and treating it as
/// success hides it behind "no image for this species", for ever, on every
/// species.
#[test]
fn an_in_band_api_error_is_an_error() {
    let failed = serde_json::json!({
        "stat": "fail", "code": 100, "message": "Invalid API Key"
    });
    let err = check_api_error(&failed).expect_err("stat:fail must not pass");
    assert!(
        err.to_string().contains("Invalid API Key"),
        "the operator needs Flickr's own words: {err}"
    );
    // The counterpart, so this is not just "every response is an error".
    check_api_error(&search_json(&serde_json::json!([photo()]))).expect("stat:ok passes");
}

/// The smallest size that is still at least as wide as the thumbnail, so the
/// rendered image is downscaled rather than blown up.
#[test]
fn the_chosen_size_is_the_smallest_one_big_enough() {
    let json = search_json(&serde_json::json!([photo()]));
    for (want_width, expect) in [
        (240_u32, "_s"),
        (300, "_m"),
        (500, "_m"),
        (640, "_z"),
        (800, "_c"),
    ] {
        let img = FlickrClient::parse_search(&json, "Turdus merula", want_width).expect("parses");
        assert!(
            img.url.ends_with(&format!("{expect}.jpg")),
            "at {want_width} px wanted {expect}, got {}",
            img.url
        );
        assert_eq!(
            img.width, want_width,
            "the recorded width is the requested one"
        );
    }
}

/// When every offered size is smaller than the thumbnail, take the largest
/// rather than failing: a 240 px photograph beats no photograph.
#[test]
fn a_photo_with_only_small_sizes_still_yields_the_largest() {
    let small = serde_json::json!({
        "id": "1", "owner": "12345@N01", "ownername": "P",
        "url_s": "https://live.staticflickr.com/1/1_s.jpg",
        "url_m": "https://live.staticflickr.com/1/1_m.jpg",
    });
    let img = FlickrClient::parse_search(&search_json(&serde_json::json!([small])), "X", 1600)
        .expect("parses");
    assert!(img.url.ends_with("_m.jpg"), "got {}", img.url);
}

/// CC BY and CC BY-SA both require attribution. An image whose photographer
/// Flickr did not name cannot be shown lawfully, so it is refused rather than
/// displayed uncredited.
#[test]
fn a_photo_with_no_photographer_to_credit_is_refused() {
    let anonymous = serde_json::json!({
        "id": "1", "owner": "12345@N01",
        "url_m": "https://live.staticflickr.com/1/1_m.jpg",
    });
    let err = FlickrClient::parse_search(&search_json(&serde_json::json!([anonymous])), "X", 300)
        .expect_err("an uncredited photo must be refused");
    assert!(err.to_string().contains("uncredited"), "and say why: {err}");
    // Counterpart: the same photo *with* a name is accepted, so the refusal is
    // about the credit and not about the shape of the record.
    let mut named = anonymous;
    named["ownername"] = serde_json::json!("A Photographer");
    assert!(
        FlickrClient::parse_search(&search_json(&serde_json::json!([named])), "X", 300).is_ok()
    );
}

/// The credit line and the link to the licence terms. Both are what makes the
/// image lawful to show, so both are pinned exactly.
#[test]
fn the_image_carries_its_attribution() {
    let img = FlickrClient::parse_search(&search_json(&serde_json::json!([photo()])), "X", 300)
        .expect("parses");
    assert_eq!(
        img.description.as_deref(),
        Some("Photograph by A Photographer, via Flickr."),
        "the credit line the species page renders"
    );
    assert_eq!(
        img.wiki_url.as_deref(),
        Some("https://www.flickr.com/photos/12345@N01/5432/"),
        "and a link to the photo page, where the licence terms live"
    );
}

/// A species Flickr has nothing for is `NotFound`, not a malformed-response
/// error — the caller distinguishes them, and only `NotFound` should fall
/// through to the next provider.
#[test]
fn an_empty_result_is_not_found_rather_than_a_parse_failure() {
    let err =
        FlickrClient::parse_search(&search_json(&serde_json::json!([])), "Turdus merula", 300)
            .expect_err("no photos means not found");
    assert!(
        matches!(err, ImageError::NotFound(ref s) if s == "Turdus merula"),
        "got {err:?}"
    );
}

/// A response that is not shaped like a search result is an API error, which
/// is a different thing from "this species has no photograph" and must not be
/// mistaken for one.
#[test]
fn a_malformed_response_is_an_api_error() {
    let err = FlickrClient::parse_search(&serde_json::json!({"nonsense": 1}), "X", 300)
        .expect_err("a malformed body must not read as not-found");
    assert!(matches!(err, ImageError::Api(_)), "got {err:?}");
}
