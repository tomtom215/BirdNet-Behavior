//! Notification channel testing routes.
//!
//! Provides a UI to send test messages via each configured notification channel.
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /admin/notifications/test | Test page |
//! | POST   | /admin/notifications/test/apprise | Send Apprise test |
//! | POST   | /admin/notifications/test/birdweather | Send BirdWeather test ping |
//! | POST   | /admin/notifications/test/all | Test all channels |

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};
use std::fmt::Write as _;

use birdnet_db::settings::ensure_settings_table;
use birdnet_db::settings::get as get_setting;

use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

/// Mount the notification channel test routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/notifications/test", get(test_page).post(test_all))
        .route(
            "/admin/notifications/test/apprise",
            axum::routing::post(test_apprise),
        )
        .route(
            "/admin/notifications/test/birdweather",
            axum::routing::post(test_birdweather),
        )
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

async fn test_page(State(state): State<AppState>) -> Html<String> {
    Html(render_test_page_for(&state))
}

fn render_test_page_for(state: &AppState) -> String {
    let (apprise_ok, bw_ok) = channels_configured(state);
    render_test_page(apprise_ok, bw_ok)
}

/// Whether the Apprise and BirdWeather channels have credentials set.
fn channels_configured(state: &AppState) -> (bool, bool) {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        let apprise = get_setting(conn, "apprise_url")
            .ok()
            .is_some_and(|v| !v.is_empty());
        let bw = get_setting(conn, "birdweather_token")
            .ok()
            .is_some_and(|v| !v.is_empty());
        (apprise, bw)
    })
}

/// Render the channels Send-test body (no document shell).
///
/// Shared with the Station **Alerts** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the
/// confirm-before-you-rely-on-it test UI in the main shell.
pub(crate) fn channels_test_body(state: &AppState) -> String {
    let (apprise_ok, bw_ok) = channels_configured(state);
    test_notifications_body(apprise_ok, bw_ok)
}

fn render_test_page(apprise_ok: bool, bw_ok: bool) -> String {
    crate::routes::admin::admin_subpage_shell(
        "Test notifications",
        "notifications",
        "Test",
        &test_notifications_body(apprise_ok, bw_ok),
    )
}

/// Page-specific body (scoped `<style>` + cards). The shared shell supplies the
/// chrome, the nav (with the Notifications tab active), and the
/// `Admin › Notifications › Test` breadcrumb.
fn test_notifications_body(apprise_ok: bool, bw_ok: bool) -> String {
    let apprise_status = if apprise_ok {
        "Configured"
    } else {
        "Not configured"
    };
    let bw_status = if bw_ok {
        "Configured"
    } else {
        "Not configured"
    };
    let apprise_icon = if apprise_ok { "✅" } else { "⚠️" };
    let bw_icon = if bw_ok { "✅" } else { "⚠️" };
    let apprise_disabled = if apprise_ok { "" } else { "disabled" };
    let bw_disabled = if bw_ok { "" } else { "disabled" };
    let apprise_btn = if apprise_ok {
        "btn-primary"
    } else {
        "btn-disabled"
    };
    let bw_btn = if bw_ok { "btn-primary" } else { "btn-disabled" };

    // WEB-3: a disabled test button is a dead end unless we say *why*. Give each
    // button a tooltip AND spell the reason out in the visible hint, because a
    // native `disabled` button suppresses pointer events in most browsers, so
    // its `title` won't fire on hover — the hint is the reliable reason surface.
    let apprise_title = if apprise_ok {
        "Send a test notification through Apprise"
    } else {
        "Disabled — set the Apprise URL in Settings first"
    };
    let bw_title = if bw_ok {
        "Ping the BirdWeather API to verify your token"
    } else {
        "Disabled — set the BirdWeather token in Settings first"
    };
    let apprise_hint = if apprise_ok {
        "Configured — send a test to confirm delivery."
    } else {
        r#"Disabled until you set the Apprise URL in <a href="/admin/settings">Settings</a>."#
    };
    let bw_hint = if bw_ok {
        "Configured — ping to confirm your token."
    } else {
        r#"Disabled until you set the BirdWeather token in <a href="/admin/settings">Settings</a>."#
    };

    let mut html = String::with_capacity(4096);
    html.push_str(r"<style>
      .card { background:var(--surface); border:1px solid var(--border); border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }
      .section-title { font-size:1.1rem; font-weight:600; color:var(--moss-ink); margin-bottom:1rem; border-bottom:1px solid var(--border); padding-bottom:0.5rem; }
      .btn { padding:0.5rem 1.5rem; border-radius:0.375rem; border:none; cursor:pointer; font-weight:600; font-size:0.875rem; }
      .btn-primary { background:var(--moss); color:var(--on-moss); }
      .btn-disabled { background:var(--border); color:var(--fg-4); cursor:not-allowed; }
      .hint { font-size:0.75rem; color:var(--fg-4); margin-bottom:1rem; }
      h1 { font-size:1.5rem; font-weight:700; margin-bottom:1.5rem; color:var(--fg); }
      .hint a { color:var(--moss-ink); }
      .result-banner { border:1px solid; border-radius:0.375rem; padding:0.75rem; margin-top:0.75rem; }
      .result-banner.ok { background:var(--moss-soft); border-color:var(--moss-soft); color:var(--moss-ink); }
      .result-banner.err { background:var(--rare-soft); border-color:var(--rare-soft); color:var(--rare); }
    </style>

  <h1>Test Notification Channels</h1>
");

    // Apprise card
    write!(
        html,
        r##"  <div class="card">
    <div class="section-title">Apprise Push Notifications</div>
    <p class="hint">{apprise_icon} Status: {apprise_status}<br>
      {apprise_hint}
    </p>
    <form hx-post="/admin/notifications/test/apprise" hx-target="#apprise-result" hx-swap="innerHTML">
      <button type="submit" class="btn {apprise_btn}" title="{apprise_title}" {apprise_disabled}>Send Test Apprise Notification</button>
    </form>
    <div id="apprise-result"></div>
  </div>
"##,
    )
    .unwrap_or_default();

    // BirdWeather card
    write!(
        html,
        r##"  <div class="card">
    <div class="section-title">BirdWeather Station Ping</div>
    <p class="hint">{bw_icon} Status: {bw_status}<br>
      {bw_hint}
    </p>
    <form hx-post="/admin/notifications/test/birdweather" hx-target="#birdweather-result" hx-swap="innerHTML">
      <button type="submit" class="btn {bw_btn}" title="{bw_title}" {bw_disabled}>Ping BirdWeather API</button>
    </form>
    <div id="birdweather-result"></div>
  </div>
"##,
    )
    .unwrap_or_default();

    // Test all card
    html.push_str(
        r##"  <div class="card">
    <div class="section-title">Test All Channels</div>
    <form hx-post="/admin/notifications/test" hx-target="#all-result" hx-swap="innerHTML">
      <button type="submit" class="btn btn-primary">Test All Configured Channels</button>
    </form>
    <div id="all-result"></div>
  </div>"##,
    );

    html
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

async fn test_apprise(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let apprise_url = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        get_setting(conn, "apprise_url")
            .ok()
            .filter(|v| !v.is_empty())
    });

    // O-18: toast the test outcome on every branch.
    match apprise_url {
        None => (
            StatusCode::OK,
            toast::with(
                Html(result_html(false, "Apprise URL not configured")),
                Toast::warn("Apprise URL not configured."),
            ),
        ),
        Some(url) => {
            let res = send_apprise_test(&url).await;
            match res {
                Ok(()) => (
                    StatusCode::OK,
                    toast::with(
                        Html(result_html(true, "Test notification sent via Apprise ✓")),
                        Toast::success("Test sent to Apprise."),
                    ),
                ),
                Err(e) => (
                    StatusCode::OK,
                    toast::with(
                        Html(result_html(false, &e)),
                        Toast::error(format!("Apprise: {e}")),
                    ),
                ),
            }
        }
    }
}

async fn test_birdweather(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let token = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        get_setting(conn, "birdweather_token")
            .ok()
            .filter(|v| !v.is_empty())
    });

    // O-18: toast the test outcome on every branch.
    match token {
        None => (
            StatusCode::OK,
            toast::with(
                Html(result_html(false, "BirdWeather token not configured")),
                Toast::warn("BirdWeather token not configured."),
            ),
        ),
        Some(tok) => {
            let res = ping_birdweather(&tok).await;
            match res {
                Ok(msg) => (
                    StatusCode::OK,
                    toast::with(
                        Html(result_html(true, &msg)),
                        Toast::success(format!("BirdWeather: {msg}")),
                    ),
                ),
                Err(e) => (
                    StatusCode::OK,
                    toast::with(
                        Html(result_html(false, &e)),
                        Toast::error(format!("BirdWeather: {e}")),
                    ),
                ),
            }
        }
    }
}

async fn test_all(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let (apprise_url, bw_token) = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        let a = get_setting(conn, "apprise_url")
            .ok()
            .filter(|v| !v.is_empty());
        let b = get_setting(conn, "birdweather_token")
            .ok()
            .filter(|v| !v.is_empty());
        (a, b)
    });

    let mut lines: Vec<String> = Vec::new();

    if let Some(url) = apprise_url {
        match send_apprise_test(&url).await {
            Ok(()) => lines.push("&#x2705; Apprise: test notification sent".to_string()),
            Err(e) => lines.push(format!("&#x274c; Apprise: {e}")),
        }
    } else {
        lines.push("&#x26a0;&#xfe0f; Apprise: not configured (skipped)".to_string());
    }

    if let Some(tok) = bw_token {
        match ping_birdweather(&tok).await {
            Ok(msg) => lines.push(format!("&#x2705; BirdWeather: {msg}")),
            Err(e) => lines.push(format!("&#x274c; BirdWeather: {e}")),
        }
    } else {
        lines.push("&#x26a0;&#xfe0f; BirdWeather: not configured (skipped)".to_string());
    }

    let body = lines.join("<br>");
    let ok = lines.iter().all(|r| !r.contains("274c"));
    // O-18: aggregate-test toast summarises the run.
    let summary = if ok {
        Toast::success("All configured channels passed.")
    } else {
        Toast::error("One or more channels failed — see results.")
    };
    (
        StatusCode::OK,
        toast::with(Html(result_html(ok, &body)), summary),
    )
}

// ---------------------------------------------------------------------------
// Integration helpers
// ---------------------------------------------------------------------------

async fn send_apprise_test(apprise_url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("{}/notify", apprise_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "title": "BirdNet-Behavior Test",
        "body": "This is a test notification from BirdNet-Behavior!",
        "type": "info"
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("Apprise returned HTTP {}", resp.status()))
    }
}

async fn ping_birdweather(token: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://app.birdweather.com/api/v1/stations?token={token}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;

    if resp.status().is_success() {
        Ok("BirdWeather API reachable -- token appears valid".to_string())
    } else if resp.status().as_u16() == 401 {
        Err("BirdWeather returned 401 -- check your token".to_string())
    } else {
        Err(format!("BirdWeather returned HTTP {}", resp.status()))
    }
}

fn result_html(ok: bool, msg: &str) -> String {
    let variant = if ok { "ok" } else { "err" };
    let icon = if ok { "&#x2713;" } else { "&#x2717;" };
    format!(r#"<div class="result-banner {variant}">{icon} {msg}</div>"#)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_has_no_inline_style_attributes() {
        // P3-3 (O-25): no inline `style=` attributes — page-specific styling
        // lives in this page's own <style> block, and the result banner uses
        // enumerable `.result-banner` variants instead of computed inline colours.
        // Check the page body, not `render_test_page` (the shared shell adds
        // `data-confirm-style` attributes a naive `style="` check would flag).
        assert!(!test_notifications_body(true, true).contains("style=\""));
        assert!(!test_notifications_body(false, false).contains("style=\""));
        assert!(!result_html(true, "ok").contains("style=\""));
        assert!(!result_html(false, "err").contains("style=\""));
    }

    #[test]
    fn disabled_buttons_explain_why() {
        // WEB-3: an unconfigured channel's test button is disabled; the page must
        // say *why* — both as a tooltip on the button and as visible hint copy,
        // pointing at the exact Settings field to fill in. (Check the `disabled>`
        // attribute, not the bare word "disabled" — the `.btn-disabled` CSS class
        // is always present in the page's <style> block.)
        let body = test_notifications_body(false, false);
        assert!(body.contains("disabled>")); // buttons carry the disabled attribute
        // Tooltips name the reason.
        assert!(body.contains("title=\"Disabled — set the Apprise URL in Settings first\""));
        assert!(body.contains("title=\"Disabled — set the BirdWeather token in Settings first\""));
        // Visible hint copy names the reason (reliable even where disabled
        // buttons suppress hover tooltips).
        assert!(body.contains("Disabled until you set the Apprise URL"));
        assert!(body.contains("Disabled until you set the BirdWeather token"));
    }

    #[test]
    fn enabled_buttons_are_not_disabled_and_have_action_tooltips() {
        let body = test_notifications_body(true, true);
        // No channel button carries the disabled attribute when both are configured.
        assert!(!body.contains("disabled>"));
        assert!(body.contains("title=\"Send a test notification through Apprise\""));
        assert!(body.contains("title=\"Ping the BirdWeather API to verify your token\""));
    }
}
