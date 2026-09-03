//! Pure-Rust MQTT 3.1.1 publisher.
//!
//! Implements a stateless, fire-and-forget MQTT publisher using only
//! `std::net::TcpStream`.  No background thread.  No heap allocations
//! beyond packet buffers.  No external MQTT library dependency.
//!
//! ## Protocol subset
//!
//! | Direction | Packet     | Notes                                         |
//! |-----------|------------|-----------------------------------------------|
//! | C → B     | CONNECT    | Variable header + payload, optional will      |
//! | B → C     | CONNACK    | Return code checked                           |
//! | C → B     | PUBLISH    | `QoS` 0, or `QoS` 1 with a packet identifier  |
//! | B → C     | PUBACK     | Read and matched at `QoS` 1                   |
//! | C → B     | PINGREQ    | Keepalive, persistent sessions only           |
//! | B → C     | PINGRESP   | Keepalive, persistent sessions only           |
//! | C → B     | DISCONNECT | Clean disconnect                              |
//!
//! `QoS` 1 works because the connection is synchronous and carries one
//! message: send PUBLISH, block for its PUBACK, disconnect. There is no
//! in-flight window to track, so none is implemented.
//!
//! This module's doc comment used to say that `QoS` 1 "is sent at `QoS` 0
//! after logging a warning". There was no warning and no branch —
//! [`MqttConfig::qos`] had no reader anywhere in the workspace, so a station
//! configured for `QoS` 1 silently got `QoS` 0, which is exactly the setting
//! where "the broker never received it" and "the broker acknowledged it" look
//! identical to the caller.
//!
//! ## Two kinds of connection
//!
//! [`publish`] is stateless: one TCP connection per message, which is right
//! for a detection stream where each message stands alone.
//!
//! [`PresenceSession`] is the opposite and exists for one reason: a last will
//! needs a session for the broker to notice dying. A will registered on a
//! connection that immediately sends DISCONNECT is *discarded* by the broker
//! (§3.14), so per-message connections cannot carry one however the flags are
//! set. The presence session holds one connection open, does nothing but
//! keepalive, and lets the broker publish `offline` when the station stops
//! answering.
//!
//! ## Wire format reference
//!
//! MQTT 3.1.1 specification: <http://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html>

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::types::{ConnAckError, MqttConfig, MqttError, QosLevel, TlsConfig, Will};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Publish a single MQTT message using a new TCP connection.
///
/// Connects to the broker, performs a CONNECT handshake, publishes
/// `payload` to `topic` at `QoS` 0, then sends DISCONNECT.
///
/// The connection is always closed before returning, even on error.
///
/// # Errors
///
/// Returns [`MqttError`] if the connection fails, the broker rejects
/// the CONNECT, or any I/O error occurs.
pub fn publish(config: &MqttConfig, topic: &str, payload: &[u8]) -> Result<(), MqttError> {
    publish_with(config, topic, payload, config.retain)
}

/// Publish a single message, overriding [`MqttConfig::retain`].
///
/// Some topics must be retained whatever the station's preference for its
/// detection stream is — Home Assistant discovery configs are the reason this
/// exists. See [`super::discovery`].
///
/// # Errors
///
/// Returns [`MqttError`] if the connection fails, the broker rejects the
/// CONNECT, the `QoS` 1 acknowledgement does not arrive, or any I/O error
/// occurs.
pub fn publish_with(
    config: &MqttConfig,
    topic: &str,
    payload: &[u8],
    retain: bool,
) -> Result<(), MqttError> {
    let mut transport = open(config, None)?;
    send_publish(&mut transport, topic, payload, retain, config.qos)
        .and_then(|()| send_disconnect(&mut transport))
}

/// Open a connection and complete the CONNECT → CONNACK handshake.
///
/// `will` is registered with the broker for the life of the session. It is
/// meaningless on a connection that goes on to send DISCONNECT, which
/// [`publish_with`] does — hence `None` there.
fn open(config: &MqttConfig, will: Option<&Will>) -> Result<Transport, MqttError> {
    // The TLS session is built *before* the socket is opened. A CA file that
    // is missing or unusable is a configuration mistake, and reporting it as
    // "connection refused" — which is what happens when the broker is also
    // down — sends the operator to look at the wrong thing.
    let tls = config
        .tls
        .as_ref()
        .map(|tls| {
            let name = tls.server_name.as_deref().unwrap_or(&config.host);
            tls_session(tls, name)
        })
        .transpose()?;

    let stream = connect(config)?;
    let mut transport = match tls {
        None => Transport::Plain(stream),
        Some(conn) => Transport::Tls(Box::new(rustls::StreamOwned::new(conn, stream))),
    };
    send_connect(&mut transport, config, will)?;
    recv_connack(&mut transport)?;
    Ok(transport)
}

/// The socket, plaintext or wrapped in TLS.
///
/// An enum rather than a generic because [`PresenceSession`] holds one across
/// calls, and boxed because `StreamOwned` carries the whole rustls connection
/// state — several kilobytes that would otherwise be the size of every
/// `Transport`, including the plaintext one.
enum Transport {
    /// A plain TCP connection to the broker.
    Plain(TcpStream),
    /// A TLS connection to the broker.
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

/// Open the TCP connection, with both timeouts applied.
fn connect(config: &MqttConfig) -> Result<TcpStream, MqttError> {
    let addr = format!("{}:{}", config.host, config.port);

    let timeout = Duration::from_millis(config.timeout_ms);
    // Bound the TCP connect so a broker host that black-holes SYNs can't hang
    // the publish for the OS default (often > 60 s) — `publish_detection` runs
    // per detection on the blocking pool, which a stuck connect would exhaust.
    // `TcpStream::connect` ignores `timeout`; `connect_timeout` needs a resolved
    // `SocketAddr`, so resolve first (DNS) and take the first address.
    let sock_addr = addr
        .to_socket_addrs()
        .map_err(|e| MqttError::Connection(format!("{addr}: {e}")))?
        .next()
        .ok_or_else(|| MqttError::Connection(format!("{addr}: no addresses resolved")))?;
    let stream = TcpStream::connect_timeout(&sock_addr, timeout)
        .map_err(|e| MqttError::Connection(format!("{addr}: {e}")))?;

    stream
        .set_read_timeout(Some(timeout))
        .map_err(MqttError::Io)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(MqttError::Io)?;

    Ok(stream)
}

/// Build the rustls session for one connection.
///
/// Trust anchors are the platform store plus anything in `ca_file`. Which
/// certificate belongs in that file depends on whether the broker is behind a
/// private CA or self-signed; see [`TlsConfig::ca_file`].
fn tls_session(tls: &TlsConfig, server_name: &str) -> Result<rustls::ClientConnection, MqttError> {
    let mut roots = rustls::RootCertStore::empty();

    // Platform roots are best-effort: a minimal container image may carry no
    // trust store at all, and that is not a failure when `ca_file` supplies
    // the anchor the broker actually uses.
    match rustls_native_certs::load_native_certs() {
        result if result.certs.is_empty() && tls.ca_file.is_none() => {
            return Err(MqttError::Tls(format!(
                "no trust anchors: the platform certificate store is empty or unreadable \
                 ({} error(s)) and no CA file was configured",
                result.errors.len()
            )));
        }
        result => {
            let (added, _) = roots.add_parsable_certificates(result.certs);
            tracing::debug!(added, "loaded platform trust anchors for MQTT TLS");
        }
    }

    if let Some(path) = &tls.ca_file {
        let pem =
            std::fs::read(path).map_err(|e| MqttError::Tls(format!("{}: {e}", path.display())))?;
        let mut cursor = std::io::BufReader::new(std::io::Cursor::new(pem));
        let certs: Vec<_> = rustls_pki_types::pem::PemObject::pem_reader_iter(&mut cursor)
            .collect::<Result<Vec<rustls_pki_types::CertificateDer<'static>>, _>>()
            .map_err(|e| MqttError::Tls(format!("{}: {e}", path.display())))?;
        if certs.is_empty() {
            return Err(MqttError::Tls(format!(
                "{}: no certificates found in this file",
                path.display()
            )));
        }
        let (added, ignored) = roots.add_parsable_certificates(certs);
        tracing::info!(
            path = %path.display(),
            added,
            ignored,
            "loaded MQTT TLS trust anchors from the configured CA file"
        );
        if added == 0 {
            return Err(MqttError::Tls(format!(
                "{}: none of the certificates in this file could be used as a trust anchor",
                path.display()
            )));
        }
    }

    // `ring` explicitly rather than the process-wide default provider: which
    // provider that is depends on which other crate installed one first, and
    // this crate cross-compiles to aarch64 where `aws-lc-rs` needs a C
    // toolchain in the cross image.
    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|e| MqttError::Tls(e.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();

    let name = rustls_pki_types::ServerName::try_from(server_name.to_string())
        .map_err(|e| MqttError::Tls(format!("{server_name:?} is not a valid server name: {e}")))?;
    rustls::ClientConnection::new(config.into(), name).map_err(|e| MqttError::Tls(e.to_string()))
}

// ---------------------------------------------------------------------------
// Presence session
// ---------------------------------------------------------------------------

/// Keepalive advertised on the presence connection, in seconds.
///
/// The broker publishes the will after 1.5 of these without hearing from us
/// (§3.1.2.10), so 30 puts the worst-case "station went offline" delay at
/// 45 seconds. Short enough that an operator watching Home Assistant sees a
/// power cut promptly; long enough that a station on a metered link is
/// sending two small packets a minute and no more.
pub const KEEPALIVE_SECS: u16 = 30;

/// A held-open connection whose only job is to carry the last will.
///
/// Nothing is published through it except the `online` that opens it. It
/// exists because a will and a per-message connection are mutually exclusive:
/// the broker discards a will when the client sends DISCONNECT (§3.14), which
/// is the *only* way [`publish_with`] ever ends. So the presence of the
/// station cannot be inferred from its detection traffic — it needs a
/// connection that is expected to stay up, and whose unexpected death is
/// therefore meaningful.
///
/// The caller owns the loop. This type does one thing per call and reports
/// what happened; reconnect policy, timing and metrics belong to the
/// application, which is also the only layer that knows when the station is
/// shutting down on purpose.
pub struct PresenceSession {
    /// The live connection, held open between calls.
    transport: Transport,
    /// The topic this session's `online`/`offline` messages go to.
    topic: String,
}

impl std::fmt::Debug for PresenceSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceSession")
            .field("topic", &self.topic)
            .finish_non_exhaustive()
    }
}

/// What the station publishes to say it is running.
pub const PRESENCE_ONLINE: &[u8] = b"online";

/// What the broker publishes on the station's behalf when it stops answering.
pub const PRESENCE_OFFLINE: &[u8] = b"offline";

impl PresenceSession {
    /// Connect, register the will, and announce `online`.
    ///
    /// The `online` is retained so a subscriber that connects later — Home
    /// Assistant restarting — sees the current state instead of waiting for
    /// the next change, which on a healthy station never comes.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if the connection, the CONNECT handshake, or the
    /// `online` publish fails.
    pub fn connect(config: &MqttConfig) -> Result<Self, MqttError> {
        let topic = config.status_topic();
        let will = Will {
            topic: topic.clone(),
            payload: PRESENCE_OFFLINE.to_vec(),
            // QoS 1: the will is published once, at the moment the station is
            // by definition not around to notice it was dropped.
            qos: QosLevel::AtLeastOnce,
            retain: true,
        };
        let mut transport = open(config, Some(&will))?;
        send_publish(
            &mut transport,
            &topic,
            PRESENCE_ONLINE,
            true,
            QosLevel::AtLeastOnce,
        )?;
        Ok(Self { transport, topic })
    }

    /// Send one keepalive and wait for the broker's answer.
    ///
    /// Call at least twice per [`KEEPALIVE_SECS`]. An `Err` means the session
    /// is gone: drop it and reconnect.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if the ping cannot be written or is not answered.
    pub fn keepalive(&mut self) -> Result<(), MqttError> {
        ping(&mut self.transport)
    }

    /// Publish a message on this session's connection rather than opening a
    /// new one.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if the publish fails or is not acknowledged.
    pub fn publish(&mut self, topic: &str, payload: &[u8], retain: bool) -> Result<(), MqttError> {
        send_publish(
            &mut self.transport,
            topic,
            payload,
            retain,
            QosLevel::AtLeastOnce,
        )
    }

    /// Say `offline` deliberately, then close the session.
    ///
    /// For a planned stop — an upgrade, a `systemctl stop`. Without this the
    /// station would look online for up to 1.5 keepalive periods after it had
    /// already exited cleanly, and an operator restarting the service would
    /// watch Home Assistant report a station that is not running as running.
    /// Sending DISCONNECT also tells the broker to discard the will, so the
    /// `offline` published here is the last word rather than being followed
    /// by a second one.
    ///
    /// # Errors
    ///
    /// Returns [`MqttError`] if the final publish or DISCONNECT fails. The
    /// connection is closed either way — the session is consumed.
    pub fn shutdown(mut self) -> Result<(), MqttError> {
        send_publish(
            &mut self.transport,
            &self.topic,
            PRESENCE_OFFLINE,
            true,
            QosLevel::AtLeastOnce,
        )?;
        send_disconnect(&mut self.transport)
    }
}

// ---------------------------------------------------------------------------
// CONNECT packet (§3.1)
// ---------------------------------------------------------------------------

fn send_connect<S: Write>(
    stream: &mut S,
    config: &MqttConfig,
    will: Option<&Will>,
) -> Result<(), MqttError> {
    // Connect flags byte (§3.1.2.3):
    //   bit 7: Username flag
    //   bit 6: Password flag
    //   bit 5: Will Retain
    //   bits 4-3: Will QoS
    //   bit 2: Will flag
    //   bit 1: Clean Session
    let mut connect_flags: u8 = 0b0000_0010; // CleanSession = 1
    if config.username.is_some() {
        connect_flags |= 0b1000_0000;
    }
    if config.password.is_some() {
        connect_flags |= 0b0100_0000;
    }
    if let Some(will) = will {
        connect_flags |= 0b0000_0100;
        connect_flags |= u8::from(will.qos) << 3;
        if will.retain {
            connect_flags |= 0b0010_0000;
        }
    }

    // Variable header: protocol name + level + flags + keepalive.
    //
    // Keepalive is what makes the will fire on a station that dies without
    // closing its socket — a power cut, a yanked cable, a kernel panic. The
    // broker publishes the will after 1.5 keepalive periods of silence, so
    // this number is the worst-case delay before Home Assistant learns the
    // station is gone. A connection that sends nothing (the stateless publish)
    // is closed before it could ever matter.
    let keepalive = will.map_or(0, |_| KEEPALIVE_SECS);
    let mut var_header = Vec::with_capacity(10);
    var_header.extend_from_slice(&encode_utf8_string("MQTT")?); // Protocol name
    var_header.push(0x04); // Protocol level: 4 = MQTT 3.1.1
    var_header.push(connect_flags);
    var_header.push((keepalive >> 8) as u8);
    var_header.push((keepalive & 0xFF) as u8);

    // Payload, in the order §3.1.3 requires: client ID, will topic, will
    // message, username, password. A broker reads these positionally, so an
    // out-of-order field is not a rejected packet but a will published to
    // whatever the username happened to be.
    let mut payload_bytes = Vec::new();
    payload_bytes.extend_from_slice(&encode_utf8_string(&config.client_id)?);
    if let Some(will) = will {
        payload_bytes.extend_from_slice(&encode_utf8_string(&will.topic)?);
        payload_bytes.extend_from_slice(&encode_binary(&will.payload)?);
    }
    if let Some(ref username) = config.username {
        payload_bytes.extend_from_slice(&encode_utf8_string(username)?);
    }
    if let Some(ref password) = config.password {
        payload_bytes.extend_from_slice(&encode_binary(password.as_bytes())?);
    }

    let remaining_len = var_header.len() + payload_bytes.len();

    let mut packet = Vec::with_capacity(2 + remaining_len);
    packet.push(0x10); // Fixed header: CONNECT (type 1, flags 0)
    encode_remaining_length(&mut packet, remaining_len)?;
    packet.extend_from_slice(&var_header);
    packet.extend_from_slice(&payload_bytes);

    stream.write_all(&packet).map_err(MqttError::Io)
}

// ---------------------------------------------------------------------------
// CONNACK packet (§3.2)
// ---------------------------------------------------------------------------

fn recv_connack<S: Read>(stream: &mut S) -> Result<(), MqttError> {
    let mut buf = [0u8; 4];
    stream
        .read_exact(&mut buf)
        .map_err(|e| MqttError::Connection(format!("did not receive CONNACK: {e}")))?;

    // buf[0] = 0x20 (CONNACK packet type)
    // buf[1] = 0x02 (remaining length)
    // buf[2] = Connect Acknowledge Flags (bit 0 = session present)
    // buf[3] = Connect Return Code

    if buf[0] != 0x20 {
        return Err(MqttError::Connection(format!(
            "expected CONNACK (0x20), got 0x{:02X}",
            buf[0]
        )));
    }

    if buf[3] != 0x00 {
        return Err(MqttError::ConnAck(ConnAckError::from(buf[3])));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// PUBLISH packet (§3.3)
// ---------------------------------------------------------------------------

fn send_publish<S: Read + Write>(
    stream: &mut S,
    topic: &str,
    payload: &[u8],
    retain: bool,
    qos: QosLevel,
) -> Result<(), MqttError> {
    // Fixed header (§3.3.1):
    //   bits 7-4: packet type 3 (PUBLISH)
    //   bit 3:    DUP flag = 0
    //   bits 2-1: QoS
    //   bit 0:    RETAIN
    let fixed_header: u8 = 0x30 | (u8::from(qos) << 1) | u8::from(retain);

    // One message per connection, so the identifier can be a constant. It has
    // to be non-zero (§2.3.1) and it has to match in the PUBACK, which is the
    // only property the acknowledgement check can actually test.
    let packet_id: u16 = 1;

    let topic_bytes = encode_utf8_string(topic)?;
    let id_len = if qos == QosLevel::AtMostOnce { 0 } else { 2 };
    let remaining_len = topic_bytes.len() + id_len + payload.len();

    let mut packet = Vec::with_capacity(2 + remaining_len);
    packet.push(fixed_header);
    encode_remaining_length(&mut packet, remaining_len)?;
    packet.extend_from_slice(&topic_bytes);
    if qos != QosLevel::AtMostOnce {
        packet.push((packet_id >> 8) as u8);
        packet.push((packet_id & 0xFF) as u8);
    }
    packet.extend_from_slice(payload);

    stream.write_all(&packet).map_err(MqttError::Io)?;
    if qos == QosLevel::AtMostOnce {
        return Ok(());
    }
    recv_puback(stream, packet_id)
}

// ---------------------------------------------------------------------------
// PUBACK packet (§3.4)
// ---------------------------------------------------------------------------

/// Wait for the broker to acknowledge a `QoS` 1 publish.
///
/// This is the entire difference between `QoS` 0 and `QoS` 1 for a
/// one-message connection: `Ok` here means the broker has the message, where
/// at `QoS` 0 it means only that the bytes reached the socket.
fn recv_puback<S: Read>(stream: &mut S, expected_id: u16) -> Result<(), MqttError> {
    let mut buf = [0u8; 4];
    stream
        .read_exact(&mut buf)
        .map_err(|e| MqttError::Protocol(format!("did not receive PUBACK: {e}")))?;
    if buf[0] != 0x40 {
        return Err(MqttError::Protocol(format!(
            "expected PUBACK (0x40), got 0x{:02X}",
            buf[0]
        )));
    }
    let got = u16::from(buf[2]) << 8 | u16::from(buf[3]);
    if got != expected_id {
        return Err(MqttError::Protocol(format!(
            "PUBACK acknowledged packet {got}, not {expected_id}"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// PINGREQ / PINGRESP (§3.12, §3.13)
// ---------------------------------------------------------------------------

/// Send a keepalive and wait for the broker's answer.
///
/// The round trip is the point: a PINGREQ that is written and never answered
/// is how a half-open connection — the broker gone, the socket still open on
/// this side, which is what a NAT timeout or a rebooted broker leaves behind
/// — is distinguished from a healthy one. Writing without reading would
/// report every such connection as live.
fn ping<S: Read + Write>(stream: &mut S) -> Result<(), MqttError> {
    stream.write_all(&[0xC0, 0x00]).map_err(MqttError::Io)?;
    let mut buf = [0u8; 2];
    stream
        .read_exact(&mut buf)
        .map_err(|e| MqttError::Protocol(format!("no PINGRESP from the broker: {e}")))?;
    if buf[0] != 0xD0 {
        return Err(MqttError::Protocol(format!(
            "expected PINGRESP (0xD0), got 0x{:02X}",
            buf[0]
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// DISCONNECT packet (§3.14)
// ---------------------------------------------------------------------------

fn send_disconnect<S: Write>(stream: &mut S) -> Result<(), MqttError> {
    // DISCONNECT: fixed header 0xE0, remaining length 0x00
    stream.write_all(&[0xE0, 0x00]).map_err(MqttError::Io)
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

/// Encode a UTF-8 string as a length-prefixed byte sequence (§1.5.3).
fn encode_utf8_string(s: &str) -> Result<Vec<u8>, MqttError> {
    let bytes = s.as_bytes();
    if bytes.len() > 65_535 {
        return Err(MqttError::Encode(format!(
            "string too long: {} bytes (max 65535)",
            bytes.len()
        )));
    }
    // Safe: already checked bytes.len() <= 65_535
    #[allow(clippy::cast_possible_truncation)]
    let len = bytes.len() as u16;
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.push((len >> 8) as u8);
    out.push((len & 0xFF) as u8);
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Encode binary data as a length-prefixed byte sequence (§1.5.6).
fn encode_binary(data: &[u8]) -> Result<Vec<u8>, MqttError> {
    if data.len() > 65_535 {
        return Err(MqttError::Encode(format!(
            "binary field too long: {} bytes (max 65535)",
            data.len()
        )));
    }
    // Safe: already checked data.len() <= 65_535
    #[allow(clippy::cast_possible_truncation)]
    let len = data.len() as u16;
    let mut out = Vec::with_capacity(2 + data.len());
    out.push((len >> 8) as u8);
    out.push((len & 0xFF) as u8);
    out.extend_from_slice(data);
    Ok(out)
}

/// Encode remaining length using MQTT variable-length encoding (§2.2.3).
fn encode_remaining_length(buf: &mut Vec<u8>, mut value: usize) -> Result<(), MqttError> {
    if value > 268_435_455 {
        return Err(MqttError::Encode(format!(
            "remaining length {value} exceeds MQTT maximum (268435455)"
        )));
    }
    loop {
        // Safe: value % 128 is always in 0..127
        #[allow(clippy::cast_possible_truncation)]
        let mut encoded_byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            encoded_byte |= 0x80;
        }
        buf.push(encoded_byte);
        if value == 0 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_utf8_string_empty() {
        let result = encode_utf8_string("").unwrap();
        assert_eq!(result, vec![0x00, 0x00]);
    }

    #[test]
    fn encode_utf8_string_ascii() {
        let result = encode_utf8_string("MQTT").unwrap();
        assert_eq!(result, vec![0x00, 0x04, b'M', b'Q', b'T', b'T']);
    }

    #[test]
    fn encode_utf8_string_too_long_errors() {
        let long = "a".repeat(65_536);
        assert!(encode_utf8_string(&long).is_err());
    }

    #[test]
    fn encode_remaining_length_single_byte() {
        let mut buf = Vec::new();
        encode_remaining_length(&mut buf, 64).unwrap();
        assert_eq!(buf, vec![64]);
    }

    #[test]
    fn encode_remaining_length_two_bytes() {
        // 128 encodes to [0x80, 0x01]
        let mut buf = Vec::new();
        encode_remaining_length(&mut buf, 128).unwrap();
        assert_eq!(buf, vec![0x80, 0x01]);
    }

    #[test]
    fn encode_remaining_length_max() {
        // Maximum 268,435,455 encodes to four bytes
        let mut buf = Vec::new();
        encode_remaining_length(&mut buf, 268_435_455).unwrap();
        assert_eq!(buf.len(), 4);
        assert_eq!(buf, vec![0xFF, 0xFF, 0xFF, 0x7F]);
    }

    #[test]
    fn encode_remaining_length_overflow_errors() {
        let mut buf = Vec::new();
        assert!(encode_remaining_length(&mut buf, 268_435_456).is_err());
    }

    #[test]
    fn encode_binary_correct_length_prefix() {
        let data = b"hello";
        let result = encode_binary(data).unwrap();
        assert_eq!(result[0], 0x00);
        assert_eq!(result[1], 5);
        assert_eq!(&result[2..], b"hello");
    }
}
