//! Serving the station from under a prefix, e.g. `https://home.example/birdnet`.
//!
//! # Why this is not a one-line `nest`
//!
//! Mounting the router under a prefix fixes *incoming* requests and nothing
//! else. Every URL the station **emits** is an absolute path from `/`: 234
//! literal `href`/`src`/`hx-get` attributes across 47 Rust files, 88 more in
//! the HTML templates, every `Location:` on a redirect, the session cookie's
//! `Path`, and three WebSocket URLs assembled in JavaScript from
//! `location.host` plus a literal path. Under a prefix each of those points
//! outside the application. The result is not a clean failure — the page
//! renders, and then some links 404 while others work, which reads like a
//! caching bug.
//!
//! # The shape of the fix
//!
//! One prefix, applied in four places, three of them central:
//!
//! 1. **Incoming** — `Router::nest(base, app)` in [`crate::server`].
//! 2. **Outgoing HTML** — rewritten in the pass that
//!    [`crate::security::security_headers_middleware`] already makes over
//!    every `text/html` body to stamp CSP nonces. That pass buffers the body
//!    and walks it regardless, so the rewrite is free, and — the reason it is
//!    done there rather than at 322 call sites — it covers markup written
//!    *after* this change as well as before it. A `url_for()` helper at every
//!    site would depend on every future line of HTML remembering to call it.
//! 3. **`Location` headers and the session cookie** — a redirect and a cookie
//!    `Path` are not HTML and are handled explicitly.
//! 4. **JavaScript** — the base path is published on `<body data-base-path>`
//!    and the scripts read it, because a URL assembled at runtime in the
//!    browser cannot be rewritten on the way out.
//!
//! The default is empty, and empty means *identical behaviour to before this
//! module existed*: [`rewrite_html`] returns its input unchanged without
//! scanning it, and the router is not nested.

use std::sync::OnceLock;

/// Why a configured base path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasePathError {
    /// A segment was empty (`//`) or the value was only slashes.
    EmptySegment,
    /// A segment was `.` or `..`, which would let the prefix escape itself.
    DotSegment,
    /// A character that has no business in a path prefix: whitespace, a query
    /// or fragment marker, or a control character.
    BadCharacter(char),
}

impl std::fmt::Display for BasePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySegment => f.write_str("empty path segment"),
            Self::DotSegment => f.write_str("`.` and `..` are not allowed in a base path"),
            Self::BadCharacter(c) => write!(f, "character {c:?} is not allowed in a base path"),
        }
    }
}

impl std::error::Error for BasePathError {}

/// A validated, normalised URL prefix.
///
/// Either empty (the default: the station is served from the root) or
/// `/segment[/segment…]` with a leading slash and no trailing one. That
/// normal form is what makes [`Self::join`] a plain concatenation, so there is
/// no place for a doubled or missing slash to be introduced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BasePath(String);

impl BasePath {
    /// Read a configured prefix.
    ///
    /// `""`, `"/"` and `"   "` all mean "no prefix". Otherwise the value is
    /// normalised: a missing leading slash is added, a trailing one removed.
    ///
    /// # Errors
    ///
    /// [`BasePathError`] for a value that cannot be normalised into the form
    /// above. Refusing is the right answer rather than sanitising: a prefix
    /// the operator did not get right will not match what their proxy sends,
    /// and a silent correction turns that into a 404 hunt.
    pub fn parse(raw: &str) -> Result<Self, BasePathError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "/" {
            return Ok(Self(String::new()));
        }
        let inner = trimmed.trim_matches('/');
        if inner.is_empty() {
            return Err(BasePathError::EmptySegment);
        }
        let mut out = String::with_capacity(inner.len() + 1);
        for segment in inner.split('/') {
            if segment.is_empty() {
                return Err(BasePathError::EmptySegment);
            }
            if segment == "." || segment == ".." {
                return Err(BasePathError::DotSegment);
            }
            if let Some(c) = segment.chars().find(|c| !is_allowed(*c)) {
                return Err(BasePathError::BadCharacter(c));
            }
            out.push('/');
            out.push_str(segment);
        }
        Ok(Self(out))
    }

    /// The prefix, `""` or `/like/this`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the station is served from the root.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Prefix one application-absolute path.
    ///
    /// `join("/today")` is `/birdnet/today`; `join("/")` is `/birdnet`, not
    /// `/birdnet/` — the two are different URLs to a proxy and to a browser's
    /// relative-link resolution, and the un-slashed form is the one the router
    /// mounts.
    ///
    /// A path that is not application-absolute (`https://…`, `//cdn…`,
    /// `#anchor`, `mailto:…`) is returned untouched: it does not point at this
    /// application and prefixing it would break it.
    #[must_use]
    pub fn join(&self, path: &str) -> String {
        if self.0.is_empty() || !is_app_absolute(path) {
            return path.to_owned();
        }
        if path == "/" {
            return self.0.clone();
        }
        format!("{}{path}", self.0)
    }

    /// The value for a cookie's `Path` attribute.
    ///
    /// Trailing slash, unlike [`Self::join`]: `Path=/birdnet` also matches
    /// `/birdnetfoo` under [RFC 6265 §5.1.4]'s prefix rule, so a second app
    /// on the same host with an adjacent name would receive this station's
    /// session cookie.
    ///
    /// [RFC 6265 §5.1.4]: https://www.rfc-editor.org/rfc/rfc6265#section-5.1.4
    #[must_use]
    pub fn cookie_path(&self) -> String {
        format!("{}/", self.0)
    }
}

/// Characters permitted in a base-path segment.
///
/// Deliberately narrow — unreserved characters from RFC 3986 plus `~`. A
/// prefix is typed once into a config file and typed again into a proxy
/// config; anything needing percent-encoding to survive that round trip is a
/// prefix nobody should be using.
const fn is_allowed(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')
}

/// Whether a URL is an absolute path into *this* application.
///
/// `/foo` yes. `//cdn.example/x` no — that is protocol-relative and points at
/// another host, and it is the case a naive `starts_with('/')` gets wrong.
fn is_app_absolute(url: &str) -> bool {
    url.starts_with('/') && !url.starts_with("//")
}

/// The process's base path, set once at start-up.
static BASE_PATH: OnceLock<BasePath> = OnceLock::new();

/// Install the base path for this process.
///
/// Returns the value in force afterwards, which is the argument unless
/// something already set it. Called once from the binary before the router is
/// built; a second call is a no-op rather than an error, so a test binary that
/// sets it and then constructs several routers behaves predictably.
pub fn init(base: BasePath) -> &'static BasePath {
    let _ = BASE_PATH.set(base);
    current()
}

/// The base path in force. Empty until [`init`] is called.
#[must_use]
pub fn current() -> &'static BasePath {
    static EMPTY: BasePath = BasePath(String::new());
    BASE_PATH.get().unwrap_or(&EMPTY)
}

/// Read the base path from `BIRDNET_BASE_PATH`.
///
/// A value that does not parse is refused loudly and treated as unset. The
/// alternative — guessing at what was meant — mounts the station somewhere the
/// operator's proxy is not looking, and every request 404s with nothing in the
/// log to say why.
#[must_use]
pub fn from_env() -> BasePath {
    let Ok(raw) = std::env::var("BIRDNET_BASE_PATH") else {
        return BasePath::default();
    };
    match BasePath::parse(&raw) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                error = %e,
                value = %raw,
                "BIRDNET_BASE_PATH did not parse; serving from the root instead"
            );
            BasePath::default()
        }
    }
}

// ---------------------------------------------------------------------------
// HTML rewriting
// ---------------------------------------------------------------------------

/// Attribute names whose value is a URL this application serves.
///
/// Only attributes, and only these: matching on the *value* alone would rewrite
/// paths inside prose, JSON embedded in a `<script type="application/json">`,
/// and any documentation on the page that shows a URL. Every entry here is
/// followed by `="` in the scan, so a value must be quoted to be rewritten,
/// which is true of all markup this application emits.
const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "action",
    "formaction",
    "poster",
    "data-src",
    "data-url",
    "data-endpoint",
    "hx-get",
    "hx-post",
    "hx-put",
    "hx-patch",
    "hx-delete",
    "ws-connect",
    "sse-connect",
];

/// Prefix every application-absolute URL in an HTML document.
///
/// A no-op — returning the input without scanning it — when the base path is
/// empty, which is every station that has not configured one.
///
/// The scan is a single left-to-right pass looking for `<attr>="/`. It does not
/// parse HTML, and does not need to: the pattern it matches (an attribute name
/// from [`URL_ATTRIBUTES`], `="`, then `/` not followed by `/`) does not occur
/// in prose or in the JSON this application embeds, and the one place it could
/// — a page documenting HTML markup — this application does not have.
#[must_use]
pub fn rewrite_html(html: &str, base: &BasePath) -> String {
    if base.is_empty() {
        return html.to_owned();
    }
    let prefix = base.as_str();
    let mut out = String::with_capacity(html.len() + html.len() / 16);
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // An attribute value can only start after `="`, so anchor on the quote
        // rather than on each attribute name: one scan instead of fifteen.
        let Some(rel) = html[i..].find("=\"/") else {
            out.push_str(&html[i..]);
            break;
        };
        let eq = i + rel;
        let slash = eq + 2;
        // Protocol-relative `//host/path` points at another origin.
        let protocol_relative = bytes.get(slash + 1) == Some(&b'/');
        out.push_str(&html[i..=eq + 1]);
        if !protocol_relative && attribute_ends_at(html, eq) {
            out.push_str(prefix);
        }
        i = eq + 2;
    }
    out
}

/// Whether the attribute name ending immediately before `eq` is one whose value
/// is a URL.
///
/// Walks backwards from the `=` over the name characters and compares the span
/// against [`URL_ATTRIBUTES`]. Backwards because the alternative — searching
/// forwards for each of fifteen names — rescans the document fifteen times and
/// still has to prove the match is a whole attribute rather than a suffix
/// (`data-href` would match `href`).
fn attribute_ends_at(html: &str, eq: usize) -> bool {
    let bytes = html.as_bytes();
    let mut start = eq;
    while start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == eq {
        return false;
    }
    // The character before the name must be whitespace or the tag's `<`,
    // otherwise this is the tail of something longer.
    if start > 0 {
        let before = bytes[start - 1];
        if !before.is_ascii_whitespace() && before != b'<' {
            return false;
        }
    }
    let name = &html[start..eq];
    URL_ATTRIBUTES.iter().any(|a| name.eq_ignore_ascii_case(a))
}

/// Prefix a `Location` header value, for a redirect this application issues.
///
/// Same rule as [`BasePath::join`]: an absolute path into this application is
/// prefixed, an absolute URL or a protocol-relative one is left alone.
#[must_use]
pub fn rewrite_location(location: &str, base: &BasePath) -> String {
    base.join(location)
}

#[cfg(test)]
mod tests;
