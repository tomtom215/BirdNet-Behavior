//! The "Secure connection" card on the Access tab, and the download of the
//! station's local CA certificate (`P-22`).
//!
//! tphakala/birdnet-go has a page to upload, generate and download TLS
//! certificates. This station already generates its own (`--tls-mode
//! self-signed`: a local CA and a leaf it signs, rotated before expiry) and
//! reloads a real one when it changes on disk (`manual`); what an operator was
//! missing was any word about it in the browser: the Access tab said nothing about
//! HTTPS and the file to trust lived only in the journal. So this is:
//!
//! * what the station is doing now: off, self-signed (with the leaf's real
//!   expiry), manual, or waiting for the clock;
//! * a **download of the CA certificate** (public by nature) and how to trust it;
//! * for a station on plain HTTP, how to turn HTTPS on, in the order of effort.
//!
//! # What is deliberately not here
//!
//! **Upload.** The page an operator opens to fix a plain-HTTP station is itself
//! served over plain HTTP, so an upload form would carry a private key across the
//! network in the clear, to set up the protection that would have prevented it.
//! A certificate and key reach `manual` mode as files on the station, which is
//! what certbot and every internal CA produce anyway.
//!
//! **Generate.** `self-signed` generates at start and rotates on its own; a
//! button could only set the mode, which is read when the listener is bound, so
//! it would still need a restart and would add a second place to set it.

use std::fmt::Write as _;
use std::path::PathBuf;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Extension, Router};

use crate::request_scheme::RequestScheme;
use crate::routes::pages::escape_html;
use crate::state::AppState;
use crate::tls::TlsMode;

/// What the binary tells the web crate about the TLS it is running.
#[derive(Debug, Clone)]
pub struct HttpsStatus {
    /// The mode in force.
    pub mode: TlsMode,
    /// Where HTTPS listens, if it does.
    pub https_addr: Option<String>,
    /// Where the generated material lives.
    pub state_dir: PathBuf,
    /// HTTPS was asked for but is waiting for the clock to be set, so the
    /// station is serving plain HTTP for now.
    pub waiting_for_clock: bool,
}

/// The CA download.
pub fn router() -> Router<AppState> {
    Router::new().route("/admin/https/local-ca.crt", get(download_ca))
}

/// `GET /admin/https/local-ca.crt`: the certificate to trust, and only that.
///
/// The CA's *private key* sits in the same directory; this route serves one
/// fixed file name and takes no path, so there is nothing to traverse.
async fn download_ca(State(state): State<AppState>) -> Response {
    let Some(status) = state.https_status() else {
        return (
            StatusCode::NOT_FOUND,
            "HTTPS is not reported by this process.",
        )
            .into_response();
    };
    if status.mode != TlsMode::SelfSigned || status.waiting_for_clock {
        return (
            StatusCode::NOT_FOUND,
            "This station is not serving a certificate of its own making, so there is no local CA to download.",
        )
            .into_response();
    }
    let path = crate::tls::ca_certificate_path(&status.state_dir);
    match state.run_blocking(move |_| std::fs::read(&path)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/x-x509-ca-cert"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"birdnet-local-ca.crt\"",
                ),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "local CA certificate not readable");
            (
                StatusCode::NOT_FOUND,
                "The station's CA certificate file is not there yet. It is created when HTTPS starts.",
            )
                .into_response()
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The card. `request_https` is whether this very page arrived over HTTPS.
#[must_use]
pub fn card(state: &AppState, request_https: bool) -> String {
    let mut out = String::from(
        r#"<div class="bnb-card pad" id="https" data-screen-label="Secure connection">
  <div class="bnb-eyebrow">Secure connection · HTTPS</div>"#,
    );
    let help = r#"See <a href="/help/admin/remote-access">Remote Access &amp; Security</a> for each step in full."#;
    match state.https_status() {
        None => out.push_str(
            r#"<div class="st-list-row"><div class="lr-main"><div>This process does not report its HTTPS setting.</div></div></div>"#,
        ),
        Some(s) if s.waiting_for_clock => {
            let _ = write!(
                out,
                r#"<div class="st-warn">HTTPS is switched on but is <b>waiting for the clock</b>: the station will not mint a certificate valid from 1970, so it is serving plain HTTP until the clock is set (check <code>timedatectl</code>). It then restarts itself into HTTPS.</div><p class="lr-sub">{help}</p>"#
            );
        }
        Some(s) if s.mode == TlsMode::Off => {
            let lock = if request_https {
                ""
            } else {
                r#"<div class="st-warn">This page came over plain HTTP: on a shared network, whatever you type here, the admin password included, can be read. On a network only you use, that may be fine.</div>"#
            };
            let _ = write!(
                out,
                r#"{lock}
  <div class="st-list-row"><div class="lr-main"><div>HTTPS is off. The dashboard speaks plain HTTP.</div>
  <div class="lr-sub">How to turn it on, easiest first:</div></div></div>
  <ol class="lr-sub">
    <li><b>Encrypt the traffic, one setting.</b> Put <code>TLS_MODE=self-signed</code> in <code>birdnet.conf</code> and restart. The station makes its own certificate authority and certificate, rotates them before they expire, and shows this card's download link. Browsers warn once, until you trust the authority.</li>
    <li><b>A real certificate.</b> <code>TLS_MODE=manual</code> with <code>TLS_CERT</code> and <code>TLS_KEY</code> pointing at your files (certbot, an internal CA). A renewal written to those files is served without a restart.</li>
    <li><b>Something else terminates TLS.</b> A reverse proxy such as Caddy, or a private network such as Tailscale, keeps the station itself on plain HTTP.</li>
  </ol>
  <p class="lr-sub">{help}</p>"#
            );
        }
        Some(s) => {
            let addr = s
                .https_addr
                .as_deref()
                .map_or_else(String::new, |a| format!(" at <code>{}</code>", escape_html(a)));
            if s.mode == TlsMode::Manual {
                let _ = write!(
                    out,
                    r#"<div class="st-list-row"><div class="lr-main"><div>HTTPS is on{addr}, with the certificate you supplied.</div>
  <div class="lr-sub">A renewed certificate written to the same files is picked up without a restart.</div></div></div>"#
                );
            } else {
                let expiry = crate::tls::self_signed_expiry(&s.state_dir).map_or_else(
                    || "its expiry is not recorded yet".to_owned(),
                    |e| {
                        let days = e.leaf_days_left(now_secs());
                        format!(
                            "the certificate runs to <b>{}</b> ({days} day{} left) and is renewed automatically before then",
                            e.leaf_expires_on(),
                            if days == 1 { "" } else { "s" }
                        )
                    },
                );
                let _ = write!(
                    out,
                    r#"<div class="st-list-row"><div class="lr-main"><div>HTTPS is on{addr}, with a certificate the station made itself.</div>
  <div class="lr-sub">{expiry}.</div></div></div>
  <div class="st-list-row"><div class="lr-main"><div>Trust it once, and the browser warning goes away</div>
  <div class="lr-sub">Download the station's own certificate authority and add it to the device that opens this page (a phone's or computer's <i>certificate</i> or <i>trusted roots</i> settings). It is a public file; the key that signs with it never leaves the station, and it stays valid across the certificate's renewals.</div></div></div>
  <p><a class="btn btn-primary" href="/admin/https/local-ca.crt" download>Download the certificate</a></p>"#
                );
            }
            if !request_https {
                out.push_str(
                    r#"<p class="lr-sub">This page itself came over plain HTTP. Use the <code>https://</code> address of the station to be encrypted from here on.</p>"#,
                );
            }
            let _ = write!(out, r#"<p class="lr-sub">{help}</p>"#);
        }
    }
    out.push_str(
        r#"
  <details class="lr-sub"><summary>Why there is no upload or generate button here</summary>
  <p><b>Upload:</b> the page you would use to fix a plain-HTTP station is itself plain HTTP, so a form for a private key would send the key across the network in the clear, to set up the thing that protects it. The certificate and key are files on the station.</p>
  <p><b>Generate:</b> the station already generates and renews its own certificate, and the mode is read when the listener starts, so a button could only set a value that needs a restart to take effect.</p></details>
</div>"#,
    );
    out
}

/// Whether the request that reached a handler came over HTTPS, as the
/// rate-limit layer recorded it.
#[must_use]
pub fn request_is_https(scheme: Option<&Extension<RequestScheme>>) -> bool {
    scheme.is_some_and(|Extension(s)| s.https)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt as _;

    fn station() -> AppState {
        AppState::from_connection(
            rusqlite::Connection::open_in_memory().unwrap(),
            PathBuf::from(":memory:"),
        )
    }

    fn status(mode: TlsMode, dir: &std::path::Path) -> HttpsStatus {
        HttpsStatus {
            mode,
            https_addr: Some("0.0.0.0:8503".into()),
            state_dir: dir.to_path_buf(),
            waiting_for_clock: false,
        }
    }

    async fn get(state: &AppState, path: &str) -> (StatusCode, Vec<u8>, Option<String>) {
        let res = router()
            .with_state(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let st = res.status();
        let disp = res
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let b = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (st, b.to_vec(), disp)
    }

    #[tokio::test]
    async fn the_ca_certificate_is_served_as_a_download_and_the_key_never_is() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("local-ca.crt"),
            b"-----BEGIN CERTIFICATE-----\nCA\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("local-ca.key"),
            b"-----BEGIN PRIVATE KEY-----\nSECRET\n",
        )
        .unwrap();
        let state = station();
        state.set_https_status(status(TlsMode::SelfSigned, dir.path()));
        let (code, body, disp) = get(&state, "/admin/https/local-ca.crt").await;
        assert_eq!(code, StatusCode::OK);
        assert!(body.starts_with(b"-----BEGIN CERTIFICATE"));
        assert!(!String::from_utf8_lossy(&body).contains("SECRET"));
        assert!(disp.unwrap().contains("attachment"));
        for other in [
            "/admin/https/local-ca.key",
            "/admin/https/..%2Flocal-ca.key",
            "/admin/https/server.key",
        ] {
            let (code, body, _) = get(&state, other).await;
            assert_eq!(code, StatusCode::NOT_FOUND, "{other}");
            assert!(!String::from_utf8_lossy(&body).contains("SECRET"));
        }
    }

    #[tokio::test]
    async fn there_is_no_ca_to_download_unless_the_station_made_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("local-ca.crt"),
            b"stale from an earlier mode",
        )
        .unwrap();
        let state = station();
        assert_eq!(
            get(&state, "/admin/https/local-ca.crt").await.0,
            StatusCode::NOT_FOUND,
            "no status"
        );
        for mode in [TlsMode::Off, TlsMode::Manual] {
            state.set_https_status(status(mode, dir.path()));
            assert_eq!(
                get(&state, "/admin/https/local-ca.crt").await.0,
                StatusCode::NOT_FOUND,
                "{mode}"
            );
        }
        // Self-signed but the file is not there yet: a plain 404, not a 500.
        let empty = tempfile::tempdir().unwrap();
        state.set_https_status(status(TlsMode::SelfSigned, empty.path()));
        assert_eq!(
            get(&state, "/admin/https/local-ca.crt").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn each_mode_says_what_is_true_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let state = station();

        let none = card(&state, false);
        assert!(none.contains("does not report"), "{none}");

        state.set_https_status(status(TlsMode::Off, dir.path()));
        let off = card(&state, false);
        assert!(off.contains("HTTPS is off"), "{off}");
        assert!(off.contains("TLS_MODE=self-signed") && off.contains("TLS_MODE=manual"));
        assert!(
            off.contains("plain HTTP"),
            "an unencrypted page is told so: {off}"
        );
        assert!(
            !off.contains("local-ca.crt"),
            "no download offered for no CA: {off}"
        );

        state.set_https_status(status(TlsMode::SelfSigned, dir.path()));
        let ss = card(&state, true);
        assert!(
            ss.contains("made itself") && ss.contains("/admin/https/local-ca.crt"),
            "{ss}"
        );
        assert!(
            ss.contains("its expiry is not recorded yet"),
            "an unknown expiry is not invented: {ss}"
        );
        assert!(!ss.contains("came over plain HTTP"), "{ss}");

        state.set_https_status(status(TlsMode::Manual, dir.path()));
        let man = card(&state, true);
        assert!(
            man.contains("certificate you supplied") && !man.contains("local-ca.crt"),
            "{man}"
        );

        let mut waiting = status(TlsMode::SelfSigned, dir.path());
        waiting.waiting_for_clock = true;
        state.set_https_status(waiting);
        let w = card(&state, false);
        assert!(
            w.contains("waiting for the clock") && !w.contains("local-ca.crt"),
            "{w}"
        );
        // Every state says why there is no upload form.
        for html in [&off, &ss, &man, &w] {
            assert!(html.contains("Why there is no upload"), "{html}");
        }
    }

    #[test]
    fn a_self_signed_station_reports_its_real_expiry() {
        let dir = tempfile::tempdir().unwrap();
        crate::tls::ensure_self_signed(dir.path(), &["localhost".to_owned()], 397).expect("mint");
        let state = station();
        state.set_https_status(status(TlsMode::SelfSigned, dir.path()));
        let html = card(&state, true);
        assert!(
            html.contains("days left") || html.contains("day left"),
            "{html}"
        );
        assert!(html.contains("runs to <b>20"), "{html}");
    }
}
