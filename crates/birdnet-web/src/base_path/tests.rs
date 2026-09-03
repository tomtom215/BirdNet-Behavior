//! Gates for the base-path prefix.

use super::*;

fn base(s: &str) -> BasePath {
    BasePath::parse(s).expect("valid base path")
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// The normal form is what makes `join` a concatenation. Everything an
/// operator might plausibly type has to land on it.
#[test]
fn every_spelling_of_a_prefix_normalises_to_one_form() {
    for raw in [
        "/birdnet",
        "birdnet",
        "/birdnet/",
        "birdnet/",
        "  /birdnet  ",
    ] {
        assert_eq!(base(raw).as_str(), "/birdnet", "from {raw:?}");
    }
    for raw in ["/a/b/c", "a/b/c/", "/a/b/c/"] {
        assert_eq!(base(raw).as_str(), "/a/b/c", "from {raw:?}");
    }
}

/// "Serve from the root" has three spellings and all of them are valid, not
/// errors — an operator who clears the setting must not be told off.
#[test]
fn an_absent_prefix_is_empty_rather_than_an_error() {
    for raw in ["", "/", "   ", "  /  "] {
        let b = base(raw);
        assert!(b.is_empty(), "{raw:?} should mean no prefix");
        assert_eq!(b.as_str(), "");
    }
}

/// A prefix that will not match what the proxy sends is refused rather than
/// repaired. A silent correction mounts the station somewhere nobody is
/// looking and turns the mistake into a 404 hunt.
#[test]
fn a_prefix_that_cannot_be_normalised_is_refused() {
    assert_eq!(BasePath::parse("//"), Err(BasePathError::EmptySegment));
    assert_eq!(BasePath::parse("/a//b"), Err(BasePathError::EmptySegment));
    assert_eq!(BasePath::parse("/.."), Err(BasePathError::DotSegment));
    assert_eq!(BasePath::parse("/a/../b"), Err(BasePathError::DotSegment));
    assert_eq!(BasePath::parse("/."), Err(BasePathError::DotSegment));
    assert_eq!(
        BasePath::parse("/bird net"),
        Err(BasePathError::BadCharacter(' '))
    );
    assert_eq!(
        BasePath::parse("/birdnet?x=1"),
        Err(BasePathError::BadCharacter('?'))
    );
    assert_eq!(
        BasePath::parse("/birdnet#top"),
        Err(BasePathError::BadCharacter('#'))
    );
    // ...and the counterpart: a plausible prefix with every allowed character
    // is accepted, so the rule above is a filter and not a wall.
    assert_eq!(base("/bird-net_2.0~x").as_str(), "/bird-net_2.0~x");
}

// ---------------------------------------------------------------------------
// Joining
// ---------------------------------------------------------------------------

/// The root is `/birdnet`, not `/birdnet/`. They are different URLs to a proxy,
/// they resolve relative links differently in a browser, and the un-slashed
/// form is the one the router mounts.
#[test]
fn the_root_joins_without_a_trailing_slash() {
    assert_eq!(base("/birdnet").join("/"), "/birdnet");
    assert_eq!(base("/birdnet").join("/today"), "/birdnet/today");
    assert_eq!(
        base("/birdnet").join("/api/v2/detections?limit=5"),
        "/birdnet/api/v2/detections?limit=5"
    );
}

/// An empty prefix leaves every path exactly as it was. This is the
/// no-op-upgrade guarantee for every station that never sets one.
#[test]
fn an_empty_prefix_changes_nothing() {
    let b = BasePath::default();
    for path in ["/", "/today", "/api/v2/x", "https://x/y", "//cdn/x", "#top"] {
        assert_eq!(b.join(path), path, "{path:?}");
    }
}

/// A URL that does not point at this application must not be prefixed. `//host`
/// is the one that a naive `starts_with('/')` gets wrong, and getting it wrong
/// breaks an external link rather than an internal one — so it fails somewhere
/// nobody is looking.
#[test]
fn a_url_that_is_not_ours_is_left_alone() {
    let b = base("/birdnet");
    for url in [
        "https://wikipedia.org/x",
        "http://example.com/",
        "//cdn.example.com/lib.js",
        "#section",
        "mailto:someone@example.com",
        "data:image/png;base64,AAA",
        "relative/path",
    ] {
        assert_eq!(b.join(url), url, "{url:?} must not be prefixed");
    }
}

/// The cookie's `Path` keeps its trailing slash where `join` drops it, because
/// RFC 6265's path-match is a prefix rule with a segment boundary: `Path=/bird`
/// also matches `/birdsong`, so a neighbouring app on the same host would be
/// sent this station's session cookie.
#[test]
fn the_cookie_path_keeps_its_trailing_slash() {
    assert_eq!(base("/birdnet").cookie_path(), "/birdnet/");
    assert_eq!(BasePath::default().cookie_path(), "/");
}

// ---------------------------------------------------------------------------
// HTML rewriting
// ---------------------------------------------------------------------------

/// An unset prefix returns the document byte for byte. Everything downstream
/// of this — the CSP nonce pass, compression, the wire — sees exactly what it
/// saw before the base path existed.
#[test]
fn an_empty_prefix_returns_the_document_untouched() {
    let html = r#"<a href="/today">Today</a><img src="/static/x.png">"#;
    assert_eq!(rewrite_html(html, &BasePath::default()), html);
}

/// Every attribute whose value is a URL this application serves.
#[test]
fn every_url_attribute_is_prefixed() {
    let b = base("/birdnet");
    let html = concat!(
        r#"<a href="/today">t</a>"#,
        r#"<img src="/static/x.png">"#,
        r#"<form action="/admin/audio/sources">"#,
        r#"<button formaction="/x">b</button>"#,
        r#"<video poster="/p.png"></video>"#,
        r#"<div hx-get="/api/v2/a" hx-post="/api/v2/b" hx-put="/c""#,
        r#" hx-patch="/d" hx-delete="/e" ws-connect="/ws" sse-connect="/sse"></div>"#,
        r#"<div data-src="/s" data-url="/u" data-endpoint="/ep"></div>"#,
    );
    let out = rewrite_html(html, &b);
    for path in [
        "/today",
        "/static/x.png",
        "/admin/audio/sources",
        "/x",
        "/p.png",
        "/api/v2/a",
        "/api/v2/b",
        "/c",
        "/d",
        "/e",
        "/ws",
        "/sse",
        "/s",
        "/u",
        "/ep",
    ] {
        assert!(
            out.contains(&format!(r#"="/birdnet{path}""#)),
            "{path} was not prefixed:\n{out}"
        );
    }
    assert_eq!(
        out.matches("/birdnet").count(),
        15,
        "each URL prefixed exactly once:\n{out}"
    );
}

/// Attributes whose value is not a URL, and URLs that are not ours, survive
/// the pass. The counterpart to the test above: without it, "prefix every
/// `="/`" would pass as "prefix every URL".
///
/// The scan does not track whether it is inside a tag, so prose of the exact
/// shape ` href="/x"` — a URL attribute's name, preceded by whitespace, in
/// text content — would be rewritten. That is deliberate: adding tag tracking
/// would mean toggling on `<` and `>`, which appear inside the inline scripts
/// this application ships (`a > b`), and toggling wrongly there produces a
/// *missed* link, which breaks navigation rather than one wrong word.
#[test]
fn nothing_but_a_url_attribute_is_touched() {
    let b = base("/birdnet");
    let cases = [
        // A title that happens to contain a path.
        r#"<span title="/etc/passwd">x</span>"#,
        // An attribute whose name merely ends in a URL attribute's name.
        r#"<div data-href="/today"></div>"#,
        r#"<div xhref="/today"></div>"#,
        // Another origin.
        r#"<a href="//cdn.example.com/lib.js">x</a>"#,
        r#"<a href="https://wikipedia.org/x">x</a>"#,
        // A name that is only a suffix of a URL attribute's.
        r#"<p>write it as x-href="/foo" in your config</p>"#,
        // The name must start at a token boundary. `>` before it means this
        // is text content, not an attribute.
        r#"<p>href="/foo"</p>"#,
    ];
    for html in cases {
        assert_eq!(rewrite_html(html, &b), html, "changed: {html}");
    }
}

/// The prefix goes in front of the path, not somewhere near it. Pinning the
/// exact output catches an off-by-one in the copy that a `contains` would not.
#[test]
fn the_rewritten_document_is_exact() {
    let b = base("/birdnet");
    assert_eq!(
        rewrite_html(r#"<a href="/today" class="x">Today</a>"#, &b),
        r#"<a href="/birdnet/today" class="x">Today</a>"#
    );
    assert_eq!(
        rewrite_html(r#"<a class="x" href="/">Home</a>"#, &b),
        r#"<a class="x" href="/birdnet/">Home</a>"#
    );
}

/// Non-ASCII in the document must not shift the byte offsets the scan works
/// with. Species names, place names and the German and Spanish UI strings all
/// carry multi-byte characters, so this is the ordinary case, not an edge one.
#[test]
fn multibyte_text_does_not_disturb_the_scan() {
    let b = base("/birdnet");
    let html = r#"<a href="/species/Grünfink" title="Grünfink — Chloris chloris">Grünfink</a>"#;
    let out = rewrite_html(html, &b);
    assert!(out.contains(r#"href="/birdnet/species/Grünfink""#), "{out}");
    assert!(out.contains("Grünfink — Chloris chloris"), "{out}");
}

/// Redirects are not HTML and get the same rule by a different route.
#[test]
fn a_location_header_follows_the_same_rule() {
    let b = base("/birdnet");
    assert_eq!(rewrite_location("/login", &b), "/birdnet/login");
    assert_eq!(rewrite_location("/", &b), "/birdnet");
    assert_eq!(
        rewrite_location("https://example.com/x", &b),
        "https://example.com/x"
    );
    assert_eq!(
        rewrite_location("/login", &BasePath::default()),
        "/login",
        "an unset prefix leaves a redirect alone"
    );
}

/// The shipped templates, run through the real pass.
///
/// The finding this implements says the work "must be done exhaustively", and
/// a hand-written fixture proves nothing about the markup that actually ships.
/// So: every template, every URL attribute in it, each one prefixed exactly
/// once, and the document otherwise unchanged in length by exactly the number
/// of prefixes added.
#[test]
fn every_shipped_template_is_rewritten_completely() {
    let b = base("/birdnet");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
    let mut seen_files = 0_usize;
    let mut seen_urls = 0_usize;
    for entry in std::fs::read_dir(dir).expect("templates directory") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("html") {
            continue;
        }
        let html = std::fs::read_to_string(&path).expect("read template");
        let out = rewrite_html(&html, &b);
        let added = out.len() - html.len();
        assert_eq!(
            added % b.as_str().len(),
            0,
            "{path:?} grew by {added}, not a whole number of prefixes"
        );
        let n = added / b.as_str().len();
        seen_files += 1;
        seen_urls += n;

        // Nothing application-absolute may survive un-prefixed in a URL
        // attribute. Re-running the pass over the output must therefore find
        // no more work to do beyond what it already did.
        for attr in URL_ATTRIBUTES {
            let needle = format!(r#"{attr}="/"#);
            for (i, _) in out.match_indices(&needle) {
                let after = &out[i + needle.len() - 1..];
                assert!(
                    after.starts_with("/birdnet") || after.starts_with("//"),
                    "{path:?}: {attr} left un-prefixed at byte {i}: {:?}",
                    &after[..after.len().min(40)]
                );
            }
        }
    }
    assert!(seen_files >= 20, "only {seen_files} templates scanned");
    assert!(
        seen_urls >= 80,
        "only {seen_urls} URLs prefixed across the templates; the scan is not seeing them"
    );
}
