//! Axum server setup and lifecycle.
//!
//! Configures the axum Router with Tower middleware (CORS, tracing),
//! mounts API routes, and manages graceful shutdown.

use axum::Router;
use std::fmt;
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::routes;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Listen address (default: 127.0.0.1:8502).
    pub addr: SocketAddr,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 8502)),
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
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "bind error: {msg}"),
            Self::Runtime(msg) => write!(f, "runtime error: {msg}"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Build the axum application router with all middleware and routes.
pub fn build_router() -> Router {
    Router::new()
        .merge(routes::api_routes())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::new())
}

/// Start the web server.
///
/// # Errors
///
/// Returns `ServerError` if the server fails to bind or start.
pub async fn start(config: ServerConfig) -> Result<(), ServerError> {
    let app = build_router();

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
