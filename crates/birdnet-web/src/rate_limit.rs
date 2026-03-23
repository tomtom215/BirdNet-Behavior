//! Per-IP token-bucket rate limiter implemented without external crates.
//!
//! Provides an axum middleware layer that protects API endpoints from
//! accidental or intentional overload. Each client IP gets its own token
//! bucket: tokens replenish at a configurable rate and excess requests
//! receive a `429 Too Many Requests` response with a `Retry-After` header.
//!
//! ## Design
//!
//! - **Token bucket** — allows controlled bursts while enforcing a sustained
//!   rate limit. No jitter, fully deterministic.
//! - **Lock-per-bucket** — `DashMap`-free; uses a single `Mutex<HashMap>`
//!   which is acceptable since lock contention is minimal at typical bird
//!   station traffic levels (≪ 100 req/s).
//! - **Pruning** — stale entries (no requests in `2 × window_secs`) are
//!   removed periodically to prevent unbounded memory growth.
//! - **X-Forwarded-For** — optional; the first address in that header is
//!   used when the real IP sits behind a trusted reverse proxy.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use birdnet_web::rate_limit::{RateLimiter, RateLimitConfig};
//!
//! let config = RateLimitConfig {
//!     requests_per_second: 20.0,
//!     burst_capacity: 40,
//!     trust_x_forwarded_for: false,
//! };
//! let _limiter = RateLimiter::new(config);
//! ```

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

// ---------------------------------------------------------------------------
// Public configuration
// ---------------------------------------------------------------------------

/// Rate limiter configuration.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Sustained request rate allowed per IP (tokens per second).
    pub requests_per_second: f64,
    /// Maximum burst above the sustained rate.
    pub burst_capacity: u32,
    /// When `true`, use the first entry in `X-Forwarded-For` as the client IP.
    pub trust_x_forwarded_for: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 30.0,
            burst_capacity: 60,
            trust_x_forwarded_for: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Internal bucket state
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Bucket {
    /// Current token count (fractional).
    tokens: f64,
    /// Last time tokens were replenished.
    last_refill: Instant,
    /// Last time any request was seen (for stale-entry pruning).
    last_seen: Instant,
}

impl Bucket {
    fn new(capacity: f64) -> Self {
        let now = Instant::now();
        Self {
            tokens: capacity,
            last_refill: now,
            last_seen: now,
        }
    }

    /// Add tokens proportional to elapsed time and try to consume one.
    ///
    /// Returns `true` if the request is allowed (a token was consumed).
    fn try_consume(&mut self, rate: f64, capacity: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();

        self.tokens = elapsed.mul_add(rate, self.tokens).min(capacity);
        self.last_refill = now;
        self.last_seen = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

/// Shared rate limiter state — cheap to clone (`Arc` inside).
#[derive(Debug, Clone)]
pub struct RateLimiter {
    config: RateLimitConfig,
    buckets: Arc<Mutex<HashMap<IpAddr, Bucket>>>,
}

impl RateLimiter {
    /// Create a new rate limiter with the given configuration.
    #[must_use]
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Check whether a request from `ip` is allowed.
    ///
    /// Returns `true` if allowed, `false` if the bucket is exhausted.
    pub fn check(&self, ip: IpAddr) -> bool {
        let capacity = f64::from(self.config.burst_capacity);
        let rate = self.config.requests_per_second;

        // On mutex poison (another thread panicked while holding the lock),
        // allow the request rather than cascading the panic.
        let Ok(mut map) = self.buckets.lock() else {
            return true;
        };

        // Prune stale entries when we hit 1024 buckets (amortised O(1)).
        if map.len() > 1024 {
            // Stale threshold: 2× the time to refill a full burst.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let stale_secs = (f64::from(self.config.burst_capacity)
                / self.config.requests_per_second
                * 2.0) as u64;
            let stale_after = Duration::from_secs(stale_secs);
            map.retain(|_, b| b.last_seen.elapsed() < stale_after);
        }

        let bucket = map.entry(ip).or_insert_with(|| Bucket::new(capacity));
        bucket.try_consume(rate, capacity)
    }

    /// Return the number of active IP buckets (for diagnostics).
    #[must_use]
    pub fn active_buckets(&self) -> usize {
        self.buckets.lock().map_or(0, |m| m.len())
    }
}

// ---------------------------------------------------------------------------
// Axum middleware
// ---------------------------------------------------------------------------

/// Axum middleware that enforces the rate limit.
///
/// Add it with `axum::middleware::from_fn_with_state` or as a layer.
pub async fn rate_limit_middleware(
    limiter: Arc<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_ip(&req, limiter.config.trust_x_forwarded_for);

    if !limiter.check(ip) {
        tracing::debug!(ip = %ip, "rate limit exceeded");
        // Retry delay: at least 1 second, or the time to earn one token.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let retry_after = ((1.0_f64 / limiter.config.requests_per_second).ceil() as u64).max(1);
        let mut response = Response::builder()
            .status(StatusCode::TOO_MANY_REQUESTS)
            .body(Body::from(
                r#"{"error":"rate limit exceeded","retry_after_seconds":1}"#,
            ))
            .unwrap_or_default();
        response.headers_mut().insert(
            "Retry-After",
            HeaderValue::from_str(&retry_after.to_string())
                .unwrap_or_else(|_| HeaderValue::from_static("1")),
        );
        response.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        return response;
    }

    next.run(req).await
}

/// Extract the client IP from the request.
///
/// Prefers `X-Forwarded-For` when `trust_xff` is enabled.
fn extract_ip(req: &Request<Body>, trust_xff: bool) -> IpAddr {
    if trust_xff
        && let Some(xff) = req.headers().get("x-forwarded-for")
        && let Ok(val) = xff.to_str()
        && let Some(first) = val.split(',').next()
        && let Ok(ip) = first.trim().parse::<IpAddr>()
    {
        return ip;
    }

    // Fall back to the socket address from axum's `ConnectInfo`.
    req.extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map_or_else(|| IpAddr::from([127, 0, 0, 1]), |ci| ci.0.ip())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_ip(last_octet: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, last_octet))
    }

    #[test]
    fn allows_within_burst() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 10.0,
            burst_capacity: 5,
            trust_x_forwarded_for: false,
        });
        let ip = test_ip(1);
        // Should allow the full burst.
        for _ in 0..5 {
            assert!(limiter.check(ip));
        }
        // Burst exhausted.
        assert!(!limiter.check(ip));
    }

    #[test]
    fn different_ips_have_independent_buckets() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 1.0,
            burst_capacity: 2,
            trust_x_forwarded_for: false,
        });
        let ip_a = test_ip(10);
        let ip_b = test_ip(11);

        assert!(limiter.check(ip_a));
        assert!(limiter.check(ip_a));
        assert!(!limiter.check(ip_a)); // Exhausted.

        // ip_b still has full burst.
        assert!(limiter.check(ip_b));
        assert!(limiter.check(ip_b));
    }

    #[test]
    fn active_buckets_reflects_unique_ips() {
        let limiter = RateLimiter::new(RateLimitConfig::default());
        assert_eq!(limiter.active_buckets(), 0);
        limiter.check(test_ip(1));
        limiter.check(test_ip(2));
        assert_eq!(limiter.active_buckets(), 2);
        // Same IP again — no new bucket.
        limiter.check(test_ip(1));
        assert_eq!(limiter.active_buckets(), 2);
    }

    #[test]
    fn tokens_refill_over_time() {
        // 100 rps → 1 token per 10 ms. Back-to-back calls (<< 10 ms apart) won't refill.
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 100.0,
            burst_capacity: 1,
            trust_x_forwarded_for: false,
        });
        let ip = test_ip(42);
        assert!(limiter.check(ip)); // Consume the one token.
        assert!(!limiter.check(ip)); // Still exhausted (< 10 ms elapsed).
        // After 20 ms, at least one token has refilled.
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.check(ip)); // Refilled.
    }
}
