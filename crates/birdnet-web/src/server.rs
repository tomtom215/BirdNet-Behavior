//! Axum server setup and lifecycle.
//!
//! Configures the axum Router with Tower middleware (`CORS`, tracing,
//! rate limiting, a stateless CSRF guard, and response-hardening security
//! headers), mounts API routes, and manages graceful shutdown.

use axum::Router;
use axum::http::{HeaderValue, Method, header};
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::rate_limit::{RateLimitConfig, RateLimiter};
use crate::routes;
use crate::state::AppState;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Listen address (default: 127.0.0.1:8502).
    pub addr: SocketAddr,
    /// Path to the `SQLite` database.
    pub db_path: PathBuf,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 8502)),
            db_path: PathBuf::from("birds.db"),
        }
    }
}

/// Server errors.
#[derive(Debug)]
pub enum ServerError {
    /// Failed to bind to address.
    Bind(String),
    /// Server runtime error.
    Runtime(String),
    /// Database initialization error.
    Database(String),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "bind error: {msg}"),
            Self::Runtime(msg) => write!(f, "runtime error: {msg}"),
            Self::Database(msg) => write!(f, "database error: {msg}"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Build the axum application router with all middleware and routes,
/// gating `/admin/*` behind the v2 cookie-session middleware.
///
/// Applies a per-IP token-bucket rate limiter (30 req/s, burst 60) to
/// protect the API from overload.  Static assets and WebSocket connections
/// share the same limit bucket as API calls but are lightweight by nature.
///
/// Admin authentication is cookie-based (the O-14/O-15 wire flip):
/// `CADDY_USER`/`CADDY_PWD` feed the cookie middleware via the env (the
/// bootstrap in `helpers::auth` hashes them into the seed admin row's
/// `pwd_argon2`), and the middleware open-bypasses when no admin password
/// is configured — matching the fresh-Pi "no password = open admin" contract.
pub fn build_router(state: AppState) -> Router {
    // Rate limiter: 30 req/s sustained, 60-request burst per IP.
    build_router_with_rate_limit(state, RateLimitConfig::default())
}

/// Build the router with an explicit rate-limit configuration.
///
/// [`build_router`] uses the shipped default and is what the station runs. This
/// variant exists for the visual-QA fixture (`examples/screenshot_server`),
/// which is deliberately hammered by a machine-speed harness: the sweep loads
/// 152 pages back to back, far above any rate a browser produces. Throttling
/// the harness that exists to hammer this server tests nothing about the
/// product, and turns an unlucky burst into a red build on an asset `429`.
///
/// Measured for contrast, so the default is not adjusted on a hunch: a cold
/// dashboard load is 24 requests, the heaviest page (recordings) 34, and two
/// rapid dashboard loads back to back 48 — all comfortably inside the 60-burst
/// default, with no `429`. Real clients are nowhere near the limit, so the
/// production numbers stay exactly as they were.
pub fn build_router_with_rate_limit(state: AppState, rate_limit: RateLimitConfig) -> Router {
    let limiter = Arc::new(RateLimiter::new(rate_limit));

    // Gate `/admin/*` behind the v2 cookie middleware. The middleware
    // handles the "no admin password configured" bypass internally
    // (matches the pre-flip basic-auth contract).
    // `/admin` *and* every state-changing page route go behind the same gate.
    // The page routes were public until it was noticed that they let anyone who
    // could load the dashboard delete a detection, rewrite a review verdict or
    // change the station's configuration — none of which is "viewing". The
    // middleware still bypasses entirely when no admin password is configured,
    // so a fresh station is unaffected.
    let admin = routes::admin_routes().merge(routes::pages::mutating_router());
    let admin = crate::auth_middleware::apply(admin, state.clone());

    let request_metrics = state.metrics();
    let router = routes::public_routes().merge(admin).with_state(state);

    // Layer order is outermost-last. The CSRF guard runs after rate limiting
    // (so request floods are still throttled) and before auth, rejecting
    // cross-site state-changing requests. The security-headers layer is added
    // last so it decorates *every* response — 401/404/429, static files, and
    // handler output alike.
    router
        .layer(axum::middleware::from_fn(
            crate::security::csrf_guard_middleware,
        ))
        .layer(axum::middleware::from_fn(move |req, next| {
            let limiter = Arc::clone(&limiter);
            crate::rate_limit::rate_limit_middleware(limiter, req, next)
        }))
        .layer(TraceLayer::new_for_http())
        .layer(build_cors_layer())
        .layer(axum::middleware::from_fn(
            crate::security::security_headers_middleware,
        ))
        // Outside everything else so it measures what the client actually
        // experienced — compression and header rewriting included — rather
        // than the handler in isolation.
        .layer(axum::middleware::from_fn(move |req, next| {
            let metrics = Arc::clone(&request_metrics);
            crate::metrics::http_metrics_middleware(metrics, req, next)
        }))
        // Outermost, and it has to be: `security_headers_middleware` buffers
        // every `text/html` body and runs `String::from_utf8_lossy` over it to
        // stamp CSP nonces. Placed *inside* that layer, this one handed it a
        // gzip stream still labelled `text/html`, and every byte above 0x7F —
        // starting with gzip's own `0x8b` magic — came back as U+FFFD. The
        // wire bytes looked plausible (`1f ef bf bd 08 …`, right length,
        // correct `Content-Encoding`) and no browser could decode a single
        // page. Compression must see the finished body, so it goes last.
        .layer(CompressionLayer::new().compress_when(should_compress))
}

/// Content types worth compressing, as prefixes matched against `Content-Type`.
///
/// An **allow**-list, not a deny-list, and deliberately so — see
/// [`should_compress`].
const COMPRESSIBLE_PREFIXES: &[&str] = &[
    "text/", // html, css, plain, calendar (the .ics feeds)
    // — but NOT text/event-stream; see the explicit refusal in should_compress.
    "application/json",          // the whole v2 API
    "application/javascript",    // htmx and the four small scripts
    "application/xml",           // OpenAPI, sitemap-shaped things
    "application/rss+xml",       // /feeds/*.rss
    "application/atom+xml",      //
    "application/manifest+json", // the PWA manifest
    "image/svg+xml",             // every chart this app draws
    "font/",                     // the self-hosted webfonts (woff2 is already
                                 // compressed and gains nothing, but the predicate below only *permits*
                                 // compression — gzip on an incompressible body costs a few ms once, and the
                                 // browser caches it immutably).
];

/// Whether a response should be gzipped.
///
/// `tower_http`'s `DefaultPredicate` is a deny-list: everything above 32 bytes
/// that is not gRPC, not `image/*` and not `text/event-stream`. That is the
/// wrong shape for this server, for one reason that matters and one that does:
///
/// * **Range requests.** `/api/v2/recordings/{file}` serves audio with
///   `206 Partial Content` and a `Content-Range` computed from the file. A
///   compressing layer rewrites the body but not that header, so the response
///   describes a byte range it no longer contains and the `<audio>` element
///   gets a corrupt clip. `DefaultPredicate` does not exclude 206, and
///   `audio/wav` is not `image/*`.
/// * **Bodies that are already compressed.** Spectrogram PNGs (now genuinely
///   deflated), the `.tar.gz` backup download, WAV audio. Gzipping those burns
///   Pi CPU to add bytes.
///
/// So this permits only what is known to be text-shaped, and anything new is
/// uncompressed until someone adds it here. `text/event-stream` — the log
/// stream — is not in the list, which is what keeps SSE unbuffered.
///
/// A response that already carries `Content-Encoding` is left alone: the layer
/// itself checks that, but stating it here is cheaper than re-deriving it.
///
/// Public so `tests/responses_are_compressed.rs` can assert the policy directly
/// rather than by inferring it from whichever routes happen to be reachable in
/// a fixture — the negative cases (audio, PNG, SSE, ranged) are the ones worth
/// pinning, and several of them need a file on disk to reach over the wire.
#[must_use]
pub fn should_compress(
    status: axum::http::StatusCode,
    _version: axum::http::Version,
    headers: &axum::http::HeaderMap,
    _extensions: &axum::http::Extensions,
) -> bool {
    use axum::http::{StatusCode, header};

    // 206 carries a Content-Range that compression would falsify.
    if status == StatusCode::PARTIAL_CONTENT || headers.contains_key(header::CONTENT_RANGE) {
        return false;
    }

    let Some(ctype) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let ctype = ctype.trim().to_ascii_lowercase();

    // `text/event-stream` is inside the `text/` prefix and must not be: the
    // live log viewer is an SSE stream, and a compressor buffers to fill its
    // window, so every line would arrive in a clump instead of when it
    // happened. Refused before the prefix scan rather than by shortening the
    // prefix, because `text/` is the right rule for everything else under it.
    if ctype.starts_with("text/event-stream") {
        return false;
    }

    COMPRESSIBLE_PREFIXES.iter().any(|p| ctype.starts_with(p))
}

/// Build the CORS policy.
///
/// Secure by default: a station's own web UI is served from the same origin it
/// calls, so no cross-origin access is needed. Echoing the previous
/// `Access-Control-Allow-Origin: *` let *any* website the operator's browser
/// visited read this station's API over the LAN — so the default now allows
/// **no** cross-origin reads. Set `BIRDNET_CORS_ALLOWED_ORIGINS` (a
/// comma-separated origin list, e.g. `https://dash.example.com`) only when
/// fronting the API from a different origin.
fn build_cors_layer() -> CorsLayer {
    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    match std::env::var("BIRDNET_CORS_ALLOWED_ORIGINS") {
        Ok(raw) if !raw.trim().is_empty() => {
            let origins: Vec<HeaderValue> = raw
                .split(',')
                .filter_map(|o| o.trim().parse::<HeaderValue>().ok())
                .collect();
            if origins.is_empty() {
                cors
            } else {
                tracing::info!(
                    count = origins.len(),
                    "CORS: allowing configured cross-origin origins"
                );
                cors.allow_origin(origins)
            }
        }
        // No configured origins → same-origin only (no ACAO header emitted).
        _ => cors,
    }
}

/// Start the web server.
///
/// # Errors
///
/// Returns `ServerError` if the server fails to bind or start.
pub async fn start(config: ServerConfig) -> Result<(), ServerError> {
    let state = AppState::new(config.db_path).map_err(|e| ServerError::Database(e.to_string()))?;
    let app = build_router(state);

    tracing::info!(addr = %config.addr, "starting web server");

    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .map_err(|e| ServerError::Bind(e.to_string()))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| ServerError::Runtime(e.to_string()))?;

    tracing::info!("web server stopped");
    Ok(())
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received Ctrl+C"),
        () = terminate => tracing::info!("received SIGTERM"),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_router, build_router_with_rate_limit};
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    async fn burst_hits_429(app: axum::Router, requests: usize) -> bool {
        for _ in 0..requests {
            let req = Request::builder()
                .uri("/static/css/app.css")
                .body(Body::empty())
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            if resp.status() == StatusCode::TOO_MANY_REQUESTS {
                return true;
            }
        }
        false
    }

    /// The shipped router must keep the strict limiter.
    ///
    /// `build_router_with_rate_limit` exists so the visual-QA fixture can opt
    /// out of throttling, which makes it newly possible to hand the *station* a
    /// permissive config and have nothing notice. The limiter's own tests cover
    /// the token bucket; this covers the wiring, which is the part that would
    /// silently regress.
    #[tokio::test]
    async fn the_shipped_router_still_rate_limits() {
        assert!(
            burst_hits_429(build_router(test_state()), 400).await,
            "build_router must apply the default limiter (30 req/s, 60 burst)"
        );
    }

    /// And the opt-out actually opts out — otherwise the fixture change is
    /// cosmetic and the visual-QA sweep stays intermittently red.
    #[tokio::test]
    async fn a_permissive_config_does_not_throttle() {
        let app = build_router_with_rate_limit(
            test_state(),
            crate::rate_limit::RateLimitConfig {
                requests_per_second: 100_000.0,
                burst_capacity: 100_000,
                trust_x_forwarded_for: false,
            },
        );
        assert!(
            !burst_hits_429(app, 400).await,
            "a permissive config must serve a burst the default would reject"
        );
    }
}
