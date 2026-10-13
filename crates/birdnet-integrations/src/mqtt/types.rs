//! MQTT integration types.
//!
//! Configuration, payload schemas, and error types for the lightweight
//! MQTT 3.1.1 publisher used to broadcast bird detection events to `IoT` (Internet of Things)
//! brokers (Home Assistant, Node-RED, Mosquitto, etc.).

use std::fmt;

/// MQTT broker connection configuration.
///
/// Shared by both connection styles: the stateless per-message publish that
/// detections use, and the one long-lived presence session that holds the
/// last will. Give the presence session a different [`client_id`]: a broker
/// that receives a second CONNECT with a client identifier already in use
/// must disconnect the first (§3.1.3.1), so sharing one would have every
/// detection publish kick the presence session off and the station would
/// flap between `online` and `offline` for ever.
///
/// [`client_id`]: Self::client_id
#[derive(Debug, Clone)]
pub struct MqttConfig {
    /// Broker hostname or IP address.
    pub host: String,
    /// Broker port (default: 1883; TLS: 8883).
    pub port: u16,
    /// MQTT client identifier (must be unique per broker session).
    pub client_id: String,
    /// Optional username for broker authentication.
    pub username: Option<String>,
    /// Optional password for broker authentication.
    pub password: Option<String>,
    /// Topic prefix for all published messages.
    ///
    /// Detection events are published to `{prefix}/detection/{species_safe}`.
    /// `{prefix}/status` carries the station's presence, both the `online`
    /// this process publishes and the `offline` the broker publishes on its
    /// behalf — see [`Will`] and `PresenceSession`. This doc comment named
    /// that topic as "the LWT topic" for a long time before anything
    /// registered a will.
    pub topic_prefix: String,
    /// Quality of service level for PUBLISH packets.
    pub qos: QosLevel,
    /// Whether to set the RETAIN flag on detection messages.
    pub retain: bool,
    /// Connection and I/O timeout.
    pub timeout_ms: u64,
    /// TLS settings, or `None` for a plaintext connection.
    pub tls: Option<TlsConfig>,
}

/// How to establish TLS to the broker.
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// PEM file of certificates to trust *in addition to* the platform store.
    ///
    /// A home broker is very often behind a private CA or a self-signed
    /// certificate, neither of which the platform store knows. Which file to
    /// point at depends on which of those it is, and the two are not
    /// interchangeable — both cases are pinned by tests in
    /// `tests/mqtt_over_tls.rs`, because the first version of this comment got
    /// it wrong:
    ///
    /// * **Private CA** — name the *CA's* certificate. Naming the broker's own
    ///   certificate fails with `UnknownIssuer`, which reads like a bad file
    ///   rather than the wrong one.
    /// * **Self-signed, no CA** — name the broker's own certificate. It signed
    ///   itself, so the chain terminates at the anchor.
    ///
    /// Either way the broker's certificate must carry `CA:FALSE`. A bare
    /// `openssl req -x509`, which is what every "make a self-signed cert"
    /// recipe gives, defaults to `CA:TRUE`, and rustls refuses such a
    /// certificate when a server presents it (`CaUsedAsEndEntity`). Add
    /// `-addext basicConstraints=critical,CA:FALSE`.
    ///
    /// There is deliberately no "skip verification" option: it is the setting
    /// that gets switched on during setup and never switched off, and an
    /// unverified TLS connection to a broker carrying station credentials is
    /// worse than a plaintext one, because it looks safe.
    pub ca_file: Option<std::path::PathBuf>,
    /// Hostname to validate the certificate against.
    ///
    /// Defaults to [`MqttConfig::host`]. Set it when connecting by IP to a
    /// broker whose certificate names a hostname, which is the common shape on
    /// a home LAN with no internal DNS.
    pub server_name: Option<String>,
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 1883,
            client_id: "birdnet-behavior".to_string(),
            username: None,
            password: None,
            topic_prefix: "birdnet".to_string(),
            qos: QosLevel::AtMostOnce,
            retain: false,
            timeout_ms: 5_000,
            tls: None,
        }
    }
}

impl MqttConfig {
    /// Build the detection topic for a species.
    ///
    /// `species_safe` should use underscores instead of spaces.
    #[must_use]
    pub fn detection_topic(&self, species_safe: &str) -> String {
        format!("{}/detection/{}", self.topic_prefix, species_safe)
    }

    /// Build the status topic (used for heartbeat / LWT messages).
    #[must_use]
    pub fn status_topic(&self) -> String {
        format!("{}/status", self.topic_prefix)
    }
}

/// A last will: what the broker publishes when this client stops answering.
///
/// Registered in CONNECT (§3.1.2.5) and held by the broker for the life of
/// the session. The broker publishes it when the connection closes for any
/// reason *except* a DISCONNECT packet — a network drop, a power cut, a
/// killed process, a keepalive that goes unanswered. That exception is the
/// whole design constraint: a will is only useful on a connection the client
/// intends to keep open.
#[derive(Debug, Clone)]
pub struct Will {
    /// Topic the broker publishes to.
    pub topic: String,
    /// Payload the broker publishes.
    pub payload: Vec<u8>,
    /// `QoS` for the broker's publish.
    pub qos: QosLevel,
    /// Whether the broker retains it.
    ///
    /// Should be `true` for a presence topic. A subscriber that connects
    /// after the station died — Home Assistant restarting, which is the
    /// common case — sees nothing at all otherwise, and an entity with no
    /// state reads as "unknown" rather than "offline".
    pub retain: bool,
}

/// MQTT Quality of Service level.
///
/// `QoS` 2 is deliberately absent. It needs a four-packet exchange and
/// duplicate suppression that only pays for itself when a repeat is harmful,
/// and nothing this station publishes is: a detection re-sent is the same
/// detection, and a status re-sent is the same status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QosLevel {
    /// Fire-and-forget. No acknowledgement, so a publish reports success
    /// whether or not the broker received it — the write went into the
    /// socket, and that is all the caller learns.
    AtMostOnce = 0,
    /// The broker acknowledges receipt with a PUBACK, which the publisher
    /// waits for. A publish that returns `Ok` at this level means the broker
    /// has the message.
    AtLeastOnce = 1,
}

impl From<QosLevel> for u8 {
    fn from(q: QosLevel) -> Self {
        q as Self
    }
}

/// MQTT detection event payload (serialised as JSON).
///
/// This is the message body published to `{prefix}/detection/{species}`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectionPayload {
    /// ISO 8601 timestamp of the detection (`YYYY-MM-DDTHH:MM:SS`).
    pub timestamp: String,
    /// Species scientific name.
    pub scientific_name: String,
    /// Species common name.
    pub common_name: String,
    /// Confidence score \[0.0, 1.0\].
    pub confidence: f32,
    /// Confidence as integer percentage (0–100).
    pub confidence_pct: u32,
    /// Path to the extracted audio clip, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// RTSP stream identifier, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtsp_id: Option<String>,
    /// The species' detections before this one — see [`SpeciesHistory`].
    #[serde(flatten)]
    pub species_history: SpeciesHistory,
}

/// What the station had heard of a species before the detection being
/// published, as `species_first_detected_at` / `species_last_detected_at`.
///
/// Three states, because the payload has to tell them apart:
///
/// * [`Seen`](Self::Seen) — both keys, each a `YYYY-MM-DDTHH:MM:SS` local
///   timestamp like `timestamp`.
/// * [`Never`](Self::Never) — both keys, `null`: nothing before this one. A
///   first-ever detection, stated as a fact.
/// * [`Unknown`](Self::Unknown) — neither key. The history could not be read,
///   and a `null` here would claim a first-ever detection the station never
///   established; an automation keyed on "new species" would fire on a
///   database error. Absent reads as absent.
///
/// tphakala/birdnet-go sends `null` for both of the last two cases
/// (`speciesFirstDetectedAt`, 6dfdc10); the keys here are `snake_case` like
/// every other field in this payload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SpeciesHistory {
    /// The history could not be read: both keys are omitted.
    #[default]
    Unknown,
    /// No earlier detection: both keys are `null`.
    Never,
    /// The earliest and latest earlier detections.
    Seen {
        /// `species_first_detected_at`.
        first: String,
        /// `species_last_detected_at`.
        last: String,
    },
}

impl serde::Serialize for SpeciesHistory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        const FIRST: &str = "species_first_detected_at";
        const LAST: &str = "species_last_detected_at";
        match self {
            Self::Unknown => serializer.serialize_map(Some(0))?.end(),
            Self::Never => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry(FIRST, &None::<&str>)?;
                map.serialize_entry(LAST, &None::<&str>)?;
                map.end()
            }
            Self::Seen { first, last } => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry(FIRST, first)?;
                map.serialize_entry(LAST, last)?;
                map.end()
            }
        }
    }
}

/// Errors produced by the MQTT publisher.
#[derive(Debug)]
pub enum MqttError {
    /// TCP connection to the broker failed.
    Connection(String),
    /// The broker rejected the CONNECT packet.
    ConnAck(ConnAckError),
    /// Writing to or reading from the TCP stream failed.
    Io(std::io::Error),
    /// MQTT packet encoding failed.
    Encode(String),
    /// The broker sent something the client could not make sense of: a packet
    /// type where another was required, or an acknowledgement carrying the
    /// wrong packet identifier.
    Protocol(String),
    /// JSON serialisation of the payload failed.
    Serialise(String),
    /// No MQTT configuration is set.
    NotConfigured,
    /// TLS could not be established: no usable trust anchor, an unreadable or
    /// unparseable CA file, or a certificate the broker presented that does
    /// not verify.
    Tls(String),
}

impl fmt::Display for MqttError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(msg) => write!(f, "MQTT connection error: {msg}"),
            Self::ConnAck(e) => write!(f, "MQTT broker rejected CONNECT: {e}"),
            Self::Io(e) => write!(f, "MQTT I/O error: {e}"),
            Self::Tls(msg) => write!(f, "MQTT TLS error: {msg}"),
            Self::Encode(msg) => write!(f, "MQTT encoding error: {msg}"),
            Self::Protocol(msg) => write!(f, "MQTT protocol error: {msg}"),
            Self::Serialise(msg) => write!(f, "MQTT payload serialisation error: {msg}"),
            Self::NotConfigured => write!(f, "MQTT not configured"),
        }
    }
}

impl std::error::Error for MqttError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Io(e) = self {
            Some(e)
        } else {
            None
        }
    }
}

impl From<std::io::Error> for MqttError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Reason codes returned in CONNACK packets (MQTT 3.1.1 §3.2.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnAckError {
    /// Protocol version not supported by the broker.
    UnacceptableProtocolVersion,
    /// Client identifier rejected.
    IdentifierRejected,
    /// Broker unavailable (server error).
    ServerUnavailable,
    /// Bad username or password.
    BadCredentials,
    /// Client not authorised.
    NotAuthorised,
    /// Unknown / reserved return code.
    Unknown(u8),
}

impl fmt::Display for ConnAckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnacceptableProtocolVersion => write!(f, "unacceptable protocol version"),
            Self::IdentifierRejected => write!(f, "client identifier rejected"),
            Self::ServerUnavailable => write!(f, "server unavailable"),
            Self::BadCredentials => write!(f, "bad username or password"),
            Self::NotAuthorised => write!(f, "not authorised"),
            Self::Unknown(code) => write!(f, "unknown return code 0x{code:02X}"),
        }
    }
}

impl From<u8> for ConnAckError {
    fn from(code: u8) -> Self {
        match code {
            1 => Self::UnacceptableProtocolVersion,
            2 => Self::IdentifierRejected,
            3 => Self::ServerUnavailable,
            4 => Self::BadCredentials,
            5 => Self::NotAuthorised,
            other => Self::Unknown(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_sensible() {
        let cfg = MqttConfig::default();
        assert_eq!(cfg.port, 1883);
        assert_eq!(cfg.qos, QosLevel::AtMostOnce);
        assert!(!cfg.retain);
        assert_eq!(cfg.timeout_ms, 5_000);
    }

    #[test]
    fn detection_topic_format() {
        let cfg = MqttConfig::default();
        assert_eq!(
            cfg.detection_topic("Turdus_merula"),
            "birdnet/detection/Turdus_merula"
        );
        assert_eq!(cfg.status_topic(), "birdnet/status");
    }

    #[test]
    fn qos_to_u8() {
        assert_eq!(u8::from(QosLevel::AtMostOnce), 0);
        assert_eq!(u8::from(QosLevel::AtLeastOnce), 1);
    }

    #[test]
    fn connack_error_from_code() {
        assert_eq!(
            ConnAckError::from(1),
            ConnAckError::UnacceptableProtocolVersion
        );
        assert_eq!(ConnAckError::from(4), ConnAckError::BadCredentials);
        assert!(matches!(ConnAckError::from(99), ConnAckError::Unknown(99)));
    }

    #[test]
    fn mqtt_error_display() {
        let e = MqttError::Connection("refused".into());
        assert!(format!("{e}").contains("connection error"));
        let e2 = MqttError::ConnAck(ConnAckError::BadCredentials);
        assert!(format!("{e2}").contains("bad username"));
    }

    #[test]
    fn detection_payload_serialises() {
        let payload = DetectionPayload {
            timestamp: "2026-03-23T06:30:00".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            confidence_pct: 87,
            file_name: None,
            rtsp_id: None,
            species_history: SpeciesHistory::Unknown,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("Turdus merula"));
        assert!(json.contains("0.87"));
        // file_name and rtsp_id should be omitted when None
        assert!(!json.contains("file_name"));
    }

    fn history_json(history: SpeciesHistory) -> serde_json::Value {
        let payload = DetectionPayload {
            timestamp: "2026-05-22T21:10:31".into(),
            scientific_name: "Turdus merula".into(),
            common_name: "Eurasian Blackbird".into(),
            confidence: 0.87,
            confidence_pct: 87,
            file_name: None,
            rtsp_id: None,
            species_history: history,
        };
        serde_json::to_value(&payload).unwrap()
    }

    /// Seen: both keys, as timestamps in the payload's own shape.
    #[test]
    fn species_history_seen_carries_both_timestamps() {
        let v = history_json(SpeciesHistory::Seen {
            first: "2026-03-01T05:10:00".into(),
            last: "2026-05-22T06:00:00".into(),
        });
        assert_eq!(v["species_first_detected_at"], "2026-03-01T05:10:00");
        assert_eq!(v["species_last_detected_at"], "2026-05-22T06:00:00");
    }

    /// Never and Unknown must not serialise alike: one is a first-ever
    /// detection, the other a failed read, and an automation that fires on
    /// "new species" can only tell them apart by key presence.
    #[test]
    fn species_history_never_is_null_and_unknown_is_absent() {
        let never = history_json(SpeciesHistory::Never);
        let obj = never.as_object().unwrap();
        assert!(obj.contains_key("species_first_detected_at"), "{never}");
        assert!(obj["species_first_detected_at"].is_null());
        assert!(obj["species_last_detected_at"].is_null());

        let unknown = history_json(SpeciesHistory::Unknown);
        let obj = unknown.as_object().unwrap();
        assert!(!obj.contains_key("species_first_detected_at"), "{unknown}");
        assert!(!obj.contains_key("species_last_detected_at"), "{unknown}");
        assert_eq!(
            obj["scientific_name"], "Turdus merula",
            "the rest is intact"
        );
    }
}
