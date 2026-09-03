//! The MQTT last will, against a broker that decodes what it is sent.
//!
//! Home Assistant discovery has always advertised a `binary_sensor` with
//! `device_class: connectivity` on `{prefix}/status`. Nothing published there
//! and nothing registered a will, so the entity was permanently *unknown* and
//! "notify me when the station goes offline" could not be built.
//!
//! These gates assert on *decoded* CONNECT and PUBLISH packets rather than on
//! byte offsets, because the failure that matters is semantic: §3.1.3 lays the
//! CONNECT payload out positionally, so a will written after the username is a
//! perfectly well-formed packet that publishes the station's password to
//! whatever topic the broker read next.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

use birdnet_integrations::mqtt::{
    MqttConfig, MqttError, PRESENCE_OFFLINE, PRESENCE_ONLINE, PresenceSession, QosLevel, publish,
    publish_with,
};

// ── a broker that parses ────────────────────────────────────────────────

/// A decoded CONNECT (§3.1).
#[derive(Debug, Clone)]
struct Connect {
    client_id: String,
    keepalive: u16,
    username: Option<String>,
    password: Option<String>,
    will: Option<Will>,
}

/// The will as the broker reads it back out of the CONNECT payload.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Will {
    topic: String,
    payload: Vec<u8>,
    qos: u8,
    retain: bool,
}

/// A decoded PUBLISH (§3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Publish {
    topic: String,
    payload: Vec<u8>,
    qos: u8,
    retain: bool,
    packet_id: Option<u16>,
}

/// Everything the broker saw after the CONNECT.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Packet {
    Publish(Publish),
    PingReq,
    Disconnect,
}

/// How the stub broker should behave.
#[derive(Debug, Clone, Copy)]
struct Behaviour {
    /// Send PUBACK for QoS-1 publishes.
    ack_publishes: bool,
    /// Send PINGRESP for PINGREQ.
    answer_pings: bool,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            ack_publishes: true,
            answer_pings: true,
        }
    }
}

/// Read one length-prefixed field, returning it and the bytes consumed.
fn read_field(buf: &[u8], at: usize) -> Option<(Vec<u8>, usize)> {
    let len = usize::from(*buf.get(at)?) << 8 | usize::from(*buf.get(at + 1)?);
    let end = at + 2 + len;
    Some((buf.get(at + 2..end)?.to_vec(), end))
}

/// Decode a CONNECT body exactly as §3.1.3 orders it.
fn parse_connect(body: &[u8]) -> Connect {
    let (proto, mut at) = read_field(body, 0).expect("protocol name");
    assert_eq!(proto, b"MQTT", "protocol name");
    assert_eq!(body[at], 0x04, "protocol level 4 = MQTT 3.1.1");
    let flags = body[at + 1];
    let keepalive = u16::from(body[at + 2]) << 8 | u16::from(body[at + 3]);
    at += 4;

    let (client_id, next) = read_field(body, at).expect("client id");
    at = next;

    let will = if flags & 0b0000_0100 == 0 {
        None
    } else {
        let (topic, next) = read_field(body, at).expect("will topic");
        let (payload, next) = read_field(body, next).expect("will payload");
        at = next;
        Some(Will {
            topic: String::from_utf8(topic).expect("utf8 will topic"),
            payload,
            qos: (flags >> 3) & 0b11,
            retain: flags & 0b0010_0000 != 0,
        })
    };

    let username = if flags & 0b1000_0000 == 0 {
        None
    } else {
        let (u, next) = read_field(body, at).expect("username");
        at = next;
        Some(String::from_utf8(u).expect("utf8 username"))
    };
    let password = if flags & 0b0100_0000 == 0 {
        None
    } else {
        let (p, _) = read_field(body, at).expect("password");
        Some(String::from_utf8(p).expect("utf8 password"))
    };

    Connect {
        client_id: String::from_utf8(client_id).expect("utf8 client id"),
        keepalive,
        username,
        password,
        will,
    }
}

/// Read a fixed header plus its variable-length body (§2.2.3).
fn read_packet(sock: &mut TcpStream) -> Option<(u8, Vec<u8>)> {
    let mut header = [0u8; 1];
    sock.read_exact(&mut header).ok()?;
    let mut remaining = 0usize;
    let mut multiplier = 1usize;
    loop {
        let mut b = [0u8; 1];
        sock.read_exact(&mut b).ok()?;
        remaining += usize::from(b[0] & 0x7F) * multiplier;
        if b[0] & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    let mut body = vec![0u8; remaining];
    sock.read_exact(&mut body).ok()?;
    Some((header[0], body))
}

/// Serve one connection, decoding what arrives and answering per `behaviour`.
///
/// Returns the address to connect to plus a receiver of `(Connect, packets)`.
#[allow(clippy::type_complexity)]
fn broker(behaviour: Behaviour) -> (String, mpsc::Receiver<(Connect, Vec<Packet>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .ok();

        let Some((ty, body)) = read_packet(&mut sock) else {
            return;
        };
        assert_eq!(ty & 0xF0, 0x10, "first packet must be CONNECT");
        let connect = parse_connect(&body);
        if sock.write_all(&[0x20, 0x02, 0x00, 0x00]).is_err() {
            return;
        }

        let mut seen = Vec::new();
        while let Some((ty, body)) = read_packet(&mut sock) {
            match ty & 0xF0 {
                0x30 => {
                    let qos = (ty >> 1) & 0b11;
                    let retain = ty & 1 == 1;
                    let (topic, at) = read_field(&body, 0).expect("publish topic");
                    let (packet_id, at) = if qos == 0 {
                        (None, at)
                    } else {
                        (
                            Some(u16::from(body[at]) << 8 | u16::from(body[at + 1])),
                            at + 2,
                        )
                    };
                    seen.push(Packet::Publish(Publish {
                        topic: String::from_utf8(topic).expect("utf8 topic"),
                        payload: body[at..].to_vec(),
                        qos,
                        retain,
                        packet_id,
                    }));
                    if qos > 0 && behaviour.ack_publishes {
                        let id = packet_id.unwrap_or(0);
                        #[allow(clippy::cast_possible_truncation)]
                        let ack = [0x40, 0x02, (id >> 8) as u8, (id & 0xFF) as u8];
                        if sock.write_all(&ack).is_err() {
                            break;
                        }
                    }
                }
                0xC0 => {
                    seen.push(Packet::PingReq);
                    if behaviour.answer_pings && sock.write_all(&[0xD0, 0x00]).is_err() {
                        break;
                    }
                }
                0xE0 => {
                    seen.push(Packet::Disconnect);
                    break;
                }
                other => panic!("unexpected packet type 0x{other:02X}"),
            }
        }
        let _ = tx.send((connect, seen));
    });

    (addr, rx)
}

/// A config pointed at `addr`, with a short timeout so a wrong expectation
/// fails the test rather than hanging it.
fn config_for(addr: &str) -> MqttConfig {
    let (host, port) = addr.split_once(':').expect("host:port");
    MqttConfig {
        host: host.to_owned(),
        port: port.parse().expect("port"),
        topic_prefix: "garden".to_owned(),
        client_id: "station-presence".to_owned(),
        timeout_ms: 3_000,
        ..MqttConfig::default()
    }
}

fn publishes(packets: &[Packet]) -> Vec<&Publish> {
    packets
        .iter()
        .filter_map(|p| match p {
            Packet::Publish(pu) => Some(pu),
            _ => None,
        })
        .collect()
}

// ── the will ────────────────────────────────────────────────────────────

#[test]
fn the_presence_connect_registers_the_will_the_status_entity_needs() {
    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);

    let session = PresenceSession::connect(&cfg).expect("presence session connects");
    drop(session);

    let (connect, packets) = rx.recv().expect("broker reported");
    let will = connect.will.expect("a will was registered");

    assert_eq!(
        will.topic, "garden/status",
        "the will must go to the topic HA discovery advertises"
    );
    assert_eq!(
        will.payload, PRESENCE_OFFLINE,
        "the payload must be what the binary_sensor's payload_off matches"
    );
    assert_eq!(will.qos, 1, "a will is published once, unretriable");
    assert!(
        will.retain,
        "unretained, a Home Assistant that restarts after the station died sees \
         nothing at all and shows the entity as unknown rather than offline"
    );
    assert_eq!(
        connect.client_id, "station-presence",
        "the identifier the broker sees is the one it will kick off if a second \
         connection claims it"
    );
    assert_eq!(
        connect.keepalive, 30,
        "the keepalive is what makes the will fire on a station that dies without \
         closing its socket"
    );

    let published = publishes(&packets);
    assert_eq!(published.len(), 1, "only the online notice: {packets:?}");
    assert_eq!(published[0].topic, "garden/status");
    assert_eq!(published[0].payload, PRESENCE_ONLINE);
    assert!(published[0].retain, "the online notice is retained too");
}

#[test]
fn a_stateless_publish_registers_no_will() {
    // The discrimination. Setting the will on every CONNECT would pass the
    // test above and be wrong: §3.14 has the broker *discard* a will when the
    // client disconnects cleanly, which every stateless publish does. The
    // result would be a will that never fires, and the appearance of one that
    // does.
    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);

    publish(&cfg, "garden/detection/Blackbird", b"{}").expect("publish");

    let (connect, packets) = rx.recv().expect("broker reported");
    assert!(
        connect.will.is_none(),
        "a connection that ends in DISCONNECT cannot carry a meaningful will"
    );
    assert_eq!(
        connect.keepalive, 0,
        "and needs no keepalive: it is closed before one could elapse"
    );
    assert_eq!(packets.last(), Some(&Packet::Disconnect));
}

#[test]
fn credentials_survive_a_will_in_the_same_connect() {
    // §3.1.3 is positional: client id, will topic, will message, username,
    // password. A will written after the username is a well-formed packet
    // that publishes the password to whatever the broker read as the topic.
    let (addr, rx) = broker(Behaviour::default());
    let cfg = MqttConfig {
        username: Some("station".to_owned()),
        password: Some("hunter2".to_owned()),
        ..config_for(&addr)
    };

    let session = PresenceSession::connect(&cfg).expect("connects");
    drop(session);

    let (connect, _) = rx.recv().expect("broker reported");
    assert_eq!(connect.username.as_deref(), Some("station"));
    assert_eq!(connect.password.as_deref(), Some("hunter2"));
    let will = connect.will.expect("will");
    assert_eq!(will.topic, "garden/status");
    assert_eq!(will.payload, PRESENCE_OFFLINE);
}

#[test]
fn a_planned_stop_says_offline_and_disconnects() {
    // Without this the station looks online for up to 1.5 keepalive periods
    // after it has already exited, and an operator restarting the service
    // watches Home Assistant report a station that is not running as running.
    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);

    let session = PresenceSession::connect(&cfg).expect("connects");
    session.shutdown().expect("clean shutdown");

    let (_, packets) = rx.recv().expect("broker reported");
    let published = publishes(&packets);
    assert_eq!(published.len(), 2, "online then offline: {packets:?}");
    assert_eq!(published[1].topic, "garden/status");
    assert_eq!(published[1].payload, PRESENCE_OFFLINE);
    assert!(published[1].retain);
    assert_eq!(
        packets.last(),
        Some(&Packet::Disconnect),
        "DISCONNECT last, so the broker discards the will and does not publish \
         a second offline after ours"
    );
}

// ── keepalive ───────────────────────────────────────────────────────────

#[test]
fn a_keepalive_that_is_not_answered_is_a_dead_session() {
    // A half-open connection — the broker rebooted, a NAT dropped the mapping
    // — accepts writes indefinitely. A ping that is written and not read back
    // would report every such connection as live, and the station would go on
    // believing it was reachable.
    let (addr, rx) = broker(Behaviour {
        answer_pings: false,
        ..Behaviour::default()
    });
    let cfg = config_for(&addr);

    let mut session = PresenceSession::connect(&cfg).expect("connects");
    let err = session
        .keepalive()
        .expect_err("an unanswered ping must fail");
    assert!(
        matches!(err, MqttError::Protocol(_)),
        "reported as a protocol failure, not a silent success: {err}"
    );
    drop(session);
    let (_, packets) = rx.recv().expect("broker reported");
    assert!(packets.contains(&Packet::PingReq), "{packets:?}");
}

#[test]
fn an_answered_keepalive_keeps_the_session() {
    // Counterpart: the check must not be "pings always fail".
    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);

    let mut session = PresenceSession::connect(&cfg).expect("connects");
    session.keepalive().expect("answered ping succeeds");
    session.keepalive().expect("and again");
    drop(session);

    let (_, packets) = rx.recv().expect("broker reported");
    assert_eq!(
        packets.iter().filter(|p| **p == Packet::PingReq).count(),
        2,
        "{packets:?}"
    );
}

// ── QoS ─────────────────────────────────────────────────────────────────

#[test]
fn a_qos_1_publish_waits_for_the_brokers_acknowledgement() {
    // `MqttConfig::qos` had no reader anywhere in the workspace: a station
    // configured for QoS 1 silently got QoS 0, where "the broker never
    // received it" and "the broker has it" are the same return value.
    let (addr, rx) = broker(Behaviour {
        ack_publishes: false,
        ..Behaviour::default()
    });
    let cfg = MqttConfig {
        qos: QosLevel::AtLeastOnce,
        ..config_for(&addr)
    };

    let err = publish(&cfg, "garden/detection/Blackbird", b"{}")
        .expect_err("an unacknowledged QoS 1 publish must not report success");
    assert!(matches!(err, MqttError::Protocol(_)), "{err}");

    let (_, packets) = rx.recv().expect("broker reported");
    let published = publishes(&packets);
    assert_eq!(published[0].qos, 1, "sent at QoS 1: {packets:?}");
    assert!(
        published[0].packet_id.is_some(),
        "and carries the packet identifier the PUBACK has to match"
    );
}

#[test]
fn a_qos_0_publish_expects_nothing_back() {
    // Counterpart, and the reason the default is unchanged: against the same
    // silent broker, QoS 0 must still succeed. Otherwise the fix would be
    // "every publish now blocks for an acknowledgement", which on a station
    // whose broker never acknowledges is a detection pipeline that stalls.
    let (addr, rx) = broker(Behaviour {
        ack_publishes: false,
        ..Behaviour::default()
    });
    let cfg = config_for(&addr);
    assert_eq!(cfg.qos, QosLevel::AtMostOnce, "the default");

    publish(&cfg, "garden/detection/Blackbird", b"{}").expect("QoS 0 does not wait");

    let (_, packets) = rx.recv().expect("broker reported");
    let published = publishes(&packets);
    assert_eq!(published[0].qos, 0);
    assert!(published[0].packet_id.is_none(), "no identifier at QoS 0");
}

// ── retain ──────────────────────────────────────────────────────────────

#[test]
fn a_retain_override_does_not_disturb_the_stations_own_setting() {
    // Home Assistant builds its entity list from what the broker replays when
    // HA starts, so a discovery config published unretained is delivered once
    // and then gone: every HA restart lost all four entities until the station
    // was restarted too. The override exists for that, and must not leak into
    // the detection stream, where retaining is the operator's choice.
    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);
    assert!(!cfg.retain, "the station's own setting, unchanged");

    publish_with(&cfg, "homeassistant/sensor/x/config", b"{}", true).expect("published");
    let (_, packets) = rx.recv().expect("broker reported");
    assert!(publishes(&packets)[0].retain, "override honoured");

    let (addr, rx) = broker(Behaviour::default());
    let cfg = config_for(&addr);
    publish(&cfg, "garden/detection/Blackbird", b"{}").expect("published");
    let (_, packets) = rx.recv().expect("broker reported");
    assert!(
        !publishes(&packets)[0].retain,
        "a detection still follows the station's setting"
    );
}
