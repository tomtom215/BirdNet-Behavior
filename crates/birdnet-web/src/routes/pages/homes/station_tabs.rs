//! The **gated Station management tabs** — Capture · Alerts · Data · Settings ·
//! Access (v3 spine, `Station_home.html`).
//!
//! These fold the twelve flat `/admin/*` pages into the six-group Station home.
//! The public Health tab lives in [`super::station`]; these five are the gated
//! "toolbox". They are mounted **inside the admin router**
//! ([`crate::routes::admin::router`]) so they inherit the same cookie-session
//! gate as `/admin/*`, but they render through the **main** shell (via
//! `render_page_for_request`) with the shared Station sub-tab row, so the
//! toolbox feels like one home rather than a separate admin app.
//!
//! Each tab re-composes the *existing* admin render bodies (audio sources,
//! species lists, alert rules, notification history, backups, import, quality,
//! accounts) plus task-scoped slices of the settings form — so the real forms
//! and their HTMX wiring are reused verbatim and keep posting to their existing
//! `/admin/...` action endpoints; only the page GETs move. The connective
//! tissue (ledes, the danger zone, the kiosk launcher) uses the mock's `st-*`
//! card treatment.
//!
//! Honest omissions (Wave D): the per-source live state-chip / 24 h uptime /
//! retry line need capture-supervisor status the web layer can't see yet; the
//! "bind to localhost" network toggle has no web backend (it's a launch/OS
//! concern) and so is described, not faked.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};

use super::station::station_subtabs;
use crate::auth_middleware::RequestUser;
use crate::routes::admin::settings::render::{SETTINGS_FORM_CSS, Section, render_section_form};
use crate::routes::pages::render_page_for_request;
use crate::state::AppState;

/// The display-preferences card (theme · density · motion · contrast), saved to
/// this device's `localStorage`. Self-contained (its own scoped style + script);
/// the inline script is nonce-stamped by the security layer like any other.
const DISPLAY_PREFS_HTML: &str = include_str!("../../../../templates/_partial_display_prefs.html");

/// Mount the five gated Station tabs.
///
/// Merged into the admin router so the cookie-session gate applies (the page
/// GETs move under `/station/...`; the admin action/partial endpoints keep their
/// `/admin/...` paths).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/station/capture", get(capture_page))
        .route("/station/alerts", get(alerts_page))
        .route("/station/data", get(data_page))
        .route("/station/settings", get(settings_page))
        .route("/station/access", get(access_page))
}

/// Render assembled `content` through the main shell with Station active.
fn page(content: &str, headers: &HeaderMap) -> Html<String> {
    render_page_for_request("Settings", content, "station", headers)
}

// ───────────────────────────────────────────────────────────────────────────
// Capture · "what am I recording?"
// ───────────────────────────────────────────────────────────────────────────

async fn capture_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let content = tokio::task::spawn_blocking(move || build_capture(&state))
        .await
        .unwrap_or_default();
    page(&content, &headers)
}

fn build_capture(state: &AppState) -> String {
    let settings = crate::routes::admin::settings::handler::load_all_settings(state);
    format!(
        r#"{tabs}
<p class="bnb-lede"><b>What your station is listening to, and which birds it keeps.</b> Add microphones or camera streams, tune which species count, and set how sure the model must be.</p>
<h2 class="st-h3" id="audio">Audio sources</h2>
{sources}
<h2 class="st-h3" id="species">Which birds count <span class="st-h3-note">· <a href="/admin/species/test">preview the filter</a> before it affects live detections</span></h2>
{species}
<h2 class="st-h3" id="detection">Capture settings <span class="st-h3-note">· the single home for the detection threshold</span></h2>
{settings_css}
{form}"#,
        tabs = station_subtabs("capture"),
        sources = crate::routes::admin::audio::sources_body(state),
        species = crate::routes::admin::species::handler::species_body(state),
        settings_css = SETTINGS_FORM_CSS,
        form = render_section_form(
            &settings,
            &[Section::Audio, Section::Location, Section::Detection]
        ),
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Alerts · "tell me when…"
// ───────────────────────────────────────────────────────────────────────────

async fn alerts_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let content = tokio::task::spawn_blocking(move || build_alerts(&state))
        .await
        .unwrap_or_default();
    page(&content, &headers)
}

fn build_alerts(state: &AppState) -> String {
    let settings = crate::routes::admin::settings::handler::load_all_settings(state);
    format!(
        r#"{tabs}
<p class="bnb-lede"><b>Get a nudge when something special happens</b> — build a rule, pick where it goes, and send yourself a test before you rely on it.</p>
<div id="rules">{rules}</div>
<h2 class="st-h3" id="channels">Channels — send a test</h2>
{channels}
<h2 class="st-h3" id="notifications">Where alerts flow</h2>
{settings_css}
{form}
{recent}"#,
        tabs = station_subtabs("alerts"),
        rules = crate::routes::admin::rules::rules_body(),
        channels = crate::routes::admin::notification_test::channels_test_body(state),
        settings_css = SETTINGS_FORM_CSS,
        form = render_section_form(&settings, &[Section::Notifications, Section::Email]),
        recent = crate::routes::admin::notifications::recent_body(state),
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Data · "keep it safe / bring it in"
// ───────────────────────────────────────────────────────────────────────────

async fn data_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let content = tokio::task::spawn_blocking(move || build_data(&state))
        .await
        .unwrap_or_default();
    page(&content, &headers)
}

fn build_data(state: &AppState) -> String {
    format!(
        r#"{tabs}
<p class="bnb-lede"><b>Protect your records, bring in your history, and check the data's trustworthy.</b> Backups bundle the database and recordings; import folds a BirdNET-Pi <span class="mono">birds.db</span> in with its original dates intact.</p>
<div id="backups">{backups}</div>
<h2 class="st-h3" id="import">Bring your history with you</h2>
{import}
<div id="quality">{quality}</div>
<div class="card" id="phantoms-section">{phantoms}</div>"#,
        tabs = station_subtabs("data"),
        backups = crate::routes::admin::backup_recovery::backups_body(state),
        import = crate::routes::admin::migration::migration_body(state),
        quality = crate::routes::admin::quality::quality_body(state),
        phantoms = crate::routes::admin::quality::render_phantoms(
            &crate::routes::admin::quality::load_phantoms(state),
        ),
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Settings · "my preferences"
// ───────────────────────────────────────────────────────────────────────────

async fn settings_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let content = tokio::task::spawn_blocking(move || build_settings(&state))
        .await
        .unwrap_or_default();
    page(&content, &headers)
}

fn build_settings(state: &AppState) -> String {
    let settings = crate::routes::admin::settings::handler::load_all_settings(state);
    format!(
        r#"{tabs}
<p class="bnb-lede"><b>Your preferences — the look, the station identity, and the wall display.</b></p>
<h2 class="st-h3" id="display-prefs">Display <span class="st-h3-note">· saved on this device only</span></h2>
{display}
<h2 class="st-h3" id="station-system">Station &amp; system</h2>
{settings_css}
{form}
<h2 class="st-h3" id="kiosk">Wall display</h2>
<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Kiosk mode</div><h3>A screen for the wall</h3></div><a class="bnb-btn ghost" href="/kiosk">Launch →</a></div>
  <div class="st-card-lede">A full-screen, auto-refreshing display for a dedicated screen — latest detections and the live signal. Press <span class="mono">Esc</span> to exit.</div>
</div>"#,
        tabs = station_subtabs("settings"),
        display = DISPLAY_PREFS_HTML,
        settings_css = SETTINGS_FORM_CSS,
        form = render_section_form(&settings, &[Section::System]),
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Access · "who can get in"
// ───────────────────────────────────────────────────────────────────────────

async fn access_page(
    State(state): State<AppState>,
    request_user: RequestUser,
    headers: HeaderMap,
) -> Html<String> {
    let content = tokio::task::spawn_blocking(move || build_access(&state, &request_user))
        .await
        .unwrap_or_default();
    page(&content, &headers)
}

fn build_access(state: &AppState, request_user: &RequestUser) -> String {
    format!(
        r#"{tabs}
<p class="bnb-lede"><b>Who can change your station's settings.</b> Viewing the dashboard is open; only the toolbox is gated.</p>
<div id="accounts">{accounts}</div>
<div id="danger-zone">{danger}</div>"#,
        tabs = station_subtabs("access"),
        accounts = crate::routes::admin::accounts::accounts_body(state, request_user),
        danger = danger_zone(),
    )
}

/// The Access danger zone — the lockout warning around the network/auth knobs.
///
/// Described, not faked: the "bind to localhost" toggle is a launch/OS concern
/// with no web backend, and a password reset prints a one-time link on the
/// device's own console — so this lists them with the lockout caveat rather than
/// shipping a switch that would do nothing.
const fn danger_zone() -> &'static str {
    r#"<div class="bnb-card pad st-danger" data-screen-label="Danger zone">
  <div class="bnb-eyebrow">Danger zone · expert only</div>
  <div class="st-warn">⚠ Network &amp; auth changes can <b>lock you out of the web interface</b>. Recovering then means plugging a monitor &amp; keyboard into the device, or connecting over SSH. If you're not certain, leave these as they are.</div>
  <div class="st-list-row"><div class="lr-main"><div>Bind the dashboard to this device only</div><div class="lr-sub">a launch / OS-firewall setting — makes the station reachable only from the device it runs on</div></div></div>
  <div class="st-list-row"><div class="lr-main"><div>Reset the admin password</div><div class="lr-sub">prints a one-time reset link on the device's own console (needs physical or SSH access)</div></div></div>
</div>"#
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    #[test]
    fn capture_folds_sources_species_and_the_detection_threshold() {
        let html = build_capture(&test_state());
        assert!(html.contains(r#"class="bnb-subtab active""#));
        assert!(html.contains(r#"href="/station/capture""#));
        // The detection threshold's single canonical home is here.
        assert!(html.contains("confidence_threshold"));
        // Species filter + its safe-preview link.
        assert!(html.contains("/admin/species/test"));
        // The settings slice posts to the real, unchanged endpoint.
        assert!(html.contains(r#"hx-post="/admin/settings""#));
    }

    #[test]
    fn alerts_folds_rules_channels_and_recent_sends() {
        let html = build_alerts(&test_state());
        assert!(html.contains("Alert Rules"));
        assert!(html.contains(r#"hx-post="/admin/settings""#));
    }

    #[test]
    fn data_folds_backups_import_and_quality() {
        let html = build_data(&test_state());
        assert!(html.contains("BirdNET-Pi"));
        assert!(html.contains(r#"href="/station/data""#));
    }

    #[test]
    fn settings_folds_display_prefs_and_the_kiosk_launcher() {
        let html = build_settings(&test_state());
        assert!(html.contains("display-prefs"));
        assert!(html.contains(r#"href="/kiosk""#));
        // The detection threshold is NOT duplicated here — Capture owns it.
        assert!(!html.contains("confidence_threshold"));
    }

    #[test]
    fn access_danger_zone_carries_the_lockout_warning() {
        let html = danger_zone();
        assert!(html.contains("st-danger"));
        assert!(html.contains("lock you out"));
        // Honest: the controls are described, not faked into dead switches.
        assert!(!html.contains("st-sw"));
    }
}
