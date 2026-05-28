//! In-UI configuration diagnostics (`/admin/doctor`).
//!
//! Surfaces the configuration half of the CLI `birdnet-behavior --doctor` in
//! the browser, reusing the canonical validator (`birdnet_core::config::
//! validate`) so the two cannot drift. This is the in-process, side-effect-free
//! subset — config parsing plus range/consistency checks. The audio-device,
//! model, disk-space, and network checks stay in the CLI doctor: they live in
//! the binary crate, and some (the audio probe) would contend with the running
//! capture daemon if run from a live server.

use axum::extract::State;
use axum::response::Html;
use axum::{Router, routing::get};

use std::fmt::Write as _;

use birdnet_core::config::Config;
use birdnet_core::config::validate::{self, Finding, Severity};

use super::admin_shell;
use crate::routes::pages::escape_html;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/doctor", get(doctor_page))
}

async fn doctor_page(State(state): State<AppState>) -> Html<String> {
    let body = state.config_path().map_or_else(no_config_body, |path| {
        let shown = path.display().to_string();
        match Config::load_from(path) {
            Ok(config) => findings_body(&shown, &validate::validate(&config)),
            Err(e) => load_error_body(&shown, &e.to_string()),
        }
    });
    Html(admin_shell("Diagnostics", "doctor", &body))
}

const CLI_NOTE: &str = r#"<hr style="border:none;border-top:1px solid var(--border-2);margin:20px 0;">
<p style="opacity:.8;font-size:14px;">This page covers <strong>configuration</strong> checks only. For audio-device,
model, disk-space, and network checks, run <code>birdnet-behavior --doctor</code> on the host —
it shares the daemon's view of the audio device.</p>"#;

fn card_open() -> String {
    // O-20 — drop the troubleshooting mdBook link next to the page heading.
    let help_link =
        crate::routes::pages::help::help_link(crate::routes::pages::help::Topic::Troubleshooting);
    format!(
        r#"<section class="bnb-card" style="max-width:820px;margin:8px auto;padding:24px 28px;">
<div style="display:flex;align-items:baseline;gap:14px;flex-wrap:wrap;margin:0 0 6px;">
  <h1 style="margin:0;font-size:22px;">Configuration diagnostics</h1>
  {help_link}
</div>"#
    )
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

    let mut out = card_open();
    let _ = write!(
        out,
        r#"<p class="mono" style="opacity:.7;margin:0 0 16px;">{}</p>"#,
        escape_html(path)
    );

    if errors.is_empty() && warnings.is_empty() {
        out.push_str(
            r#"<p><span class="bnb-pill moss"><span class="bnb-dot"></span> All configuration checks passed</span></p>"#,
        );
    } else {
        let _ = write!(
            out,
            r#"<p style="margin:0 0 4px;">{} error(s), {} warning(s).</p>"#,
            errors.len(),
            warnings.len()
        );
    }

    if !errors.is_empty() {
        out.push_str(
            r#"<h2 style="font-size:16px;margin:18px 0 8px;">Errors — these prevent normal operation</h2>"#,
        );
        out.push_str(&render_findings(&errors));
    }
    if !warnings.is_empty() {
        out.push_str(
            r#"<h2 style="font-size:16px;margin:18px 0 8px;">Warnings — functionality may be degraded</h2>"#,
        );
        out.push_str(&render_findings(&warnings));
    }

    out.push_str(CLI_NOTE);
    out.push_str("</section>");
    out
}

fn render_findings(findings: &[&Finding]) -> String {
    let mut out = String::new();
    for f in findings {
        let _ = write!(
            out,
            r#"<div style="border-left:3px solid var(--border-2);padding:8px 14px;margin:8px 0;border-radius:var(--r-sm);">
<div class="mono" style="font-weight:600;">{key}</div>
<div style="margin:2px 0;">{msg}</div>
<div style="opacity:.8;font-size:14px;"><strong>Fix:</strong> {rem}</div>
</div>"#,
            key = escape_html(&f.key),
            msg = escape_html(&f.message),
            rem = escape_html(&f.remediation),
        );
    }
    out
}

fn no_config_body() -> String {
    let mut out = card_open();
    out.push_str(
        r"<p>No configuration file path is known to the running server, so it cannot be validated here.</p>",
    );
    out.push_str(CLI_NOTE);
    out.push_str("</section>");
    out
}

fn load_error_body(path: &str, err: &str) -> String {
    let mut out = card_open();
    let _ = write!(
        out,
        r#"<p><span class="bnb-pill"><span class="bnb-dot"></span> Could not read the configuration file</span></p>
<p class="mono" style="opacity:.7;">{path}</p>
<p>{err}</p>"#,
        path = escape_html(path),
        err = escape_html(err),
    );
    out.push_str(CLI_NOTE);
    out.push_str("</section>");
    out
}
