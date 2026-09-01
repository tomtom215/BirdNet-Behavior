//! Pure-Rust MQTT 3.1.1 publisher.
//!
//! Implements a stateless, fire-and-forget MQTT publisher using only
//! `std::net::TcpStream`.  No background thread.  No heap allocations
//! beyond packet buffers.  No external MQTT library dependency.
//!
//! ## Protocol subset
//!
//! Only the packets needed for single-message publishing are implemented:
//!
//! | Direction | Packet       | Notes                        |
//! |-----------|-------------|------------------------------|
//! | C → B     | CONNECT      | Variable header + payload    |
//! | B → C     | CONNACK      | Return code checked          |
//! | C → B     | PUBLISH      | `QoS` 0 (no PUBACK required)   |
//! | C → B     | DISCONNECT   | Clean disconnect               |
//!
//! For `QoS` 1 (`AtLeastOnce`) the packet is sent at `QoS` 0 after logging
//! a warning — a conservative degradation rather than a failure.
//!
//! ## Wire format reference
//!
//! MQTT 3.1.1 specification: <http://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html>

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::types::{ConnAckError, MqttConfig, MqttError, TlsConfig};

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
    match tls {
        None => session(stream, config, topic, payload),
        Some(conn) => session(
            rustls::StreamOwned::new(conn, stream),
            config,
            topic,
            payload,
        ),
    }
}

/// The CONNECT → CONNACK → PUBLISH → DISCONNECT exchange, over any transport.
fn session<S: Read + Write>(
    mut stream: S,
    config: &MqttConfig,
    topic: &str,
    payload: &[u8],
) -> Result<(), MqttError> {
    send_connect(&mut stream, config)?;
    recv_connack(&mut stream)?;
    send_publish(&mut stream, topic, payload, config.retain)?;
    send_disconnect(&mut stream)?;
    Ok(())
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
// CONNECT packet (§3.1)
// ---------------------------------------------------------------------------

fn send_connect<S: Write>(stream: &mut S, config: &MqttConfig) -> Result<(), MqttError> {
    // Connect flags byte:
    //   bit 7: Username flag
    //   bit 6: Password flag
    //   bit 2: Clean Session
    let mut connect_flags: u8 = 0b0000_0010; // CleanSession = 1
    if config.username.is_some() {
        connect_flags |= 0b1000_0000;
    }
    if config.password.is_some() {
        connect_flags |= 0b0100_0000;
    }

    // Variable header: protocol name + level + flags + keepalive
    let mut var_header = Vec::with_capacity(10);
    var_header.extend_from_slice(&encode_utf8_string("MQTT")?); // Protocol name
    var_header.push(0x04); // Protocol level: 4 = MQTT 3.1.1
    var_header.push(connect_flags);
    var_header.push(0x00); // Keep-alive MSB (0 = disabled)
    var_header.push(0x3C); // Keep-alive LSB (60 seconds)

    // Payload: client ID, optional username, optional password
    let mut payload_bytes = Vec::new();
    payload_bytes.extend_from_slice(&encode_utf8_string(&config.client_id)?);
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

fn send_publish<S: Write>(
    stream: &mut S,
    topic: &str,
    payload: &[u8],
    retain: bool,
) -> Result<(), MqttError> {
    // Fixed header for QoS 0:
    //   bits 7-4: packet type 3 (PUBLISH)
    //   bit 3:    DUP flag = 0
    //   bits 2-1: QoS = 00
    //   bit 0:    RETAIN
    let retain_bit = u8::from(retain);
    let fixed_header: u8 = 0x30 | retain_bit;

    let topic_bytes = encode_utf8_string(topic)?;
    let remaining_len = topic_bytes.len() + payload.len();

    let mut packet = Vec::with_capacity(2 + remaining_len);
    packet.push(fixed_header);
    encode_remaining_length(&mut packet, remaining_len)?;
    packet.extend_from_slice(&topic_bytes);
    packet.extend_from_slice(payload);

    stream.write_all(&packet).map_err(MqttError::Io)
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
