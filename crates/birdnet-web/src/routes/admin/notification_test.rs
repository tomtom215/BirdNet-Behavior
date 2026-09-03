//! Notification channel testing routes.
//!
//! Provides a UI to send test messages via each configured notification channel.
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /admin/notifications/test | Test page |
//! | POST   | /admin/notifications/test/apprise | Send push test |
//! | POST   | /admin/notifications/test/birdweather | Send BirdWeather test ping |
//! | POST   | /admin/notifications/test | Test all channels |
//!
//! # The push test sends what an alert sends (`OB-9`)
//!
//! It did not. It built a fresh `reqwest::Client` and `POST`ed
//! `{apprise_url}/notify` itself, so it exercised neither the native routes
//! delivered in-process, nor the `apprise` CLI fallback, nor the per-destination
//! circuit breaker, nor the rate limiter — none of the machinery that decides
//! whether an alert about the station leaves the box. And it keyed its button
//! off the `apprise_url` **setting**, so a station configured only with native
//! notification URLs — the configuration most stations have — saw "Not
//! configured" and a disabled button while its alerts worked fine.
//!
//! It now locks [`crate::notifier::Notifier::client`] and calls
//! `send_operational_alert` on it: the same handle, the same call and the same
//! guards as `announce::flush`, which is what every deadman, station-health and
//! stream-fault alert goes through. Anything that would drop a real alert
//! silently now shows up here, by name, when an operator presses the button.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};
use std::fmt::Write as _;

use birdnet_db::settings::ensure_settings_table;
use birdnet_db::settings::get as get_setting;
use birdnet_integrations::apprise::NotifyType;

use crate::notifier::Notifier;
use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

/// What the operator is told when the station resolved no destination.
///
/// One string, used by the button's own refusal and by "Test all", so the two
/// cannot drift into disagreeing about what "not configured" means.
const NOWHERE_TO_SEND: &str = "no notification destination is configured on this station";

/// Title of the message the push test sends.
const TEST_TITLE: &str = "BirdNet-Behavior test notification";

/// Body of the message the push test sends.
///
/// Says which button produced it, because it arrives on the same channel as
/// the alerts and an operator finding it later should not have to wonder
/// whether their station was in trouble at 3 a.m.
const TEST_BODY: &str = "This is a test, sent from Admin › Notifications › Test \
                         through the same path this station's alerts take. \
                         Nothing is wrong.";

/// Mount the notification channel test routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/notifications/test", get(test_page).post(test_all))
        .route(
            "/admin/notifications/test/apprise",
            axum::routing::post(test_push),
        )
        .route(
            "/admin/notifications/test/birdweather",
            axum::routing::post(test_birdweather),
        )
}

// ---------------------------------------------------------------------------
// What the push card renders from
// ---------------------------------------------------------------------------

/// The destinations the *running* station resolved for push notifications.
///
/// Not what is typed into the settings form: a value saved there takes effect
/// at the next restart, and the difference between the two is exactly the
/// wrong answer `OB-9` describes — a station delivering happily over native
/// routes being told it was "Not configured".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PushChannel {
    /// Credential-free labels for the natively delivered destinations.
    destinations: Vec<String>,
    /// The `apprise` CLI will be invoked for a configured config file.
    apprise_cli: bool,
    /// An Apprise API server URL was resolved.
    apprise_server: bool,
}

impl PushChannel {
    /// What the station's notifier resolved, or nothing if it has none.
    fn of(state: &AppState) -> Self {
        state.notifier().map_or_else(Self::default, Self::from)
    }

    /// Whether anything at all resolved — the predicate that enables the button.
    const fn configured(&self) -> bool {
        !self.destinations.is_empty() || self.apprise_cli || self.apprise_server
    }

    /// One line per resolved destination, for the operator to read.
    fn lines(&self) -> Vec<String> {
        let mut out: Vec<String> = self.destinations.clone();
        if self.apprise_server {
            out.push("Apprise API server".to_owned());
        }
        if self.apprise_cli {
            out.push("apprise CLI (config file)".to_owned());
        }
        out
    }
}

impl From<&Notifier> for PushChannel {
    fn from(n: &Notifier) -> Self {
        Self {
            destinations: n.destinations().to_vec(),
            apprise_cli: n.apprise_cli(),
            apprise_server: n.apprise_server(),
        }
    }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

async fn test_page(State(state): State<AppState>) -> Html<String> {
    Html(render_test_page_for(&state))
}

fn render_test_page_for(state: &AppState) -> String {
    render_test_page(&PushChannel::of(state), birdweather_configured(state))
}

/// Whether the BirdWeather channel has a token set.
///
/// Still a settings read, and correctly so: `ping_birdweather` reads the token
/// at request time, so the settings row *is* what that test uses.
fn birdweather_configured(state: &AppState) -> bool {
    state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        get_setting(conn, "birdweather_token").is_ok_and(|v| !v.is_empty())
    })
}

/// Render the channels Send-test body (no document shell).
///
/// Shared with the Station **Alerts** tab
/// (`crate::routes::pages::homes::station_tabs`), which renders the
/// confirm-before-you-rely-on-it test UI in the main shell.
pub(crate) fn channels_test_body(state: &AppState) -> String {
    test_notifications_body(&PushChannel::of(state), birdweather_configured(state))
}

fn render_test_page(push: &PushChannel, bw_ok: bool) -> String {
    crate::routes::admin::admin_subpage_shell(
        "Test notifications",
        "notifications",
        "Test",
        &test_notifications_body(push, bw_ok),
    )
}

/// Page-specific body (scoped `<style>` + cards). The shared shell supplies the
/// chrome, the nav (with the Notifications tab active), and the
/// `Admin › Notifications › Test` breadcrumb.
fn test_notifications_body(push: &PushChannel, bw_ok: bool) -> String {
    let push_ok = push.configured();
    let push_status = if push_ok {
        let lines = push.lines();
        let listed = lines
            .iter()
            .map(|l| escape_html(l))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} destination(s) — {listed}", lines.len())
    } else {
        "No destination resolved".to_owned()
    };
    let bw_status = if bw_ok {
        "Configured"
    } else {
        "Not configured"
    };
    let push_icon = if push_ok { "✅" } else { "⚠️" };
    let bw_icon = if bw_ok { "✅" } else { "⚠️" };
    let push_disabled = if push_ok { "" } else { "disabled" };
    let bw_disabled = if bw_ok { "" } else { "disabled" };
    let push_btn = if push_ok {
        "btn-primary"
    } else {
        "btn-disabled"
    };
    let bw_btn = if bw_ok { "btn-primary" } else { "btn-disabled" };

    // WEB-3: a disabled test button is a dead end unless we say *why*. Give each
    // button a tooltip AND spell the reason out in the visible hint, because a
    // native `disabled` button suppresses pointer events in most browsers, so
    // its `title` won't fire on hover — the hint is the reliable reason surface.
    let push_title = if push_ok {
        "Send a test through the same path this station's alerts take"
    } else {
        "Disabled — this station resolved no notification destination"
    };
    let bw_title = if bw_ok {
        "Ping the BirdWeather API to verify your token"
    } else {
        "Disabled — set the BirdWeather token in Settings first"
    };
    // The reason for the disabled state names all three ways a destination can
    // resolve, and says the restart part out loud: the notifier is built at
    // startup, so a URL saved in Settings a moment ago is not one this station
    // has resolved yet.
    let push_hint = if push_ok {
        "Configured — this test goes through the same client, circuit breaker \
         and rate limiter as an alert about the station, so a result here is a \
         result for your alerts."
    } else {
        r#"Disabled until this station resolves a destination: set Notification URLs, an Apprise server URL, or an Apprise config file in <a href="/admin/settings">Settings</a> and restart."#
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

    // Push card
    write!(
        html,
        r##"  <div class="card">
    <div class="section-title">Push Notifications</div>
    <p class="hint">{push_icon} Status: {push_status}<br>
      {push_hint}
    </p>
    <form hx-post="/admin/notifications/test/apprise" hx-target="#apprise-result" hx-swap="innerHTML">
      <button type="submit" class="btn {push_btn}" title="{push_title}" {push_disabled}>Send Test Push Notification</button>
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

/// What the push test did, as one sentence.
///
/// `Err` carries the notifier's own message, which distinguishes "every
/// destination was skipped (1 with an open circuit, 0 rate-limited)" from a
/// delivery that was tried and failed — the distinction an operator needs and
/// the one a fresh-client test could not make.
async fn push_test(state: &AppState) -> Result<String, String> {
    let Some(notifier) = state.notifier() else {
        return Err(NOWHERE_TO_SEND.to_owned());
    };
    // The same predicate the button's enabled state uses, so a station the
    // page calls configured is one the handler will actually try.
    if !PushChannel::from(notifier).configured() {
        return Err(NOWHERE_TO_SEND.to_owned());
    }
    notifier
        .client()
        .lock()
        .await
        .send_operational_alert(TEST_TITLE, TEST_BODY, NotifyType::Info)
        .await
        .map(|()| {
            let n = notifier.destinations().len();
            format!("Test notification sent through the station's notifier ({n} native destination(s)) \u{2713}")
        })
        .map_err(|e| e.to_string())
}

async fn test_push(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    // O-18: toast the test outcome on every branch.
    match push_test(&state).await {
        Ok(msg) => (
            StatusCode::OK,
            toast::with(
                Html(result_html(true, &escape_html(&msg))),
                Toast::success("Test push notification sent."),
            ),
        ),
        Err(e) => (
            StatusCode::OK,
            toast::with(
                Html(result_html(false, &escape_html(&e))),
                Toast::error(format!("Push notification: {e}")),
            ),
        ),
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
                        Html(result_html(true, &escape_html(&msg))),
                        Toast::success(format!("BirdWeather: {msg}")),
                    ),
                ),
                Err(e) => (
                    StatusCode::OK,
                    toast::with(
                        Html(result_html(false, &escape_html(&e))),
                        Toast::error(format!("BirdWeather: {e}")),
                    ),
                ),
            }
        }
    }
}

async fn test_all(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let mut lines: Vec<String> = Vec::new();

    if PushChannel::of(&state).configured() {
        match push_test(&state).await {
            Ok(msg) => lines.push(format!("&#x2705; Push: {}", escape_html(&msg))),
            Err(e) => lines.push(format!("&#x274c; Push: {}", escape_html(&e))),
        }
    } else {
        lines.push("&#x26a0;&#xfe0f; Push: no destination resolved (skipped)".to_owned());
    }

    let bw_token = state.with_db(|conn| {
        ensure_settings_table(conn).ok();
        get_setting(conn, "birdweather_token")
            .ok()
            .filter(|v| !v.is_empty())
    });
    if let Some(tok) = bw_token {
        match ping_birdweather(&tok).await {
            Ok(msg) => lines.push(format!("&#x2705; BirdWeather: {}", escape_html(&msg))),
            Err(e) => lines.push(format!("&#x274c; BirdWeather: {}", escape_html(&e))),
        }
    } else {
        lines.push("&#x26a0;&#xfe0f; BirdWeather: not configured (skipped)".to_owned());
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

    /// A station with one native route resolved and no Apprise server.
    fn native_only() -> PushChannel {
        PushChannel {
            destinations: vec!["ntfy https://ntfy.sh (1 topic(s))".to_owned()],
            apprise_cli: false,
            apprise_server: false,
        }
    }

    /// A station with an Apprise API server and nothing else.
    fn server_only() -> PushChannel {
        PushChannel {
            destinations: Vec::new(),
            apprise_cli: false,
            apprise_server: true,
        }
    }

    #[test]
    fn test_page_has_no_inline_style_attributes() {
        // P3-3 (O-25): no inline `style=` attributes — page-specific styling
        // lives in this page's own <style> block, and the result banner uses
        // enumerable `.result-banner` variants instead of computed inline colours.
        // Check the page body, not `render_test_page` (the shared shell adds
        // `data-confirm-style` attributes a naive `style="` check would flag).
        assert!(!test_notifications_body(&native_only(), true).contains("style=\""));
        assert!(!test_notifications_body(&PushChannel::default(), false).contains("style=\""));
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
        let body = test_notifications_body(&PushChannel::default(), false);
        assert!(body.contains("disabled>")); // buttons carry the disabled attribute
        // Tooltips name the reason.
        assert!(
            body.contains("title=\"Disabled — this station resolved no notification destination\"")
        );
        assert!(body.contains("title=\"Disabled — set the BirdWeather token in Settings first\""));
        // Visible hint copy names the reason (reliable even where disabled
        // buttons suppress hover tooltips).
        assert!(body.contains("Disabled until this station resolves a destination"));
        assert!(body.contains("Disabled until you set the BirdWeather token"));
    }

    #[test]
    fn enabled_buttons_are_not_disabled_and_have_action_tooltips() {
        let body = test_notifications_body(&native_only(), true);
        // No channel button carries the disabled attribute when both are configured.
        assert!(!body.contains("disabled>"));
        assert!(
            body.contains("title=\"Send a test through the same path this station's alerts take\"")
        );
        assert!(body.contains("title=\"Ping the BirdWeather API to verify your token\""));
    }

    #[test]
    fn a_native_route_is_a_configured_push_channel() {
        // OB-9's headline defect: the button was enabled only for an Apprise
        // *API server*, so a station delivering over `ntfy://` — the
        // configuration most stations have — saw a dead button.
        assert!(native_only().configured());
        assert!(server_only().configured());
        assert!(
            PushChannel {
                destinations: Vec::new(),
                apprise_cli: true,
                apprise_server: false,
            }
            .configured()
        );
        // The counterpart that stops the fix being "always enabled".
        assert!(!PushChannel::default().configured());
    }

    #[test]
    fn the_page_names_the_destinations_it_resolved() {
        let body = test_notifications_body(&native_only(), false);
        assert!(
            body.contains("ntfy https://ntfy.sh (1 topic(s))"),
            "the resolved destination must be named: {body}"
        );
        assert!(body.contains("1 destination(s)"));
    }

    #[test]
    fn destination_labels_are_escaped() {
        // `label_for` builds labels from parsed URLs, so this is defence in
        // depth rather than a live hole — but the label reaches the page as
        // markup, and every other operator-supplied string on this page goes
        // through the one escaper.
        let hostile = PushChannel {
            destinations: vec!["json http://<script>alert(1)</script>".to_owned()],
            apprise_cli: false,
            apprise_server: false,
        };
        let body = test_notifications_body(&hostile, false);
        assert!(!body.contains("<script>"), "{body}");
        assert!(body.contains("&lt;script&gt;"));
    }
}
