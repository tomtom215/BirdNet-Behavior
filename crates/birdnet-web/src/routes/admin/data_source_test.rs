//! "Test connection" for the integrations that are not notification channels
//! (`P-15`): MQTT, eBird and the weather provider.
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | POST   | /admin/notifications/test/mqtt    | Publish a test message to the broker |
//! | POST   | /admin/notifications/test/ebird   | Ask eBird a real question with the key |
//! | POST   | /admin/notifications/test/weather | Fetch from the weather provider |
//!
//! tphakala/birdnet-go has a button for each (`/api/v2/integrations/{mqtt,ebird,
//! weather}/test`); `POST /admin/notifications/test` ("Test all") already ran
//! MQTT, but there was no way to ask the broker alone, and nothing at all for
//! eBird or weather.
//!
//! # The probe is the binary's, the answer is the integration's
//!
//! All three are configured from the command line, the environment and the
//! config file, which the web crate cannot see (see [`crate::notification_probes`]).
//! The binary hands over a probe for each it can build, and what the probe
//! returns is shown **as it came**: the broker's refusal, eBird's HTTP status,
//! the connect error. A test that answered "check your configuration" when the
//! real answer was "connection refused" or "eBird refused the key (HTTP 403)"
//! would send an operator to the wrong place, which is worse than no button.
//!
//! # An answer, always
//!
//! htmx discards a 4xx/5xx body, so every outcome — including a failure — is a
//! `200` carrying the result banner and a toast.

use std::fmt::Write as _;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::post;

use crate::notification_probes::Probe;
use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

/// The longest a probe may take before the page says it did not answer. The
/// probes bound their own network calls (5 s to 20 s); this is the backstop for
/// one that does not.
const PROBE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);

/// Which integration a card is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Mqtt,
    Ebird,
    Weather,
}

impl Source {
    const ALL: [Self; 3] = [Self::Mqtt, Self::Ebird, Self::Weather];

    const fn title(self) -> &'static str {
        match self {
            Self::Mqtt => "MQTT broker",
            Self::Ebird => "eBird",
            Self::Weather => "Weather",
        }
    }

    const fn slug(self) -> &'static str {
        match self {
            Self::Mqtt => "mqtt",
            Self::Ebird => "ebird",
            Self::Weather => "weather",
        }
    }

    const fn button(self) -> &'static str {
        match self {
            Self::Mqtt => "Publish a test message",
            Self::Ebird => "Test the eBird key",
            Self::Weather => "Fetch weather now",
        }
    }

    /// What the button does, and what a pass means.
    const fn what(self) -> &'static str {
        match self {
            Self::Mqtt => {
                "Connects to the broker with the station's settings (TLS and client certificate \
                 included) and publishes one message to <code>&lt;prefix&gt;/test</code>."
            }
            Self::Ebird => {
                "Asks eBird for the recent observations around the station with the configured \
                 key, the same request the station makes every few hours."
            }
            Self::Weather => {
                "Asks the configured weather provider for the station's hourly data, the same \
                 request the station makes every half hour."
            }
        }
    }

    /// How to turn it on, shown when it is not.
    const fn how_to_enable(self) -> &'static str {
        match self {
            Self::Mqtt => {
                "Set <code>MQTT_HOST</code> (and <code>MQTT_TLS</code> for an encrypted broker) \
                 in the config file, or <code>BIRDNET_MQTT_HOST</code> in the environment, and restart."
            }
            Self::Ebird => {
                "Set <code>EBIRD_API_KEY</code> (a key from ebird.org/api/keygen) in the config \
                 file, and restart."
            }
            Self::Weather => {
                "Set <code>BNB_WEATHER_ENABLED=1</code> in the environment (it is off by \
                 default, so the station does not phone out unasked) and restart."
            }
        }
    }

    fn probe(self, state: &AppState) -> Option<Probe> {
        let p = state.notification_probes();
        match self {
            Self::Mqtt => p.mqtt,
            Self::Ebird => p.ebird,
            Self::Weather => p.weather,
        }
    }
}

/// Mount the three test routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/notifications/test/mqtt", post(test_mqtt))
        .route("/admin/notifications/test/ebird", post(test_ebird))
        .route("/admin/notifications/test/weather", post(test_weather))
}

/// The three cards, to sit under the notification-channel cards on the test
/// page and the Station Alerts tab. Uses that page's own classes (`card`,
/// `section-title`, `hint`, `btn`, `result-banner`).
#[must_use]
pub(crate) fn cards(state: &AppState) -> String {
    let mut html = String::with_capacity(2048);
    for source in Source::ALL {
        let configured = source.probe(state).is_some();
        let (status, button_class, disabled) = if configured {
            (
                "Configured. A pass here means the station can really do this.",
                "btn-primary",
                "",
            )
        } else {
            ("Not configured.", "btn-disabled", " disabled")
        };
        let hint = if configured {
            source.what().to_owned()
        } else {
            format!("Disabled until it is set up. {}", source.how_to_enable())
        };
        let slug = source.slug();
        let title = source.title();
        let button = source.button();
        let _ = write!(
            html,
            r##"  <div class="card">
    <div class="section-title">{title}</div>
    <p class="hint">{status}<br>{hint}</p>
    <form hx-post="/admin/notifications/test/{slug}" hx-target="#{slug}-result" hx-swap="innerHTML">
      <button type="submit" class="btn {button_class}"{disabled}>{button}</button>
    </form>
    <div id="{slug}-result"></div>
  </div>
"##
        );
    }
    html
}

fn banner(ok: bool, msg: &str) -> String {
    let (variant, icon) = if ok {
        ("ok", "&#x2713;")
    } else {
        ("err", "&#x2717;")
    };
    format!(r#"<div class="result-banner {variant}">{icon} {msg}</div>"#)
}

/// Run one probe and answer with what it said.
async fn run(
    state: &AppState,
    source: Source,
    deadline: std::time::Duration,
) -> (StatusCode, Html<String>) {
    let title = source.title();
    let Some(probe) = source.probe(state) else {
        let msg =
            format!("{title} is not configured on this station, so there is nothing to test.");
        return (
            StatusCode::OK,
            toast::with(
                Html(banner(
                    false,
                    &format!("{} {}", escape_html(&msg), source.how_to_enable()),
                )),
                Toast::warn(msg),
            ),
        );
    };
    let outcome = tokio::time::timeout(deadline, probe())
        .await
        .unwrap_or_else(|_| Err(format!("no answer within {} seconds", deadline.as_secs())));
    match outcome {
        Ok(msg) => (
            StatusCode::OK,
            toast::with(
                Html(banner(true, &escape_html(&msg))),
                Toast::success(format!("{title}: {msg}")),
            ),
        ),
        Err(e) => (
            StatusCode::OK,
            toast::with(
                Html(banner(false, &escape_html(&e))),
                Toast::error(format!("{title}: {e}")),
            ),
        ),
    }
}

async fn test_mqtt(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    run(&state, Source::Mqtt, PROBE_DEADLINE).await
}

async fn test_ebird(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    run(&state, Source::Ebird, PROBE_DEADLINE).await
}

async fn test_weather(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    run(&state, Source::Weather, PROBE_DEADLINE).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification_probes::NotificationProbes;
    use tower::ServiceExt as _;

    fn station() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    async fn post(state: &AppState, path: &str) -> (u16, String) {
        let resp = router()
            .with_state(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn with_probes(ebird: Option<Probe>, weather: Option<Probe>, mqtt: Option<Probe>) -> AppState {
        let state = station();
        state.set_notification_probes(NotificationProbes {
            mqtt,
            email: None,
            ebird,
            weather,
        });
        state
    }

    #[tokio::test]
    async fn a_failure_is_shown_as_it_came_and_is_a_200() {
        // htmx discards a 4xx/5xx body, and a reassuring paraphrase would send
        // the operator to the wrong place. The real reason is in the banner and
        // in the toast, in full.
        let reason = "eBird refused the API key (HTTP 403)";
        let state = with_probes(
            Some(NotificationProbes::fixed(Err(reason.into()))),
            None,
            None,
        );
        let (status, body) = post(&state, "/admin/notifications/test/ebird").await;
        assert_eq!(status, 200);
        assert!(body.contains("result-banner err"), "{body}");
        assert!(body.contains(reason), "the reason was not shown: {body}");
        assert!(body.contains("toast"), "no toast: {body}");
        assert!(!body.contains("result-banner ok"), "{body}");
    }

    #[tokio::test]
    async fn a_pass_is_shown_with_what_it_found() {
        let state = with_probes(
            None,
            Some(NotificationProbes::fixed(Ok(
                "Open-Meteo answered with 72 hourly rows".into(),
            ))),
            None,
        );
        let (status, body) = post(&state, "/admin/notifications/test/weather").await;
        assert_eq!(status, 200);
        assert!(body.contains("result-banner ok"), "{body}");
        assert!(body.contains("72 hourly rows"), "{body}");
    }

    #[tokio::test]
    async fn each_route_runs_its_own_probe_and_no_other() {
        let state = with_probes(
            Some(NotificationProbes::fixed(Ok("EBIRD-PROBE".into()))),
            Some(NotificationProbes::fixed(Ok("WEATHER-PROBE".into()))),
            Some(NotificationProbes::fixed(Ok("MQTT-PROBE".into()))),
        );
        for (path, wants, others) in [
            ("mqtt", "MQTT-PROBE", ["EBIRD-PROBE", "WEATHER-PROBE"]),
            ("ebird", "EBIRD-PROBE", ["MQTT-PROBE", "WEATHER-PROBE"]),
            ("weather", "WEATHER-PROBE", ["MQTT-PROBE", "EBIRD-PROBE"]),
        ] {
            let (_, body) = post(&state, &format!("/admin/notifications/test/{path}")).await;
            assert!(body.contains(wants), "{path}: {body}");
            for other in others {
                assert!(!body.contains(other), "{path} ran the wrong probe: {body}");
            }
        }
    }

    #[tokio::test]
    async fn an_unconfigured_integration_says_so_rather_than_passing() {
        let state = station();
        for path in ["mqtt", "ebird", "weather"] {
            let (status, body) = post(&state, &format!("/admin/notifications/test/{path}")).await;
            assert_eq!(status, 200, "{path}");
            assert!(body.contains("not configured"), "{path}: {body}");
            assert!(!body.contains("result-banner ok"), "{path} passed: {body}");
        }
    }

    #[tokio::test]
    async fn a_probe_that_never_answers_is_reported_not_waited_for_forever() {
        let slow: Probe = std::sync::Arc::new(|| {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                Ok("too late".to_owned())
            })
        });
        let state = with_probes(Some(slow), None, None);
        let (status, body) = run(&state, Source::Ebird, std::time::Duration::from_millis(50)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.0.contains("no answer within"), "{}", body.0);
        assert!(!body.0.contains("too late"), "{}", body.0);
    }

    #[test]
    fn the_cards_enable_the_button_only_when_there_is_a_probe() {
        let configured = with_probes(
            Some(NotificationProbes::fixed(Ok(String::new()))),
            None,
            None,
        );
        let html = cards(&configured);
        // eBird's button is live; MQTT's and weather's are disabled and say how
        // to turn them on.
        let ebird = html.split("Test the eBird key").next().unwrap();
        assert!(
            !ebird.rsplit("<button").next().unwrap().contains("disabled"),
            "{html}"
        );
        assert!(
            html.contains("btn-disabled\" disabled>Publish a test message"),
            "{html}"
        );
        assert!(html.contains("BNB_WEATHER_ENABLED=1"), "{html}");
        assert!(html.contains("MQTT_HOST"), "{html}");
        assert!(!html.contains("style=\""), "{html}");
    }
}
