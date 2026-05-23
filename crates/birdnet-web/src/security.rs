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

/// `Content-Security-Policy` for the server-rendered HTMX UI.
///
/// `'unsafe-inline'` is permitted for styles and scripts because the pages use
/// inline `style="…"` attributes and a few small inline bootstrap scripts; the
/// policy still blocks off-origin script/object loads and restricts framing.
/// `connect-src 'self'` covers the same-origin live WebSocket (`ws://<host>/…`).
/// Tighten to nonce/hash-based `script-src` in a later pass.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; \
img-src 'self' data: https:; \
style-src 'self' 'unsafe-inline'; \
script-src 'self' 'unsafe-inline'; \
connect-src 'self'; \
font-src 'self'; \
object-src 'none'; \
base-uri 'self'; \
frame-ancestors 'self'; \
form-action 'self'";

/// Attach defence-in-depth response headers to every response.
///
/// Added as the outermost layer so it decorates errors (401/404/429), static
/// files, and handler responses alike. No HSTS: the binary serves plain HTTP
/// and a reverse proxy is expected to terminate TLS and own that header.
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let headers = res.headers_mut();

    // Allow a handler to set its own CSP (escape hatch); otherwise apply ours.
    headers
        .entry(header::CONTENT_SECURITY_POLICY)
        .or_insert_with(|| HeaderValue::from_static(CONTENT_SECURITY_POLICY));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    res
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
