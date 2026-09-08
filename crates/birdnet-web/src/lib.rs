//! BirdNet-Behavior web server.
//!
//! REST API, WebSocket, and HTMX page serving via axum.

pub mod analytics_cache;
pub mod api_token;
pub mod audit;
pub mod auth_middleware;
pub mod base_path;
pub mod boot_journal;
pub mod client_ip;
pub mod data_volume;
pub mod db_pool;
pub mod diagnostics;
pub mod login_throttle;
pub mod metrics;
pub mod notification_probes;
pub mod notifier;
pub mod rate_limit;
pub mod routes;
pub mod security;
pub mod server;
pub mod session;
pub mod state;
pub mod station_conditions;
pub mod system_info;
pub mod tls;
pub mod tracking;
pub mod urls;
