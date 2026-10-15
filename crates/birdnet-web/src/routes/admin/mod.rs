//! Admin panel routes.
//!
//! Provides web UI and REST endpoints for managing the system:
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET  /admin`              | Admin landing page (redirects to /admin/overview) |
//! | `GET  /admin/overview`     | Admin dashboard overview |
//! | `GET  /admin/settings`     | Settings form (all categories) |
//! | `POST /admin/settings`     | Save settings (HTMX partial) |
//! | `GET  /admin/species`      | Species exclusion / allow-list management |
//! | `GET  /admin/migrate`      | BirdNET-Pi migration page |
//! | `POST /admin/migrate/validate` | Pre-flight validation (JSON) |
//! | `POST /admin/migrate/run`  | Start import (async, progress via polling) |
//! | `GET  /admin/migrate/progress` | Poll migration progress (JSON) |
//! | `GET  /admin/system`       | System status page |
//! | `POST /admin/system/backup` | Trigger database backup |
//! | `GET  /admin/system/logs`  | SSE live log stream |
//! | `GET  /admin/system/logs/page` | Live log viewer page |
//! | `GET  /admin/notifications` | Notification history log |
//! | `GET  /admin/notifications/test` | Test notification channels |
//! | `DELETE /admin/notifications/prune` | Prune old log entries |
//! | `GET  /admin/system/backups`        | List database backups |
//! | `GET  /admin/system/backups/{name}` | Download a backup file |
//! | `DELETE /admin/system/backups/{name}` | Delete a backup file |
//! | `GET  /admin/species/test`            | Preview the species filter against this station's species |
//! | `GET  /admin/rules`                   | Alert rules management |
//! | `POST /admin/rules`                   | Create new alert rule |
//! | `POST /admin/rules/{id}/delete`       | Delete an alert rule |
//! | `POST /admin/rules/{id}/toggle`       | Enable / disable an alert rule |
//! | `GET  /admin/quality`                 | Data quality metrics dashboard |

pub mod accounts;
pub mod audio;
pub mod backup;
pub mod backup_recovery;
pub mod doctor;
pub mod eq_curve;
pub mod images;
pub mod logs;
pub mod migration;
pub mod nav;
pub mod notification_test;
pub mod notifications;
pub mod overview;
#[cfg(feature = "profiling")]
pub mod profiling;
pub mod quality;
pub mod rules;
pub mod settings;
pub mod species;
pub mod system;
pub mod system_controls;
pub mod today_layout;
pub mod update;

use std::fmt::Write as _;

use axum::{Router, routing::get};

use crate::state::AppState;

/// Wrap an admin page `body` in the standard standalone admin shell — FOUC
/// theme guard, the design-system stylesheet, HTMX and a slim nav row. `active`
/// highlights the matching nav link.
pub(crate) fn admin_shell(title: &str, active: &str, body: &str) -> String {
    admin_shell_with(title, active, &nav::breadcrumb(active), body)
}

/// Like [`admin_shell`] but for a **sub-page** that lives beneath nav tab
/// `parent_key`: the parent tab stays highlighted and the breadcrumb gains the
/// sub-page leaf (`Home › Admin › <Parent> › <subpage>`), so an operator on,
/// say, the species Filter-test page still sees "Species" lit in the nav and a
/// one-click trail back up.
pub(crate) fn admin_subpage_shell(
    title: &str,
    parent_key: &str,
    subpage: &str,
    body: &str,
) -> String {
    admin_shell_with(
        title,
        parent_key,
        &nav::breadcrumb_subpage(parent_key, subpage),
        body,
    )
}

/// Shared shell renderer. `active` highlights its nav tab; `breadcrumb` is the
/// pre-rendered trail (a top-level crumb from [`nav::breadcrumb`] or a sub-page
/// crumb from [`nav::breadcrumb_subpage`]).
///
/// The page is the **shared layout** with the admin sub-navigation inside it, not
/// a document of its own. It used to be its own `<html>`: a slim nav row, none
/// of the site header, no footer, no theme toggle, and — on a phone — no bottom
/// tab bar, so the one place an operator goes to fix something was the one
/// place with no way back to Today but a breadcrumb, under a nav that wrapped
/// to five rows and pushed the page below the fold. Every admin page is a
/// detail of the Station home, so it is rendered as one: the Station tab is lit
/// and the page takes the Station's width.
fn admin_shell_with(title: &str, active: &str, breadcrumb: &str, body: &str) -> String {
    // The nav row is generated from the single-source manifest in `nav.rs`, so
    // every admin page renders the same tabs with consistent active-state.
    let mut nav_html = nav::nav_links(active);
    // Sign-out form — appears at the right end of the admin nav. The
    // form posts to /logout which revokes the bound session row before
    // clearing the cookie. Safe to always render because /admin/* is
    // gated by the cookie middleware; if you're here, you're signed in
    // (or in the no-password "open admin" bypass, where the action is
    // a no-op). It lives here rather than in the site header (which is told
    // `signed_in = false` below) because this shell has no request to read the
    // session from, and one control is better than two.
    let _ = write!(
        nav_html,
        r#"<form action="/logout" method="post" class="am-logout-form">
  <button type="submit" class="bnb-btn ghost am-signout-btn">Sign out</button>
</form>"#
    );
    let content = format!(
        r#"<nav class="admin-nav" aria-label="Admin sections">{nav_html}</nav>
{breadcrumb}
{body}"#
    );
    crate::routes::pages::render_page_for_request(
        title,
        &content,
        "station",
        &axum::http::HeaderMap::new(),
    )
    .0
}

/// Build the admin router and mount all sub-routes.
pub fn router() -> Router<AppState> {
    Router::new()
        // Landing page → redirect to overview
        .route("/admin", get(landing))
        // Overview dashboard
        .merge(overview::router())
        // Settings
        .merge(settings::router())
        // Audio / microphone setup
        .merge(audio::router())
        // Backups, restore & system admin
        .merge(backup_recovery::router())
        .merge(doctor::router())
        // Species list management
        .merge(species::router())
        // Migration
        .merge(migration::router())
        // System
        .merge(system::router())
        // Live log streaming
        .merge(logs::router())
        // Notification history
        .merge(notifications::router())
        // Notification testing
        .merge(notification_test::router())
        // Backup management
        .merge(backup::router())
        // System controls (clear data)
        .merge(system_controls::router())
        // Image blacklist management
        .merge(images::router())
        // Software update
        .merge(update::router())
        .merge(today_layout::router())
        // Alert rules engine
        .merge(rules::router())
        // Data quality dashboard
        .merge(quality::router())
        // Accounts, sessions, audit log (O-15)
        .merge(accounts::router())
        // v3 spine: the gated Station management tabs (Capture · Alerts · Data ·
        // Settings · Access). Mounted here so they inherit the same cookie gate
        // as `/admin/*`, but they render through the main shell with the shared
        // Station sub-tab row. The folded admin POST/partial endpoints above keep
        // their `/admin/...` paths.
        .merge(crate::routes::pages::homes::station_tabs::router())
        // CPU profiles (G-24): an empty router unless built with `profiling`.
        .merge(profiling_router())
}

#[cfg(feature = "profiling")]
fn profiling_router() -> Router<AppState> {
    profiling::router()
}

#[cfg(not(feature = "profiling"))]
fn profiling_router() -> Router<AppState> {
    Router::new()
}

/// Redirect `/admin` to the Station home (the v3-spine toolbox landing).
async fn landing() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station")
}
