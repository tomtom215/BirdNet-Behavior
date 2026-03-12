//! HTTP Basic Authentication middleware.
//!
//! Provides optional basic auth matching the Caddy reverse proxy setup
//! used in BirdNET-Pi. When enabled, all API and page routes require
//! valid credentials. Health and WebSocket endpoints are excluded
//! to support monitoring tools and live detection streams.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Authentication configuration.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Username for basic auth.
    username: String,
    /// Password for basic auth (plaintext -- same as Caddy's basicauth).
    password: String,
    /// Paths that bypass authentication (e.g., health checks).
    excluded_paths: Vec<String>,
}

impl AuthConfig {
    /// Create a new auth configuration.
    ///
    /// # Errors
    ///
    /// Returns `None` if username or password is empty.
    pub fn new(username: &str, password: &str) -> Option<Self> {
        if username.is_empty() || password.is_empty() {
            return None;
        }

        Some(Self {
            username: username.to_string(),
            password: password.to_string(),
            excluded_paths: vec![
                "/api/v2/health".to_string(),
                "/api/v2/ws/detections".to_string(),
            ],
        })
    }

    /// Add a path to the exclusion list.
    pub fn exclude_path(&mut self, path: &str) {
        self.excluded_paths.push(path.to_string());
    }

    /// Check if a path is excluded from authentication.
    fn is_excluded(&self, path: &str) -> bool {
        self.excluded_paths
            .iter()
            .any(|excluded| path.starts_with(excluded))
    }

    /// Validate credentials against the configured username and password.
    fn validate(&self, username: &str, password: &str) -> bool {
        // Constant-time comparison to prevent timing attacks
        let user_match = constant_time_eq(username.as_bytes(), self.username.as_bytes());
        let pass_match = constant_time_eq(password.as_bytes(), self.password.as_bytes());
        user_match && pass_match
    }
}

/// Axum middleware for HTTP Basic Authentication.
///
/// Extracts the `Authorization: Basic <base64>` header, decodes it,
/// and validates against the configured credentials.
pub async fn basic_auth_middleware(
    request: Request<Body>,
    next: Next,
    config: &AuthConfig,
) -> Response {
    let path = request.uri().path();

    // Skip auth for excluded paths
    if config.is_excluded(path) {
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let Some(auth_value) = auth_header else {
        return unauthorized_response();
    };

    // Parse "Basic <base64>" format
    let Some(credentials) = auth_value.strip_prefix("Basic ") else {
        return unauthorized_response();
    };

    // Decode base64
    let Ok(decoded) = base64_decode(credentials.trim()) else {
        return unauthorized_response();
    };

    let Ok(decoded_str) = std::str::from_utf8(&decoded) else {
        return unauthorized_response();
    };

    // Split "username:password"
    let Some((username, password)) = decoded_str.split_once(':') else {
        return unauthorized_response();
    };

    if config.validate(username, password) {
        next.run(request).await
    } else {
        unauthorized_response()
    }
}

/// Return a 401 Unauthorized response with WWW-Authenticate header.
fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Basic realm=\"BirdNet-Behavior\"")],
        "Unauthorized",
    )
        .into_response()
}

/// Decode a base64 string (RFC 4648 standard alphabet).
///
/// Minimal implementation to avoid pulling in a base64 crate dependency.
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    let input = input.trim_end_matches('=');
    let mut output = Vec::with_capacity(input.len() * 3 / 4);

    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for byte in input.bytes() {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' | b' ' => continue,
            _ => return Err(()),
        };

        buf = (buf << 6) | u32::from(val);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            #[allow(clippy::cast_possible_truncation)]
            output.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }

    Ok(output)
}

/// Constant-time byte comparison to prevent timing attacks.
///
/// Returns `true` if slices are equal, always examining all bytes.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0_u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_rejects_empty_credentials() {
        assert!(AuthConfig::new("", "pass").is_none());
        assert!(AuthConfig::new("user", "").is_none());
        assert!(AuthConfig::new("", "").is_none());
    }

    #[test]
    fn auth_config_accepts_valid_credentials() {
        let config = AuthConfig::new("admin", "secret").unwrap();
        assert!(config.validate("admin", "secret"));
    }

    #[test]
    fn auth_config_rejects_wrong_credentials() {
        let config = AuthConfig::new("admin", "secret").unwrap();
        assert!(!config.validate("admin", "wrong"));
        assert!(!config.validate("wrong", "secret"));
        assert!(!config.validate("wrong", "wrong"));
    }

    #[test]
    fn excluded_paths() {
        let config = AuthConfig::new("admin", "secret").unwrap();
        assert!(config.is_excluded("/api/v2/health"));
        assert!(config.is_excluded("/api/v2/ws/detections"));
        assert!(!config.is_excluded("/api/v2/detections"));
        assert!(!config.is_excluded("/"));
    }

    #[test]
    fn exclude_path_custom() {
        let mut config = AuthConfig::new("admin", "secret").unwrap();
        config.exclude_path("/api/v2/stats");
        assert!(config.is_excluded("/api/v2/stats"));
    }

    #[test]
    fn base64_decode_valid() {
        // "admin:secret" → "YWRtaW46c2VjcmV0"
        let decoded = base64_decode("YWRtaW46c2VjcmV0").unwrap();
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), "admin:secret");
    }

    #[test]
    fn base64_decode_with_padding() {
        // "a" → "YQ=="
        let decoded = base64_decode("YQ==").unwrap();
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), "a");
    }

    #[test]
    fn base64_decode_longer() {
        // "Hello, World!" → "SGVsbG8sIFdvcmxkIQ=="
        let decoded = base64_decode("SGVsbG8sIFdvcmxkIQ==").unwrap();
        assert_eq!(std::str::from_utf8(&decoded).unwrap(), "Hello, World!");
    }

    #[test]
    fn base64_decode_invalid() {
        assert!(base64_decode("!!!invalid!!!").is_err());
    }

    #[test]
    fn constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_not_equal() {
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hello", b"hell"));
        assert!(!constant_time_eq(b"a", b"b"));
    }

    #[test]
    fn constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
