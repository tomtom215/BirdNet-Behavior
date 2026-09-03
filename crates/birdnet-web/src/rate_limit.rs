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
//! - **Client identity** — delegated to [`crate::client_ip::TrustedProxies`],
//!   which decides whether a forwarded header may be believed at all. This
//!   used to be a `trust_x_forwarded_for` boolean and neither of its settings
//!   was correct: see that module's own documentation for the probe output.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use birdnet_web::rate_limit::{RateLimiter, RateLimitConfig};
//!
//! let config = RateLimitConfig {
//!     requests_per_second: 20.0,
//!     burst_capacity: 40,
//!     ..RateLimitConfig::default()
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

use crate::client_ip::{ClientIp, TrustedProxies};
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
    /// Peers whose forwarded client-IP headers may be believed.
    ///
    /// Buckets are keyed on the address this resolves to, so getting it wrong
    /// is not cosmetic: trusting too little puts an entire household behind a
    /// proxy into one bucket, and trusting too much lets any client mint a
    /// fresh bucket per request by setting a header.
    pub trusted_proxies: TrustedProxies,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 30.0,
            burst_capacity: 60,
            trusted_proxies: TrustedProxies::default(),
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
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let ip = extract_ip(&req, &limiter.config.trusted_proxies);
    // Publish the resolution so every handler downstream agrees with the
    // limiter about who is calling. This layer is the first that needs it, so
    // resolving once here costs nothing and removes the temptation for a
    // handler to read `ConnectInfo` and get the proxy instead.
    req.extensions_mut().insert(ClientIp(ip));

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
/// The peer comes from axum's [`ConnectInfo`]; whether any forwarded header
/// may override it is [`TrustedProxies`]' decision, not this function's.
///
/// A request with no `ConnectInfo` at all is not a real connection — it is a
/// direct `oneshot` in a test, or a future transport that forgot to insert it.
/// Falling back to loopback keeps such a request in one shared bucket rather
/// than giving it an unthrottled path, and loopback is trusted by default, so
/// a test that sets forwarded headers still exercises the proxied path.
pub(crate) fn extract_ip(req: &Request<Body>, trusted: &TrustedProxies) -> IpAddr {
    let peer = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map_or_else(|| IpAddr::from([127, 0, 0, 1]), |ci| ci.0.ip());
    trusted.client_ip(req.headers(), peer)
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
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
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
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
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
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
        });
        let ip = test_ip(42);
        assert!(limiter.check(ip)); // Consume the one token.
        assert!(!limiter.check(ip)); // Still exhausted (< 10 ms elapsed).
        // After 20 ms, at least one token has refilled.
        std::thread::sleep(Duration::from_millis(20));
        assert!(limiter.check(ip)); // Refilled.
    }

    // -----------------------------------------------------------------------
    // Client identity: which address the bucket is keyed on
    // -----------------------------------------------------------------------

    /// Build a request with an optional peer and forwarded header.
    fn probe(peer: Option<&str>, xff: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().uri("/");
        if let Some(v) = xff {
            b = b.header("x-forwarded-for", v);
        }
        let mut r = b.body(Body::empty()).unwrap();
        if let Some(p) = peer {
            let sock: std::net::SocketAddr = format!("{p}:40000").parse().unwrap();
            r.extensions_mut().insert(ConnectInfo(sock));
        }
        r
    }

    /// The two halves the old boolean could not satisfy at once, asserted
    /// through the function the middleware actually calls.
    ///
    /// A probe against the previous implementation recorded both failures:
    /// `trust=false` gave A=203.0.113.5 (right) and B=127.0.0.1 (wrong);
    /// `trust=true` gave A=9.9.9.9 (wrong) and B=9.9.9.9 (right).
    #[test]
    fn the_bucket_key_ignores_a_forged_header_and_honours_a_proxied_one() {
        let trusted = crate::client_ip::TrustedProxies::default();

        assert_eq!(
            extract_ip(&probe(Some("203.0.113.5"), Some("9.9.9.9")), &trusted),
            "203.0.113.5".parse::<IpAddr>().unwrap(),
            "a direct client must not be able to pick its own bucket"
        );
        assert_eq!(
            extract_ip(&probe(Some("127.0.0.1"), Some("9.9.9.9")), &trusted),
            "9.9.9.9".parse::<IpAddr>().unwrap(),
            "behind a local proxy every visitor must get their own bucket"
        );
    }

    /// The consequence, stated in the limiter's own terms: two visitors
    /// arriving through the same proxy must not share a burst.
    ///
    /// This is what the shipped default actually did — `trust_x_forwarded_for`
    /// was `false` and nothing could set it — so one busy tab throttled the
    /// whole household.
    #[test]
    fn two_visitors_through_one_proxy_do_not_share_a_burst() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 1.0,
            burst_capacity: 2,
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
        });
        let trusted = crate::client_ip::TrustedProxies::default();

        let a = extract_ip(&probe(Some("127.0.0.1"), Some("198.51.100.7")), &trusted);
        let b = extract_ip(&probe(Some("127.0.0.1"), Some("198.51.100.8")), &trusted);

        assert!(limiter.check(a));
        assert!(limiter.check(a));
        assert!(!limiter.check(a), "visitor A has spent their burst");
        assert!(limiter.check(b), "visitor B must still have theirs");
    }

    /// And the counterpart, or the test above is satisfied by any rule that
    /// reads the header: two *forged* headers from the same untrusted peer
    /// must land in one bucket, not two. Otherwise the limiter is trivially
    /// defeated by varying a header.
    #[test]
    fn one_untrusted_peer_cannot_split_itself_across_buckets() {
        let limiter = RateLimiter::new(RateLimitConfig {
            requests_per_second: 1.0,
            burst_capacity: 2,
            trusted_proxies: crate::client_ip::TrustedProxies::default(),
        });
        let trusted = crate::client_ip::TrustedProxies::default();

        let a = extract_ip(&probe(Some("203.0.113.5"), Some("198.51.100.7")), &trusted);
        let b = extract_ip(&probe(Some("203.0.113.5"), Some("198.51.100.8")), &trusted);
        assert_eq!(a, b, "both requests are the same client");

        assert!(limiter.check(a));
        assert!(limiter.check(b));
        assert!(
            !limiter.check(b),
            "a second forged header must not buy a fresh burst"
        );
    }

    /// A request with no `ConnectInfo` — a direct `oneshot` in a test — falls
    /// back to loopback rather than to an unthrottled path.
    #[test]
    fn a_request_without_connect_info_falls_back_to_loopback() {
        let trusted = crate::client_ip::TrustedProxies::loopback_only();
        assert_eq!(
            extract_ip(&probe(None, None), &trusted),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }
}
