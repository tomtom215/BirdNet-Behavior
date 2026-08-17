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

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Form, Router, routing::get};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use birdnet_migrate::progress::{MigrationProgress, MigrationStage, ProgressHandle};

use crate::routes::pages::toast::{self, Toast};

use crate::state::AppState;

/// Shared migration state (one active job at a time).
type MigrationState = Arc<Mutex<Option<ProgressHandle>>>;

/// Upper bound on an uploaded BirdNET-Pi database. axum's default request-body
/// limit is a mere 2 MiB — far smaller than a real `birds.db` (tens to hundreds
/// of MB after a season of detections), so without this override every genuine
/// upload is rejected and the import feature is dead on arrival. 4 GiB
/// comfortably covers even a multi-year station; the route is admin-only (RBAC),
/// so the larger ceiling is not a public denial-of-service surface.
const MAX_UPLOAD_BYTES: usize = 4 * 1024 * 1024 * 1024;

/// Mount migration routes.
pub fn router() -> Router<AppState> {
    let migration_state: MigrationState = Arc::new(Mutex::new(None));

    Router::new()
        .route("/admin/migrate", get(migration_page))
        .route(
            "/admin/migrate/validate",
            axum::routing::post(validate_handler),
        )
        .route(
            "/admin/migrate/upload",
            axum::routing::post({
                let ms = Arc::clone(&migration_state);
                move |state, multipart| upload_and_run_handler(state, multipart, ms)
            })
            // Raise the body limit for the DB upload specifically (see
            // MAX_UPLOAD_BYTES) so a real BirdNET-Pi database isn't rejected.
            .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
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

/// The standalone `/admin/migrate` page GET folded into the Station **Data**
/// tab; its old URL permanently redirects there. The validate/run/progress
/// endpoints below keep their `/admin/migrate/...` paths.
async fn migration_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/data")
}

/// Render the BirdNET-Pi import body (no document shell).
///
/// Shared with the Station **Data** tab
/// (`crate::routes::pages::homes::station_tabs`).
pub(crate) fn migration_body(state: &AppState) -> String {
    render::migration_body(&state.db_path().display().to_string())
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/validate  (server-side path)
// ---------------------------------------------------------------------------

/// The import form's fields.
///
/// The shared `source_` prefix is not redundancy: these are the `name`
/// attributes the HTML form posts, and every one of them describes the *source*
/// station as distinct from this one — which is the whole distinction the form
/// exists to capture. Renaming them to satisfy the lint would rename the wire
/// format.
#[derive(Debug, Deserialize)]
#[allow(clippy::struct_field_names)]
struct MigrateForm {
    source_path: String,
    /// Operator's name for the source station, recorded with the import batch.
    #[serde(default)]
    source_label: Option<String>,
    /// The source station's UTC offset in seconds, if the operator gave one.
    /// Absent means "same clock as this station", which shifts nothing.
    #[serde(default, deserialize_with = "empty_string_as_none")]
    source_utc_offset_secs: Option<i64>,
}

/// Treat a blank form field as absent rather than as a parse error.
///
/// An HTML number input that the operator left alone posts `""`, which serde
/// would otherwise reject — turning "I did not answer the optional question"
/// into a 422 on the whole import.
fn empty_string_as_none<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(d)?;
    Ok(raw
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok()))
}

/// This station's configured coordinates, for the import's location check.
///
/// The migrate crate deliberately never opens the destination, so the station's
/// own location has to be handed to it. Read from the settings table rather than
/// the config file because that is where the onboarding wizard and the settings
/// form write it, and it is what every other location-dependent surface reads.
fn station_coords(state: &AppState) -> (Option<f64>, Option<f64>) {
    state.with_db(|conn| {
        let get = |key: &str| -> Option<f64> {
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                rusqlite::params![key],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
        };
        (get("latitude"), get("longitude"))
    })
}

/// The reconciliation the operator asked for on the import form.
///
/// `source_utc_offset_secs` is what they said the source station's clock was;
/// the shift applied is that minus this station's own offset, so both histories
/// end up on one clock. A blank field means "same clock", which shifts nothing.
fn import_options(form: &MigrateForm) -> birdnet_migrate::ImportOptions {
    let source_offset = form.source_utc_offset_secs;
    let shift = source_offset.map_or(0, |src| {
        let here = birdnet_db::clock::local_utc_offset_secs();
        // A timestamp written in the source's clock reads `src - here` seconds
        // early here, so adding that difference puts it on this station's clock.
        here - src
    });
    birdnet_migrate::ImportOptions {
        shift_secs: shift,
        label: form
            .source_label
            .as_ref()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        source_utc_offset_secs: source_offset,
        notes: None,
    }
}

async fn validate_handler(
    State(state): State<AppState>,
    Form(form): Form<MigrateForm>,
) -> Result<Html<String>, StatusCode> {
    let source_path = PathBuf::from(&form.source_path);
    // The station's own coordinates, so the report can say whether this file
    // came from here. The migrate crate never opens the destination, so it
    // cannot find them itself.
    let (lat, lon) = station_coords(&state);
    let result = tokio::task::spawn_blocking(move || {
        birdnet_migrate::birdnet_pi::validate_source_against_station(&source_path, lat, lon)
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
    // Stream the uploaded multipart field straight to a temp file in chunks
    // (the Migrator later opens that file read-only). Streaming — rather than
    // buffering the whole upload in memory — keeps RAM flat regardless of
    // database size: a real BirdNET-Pi `birds.db` is routinely hundreds of MB
    // and can reach several GB, and the previous `field.bytes()` + `to_vec()`
    // held two full copies in memory at once, which would OOM a Raspberry Pi.
    use tokio::io::AsyncWriteExt as _;

    let mut file_name = String::from("upload.db");
    // NamedTempFile reserves a unique path and auto-cleans it on drop (even on
    // an early return); we stream into that path asynchronously below.
    let tmp = tokio::task::spawn_blocking(|| tempfile::Builder::new().suffix(".db").tempfile())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tmp_path = tmp.path().to_path_buf();

    let mut bytes_written: u64 = 0;
    let mut found_field = false;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?
    {
        if field.name().is_none_or(|n| n != "source_file") {
            continue;
        }
        if let Some(name) = field.file_name() {
            file_name = name.to_string();
        }
        let mut out = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        while let Some(chunk) = field.chunk().await.map_err(|_| StatusCode::BAD_REQUEST)? {
            out.write_all(&chunk)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            bytes_written += chunk.len() as u64;
        }
        out.flush()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        found_field = true;
        break;
    }

    if !found_field {
        return Ok(Html(render::upload_error(
            "No file field 'source_file' in upload",
        )));
    }
    if bytes_written == 0 {
        return Ok(Html(render::upload_error("Uploaded file is empty")));
    }

    let dest_path = state.db_path().to_path_buf();

    // Validate first (read-only; never modifies the temp file).
    let validate_path = tmp_path.clone();
    let (u_lat, u_lon) = station_coords(&state);
    let val_result = tokio::task::spawn_blocking(move || {
        birdnet_migrate::birdnet_pi::validate_source_against_station(&validate_path, u_lat, u_lon)
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
            Ok(summary) => {
                tracing::info!(
                    file = %file_name,
                    imported = summary.imported_rows,
                    skipped = summary.skipped_rows,
                    "upload migration completed"
                );
                // Same rebuild the server-path `run_handler` does: the import
                // wrote back-dated history straight to SQLite, so the DuckDB
                // analytics copy must be rebuilt or the incremental startup sync
                // skips every imported row as "older than the latest already
                // synced" — uploaded history would silently never reach the
                // behavioural / time-series analytics.
                rebuild_analytics_after_import(&state, &progress);
            }
            Err(e) => {
                tracing::error!(error = %e, "upload migration failed");
                progress.fail(e.to_string());
            }
        }
    });

    // O-18: sticky warn toast so the user knows the import is in flight even
    // after navigating away from the progress card.
    Ok(toast::with(
        Html(render::import_started()),
        Toast::warn("Import running — see progress below.").sticky(),
    ))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/run  (server-side path → run)
// ---------------------------------------------------------------------------

#[allow(clippy::unused_async)] // required by axum Handler trait
async fn run_handler(
    State(state): State<AppState>,
    Form(form): Form<MigrateForm>,
    migration_state: MigrationState,
) -> Result<Html<String>, StatusCode> {
    let options = import_options(&form);
    let station = station_coords(&state);
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
        match birdnet_migrate::birdnet_pi::run_migration_with_options(
            &source_path,
            &dest_path,
            false,
            &progress,
            &options,
            station,
        ) {
            Ok(s) => {
                tracing::info!(
                    imported = s.imported_rows,
                    skipped = s.skipped_rows,
                    "migration completed"
                );
                // The import wrote back-dated history straight to SQLite. Rebuild
                // the DuckDB analytics copy so that history reaches the
                // behavioural / time-series analytics with its original
                // timestamps — the incremental startup sync would skip every
                // imported row as "older than the latest already synced".
                rebuild_analytics_after_import(&state, &progress);
            }
            Err(e) => {
                tracing::error!(error = %e, "migration failed");
                progress.fail(e.to_string());
            }
        }
    });
    // O-18: sticky warn toast so the user knows the import is in flight even
    // after navigating away from the progress card.
    Ok(toast::with(
        Html(render::import_started()),
        Toast::warn("Import running — see progress below.").sticky(),
    ))
}

/// Rebuild the `DuckDB` analytics copy from `SQLite` after a successful import
/// and surface the step in the migration progress UI.
///
/// Best-effort: the import already succeeded and lives in `SQLite`, so a failed
/// analytics rebuild is logged and the migration is still reported complete
/// (a restart re-runs the startup sync). Re-enters a non-terminal stage first
/// so the progress poller keeps running during the (potentially long) rebuild.
#[cfg(feature = "analytics")]
fn rebuild_analytics_after_import(state: &AppState, progress: &ProgressHandle) {
    if !state.has_analytics() {
        return;
    }
    progress.set_stage(MigrationStage::Verifying, "Rebuilding analytics…");
    match state.resync_analytics_full() {
        Some(Ok(rows)) => {
            tracing::info!(rows, "rebuilt analytics after import");
            progress.set_stage(
                MigrationStage::Complete,
                format!("Import complete — analytics rebuilt ({rows} rows)."),
            );
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "analytics rebuild after import failed (non-fatal)");
            progress.set_stage(
                MigrationStage::Complete,
                "Import complete — analytics rebuild failed; restart to retry.".to_string(),
            );
        }
        None => {}
    }
}

/// No-op when the `analytics` feature is disabled: there is no `DuckDB` copy to
/// rebuild, and `run_migration` has already marked the import complete.
#[cfg(not(feature = "analytics"))]
const fn rebuild_analytics_after_import(_state: &AppState, _progress: &ProgressHandle) {}

// ---------------------------------------------------------------------------
// GET /admin/migrate/progress
// ---------------------------------------------------------------------------

#[allow(clippy::unused_async)] // required by axum Handler trait
async fn progress_handler(migration_state: MigrationState) -> Html<String> {
    let snap: Option<MigrationProgress> = {
        let guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().map(ProgressHandle::snapshot)
    };
    let Some(p) = snap else {
        return Html(
            r#"<div id="migrate-progress" class="amig-empty">No migration in progress.</div>"#
                .to_string(),
        );
    };
    Html(render::progress_bar(&p))
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn migration_body_has_upload_form() {
        // The importer body now folds into the Station Data tab (the standalone
        // `/admin/migrate` GET redirects there); the body still carries the
        // upload form posting to the unchanged `/admin/migrate/upload` endpoint.
        let html = render::migration_body("/home/pi/birds.db");
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
