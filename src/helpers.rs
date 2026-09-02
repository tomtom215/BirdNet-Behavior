//! Helper functions for the main binary entry point.
//!
//! Extracted from `main.rs` for modularity, then split by responsibility:
//!
//! - `db` — database-path resolution and the run-and-exit maintenance
//!   commands (`--check-db`, `--backup-db`).
//! - `egress` — which outbound connections the station may make on its own.
//! - `resolve` — the shared precedence rule for a setting the operator can
//!   supply from a CLI flag, the config file, or the admin settings form.
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
pub mod egress;
pub mod resolve;
mod settings_overlay;
mod state;
mod system;
pub mod tls;

#[cfg(test)]
pub mod test_support;

/// Shared with `crate::doctor::config` so the diagnostic and the auth bootstrap
/// resolve `CADDY_PWD` through one implementation instead of two that drifted.
pub use auth::resolve_admin_password;
pub use auth::{bootstrap_admin_password, purge_legacy_credential_settings};
pub use db::{
    db_path_from_config, ensure_db_dir, run_backup, run_integrity_check, run_migration_report,
    run_rebuild_species_summary,
};
pub use settings_overlay::{overlay_db_settings, seed_db_settings_from_config};
pub use state::{
    init_i18n, init_image_cache, init_site_name, run_refresh_extension, run_verify_extension,
};
pub use system::{
    maybe_install_avahi_service, start_disk_manager, start_live_spectrogram, stream_dir,
};

#[cfg(feature = "analytics")]
pub use state::build_state_with_analytics;
