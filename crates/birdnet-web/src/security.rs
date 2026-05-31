//! Security middleware: response-hardening headers and a stateless CSRF guard.
//!
//! The web UI authenticates with HTTP Basic Auth and keeps no cookies or
//! sessions, so the classic CSRF vector — a malicious page auto-submitting a
//! form to an admin endpoint while the browser silently attaches the cached
//! credentials — is mitigated here by a same-origin check on state-changing
//! requests rather than by per-form synchroniser tokens. This is the
//! OWASP-recommended defence for an app without a session token to bind to.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Upper bound on an HTML body we will buffer to stamp script nonces into.
/// Page/fragment responses are far smaller; anything larger is passed through
/// untouched (it is not one of our rendered pages). Non-HTML responses — the
/// audio `/stream`, the live WebSocket, JSON, images — are never buffered.
const MAX_HTML_REWRITE_BYTES: usize = 8 * 1024 * 1024;

/// `Content-Security-Policy` for the server-rendered HTMX UI, carrying the
/// per-request script nonce.
///
/// `script-src 'nonce-…' 'strict-dynamic'` admits exactly the parser-inserted
/// `<script>`s we stamp the nonce onto; `'strict-dynamic'` then lets those
/// trusted scripts (htmx) inject further scripts — e.g. into HTMX-swapped
/// fragments — without re-opening an inline-script free-for-all. `'unsafe-inline'`
/// is gone for scripts; it remains under `style-src` only (inline `style="…"`
/// attributes, tightened in a later pass). `connect-src 'self'` covers the
/// same-origin live WebSocket (`ws://<host>/…`).
fn content_security_policy(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         img-src 'self' data: https:; \
         style-src 'self' 'unsafe-inline'; \
         script-src 'nonce-{nonce}' 'strict-dynamic'; \
         connect-src 'self'; \
         font-src 'self'; \
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'self'; \
         form-action 'self'"
    )
}

/// Mint a fresh 128-bit CSP nonce (base64, unpadded).
fn generate_nonce() -> String {
    use base64::Engine;
    use password_hash::rand_core::{OsRng, RngCore};
    let mut bytes = [0_u8; 16];
    if OsRng.try_fill_bytes(&mut bytes).is_ok() {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
    } else {
        // OsRng failure is essentially impossible on Linux; defer to the
        // session-id generator (its own documented fail-secure fallback)
        // rather than ever hand out an empty, CSP-breaking nonce.
        crate::session::generate_session_id()
    }
}

/// Stamp `nonce="…"` onto every parser-inserted `<script>` opening tag in an
/// HTML body. Two non-overlapping passes — attributed tags (`<script src=…>`,
/// `<script defer>`, `<script type=…>`) first, then the attribute-less
/// `<script>` — so neither re-matches the other's output, and `</script>` is
/// never touched. The CSP is `'strict-dynamic'`, so any further scripts these
/// trusted ones go on to inject (e.g. into HTMX-swapped fragments) are admitted
/// without a nonce of their own.
fn inject_script_nonce(html: &str, nonce: &str) -> String {
    html.replace("<script ", &format!("<script nonce=\"{nonce}\" "))
        .replace("<script>", &format!("<script nonce=\"{nonce}\">"))
}

/// Attach defence-in-depth response headers to every response.
///
/// Added as the outermost layer so it decorates errors (401/404/429), static
/// files, and handler responses alike. No HSTS: the binary serves plain HTTP
/// and a reverse proxy is expected to terminate TLS and own that header.
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    // One per-request CSP nonce, minted here so a single place owns the whole
    // dance: it is stamped onto every parser-inserted <script> of an HTML
    // response body (below) and mirrored into that response's script-src. No
    // page renderer threads it, so a new page or admin shell can't silently
    // ship an un-nonced inline script.
    let nonce = generate_nonce();

    let res = next.run(req).await;
    let (mut parts, body) = res.into_parts();

    // Only rewrite HTML. Non-HTML responses — the audio `/stream`, the live
    // WebSocket upgrade, JSON, images, static assets — pass through untouched
    // and, crucially, are never buffered.
    let is_html = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));

    let body = if is_html {
        match axum::body::to_bytes(body, MAX_HTML_REWRITE_BYTES).await {
            Ok(bytes) => {
                let stamped = inject_script_nonce(String::from_utf8_lossy(&bytes).as_ref(), &nonce);
                // The body length changed; let the stack recompute it.
                parts.headers.remove(header::CONTENT_LENGTH);
                axum::body::Body::from(stamped)
            }
            // Oversized or a mid-stream error: the body is consumed and can't be
            // safely rewritten. Fail closed with an empty body rather than ship
            // un-nonced HTML the new CSP would break in the browser anyway.
            Err(_) => axum::body::Body::empty(),
        }
    } else {
        body
    };

    // Allow a handler to set its own CSP (escape hatch); otherwise apply ours.
    parts
        .headers
        .entry(header::CONTENT_SECURITY_POLICY)
        .or_insert_with(|| {
            HeaderValue::from_str(&content_security_policy(&nonce))
                .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'"))
        });
    parts.headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    parts.headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    parts.headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );

    Response::from_parts(parts, body)
}

/// Reject state-changing requests whose `Origin`/`Referer` is cross-site.
pub async fn csrf_guard_middleware(req: Request, next: Next) -> Response {
    if is_state_changing(req.method()) && !is_same_origin(req.headers()) {
        return (
            StatusCode::FORBIDDEN,
            "Cross-origin request blocked by CSRF protection. \
             Submit this request from the BirdNet-Behavior web UI.",
        )
            .into_response();
    }
    next.run(req).await
}

fn is_state_changing(method: &Method) -> bool {
    // `http::Method` is not a fieldless enum, so its associated consts cannot
    // be used as match patterns; compare the canonical token instead.
    matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
}

/// True if the request is safe from a CSRF standpoint: either it carries no
/// browser origin information (a non-browser client — the CLI, a script, or
/// `curl` — which a remote site cannot coerce into a cross-site request), or
/// its `Origin`/`Referer` authority matches the `Host` it was sent to.
fn is_same_origin(headers: &HeaderMap) -> bool {
    let Some(host) = header_str(headers, &header::HOST) else {
        // No Host header to compare against; nothing we can verify.
        return true;
    };
    if let Some(origin) = header_str(headers, &header::ORIGIN) {
        return authority_matches(origin, host);
    }
    if let Some(referer) = header_str(headers, &header::REFERER) {
        return authority_matches(referer, host);
    }
    // Neither Origin nor Referer present: not a browser-driven cross-site
    // submission (a browser always attaches Origin to a cross-origin POST).
    true
}

fn header_str<'a>(headers: &'a HeaderMap, name: &header::HeaderName) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

/// Compare the authority (`host[:port]`) of a URL (an `Origin` or `Referer`
/// value) against a `Host` header. A scheme-less or malformed value — including
/// the opaque `"null"` origin — does not match, so it is rejected.
fn authority_matches(url: &str, host: &str) -> bool {
    let Some((_scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    !authority.is_empty() && authority == host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_script_nonce_stamps_each_opening_tag_once() {
        let html = concat!(
            r#"<script src="/a.js"></script>"#,
            r#"<script>x()</script>"#,
            r#"<script defer src="/b.js"></script>"#,
        );
        let out = inject_script_nonce(html, "ABC");
        // Every opening tag stamped exactly once — and never double-stamped.
        assert_eq!(out.matches(r#"nonce="ABC""#).count(), 3);
        assert!(!out.contains(r#"nonce="ABC" nonce="#));
        assert!(out.contains(r#"<script nonce="ABC" src="/a.js">"#));
        assert!(out.contains(r#"<script nonce="ABC">x()"#));
        assert!(out.contains(r#"<script nonce="ABC" defer src="/b.js">"#));
        // Close tags are left alone.
        assert_eq!(out.matches("</script>").count(), 3);
    }

    #[test]
    fn inject_script_nonce_leaves_non_script_markup_unchanged() {
        let html = "<p>hi</p><div data-x=\"scripted\">no tags here</div>";
        assert_eq!(inject_script_nonce(html, "N"), html);
    }

    #[test]
    fn content_security_policy_carries_nonce_and_strict_dynamic() {
        let csp = content_security_policy("XYZ");
        assert!(csp.contains("script-src 'nonce-XYZ' 'strict-dynamic'"));
        // 'unsafe-inline' must be gone for scripts (kept only under style-src).
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
    }

    #[test]
    fn authority_matches_exact_host_and_port() {
        assert!(authority_matches(
            "http://192.168.1.5:8502",
            "192.168.1.5:8502"
        ));
        assert!(authority_matches(
            "https://birds.example.com",
            "birds.example.com"
        ));
        // Trailing path is ignored (Referer carries the full URL).
        assert!(authority_matches(
            "http://localhost:8502/admin/settings",
            "localhost:8502"
        ));
    }

    #[test]
    fn authority_mismatch_is_rejected() {
        assert!(!authority_matches(
            "https://evil.example.com",
            "birds.example.com"
        ));
        assert!(!authority_matches(
            "http://localhost:9999",
            "localhost:8502"
        ));
        // A missing port is a different authority.
        assert!(!authority_matches("http://localhost", "localhost:8502"));
    }

    #[test]
    fn malformed_or_null_origin_is_rejected() {
        assert!(!authority_matches("null", "localhost:8502"));
        assert!(!authority_matches("", "localhost:8502"));
        // Scheme-less value never matches.
        assert!(!authority_matches("localhost:8502", "localhost:8502"));
    }

    #[test]
    fn state_changing_methods_are_classified() {
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(is_state_changing(&m), "{m} should be state-changing");
        }
        for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_state_changing(&m), "{m} should be safe");
        }
    }

    fn hmap(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        h
    }

    #[test]
    fn no_browser_headers_is_allowed() {
        // No Host at all.
        assert!(is_same_origin(&HeaderMap::new()));
        // Host but neither Origin nor Referer (curl / CLI / a script).
        assert!(is_same_origin(&hmap(&[(header::HOST, "localhost:8502")])));
    }

    #[test]
    fn matching_origin_is_allowed() {
        assert!(is_same_origin(&hmap(&[
            (header::HOST, "localhost:8502"),
            (header::ORIGIN, "http://localhost:8502"),
        ])));
    }

    #[test]
    fn cross_site_origin_is_blocked() {
        assert!(!is_same_origin(&hmap(&[
            (header::HOST, "localhost:8502"),
            (header::ORIGIN, "http://evil.test"),
        ])));
    }

    #[test]
    fn referer_is_used_when_origin_absent() {
        assert!(is_same_origin(&hmap(&[
            (header::HOST, "pi.local:8502"),
            (header::REFERER, "http://pi.local:8502/admin/settings"),
        ])));
        assert!(!is_same_origin(&hmap(&[
            (header::HOST, "pi.local:8502"),
            (header::REFERER, "http://evil.test/x"),
        ])));
    }

    #[test]
    fn origin_takes_precedence_over_referer() {
        // A present, valid Origin is authoritative; the mismatched Referer is
        // not consulted.
        assert!(is_same_origin(&hmap(&[
            (header::HOST, "localhost:8502"),
            (header::ORIGIN, "http://localhost:8502"),
            (header::REFERER, "http://evil.test/x"),
        ])));
    }
}
