//! BirdNET-Pi migration routes.
//!
//! Supports two import methods:
//! - **File upload** (`POST /admin/migrate/upload`) — user uploads a `.db` or `.txt`
//!   file from the browser; written to a temp location then validated + imported.
//! - **Server path** (`POST /admin/migrate/validate` / `run`) — absolute path on the
//!   server (useful for Pi-local installs where the file is already on disk).
//!
//! The source file is **never modified**.

mod render;

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
        .route("/admin/migrate/validate", axum::routing::post(validate_handler))
        .route(
            "/admin/migrate/upload",
            axum::routing::post({
                let ms = Arc::clone(&migration_state);
                move |state, multipart| upload_and_run_handler(state, multipart, ms)
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

async fn migration_page(State(state): State<AppState>) -> Html<String> {
    Html(render::migration_page(&state.db_path().display().to_string()))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/validate  (server-side path)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct MigrateForm {
    source_path: String,
}

async fn validate_handler(
    State(_state): State<AppState>,
    Form(form): Form<MigrateForm>,
) -> Result<Html<String>, StatusCode> {
    let source_path = PathBuf::from(&form.source_path);
    let result = tokio::task::spawn_blocking(move || {
        birdnet_migrate::birdnet_pi::validate_source(&source_path)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(render::validation_result(result, false)))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/upload  (multipart upload → validate + run)
// ---------------------------------------------------------------------------

async fn upload_and_run_handler(
    State(state): State<AppState>,
    mut multipart: axum::extract::Multipart,
    migration_state: MigrationState,
) -> Result<Html<String>, StatusCode> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name = String::from("upload.db");

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        if field.name().is_some_and(|n| n == "source_file") {
            if let Some(name) = field.file_name() {
                file_name = name.to_string();
            }
            let data = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            file_bytes = Some(data.to_vec());
            break;
        }
    }

    let Some(bytes) = file_bytes else {
        return Ok(Html(render::upload_error("No file field 'source_file' in upload")));
    };
    if bytes.is_empty() {
        return Ok(Html(render::upload_error("Uploaded file is empty")));
    }

    // Write to a temp file; the Migrator opens it read-only.
    let tmp = tokio::task::spawn_blocking(move || -> std::io::Result<tempfile::NamedTempFile> {
        let mut tmp = tempfile::Builder::new().suffix(".db").tempfile()?;
        std::io::Write::write_all(&mut tmp, &bytes)?;
        Ok(tmp)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let tmp_path = tmp.path().to_path_buf();
    let dest_path = state.db_path().to_path_buf();

    // Validate first (read-only; never modifies the temp file).
    let validate_path = tmp_path.clone();
    let val_result = tokio::task::spawn_blocking(move || {
        birdnet_migrate::birdnet_pi::validate_source(&validate_path)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (schema, report, _migration_report) = match val_result {
        Ok(triple) => triple,
        Err(e) => {
            return Ok(Html(render::upload_error(&format!(
                "Validation failed for {file_name}: {e}"
            ))));
        }
    };

    if !report.passed {
        let failures: Vec<_> = report
            .checks
            .iter()
            .filter(|c| !c.passed && c.required)
            .map(|c| c.detail.as_str())
            .collect();
        return Ok(Html(render::upload_error(&format!(
            "File {file_name} failed required checks: {}",
            failures.join("; ")
        ))));
    }

    let rows_hint = schema.row_count();
    let progress = ProgressHandle::new();
    {
        let mut guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(progress.clone());
    }

    tokio::task::spawn_blocking(move || {
        let _keep_tmp = tmp; // keeps temp file alive until migration finishes
        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: rows_hint,
            message: format!("Importing {file_name}…"),
            error: None,
        });
        match birdnet_migrate::birdnet_pi::run_migration(&tmp_path, &dest_path, false, &progress) {
            Ok(summary) => tracing::info!(
                file = %file_name,
                imported = summary.imported_rows,
                skipped = summary.skipped_rows,
                "upload migration completed"
            ),
            Err(e) => {
                tracing::error!(error = %e, "upload migration failed");
                progress.fail(e.to_string());
            }
        }
    });

    Ok(Html(render::import_started()))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/run  (server-side path → run)
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
    tokio::task::spawn_blocking(move || {
        progress.set_stage(MigrationStage::Detecting, "Detecting schema…");
        match birdnet_migrate::birdnet_pi::run_migration(&source_path, &dest_path, false, &progress) {
            Ok(s) => tracing::info!(imported = s.imported_rows, skipped = s.skipped_rows, "migration completed"),
            Err(e) => { tracing::error!(error = %e, "migration failed"); progress.fail(e.to_string()); }
        }
    });
    Ok(Html(render::import_started()))
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
        return Html(r#"<div id="migrate-progress" style="color:#64748b">No migration in progress.</div>"#.to_string());
    };
    Html(render::progress_bar(&p))
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn render_migration_page_has_upload_form() {
        let html = render::migration_page("/home/pi/birds.db");
        assert!(html.contains("source_file"));
        assert!(html.contains("/admin/migrate/upload"));
        assert!(html.contains("BirdNET-Pi"));
    }

    #[test]
    fn render_upload_error_escapes_html() {
        let html = render::upload_error("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn render_import_started_has_progress_poll() {
        let html = render::import_started();
        assert!(html.contains("/admin/migrate/progress"));
    }
}
