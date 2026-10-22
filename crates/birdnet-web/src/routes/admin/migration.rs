//! BirdNET-Pi migration routes.
//!
//! Supports two import methods:
//! - **File upload** (`POST /admin/migrate/upload`) — user uploads a `.db` or `.txt`
//!   file from the browser; written to a temp location then validated + imported.
//! - **Server path** (`POST /admin/migrate/validate` / `run`) — absolute path on the
//!   server (useful for Pi-local installs where the file is already on disk).
//!
//! The source file is **never modified**.

mod clip_import;
mod conf_handlers;
pub mod conf_import;
mod conf_view;
mod render;

use crate::routes::form::HtmxForm;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use birdnet_migrate::progress::{MigrationProgress, MigrationStage, ProgressHandle};

use crate::routes::pages::toast::{self, Toast};

use birdnet_behavioral::queries::EXCLUDE_IMPORTS_SETTING;

use crate::state::AppState;

use render::UploadPreview;

/// Shared migration state (one active job at a time).
type MigrationState = Arc<Mutex<Option<ProgressHandle>>>;

/// Take the one import slot, or `None` while an import still owns it.
///
/// Decided and claimed under one lock, with the new handle already in a stage
/// that counts as running: a handle left in `Pending` between "claimed" and
/// "the worker's first update" was read as "not running", so two requests in
/// that window both started, shared one temporary file name, and each could
/// clear the other's temporaries.
fn claim_slot(slot: &MigrationState) -> Option<ProgressHandle> {
    let progress = ProgressHandle::new();
    progress.set_stage(MigrationStage::Detecting, "Starting…");
    {
        let mut guard = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().is_some_and(|p| !p.snapshot().is_terminal()) {
            return None;
        }
        *guard = Some(progress.clone());
    }
    Some(progress)
}

/// Run an import's work on the calling (blocking) thread; if it panics, the
/// stage becomes `Failed` instead of staying wherever it was.
///
/// Without this a panic in the worker left the stage non-terminal for good: the
/// page polled a bar that never moved, and every later import was refused as
/// "still running" until the station restarted.
pub(super) fn run_guarded(progress: &ProgressHandle, work: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err() {
        tracing::error!("an import worker panicked");
        progress.fail(
            "The import stopped unexpectedly. The station's log has the details. Nothing is \
             running now, so you can start it again.",
        );
    }
}

/// The message for a refused second import.
pub(super) const ALREADY_RUNNING: &str = "Another import is still running. Wait for it to finish.";

/// An uploaded file that has been validated and is waiting for the operator to
/// say "yes, import it".
///
/// # Why the upload is staged instead of imported straight away
///
/// It used to import straight away. `upload_and_run_handler` ran the same
/// validation the Server Path tab runs, refused the file if a **required**
/// check failed, and then dropped the report on the floor — the binding was
/// literally `let (schema, report, _migration_report) = …`, with `report` never
/// read again after the required-checks test.
///
/// The checks that were being discarded are the ones that matter for the
/// question this feature exists to answer. `provenance::location_check` is
/// deliberately never `required` — its own comment explains why: "merging two
/// sites is a legitimate thing to want … the job is to make that a decision
/// instead of an accident." Being non-required meant that on the Upload tab it
/// could not reach the operator at all. Neither could the duplicate count, the
/// date range or the species preview.
///
/// So an operator who uploaded another station's history saw no distance
/// warning, no preview, and no confirmation step — while the manual described
/// the Server-Path journey as if it were both.
///
/// One slot, because one import can run at a time anyway (the progress handle
/// above is also a single slot). A newer upload replaces an unconfirmed older
/// one, and its temp file is dropped with it.
///
/// The replacement is why each staging carries a [`Staged::token`]: with one
/// slot and two admins, the first could otherwise confirm the second's file
/// after reading a report about their own — reviewing one file and importing
/// another, which is the exact failure this two-step flow exists to prevent.
/// The confirm button carries the token of the report it was rendered with, and
/// a mismatch is refused rather than imported.
type StagedUpload = Arc<Mutex<Option<Staged>>>;

/// Source of [`Staged::token`]. Monotonic, not secret: the route is already
/// behind admin RBAC, so this identifies a staging rather than authorising one.
static STAGING_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A validated upload waiting for confirmation.
struct Staged {
    /// Kept alive so the file survives until the import reads it; dropping this
    /// removes it from disk, and the `-wal`/`-shm` files SQLite left beside it.
    tmp: StagedFile,
    /// The name the browser sent, for the progress message and the log line.
    file_name: String,
    /// The reconciliation the operator asked for on the upload form.
    options: birdnet_migrate::ImportOptions,
    /// Row count from the detected schema, so the progress bar has a total.
    rows_hint: u64,
    /// Identifies this staging. See [`StagedUpload`].
    token: u64,
}

/// Where an uploaded database waits for confirmation and then import.
///
/// Beside the station's own database, on the data disk, and not in the system
/// temporary directory: on a distribution whose `/tmp` is a `tmpfs` (Debian
/// Trixie) a 4 GiB upload there is RAM, charged to the service's `MemoryMax`
/// before the import has read a byte of it.
fn staging_dir(db_path: &Path) -> PathBuf {
    crate::data_volume::data_dir_of(db_path).join("migration-staging")
}

/// A staging file older than this is a leftover from a process that died; the
/// next upload removes it.
const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// An uploaded database in the staging directory.
///
/// A temporary file that removes itself, plus what opening a WAL-mode database
/// makes SQLite create beside it. The audit reproduced the leak: a read-only
/// open of an uploaded WAL database left a 32 KiB `-shm` and a `-wal` next to
/// a staged file whose own removal was handled and theirs was not.
struct StagedFile {
    file: tempfile::NamedTempFile,
}

impl StagedFile {
    /// Create one in `dir`, creating `dir` (mode 0700) and clearing stale
    /// leftovers from it first.
    fn create_in(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let stale = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > STALE_STAGING);
                if stale {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let file = tempfile::Builder::new().suffix(".db").tempfile_in(dir)?;
        Ok(Self { file })
    }

    fn path(&self) -> &Path {
        self.file.path()
    }
}

impl Drop for StagedFile {
    fn drop(&mut self) {
        let path = self.file.path().as_os_str().to_owned();
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut sibling = path.clone();
            sibling.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(sibling));
        }
    }
}

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
    let staged_upload: StagedUpload = Arc::new(Mutex::new(None));
    let staged_conf: conf_import::StagedConf = Arc::new(Mutex::new(None));
    let conf_lines: conf_import::ConfigLines = Arc::new(Mutex::new(None));

    Router::new()
        .route(
            "/admin/migrate/conf/preview",
            axum::routing::post({
                let sc = Arc::clone(&staged_conf);
                move |state, multipart| conf_handlers::preview_handler(state, multipart, sc)
            })
            // Eight small text files at most; far below the database upload's ceiling.
            .layer(DefaultBodyLimit::max(4 * 1024 * 1024)),
        )
        .route(
            "/admin/migrate/conf/apply",
            axum::routing::post({
                let sc = Arc::clone(&staged_conf);
                let cl = Arc::clone(&conf_lines);
                move |state, user, form| conf_handlers::apply_handler(state, user, sc, cl, form)
            }),
        )
        .route(
            "/admin/migrate/clips/preview",
            axum::routing::post(clip_import::preview_handler),
        )
        .route(
            "/admin/migrate/clips/run",
            axum::routing::post({
                let ms = Arc::clone(&migration_state);
                move |state, form| clip_import::run_handler(state, ms, form)
            }),
        )
        .route(
            "/admin/migrate/conf/lines",
            get({
                let cl = Arc::clone(&conf_lines);
                move |query| std::future::ready(conf_handlers::lines_handler(&cl, query))
            }),
        )
        .route("/admin/migrate", get(migration_page))
        .route(
            "/admin/migrate/validate",
            axum::routing::post(validate_handler),
        )
        .route(
            "/admin/migrate/upload",
            axum::routing::post({
                let su = Arc::clone(&staged_upload);
                move |state, multipart| upload_handler(state, multipart, su)
            })
            // Raise the body limit for the DB upload specifically (see
            // MAX_UPLOAD_BYTES) so a real BirdNET-Pi database isn't rejected.
            .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route(
            "/admin/migrate/upload/confirm",
            axum::routing::post({
                let ms = Arc::clone(&migration_state);
                let su = Arc::clone(&staged_upload);
                move |state, form| upload_confirm_handler(state, ms, su, form)
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
                move |query| progress_handler(ms, query)
            }),
        )
        .route("/admin/migrate/batches", get(batches_handler))
        .route(
            "/admin/migrate/batches/provenance",
            axum::routing::post(provenance_handler),
        )
        .route(
            "/admin/migrate/batches/delete",
            axum::routing::post(delete_batch_handler),
        )
}

/// The imported-history list.
async fn batches_handler(State(state): State<AppState>) -> Html<String> {
    Html(render_batches(state).await)
}

/// The imported-history list plus the provenance toggle, read fresh.
///
/// Both handlers render from the database rather than from what they think they
/// just changed, so a partial failure shows what is actually there.
///
/// A list that could not be read is not "No histories have been imported", and
/// a batch whose row count could not be read is not a batch of zero rows: the
/// operator decides what to delete from these numbers.
async fn render_batches(state: AppState) -> String {
    let read = state
        .run_blocking(|state| {
            state.with_read_db(|conn| {
                let batches = birdnet_db::sqlite::list_import_batches(conn)?
                    .into_iter()
                    .map(|b| {
                        let rows = birdnet_db::sqlite::import_batch_row_count(conn, b.id)?;
                        Ok((b, rows))
                    })
                    .collect::<Result<Vec<_>, birdnet_db::sqlite::DbError>>()?;
                // The toggle is a setting the operator chose; a read that
                // fails shows it off, which is its default, and the page does
                // not claim otherwise.
                let exclude = birdnet_db::settings::get_or(conn, EXCLUDE_IMPORTS_SETTING, "false")
                    .is_ok_and(|v| v == "true");
                Ok::<_, birdnet_db::sqlite::DbError>((batches, exclude))
            })
        })
        .await;
    match read {
        Ok((batches, exclude)) => render::import_batches(&batches, exclude),
        Err(e) => {
            tracing::warn!(error = %e, "import history could not be read");
            crate::routes::pages::error_states::inline("the import history")
        }
    }
}

/// Which import to remove.
#[derive(Debug, Deserialize)]
struct DeleteBatchForm {
    batch_id: i64,
}

/// Remove an import batch and every detection it brought in.
///
/// # Why this endpoint exists
///
/// Importing another station's history used to be a one-way door. The
/// pre-import warning is good — `provenance.rs` profiles the source, compares
/// its modal coordinate to this station's, and flags anything past 5 km — but
/// afterwards the operator's only options were to keep the merge forever or to
/// delete every detection they had. The page even told them the shift "stays
/// reversible", which was true of the *record* and not of the data.
///
/// Both stores are updated. `SQLite` is the one the species lists and heat map
/// read; the `DuckDB` copy is the one sessionize, funnel, retention,
/// next-species, phenology and every time-series query read, and an incremental
/// sync cannot notice a removal — so leaving it out would undo the import on
/// half the application and not the other half.
async fn delete_batch_handler(
    State(state): State<AppState>,
    HtmxForm(form): HtmxForm<DeleteBatchForm>,
) -> Html<String> {
    let batch_id = form.batch_id;
    let list_state = state.clone();

    let removed = tokio::task::spawn_blocking(move || {
        // Locked rows stay (every bulk action spares them), and so does the
        // batch record they point at. They are read first so the analytics
        // copy — which has no lock column — can be told which rows to keep;
        // deleting the whole batch there left the two stores disagreeing.
        let result = state.with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT Date, Time, Sci_Name, File_Name FROM detections \
                 WHERE import_batch_id = ?1 AND COALESCE(is_locked, 0) = 1",
            )?;
            let kept = stmt
                .query_map([batch_id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })?
                .collect::<Result<Vec<(String, String, String, Option<String>)>, _>>()?;
            drop(stmt);
            let removal = birdnet_db::sqlite::remove_import_batch(conn, batch_id)?;
            Ok::<_, birdnet_db::sqlite::DbError>((removal, kept))
        });

        // Mirror into the analytics copy whatever happened to SQLite.
        #[cfg(feature = "analytics")]
        if let Ok((_, kept)) = &result
            && let Some(Err(e)) =
                state.with_analytics(|adb| adb.delete_import_batch_keeping(batch_id, kept))
        {
            tracing::warn!(
                error = %e,
                batch_id,
                "removed an import from SQLite but not from the analytics copy; \
                 the two stores now disagree until the next full resync"
            );
        }

        result.map(|(removal, _)| removal)
    })
    .await;

    let toast = match removed {
        Ok(Ok(removal)) => {
            tracing::info!(
                batch_id,
                deleted = removal.deleted,
                kept_locked = removal.kept_locked,
                "import batch removed"
            );
            if removal.kept_locked > 0 {
                Some(crate::routes::pages::toast::Toast::info(format!(
                    "Removed {} detections. {} locked ones were kept, so the import is still listed — unlock them to remove it completely.",
                    removal.deleted, removal.kept_locked
                )))
            } else {
                Some(crate::routes::pages::toast::Toast::success(format!(
                    "Removed the import ({} detections).",
                    removal.deleted
                )))
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, batch_id, "import batch removal failed");
            Some(crate::routes::pages::toast::Toast::error(
                "The station could not remove that import. Nothing was removed.",
            ))
        }
        Err(e) => {
            tracing::warn!(error = %e, batch_id, "import batch removal task panicked");
            Some(crate::routes::pages::toast::Toast::error(
                "The station could not remove that import.",
            ))
        }
    };

    let body = Html(render_batches(list_state).await);
    match toast {
        Some(t) => crate::routes::pages::toast::with(body, t),
        None => body,
    }
}

/// Whether imported detections should count as this station's data.
#[derive(Debug, Deserialize)]
struct ProvenanceForm {
    /// Present and `"true"` when the checkbox is ticked; an unticked checkbox
    /// posts nothing at all, which is why this is an `Option` rather than a
    /// `bool` with a serde default that would read "absent" as an error.
    #[serde(default)]
    exclude: Option<String>,
}

/// Turn the analytics provenance filter on or off.
///
/// # What this governs
///
/// Migration 25 tagged every imported detection with its batch, and until
/// migration 34 no analytic read the tag: the life list, first-of-year, species
/// richness, phenology, the heat map, co-occurrence and the dawn chorus all
/// counted another site's records as this station's. `provenance.rs` warns about
/// exactly that before an import and says the damage "is not detectable after
/// the fact" — this is the control that lets an operator keep the history and
/// keep the numbers separate.
///
/// # Both engines, in one action
///
/// SQLite reads the flag through `detections_analytic`'s subquery, so it takes
/// effect on the next query with nothing to rebuild. The `DuckDB` copy has no
/// settings table, so its view carries the rule as a literal and is recreated
/// here. Doing only one of the two would leave the species lists and the
/// behavioural dashboards answering with different histories, which is worse
/// than merging everything because the disagreement is invisible.
async fn provenance_handler(
    State(state): State<AppState>,
    HtmxForm(form): HtmxForm<ProvenanceForm>,
) -> Html<String> {
    let exclude = form.exclude.as_deref() == Some("true");
    let write_state = state.clone();

    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = write_state.with_db(|conn| {
            birdnet_db::settings::set(
                conn,
                EXCLUDE_IMPORTS_SETTING,
                if exclude { "true" } else { "false" },
                birdnet_db::settings::SettingsCategory::System,
            )
        }) {
            tracing::warn!(error = %e, "could not save the analytics provenance setting");
            return;
        }

        #[cfg(feature = "analytics")]
        if let Some(Err(e)) = write_state.with_analytics(|adb| adb.set_exclude_imports(exclude)) {
            tracing::warn!(
                error = %e,
                "saved the analytics provenance setting but could not rebuild the \
                 analytics view; the two stores disagree until the next sync"
            );
        }
        tracing::info!(
            exclude_imports = exclude,
            "analytics provenance filter changed"
        );
    })
    .await;

    Html(render_batches(state).await)
}

/// The standalone `/admin/migrate` page GET folded into the Station **Data**
/// tab; its old URL permanently redirects there. The validate/run/progress
/// endpoints below keep their `/admin/migrate/...` paths.
async fn migration_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/data#import")
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
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    source_utc_offset_secs: Option<i64>,
}

/// This station's configured coordinates, for the import's location check.
///
/// The migrate crate deliberately never opens the destination, so the station's
/// own location has to be handed to it. Read from the settings table rather than
/// the config file because that is where the onboarding wizard and the settings
/// form write it, and it is what every other location-dependent surface reads.
fn station_coords(state: &AppState) -> (Option<f64>, Option<f64>) {
    state.with_read_db(|conn| {
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
    birdnet_migrate::ImportOptions {
        // Zero: the importer converts per row from `source_utc_offset_secs`
        // instead. This used to be `here - src`, with `here` read from
        // `local_utc_offset_secs()` — *today's* offset — and applied flat to a
        // multi-year history, so it was an hour out for however much of that
        // history fell under a different daylight-saving regime. Measured on a
        // Europe/Berlin host importing a UTC+0 source: wrong on three of six
        // representative timestamps. See `to_local_here` in the migrate crate.
        shift_secs: 0,
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
    HtmxForm(form): HtmxForm<MigrateForm>,
) -> Result<Html<String>, StatusCode> {
    let source_path = PathBuf::from(&form.source_path);
    // The station's own coordinates, so the report can say whether this file
    // came from here. The migrate crate never opens the destination, so it
    // cannot find them itself.
    let result = tokio::task::spawn_blocking(move || {
        let (lat, lon) = station_coords(&state);
        birdnet_migrate::birdnet_pi::validate_source_against_station(&source_path, lat, lon)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Html(render::validation_result(
        result,
        UploadPreview::ServerPath,
    )))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/upload  (multipart upload → validate + run)
// ---------------------------------------------------------------------------

/// The upload stopped arriving: the connection dropped, or the file is larger
/// than the station accepts. Answered in the result slot, because a `400` or
/// `413` is never swapped by htmx and the person would see the progress bar
/// stop and nothing else.
fn upload_cut_off(e: &dyn std::fmt::Display) -> Html<String> {
    tracing::warn!(error = %e, "migration upload stopped arriving");
    Html(render::upload_error(
        "The upload stopped before the whole file arrived, or the file is larger than this \
         station accepts. Nothing was imported. Try again, or import it from a folder on the \
         station instead.",
    ))
}

/// The station could not do something with the upload that only the station
/// can see (a full disk, a folder it cannot write); said plainly, with the
/// detail in the log.
fn upload_refused(what: &str, e: &dyn std::fmt::Display) -> Html<String> {
    tracing::error!(error = %e, what, "migration upload could not be staged");
    Html(render::upload_error(&format!(
        "The station could not {what}. Nothing was imported. Check that the disk is not full, \
         then try again."
    )))
}

async fn upload_handler(
    State(state): State<AppState>,
    mut multipart: axum::extract::Multipart,
    staged: StagedUpload,
) -> Result<Html<String>, StatusCode> {
    // Stream the uploaded multipart field straight to a temp file in chunks
    // (the Migrator later opens that file read-only). Streaming — rather than
    // buffering the whole upload in memory — keeps RAM flat regardless of
    // database size: a real BirdNET-Pi `birds.db` is routinely hundreds of MB
    // and can reach several GB, and the previous `field.bytes()` + `to_vec()`
    // held two full copies in memory at once, which would OOM a Raspberry Pi.
    use tokio::io::AsyncWriteExt as _;

    let mut file_name = String::from("upload.db");
    // The upload tab posts the same origin fields the server-path tab does, so
    // a browser user can reconcile an import's clock and name its source. They
    // arrive as ordinary text parts, in whatever order the browser sends them,
    // which is why the loop below no longer breaks the moment it has the file.
    let mut source_label: Option<String> = None;
    let mut source_utc_offset_secs: Option<i64> = None;
    // The staged file reserves a unique path in the data directory and cleans
    // up after itself on drop (even on an early return); we stream into that
    // path asynchronously below.
    let staging = staging_dir(state.db_path());
    let tmp = tokio::task::spawn_blocking(move || StagedFile::create_in(&staging))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let tmp = match tmp {
        Ok(t) => t,
        Err(e) => return Ok(upload_refused("make room to receive the upload", &e)),
    };
    let tmp_path = tmp.path().to_path_buf();

    let mut bytes_written: u64 = 0;
    let mut found_field = false;
    loop {
        let mut field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Ok(upload_cut_off(&e)),
        };
        match field.name() {
            Some("source_label") => {
                let text = match field.text().await {
                    Ok(t) => t,
                    Err(e) => return Ok(upload_cut_off(&e)),
                };
                let text = text.trim();
                if !text.is_empty() {
                    source_label = Some(text.to_owned());
                }
                continue;
            }
            Some("source_utc_offset_secs") => {
                let text = match field.text().await {
                    Ok(t) => t,
                    Err(e) => return Ok(upload_cut_off(&e)),
                };
                // A field the operator left alone posts "": absent, not zero.
                // Zero is a real answer ("the source ran on UTC") and must stay
                // distinguishable from "I did not say".
                source_utc_offset_secs = text.trim().parse().ok();
                continue;
            }
            Some("source_file") => {}
            _ => continue,
        }
        if let Some(name) = field.file_name() {
            file_name = name.to_string();
        }
        let mut out = match tokio::fs::File::create(&tmp_path).await {
            Ok(f) => f,
            Err(e) => return Ok(upload_refused("save the uploaded file", &e)),
        };
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if let Err(e) = out.write_all(&chunk).await {
                        return Ok(upload_refused("save the uploaded file", &e));
                    }
                    bytes_written += chunk.len() as u64;
                }
                Ok(None) => break,
                Err(e) => return Ok(upload_cut_off(&e)),
            }
        }
        if let Err(e) = out.flush().await {
            return Ok(upload_refused("save the uploaded file", &e));
        }
        found_field = true;
    }

    if !found_field {
        return Ok(Html(render::upload_error(
            "No file field 'source_file' in upload",
        )));
    }
    if bytes_written == 0 {
        return Ok(Html(render::upload_error("Uploaded file is empty")));
    }

    // Validate first (read-only; never modifies the temp file).
    let validate_path = tmp_path.clone();
    let coords_state = state.clone();
    let val_result = tokio::task::spawn_blocking(move || {
        let (u_lat, u_lon) = station_coords(&coords_state);
        birdnet_migrate::birdnet_pi::validate_source_against_station(&validate_path, u_lat, u_lon)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let (schema, report, migration_report) = match val_result {
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

    // The reconciliation the operator asked for on the upload form. `u_lat` /
    // `u_lon` were already read above to validate the file; the confirm handler
    // passes them on, which is what lets the batch record how far away the
    // source station was. Calling the bare `run_migration` — which is
    // `run_migration_with_options` with `ImportOptions::default()` and
    // `station = (None, None)` — left `station_lat`, `station_lon` and
    // `distance_km` NULL on every browser import, so the Patterns note that
    // names an imported foreign site could never fire: it keys on a distance
    // nothing had computed.
    let options = import_options(&MigrateForm {
        source_path: String::new(),
        source_label,
        source_utc_offset_secs,
    });

    // Hold the validated file and show the operator the report, rather than
    // importing behind their back. See [`StagedUpload`].
    let token = STAGING_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    {
        let mut guard = staged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(Staged {
            tmp,
            file_name: file_name.clone(),
            options,
            rows_hint: schema.row_count(),
            token,
        });
    }

    Ok(Html(render::validation_result(
        Ok((schema, report, migration_report)),
        UploadPreview::Staged {
            file_name: &file_name,
            token,
        },
    )))
}

// ---------------------------------------------------------------------------
// POST /admin/migrate/upload/confirm  (run the staged upload)
// ---------------------------------------------------------------------------

/// What the confirm button posts: the identity of the report it was rendered
/// with. See [`StagedUpload`].
#[derive(serde::Deserialize)]
struct ConfirmForm {
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    token: Option<u64>,
}

/// Import the file the operator uploaded and then confirmed.
///
/// Takes the staged upload out of its slot, so a double-submit cannot start two
/// imports of the same file — and only if its token matches the report the
/// button came from, so a second upload arriving in between is not imported by
/// someone who reviewed the first.
async fn upload_confirm_handler(
    State(state): State<AppState>,
    migration_state: MigrationState,
    staged: StagedUpload,
    HtmxForm(form): HtmxForm<ConfirmForm>,
) -> Result<Html<String>, StatusCode> {
    // Before the staging is taken: a caller dropped at this await (a client
    // that hung up) must leave the upload staged, not discard it.
    let station = state.run_blocking(station_coords).await;
    let staged_now = {
        let mut guard = staged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Peek before taking: a token that does not match belongs to a report
        // the operator never saw, and taking it would discard a staging its own
        // owner is still entitled to confirm.
        match guard.as_ref() {
            Some(s) if form.token.is_none_or(|t| t == s.token) => guard.take(),
            _ => None,
        }
    };

    let Some(Staged {
        tmp,
        file_name,
        options,
        rows_hint,
        token: staged_token,
    }) = staged_now
    else {
        return Ok(Html(render::upload_error(
            "That upload is no longer staged — it was already imported, or \
             another upload replaced it. Upload the file again.",
        )));
    };

    let Some(progress) = claim_slot(&migration_state) else {
        // The upload stays staged: refusing the click must not cost the file.
        *staged
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Staged {
            tmp,
            file_name,
            options,
            rows_hint,
            token: staged_token,
        });
        return Ok(Html(render::upload_error(ALREADY_RUNNING)));
    };
    let tmp_path = tmp.path().to_path_buf();
    let dest_path = state.db_path().to_path_buf();

    tokio::task::spawn_blocking(move || {
        let worker_progress = progress.clone();
        run_guarded(&worker_progress, move || {
            let _keep_tmp = tmp; // keeps the temp file alive until migration finishes
            progress.update(MigrationProgress {
                stage: MigrationStage::Importing,
                rows_imported: 0,
                rows_total: rows_hint,
                message: format!("Importing {file_name}…"),
                error: None,
            });
            match birdnet_migrate::birdnet_pi::run_migration_with_options(
                &tmp_path, &dest_path, false, &progress, &options, station,
            ) {
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

async fn run_handler(
    State(state): State<AppState>,
    HtmxForm(form): HtmxForm<MigrateForm>,
    migration_state: MigrationState,
) -> Result<Html<String>, StatusCode> {
    let options = import_options(&form);
    let station = state.run_blocking(station_coords).await;
    let source_path = PathBuf::from(form.source_path);
    let dest_path = state.db_path().to_path_buf();
    let Some(progress) = claim_slot(&migration_state) else {
        return Ok(Html(render::upload_error(ALREADY_RUNNING)));
    };
    tokio::task::spawn_blocking(move || {
        let worker_progress = progress.clone();
        run_guarded(&worker_progress, move || {
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

/// Which step is asking: the history import (the default) or the recordings.
#[derive(Debug, Deserialize)]
struct ProgressQuery {
    #[serde(rename = "for")]
    step: Option<String>,
}

#[allow(clippy::unused_async)] // required by axum Handler trait
async fn progress_handler(
    migration_state: MigrationState,
    axum::extract::Query(query): axum::extract::Query<ProgressQuery>,
) -> Html<String> {
    let clips = query.step.as_deref() == Some("clips");
    let snap: Option<MigrationProgress> = {
        let guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref().map(ProgressHandle::snapshot)
    };
    let Some(p) = snap else {
        return Html(if clips {
            r#"<div id="clips-progress" class="amig-empty">Nothing is running.</div>"#.to_string()
        } else {
            r#"<div id="migrate-progress" class="amig-empty">No migration in progress.</div>"#
                .to_string()
        });
    };
    Html(if clips {
        render::clips_progress_bar(&p)
    } else {
        render::progress_bar(&p)
    })
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

    // ---- one import at a time, and a panic ends one --------------------------

    use super::{ALREADY_RUNNING, MigrationState, claim_slot, run_guarded};
    use birdnet_migrate::progress::{MigrationStage, ProgressHandle};
    use std::sync::{Arc, Barrier, Mutex};

    #[test]
    fn of_many_simultaneous_requests_exactly_one_gets_the_slot() {
        for _ in 0..50 {
            let slot: MigrationState = Arc::new(Mutex::new(None));
            let barrier = Arc::new(Barrier::new(8));
            // Spawned in a loop and joined afterwards: a lazy chain would join
            // each before the next started and never fill the barrier.
            let mut threads = Vec::new();
            for _ in 0..8 {
                let (slot, barrier) = (Arc::clone(&slot), Arc::clone(&barrier));
                threads.push(std::thread::spawn(move || {
                    barrier.wait();
                    claim_slot(&slot).is_some()
                }));
            }
            let winners = threads
                .into_iter()
                .map(|t| t.join().expect("thread"))
                .filter(|won| *won)
                .count();
            assert_eq!(winners, 1, "{ALREADY_RUNNING}");
        }
    }

    #[test]
    fn a_claimed_slot_counts_as_running_before_the_worker_has_said_anything() {
        let slot: MigrationState = Arc::new(Mutex::new(None));
        let first = claim_slot(&slot).expect("free");
        let stage = first.snapshot().stage;
        assert!(
            !matches!(stage, MigrationStage::Pending) && !first.snapshot().is_terminal(),
            "{stage:?}: a request in this window was read as 'not running'"
        );
        assert!(claim_slot(&slot).is_none());
        // The counterpart: once it finishes, the next import may start.
        first.set_stage(MigrationStage::Complete, "done");
        assert!(claim_slot(&slot).is_some());
    }

    #[test]
    fn a_worker_that_panics_leaves_a_failed_import_not_a_running_one() {
        let slot: MigrationState = Arc::new(Mutex::new(None));
        let progress = claim_slot(&slot).expect("free");
        let worker = progress.clone();
        std::thread::spawn(move || run_guarded(&worker, || panic!("boom")))
            .join()
            .expect("the guard catches it");
        let snap = progress.snapshot();
        assert_eq!(snap.stage, MigrationStage::Failed);
        assert!(snap.error.as_deref().is_some_and(|e| e.contains("stopped")));
        assert!(
            claim_slot(&slot).is_some(),
            "later imports must not be refused until a restart"
        );
        // And a worker that does not panic is left exactly as it set itself.
        let ok = ProgressHandle::new();
        run_guarded(&ok, || ok.set_stage(MigrationStage::Complete, "fine"));
        assert_eq!(ok.snapshot().stage, MigrationStage::Complete);
    }
}
