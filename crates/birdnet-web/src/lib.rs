//! BirdNet-Behavior web server.
//!
//! REST API, WebSocket, and HTMX page serving via axum.

pub mod auth;
pub mod rate_limit;
pub mod routes;
pub mod server;
pub mod state;
pub mod system_info;
