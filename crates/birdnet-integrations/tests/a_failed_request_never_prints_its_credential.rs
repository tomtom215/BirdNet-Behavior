//! An HTTP failure must not print the credential that was in its URL.
//!
//! `reqwest::Error`'s `Display` ends in ` for url (<the whole URL>)`. Three
//! clients put their secret in exactly that URL — the BirdWeather station
//! token as a path segment, the heartbeat ping id as the path, the Apprise API
//! key in the path — and stringified the error with `e.to_string()`. From there
//! it reached a WARN line in the journal, the notification log, the
//! store-and-forward queue's `last_error`, `errors.jsonl`, and the support
//! bundle, which ships the last two without redaction. The native dispatcher
//! and the webhook client already used `without_url()`; these did not.
//!
//! Each client is pointed at a port nothing listens on, with a recognisable
//! secret in the URL, and the error it returns must not contain it.

use birdnet_integrations::apprise::{Client as Apprise, NotifyConfig, NotifyType};
use birdnet_integrations::birdweather::{Client as BirdWeather, DetectionPost};
use birdnet_integrations::heartbeat::HeartbeatClient as Heartbeat;

const SECRET: &str = "s3cr3t-token-7f2e";

/// A local port that refuses connections: bind, note the port, drop.
fn refused() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    format!("http://{addr}")
}

#[tokio::test]
async fn birdweather_does_not_print_its_station_token() {
    let client = BirdWeather::new(SECRET, 51.5, -0.1)
        .expect("client")
        .with_base_url(&refused());
    let err = client
        .post_detection(&DetectionPost {
            timestamp: "2026-05-19T06:30:00+01:00".into(),
            common_name: "Eurasian Magpie".into(),
            scientific_name: "Pica pica".into(),
            confidence: 0.9,
            lat: 51.5,
            lon: -0.1,
            soundscape: None,
            algorithm: None,
        })
        .await
        .expect_err("nothing is listening");
    let text = err.to_string();
    assert!(
        !text.contains(SECRET),
        "the station token is in the error: {text}"
    );
    // Counterpart: the error still says something happened.
    assert!(!text.trim().is_empty());
}

#[tokio::test]
async fn the_heartbeat_does_not_print_its_ping_id() {
    let client = Heartbeat::new(&format!("{}/{SECRET}", refused())).expect("client");
    let text = client
        .ping()
        .await
        .expect_err("nothing is listening")
        .to_string();
    assert!(
        !text.contains(SECRET),
        "the ping id is in the error: {text}"
    );
}

#[tokio::test]
async fn the_apprise_api_does_not_print_its_key() {
    let mut client = Apprise::new(
        &format!("{}/notify/{SECRET}", refused()),
        NotifyConfig::default(),
    )
    .expect("client");
    let text = client
        .send_notification("t", "b", NotifyType::Info)
        .await
        .expect_err("nothing is listening")
        .to_string();
    assert!(
        !text.contains(SECRET),
        "the Apprise key is in the error: {text}"
    );
}
