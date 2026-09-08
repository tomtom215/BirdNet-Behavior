//! The channel probes "Test all channels" runs (DD-24), built from the
//! integrations this binary configured.

use std::sync::Arc;

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
    }
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
