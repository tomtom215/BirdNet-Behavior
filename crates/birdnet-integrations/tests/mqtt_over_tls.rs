//! MQTT over TLS, against a real TLS server speaking real MQTT bytes.
//!
//! The gate that matters here is not "a TLS connection succeeds" — a client
//! that skipped certificate verification would pass that just as well. It is
//! the pair: the same broker, the same bytes, and the connection **fails**
//! when the client has not been given the certificate to trust.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use birdnet_integrations::mqtt::{MqttConfig, MqttError, TlsConfig, publish};

/// A self-signed certificate and key on disk, for one test.
struct Cert {
    /// Directory holding `cert.pem` and `key.pem`.
    dir: tempfile::TempDir,
}

impl Cert {
    /// Generate a private CA and a `localhost` leaf certificate signed by it.
    ///
    /// Not a bare `openssl req -x509` self-signed certificate: that sets
    /// `basicConstraints=CA:TRUE`, and rustls refuses such a certificate when
    /// it is presented as the *server's* — `CaUsedAsEndEntity`. Since that is
    /// the command every "self-signed cert" recipe on the internet gives, the
    /// documentation says so too.
    fn generate() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = |name: &str| dir.path().join(name);

        run_openssl(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=Test Private CA",
            "-keyout",
            p("ca-key.pem").to_str().unwrap(),
            "-out",
            p("ca.pem").to_str().unwrap(),
        ]);

        run_openssl(&[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-keyout",
            p("key.pem").to_str().unwrap(),
            "-out",
            p("csr.pem").to_str().unwrap(),
        ]);

        std::fs::write(
            p("ext.cnf"),
            "basicConstraints=critical,CA:FALSE\n\
             keyUsage=critical,digitalSignature,keyEncipherment\n\
             extendedKeyUsage=serverAuth\n\
             subjectAltName=DNS:localhost\n",
        )
        .expect("write extensions");

        run_openssl(&[
            "x509",
            "-req",
            "-in",
            p("csr.pem").to_str().unwrap(),
            "-CA",
            p("ca.pem").to_str().unwrap(),
            "-CAkey",
            p("ca-key.pem").to_str().unwrap(),
            "-set_serial",
            "1",
            "-days",
            "1",
            "-extfile",
            p("ext.cnf").to_str().unwrap(),
            "-out",
            p("cert.pem").to_str().unwrap(),
        ]);

        Self { dir }
    }

    /// Generate one self-signed `localhost` certificate with no CA behind it.
    ///
    /// `CA:FALSE` is not optional: `openssl req -x509` defaults to `CA:TRUE`,
    /// and rustls refuses such a certificate when a *server* presents it.
    fn self_signed_leaf() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = |name: &str| dir.path().join(name);
        run_openssl(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-addext",
            "subjectAltName=DNS:localhost",
            "-keyout",
            p("key.pem").to_str().unwrap(),
            "-out",
            p("cert.pem").to_str().unwrap(),
        ]);
        Self { dir }
    }

    /// Path to the CA certificate, which is the trust anchor to configure.
    fn ca_path(&self) -> PathBuf {
        self.dir.path().join("ca.pem")
    }

    /// Path to the PEM certificate, usable as a trust anchor.
    fn cert_path(&self) -> PathBuf {
        self.dir.path().join("cert.pem")
    }

    /// Path to the PEM private key.
    fn key_path(&self) -> PathBuf {
        self.dir.path().join("key.pem")
    }
}

/// Run `openssl` with the given arguments, failing loudly if it is absent.
fn run_openssl(args: &[&str]) {
    let status = std::process::Command::new("openssl")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect(
            "openssl must be available: this test is about certificate verification, \
             and skipping it would leave that unverified",
        );
    assert!(status.success(), "openssl {} failed", args[0]);
}

/// Everything one broker connection received after the CONNACK.
type Received = mpsc::Receiver<Vec<u8>>;

/// Serve one TLS + MQTT connection: CONNACK, then record what follows.
fn tls_broker(cert: &Cert) -> (String, Received) {
    use rustls_pki_types::pem::PemObject as _;

    let certs: Vec<rustls_pki_types::CertificateDer<'static>> =
        rustls_pki_types::CertificateDer::pem_file_iter(cert.cert_path())
            .expect("read cert")
            .collect::<Result<_, _>>()
            .expect("parse cert");
    let key = rustls_pki_types::PrivateKeyDer::from_pem_file(cert.key_path()).expect("read key");

    let config = rustls::ServerConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("protocol versions")
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .expect("server config");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((sock, _)) = listener.accept() else {
            return;
        };
        let Ok(conn) = rustls::ServerConnection::new(config.into()) else {
            return;
        };
        let mut tls = rustls::StreamOwned::new(conn, sock);

        let mut buf = [0_u8; 4096];
        // The client blocks on CONNACK, so read its CONNECT first.
        if tls.read(&mut buf).is_err() {
            return;
        }
        // CONNACK: session-present 0, return code 0 (accepted).
        if tls.write_all(&[0x20, 0x02, 0x00, 0x00]).is_err() || tls.flush().is_err() {
            return;
        }

        let mut rest = Vec::new();
        while let Ok(n) = tls.read(&mut buf) {
            if n == 0 {
                break;
            }
            rest.extend_from_slice(&buf[..n]);
        }
        let _ = tx.send(rest);
    });

    (addr, rx)
}

/// A config pointed at `addr`, with the given trust anchor.
fn config_for(addr: &str, ca_file: Option<&Path>) -> MqttConfig {
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    MqttConfig {
        host: host.to_string(),
        port: port.parse().expect("port"),
        topic_prefix: "birdnet".to_string(),
        timeout_ms: 5_000,
        tls: Some(TlsConfig {
            ca_file: ca_file.map(Path::to_path_buf),
            // The listener is on 127.0.0.1 but the certificate names
            // `localhost`, which is exactly the home-LAN-without-DNS shape
            // `server_name` exists for.
            server_name: Some("localhost".to_string()),
        }),
        ..MqttConfig::default()
    }
}

#[test]
fn a_trusted_broker_certificate_lets_the_publish_through() {
    let cert = Cert::generate();
    let (addr, rx) = tls_broker(&cert);
    let ca = cert.ca_path();

    publish(
        &config_for(&addr, Some(&ca)),
        "birdnet/detection/Strix_aluco",
        br#"{"species":"Strix aluco"}"#,
    )
    .expect("a broker whose certificate we trust must accept the publish");

    let received = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the broker recorded what followed the CONNACK");
    let text = String::from_utf8_lossy(&received);
    assert!(
        text.contains("birdnet/detection/Strix_aluco"),
        "the topic did not arrive: {text:?}"
    );
    assert!(
        text.contains("Strix aluco"),
        "the payload did not arrive: {text:?}"
    );
}

#[test]
fn an_untrusted_broker_certificate_is_refused() {
    // The gate the happy path cannot give: same broker, same bytes, and the
    // only difference is that the client was not given the certificate. A
    // client that skipped verification — or one whose trust anchors silently
    // failed to load and defaulted to "accept" — passes the test above and
    // fails this one.
    let cert = Cert::generate();
    let (addr, _rx) = tls_broker(&cert);

    let err = publish(
        &config_for(&addr, None),
        "birdnet/detection/Strix_aluco",
        b"{}",
    )
    .expect_err("a self-signed certificate is not in the platform trust store");

    // It must fail *because of the certificate*, not because of some unrelated
    // I/O problem that would mask a real verification bug.
    let rendered = err.to_string().to_lowercase();
    assert!(
        rendered.contains("certificate")
            || rendered.contains("unknownissuer")
            || rendered.contains("unknown issuer")
            || rendered.contains("invalid peer"),
        "failed, but not visibly on certificate verification: {err}"
    );
}

#[test]
fn a_certificate_that_does_not_name_the_server_is_refused() {
    // Counterpart on the other half of verification: trusting the certificate
    // is not the same as it being the right one for this host. Without the
    // name check, any certificate this station trusts for any purpose would
    // authenticate any broker.
    let cert = Cert::generate();
    let (addr, _rx) = tls_broker(&cert);
    let ca = cert.ca_path();

    let mut config = config_for(&addr, Some(&ca));
    if let Some(tls) = config.tls.as_mut() {
        tls.server_name = Some("not-the-broker.example".to_string());
    }

    let err = publish(&config, "birdnet/x", b"{}").expect_err("the name does not match");
    let rendered = err.to_string().to_lowercase();
    assert!(
        rendered.contains("name") || rendered.contains("certificate"),
        "failed, but not visibly on the name check: {err}"
    );
}

#[test]
fn a_ca_file_that_is_not_a_certificate_is_reported_by_path() {
    // The operator's next action is to look at the file, so the error has to
    // name it — and must not quote its contents, which for a misdirected path
    // could be a private key.
    let dir = tempfile::tempdir().expect("tempdir");
    let bogus = dir.path().join("not-a-cert.pem");
    std::fs::write(&bogus, "-----BEGIN PRIVATE KEY-----\nSUPERSECRET\n").expect("write");

    let err = publish(&config_for("127.0.0.1:1", Some(&bogus)), "birdnet/x", b"{}")
        .expect_err("an unusable CA file must fail before any connection");

    assert!(matches!(err, MqttError::Tls(_)), "{err:?}");
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("not-a-cert.pem"), "{rendered}");
    assert!(!rendered.contains("SUPERSECRET"), "{rendered}");
}

#[test]
fn a_missing_ca_file_is_reported_by_path() {
    let err = publish(
        &config_for("127.0.0.1:1", Some(Path::new("/nonexistent/ca.pem"))),
        "birdnet/x",
        b"{}",
    )
    .expect_err("a missing CA file must fail before any connection");
    assert!(matches!(err, MqttError::Tls(_)), "{err:?}");
    assert!(err.to_string().contains("/nonexistent/ca.pem"), "{err}");
}

#[test]
fn a_plaintext_config_still_publishes_over_plain_tcp() {
    // Counterpart to every gate above: adding TLS must not have made the
    // ordinary Mosquitto-on-the-LAN case require it.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        let mut buf = [0_u8; 4096];
        if sock.read(&mut buf).is_err() {
            return;
        }
        if sock.write_all(&[0x20, 0x02, 0x00, 0x00]).is_err() {
            return;
        }
        let mut rest = Vec::new();
        while let Ok(n) = sock.read(&mut buf) {
            if n == 0 {
                break;
            }
            rest.extend_from_slice(&buf[..n]);
        }
        let _ = tx.send(rest);
    });

    let (host, port) = addr.rsplit_once(':').expect("host:port");
    publish(
        &MqttConfig {
            host: host.to_string(),
            port: port.parse().expect("port"),
            timeout_ms: 5_000,
            tls: None,
            ..MqttConfig::default()
        },
        "birdnet/detection/Parus_major",
        b"{}",
    )
    .expect("plaintext publishing must still work");

    let received = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the broker recorded the publish");
    assert!(
        String::from_utf8_lossy(&received).contains("birdnet/detection/Parus_major"),
        "the topic did not arrive"
    );
}

#[test]
fn a_ca_signed_leaf_is_not_its_own_trust_anchor() {
    // Trusting the *leaf* of a CA-signed chain does not work, and the error is
    // `UnknownIssuer` — which reads like "you configured the wrong file" and
    // is easy to misdiagnose as a broken CA file. The setting has to name the
    // certificate that signed the broker's, not the broker's own.
    //
    // This was written the other way round first, asserting that a leaf works,
    // because the `ca_file` documentation said so. It does not.
    let cert = Cert::generate();
    let (addr, _rx) = tls_broker(&cert);
    let leaf = cert.cert_path();

    let err = publish(&config_for(&addr, Some(&leaf)), "birdnet/x", b"{}")
        .expect_err("a CA-signed leaf does not verify against itself");
    assert!(
        err.to_string().to_lowercase().contains("unknownissuer"),
        "{err}"
    );
}

#[test]
fn a_self_signed_broker_certificate_works_when_the_operator_trusts_it() {
    // The other shape, and the one most home brokers actually have: a single
    // self-signed certificate with no CA behind it. Pointing `ca_file` at that
    // certificate does verify — it signed itself, so the chain terminates at
    // the anchor.
    //
    // It must still be generated with `CA:FALSE`. `openssl req -x509` alone
    // sets `CA:TRUE`, and rustls then refuses it as a server certificate with
    // `CaUsedAsEndEntity` — which is the first thing this test found.
    let cert = Cert::self_signed_leaf();
    let (addr, rx) = tls_broker(&cert);
    let own = cert.cert_path();

    publish(&config_for(&addr, Some(&own)), "birdnet/x", b"{}")
        .expect("a self-signed broker certificate the operator trusts must verify");

    let received = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the broker recorded the publish");
    assert!(String::from_utf8_lossy(&received).contains("birdnet/x"));
}
