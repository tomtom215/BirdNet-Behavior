//! The channel probes "Test all channels" runs (DD-24), built from the
//! integrations this binary configured.

use std::sync::Arc;

use birdnet_core::config::Config;
use birdnet_integrations::email::DetectionEmail;
use birdnet_integrations::mqtt::MqttClient;
use birdnet_web::notification_probes::{NotificationProbes, Probe};

use super::email::EmailHandle;

/// Probes for whichever of MQTT and email are configured.
#[must_use]
pub fn notification_probes(
    mqtt: Option<MqttClient>,
    email: Option<EmailHandle>,
) -> NotificationProbes {
    NotificationProbes {
        mqtt: mqtt.map(mqtt_probe),
        email: email.map(email_probe),
        ebird: None,
        weather: None,
    }
}

/// The eBird and weather probes (`P-15`), for whichever is configured.
///
/// A probe exists when the integration was *asked for*, whether or not the
/// rest of its configuration is usable: an eBird key with no coordinates, or an
/// unknown weather provider, answers with the real reason ("`EBIRD_API_KEY` is
/// set but the station has no coordinates") instead of a button that is greyed
/// out for a reason nothing says.
#[must_use]
pub fn data_source_probes(config: Option<&Config>) -> (Option<Probe>, Option<Probe>) {
    (ebird_probe(config), weather_probe(config))
}

fn ebird_probe(config: Option<&Config>) -> Option<Probe> {
    match super::ebird::resolve_settings(config) {
        Ok(None) => None,
        Ok(Some(settings)) => {
            let base = std::env::var("BNB_EBIRD_BASE_URL")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            Some(ebird_probe_for(settings, base))
        }
        Err(e) => Some(failed_probe(e.to_string())),
    }
}

/// A probe that answers `reason`, for an integration that was asked for and
/// cannot start.
fn failed_probe(reason: String) -> Probe {
    Arc::new(move || {
        let reason = reason.clone();
        Box::pin(async move { Err(reason) })
    })
}

/// Ask eBird the question the poll asks, with the key the poll uses.
///
/// Not a cheaper "ping": a `HEAD` on the taxonomy answers for the network and
/// not for the key, and the key is what goes wrong. Fetching what the poll
/// fetches means a pass is a pass for the feature.
fn ebird_probe_for(settings: birdnet_integrations::ebird::Settings, base: Option<String>) -> Probe {
    Arc::new(move || {
        let settings = settings.clone();
        let base = base.clone();
        Box::pin(async move {
            let client = birdnet_integrations::ebird::Client::new(settings.api_key.clone(), base)
                .map_err(|e| e.to_string())?;
            let snapshot = client
                .recent(
                    &settings.scope,
                    settings.back_days,
                    birdnet_integrations::ebird::now_unix(),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "eBird accepted the key: {} species reported within {} in the last {} day(s)",
                snapshot.len(),
                settings.scope.label(),
                settings.back_days
            ))
        })
    })
}

fn weather_probe(config: Option<&Config>) -> Option<Probe> {
    if !birdnet_integrations::weather::is_enabled() {
        return None;
    }
    let provider = match super::weather::resolve_provider(config) {
        Ok(p) => p,
        Err(e) => return Some(failed_probe(e.to_string())),
    };
    let (Some(lat), Some(lon)) = (
        super::weather::resolve_lat(config),
        super::weather::resolve_lon(config),
    ) else {
        return Some(failed_probe(
            "the weather poll is on but the station has no coordinates; \
             set LATITUDE and LONGITUDE"
                .to_owned(),
        ));
    };
    let base = std::env::var("BNB_WEATHER_BASE_URL")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    Some(weather_probe_for(provider, base, lat, lon))
}

/// Fetch what the poll fetches, from the provider the poll uses.
fn weather_probe_for(
    provider: birdnet_integrations::weather::Provider,
    base: Option<String>,
    lat: f64,
    lon: f64,
) -> Probe {
    Arc::new(move || {
        let provider = provider.clone();
        let base = base.clone();
        Box::pin(async move {
            let label = provider.label();
            let client = birdnet_integrations::weather::Client::new_for(provider, base)
                .map_err(|e| e.to_string())?;
            let rows = client
                .fetch_hourly(lat, lon)
                .await
                .map_err(|e| e.to_string())?;
            if rows.is_empty() {
                return Err(format!(
                    "{label} answered, but with no hourly data for the station's position"
                ));
            }
            Ok(format!(
                "{label} answered with {} hourly row(s)",
                rows.len()
            ))
        })
    })
}

/// Publish a small JSON payload to `{prefix}/test` on a blocking thread —
/// the client is synchronous — and report the topic it reached.
fn mqtt_probe(client: MqttClient) -> Probe {
    Arc::new(move || {
        let client = client.clone();
        Box::pin(async move {
            let payload = serde_json::json!({
                "test": true,
                "station": "birdnet-behavior",
                "sent_at": super::unix_now_secs(),
            })
            .to_string();
            tokio::task::spawn_blocking(move || client.publish_test(payload.as_bytes()))
                .await
                .map_err(|e| format!("probe task failed: {e}"))?
                .map(|topic| format!("published to {topic}"))
                .map_err(|e| e.to_string())
        })
    })
}

/// Send a test email through the configured notifier, past its cooldown.
///
/// A synthetic detection at confidence 1.0 so the notifier's own threshold
/// admits it; `Ok(false)` from the notifier (suppressed) is reported as a
/// failure, because a test that sent nothing has tested nothing.
fn email_probe(handle: EmailHandle) -> Probe {
    Arc::new(move || {
        let handle = Arc::clone(&handle);
        Box::pin(async move {
            const SPECIES: &str = "Test notification";
            handle.reset_cooldown(SPECIES);
            let now = super::unix_now_secs();
            let civil = birdnet_core::civil::civil_from_unix_secs(i64::try_from(now).unwrap_or(0));
            let message = DetectionEmail {
                common_name: SPECIES.to_owned(),
                scientific_name: "BirdNet-Behavior".to_owned(),
                confidence: 1.0,
                date: format!("{:04}-{:02}-{:02}", civil.year, civil.month, civil.day),
                time: format!("{:02}:{:02}:{:02}", civil.hour, civil.minute, civil.second),
                station_name: None,
                detection_url: None,
            };
            match handle.notify(&message).await {
                Ok(true) => Ok("test email sent".to_owned()),
                Ok(false) => Err("the notifier suppressed the test message".to_owned()),
                Err(e) => Err(e.to_string()),
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::test_support::config_with;

    /// A one-shot HTTP server on loopback answering every request with
    /// `status` and `body`; returns its base URL.
    fn stub(status: &'static str, body: &'static str) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0_u8; 4096];
                let _ = s.read(&mut buf);
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    fn ebird_settings() -> birdnet_integrations::ebird::Settings {
        birdnet_integrations::ebird::resolve(
            Some("test-key"),
            None,
            None,
            None,
            Some(52.5),
            Some(13.4),
        )
        .expect("valid")
        .expect("a key is set")
    }

    const EBIRD_BODY: &str = r#"[
        {"sciName":"Pica pica","comName":"Eurasian Magpie","obsDt":"2026-10-20 08:00","howMany":2},
        {"sciName":"Turdus merula","comName":"Common Blackbird","obsDt":"2026-10-20 08:10"}]"#;

    #[tokio::test]
    async fn the_ebird_probe_passes_with_a_working_key_and_says_what_it_found() {
        let probe = ebird_probe_for(ebird_settings(), Some(stub("200 OK", EBIRD_BODY)));
        let ok = probe().await.expect("a 200 with observations passes");
        assert!(ok.contains("2 species"), "{ok}");
        assert!(ok.contains("accepted the key"), "{ok}");
    }

    #[tokio::test]
    async fn the_ebird_probe_reports_a_refused_key_as_the_keys_fault() {
        // The discrimination: the same stub, the same request, only the status
        // differs. A probe that said "reachable" for either would pass the
        // test above and fail this one.
        for status in ["403 Forbidden", "401 Unauthorized"] {
            let probe = ebird_probe_for(ebird_settings(), Some(stub(status, "{}")));
            let err = probe().await.expect_err("a refused key must fail");
            assert!(err.contains("refused the API key"), "{status}: {err}");
            assert!(err.contains("EBIRD_API_KEY"), "{status}: {err}");
            assert!(
                !err.contains("test-key"),
                "the key reached the message: {err}"
            );
        }
    }

    #[tokio::test]
    async fn the_ebird_probe_names_a_server_error_and_a_dead_host() {
        let err = ebird_probe_for(ebird_settings(), Some(stub("503 Service Unavailable", "")))()
            .await
            .expect_err("a 503 fails");
        assert!(err.contains("503"), "{err}");

        // Nothing listens on the port the OS just handed out and released.
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let err = ebird_probe_for(ebird_settings(), Some(dead))()
            .await
            .expect_err("a dead host fails");
        assert!(err.to_lowercase().contains("http error"), "{err}");
    }

    #[test]
    fn ebird_is_untested_without_a_key_and_explains_a_key_it_cannot_use() {
        // No key: no probe (the card says "not configured").
        assert!(ebird_probe(Some(&config_with(&[]))).is_none());
        // A key and no coordinates: a probe, and its answer is the reason.
        let probe = ebird_probe(Some(&config_with(&[("EBIRD_API_KEY", "k")])))
            .expect("a key was set, so the integration was asked for");
        let err = futures_lite_block(probe()).expect_err("unusable");
        assert!(err.contains("no coordinates"), "{err}");
    }

    /// Drive a future to completion on a throwaway runtime.
    fn futures_lite_block<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(f)
    }

    const OPEN_METEO_BODY: &str = r#"{"hourly":{"time":["2026-10-21T00:00","2026-10-21T01:00"],
        "temperature_2m":[8.1,7.9],"precipitation":[0.0,0.1],"wind_speed_10m":[3.0,3.5],
        "wind_direction_10m":[200,210],"pressure_msl":[1012.0,1012.5],"cloud_cover":[40,50],
        "weather_code":[1,2]}}"#;

    #[tokio::test]
    async fn the_weather_probe_passes_and_counts_the_rows() {
        let probe = weather_probe_for(
            birdnet_integrations::weather::Provider::OpenMeteo,
            Some(stub("200 OK", OPEN_METEO_BODY)),
            52.5,
            13.4,
        );
        let ok = probe().await.expect("a forecast passes");
        assert!(ok.contains("Open-Meteo answered with 2 hourly row"), "{ok}");
    }

    #[tokio::test]
    async fn the_weather_probe_names_what_went_wrong() {
        let refused = weather_probe_for(
            birdnet_integrations::weather::Provider::PirateWeather {
                api_key: "pw-secret".to_owned(),
            },
            Some(stub("401 Unauthorized", "{}")),
            52.5,
            13.4,
        )()
        .await
        .expect_err("401");
        assert!(refused.contains("401"), "{refused}");
        assert!(
            !refused.contains("pw-secret"),
            "the key reached the message: {refused}"
        );

        let empty = weather_probe_for(
            birdnet_integrations::weather::Provider::OpenMeteo,
            Some(stub("200 OK", r#"{"hourly":{"time":[]}}"#)),
            52.5,
            13.4,
        )()
        .await
        .expect_err("an answer with no rows is not a pass");
        assert!(empty.contains("no hourly data"), "{empty}");
    }
}
