//! BirdNET-Pi migration routes.
//!
//! Provides the web UI and API for importing an existing BirdNET-Pi
//! `BirdDB.txt` (or `birds.db`) SQLite file into BirdNet-Behavior.
//!
//! # Workflow
//!
//! 1. User navigates to `GET /admin/migrate`
//! 2. User enters the path of the source file (server-side path) and/or
//!    uploads the file via the file picker.
//! 3. `POST /admin/migrate/validate` runs a read-only pre-flight check.
//! 4. `POST /admin/migrate/run` starts the import in a background task.
//! 5. `GET  /admin/migrate/progress` is polled every 2 s to track progress.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Form, Router, routing::get};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use birdnet_migrate::progress::{MigrationProgress, MigrationStage, ProgressHandle};

use crate::state::AppState;

/// Shared migration state (one active job at a time).
type MigrationState = Arc<Mutex<Option<ProgressHandle>>>;

/// Mount migration routes.
pub fn router() -> Router<AppState> {
    let migration_state: MigrationState = Arc::new(Mutex::new(None));

    Router::new()
        .route("/admin/migrate", get(migration_page))
        .route(
            "/admin/migrate/validate",
            axum::routing::post({
                move |state, form| validate_handler(state, form)
            }),
        )
        .route(
            "/admin/migrate/run",
            axum::routing::post({
                let ms = Arc::clone(&migration_state);
                move |state, form| run_handler(state, form, ms)
            }),
        )
        .route(
            "/admin/migrate/progress",
            get({
                let ms = Arc::clone(&migration_state);
                move || progress_handler(ms)
            }),
        )
}

// ---------------------------------------------------------------------------
// GET /admin/migrate
// ---------------------------------------------------------------------------

async fn migration_page(State(state): State<AppState>) -> Html<String> {
    let db_path = state.db_path().display().to_string();
    Html(render_migration_page(&db_path))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/validate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MigrateForm {
    source_path: String,
}


async fn validate_handler(
    State(state): State<AppState>,
    Form(form): Form<MigrateForm>,
) -> Result<Html<String>, StatusCode> {
    let source_path = PathBuf::from(&form.source_path);

    // Run validation in a blocking task (SQLite I/O).
    let dest_path = state.db_path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        birdnet_migrate::birdnet_pi::validate_source(&source_path)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let html = match result {
        Ok((schema, report)) => {
            let schema_name = schema.name();
            let rows = report.source_rows;
            let ok = report.passed;
            let _ = dest_path; // consumed by spawn

            let checks_html: String = report
                .checks
                .iter()
                .map(|c| {
                    let icon = if c.passed {
                        r#"<span style="color:#4ade80">✔</span>"#
                    } else if c.required {
                        r#"<span style="color:#f87171">✘</span>"#
                    } else {
                        r#"<span style="color:#fbbf24">⚠</span>"#
                    };
                    format!(
                        r#"<li style="margin-bottom:0.5rem">{icon} <strong>{name}</strong>: {detail}</li>"#,
                        name = escape_html(&c.name),
                        detail = escape_html(&c.detail),
                    )
                })
                .collect();

            let status_color = if ok { "#4ade80" } else { "#fbbf24" };
            let status_label = if ok { "Validation passed" } else { "Validation passed with warnings" };

            format!(
                r##"<div class="card" style="border-color:{status_color}">
                  <div style="font-weight:600;color:{status_color};margin-bottom:0.75rem;">
                    {status_label}
                  </div>
                  <p><strong>Schema:</strong> {schema_name}</p>
                  <p><strong>Rows to import:</strong> {rows}</p>
                  <ul style="list-style:none;padding:0;margin:0.75rem 0;">{checks_html}</ul>
                  <button class="btn btn-primary"
                          hx-post="/admin/migrate/run"
                          hx-include="#migrate-source-path"
                          hx-target="#migrate-status"
                          style="margin-top:0.75rem;">
                    Start Import
                  </button>
                </div>"##
            )
        }
        Err(e) => {
            format!(
                r#"<div class="card" style="border-color:#f87171">
                  <div style="font-weight:600;color:#f87171;margin-bottom:0.5rem;">
                    Validation failed
                  </div>
                  <p>{err}</p>
                </div>"#,
                err = escape_html(&e.to_string())
            )
        }
    };

    Ok(Html(html))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/run
// ---------------------------------------------------------------------------

async fn run_handler(
    State(state): State<AppState>,
    Form(form): Form<MigrateForm>,
    migration_state: MigrationState,
) -> Result<Html<String>, StatusCode> {
    let source_path = PathBuf::from(form.source_path);
    let dest_path = state.db_path().to_path_buf();

    let progress = ProgressHandle::new();
    {
        let mut guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(progress.clone());
    }

    // Spawn a blocking task so the HTTP response returns immediately.
    tokio::task::spawn_blocking(move || {
        progress.set_stage(MigrationStage::Detecting, "Detecting schema…");
        match birdnet_migrate::birdnet_pi::run_migration(
            &source_path,
            &dest_path,
            false,
            &progress,
        ) {
            Ok(summary) => {
                tracing::info!(
                    imported = summary.imported_rows,
                    skipped = summary.skipped_rows,
                    "migration completed"
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "migration failed");
                progress.fail(e.to_string());
            }
        }
    });

    Ok(Html(
        r#"<div id="migrate-status">
          <p style="color:#94a3b8">Import started. Polling progress…</p>
          <div id="migrate-progress" hx-get="/admin/migrate/progress"
               hx-trigger="every 2s" hx-swap="outerHTML">
            <progress style="width:100%" max="100" value="0"></progress>
          </div>
        </div>"#
        .to_string(),
    ))
}

// ---------------------------------------------------------------------------
// GET /admin/migrate/progress
// ---------------------------------------------------------------------------

async fn progress_handler(migration_state: MigrationState) -> Html<String> {
    let snap: Option<MigrationProgress> = {
        let guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().map(ProgressHandle::snapshot)
    };

    let Some(p) = snap else {
        return Html(
            r#"<div id="migrate-progress" style="color:#64748b">No migration in progress.</div>"#
                .to_string(),
        );
    };

    let pct = p.percent();
    let msg = escape_html(&p.message);

    let trigger = if p.is_terminal() {
        ""
    } else {
        r#" hx-get="/admin/migrate/progress" hx-trigger="every 2s" hx-swap="outerHTML""#
    };

    let color = match p.stage {
        MigrationStage::Complete => "#4ade80",
        MigrationStage::Failed => "#f87171",
        MigrationStage::Cancelled => "#fbbf24",
        _ => "#38bdf8",
    };

    Html(format!(
        r#"<div id="migrate-progress"{trigger}>
          <p style="color:{color};margin-bottom:0.5rem;">{msg}</p>
          <div style="background:#1e293b;border-radius:9999px;height:8px;overflow:hidden;">
            <div style="background:{color};height:100%;width:{pct}%;transition:width 0.3s;"></div>
          </div>
          <p style="color:#64748b;font-size:0.8rem;margin-top:0.25rem;">
            {imported} / {total} rows
          </p>
        </div>"#,
        pct = pct,
        imported = p.rows_imported,
        total = p.rows_total,
    ))
}

// ---------------------------------------------------------------------------
// Page HTML
// ---------------------------------------------------------------------------

fn render_migration_page(dest_db_path: &str) -> String {
    // Use r##"..."## to allow "#id" HTMX selectors inside the string.
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>BirdNET-Pi Migration - BirdNet-Behavior</title>
    <script src="/static/htmx.min.js"></script>
    <style>
      body {{ background: #0f172a; color: #e2e8f0; font-family: system-ui,sans-serif; }}
      .container {{ max-width: 860px; margin: 0 auto; padding: 2rem 1rem; }}
      nav a {{ color: #94a3b8; text-decoration: none; margin-right: 1.5rem; }}
      nav a:hover {{ color: #38bdf8; }}
      .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 0.75rem;
               padding: 1.5rem; margin-bottom: 1.5rem; }}
      label {{ display: block; font-size: 0.85rem; color: #94a3b8; margin-bottom: 0.25rem; }}
      input {{ width: 100%; background: #0f172a; border: 1px solid #334155;
               border-radius: 0.375rem; padding: 0.5rem 0.75rem; color: #e2e8f0;
               font-size: 0.875rem; box-sizing: border-box; }}
      .btn {{ padding: 0.5rem 1.5rem; border-radius: 0.375rem; border: none;
               cursor: pointer; font-weight: 600; font-size: 0.875rem; }}
      .btn-primary {{ background: #0ea5e9; color: #fff; }}
      .btn-primary:hover {{ background: #38bdf8; }}
      .hint {{ font-size: 0.75rem; color: #64748b; margin-top: 0.25rem; }}
      code {{ background: #0f172a; border: 1px solid #334155; padding: 0.1em 0.4em;
               border-radius: 0.2rem; font-family: monospace; font-size: 0.875rem; }}
    </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/migrate">Migration</a>
    <a href="/admin/system">System</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:0.5rem;color:#f1f5f9;">
    BirdNET-Pi Migration
  </h1>
  <p style="color:#94a3b8;margin-bottom:1.5rem;">
    Import your existing BirdNET-Pi detection history into BirdNet-Behavior.
    Your source file is <strong>never modified</strong>.
  </p>

  <div class="card" style="border-color:#334155">
    <div style="font-weight:600;color:#38bdf8;margin-bottom:0.75rem;">How it works</div>
    <ol style="padding-left:1.25rem;line-height:1.8;color:#cbd5e1;">
      <li>Shut down BirdNET-Pi services
          (<code>sudo systemctl stop birdnet_analysis birdnet_recording</code>).</li>
      <li>Note the path to your BirdNET-Pi database (usually
          <code>~/BirdNET-Pi/scripts/BirdDB.txt</code>).</li>
      <li>Enter the path below and click <strong>Validate</strong> to check the file.</li>
      <li>Click <strong>Start Import</strong> to copy detections into BirdNet-Behavior.</li>
      <li>Your original BirdNET-Pi installation is untouched and safe.</li>
    </ol>
    <p style="color:#64748b;font-size:0.85rem;margin-top:0.5rem;">
      Destination database: <code>{dest_db_path}</code>
    </p>
  </div>

  <div class="card">
    <div style="font-weight:600;color:#f1f5f9;margin-bottom:1rem;">Source database</div>
    <label for="migrate-source-path">Path on this server</label>
    <input id="migrate-source-path" name="source_path"
           placeholder="/home/pi/BirdNET-Pi/scripts/BirdDB.txt"
           style="margin-bottom:0.75rem;">
    <p class="hint">Absolute path to the BirdNET-Pi BirdDB.txt or any .db file</p>
    <div style="margin-top:1rem;display:flex;gap:0.75rem;flex-wrap:wrap;">
      <button class="btn btn-primary"
              hx-post="/admin/migrate/validate"
              hx-include="#migrate-source-path"
              hx-target="#validate-result"
              hx-swap="innerHTML">
        Validate
      </button>
    </div>
  </div>

  <div id="validate-result"></div>
  <div id="migrate-status"></div>
</div>
</body>
</html>"##
    )
}

/// Escape special HTML characters.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_special_chars() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html(r#"say "hi""#), "say &quot;hi&quot;");
    }

    #[test]
    fn render_migration_page_contains_key_elements() {
        let html = render_migration_page("/home/pi/birds.db");
        assert!(html.contains("/admin/migrate/validate"));
        assert!(html.contains("BirdNET-Pi"));
        assert!(html.contains("/home/pi/birds.db"));
    }
}
