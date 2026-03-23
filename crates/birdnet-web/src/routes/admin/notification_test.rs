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

use crate::state::AppState;

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
    let (apprise_configured, bw_configured) = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        let apprise = get_setting(conn, "apprise_url")
            .ok()
            .is_some_and(|v| !v.is_empty());
        let bw = get_setting(conn, "birdweather_token")
            .ok()
            .is_some_and(|v| !v.is_empty());
        (apprise, bw)
    });

    Html(render_test_page(apprise_configured, bw_configured))
}

fn render_test_page(apprise_ok: bool, bw_ok: bool) -> String {
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

    let mut html = String::with_capacity(4096);
    html.push_str(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Test Notifications — BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <link rel="stylesheet" href="/static/style.css">
    <style>
      body { background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }
      .container { max-width:800px; margin:0 auto; padding:2rem 1rem; }
      nav a { color:#94a3b8; text-decoration:none; margin-right:1.5rem; }
      nav a.active, nav a:hover { color:#38bdf8; }
      .card { background:#1e293b; border:1px solid #334155; border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }
      .section-title { font-size:1.1rem; font-weight:600; color:#38bdf8; margin-bottom:1rem; border-bottom:1px solid #334155; padding-bottom:0.5rem; }
      .btn { padding:0.5rem 1.5rem; border-radius:0.375rem; border:none; cursor:pointer; font-weight:600; font-size:0.875rem; }
      .btn-primary { background:#0ea5e9; color:#fff; }
      .btn-disabled { background:#334155; color:#64748b; cursor:not-allowed; }
      .hint { font-size:0.75rem; color:#64748b; }
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin">Admin</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/notifications/test" class="active">Test Notifications</a>
  </nav>
  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:1.5rem;color:#f1f5f9;">Test Notification Channels</h1>
"#);

    // Apprise card
    write!(
        html,
        r##"  <div class="card">
    <div class="section-title">Apprise Push Notifications</div>
    <p class="hint" style="margin-bottom:1rem;">{apprise_icon} Status: {apprise_status}<br>
      Configure the Apprise URL in <a href="/admin/settings" style="color:#38bdf8;">Settings</a>.
    </p>
    <form hx-post="/admin/notifications/test/apprise" hx-target="#apprise-result" hx-swap="innerHTML">
      <button type="submit" class="btn {apprise_btn}" {apprise_disabled}>Send Test Apprise Notification</button>
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
    <p class="hint" style="margin-bottom:1rem;">{bw_icon} Status: {bw_status}<br>
      Configure the BirdWeather token in <a href="/admin/settings" style="color:#38bdf8;">Settings</a>.
    </p>
    <form hx-post="/admin/notifications/test/birdweather" hx-target="#birdweather-result" hx-swap="innerHTML">
      <button type="submit" class="btn {bw_btn}" {bw_disabled}>Ping BirdWeather API</button>
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
  </div>
</div>
</body>
</html>"##,
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

    match apprise_url {
        None => (
            StatusCode::OK,
            Html(result_html(false, "Apprise URL not configured")),
        ),
        Some(url) => {
            let res = send_apprise_test(&url).await;
            match res {
                Ok(()) => (
                    StatusCode::OK,
                    Html(result_html(true, "Test notification sent via Apprise ✓")),
                ),
                Err(e) => (StatusCode::OK, Html(result_html(false, &e))),
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

    match token {
        None => (
            StatusCode::OK,
            Html(result_html(false, "BirdWeather token not configured")),
        ),
        Some(tok) => {
            let res = ping_birdweather(&tok).await;
            match res {
                Ok(msg) => (StatusCode::OK, Html(result_html(true, &msg))),
                Err(e) => (StatusCode::OK, Html(result_html(false, &e))),
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
    (StatusCode::OK, Html(result_html(ok, &body)))
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
    let bg = if ok { "#064e3b" } else { "#450a0a" };
    let border = if ok { "#065f46" } else { "#7f1d1d" };
    let color = if ok { "#6ee7b7" } else { "#fca5a5" };
    let icon = if ok { "&#x2713;" } else { "&#x2717;" };
    format!(
        r#"<div style="background:{bg};border:1px solid {border};border-radius:0.375rem;padding:0.75rem;margin-top:0.75rem;color:{color};">{icon} {msg}</div>"#,
    )
}
