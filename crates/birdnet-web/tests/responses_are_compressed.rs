//! Text responses must be compressed; ranged and binary ones must not be.
//!
//! # What this is defending
//!
//! The station served **everything** uncompressed. Measured against a running
//! `examples/screenshot_server`, with `Accept-Encoding: gzip, br` on the
//! request, no response carried a `Content-Encoding` at all:
//!
//! ```text
//! /                    57 142 B   (16 025 gzipped)
//! /species             50 958 B   (13 011)
//! /recordings         109 150 B   (20 722)
//! /static/css/app.css 212 950 B   (43 224)
//! /static/htmx.min.js  50 917 B   (16 326)
//! ```
//!
//! A cold first load moved ~330 KB where ~75 KB would do — on a Pi's `WiFi` at
//! the end of a garden, or a phone on rural cellular, that is the difference
//! between a page appearing and a page you wait for.
//!
//! # Why the second half of this file matters as much as the first
//!
//! `tower_http`'s `DefaultPredicate` would have compressed the audio route's
//! `206 Partial Content` responses, whose `Content-Range` is computed from the
//! file and is *not* rewritten by the compressing layer — a corrupt clip in
//! every `<audio>` element that seeks. The allow-list predicate in
//! `server::should_compress` exists for that, so the negative assertions here
//! are the ones that keep it honest: a blanket "compress everything" change
//! passes the positive tests perfectly.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use std::io::Read as _;
use tower::ServiceExt as _;

/// A station with a real on-disk database. `AppState::new` runs the migrations,
/// so the pages have a schema to read.
fn station(dir: &std::path::Path) -> AppState {
    AppState::new(dir.join("birds.db")).expect("open state")
}

/// `(status, content-encoding, body bytes as they arrive on the wire)`.
async fn get(
    state: &AppState,
    path: &str,
    accept_encoding: Option<&str>,
) -> (StatusCode, String, Vec<u8>) {
    let mut req = Request::builder().uri(path);
    if let Some(enc) = accept_encoding {
        req = req.header(header::ACCEPT_ENCODING, enc);
    }
    let response = build_router(state.clone())
        .oneshot(req.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let encoding = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    (status, encoding, body)
}

/// Inflate a gzip body, failing the test with the wire bytes on error.
///
/// This is the assertion that matters. The first version of this file checked
/// only the `Content-Encoding` header — and passed while every page was
/// undecodable, because the compression layer sat *inside* the security-headers
/// middleware, which buffers `text/html` and runs `String::from_utf8_lossy`
/// over it. That turned gzip's `0x8b` magic byte into U+FFFD. The header said
/// `gzip`, the length was plausible, and no browser could render a single page.
fn gunzip(bytes: &[u8], what: &str) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .unwrap_or_else(|e| {
            panic!(
                "{what} claimed Content-Encoding: gzip but does not decode: {e}\n\
                 first 8 bytes on the wire: {:02x?} (gzip must start 1f 8b 08)",
                &bytes[..bytes.len().min(8)]
            )
        });
    out
}

#[tokio::test]
async fn html_and_css_are_gzipped_when_the_client_asks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = station(tmp.path());

    // `/` 303s to onboarding on a station with no detections, so the HTML case
    // is asserted on the page that redirect lands on.
    for path in ["/onboarding", "/static/css/app.css", "/static/htmx.min.js"] {
        let (status, encoding, wire) = get(&state, path, Some("gzip")).await;
        assert_eq!(status, StatusCode::OK, "{path} did not serve");
        assert_eq!(
            encoding, "gzip",
            "{path} was served uncompressed despite Accept-Encoding: gzip"
        );

        let plain = gunzip(&wire, path);
        assert!(!plain.is_empty(), "{path} decoded to nothing");
        assert!(
            wire.len() < plain.len(),
            "{path} got bigger: {} bytes on the wire for {} of content",
            wire.len(),
            plain.len()
        );

        // …and the decoded bytes must be the real page, not a mangled one. The
        // nonce is stamped by the security-headers middleware, so its presence
        // proves that layer ran on plaintext and this one ran after it.
        if path == "/onboarding" {
            let html = String::from_utf8(plain).expect("HTML is valid UTF-8 after the round trip");
            assert!(html.contains("<!DOCTYPE html>"), "not an HTML document");
            assert!(
                html.contains("<script nonce=\""),
                "the CSP nonce is missing — the security layer did not see this body"
            );
            assert!(
                !html.contains('\u{fffd}'),
                "the decoded page contains U+FFFD: a layer ran from_utf8_lossy over \
                 compressed bytes"
            );
        }
    }
}

#[tokio::test]
async fn a_client_that_does_not_ask_still_gets_plain_bytes() {
    // The counterpart. A layer that compressed unconditionally would break
    // every client that cannot decode, and would pass the test above.
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = station(tmp.path());

    let (status, encoding, wire) = get(&state, "/onboarding", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8_lossy(&wire).contains("<!DOCTYPE html>"),
        "a client that sent no Accept-Encoding did not get readable HTML"
    );
    assert_eq!(
        encoding, "",
        "compressed a response the client never said it could decode"
    );
}

#[tokio::test]
async fn json_is_compressed_and_the_health_probe_still_parses() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = station(tmp.path());

    let (status, encoding, wire) = get(&state, "/api/v2/health", Some("gzip")).await;
    assert!(status.is_success() || status == StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(encoding, "gzip", "the v2 API is not compressed");

    // An external monitor polls this endpoint and parses it. If the body does
    // not survive the round trip, the station reports nothing at all.
    let json = gunzip(&wire, "/api/v2/health");
    let text = String::from_utf8(json).expect("JSON is valid UTF-8");
    assert!(
        text.contains("\"status\""),
        "health JSON did not survive compression: {text}"
    );
}

#[tokio::test]
async fn a_range_request_is_never_compressed() {
    // A 206 carries a Content-Range describing byte offsets in the *original*
    // representation. Compressing the body without rewriting that header hands
    // the browser a clip whose length disagrees with its own header.
    //
    // A real file on disk, so this exercises the 206 path rather than the 404
    // one — the first draft of this test asserted against a 404 `text/plain`
    // body, which is compressed and harmlessly so, and reported a failure that
    // was not the one it names.
    let tmp = tempfile::tempdir().expect("tempdir");
    let rec_dir = tmp.path().join("recordings");
    std::fs::create_dir_all(&rec_dir).expect("recordings dir");
    // Compressible content, so a compressing layer would visibly change the
    // length rather than coincidentally leaving it alone.
    std::fs::write(rec_dir.join("clip.wav"), vec![0u8; 64 * 1024]).expect("clip");
    let state = station(tmp.path()).with_recording_dir(rec_dir);

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v2/recordings/clip.wav")
                .header(header::ACCEPT_ENCODING, "gzip")
                .header(header::RANGE, "bytes=0-1023")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(
        response.status(),
        StatusCode::PARTIAL_CONTENT,
        "expected a 206 so the Content-Range case is actually exercised"
    );
    assert!(
        response.headers().contains_key(header::CONTENT_RANGE),
        "a 206 without Content-Range is not the case under test"
    );
    assert!(
        !response.headers().contains_key(header::CONTENT_ENCODING),
        "a Range request came back compressed — Content-Range would be a lie"
    );
}

#[tokio::test]
async fn the_predicate_is_an_allow_list_not_a_deny_list() {
    // Spectrogram PNGs are now genuinely deflated (see routes::spectrogram::png),
    // so gzipping them would spend Pi CPU to add bytes. `image/svg+xml` is the
    // deliberate exception — every chart in this app is inline SVG markup.
    //
    // Asserted through the shipped predicate rather than over the wire, because
    // producing a real PNG response needs a recording on disk and this is a
    // statement about the policy, not about that route.
    use axum::http::{Extensions, HeaderMap, HeaderValue, Version};

    fn ok_with(ctype: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(ctype).expect("header"),
        );
        h
    }

    let compress = |ctype: &str| {
        birdnet_web::server::should_compress(
            StatusCode::OK,
            Version::HTTP_11,
            &ok_with(ctype),
            &Extensions::new(),
        )
    };

    assert!(compress("text/html; charset=utf-8"));
    assert!(compress("application/json"));
    assert!(compress("image/svg+xml"));
    assert!(compress("text/css; charset=utf-8"));

    assert!(!compress("image/png"), "PNGs are already deflated");
    assert!(!compress("audio/wav"));
    assert!(!compress("application/gzip"), "the backup tarball");
    assert!(
        !compress("text/event-stream"),
        "SSE must stay unbuffered — the live log viewer depends on it"
    );
}
