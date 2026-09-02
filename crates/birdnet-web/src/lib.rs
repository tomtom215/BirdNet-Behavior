//! BirdNet-Behavior web server.
//!
//! REST API, WebSocket, and HTMX page serving via axum.

pub mod analytics_cache;
pub mod auth_middleware;
pub mod base_path;
pub mod client_ip;
pub mod db_pool;
pub mod metrics;
pub mod rate_limit;
pub mod routes;
pub mod security;
pub mod server;
pub mod session;
pub mod state;
pub mod system_info;
pub mod tls;
pub mod tracking;
pub mod urls;
