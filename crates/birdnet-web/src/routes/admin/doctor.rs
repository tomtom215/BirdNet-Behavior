//! In-UI diagnostics (`/admin/doctor`, `/admin/doctor.json`,
//! `/admin/support-bundle`).
//!
//! Two layers, because they run in different places:
//!
//! * **The station's own `--doctor`**, when the binary has installed its
//!   [`Diagnostics`](crate::diagnostics::Diagnostics) hooks (`OP-1`). The page
//!   renders every check the CLI would print — audio device listing, model,
//!   clock, TLS, database, offsite, disk — `/admin/doctor.json` serves the same
//!   document `--doctor-json` prints, and `/admin/support-bundle` hands over
//!   the same redacted archive `--support-bundle` writes. Before this the
//!   entire diagnostic apparatus was reachable only over SSH.
//! * **The configuration validator** (`birdnet_core::config::validate`),
//!   always, in-process and side-effect-free. It is the fallback for a process
//!   that installed no hooks — tooling and tests — and it stays on the page
//!   beside the full report so the two cannot drift: the same validator is
//!   what the CLI doctor's configuration family runs.
//!
//! The doctor is run read-only here: a `GET` never implies `--fix`. That is
//! the binary's promise (see `helpers::diagnostics` in the binary crate), and
//! the test there holds it.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::{Router, routing::get};

use std::fmt::Write as _;

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self, Finding, Severity};

use super::admin_shell;
use crate::routes::pages::escape_html;
use crate::state::AppState;

/// Mount the diagnostics routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/doctor", get(doctor_page))
        .route("/admin/doctor.json", get(doctor_json))
        .route("/admin/support-bundle", get(support_bundle))
}

/// What a process that installed no hooks answers, on both machine routes.
///
/// `503` rather than `404`: the route exists, the capability does not in this
/// process. A monitor that polls `doctor.json` should read that as "ask the
/// station, not the tooling", not as a typo in the URL.
fn not_wired() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::json!({
            "error": "the station's diagnostics are not wired into this process; \
                      run `birdnet-behavior --doctor` on the host"
        })
        .to_string(),
    )
        .into_response()
}

async fn doctor_json(State(state): State<AppState>) -> Response {
    let Some(diagnostics) = state.diagnostics().cloned() else {
        return not_wired();
    };
    match tokio::task::spawn_blocking(move || diagnostics.doctor_json()).await {
        Ok(body) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            body,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": crate::routes::log_internal("internal error", &e)})
                .to_string(),
        )
            .into_response(),
    }
}

/// Where the bundle is assembled: beside the database, so it is on the data
/// partition (the one path a hardened unit can always write) and `tar` never
/// crosses a filesystem — the same reasoning as the CLI's staging directory.
fn bundle_scratch_path(state: &AppState) -> std::path::PathBuf {
    let dir = state
        .db_path()
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(std::env::temp_dir, std::path::Path::to_path_buf);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    dir.join(format!(
        ".birdnet-support-http-{}-{nanos}.tar.gz",
        std::process::id()
    ))
}

async fn support_bundle(State(state): State<AppState>) -> Response {
    let Some(diagnostics) = state.diagnostics().cloned() else {
        return not_wired();
    };
    let dest = bundle_scratch_path(&state);
    let result = tokio::task::spawn_blocking(move || {
        let outcome = diagnostics
            .write_support_bundle(&dest)
            .and_then(|()| std::fs::read(&dest).map_err(|e| format!("reading the bundle: {e}")));
        // The archive is handed over as the response body; nothing keeps the
        // file, and a failed attempt leaves nothing behind either.
        let _ = std::fs::remove_file(&dest);
        outcome
    })
    .await;

    match result {
        Ok(Ok(bytes)) => {
            let stamp = crate::routes::pages::today_date_string();
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "application/gzip".to_owned()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"birdnet-support-{stamp}.tar.gz\""),
                    ),
                    (header::CACHE_CONTROL, "no-store".to_owned()),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(Err(detail)) => {
            tracing::warn!(error = %detail, "support bundle could not be produced over HTTP");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": "the support bundle could not be written; the station's log has the reason"
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({"error": crate::routes::log_internal("internal error", &e)})
                .to_string(),
        )
            .into_response(),
    }
}

async fn doctor_page(State(state): State<AppState>) -> Html<String> {
    // The full report first, when the process has it. Blocking — it lists
    // audio devices and runs the deep integrity check — so off the runtime.
    let report = match state.diagnostics().cloned() {
        Some(diagnostics) => tokio::task::spawn_blocking(move || diagnostics.doctor_json())
            .await
            .ok(),
        None => None,
    };

    let config = state.config_path().map_or_else(no_config_body, |path| {
        let shown = path.display().to_string();
        match Config::load_from(path) {
            Ok(config) => findings_body(&shown, &validate::validate(&config)),
            Err(e) => load_error_body(&shown, &e.to_string()),
        }
    });

    let body = report.as_deref().map_or_else(
        || format!("{config}{CLI_NOTE}</section>"),
        |json| format!("{}{config}</section>", report_body(json)),
    );
    Html(admin_shell("Diagnostics", "doctor", &body))
}

const CLI_NOTE: &str = r#"<hr class="doc-hr">
<p class="doc-note">This page covers <strong>configuration</strong> checks only, because the station's
own diagnostics are not wired into this process. For audio-device, model, clock, disk-space and network
checks, run <code>birdnet-behavior --doctor</code> on the host.</p>"#;

fn card_open() -> String {
    // O-20 — drop the troubleshooting mdBook link next to the page heading.
    let help_link =
        crate::routes::pages::help::help_link(crate::routes::pages::help::Topic::Troubleshooting);
    format!(
        r#"<section class="bnb-card doc-card">
<div class="doc-head">
  <h1>Diagnostics</h1>
  {help_link}
</div>"#
    )
}

/// The pill class for a doctor verdict, from the design system's status
/// vocabulary: green for pass, dawn for warn, the rare-red for fail, and the
/// muted paused pill for a check that did not apply.
const fn status_pill(status: &str) -> (&'static str, &'static str) {
    match status.as_bytes() {
        b"pass" => ("moss", "PASS"),
        b"warn" => ("dawn", "WARN"),
        b"fail" => ("rare", "FAIL"),
        _ => ("paused", "SKIP"),
    }
}

/// Render the doctor's JSON report — the same document `--doctor-json`
/// prints — as the top of the page.
///
/// A document that does not parse is shown as text rather than dropped: the
/// operator came here for the diagnostic, and a rendering bug in this page
/// must not stand between them and it.
fn report_body(json: &str) -> String {
    let mut out = card_open();
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(json) else {
        let _ = write!(
            out,
            r#"<p class="doc-note">The diagnostic did not render as a table; here it is as text.</p>
<pre class="mono">{}</pre>"#,
            escape_html(json)
        );
        return out;
    };

    let summary = &doc["summary"];
    let n = |k: &str| summary[k].as_u64().unwrap_or(0);
    let _ = write!(
        out,
        r#"<p class="doc-count">{} passed, {} warnings, {} errors, {} skipped — the same checks
<code>birdnet-behavior --doctor</code> runs, run now, read-only.</p>
<p class="doc-note"><a href="/admin/doctor.json">Download as JSON</a> ·
<a href="/admin/support-bundle">Download a support bundle</a> — the diagnostic, the station's version,
its redacted configuration, its own error log and the recent journal, in one archive to attach to a
bug report. Passwords, tokens and URL credentials are masked; read it before posting it anywhere public
all the same.</p>"#,
        n("passed"),
        n("warnings"),
        n("errors"),
        n("skipped"),
    );

    if let Some(checks) = doc["checks"].as_array() {
        for check in checks {
            let status = check["status"].as_str().unwrap_or("skip");
            let (class, label) = status_pill(status);
            let _ = write!(
                out,
                r#"<div class="doc-finding">
<div class="doc-finding-key"><span class="bnb-pill {class}"><span class="bnb-dot"></span> {label}</span> {name}</div>
<div class="doc-finding-msg">{msg}</div>"#,
                name = escape_html(check["name"].as_str().unwrap_or("")),
                msg = escape_html(check["message"].as_str().unwrap_or("")),
            );
            if let Some(rem) = check["remediation"].as_str() {
                let _ = write!(
                    out,
                    r#"<div class="doc-note"><strong>Fix:</strong> {}</div>"#,
                    escape_html(rem)
                );
            }
            out.push_str("</div>");
        }
    }
    out.push_str(r#"<hr class="doc-hr"><h2 class="doc-subhead">Configuration file</h2>"#);
    out
}

fn findings_body(path: &str, findings: &[Finding]) -> String {
    let errors: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    let warnings: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .collect();

    let mut out = String::new();
    let _ = write!(out, r#"<p class="mono doc-path">{}</p>"#, escape_html(path));

    if errors.is_empty() && warnings.is_empty() {
        out.push_str(
            r#"<p><span class="bnb-pill moss"><span class="bnb-dot"></span> All configuration checks passed</span></p>"#,
        );
    } else {
        let _ = write!(
            out,
            r#"<p class="doc-count">{} error(s), {} warning(s).</p>"#,
            errors.len(),
            warnings.len()
        );
    }

    if !errors.is_empty() {
        out.push_str(r#"<h2 class="doc-subhead">Errors — these prevent normal operation</h2>"#);
        out.push_str(&render_findings(&errors));
    }
    if !warnings.is_empty() {
        out.push_str(r#"<h2 class="doc-subhead">Warnings — functionality may be degraded</h2>"#);
        out.push_str(&render_findings(&warnings));
    }
    out
}

fn render_findings(findings: &[&Finding]) -> String {
    let mut out = String::new();
    for f in findings {
        let _ = write!(
            out,
            r#"<div class="doc-finding">
<div class="mono doc-finding-key">{key}</div>
<div class="doc-finding-msg">{msg}</div>
<div class="doc-note"><strong>Fix:</strong> {rem}</div>
</div>"#,
            key = escape_html(&f.key),
            msg = escape_html(&f.message),
            rem = escape_html(&f.remediation),
        );
    }
    out
}

fn no_config_body() -> String {
    r"<p>No configuration file path is known to the running server, so it cannot be validated here.</p>".to_owned()
}

fn load_error_body(path: &str, err: &str) -> String {
    format!(
        r#"<p><span class="bnb-pill"><span class="bnb-dot"></span> Could not read the configuration file</span></p>
<p class="mono doc-path tight">{path}</p>
<p>{err}</p>"#,
        path = escape_html(path),
        err = escape_html(err),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_that_is_not_json_is_still_shown() {
        let html = report_body("not json at all");
        assert!(html.contains("not json at all"));
        assert!(html.contains("as text"));
    }

    #[test]
    fn every_verdict_has_a_pill_and_a_remediation_is_rendered_when_present() {
        let html = report_body(
            r#"{"summary":{"passed":1,"warnings":1,"errors":1,"skipped":1,"exit_code":2},
                "checks":[
                  {"status":"pass","name":"A","message":"fine","remediation":null},
                  {"status":"warn","name":"B","message":"meh","remediation":"turn it"},
                  {"status":"fail","name":"C<","message":"bad","remediation":null},
                  {"status":"skip","name":"D","message":"n/a","remediation":null}]}"#,
        );
        for label in ["PASS", "WARN", "FAIL", "SKIP"] {
            assert!(html.contains(label), "{label} missing: {html}");
        }
        assert!(html.contains("<strong>Fix:</strong> turn it"));
        assert!(html.contains("C&lt;"), "names are escaped: {html}");
        assert!(html.contains("1 passed, 1 warnings, 1 errors, 1 skipped"));
        assert!(html.contains("/admin/support-bundle"));
    }
}
