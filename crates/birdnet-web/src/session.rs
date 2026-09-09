//! Stateless cookie-session helpers for O-14.
//!
//! The cookie value is a versioned string carrying its own expiry and an
//! HMAC over that expiry — no database lookup is required to validate it.
//! O-15 layers a `sessions` table on top to bind cookies to specific users,
//! devices, and audit-log rows; this module is the underlying primitive
//! both paths share.
//!
//! ## Wire format
//!
//! ```text
//! v1.{expires-ms}.{base64url(hmac-sha256(secret, expires-ms)[..16])}
//! ```
//!
//! * `v1` lets us evolve the encoding without invalidating in-flight cookies
//!   on day zero of a future format.
//! * `expires-ms` is a unix epoch in milliseconds; verifying that
//!   `expires-ms > now` is what makes the cookie a session.
//! * The MAC is truncated to 128 bits — the same margin the share-link
//!   tokens in `routes::share` use against forgery.
//!
//! ## Secret derivation
//!
//! Operators can set `BNB_SESSION_SECRET` to lock the secret across restarts
//! and process moves. Otherwise the station uses a secret it generated once
//! and keeps beside its database (`session.secret`, mode 0600 — see
//! [`install_persisted_secret`]), so sessions survive a restart on every
//! install, not only the ones that happen to export a variable. A process
//! that did not install one falls back to a derivation from the configured
//! admin password, `HMAC-SHA256(CADDY_PWD, b"bnb-session-v1")`, and finally
//! to a fail-secure per-process random secret so outstanding cookies
//! invalidate on restart. Signing other sessions out on a password rotation
//! is the sessions table's job (`SessionStore::revoke_others`), not the
//! secret's.
//!
//! The DIFF for O-14 originally specified `BLAKE3(hashed_password || …)`;
//! `blake3` is not in the dep tree and the project rule forbids adding
//! new crates here, so we substitute `HMAC-SHA256` over the same inputs.
//! Security goal is identical (pseudo-random function of the password
//! material) and `hmac` + `sha2` are already in `Cargo.toml`.
//!
//! ## Scope
//!
//! This module is the stateless cookie *primitive* — it mints and validates
//! the token and stores nothing itself. Gating `/admin/*` is
//! [`crate::auth_middleware`]'s job: it validates the token minted here,
//! binds it to a `sessions` row (O-15), touches `last_seen`, and enforces
//! the viewer/admin role split on writes.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Cookie name carrying the session token.
pub const COOKIE_NAME: &str = "bnb-session";

/// Default session lifetime (14 days, in milliseconds).
pub const DEFAULT_TTL_MS: u64 = 14 * 24 * 60 * 60 * 1000;

/// "Remember me" session lifetime (90 days, in milliseconds).
pub const REMEMBER_ME_TTL_MS: u64 = 90 * 24 * 60 * 60 * 1000;

/// Bytes of the truncated MAC carried in the cookie value. 128 bits is
/// the same margin used for share-link tokens (`routes::share`).
const TRUNCATED_MAC_LEN: usize = 16;

/// Cookie format version. Bump when the wire format changes incompatibly.
///
/// * `v1` (legacy, shipped in #89): `v1.{expires_ms}.{mac}`. Stateless
///   carrier — couldn't be revoked, didn't bind a session id.
/// * `v2` (current, used after the auth wire flip): `v2.{session_id}.{expires_ms}.{mac}`
///   where the MAC covers `{session_id}.{expires_ms}`. The middleware
///   looks up `session_id` in the `sessions` table on every request, so
///   revoking a row in the UI also invalidates the cookie. v1 cookies
///   are rejected — operators re-sign-in on the first request after
///   upgrade.
const FORMAT_VERSION: &str = "v2";

/// Length of a session id, in bytes of base32 output. 26 chars carries
/// 128 bits of randomness, matching the O-15 DIFF.
pub const SESSION_ID_LEN: usize = 26;

/// Secret used to derive a fail-secure per-process secret when neither
/// `BNB_SESSION_SECRET` nor `CADDY_PWD` is set. Treats sessions as
/// per-process (they invalidate on restart).
fn process_random_secret() -> &'static [u8] {
    static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
    SECRET.get_or_init(|| {
        // Same best-effort scramble routes::share uses for its per-process
        // fallback. Quality is bounded by std-only entropy, so callers
        // should set BNB_SESSION_SECRET in production.
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0xDEAD_BEEF_u64, |d| {
                u64::from(d.subsec_nanos()) ^ d.as_secs().rotate_left(21)
            })
            ^ u64::from(std::process::id());
        let mut x = seed;
        let mut buf = [0u8; 32];
        for b in &mut buf {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_possible_truncation)]
            {
                *b = (x >> 56) as u8;
            }
        }
        buf
    })
}

/// The secret [`install_persisted_secret`] loaded, if it ran.
static PERSISTED_SECRET: OnceLock<Vec<u8>> = OnceLock::new();

/// The file beside the database that holds the station's own signing secret:
/// 64 lowercase hex characters, mode 0600.
pub const SECRET_FILE_NAME: &str = "session.secret";

/// Load the station's signing secret from `dir`, creating it on first use.
///
/// Every bare-metal install ran on a per-process random secret (DD-15): the
/// installer writes `CADDY_PWD` to the config file, nothing exports it to the
/// environment, and `BNB_SESSION_SECRET` is set by nobody — so every login
/// session died on restart while the accounts page promised fourteen days.
/// A secret the station generates once and keeps beside its database is what
/// makes that promise true without asking the operator for anything.
///
/// Written atomically (temp file, then rename) with mode 0600, so a crash
/// mid-write cannot leave a half-secret that a later start reads as the
/// whole one. Unreadable or malformed content is an error rather than a
/// regenerated secret: regenerating silently would sign everyone out and
/// hide the reason.
///
/// # Errors
///
/// The file cannot be read or created, or holds something other than 64 hex
/// characters.
pub fn load_or_create_secret(dir: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Write as _};
    let path = dir.join(SECRET_FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let hex = text.trim();
            return decode_hex_secret(hex).ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("{} is not a 64-character hex secret", path.display()),
                )
            });
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut bytes = [0_u8; 32];
    {
        use password_hash::rand_core::{OsRng, RngCore};
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|e| Error::other(format!("CSPRNG unavailable for the session secret: {e}")))?;
    }
    let hex = bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        use std::fmt::Write as _;
        // Infallible: writing to a String never errors.
        let _ = write!(out, "{b:02x}");
        out
    });
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{SECRET_FILE_NAME}.{}.tmp", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(hex.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(bytes.to_vec())
}

fn decode_hex_secret(hex: &str) -> Option<Vec<u8>> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Load (or create) the persisted secret in `dir` and make it the signing
/// secret for this process, ahead of the `CADDY_PWD` derivation.
///
/// Called once at start-up by the binary, after the database path is known.
/// Idempotent: a second call keeps the first secret.
///
/// # Errors
///
/// As [`load_or_create_secret`]; the caller logs and continues on the
/// previous derivation, so a read-only data directory degrades to the old
/// behaviour rather than refusing to start.
pub fn install_persisted_secret(dir: &std::path::Path) -> std::io::Result<()> {
    if PERSISTED_SECRET.get().is_some() {
        return Ok(());
    }
    let secret = load_or_create_secret(dir)?;
    let _ = PERSISTED_SECRET.set(secret);
    Ok(())
}

/// Resolve the signing secret in priority order:
/// 1. `BNB_SESSION_SECRET` (operator-supplied, survives restarts).
/// 2. The station's own secret, persisted beside the database by
///    [`install_persisted_secret`] (survives restarts; DD-15).
/// 3. `HMAC-SHA256(CADDY_PWD, "bnb-session-v1")` (deterministic from the
///    admin password, rotates with it) — reached only by a process that
///    did not install a persisted secret.
/// 4. A fail-secure per-process random secret.
///
/// Rotating the password no longer rotates the secret on the paths that
/// have a persisted one; the sessions table is where a rotation signs the
/// other sessions out (`SessionStore::revoke_others`), which is the same
/// outcome by the mechanism that can also tell the operator's own session
/// apart.
///
/// # Panics
///
/// Panics if the underlying HMAC implementation rejects a key of arbitrary
/// length, which the contract of `Hmac<Sha256>` guarantees it does not. The
/// `expect` is a documented invariant, not a runtime path.
pub fn secret() -> Vec<u8> {
    if let Ok(s) = std::env::var("BNB_SESSION_SECRET")
        && !s.is_empty()
    {
        return s.into_bytes();
    }
    if let Some(persisted) = PERSISTED_SECRET.get() {
        return persisted.clone();
    }
    if let Ok(pwd) = std::env::var("CADDY_PWD")
        && !pwd.is_empty()
    {
        let mut mac =
            HmacSha256::new_from_slice(pwd.as_bytes()).expect("HMAC accepts any key length");
        mac.update(b"bnb-session-v1");
        return mac.finalize().into_bytes().to_vec();
    }
    process_random_secret().to_vec()
}

/// Read `BNB_SESSION_TTL_DAYS` if set; clamped to `[1, 365]`. Falls back
/// to the 14-day default when unset or invalid.
#[must_use]
pub fn default_ttl_ms() -> u64 {
    std::env::var("BNB_SESSION_TTL_DAYS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|d| (1..=365).contains(d))
        .map_or(DEFAULT_TTL_MS, |days| u64::from(days) * 86_400_000)
}

/// Generate a fresh session id.
///
/// 26 lowercase base32 chars carrying 128 bits of randomness, per the
/// O-15 DIFF. Sources entropy from `password-hash::rand_core::OsRng`
/// (the same CSPRNG argon2 uses for salts), already in the dep tree via
/// the argon2 helpers shipped earlier on this branch.
#[must_use]
pub fn generate_session_id() -> String {
    use password_hash::rand_core::{OsRng, RngCore};
    // 16 bytes = 128 bits → exactly 26 base32 chars (no padding needed
    // since 128/5 = 25.6 → 26 chars covers it).
    let mut bytes = [0_u8; 16];
    if let Err(e) = OsRng.try_fill_bytes(&mut bytes) {
        // CSPRNG failure on Linux is essentially impossible (getrandom(2)
        // and /dev/urandom both available), so this branch is a defensive
        // belt — log loudly and fall back to a SplitMix64 over startup
        // entropy so the binary doesn't hard-crash on exotic kernels.
        tracing::warn!(
            error = %e,
            "OsRng failed for session id; using process-time fallback"
        );
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0xDEAD_BEEF_u64, |d| {
                u64::from(d.subsec_nanos()) ^ d.as_secs().rotate_left(13)
            })
            ^ u64::from(std::process::id());
        let mut x = seed;
        for b in &mut bytes {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_possible_truncation)]
            {
                *b = (x >> 56) as u8;
            }
        }
    }
    base32_lower(&bytes)
}

/// Lowercase RFC 4648 base32 (no padding), used only for session ids
/// (where the alphabet bias / typo resistance trade-off matches the
/// O-15 DIFF). Inputs are exactly 16 bytes so output is exactly 26 chars.
fn base32_lower(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for b in bytes {
        buf = (buf << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1F) as usize;
            out.push(ALPHA[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1F) as usize;
        out.push(ALPHA[idx] as char);
    }
    out
}

/// Issue a v2 session cookie binding `session_id` for `ttl_ms` from now.
/// Pair with a row in the `sessions` table holding the same id.
#[must_use]
pub fn issue_token(session_id: &str, ttl_ms: u64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let expires_ms = now_ms.saturating_add(ttl_ms);
    encode_token(session_id, expires_ms)
}

fn encode_token(session_id: &str, expires_ms: u64) -> String {
    let exp_str = expires_ms.to_string();
    let payload = format!("{session_id}.{exp_str}");
    let mut mac = HmacSha256::new_from_slice(&secret()).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    let tag = mac.finalize().into_bytes();
    let mac_b64 = URL_SAFE_NO_PAD.encode(&tag[..TRUNCATED_MAC_LEN]);
    format!("{FORMAT_VERSION}.{payload}.{mac_b64}")
}

/// Validated cookie payload. Returned by [`validate_token`] when the MAC
/// verifies and the expiry is in the future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedToken {
    /// Opaque session identifier extracted from the verified cookie payload.
    pub session_id: String,
    /// Absolute expiry as milliseconds since the Unix epoch.
    pub expires_ms: u64,
}

/// Validate a cookie value. Returns the `(session_id, expires_ms)` pair
/// when the version, MAC, and expiry all check out.
///
/// v1 cookies (the legacy format shipped in #89) are rejected — they
/// don't carry a session id, so the middleware cannot bind them to a
/// session row. Operators re-sign-in on first request after upgrade.
///
/// # Panics
///
/// Panics if the underlying HMAC implementation rejects a key of arbitrary
/// length — see [`secret`].
#[must_use]
pub fn validate_token(value: &str) -> Option<ValidatedToken> {
    // v2 wire: v2.{session_id}.{expires_ms}.{mac}
    let mut parts = value.splitn(4, '.');
    let version = parts.next()?;
    if version != FORMAT_VERSION {
        return None;
    }
    let session_id = parts.next()?;
    let exp_str = parts.next()?;
    let mac_b64 = parts.next()?;
    // No trailing data — splitn(4) caps at 4 components.
    if mac_b64.contains('.') {
        return None;
    }
    if session_id.is_empty() || session_id.len() > 64 {
        return None;
    }
    let expires_ms: u64 = exp_str.parse().ok()?;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    if expires_ms <= now_ms {
        return None;
    }

    let provided = URL_SAFE_NO_PAD.decode(mac_b64.as_bytes()).ok()?;
    if provided.len() != TRUNCATED_MAC_LEN {
        return None;
    }

    let payload = format!("{session_id}.{exp_str}");
    let mut mac = HmacSha256::new_from_slice(&secret()).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    mac.verify_truncated_left(&provided).ok()?;
    Some(ValidatedToken {
        session_id: session_id.to_string(),
        expires_ms,
    })
}

/// Build a `Set-Cookie` header value for a freshly issued session.
///
/// `public_url` is the configured `BNB_PUBLIC_URL` (if any). When it starts
/// with `https://` the `Secure` attribute is set; on a LAN-only deployment
/// without HTTPS, `Secure` would prevent the browser from sending the
/// cookie back over HTTP, so we omit it.
#[must_use]
pub fn build_set_cookie(token: &str, ttl_ms: u64, public_url: Option<&str>) -> String {
    let max_age_secs = ttl_ms / 1000;
    let secure = if public_url.is_some_and(|u| u.starts_with("https://")) {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{COOKIE_NAME}={token}; HttpOnly; SameSite=Lax; Path={path}; Max-Age={max_age_secs}{secure}",
        path = crate::base_path::current().cookie_path()
    )
}

/// Build a `Set-Cookie` header value that immediately clears the cookie.
#[must_use]
pub fn build_clear_cookie(public_url: Option<&str>) -> String {
    let secure = if public_url.is_some_and(|u| u.starts_with("https://")) {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{COOKIE_NAME}=; HttpOnly; SameSite=Lax; Path={path}; Max-Age=0{secure}",
        path = crate::base_path::current().cookie_path()
    )
}

/// Extract the `bnb-session` cookie value from a request's `Cookie` header.
///
/// Returns `None` when the header is missing or does not contain the
/// session cookie. Does not validate the token — pair with
/// [`validate_token`].
#[must_use]
pub fn extract_token(cookie_header: &str) -> Option<&str> {
    for raw in cookie_header.split(';') {
        let pair = raw.trim();
        if let Some(value) = pair.strip_prefix(&format!("{COOKIE_NAME}=")) {
            return Some(value);
        }
    }
    None
}

/// HMAC-only check that a request appears to be coming from a signed-in
/// session — used by the layout renderer to decide whether to show the
/// "Sign out" link in the topnav.
///
/// Validates the cookie's MAC and expiry, but does **not** consult the
/// `sessions` table. The cheap signal is enough for UX purposes; if a
/// revoked-but-still-in-browser cookie surfaces a sign-out button, the
/// `POST /logout` action will simply clear the dead cookie (idempotent
/// no-op on the server side).
///
/// Pages that need stronger guarantees (e.g. the `/admin/*` middleware)
/// continue to do the DB lookup themselves.
#[must_use]
pub fn looks_signed_in(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_token)
        .and_then(validate_token)
        .is_some()
}

// ---------------------------------------------------------------------------
// Device fingerprint
// ---------------------------------------------------------------------------

/// Bytes of the truncated IP MAC stored in `sessions.ip_hash`.
///
/// Sixteen bytes — the same 128-bit margin the session cookie and the share
/// links use. There is nothing to brute-force here (the input space is 2^32
/// addresses, so any hash of an IP is reversible by enumeration to anyone
/// holding the key), which is exactly why the key is the session secret and
/// not a constant: without the secret the column is opaque, and with it the
/// operator can confirm "was this session mine" without the database ever
/// storing a visitor's address in the clear.
const IP_HASH_BYTES: usize = 16;

/// Hash a resolved client address for storage in `sessions.ip_hash`.
///
/// Keyed on the session secret, so rotating the admin password invalidates
/// these alongside the cookies they belong to — the column would otherwise
/// outlive the sessions it describes and become a standing record of who
/// connected from where.
///
/// # Panics
///
/// Only if `Hmac<Sha256>` were to reject a key length, which its contract
/// says it never does — the same `expect` the token path takes.
#[must_use]
pub fn hash_client_ip(ip: std::net::IpAddr) -> String {
    let mut mac = HmacSha256::new_from_slice(&secret()).expect("HMAC accepts any key length");
    // Hash the canonical textual form. An IPv4-mapped IPv6 address is
    // unmapped before it reaches here (`client_ip::TrustedProxies`), so one
    // address has one spelling and therefore one hash.
    mac.update(ip.to_string().as_bytes());
    let tag = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(&tag[..IP_HASH_BYTES])
}

#[cfg(test)]
mod tests {
    // Tests use the per-process random secret (no env vars set), which is
    // stable for the lifetime of one cargo test binary. Mirrors the pattern
    // in `routes::share` tests — `unsafe_code = "deny"` workspace-wide
    // forbids the `std::env::set_var` route.
    use super::*;

    /// DD-15: the secret is created once, read back identical, and protected.
    #[test]
    fn the_persisted_secret_is_created_once_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create_secret(dir.path()).unwrap();
        let second = load_or_create_secret(dir.path()).unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(
            first, second,
            "a second start reads the first start's secret"
        );
        let path = dir.path().join(SECRET_FILE_NAME);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.trim().len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "mode {mode:o}");
        }
        assert!(
            std::fs::read_dir(dir.path()).unwrap().count() == 1,
            "no temp file is left behind"
        );
        // Two stations do not share a secret.
        let other = tempfile::tempdir().unwrap();
        assert_ne!(load_or_create_secret(other.path()).unwrap(), first);
    }

    #[test]
    fn a_corrupt_secret_file_is_an_error_not_a_new_secret() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(SECRET_FILE_NAME), "not-hex\n").unwrap();
        let err = load_or_create_secret(dir.path()).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
    }

    #[test]
    fn looks_signed_in_true_for_fresh_token() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let session_id = generate_session_id();
        let token = issue_token(&session_id, 60_000);
        let mut headers = HeaderMap::new();
        let cookie_value = format!("other=42; {COOKIE_NAME}={token}; foo=bar");
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&cookie_value).unwrap(),
        );
        assert!(looks_signed_in(&headers));
    }

    #[test]
    fn looks_signed_in_false_without_cookie() {
        use axum::http::HeaderMap;
        let headers = HeaderMap::new();
        assert!(!looks_signed_in(&headers));
    }

    #[test]
    fn looks_signed_in_false_for_expired_token() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let token = encode_token("sid", 0); // already expired
        let mut headers = HeaderMap::new();
        let cookie_value = format!("{COOKIE_NAME}={token}");
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&cookie_value).unwrap(),
        );
        assert!(!looks_signed_in(&headers));
    }

    #[test]
    fn looks_signed_in_false_for_tampered_mac() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let session_id = generate_session_id();
        let mut token = issue_token(&session_id, 60_000);
        // Flip the last byte of the MAC — invalidates the signature.
        token.pop();
        token.push('a');
        let mut headers = HeaderMap::new();
        let cookie_value = format!("{COOKIE_NAME}={token}");
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&cookie_value).unwrap(),
        );
        assert!(!looks_signed_in(&headers));
    }

    #[test]
    fn round_trip_validates() {
        let session_id = generate_session_id();
        let token = issue_token(&session_id, 60_000);
        let parsed = validate_token(&token).expect("fresh token validates");
        assert_eq!(parsed.session_id, session_id);
        assert!(parsed.expires_ms > 0);
    }

    #[test]
    fn expired_token_rejected() {
        let token = encode_token("sid", 0); // already expired
        assert!(validate_token(&token).is_none());
    }

    #[test]
    fn legacy_v1_token_rejected() {
        // The previous (pre-flip) wire format. Operators re-sign-in
        // after upgrade because we can't bind a v1 cookie to a session row.
        assert!(validate_token("v1.99999999999.AAAAAAAAAAAAAAAAAAAAAA").is_none());
    }

    #[test]
    fn tampered_session_id_rejected() {
        let token = issue_token("real-sid", 60_000);
        let parts: Vec<&str> = token.split('.').collect();
        let bad = format!("{}.{}.{}.{}", parts[0], "evil-sid", parts[2], parts[3]);
        assert!(validate_token(&bad).is_none());
    }

    #[test]
    fn tampered_mac_rejected() {
        let token = issue_token("sid", 60_000);
        let mut parts: Vec<&str> = token.split('.').collect();
        let len = parts.len();
        parts[len - 1] = "AAAAAAAAAAAAAAAAAAAAAA";
        assert!(validate_token(&parts.join(".")).is_none());
    }

    #[test]
    fn tampered_expiry_rejected() {
        let token = issue_token("sid", 60_000);
        let parts: Vec<&str> = token.split('.').collect();
        // Bump the expiry — MAC no longer covers the modified expiry.
        let later = u64::MAX.to_string();
        let bad = format!("{}.{}.{}.{}", parts[0], parts[1], later, parts[3]);
        assert!(validate_token(&bad).is_none());
    }

    #[test]
    fn malformed_tokens_rejected() {
        assert!(validate_token("").is_none());
        assert!(validate_token("v2.sid.123").is_none()); // missing mac
        assert!(validate_token("v2.sid.notanumber.abc").is_none());
        assert!(validate_token("v2..123.abc").is_none()); // empty sid
        // Trailing components after the MAC are rejected (defends against
        // any caller appending extra fields).
        assert!(validate_token("v2.sid.123.abc.extra").is_none());
    }

    #[test]
    fn generate_session_id_is_26_lowercase_base32_chars() {
        let id = generate_session_id();
        assert_eq!(id.len(), 26);
        assert!(id.chars().all(|c| matches!(c, 'a'..='z' | '2'..='7')));
        // Two consecutive ids must differ (collision probability ~2^-128).
        let id2 = generate_session_id();
        assert_ne!(id, id2);
    }

    #[test]
    fn extracts_cookie_from_header() {
        let h = "foo=bar; bnb-session=v2.sid.123.abc; other=42";
        assert_eq!(extract_token(h), Some("v2.sid.123.abc"));
        assert_eq!(extract_token("only=value"), None);
        assert_eq!(extract_token(""), None);
    }

    #[test]
    fn set_cookie_includes_secure_only_on_https() {
        let v = build_set_cookie("v2.sid.1.x", 60_000, Some("https://birds.example.com"));
        assert!(v.contains("Secure"));
        assert!(v.contains("Max-Age=60"));

        let v = build_set_cookie("v2.sid.1.x", 60_000, Some("http://birds.local"));
        assert!(!v.contains("Secure"));

        let v = build_set_cookie("v2.sid.1.x", 60_000, None);
        assert!(!v.contains("Secure"));
    }

    #[test]
    fn clear_cookie_zeros_max_age() {
        let v = build_clear_cookie(None);
        assert!(v.contains("Max-Age=0"));
        assert!(v.contains("bnb-session="));
    }

    /// The two attributes that carry the defence, on both cookies.
    ///
    /// `HttpOnly` keeps the session token out of reach of any script that
    /// gets to run in the page (a stored XSS in a species name, say), and
    /// `SameSite=Lax` is half of the CSRF defence: a cross-site form post does
    /// not carry the cookie, so the same-origin check in `security.rs` is the
    /// second half rather than the only one. Neither was asserted anywhere;
    /// deleting `SameSite=Lax` from `build_set_cookie` passed the entire suite
    /// (RC-6). Observed failing against exactly that deletion:
    /// `the issued cookie has lost SameSite=Lax`.
    #[test]
    fn both_cookies_carry_httponly_and_samesite_lax() {
        for (label, cookie) in [
            (
                "issued",
                build_set_cookie("v2.sid.1.x", 60_000, Some("https://birds.example.com")),
            ),
            (
                "issued (plain http)",
                build_set_cookie("v2.sid.1.x", 60_000, None),
            ),
            ("cleared", build_clear_cookie(None)),
        ] {
            let attrs: Vec<&str> = cookie.split("; ").skip(1).collect();
            assert!(
                attrs.contains(&"HttpOnly"),
                "the {label} cookie has lost HttpOnly: {cookie}"
            );
            assert!(
                attrs.contains(&"SameSite=Lax"),
                "the {label} cookie has lost SameSite=Lax: {cookie}"
            );
            // Not Strict: a link from an email or a chat into `/station`
            // must arrive signed in, and Strict would drop the cookie on
            // exactly that top-level navigation.
            assert!(
                !attrs
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case("SameSite=Strict")),
                "{label}: {cookie}"
            );
        }
    }
}
