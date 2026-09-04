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
/// fragments — without re-opening an inline-script free-for-all.
///
/// `style-src 'self' 'nonce-…'` is the same idea for styles, with one twist:
/// there is no `'strict-dynamic'` for styles, and a per-request nonce on a
/// `<style>` in an HTMX-swapped fragment would never match the *host page's*
/// nonce. So inline `style="…"` **attributes** are eliminated entirely —
/// computed values move to a `data-style` attribute applied via a CSSOM
/// `el.style` writer (which CSP does not police) that re-runs on
/// `htmx:afterSwap`; static ones become `app.css` classes (`'self'`). The
/// nonce then only has to admit our own full-document `<style>` blocks. With
/// that, `'unsafe-inline'` is gone for styles too. `connect-src 'self'` covers
/// the same-origin live WebSocket (`ws://<host>/…`).
fn content_security_policy(nonce: &str) -> String {
    format!(
        "default-src 'self'; \
         img-src 'self' data: https:; \
         style-src 'self' 'nonce-{nonce}'; \
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

/// Stamp `nonce="…"` onto every parser-inserted `<style>` opening tag, the
/// `style-src` counterpart of [`inject_script_nonce`]. Style **attributes**
/// (`style="…"`) cannot carry a nonce, so they are eliminated in the renderers;
/// only `<style>` **elements** are nonceable, and this admits exactly the ones
/// our own page renderers emit. Unlike scripts there is no `'strict-dynamic'`
/// for styles, so this only covers full-document renders — an HTMX-swapped
/// fragment is inserted under the *host page's* nonce and cannot match its own.
fn inject_style_nonce(html: &str, nonce: &str) -> String {
    html.replace("<style ", &format!("<style nonce=\"{nonce}\" "))
        .replace("<style>", &format!("<style nonce=\"{nonce}\">"))
}

/// The CSSOM applier that lets computed styles ride a `data-style` attribute
/// instead of a (now CSP-forbidden) inline `style=""`. It writes each
/// declaration onto `element.style` via `setProperty`, which CSP does not
/// police, and re-runs on `htmx:afterSwap` so swapped-in fragments are styled
/// before paint.
///
/// Injected here, once per full document, rather than living in a template:
/// the app has ~20 distinct full-page shells (the main `layout.html`, the
/// admin shells, and standalone pages like `/admin/system`, `/player`,
/// `kiosk`, `onboarding`), and threading the applier through each one would
/// let a new page silently ship `data-style` markup that never gets applied —
/// the same failure mode the per-request nonce design avoids for scripts.
/// HTMX fragment responses (no `</body>`) are left alone; their host page's
/// applier styles them on swap.
const DYN_STYLE_APPLIER: &str = r"<script>
(function () {
  function apply(el) {
    var d = el.getAttribute('data-style');
    if (!d) return;
    var decls = d.split(';');
    for (var i = 0; i < decls.length; i++) {
      var c = decls[i].indexOf(':');
      if (c < 0) continue;
      var p = decls[i].slice(0, c).trim();
      if (p) el.style.setProperty(p, decls[i].slice(c + 1).trim());
    }
  }
  function walk(root) {
    if (root.nodeType === 1 && root.hasAttribute('data-style')) apply(root);
    var els = root.querySelectorAll ? root.querySelectorAll('[data-style]') : [];
    for (var i = 0; i < els.length; i++) apply(els[i]);
  }
  walk(document);
  if (document.body) document.body.addEventListener('htmx:afterSwap', function (e) { walk(e.target); });
})();
</script>";

/// Insert [`DYN_STYLE_APPLIER`] just before `</body>` of a full document. HTMX
/// fragments carry no `</body>`, so they pass through untouched.
fn inject_dyn_style_applier(html: &str) -> String {
    html.rfind("</body>").map_or_else(
        || html.to_string(),
        |pos| format!("{}{}{}", &html[..pos], DYN_STYLE_APPLIER, &html[pos..]),
    )
}

/// On init htmx appends an inline `.htmx-indicator` `<style>` to `<head>`; with
/// a nonce `style-src` that un-nonced element is refused. Disable it via the
/// `htmx-config` meta (htmx reads config before injecting the style) — our own,
/// fuller `.htmx-indicator` rules already live in `app.css`. Inserted right
/// after `<head>`; HTMX fragments have no `<head>`, so they pass through.
const HTMX_CONFIG_META: &str =
    r#"<meta name="htmx-config" content='{"includeIndicatorStyles":false}'>"#;

fn inject_htmx_config(html: &str) -> String {
    html.replacen("<head>", &format!("<head>{HTMX_CONFIG_META}"), 1)
}

/// Publish the base path to the browser as `<body data-base-path="…">`.
///
/// The rewriting pass fixes URLs that are *written* in the markup. It cannot
/// fix one a script assembles at run time, and this application has three:
/// `live-detections.js` and the two inline spectrogram sockets build
/// `location.host` plus a literal path. They read this attribute instead.
///
/// Stamped here rather than added to each `<body>` tag because there is no one
/// layout: `templates/layout.html`, the login and share shells, the admin
/// shell, the kiosk, the audio player and the log viewer are seven independent
/// full-page documents, and a new one would silently miss it.
///
/// Nothing is added when no base path is set, so a station serving from the
/// root ships exactly the bytes it shipped before.
fn inject_base_path(html: &str, base: &crate::base_path::BasePath) -> String {
    if base.is_empty() {
        return html.to_owned();
    }
    // `<body` rather than `<body>`: the tag may already carry attributes.
    html.find("<body").map_or_else(
        || html.to_owned(),
        |pos| {
            let cut = pos + "<body".len();
            format!(
                r#"{} data-base-path="{}"{}"#,
                &html[..cut],
                base.as_str(),
                &html[cut..]
            )
        },
    )
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
    let base = crate::base_path::current();

    let res = next.run(req).await;
    let (mut parts, body) = res.into_parts();

    // A redirect's target is not HTML and never reaches the body pass. Missed,
    // it sends the browser out of the prefix on every login and every form
    // that redirects — the single most visible way base-path support fails.
    if !base.is_empty()
        && let Some(loc) = parts.headers.get(header::LOCATION)
        && let Ok(text) = loc.to_str()
        && let Ok(v) = HeaderValue::from_str(&crate::base_path::rewrite_location(text, base))
    {
        parts.headers.insert(header::LOCATION, v);
    }

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
                let html = String::from_utf8_lossy(&bytes);
                // Suppress htmx's un-nonceable indicator <style> (its rules are
                // already in app.css), then add the data-style applier; its
                // <script> is nonced by the pass below like any other.
                let configured = inject_htmx_config(html.as_ref());
                let with_applier = inject_dyn_style_applier(&configured);
                let scripted = inject_script_nonce(&with_applier, &nonce);
                let stamped = inject_style_nonce(&scripted, &nonce);
                // The base-path prefix rides this pass rather than making its
                // own: the body is already buffered and already walked, so the
                // rewrite is free, and every page written after this change
                // gets it without remembering to ask. A no-op — not even a
                // scan — when no base path is configured.
                let stamped = crate::base_path::rewrite_html(&stamped, base);
                let stamped = inject_base_path(&stamped, base);
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
    if is_state_changing(req.method())
        && !is_bearer_api_request(req.uri().path(), req.headers())
        && !is_same_origin(req.headers())
    {
        return (
            StatusCode::FORBIDDEN,
            "Cross-origin request blocked by CSRF protection. \
             Submit this request from the BirdNet-Behavior web UI.",
        )
            .into_response();
    }
    next.run(req).await
}

/// Whether this is a bearer-authenticated call to the mutating API (`O-1`).
///
/// The CSRF check exists because a cross-site *form* can be made to submit to
/// this station with the victim's cookies attached. A form cannot set an
/// `Authorization` header — that is the entire premise — and a cross-origin
/// `fetch` that tries to set one triggers a preflight the CORS layer refuses
/// unless the operator has allow-listed the origin. So a request carrying a
/// bearer credential has nothing for this guard to protect, and applying the
/// same-origin rule to it would make the API unusable from anything that is
/// not a browser on the station's own hostname — which is every automation it
/// exists for.
///
/// Deliberately scoped to the paths in
/// [`crate::routes::api_write::WRITE_ROUTES`] rather than to "any request with
/// an `Authorization` header". The cookie-authenticated `/admin` surface must
/// keep its CSRF protection whatever headers a request carries, and a skip
/// keyed only on the header would hand it away.
///
/// Presence of the header is enough here: whether the credential is *correct*
/// is [`crate::api_token::require_bearer`]'s question, and it is asked after
/// this one. A forged header buys a caller nothing but a 401.
fn is_bearer_api_request(path: &str, headers: &HeaderMap) -> bool {
    crate::routes::api_write::is_write_route(path)
        && header_str(headers, &header::AUTHORIZATION)
            .and_then(crate::api_token::bearer_credential)
            .is_some()
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
        // Styles are nonced now too, and 'unsafe-inline' is gone everywhere:
        // inline style="" attributes are eliminated in the renderers (moved to
        // data-style + a CSSOM applier), and the only <style> elements left are
        // our own full-page blocks, admitted by this nonce.
        assert!(csp.contains("style-src 'self' 'nonce-XYZ'"));
        assert!(!csp.contains("'unsafe-inline'"));
    }

    #[test]
    fn inject_style_nonce_stamps_each_opening_tag_once() {
        let html = concat!(
            r#"<style>.a{color:red}</style>"#,
            r#"<style type="text/css">.b{color:blue}</style>"#,
        );
        let out = inject_style_nonce(html, "ABC");
        assert_eq!(out.matches(r#"nonce="ABC""#).count(), 2);
        assert!(out.contains(r#"<style nonce="ABC">.a"#));
        assert!(out.contains(r#"<style nonce="ABC" type="text/css">.b"#));
        // Close tags are left alone.
        assert_eq!(out.matches("</style>").count(), 2);
    }

    #[test]
    fn dyn_style_applier_injected_only_into_full_documents() {
        // Full document: the applier is inserted once, before </body>.
        let page = r#"<html><body><div data-style="width:42%"></div></body></html>"#;
        let out = inject_dyn_style_applier(page);
        assert_eq!(out.matches("htmx:afterSwap").count(), 1);
        assert!(out.find("htmx:afterSwap").unwrap() < out.rfind("</body>").unwrap());
        // HTMX fragment (no </body>): left untouched — its host page applies it.
        let frag = r#"<div data-style="width:42%"></div>"#;
        assert_eq!(inject_dyn_style_applier(frag), frag);
    }

    #[test]
    fn htmx_config_meta_disables_indicator_styles_in_head() {
        let page = "<html><head><title>x</title></head><body></body></html>";
        let out = inject_htmx_config(page);
        assert!(out.contains(r#"<meta name="htmx-config""#));
        assert!(out.contains(r#""includeIndicatorStyles":false"#));
        // Inserted inside <head>, before the existing head content.
        assert!(out.find("htmx-config").unwrap() < out.find("<title>").unwrap());
        // Fragment with no <head>: untouched.
        let frag = r#"<div class="x"></div>"#;
        assert_eq!(inject_htmx_config(frag), frag);
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
