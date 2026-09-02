//! HTTPS for the dashboard's own listener.
//!
//! Until this module existed the server spoke plain HTTP and nothing else, and
//! the documentation told the operator to put Caddy or nginx in front of it.
//! That is a correct answer and a bad default: it is a second daemon, a second
//! config file and a second thing to get wrong, on a box whose whole point is
//! that it is one binary you leave in a field for a year.
//!
//! # Three modes, and what each is for
//!
//! | `--tls-mode`  | Certificate | Use it when |
//! |---------------|-------------|-------------|
//! | `off`         | —           | LAN-only, or something else already terminates TLS. The default; nothing changes. |
//! | `self-signed` | generated here, kept on disk | You want the LAN traffic encrypted and are willing to click through the browser warning once. |
//! | `manual`      | yours, from disk | You have a real certificate — from your own ACME client, an internal CA, or a purchase. |
//!
//! There is deliberately **no ACME client here**. Fetching and renewing a
//! publicly-trusted certificate needs a reachable name, an open port 80 or a
//! DNS credential, and an account key to look after; a station on a home LAN
//! usually has none of those. `manual` mode plus the reload below is the
//! supported path for anyone who does: point `certbot` at the files and this
//! picks up the renewal without a restart.
//!
//! # Why a certificate *resolver* rather than a fixed certificate
//!
//! `rustls::ServerConfig::with_single_cert` bakes the keypair into the config
//! at startup, so a certificate renewed on disk at 03:00 is not served until
//! somebody restarts the process. Since the common `manual` deployment is
//! exactly "an ACME client rewrites these two files every 60 days", that is the
//! failure mode to design out, not to document. [`Resolver`] holds the keypair
//! behind an `RwLock` and [`spawn_reloader`] swaps it when the files change on
//! disk, so a renewal is picked up by the next handshake.
//!
//! # Why the accept loop is written out by hand
//!
//! `axum::serve` takes a [`TcpListener`] and offers no seam for a TLS acceptor,
//! so [`serve`] runs the loop itself over `hyper-util`'s auto (h1 + h2)
//! builder. Two things that fall out of `axum::serve` for free have to be done
//! explicitly here, and both are load-bearing:
//!
//! * **`ConnectInfo`.** The per-IP rate limiter reads the peer address out of
//!   request extensions. `axum::serve` puts it there via
//!   `into_make_service_with_connect_info`; this loop inserts it per request.
//!   Without it every TLS request looks like it came from nowhere and the
//!   limiter's per-IP bucket silently becomes a single global one.
//! * **Upgrades.** The dashboard holds a `/ws/detections` socket open and the
//!   spectrogram page another. `serve_connection` alone drops the upgrade, so
//!   this uses `serve_connection_with_upgrades`.

use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::ConnectInfo;
use rustls::ServerConfig;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls_pki_types::pem::PemObject as _;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::Service as _;

/// Filename of the local CA certificate — the one file an operator imports.
const CA_CERT: &str = "local-ca.crt";
/// Filename of the local CA's private key.
const CA_KEY: &str = "local-ca.key";
/// Filename of the served chain: the leaf certificate followed by the CA.
const LEAF_CERT: &str = "server.crt";
/// Filename of the leaf's private key.
const LEAF_KEY: &str = "server.key";
/// Filename of the sidecar recording what the generated material covers and
/// when each half expires. See [`SelfSignedMeta`] for why this is not read back
/// off the certificates themselves.
const META: &str = "server.meta";

/// How long the local CA is valid for.
///
/// Much longer than the leaf on purpose: the CA is the file a person imports
/// into a browser or OS trust store, and making them redo that on every leaf
/// rotation is exactly the friction this mode exists to remove.
const CA_VALIDITY_DAYS: u32 = 3650;

/// Regenerate a self-signed certificate once it has this little life left.
///
/// Renewing early means a station that is only powered up occasionally still
/// rotates before the browser starts refusing it, rather than at the exact
/// moment somebody needs the dashboard.
const RENEW_BEFORE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Backdate a generated certificate's `notBefore` by this much.
///
/// A Pi with no RTC boots at whatever the filesystem timestamp implies until
/// NTP lands. Capture already fails open across that window
/// (`--doctor`'s clock check covers it); a certificate minted with
/// `notBefore = now` on a clock that is then corrected *backwards* would be
/// rejected as not-yet-valid, which looks exactly like a broken install.
const BACKDATE: Duration = Duration::from_secs(24 * 60 * 60);

/// How often [`spawn_reloader`] restats the certificate files.
const RELOAD_POLL: Duration = Duration::from_secs(60);

/// How long a client has to finish its TLS handshake before the connection is
/// dropped.
///
/// Without this a peer that opens a socket and sends nothing holds a task and
/// a file descriptor forever; a few hundred of those are a trivial way to
/// exhaust a Pi's descriptor budget from off-LAN.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// How the listener terminates TLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TlsMode {
    /// Plain HTTP. The default, and what every station did before this existed.
    #[default]
    Off,
    /// Generate (and rotate) a self-signed certificate, kept in the state
    /// directory so the browser fingerprint survives restarts.
    SelfSigned,
    /// Serve an operator-supplied certificate and key, reloaded when they
    /// change on disk.
    Manual,
}

impl TlsMode {
    /// The spelling used in configuration and in `--tls-mode`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::SelfSigned => "self-signed",
            Self::Manual => "manual",
        }
    }

    /// Whether this mode terminates TLS at all.
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

impl fmt::Display for TlsMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for TlsMode {
    type Err = TlsError;

    /// Accepts the hyphenated spelling and the two obvious variants operators
    /// type instead (`selfsigned`, `self_signed`), plus `none`/`disabled` for
    /// `off`. Being liberal here costs nothing and turns a station that
    /// silently served plain HTTP into one that starts as asked.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "off" | "none" | "disabled" | "" => Ok(Self::Off),
            "self-signed" | "selfsigned" => Ok(Self::SelfSigned),
            "manual" | "file" => Ok(Self::Manual),
            other => Err(TlsError::Mode(other.to_string())),
        }
    }
}

/// Everything [`server_config`] needs to produce a working TLS listener.
#[derive(Debug, Clone)]
pub struct TlsSettings {
    /// Which mode to run in.
    pub mode: TlsMode,
    /// Certificate chain, PEM. Required by [`TlsMode::Manual`], ignored
    /// otherwise.
    pub cert: Option<PathBuf>,
    /// Private key, PEM (PKCS#8, PKCS#1 or SEC1). Required by
    /// [`TlsMode::Manual`], ignored otherwise.
    pub key: Option<PathBuf>,
    /// Directory the generated self-signed material lives in. Created if
    /// missing.
    pub state_dir: PathBuf,
    /// Names the generated certificate should cover. Anything that parses as
    /// an IP address becomes an `iPAddress` SAN, everything else a `dNSName`.
    pub hostnames: Vec<String>,
    /// How long a generated certificate is valid for, in days.
    pub validity_days: u32,
}

impl Default for TlsSettings {
    fn default() -> Self {
        Self {
            mode: TlsMode::Off,
            cert: None,
            key: None,
            state_dir: PathBuf::from("/var/lib/birdnet/tls"),
            hostnames: Vec::new(),
            validity_days: 397,
        }
    }
}

/// Errors from certificate loading, generation and the serve loop.
#[derive(Debug)]
pub enum TlsError {
    /// `--tls-mode` was not one of the accepted spellings.
    Mode(String),
    /// A path required by the selected mode was not supplied.
    MissingPath(&'static str),
    /// A certificate or key file could not be read or written.
    Io(PathBuf, io::Error),
    /// A PEM file parsed but contained nothing usable.
    Empty(PathBuf, &'static str),
    /// A PEM file could not be parsed.
    Pem(PathBuf, rustls_pki_types::pem::Error),
    /// rustls refused the keypair — most often because the key does not match
    /// the certificate.
    Rustls(rustls::Error),
    /// Self-signed generation failed.
    Generate(String),
    /// The listener could not be bound or the accept loop failed.
    Serve(io::Error),
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mode(m) => write!(
                f,
                "unknown TLS mode {m:?} — expected one of: off, self-signed, manual"
            ),
            Self::MissingPath(what) => write!(f, "TLS mode 'manual' needs {what}"),
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Empty(p, what) => write!(f, "{} contains no {what}", p.display()),
            Self::Pem(p, e) => write!(f, "{} is not readable PEM: {e}", p.display()),
            Self::Rustls(e) => write!(f, "rustls rejected the certificate/key: {e}"),
            Self::Generate(m) => write!(f, "could not generate a self-signed certificate: {m}"),
            Self::Serve(e) => write!(f, "TLS listener failed: {e}"),
        }
    }
}

impl std::error::Error for TlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(_, e) | Self::Serve(e) => Some(e),
            Self::Rustls(e) => Some(e),
            Self::Pem(_, e) => Some(e),
            Self::Mode(_) | Self::MissingPath(_) | Self::Empty(..) | Self::Generate(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Certificate material
// ---------------------------------------------------------------------------

/// The crypto provider every rustls construction in this module uses.
///
/// Named explicitly rather than taken from the process-wide default, for the
/// same reason `birdnet-integrations::mqtt` does: which provider the default is
/// depends on whichever crate in the tree installed one first, and that is not
/// a property this code should depend on.
fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Read a PEM certificate chain and private key into a rustls [`CertifiedKey`].
///
/// `CertifiedKey::from_der` checks that the key matches the leaf certificate
/// where the algorithm allows it, so a mismatched pair fails here rather than
/// during a handshake an operator is not watching.
fn load_pair(cert_path: &Path, key_path: &Path) -> Result<Arc<CertifiedKey>, TlsError> {
    let cert_pem = std::fs::read(cert_path).map_err(|e| TlsError::Io(cert_path.into(), e))?;
    let key_pem = std::fs::read(key_path).map_err(|e| TlsError::Io(key_path.into(), e))?;

    // `rustls_pki_types::pem`, not `rustls-pemfile`: the latter was archived in
    // August 2025 (RUSTSEC-2025-0134) and its final release is a thin wrapper
    // around exactly this code. Same parser, one fewer crate, no advisory.
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Pem(cert_path.into(), e))?;
    if chain.is_empty() {
        return Err(TlsError::Empty(cert_path.into(), "certificate"));
    }

    let key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| {
        // An empty file and a malformed one are different operator problems,
        // and the PEM parser reports both as `NoItemsFound`.
        if key_pem.iter().all(u8::is_ascii_whitespace) {
            TlsError::Empty(key_path.into(), "private key")
        } else {
            TlsError::Pem(key_path.into(), e)
        }
    })?;

    let certified = CertifiedKey::from_der(chain, key, &provider()).map_err(TlsError::Rustls)?;
    Ok(Arc::new(certified))
}

/// What the generated material covers and when each half stops being valid.
///
/// Written beside the certificates rather than read back out of their DER:
/// recovering `notAfter` from X.509 means an ASN.1 stack to answer a question
/// this module already knew the answer to when it minted the things. `rcgen`
/// carries `x509-parser` as an *optional* dependency, so it sits in
/// `Cargo.lock` with eight more (`asn1-rs`, `der-parser`, `oid-registry`, …)
/// and none of them is compiled — enabling it to read one integer would build
/// all nine. The trade is that a
/// missing or unreadable sidecar forces a regeneration — cheap, and it
/// re-converges rather than serving material whose expiry nothing can
/// establish.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SelfSignedMeta {
    /// The leaf's `notAfter`, as seconds since the Unix epoch.
    not_after: i64,
    /// The CA's `notAfter`, as seconds since the Unix epoch.
    ca_not_after: i64,
    /// The SAN list the leaf was minted for, in the order given.
    hostnames: Vec<String>,
}

impl SelfSignedMeta {
    /// Serialise as three `key=value` lines.
    ///
    /// Not JSON: it is written and read only here, an operator may well `cat`
    /// it while working out why their browser is unhappy, and this project
    /// already prefers a line to a serialiser where a line will do.
    fn encode(&self) -> String {
        format!(
            "not_after={}\nca_not_after={}\nhostnames={}\n",
            self.not_after,
            self.ca_not_after,
            self.hostnames.join(",")
        )
    }

    /// Parse [`Self::encode`]'s output. Any deviation returns `None`, which the
    /// caller treats as "regenerate".
    fn decode(raw: &str) -> Option<Self> {
        let mut not_after = None;
        let mut ca_not_after = None;
        let mut hostnames = None;
        for line in raw.lines() {
            let (k, v) = line.split_once('=')?;
            match k.trim() {
                "not_after" => not_after = Some(v.trim().parse::<i64>().ok()?),
                "ca_not_after" => ca_not_after = Some(v.trim().parse::<i64>().ok()?),
                "hostnames" => {
                    hostnames = Some(
                        v.trim()
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>(),
                    );
                }
                _ => return None,
            }
        }
        Some(Self {
            not_after: not_after?,
            ca_not_after: ca_not_after?,
            hostnames: hostnames?,
        })
    }

    /// Whether the leaf can still be served for `wanted` at `now`.
    ///
    /// Both halves matter. Expiry is the obvious one; the hostname comparison
    /// catches the operator who added `--tls-hostname bird.example.com` and
    /// would otherwise keep being served the old certificate that does not name
    /// it, with no indication why the browser still complains.
    fn leaf_usable_for(&self, wanted: &[String], now: i64) -> bool {
        before_renewal(self.not_after, now) && self.hostnames == wanted
    }

    /// Whether the CA is still far enough from expiry to keep signing with.
    fn ca_usable(&self, now: i64) -> bool {
        before_renewal(self.ca_not_after, now)
    }
}

/// Whether `not_after` is far enough away that the material need not be
/// replaced yet.
fn before_renewal(not_after: i64, now: i64) -> bool {
    let renew_at =
        not_after.saturating_sub(i64::try_from(RENEW_BEFORE.as_secs()).unwrap_or(i64::MAX));
    now < renew_at
}

/// Seconds since the Unix epoch, saturating at 0 before it.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Write `contents` to `path` with `mode`, replacing whatever was there.
///
/// Writes to a sibling temporary file and renames, so a crash between the two
/// leaves the previous (still valid) material rather than a half-written key.
/// The mode is applied to the temporary file *before* the rename, so a private
/// key is never briefly world-readable at its final name.
fn write_private(path: &Path, contents: &str, mode: u32) -> Result<(), TlsError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| TlsError::Io(tmp.clone(), e))?;
    set_mode(&tmp, mode)?;
    std::fs::rename(&tmp, path).map_err(|e| TlsError::Io(path.into(), e))?;
    Ok(())
}

/// Apply a Unix permission bitmask. A no-op on platforms without them.
#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<(), TlsError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| TlsError::Io(path.into(), e))
}

/// Apply a Unix permission bitmask. A no-op on platforms without them.
#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<(), TlsError> {
    Ok(())
}

/// The path an operator imports to make this station's HTTPS trusted.
///
/// Public because `--doctor` and the docs both need to name it, and neither
/// should be re-deriving the filename.
#[must_use]
pub fn ca_certificate_path(state_dir: &Path) -> PathBuf {
    state_dir.join(CA_CERT)
}

/// Return the self-signed keypair in `state_dir`, generating what is missing.
///
/// The material is a **local CA and a leaf it signs**, not one self-signed
/// certificate. A single self-signed certificate cannot do the job: with
/// `CA:FALSE` a client that trusts the file rejects the handshake
/// (`BadSignature`), and with `CA:TRUE` it rejects the same certificate for
/// being a `CaUsedAsEndEntity`. Both were observed against rustls-webpki before
/// this was written the other way. Splitting them means the operator imports
/// `local-ca.crt` once and keeps working across every later leaf rotation.
///
/// The CA is reused while it is far from expiry, so a rotation replaces only
/// the leaf.
///
/// # Errors
///
/// Returns [`TlsError`] if the directory cannot be created, generation fails,
/// or the material cannot be written or read back.
pub fn ensure_self_signed(
    state_dir: &Path,
    hostnames: &[String],
    validity_days: u32,
) -> Result<Arc<CertifiedKey>, TlsError> {
    if hostnames.is_empty() {
        return Err(TlsError::Generate(
            "no hostnames to put in the certificate".into(),
        ));
    }
    std::fs::create_dir_all(state_dir).map_err(|e| TlsError::Io(state_dir.into(), e))?;

    let ca_cert_path = state_dir.join(CA_CERT);
    let ca_key_path = state_dir.join(CA_KEY);
    let leaf_cert_path = state_dir.join(LEAF_CERT);
    let leaf_key_path = state_dir.join(LEAF_KEY);
    let meta_path = state_dir.join(META);

    let now = unix_now();
    let meta = std::fs::read_to_string(&meta_path)
        .ok()
        .as_deref()
        .and_then(SelfSignedMeta::decode);

    let ca_ok = meta.as_ref().is_some_and(|m| m.ca_usable(now))
        && ca_cert_path.exists()
        && ca_key_path.exists();
    let leaf_ok = meta
        .as_ref()
        .is_some_and(|m| m.leaf_usable_for(hostnames, now))
        && leaf_cert_path.exists()
        && leaf_key_path.exists();

    if ca_ok && leaf_ok {
        tracing::debug!(dir = %state_dir.display(), "reusing the stored self-signed material");
        return load_pair(&leaf_cert_path, &leaf_key_path);
    }

    // Reuse the CA whenever it is still good, so a leaf rotation does not
    // invalidate a trust-store import the operator already did.
    let (ca_key_pem, ca_cert_pem, ca_not_after) = if ca_ok {
        let key = std::fs::read_to_string(&ca_key_path)
            .map_err(|e| TlsError::Io(ca_key_path.clone(), e))?;
        let cert = std::fs::read_to_string(&ca_cert_path)
            .map_err(|e| TlsError::Io(ca_cert_path.clone(), e))?;
        let expiry = meta.as_ref().map_or(0, |m| m.ca_not_after);
        (key, cert, expiry)
    } else {
        generate_ca()?
    };

    let (leaf_cert_pem, leaf_key_pem, not_after) =
        generate_leaf(&ca_key_pem, hostnames, validity_days)?;

    if !ca_ok {
        write_private(&ca_cert_path, &ca_cert_pem, 0o644)?;
        write_private(&ca_key_path, &ca_key_pem, 0o600)?;
    }
    // The served chain is leaf-then-CA so a client that has never seen the CA
    // still receives it and can at least report the right issuer.
    write_private(
        &leaf_cert_path,
        &format!("{leaf_cert_pem}{ca_cert_pem}"),
        0o644,
    )?;
    write_private(&leaf_key_path, &leaf_key_pem, 0o600)?;
    write_private(
        &meta_path,
        &SelfSignedMeta {
            not_after,
            ca_not_after,
            hostnames: hostnames.to_vec(),
        }
        .encode(),
        0o644,
    )?;

    tracing::info!(
        ca = %ca_cert_path.display(),
        hostnames = %hostnames.join(", "),
        days = validity_days,
        reused_ca = ca_ok,
        "generated a self-signed TLS certificate; import the CA file above to stop the browser warning"
    );
    load_pair(&leaf_cert_path, &leaf_key_path)
}

/// The local CA's parameters.
///
/// Deterministic, and it has to be: [`ensure_self_signed`] rebuilds these from
/// scratch when it reuses a CA off disk, and the reconstructed distinguished
/// name has to match the stored certificate's subject exactly or the leaf it
/// signs will chain to nothing.
fn ca_params() -> rcgen::CertificateParams {
    let mut params = rcgen::CertificateParams::default();
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, "BirdNet-Behavior local CA");
    dn.push(rcgen::DnType::OrganizationName, "BirdNet-Behavior");
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    params
}

/// Mint the local CA. Returns `(key PEM, certificate PEM, notAfter)`.
fn generate_ca() -> Result<(String, String, i64), TlsError> {
    let mut params = ca_params();
    let now = unix_now();
    let (not_before, not_after) = validity_window(now, CA_VALIDITY_DAYS);
    let (y, m, d) = civil_ymd(not_before);
    params.not_before = rcgen::date_time_ymd(y, m, d);
    let (y, m, d) = civil_ymd(not_after);
    params.not_after = rcgen::date_time_ymd(y, m, d);

    let key = rcgen::KeyPair::generate().map_err(|e| TlsError::Generate(e.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| TlsError::Generate(e.to_string()))?;
    Ok((key.serialize_pem(), cert.pem(), not_after))
}

/// Mint a server certificate for `hostnames`, signed by the CA whose key is
/// `ca_key_pem`. Returns `(certificate PEM, key PEM, notAfter)`.
fn generate_leaf(
    ca_key_pem: &str,
    hostnames: &[String],
    validity_days: u32,
) -> Result<(String, String, i64), TlsError> {
    let mut params = rcgen::CertificateParams::new(hostnames.to_vec())
        .map_err(|e| TlsError::Generate(e.to_string()))?;

    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, hostnames[0].clone());
    dn.push(rcgen::DnType::OrganizationName, "BirdNet-Behavior");
    params.distinguished_name = dn;
    params.is_ca = rcgen::IsCa::ExplicitNoCa;
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

    let now = unix_now();
    let (not_before, not_after) = validity_window(now, validity_days);
    let (y, m, d) = civil_ymd(not_before);
    params.not_before = rcgen::date_time_ymd(y, m, d);
    let (y, m, d) = civil_ymd(not_after);
    params.not_after = rcgen::date_time_ymd(y, m, d);

    let ca_key =
        rcgen::KeyPair::from_pem(ca_key_pem).map_err(|e| TlsError::Generate(e.to_string()))?;
    let ca_params = ca_params();
    let issuer = rcgen::Issuer::new(ca_params, ca_key);

    let leaf_key = rcgen::KeyPair::generate().map_err(|e| TlsError::Generate(e.to_string()))?;
    let cert = params
        .signed_by(&leaf_key, &issuer)
        .map_err(|e| TlsError::Generate(e.to_string()))?;

    Ok((cert.pem(), leaf_key.serialize_pem(), not_after))
}

/// `(notBefore, notAfter)` for a certificate minted at `now`, in Unix seconds.
///
/// `notBefore` is backdated by [`BACKDATE`]: a Pi with no RTC boots at whatever
/// the filesystem timestamp implies until NTP lands, and a certificate minted
/// with `notBefore = now` on a clock that is then corrected *backwards* is
/// rejected as not-yet-valid, which looks exactly like a broken install.
fn validity_window(now: i64, days: u32) -> (i64, i64) {
    let day_secs = 24 * 60 * 60;
    (
        now.saturating_sub(i64::try_from(BACKDATE.as_secs()).unwrap_or(0)),
        now.saturating_add(i64::from(days).saturating_mul(day_secs)),
    )
}

/// The civil `(year, month, day)` in UTC containing `unix_secs`, in the shape
/// `rcgen::date_time_ymd` wants.
///
/// `rcgen` types its validity bounds with `time::OffsetDateTime` and does not
/// re-export the crate. `date_time_ymd` is the seam that avoids naming it: the
/// calendar arithmetic stays the one implementation the rest of the binary
/// already shares (`birdnet_core::civil`, whose header says why that matters),
/// and no workspace crate gains a direct `time` dependency.
///
/// To be clear about what this does *not* buy, because the equivalent claim in
/// `birdnet_db::clock` was wrong for two years and the note there now records
/// it: `rcgen` puts `time` in the tree regardless, in every build
/// configuration. This is about keeping one calendar implementation, not about
/// dependency weight — that cost is already paid.
fn civil_ymd(unix_secs: i64) -> (i32, u8, u8) {
    let days = unix_secs.div_euclid(24 * 60 * 60);
    let (y, m, d) = birdnet_core::civil::civil_from_days(days);
    (
        i32::try_from(y).unwrap_or(1970),
        u8::try_from(m).unwrap_or(1),
        u8::try_from(d).unwrap_or(1),
    )
}

// ---------------------------------------------------------------------------
// Resolver + hot reload
// ---------------------------------------------------------------------------

/// A [`ResolvesServerCert`] whose keypair can be replaced while the server runs.
///
/// Every handshake reads through the lock, so a swap is picked up by the next
/// connection without touching the ones already established.
#[derive(Debug)]
pub struct Resolver {
    /// The keypair every handshake is answered with.
    current: RwLock<Arc<CertifiedKey>>,
}

impl Resolver {
    /// Wrap an initial keypair.
    #[must_use]
    pub const fn new(initial: Arc<CertifiedKey>) -> Self {
        Self {
            current: RwLock::new(initial),
        }
    }

    /// Replace the served keypair.
    ///
    /// A poisoned lock is recovered from rather than propagated: the only thing
    /// under it is an `Arc` swap, so a panic elsewhere cannot have left it
    /// inconsistent, and refusing to serve HTTPS for the rest of the process's
    /// life is a far worse outcome than continuing.
    pub fn replace(&self, next: Arc<CertifiedKey>) {
        let mut guard = match self.current.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = next;
    }

    /// The keypair currently being served.
    #[must_use]
    pub fn current(&self) -> Arc<CertifiedKey> {
        match self.current.read() {
            Ok(g) => Arc::clone(&g),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current())
    }
}

/// Watch `cert`/`key` and hand any change to `resolver`.
///
/// Polls rather than using `notify`: two `stat` calls a minute is nothing, and
/// an ACME client that writes via rename (which every one of them does) produces
/// inode churn that a naive watcher misses anyway. A read that fails is logged
/// and skipped — a renewal caught mid-write retries a minute later rather than
/// tearing down the keypair that currently works.
pub fn spawn_reloader(resolver: Arc<Resolver>, cert: PathBuf, key: PathBuf) {
    tokio::spawn(async move {
        let mut last = fingerprint(&cert, &key);
        loop {
            tokio::time::sleep(RELOAD_POLL).await;
            let now = fingerprint(&cert, &key);
            if now == last {
                continue;
            }
            match load_pair(&cert, &key) {
                Ok(next) => {
                    resolver.replace(next);
                    last = now;
                    tracing::info!(
                        cert = %cert.display(),
                        "TLS certificate changed on disk and was reloaded"
                    );
                }
                Err(e) => {
                    // Deliberately does not update `last`, so a file caught
                    // mid-rewrite is retried on the next tick.
                    tracing::warn!(
                        cert = %cert.display(),
                        error = %e,
                        "TLS certificate changed on disk but could not be loaded; still serving the previous one"
                    );
                }
            }
        }
    });
}

/// `(len, mtime)` for both files — enough to notice a renewal, cheap enough to
/// run every minute. Unreadable files collapse to `None`, so a file that
/// disappears and comes back is treated as a change.
fn fingerprint(cert: &Path, key: &Path) -> [Option<(u64, SystemTime)>; 2] {
    [cert, key].map(|p| {
        let m = std::fs::metadata(p).ok()?;
        Some((m.len(), m.modified().ok()?))
    })
}

// ---------------------------------------------------------------------------
// Server config
// ---------------------------------------------------------------------------

/// A built TLS listener configuration and the resolver whose keypair it serves.
///
/// The two travel together because the caller needs both: the config to hand to
/// [`serve`], and the resolver to hand to [`spawn_reloader`] so a renewed
/// certificate is picked up without a restart.
pub type ConfiguredTls = (Arc<ServerConfig>, Arc<Resolver>);

/// Build a rustls [`ServerConfig`] from `settings`, plus the resolver behind it.
///
/// Returns `Ok(None)` when the mode is [`TlsMode::Off`] — the caller then serves
/// plain HTTP exactly as before, and nothing in this module runs.
///
/// The returned [`Resolver`] is handed back so the caller can start
/// [`spawn_reloader`] on it; it is already installed in the config.
///
/// # Errors
///
/// Returns [`TlsError`] if the configured material is missing, unreadable,
/// mismatched, or (in self-signed mode) cannot be generated.
pub fn server_config(settings: &TlsSettings) -> Result<Option<ConfiguredTls>, TlsError> {
    let keypair = match settings.mode {
        TlsMode::Off => return Ok(None),
        TlsMode::SelfSigned => ensure_self_signed(
            &settings.state_dir,
            &settings.hostnames,
            settings.validity_days,
        )?,
        TlsMode::Manual => {
            let cert = settings
                .cert
                .as_deref()
                .ok_or(TlsError::MissingPath("--tls-cert"))?;
            let key = settings
                .key
                .as_deref()
                .ok_or(TlsError::MissingPath("--tls-key"))?;
            load_pair(cert, key)?
        }
    };

    let resolver = Arc::new(Resolver::new(keypair));
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(TlsError::Rustls)?
        .with_no_client_auth()
        .with_cert_resolver(Arc::clone(&resolver) as Arc<dyn ResolvesServerCert>);

    // Offer h2 before http/1.1: the dashboard opens a WebSocket and several
    // HTMX partials at once, and h2 multiplexes them onto the one connection.
    // The auto builder below serves whichever the client selects.
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok(Some((Arc::new(config), resolver)))
}

// ---------------------------------------------------------------------------
// Serve loop
// ---------------------------------------------------------------------------

/// Serve `app` over TLS on `listener` until `shutdown` resolves.
///
/// Returns once the listener has stopped accepting and every connection that
/// was still open has been asked to close. It does **not** wait for a peer that
/// ignores that request — the caller's shutdown backstop bounds that, exactly as
/// it does for the plain-HTTP path.
///
/// # Errors
///
/// Returns [`TlsError::Serve`] only for an accept error that is not transient;
/// per-connection failures are logged and dropped, because one peer failing a
/// handshake must not take the listener down.
pub async fn serve<F>(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    app: Router,
    shutdown: F,
) -> Result<(), TlsError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let acceptor = TlsAcceptor::from(config);
    // `false` while running; flipped once, which every live connection task is
    // watching so it can start its own graceful close.
    let (closing_tx, closing_rx) = tokio::sync::watch::channel(false);
    let tracker = tokio_util::task::TaskTracker::new();
    let mut shutdown = std::pin::pin!(shutdown);

    loop {
        let accepted = tokio::select! {
            res = listener.accept() => res,
            () = &mut shutdown => break,
        };

        let (stream, peer) = match accepted {
            Ok(v) => v,
            Err(e) if is_transient_accept_error(&e) => {
                // A peer that vanished between the SYN and our accept, or a
                // momentarily exhausted descriptor table. Taking the whole
                // listener down for either would turn a blip into an outage.
                tracing::debug!(error = %e, "transient TLS accept error; continuing");
                continue;
            }
            Err(e) => return Err(TlsError::Serve(e)),
        };

        let acceptor = acceptor.clone();
        let app = app.clone();
        let mut closing = closing_rx.clone();
        tracker.spawn(async move {
            let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream));
            let tls_stream = match handshake.await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    // Routine: a health-checker speaking plain HTTP at the TLS
                    // port, or a browser that refused the self-signed cert.
                    tracing::debug!(%peer, error = %e, "TLS handshake failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(%peer, "TLS handshake timed out");
                    return;
                }
            };

            let service = hyper::service::service_fn(move |mut req| {
                // The rate limiter and every handler that logs a client address
                // read this. `axum::serve` installs it for the plain-HTTP
                // listener; this loop is the TLS equivalent.
                req.extensions_mut().insert(ConnectInfo(peer));
                app.clone().call(req)
            });

            let builder = hyper_util::server::conn::auto::Builder::new(
                hyper_util::rt::TokioExecutor::new(),
            );
            let conn = builder
                .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(tls_stream), service);
            let mut conn = std::pin::pin!(conn);

            loop {
                tokio::select! {
                    res = conn.as_mut() => {
                        if let Err(e) = res {
                            tracing::debug!(%peer, error = %e, "TLS connection ended with an error");
                        }
                        break;
                    }
                    // `changed()` errors only once the sender is dropped, which
                    // happens at the end of `serve` — by then this task is
                    // being torn down anyway, so treat it as "start closing".
                    _ = closing.changed() => {
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }
        });
    }

    tracing::info!("TLS listener stopped accepting; draining connections");
    // Wakes every live connection into its `graceful_shutdown` arm.
    let _ = closing_tx.send(true);
    tracker.close();
    tracker.wait().await;
    Ok(())
}

/// Whether an `accept` error is worth continuing through.
///
/// The listener itself is still healthy in all of these: the peer went away, or
/// the process momentarily had no descriptors. Anything else — the socket
/// closed under us, say — is real and returns.
fn is_transient_accept_error(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::TimedOut
    ) || e.raw_os_error() == Some(24) // EMFILE
        || e.raw_os_error() == Some(23) // ENFILE
}

// ---------------------------------------------------------------------------
// HTTP → HTTPS redirect
// ---------------------------------------------------------------------------

/// A router whose every route answers `308 Permanent Redirect` to `https_port`
/// on the host the client asked for.
///
/// Deliberately preserves the path and query and reuses the request's own
/// `Host`, so a redirect works for `birdnetpi.local`, a bare IP and a name this
/// station has never heard of alike. `308` rather than `301` because it is the
/// one redirect status that is defined to preserve the method and body — a
/// `POST` to `/admin/settings` over plain HTTP would otherwise silently become
/// a `GET` and lose the form.
pub fn redirect_router(https_port: u16) -> Router {
    use axum::http::{HeaderMap, StatusCode, Uri, header};
    use axum::response::IntoResponse;

    Router::new().fallback(move |uri: Uri, headers: HeaderMap| async move {
        let path = uri.path_and_query().map_or("/", |pq| pq.as_str());
        let Some(hostname) = request_hostname(&uri, &headers) else {
            // No `Host` header and no `:authority` (HTTP/1.0, or a scanner).
            // There is nothing to build an absolute URL from, so say so rather
            // than redirect somewhere invented.
            return (StatusCode::BAD_REQUEST, "missing Host header").into_response();
        };
        let target = format!("https://{hostname}:{https_port}{path}");
        axum::http::HeaderValue::from_str(&target).map_or_else(
            |_| (StatusCode::BAD_REQUEST, "unusable Host header").into_response(),
            |value| (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, value)]).into_response(),
        )
    })
}

/// The host the client asked for, without any port it supplied.
///
/// HTTP/2 carries it in `:authority`, which hyper surfaces on the URI; HTTP/1.1
/// carries it in the `Host` header. Both have to work, because the redirect
/// listener is the first thing a browser touches and the auto builder will have
/// negotiated either.
///
/// Anything that is not a plausible host is refused rather than echoed: this
/// string goes straight into a `Location` header, and reflecting an arbitrary
/// attacker-supplied `Host` there is an open redirect.
fn request_hostname(uri: &axum::http::Uri, headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = uri.host().map(str::to_owned).or_else(|| {
        headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    })?;

    // Strip the port the client sent; ours is the one that matters. An IPv6
    // literal is bracketed, so only split on the last colon outside brackets.
    let host = raw.rsplit_once(']').map_or_else(
        || raw.split(':').next().unwrap_or(&raw).to_string(),
        |(head, _)| format!("{head}]"),
    );

    let host = host.trim();
    let plausible = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'));
    plausible.then(|| host.to_owned())
}

/// Serve [`redirect_router`] on `listener` until `shutdown` resolves.
///
/// # Errors
///
/// Returns [`TlsError::Serve`] if the plain listener fails.
pub async fn serve_redirect<F>(
    listener: TcpListener,
    https_port: u16,
    shutdown: F,
) -> Result<(), TlsError>
where
    F: Future<Output = ()> + Send + 'static,
{
    axum::serve(
        listener,
        redirect_router(https_port).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(TlsError::Serve)
}

// ---------------------------------------------------------------------------
// Hostname discovery
// ---------------------------------------------------------------------------

/// The names a generated certificate should cover when the operator names none.
///
/// A station is reached by whatever the person typing knows about it, which on
/// a LAN is usually three different things: `localhost` from the box itself,
/// `<hostname>.local` over mDNS, and the bare IP from a phone. A certificate
/// naming only one of those produces a *second*, different browser warning the
/// first time somebody uses another, which reads like the encryption broke.
///
/// `bind` contributes its address when it is a concrete one; a wildcard bind
/// (`0.0.0.0`) names no particular interface and is skipped.
#[must_use]
pub fn default_hostnames(bind: SocketAddr, system_hostname: Option<&str>) -> Vec<String> {
    let mut out = vec!["localhost".to_string()];

    if let Some(h) = system_hostname {
        let h = h.trim();
        if !h.is_empty() && h != "localhost" {
            out.push(h.to_string());
            if !h.contains('.') {
                out.push(format!("{h}.local"));
            }
        }
    }

    let ip = bind.ip();
    if !ip.is_unspecified() {
        out.push(ip.to_string());
    }
    // Always name the loopback literal: `https://127.0.0.1:8502` is what the
    // installer prints and what `--doctor` probes.
    out.push("127.0.0.1".to_string());

    out.dedup();
    out
}

/// The system hostname, if the platform will say.
///
/// Reads `/etc/hostname` rather than calling `gethostname(2)`, because the
/// latter needs `unsafe` or a crate and this is a single line of text on every
/// platform this project supports.
#[must_use]
pub fn system_hostname() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn mode_parses_the_spellings_operators_type() {
        for (raw, want) in [
            ("off", TlsMode::Off),
            ("OFF", TlsMode::Off),
            ("none", TlsMode::Off),
            ("", TlsMode::Off),
            ("self-signed", TlsMode::SelfSigned),
            ("selfsigned", TlsMode::SelfSigned),
            ("self_signed", TlsMode::SelfSigned),
            (" Self-Signed ", TlsMode::SelfSigned),
            ("manual", TlsMode::Manual),
        ] {
            assert_eq!(raw.parse::<TlsMode>().unwrap(), want, "parsing {raw:?}");
        }
        assert!("sometimes".parse::<TlsMode>().is_err());
    }

    #[test]
    fn only_off_is_disabled() {
        assert!(!TlsMode::Off.enabled());
        assert!(TlsMode::SelfSigned.enabled());
        assert!(TlsMode::Manual.enabled());
    }

    #[test]
    fn meta_round_trips() {
        let m = SelfSignedMeta {
            not_after: 1_800_000_000,
            ca_not_after: 2_100_000_000,
            hostnames: names(&["localhost", "pi.local"]),
        };
        assert_eq!(SelfSignedMeta::decode(&m.encode()).unwrap(), m);
    }

    #[test]
    fn meta_decode_rejects_junk() {
        assert!(SelfSignedMeta::decode("").is_none());
        assert!(SelfSignedMeta::decode("not_after=soon\nca_not_after=2\nhostnames=a\n").is_none());
        assert!(SelfSignedMeta::decode("hostnames=a\n").is_none());
        assert!(SelfSignedMeta::decode("not_after=1\nhostnames=a\n").is_none());
        assert!(SelfSignedMeta::decode("not_after=1\nca_not_after=2\n").is_none());
        assert!(SelfSignedMeta::decode("surprise=1\n").is_none());
    }

    #[test]
    fn meta_is_unusable_once_inside_the_renewal_window() {
        let now = 1_000_000_000;
        let renew = i64::try_from(RENEW_BEFORE.as_secs()).unwrap();
        let hosts = names(&["localhost"]);

        let far = now + renew + 10 * 365 * 24 * 60 * 60;

        let fresh = SelfSignedMeta {
            not_after: now + renew + 60,
            ca_not_after: far,
            hostnames: hosts.clone(),
        };
        assert!(
            fresh.leaf_usable_for(&hosts, now),
            "outside the window: reuse"
        );

        let due = SelfSignedMeta {
            not_after: now + renew - 60,
            ca_not_after: far,
            hostnames: hosts.clone(),
        };
        assert!(
            !due.leaf_usable_for(&hosts, now),
            "inside the window: regenerate"
        );

        let expired = SelfSignedMeta {
            not_after: now - 1,
            ca_not_after: far,
            hostnames: hosts.clone(),
        };
        assert!(!expired.leaf_usable_for(&hosts, now));

        // The CA gets the same margin, on its own clock.
        assert!(due.ca_usable(now), "a far-off CA is still signable with");
        let ca_due = SelfSignedMeta {
            ca_not_after: now + renew - 60,
            ..fresh
        };
        assert!(!ca_due.ca_usable(now));
    }

    #[test]
    fn meta_is_unusable_when_the_hostnames_changed() {
        let now = 1_000_000_000;
        let m = SelfSignedMeta {
            not_after: now + 10 * 365 * 24 * 60 * 60,
            ca_not_after: now + 20 * 365 * 24 * 60 * 60,
            hostnames: names(&["localhost"]),
        };
        assert!(m.leaf_usable_for(&names(&["localhost"]), now));
        assert!(
            !m.leaf_usable_for(&names(&["localhost", "pi.local"]), now),
            "an added --tls-hostname must force a regeneration, or the browser \
             keeps rejecting the name the operator just configured"
        );
    }

    #[test]
    fn default_hostnames_cover_the_three_ways_in() {
        let bind: SocketAddr = "192.168.1.40:8502".parse().unwrap();
        let got = default_hostnames(bind, Some("birdpi"));
        for want in [
            "localhost",
            "birdpi",
            "birdpi.local",
            "192.168.1.40",
            "127.0.0.1",
        ] {
            assert!(got.iter().any(|h| h == want), "{want} missing from {got:?}");
        }
    }

    #[test]
    fn default_hostnames_skips_a_wildcard_bind() {
        let got = default_hostnames("0.0.0.0:8502".parse().unwrap(), None);
        assert!(
            !got.iter().any(|h| h == "0.0.0.0"),
            "0.0.0.0 is not a name anything connects to: {got:?}"
        );
        assert!(got.iter().any(|h| h == "127.0.0.1"));
    }

    #[test]
    fn default_hostnames_does_not_append_local_to_an_fqdn() {
        let got = default_hostnames("0.0.0.0:8502".parse().unwrap(), Some("pi.example.com"));
        assert!(got.iter().any(|h| h == "pi.example.com"));
        assert!(!got.iter().any(|h| h == "pi.example.com.local"), "{got:?}");
    }

    #[test]
    fn civil_ymd_matches_the_calendar() {
        // 2026-09-02T00:00:00Z = 1 788 307 200; one second earlier must still
        // be 2026-09-01, which is the truncation `div_euclid` is there for.
        assert_eq!(civil_ymd(1_788_307_200), (2026, 9, 2));
        assert_eq!(civil_ymd(1_788_307_199), (2026, 9, 1));
        assert_eq!(civil_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn request_hostname_prefers_authority_then_host_and_drops_the_port() {
        use axum::http::{HeaderMap, HeaderValue, Uri, header};

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("pi.local:8502"));
        let rel: Uri = "/today".parse().unwrap();
        assert_eq!(
            request_hostname(&rel, &headers).as_deref(),
            Some("pi.local")
        );

        // HTTP/2: the authority is on the URI and wins.
        let abs: Uri = "https://other.example:9/today".parse().unwrap();
        assert_eq!(
            request_hostname(&abs, &headers).as_deref(),
            Some("other.example")
        );

        assert_eq!(request_hostname(&rel, &HeaderMap::new()), None);
    }

    #[test]
    fn request_hostname_refuses_a_host_it_would_be_unsafe_to_reflect() {
        use axum::http::{HeaderMap, HeaderValue, Uri, header};
        let rel: Uri = "/".parse().unwrap();

        for bad in ["evil.example/@attacker.test", "a b", "x\\y"] {
            let mut headers = HeaderMap::new();
            let Ok(value) = HeaderValue::from_str(bad) else {
                continue;
            };
            headers.insert(header::HOST, value);
            assert_eq!(
                request_hostname(&rel, &headers),
                None,
                "{bad:?} must not reach a Location header"
            );
        }
    }

    #[test]
    fn request_hostname_keeps_an_ipv6_literal_intact() {
        use axum::http::{HeaderMap, HeaderValue, Uri, header};
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("[fe80::1]:8502"));
        let rel: Uri = "/".parse().unwrap();
        assert_eq!(
            request_hostname(&rel, &headers).as_deref(),
            Some("[fe80::1]"),
            "splitting an IPv6 literal on the first colon would produce '['"
        );
    }

    #[test]
    fn generated_pair_loads_and_matches() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost", "127.0.0.1"]);
        let key = ensure_self_signed(dir.path(), &hosts, 30).expect("generate");
        assert!(!key.cert.is_empty(), "a chain with no certificate in it");
    }

    #[test]
    fn generation_refuses_an_empty_hostname_list() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_self_signed(dir.path(), &[], 30).unwrap_err();
        assert!(matches!(err, TlsError::Generate(_)), "{err:?}");
    }

    #[test]
    fn stored_material_is_reused_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost"]);
        ensure_self_signed(dir.path(), &hosts, 365).expect("first");
        let first = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        ensure_self_signed(dir.path(), &hosts, 365).expect("second");
        let second = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        assert_eq!(
            first, second,
            "a restart must not mint a new certificate — the operator would have \
             to re-trust it every time the service restarts"
        );
    }

    #[test]
    fn changed_hostnames_force_a_new_certificate() {
        let dir = tempfile::tempdir().unwrap();
        ensure_self_signed(dir.path(), &names(&["localhost"]), 365).expect("first");
        let first = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        ensure_self_signed(dir.path(), &names(&["localhost", "pi.local"]), 365).expect("second");
        let second = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn an_expiring_certificate_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost"]);
        ensure_self_signed(dir.path(), &hosts, 365).expect("first");
        let first = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();

        // Rewrite the sidecar so the stored material looks like it expires
        // tomorrow — well inside RENEW_BEFORE.
        std::fs::write(
            dir.path().join(META),
            SelfSignedMeta {
                not_after: unix_now() + 24 * 60 * 60,
                ca_not_after: unix_now() + 3600 * 24 * 3650,
                hostnames: hosts.clone(),
            }
            .encode(),
        )
        .unwrap();

        ensure_self_signed(dir.path(), &hosts, 365).expect("second");
        let second = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        assert_ne!(
            first, second,
            "a certificate inside its renewal window must be replaced, or the \
             station stops serving HTTPS on a date nobody is watching"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_private_key_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        ensure_self_signed(dir.path(), &names(&["localhost"]), 30).expect("generate");
        let mode = std::fs::metadata(dir.path().join(LEAF_KEY))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "key mode was {mode:o}");
    }

    #[test]
    fn missing_sidecar_regenerates_rather_than_serving_an_unknowable_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost"]);
        ensure_self_signed(dir.path(), &hosts, 365).expect("first");
        let first = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        std::fs::remove_file(dir.path().join(META)).unwrap();
        ensure_self_signed(dir.path(), &hosts, 365).expect("second");
        let second = std::fs::read(dir.path().join(LEAF_CERT)).unwrap();
        assert_ne!(first, second);
    }

    #[test]
    fn manual_mode_rejects_a_key_that_does_not_match_the_certificate() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost"]);
        ensure_self_signed(a.path(), &hosts, 30).expect("pair a");
        ensure_self_signed(b.path(), &hosts, 30).expect("pair b");

        let err = load_pair(&a.path().join(LEAF_CERT), &b.path().join(LEAF_KEY))
            .expect_err("a mismatched pair must not build a server config");
        assert!(matches!(err, TlsError::Rustls(_)), "{err:?}");
    }

    #[test]
    fn manual_mode_needs_both_paths() {
        let settings = TlsSettings {
            mode: TlsMode::Manual,
            ..TlsSettings::default()
        };
        let err = server_config(&settings).unwrap_err();
        assert!(
            matches!(err, TlsError::MissingPath("--tls-cert")),
            "{err:?}"
        );

        let settings = TlsSettings {
            mode: TlsMode::Manual,
            cert: Some(PathBuf::from("/nonexistent.crt")),
            ..TlsSettings::default()
        };
        let err = server_config(&settings).unwrap_err();
        assert!(matches!(err, TlsError::MissingPath("--tls-key")), "{err:?}");
    }

    #[test]
    fn off_builds_nothing() {
        assert!(server_config(&TlsSettings::default()).unwrap().is_none());
    }

    #[test]
    fn self_signed_builds_a_server_config_offering_both_alpn_protocols() {
        let dir = tempfile::tempdir().unwrap();
        let settings = TlsSettings {
            mode: TlsMode::SelfSigned,
            state_dir: dir.path().to_path_buf(),
            hostnames: names(&["localhost"]),
            ..TlsSettings::default()
        };
        let (config, _resolver) = server_config(&settings).unwrap().expect("some config");
        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn resolver_swaps_what_it_serves() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let hosts = names(&["localhost"]);
        let first = ensure_self_signed(a.path(), &hosts, 30).unwrap();
        let second = ensure_self_signed(b.path(), &hosts, 30).unwrap();

        let resolver = Resolver::new(Arc::clone(&first));
        assert_eq!(resolver.current().cert, first.cert);
        resolver.replace(Arc::clone(&second));
        assert_eq!(resolver.current().cert, second.cert);
        assert_ne!(
            first.cert, second.cert,
            "the fixture is broken if both directories minted the same certificate"
        );
    }

    #[test]
    fn transient_accept_errors_do_not_stop_the_listener() {
        for kind in [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::Interrupted,
        ] {
            assert!(is_transient_accept_error(&io::Error::new(kind, "x")));
        }
        assert!(is_transient_accept_error(&io::Error::from_raw_os_error(24)));
        assert!(!is_transient_accept_error(&io::Error::new(
            io::ErrorKind::InvalidInput,
            "x"
        )));
    }

    /// An empty file and a corrupt one are different operator problems.
    ///
    /// The PEM parser reports both as `NoItemsFound`, so `load_pair` decides
    /// between them itself — and a discrimination with no test is a coin flip.
    /// "your key file is empty" sends an operator to whatever was supposed to
    /// write it; "not valid PEM" sends them to what is in it.
    #[test]
    fn an_empty_key_reads_as_empty_and_a_corrupt_one_as_malformed() {
        let dir = tempfile::tempdir().unwrap();
        ensure_self_signed(dir.path(), &names(&["localhost"]), 30).expect("generate");
        let cert = dir.path().join(LEAF_CERT);

        let blank = dir.path().join("blank.key");
        std::fs::write(&blank, "  \n\t\n").unwrap();
        let err = load_pair(&cert, &blank).expect_err("an empty file is not a key");
        assert!(
            matches!(&err, TlsError::Empty(p, "private key") if p == &blank),
            "{err:?}"
        );

        let junk = dir.path().join("junk.key");
        std::fs::write(&junk, "-----BEGIN PRIVATE KEY-----\nnot base64!\n").unwrap();
        let err = load_pair(&cert, &junk).expect_err("a corrupt file is not a key");
        assert!(matches!(&err, TlsError::Pem(p, _) if p == &junk), "{err:?}");
    }

    /// The same question on the certificate side, which reaches the answer by
    /// a different route: `pem_slice_iter` yields nothing for an empty file
    /// rather than failing, so emptiness is a separate statement here and can
    /// rot on its own.
    #[test]
    fn an_empty_certificate_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        ensure_self_signed(dir.path(), &names(&["localhost"]), 30).expect("generate");
        let key = dir.path().join(LEAF_KEY);

        let blank = dir.path().join("blank.crt");
        std::fs::write(&blank, "").unwrap();
        let err = load_pair(&blank, &key).expect_err("an empty file is not a chain");
        assert!(
            matches!(&err, TlsError::Empty(p, "certificate") if p == &blank),
            "{err:?}"
        );
    }
}
