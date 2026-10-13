//! `{:?}` on any type that holds a credential prints the type without it.
//!
//! The root crate's `no_secret_in_derived_debug_guard` keeps a credential-
//! holding type from *deriving* `Debug`; this checks that the impls written in
//! its place actually leave the secret out — and keep what makes the value
//! worth printing, so "print nothing" cannot pass for a redaction.

use birdnet_integrations::dispatch::{self, Message, Severity};
use birdnet_integrations::email::EmailConfig;
use birdnet_integrations::mqtt::{MqttConfig, QosLevel};
use birdnet_integrations::offsite::sigv4;

/// Every secret below contains this, so one search finds any of them.
const SECRET: &str = "S3CRET";

fn check(what: &str, shown: &str, kept: &str) {
    assert!(
        !shown.contains(SECRET),
        "{what} printed its secret: {shown}"
    );
    assert!(shown.contains(kept), "{what} lost `{kept}`: {shown}");
}

#[test]
fn notification_targets_and_their_requests() {
    let msg = Message {
        title: "t".into(),
        body: "b".into(),
        severity: Severity::Info,
        image_url: None,
    };
    for (url, kept) in [
        ("discord://1234567890/S3CRETtoken/", "1234567890"),
        ("tgram://123456789:S3CRET/4242/", "4242"),
        ("ntfy://ada:S3CRET@ntfy.lan/garden", "ntfy.lan"),
        ("ntfy://tk_S3CRET@ntfy.lan/garden", "garden"),
        ("gotify://gotify.lan/S3CRET", "gotify.lan"),
        ("pover://S3CRETuser@S3CRETtoken", "Pushover"),
        ("slack://T0/B0/S3CRET/#birds", "#birds"),
        ("slack://xoxb-S3CRET/#c", "Slack"),
        (
            "jsons://ada:S3CRET@hooks.example.com/b",
            "hooks.example.com",
        ),
    ] {
        let target = dispatch::parse(url).unwrap_or_else(|e| panic!("{url}: {e}"));
        check(url, &format!("{target:?}"), kept);
        for plan in dispatch::plans(&target, &msg) {
            check(url, &format!("{plan:?}"), plan.kind);
        }
    }
}

#[test]
fn service_clients_and_configs() {
    let email = EmailConfig {
        smtp_host: "smtp.example.com".into(),
        smtp_port: 587,
        username: "ada".into(),
        password: SECRET.into(),
        from_address: "a@example.com".into(),
        to_address: "b@example.com".into(),
        from_name: None,
        use_starttls: true,
        min_confidence: 0.8,
        cooldown_secs: 60,
    };
    check("EmailConfig", &format!("{email:?}"), "smtp.example.com");

    let mqtt = MqttConfig {
        host: "broker.lan".into(),
        port: 1883,
        client_id: "c".into(),
        username: Some("ada".into()),
        password: Some(SECRET.into()),
        topic_prefix: "birdnet".into(),
        qos: QosLevel::AtMostOnce,
        retain: false,
        timeout_ms: 1000,
        tls: None,
    };
    check("MqttConfig", &format!("{mqtt:?}"), "broker.lan");

    let hb =
        birdnet_integrations::heartbeat::HeartbeatClient::new("https://hc-ping.com/S3CRET-uuid")
            .expect("heartbeat");
    check("HeartbeatClient", &format!("{hb:?}"), "hc-ping.com");

    let bw = birdnet_integrations::birdweather::Client::new(SECRET, 1.0, 2.0).expect("bw");
    check("birdweather::Client", &format!("{bw:?}"), "lat");

    let eb = birdnet_integrations::ebird::Client::new(SECRET, None).expect("ebird");
    check("ebird::Client", &format!("{eb:?}"), "base_url");

    let apprise = birdnet_integrations::apprise::Client::new(
        "http://apprise.lan:8000/notify/S3CRET",
        birdnet_integrations::apprise::NotifyConfig::default(),
    )
    .expect("apprise");
    check("apprise::Client", &format!("{apprise:?}"), "apprise.lan");

    for (provider, kept) in [
        (
            birdnet_integrations::weather::Provider::Wunderground {
                station_id: "KMAHANOV10".into(),
                api_key: SECRET.into(),
            },
            "KMAHANOV10",
        ),
        (
            birdnet_integrations::weather::Provider::PirateWeather {
                api_key: SECRET.into(),
            },
            "PirateWeather",
        ),
    ] {
        check("weather::Provider", &format!("{provider:?}"), kept);
    }
}

#[test]
fn a_signed_request_keeps_its_scope_and_drops_its_signature() {
    let creds = sigv4::Credentials {
        access_key: "AKIDEXAMPLE".into(),
        secret_key: SECRET.into(),
    };
    let signed = sigv4::sign(
        &creds,
        &sigv4::Request {
            method: "PUT",
            host: "s3.example.com",
            path: "/bucket/key",
            query: &[],
            payload_sha256: "UNSIGNED-PAYLOAD",
            extra_headers: &[],
            region: "us-east-1",
            timestamp: "20261013T000000Z",
        },
    );
    let signature = signed
        .authorization
        .split("Signature=")
        .nth(1)
        .expect("a signature");
    let shown = format!("{signed:?}");
    assert!(!shown.contains(signature), "{shown}");
    check(
        "Signed",
        &shown,
        "Credential=AKIDEXAMPLE/20261013/us-east-1",
    );
}
