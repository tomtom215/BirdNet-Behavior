//! MQTT mutual TLS (`P-10`): a broker that demands a client certificate, real
//! TLS, real MQTT bytes.
//!
//! The pair that matters, as in `mqtt_over_tls.rs`: the same broker accepts the
//! publish when the client presents a certificate its CA signed, and refuses
//! the client that presents none, one from another CA, or a key that is not
//! the certificate's. A client that never sent its certificate would pass the
//! first alone.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use birdnet_integrations::mqtt::{MqttConfig, MqttError, TlsConfig, check_tls, publish};

/// One CA, a `localhost` server leaf, and a client leaf, on disk.
struct Pki {
    dir: tempfile::TempDir,
}

impl Pki {
    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A private CA called `ca_name`, with files `<ca_name>.pem` / `<ca_name>-key.pem`.
    fn make_ca(&self, ca_name: &str) {
        run_openssl(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            &format!("/CN={ca_name}"),
            "-keyout",
            self.path(&format!("{ca_name}-key.pem")).to_str().unwrap(),
            "-out",
            self.path(&format!("{ca_name}.pem")).to_str().unwrap(),
        ]);
    }

    /// A leaf `<name>.pem` / `<name>-key.pem` signed by `ca_name`.
    fn make_leaf(&self, name: &str, ca_name: &str, eku: &str, san: &str) {
        run_openssl(&[
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            &format!("/CN={name}"),
            "-keyout",
            self.path(&format!("{name}-key.pem")).to_str().unwrap(),
            "-out",
            self.path(&format!("{name}.csr")).to_str().unwrap(),
        ]);
        std::fs::write(
            self.path(&format!("{name}.cnf")),
            format!(
                "basicConstraints=critical,CA:FALSE\n\
                 keyUsage=critical,digitalSignature,keyEncipherment\n\
                 extendedKeyUsage={eku}\n\
                 subjectAltName={san}\n"
            ),
        )
        .expect("extensions");
        run_openssl(&[
            "x509",
            "-req",
            "-in",
            self.path(&format!("{name}.csr")).to_str().unwrap(),
            "-CA",
            self.path(&format!("{ca_name}.pem")).to_str().unwrap(),
            "-CAkey",
            self.path(&format!("{ca_name}-key.pem")).to_str().unwrap(),
            "-set_serial",
            "7",
            "-days",
            "1",
            "-extfile",
            self.path(&format!("{name}.cnf")).to_str().unwrap(),
            "-out",
            self.path(&format!("{name}.pem")).to_str().unwrap(),
        ]);
    }

    /// The fixture: CA `ca`, server `server`, client `client` signed by `ca`,
    /// and a second CA `other` with its own client `stranger`.
    fn generate() -> Self {
        let pki = Self {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        pki.make_ca("ca");
        pki.make_leaf("server", "ca", "serverAuth", "DNS:localhost");
        pki.make_leaf("client", "ca", "clientAuth", "DNS:birdnet-station");
        pki.make_ca("other");
        pki.make_leaf("stranger", "other", "clientAuth", "DNS:stranger");
        pki
    }
}

fn run_openssl(args: &[&str]) {
    let status = std::process::Command::new("openssl")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("openssl must be available: this test is about certificate verification");
    assert!(status.success(), "openssl {} failed", args[0]);
}

/// What the broker saw of one connection.
#[derive(Debug)]
struct Seen {
    /// The CN-bearing leaf certificate the client presented, as DER, if any.
    client_cert_der: Option<Vec<u8>>,
    /// Bytes after the CONNACK.
    rest: Vec<u8>,
}

/// A TLS+MQTT broker that **requires** a client certificate signed by `ca`.
fn mutual_tls_broker(pki: &Pki) -> (String, mpsc::Receiver<Seen>) {
    use rustls_pki_types::pem::PemObject as _;

    let certs: Vec<rustls_pki_types::CertificateDer<'static>> =
        rustls_pki_types::CertificateDer::pem_file_iter(pki.path("server.pem"))
            .expect("read")
            .collect::<Result<_, _>>()
            .expect("parse");
    let key =
        rustls_pki_types::PrivateKeyDer::from_pem_file(pki.path("server-key.pem")).expect("key");

    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_pki_types::CertificateDer::pem_file_iter(pki.path("ca.pem")).expect("ca") {
        roots.add(c.expect("ca cert")).expect("add root");
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        provider.clone(),
    )
    .build()
    .expect("client verifier");
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .expect("server config");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr").to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((sock, _)) = listener.accept() else {
            return;
        };
        let Ok(conn) = rustls::ServerConnection::new(Arc::new(config)) else {
            return;
        };
        let mut tls = rustls::StreamOwned::new(conn, sock);
        let mut buf = [0_u8; 4096];
        // The handshake completes (or is refused) on the first read.
        if tls.read(&mut buf).is_err() {
            return;
        }
        let client_cert_der = tls
            .conn
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|c| c.as_ref().to_vec());
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
        let _ = tx.send(Seen {
            client_cert_der,
            rest,
        });
    });
    (addr, rx)
}

fn config_for(addr: &str, pki: &Pki, client: Option<(&str, &str)>) -> MqttConfig {
    let (host, port) = addr.rsplit_once(':').expect("host:port");
    MqttConfig {
        host: host.to_string(),
        port: port.parse().expect("port"),
        topic_prefix: "birdnet".to_string(),
        timeout_ms: 5_000,
        tls: Some(TlsConfig {
            ca_file: Some(pki.path("ca.pem")),
            server_name: Some("localhost".to_string()),
            client_cert: client.map(|(c, _)| pki.path(c)),
            client_key: client.map(|(_, k)| pki.path(k)),
        }),
        ..MqttConfig::default()
    }
}

fn der_of(path: &Path) -> Vec<u8> {
    use rustls_pki_types::pem::PemObject as _;
    rustls_pki_types::CertificateDer::pem_file_iter(path)
        .expect("read")
        .next()
        .expect("one cert")
        .expect("parse")
        .as_ref()
        .to_vec()
}

#[test]
fn a_broker_that_demands_a_client_certificate_gets_ours_and_takes_the_publish() {
    let pki = Pki::generate();
    let (addr, rx) = mutual_tls_broker(&pki);
    publish(
        &config_for(&addr, &pki, Some(("client.pem", "client-key.pem"))),
        "birdnet/detection/Strix_aluco",
        b"{}",
    )
    .expect("a client certificate the broker's CA signed must be accepted");
    let seen = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the broker saw the connection");
    assert_eq!(
        seen.client_cert_der.as_deref(),
        Some(der_of(&pki.path("client.pem")).as_slice()),
        "the broker saw a certificate, but not the one configured"
    );
    assert!(
        String::from_utf8_lossy(&seen.rest).contains("birdnet/detection/Strix_aluco"),
        "the publish did not arrive"
    );
}

#[test]
fn without_a_client_certificate_the_same_broker_refuses() {
    // The discrimination: identical broker and bytes, no identity configured.
    // A client that always sent a certificate (or a broker that never asked)
    // passes the test above and fails this one.
    let pki = Pki::generate();
    let (addr, _rx) = mutual_tls_broker(&pki);
    let err = publish(&config_for(&addr, &pki, None), "birdnet/x", b"{}")
        .expect_err("the broker requires a client certificate");
    assert!(
        matches!(
            err,
            MqttError::Io(_) | MqttError::Tls(_) | MqttError::Connection(_)
        ),
        "{err:?}"
    );
}

#[test]
fn a_client_certificate_from_another_ca_is_refused() {
    let pki = Pki::generate();
    let (addr, _rx) = mutual_tls_broker(&pki);
    publish(
        &config_for(&addr, &pki, Some(("stranger.pem", "stranger-key.pem"))),
        "birdnet/x",
        b"{}",
    )
    .expect_err("a certificate the broker's CA did not sign must not be accepted");
}

#[test]
fn a_certificate_with_someone_elses_key_is_refused_before_connecting() {
    // Cert and key from different pairs. Caught while building the session, so
    // the operator is told about the files and not about a handshake failure.
    let pki = Pki::generate();
    let err = publish(
        &config_for(
            "127.0.0.1:1",
            &pki,
            Some(("client.pem", "stranger-key.pem")),
        ),
        "birdnet/x",
        b"{}",
    )
    .expect_err("mismatched pair");
    let MqttError::Tls(msg) = err else {
        panic!("expected a TLS configuration error, got {err:?}");
    };
    assert!(msg.contains("do not match"), "{msg}");
}

#[test]
fn half_an_identity_is_refused_and_names_the_file_that_is_there() {
    let pki = Pki::generate();
    let mut cfg = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    cfg.tls.as_mut().unwrap().client_key = None;
    let err = publish(&cfg, "birdnet/x", b"{}").expect_err("a certificate without a key");
    assert!(matches!(err, MqttError::Tls(_)), "{err:?}");
    assert!(err.to_string().contains("client.pem"), "{err}");
    assert!(err.to_string().contains("private key"), "{err}");

    let mut cfg = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    cfg.tls.as_mut().unwrap().client_cert = None;
    let err = publish(&cfg, "birdnet/x", b"{}").expect_err("a key without a certificate");
    assert!(err.to_string().contains("client-key.pem"), "{err}");
}

#[test]
fn an_unreadable_or_wrong_file_is_named_and_its_contents_are_not_quoted() {
    let pki = Pki::generate();
    let bogus = pki.path("notes.txt");
    std::fs::write(&bogus, "hunter2-SUPERSECRET").expect("write");

    // A key file that holds no key.
    let mut cfg = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    cfg.tls.as_mut().unwrap().client_key = Some(bogus.clone());
    let err = publish(&cfg, "birdnet/x", b"{}").expect_err("no key in the file");
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("notes.txt"), "{rendered}");
    assert!(!rendered.contains("SUPERSECRET"), "{rendered}");

    // A certificate file that holds no certificate.
    let mut cfg = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    cfg.tls.as_mut().unwrap().client_cert = Some(bogus);
    let err = publish(&cfg, "birdnet/x", b"{}").expect_err("no certificate in the file");
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("notes.txt"), "{rendered}");
    assert!(!rendered.contains("SUPERSECRET"), "{rendered}");

    // And one that does not exist.
    let mut cfg = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    cfg.tls.as_mut().unwrap().client_cert = Some(PathBuf::from("/nonexistent/client.pem"));
    let err = publish(&cfg, "birdnet/x", b"{}").expect_err("missing file");
    assert!(err.to_string().contains("/nonexistent/client.pem"), "{err}");
}

#[test]
fn the_doctor_check_agrees_with_a_real_connection() {
    // `check_tls` is what `--doctor` runs. It must accept exactly what a
    // publish gets past, and refuse what it refuses.
    let pki = Pki::generate();
    let ok = config_for("127.0.0.1:1", &pki, Some(("client.pem", "client-key.pem")));
    check_tls(ok.tls.as_ref().unwrap(), "localhost").expect("a good identity passes the check");

    let bad = config_for(
        "127.0.0.1:1",
        &pki,
        Some(("client.pem", "stranger-key.pem")),
    );
    check_tls(bad.tls.as_ref().unwrap(), "localhost").expect_err("a mismatched pair fails it");

    let none = config_for("127.0.0.1:1", &pki, None);
    check_tls(none.tls.as_ref().unwrap(), "localhost")
        .expect("no identity is still a valid TLS config");
}
