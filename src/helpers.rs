//! Helper functions for the main binary entry point.
//!
//! Extracted from `main.rs` for modularity, then split by responsibility:
//!
//! - `db` — database-path resolution and the run-and-exit maintenance
//!   commands (`--check-db`, `--backup-db`).
//! - `settings_overlay` — bridge the admin-UI settings table onto the
//!   file-based config so saved settings actually take effect.
//! - `state` — `AppState` construction and the optional-subsystem
//!   initialisers (image cache, i18n, site name, DuckDB
//!   analytics).
//! - `system` — OS-level integration (the disk-manager background thread,
//!   Avahi mDNS service registration).
//!
//! This module is a thin facade that re-exports the public surface so
//! callers keep using `helpers::<fn>` unchanged.

mod auth;
mod db;
mod settings_overlay;
mod state;
mod system;

#[cfg(test)]
mod test_support;

pub use auth::bootstrap_admin_password;
pub use db::{db_path_from_config, run_backup, run_integrity_check};
pub use settings_overlay::{overlay_db_settings, seed_db_settings_from_config};
pub use state::{init_i18n, init_image_cache, init_site_name, run_refresh_extension};
pub use system::{maybe_install_avahi_service, start_disk_manager, start_live_spectrogram};

#[cfg(feature = "analytics")]
pub use state::build_state_with_analytics;
