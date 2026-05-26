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

/// Build the axum application router with all middleware and routes.
pub fn build_router(state: AppState) -> Router {
    build_router_with_auth(state, None)
}

/// Build the axum application router with optional basic authentication.
///
/// Applies a per-IP token-bucket rate limiter (30 req/s, burst 60) to
/// protect the API from overload.  Static assets and WebSocket connections
/// share the same limit bucket as API calls but are lightweight by nature.
pub fn build_router_with_auth(
    state: AppState,
    auth_config: Option<crate::auth::AuthConfig>,
) -> Router {
    // Rate limiter: 30 req/s sustained, 60-request burst per IP.
    let limiter = Arc::new(RateLimiter::new(RateLimitConfig::default()));

    let router = Router::new().merge(routes::api_routes()).with_state(state);

    let router = if let Some(config) = auth_config {
        let config = Arc::new(config);
        router.layer(axum::middleware::from_fn(move |req, next| {
            let config = Arc::clone(&config);
            async move { crate::auth::basic_auth_middleware(req, next, &config).await }
        }))
    } else {
        router
    };

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
