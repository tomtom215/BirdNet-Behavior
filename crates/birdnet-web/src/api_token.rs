//! Bearer-token authentication for the mutating `/api/v2` surface (`O-1`).
//!
//! # Why this exists
//!
//! The `/api/v2` surface is entirely read-only: no `post`, `put`, `delete` or
//! `patch` route method appears in any of the modules mounted under it, against
//! upstream `birdnet-go`'s fifty-four mutating routes. Every state change in
//! this product is an HTMX form post returning an HTML fragment behind a
//! same-origin check — which any script can satisfy by setting a matching
//! `Origin` header, and which therefore is not a contract anyone can build on.
//! Home Assistant and Node-RED can read a station and never act on it, and our
//! own front end is the only client, so a change to fragment markup silently
//! breaks whatever automation exists in the wild.
//!
//! # Why a token and not the cookie session
//!
//! [`crate::auth_middleware`] lets **everything** through when no admin
//! password is configured, which is the default (`O-4`). Mutating endpoints
//! must not inherit that: a station that has not been given a token has no
//! mutating API at all. That is the difference between publishing a stable,
//! documented way to change a station and publishing an open one.
//!
//! # Why the token is not in the `settings` table
//!
//! Because this project already decided that, and enforces it: the admin
//! password is resolved from the configuration file and then the environment,
//! and `helpers::auth::purge_legacy_credential_settings` *deletes* plaintext
//! credential rows a previous build's settings form could write. A token in
//! `settings` would also be readable through the dashboard, which on a default
//! station is unauthenticated. `BNB_API_TOKEN` follows `CADDY_PWD`: config
//! first, then environment.
//!
//! # Why only the digest is kept
//!
//! `AppStateInner` derives `Debug`. A plaintext token held there would be one
//! `{state:?}` away from a log line, so only a SHA-256 digest of it is stored
//! and the `Debug` impl below prints neither.

use axum::response::IntoResponse as _;
use sha2::{Digest as _, Sha256};

/// The environment variable and configuration key the token is read from.
pub const API_TOKEN_KEY: &str = "BNB_API_TOKEN";

/// Shortest token this station will accept, in bytes.
///
/// A bearer token is a password that never expires and is presented on every
/// request, so it has to be generated rather than chosen. Thirty-two bytes is
/// the same floor `.env.example` already states for `BNB_SESSION_SECRET` and
/// `BNB_SHARE_SECRET`, and `openssl rand -base64 48` clears it comfortably.
///
/// Below the floor the station refuses to enable the mutating API rather than
/// enabling it weakly — the failure an operator can see beats the one they
/// cannot.
pub const MIN_TOKEN_LEN: usize = 32;

/// Why a configured token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiTokenError {
    /// The value was shorter than [`MIN_TOKEN_LEN`].
    TooShort {
        /// How long it actually was.
        len: usize,
    },
}

impl std::fmt::Display for ApiTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort { len } => write!(
                f,
                "{API_TOKEN_KEY} is {len} bytes; at least {MIN_TOKEN_LEN} are required"
            ),
        }
    }
}

impl std::error::Error for ApiTokenError {}

/// A station's API token, held as a digest.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiToken {
    /// SHA-256 of the configured token.
    digest: [u8; 32],
}

impl std::fmt::Debug for ApiToken {
    /// Prints no bytes at all.
    ///
    /// The digest is not itself a credential, but `AppState` is `Debug` and
    /// gets logged; a reader who sees 32 hex bytes next to the word "token"
    /// will reasonably assume they have found one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

impl ApiToken {
    /// Accept `token` as this station's API token.
    ///
    /// # Errors
    ///
    /// [`ApiTokenError::TooShort`] when the value is under [`MIN_TOKEN_LEN`]
    /// bytes.
    pub fn new(token: &str) -> Result<Self, ApiTokenError> {
        if token.len() < MIN_TOKEN_LEN {
            return Err(ApiTokenError::TooShort { len: token.len() });
        }
        Ok(Self {
            digest: Self::digest(token),
        })
    }

    /// Whether `presented` is this station's token.
    ///
    /// Both sides are hashed before they are compared, so the comparison runs
    /// over digests rather than over the secret. A byte-wise `==` on the
    /// digests can still return early, but what that leaks is the position of
    /// the first differing *digest* byte, and turning that into the token
    /// requires inverting SHA-256. This is the standard hash-then-compare
    /// defence; it is deliberately **not** described as a constant-time
    /// comparison, because it is not one.
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        Self::digest(presented) == self.digest
    }

    fn digest(s: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(s.as_bytes());
        hasher.finalize().into()
    }
}

/// Pull the bearer credential out of an `Authorization` header value.
///
/// The scheme is matched case-insensitively because RFC 7235 says it is
/// case-insensitive, and clients differ: `reqwest`'s `bearer_auth` writes
/// `Bearer`, some shell scripts write `bearer`.
///
/// Returns `None` for a header that is not `Bearer`, so a `Basic` credential
/// is a missing token rather than a wrong one — the distinction the caller
/// reports back.
#[must_use]
pub fn bearer_credential(header: &str) -> Option<&str> {
    let (scheme, rest) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = rest.trim_start();
    (!token.is_empty()).then_some(token)
}

/// Wrap `router` so every request must present this station's API token.
///
/// # The two refusals, and why they differ
///
/// * **No token configured** — the default — is `404`. A station that has not
///   been given a `BNB_API_TOKEN` does not have this surface, and saying so as
///   "not found" tells an unauthenticated scanner nothing and advertises no
///   capability the station will not honour. The routes stay *mounted* so the
///   path set the OpenAPI document describes does not change with
///   configuration.
/// * **Configured, but the credential is missing or wrong** is `401` with a
///   `WWW-Authenticate: Bearer` challenge, which is what an HTTP client expects
///   and what `crates/birdnet-web/tests/public_router_is_read_only.rs` already
///   asserts for an unauthenticated write.
///
/// Crucially this does **not** consult
/// `auth_middleware::admin_password_configured`. That path lets everything
/// through on a station with no admin password, which is the default (`O-4`);
/// inheriting it here would publish a documented, open way to change a station.
pub fn require_bearer(
    router: axum::Router<crate::state::AppState>,
    state: crate::state::AppState,
) -> axum::Router<crate::state::AppState> {
    use axum::http::{StatusCode, header};

    router.layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let state = state.clone();
            async move {
                let Some(token) = state.api_token() else {
                    return (
                        StatusCode::NOT_FOUND,
                        [(header::CONTENT_TYPE, "application/json")],
                        r#"{"error":"the mutating API is not enabled on this station; set BNB_API_TOKEN"}"#,
                    )
                        .into_response();
                };
                let presented = req
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(bearer_credential);
                match presented {
                    Some(p) if token.matches(p) => next.run(req).await,
                    _ => {
                        // Never logs the credential, correct or otherwise.
                        tracing::info!(
                            path = %req.uri().path(),
                            "mutating API request refused: no valid bearer token"
                        );
                        (
                            StatusCode::UNAUTHORIZED,
                            [
                                (header::WWW_AUTHENTICATE, "Bearer"),
                                (header::CONTENT_TYPE, "application/json"),
                            ],
                            r#"{"error":"a valid Authorization: Bearer <token> header is required"}"#,
                        )
                            .into_response()
                    }
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::{ApiToken, ApiTokenError, MIN_TOKEN_LEN, bearer_credential};

    /// A token long enough to be accepted.
    fn good() -> String {
        "a".repeat(MIN_TOKEN_LEN)
    }

    #[test]
    fn a_token_matches_itself_and_nothing_else() {
        let t = ApiToken::new(&good()).expect("long enough");
        assert!(t.matches(&good()));
        assert!(!t.matches(&"b".repeat(MIN_TOKEN_LEN)));
        assert!(!t.matches(""));
        // A prefix of the real token must not pass. Without hashing, a
        // comparison that stopped at the shorter length would let it.
        assert!(!t.matches(&"a".repeat(MIN_TOKEN_LEN - 1)));
        // Nor a value that merely starts with it.
        assert!(!t.matches(&format!("{}x", good())));
    }

    #[test]
    fn a_short_token_is_refused_rather_than_accepted_weakly() {
        let short = "a".repeat(MIN_TOKEN_LEN - 1);
        assert_eq!(
            ApiToken::new(&short),
            Err(ApiTokenError::TooShort {
                len: MIN_TOKEN_LEN - 1
            })
        );
        // The counterpart: exactly at the floor is accepted, so the rule is a
        // boundary and not a blanket refusal.
        assert!(ApiToken::new(&good()).is_ok());
    }

    #[test]
    fn the_debug_impl_prints_nothing_useful() {
        let t = ApiToken::new(&good()).expect("long enough");
        let rendered = format!("{t:?}");
        assert_eq!(rendered, "ApiToken(<redacted>)");
        assert!(!rendered.contains(&good()));
    }

    #[test]
    fn a_bearer_credential_is_read_case_insensitively() {
        assert_eq!(bearer_credential("Bearer abc"), Some("abc"));
        assert_eq!(bearer_credential("bearer abc"), Some("abc"));
        assert_eq!(bearer_credential("BEARER abc"), Some("abc"));
        assert_eq!(bearer_credential("Bearer   abc"), Some("abc"));
    }

    #[test]
    fn anything_that_is_not_a_bearer_credential_is_none() {
        // The counterpart to the gate above: "read it case-insensitively"
        // must not become "read anything".
        assert_eq!(bearer_credential("Basic abc"), None);
        assert_eq!(bearer_credential("Bearer"), None);
        assert_eq!(bearer_credential("Bearer "), None);
        assert_eq!(bearer_credential(""), None);
        assert_eq!(bearer_credential("abc"), None);
    }
}
