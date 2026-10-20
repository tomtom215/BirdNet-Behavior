//! The one table that answers "what is this species costing me, and how do I
//! make it stop" (`N-4`).
//!
//! # The need this exists for
//!
//! A squeaky gate produces four thousand Eurasian Wrens. Removing them was
//! three screens: the species list to exclude it, the recordings browser to
//! find its clips, and nothing at all for the detections themselves. This is
//! one row per species with the numbers that matter and the three actions,
//! each confirmed and each written to the audit log.
//!
//! # Locked detections are never touched
//!
//! A single-row delete does not check the lock, because there the operator is
//! looking at the row they named. A bulk action is issued against a *species*
//! and sweeps up rows nobody is thinking about — including the one the
//! operator locked last spring because it was the first record for the county.
//!
//! So every bulk action here skips locked rows, the table shows the locked
//! count **before** the operator acts, and the result says how many were kept.
//! An operator who is not told would later find detections of a species they
//! believe they removed, and would reasonably conclude the button is broken.
//!
//! # Bytes are measured, not estimated
//!
//! Clip size is not in the database, so the byte total for a species is the
//! sum of `stat` over its clip files. That is thousands of syscalls for the
//! species this page exists for, which is why it is a **per-row action** the
//! operator asks for rather than a column computed for every species on every
//! page load. A number that made the page take twenty seconds to open would
//! not be worth having.

use std::fmt::Write as _;

use axum::extract::State;
use axum::response::{Html, IntoResponse as _, Response};
use axum::{Form, Router, routing::get, routing::post};
use serde::Deserialize;

use crate::auth_middleware::RequestUser;
use crate::routes::pages::toast::{Toast, not_applied};
use crate::state::AppState;
use birdnet_db::sqlite::{BulkOutcome, SpeciesUsage};

use super::super::admin_subpage_shell;
use crate::routes::pages::escape_html;

/// Mount the bulk species management routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/species/manage", get(manage_page))
        .route("/admin/species/manage/partial", get(manage_partial))
        .route("/admin/species/manage/measure", post(measure))
        .route("/admin/species/manage/exclude", post(exclude))
        .route("/admin/species/manage/detections", post(delete_detections))
        .route("/admin/species/manage/clips", post(delete_clips))
}

/// The form every action here takes: one species, by scientific name.
///
/// The scientific name and not the common one: it is the key the exclusion
/// list, the thresholds and the detections table are all stored under, and two
/// species can share a common name across label-file versions.
#[derive(Debug, Deserialize)]
struct SpeciesForm {
    /// Scientific name of the species to act on.
    sci_name: String,
}

async fn manage_page(State(state): State<AppState>) -> Html<String> {
    let table = rendered(state, None).await.0;
    Html(admin_subpage_shell(
        "Species storage",
        "species",
        "Species storage",
        &format!(r#"<div id="species-storage">{table}</div>"#),
    ))
}

async fn manage_partial(State(state): State<AppState>) -> Html<String> {
    rendered(state, None).await
}

/// Every species with what it costs, or the error that stopped it being read.
///
/// An unreadable table is not an empty one: [`table`] says "No detections yet."
/// for no rows, which is a statement about the station and the wrong thing to
/// tell an operator who is here to delete things.
async fn load(state: AppState) -> Result<Vec<SpeciesUsage>, birdnet_db::sqlite::DbError> {
    let names = state.names_handle();
    let mut rows = state
        .run_blocking(|s| s.with_read_db(birdnet_db::sqlite::species_disk_usage))
        .await?;
    // `com_name` is only drawn here — every action posts `sci_name` — so it is
    // the one field to put in the station's language.
    for r in &mut rows {
        r.com_name = names.name(&r.sci_name, &r.com_name).to_owned();
    }
    Ok(rows)
}

/// The table, re-read and rendered under `note`, or `note` over the failure
/// note when it could not be read.
///
/// The note is kept either way: it reports an action that did happen, and a
/// table that cannot be redrawn afterwards does not un-happen it.
async fn rendered(state: AppState, note: Option<&str>) -> Html<String> {
    match load(state).await {
        Ok(rows) => Html(table(&rows, note)),
        Err(e) => {
            tracing::warn!(error = %e, "species storage table could not be read");
            let mut out = String::new();
            if let Some(note) = note {
                let _ = write!(out, r#"<p class="empty-note mb">{note}</p>"#);
            }
            out.push_str(&crate::routes::pages::error_states::inline(
                "the species storage table",
            ));
            Html(out)
        }
    }
}

/// `POST /admin/species/manage/exclude` — stop recording this species.
///
/// Adds to the exclusion list the operator already has rather than inventing a
/// second mechanism, the same reasoning the suspect-species report gives.
///
/// "Will no longer be recorded" is only said once the list was written. A list
/// the database could not read or write is answered with a toast and the table
/// left as it was, and is not audited.
async fn exclude(
    State(state): State<AppState>,
    request_user: RequestUser,
    Form(form): Form<SpeciesForm>,
) -> Response {
    let sci_name = form.sci_name.trim().to_owned();
    if sci_name.is_empty() {
        return rendered(state, None).await.into_response();
    }
    let name = sci_name.clone();
    let result = state
        .run_blocking(move |s| {
            let result = super::handler::add_to_exclude_list(s, &name);
            if result.is_ok() {
                crate::audit::audit(
                    s,
                    Some(&request_user),
                    "species.exclude.add",
                    Some(&name),
                    Some("via=species-storage"),
                );
            }
            result
        })
        .await;
    if let Err(e) = result {
        tracing::warn!(error = %e, species = %sci_name, "species not excluded");
        return not_applied(&Toast::error(
            e.toast(&sci_name, "added to the exclusion list"),
        ));
    }
    let note = format!("{} will no longer be recorded.", escape_html(&sci_name));
    rendered(state, Some(&note)).await.into_response()
}

/// `POST /admin/species/manage/detections` — remove this species' detections.
///
/// A delete the database refused is answered with a toast and the table left
/// as it was ([`not_applied`]), not with "0 detections removed": that sentence
/// is a claim about the station's data, and "the write failed" is a different
/// claim. Nor is it audited as a delete of nothing: the audit log records what
/// changed, and a refused write changed nothing.
async fn delete_detections(
    State(state): State<AppState>,
    request_user: RequestUser,
    Form(form): Form<SpeciesForm>,
) -> Response {
    let sci_name = form.sci_name.trim().to_owned();
    if sci_name.is_empty() {
        return rendered(state, None).await.into_response();
    }
    let name = sci_name.clone();
    let result = state
        .run_blocking(move |s| {
            let result =
                s.with_db(|conn| birdnet_db::sqlite::delete_species_detections(conn, &name));
            match &result {
                Ok(outcome) => crate::audit::audit(
                    s,
                    Some(&request_user),
                    "species.detections.delete",
                    Some(&name),
                    Some(&format!(
                        "deleted={} locked_kept={}",
                        outcome.affected, outcome.locked_skipped
                    )),
                ),
                // Nothing was deleted, so nothing is audited as a delete: the
                // log records what changed, and `warn` (which is persisted)
                // records that it was tried.
                Err(e) => {
                    tracing::warn!(error = %e, species = %name, "species detections not removed");
                }
            }
            result
        })
        .await;
    let Ok(outcome) = result else {
        return not_applied(&Toast::error(format!(
            "The detections of {sci_name} were not deleted: the database refused the change."
        )));
    };
    tracing::info!(
        species = %sci_name,
        deleted = outcome.affected,
        locked_kept = outcome.locked_skipped,
        "species detections removed"
    );
    let note = outcome_note(&sci_name, outcome, "detections removed");
    rendered(state, Some(&note)).await.into_response()
}

/// `POST /admin/species/manage/clips` — reclaim this species' audio.
///
/// The rows stay. Only the files go, and the rows are marked reclaimed — see
/// `prune_species_clips` and migration 22 for why the filename is kept.
///
/// # The files are only ever removed after the mark succeeded
///
/// The order is list, mark, unlink — chosen so that being wrong costs disk
/// space rather than a player for audio that is gone
/// (`prune_species_clips`). That reasoning only holds if a failed mark stops
/// the sequence: this handler used to default a refused mark to "0 marked" and
/// go on to unlink every file anyway, which is the order it exists to avoid
/// and cannot be taken back. A failed list is the same: it used to default to
/// "no files", mark the rows, and report success having removed nothing.
async fn delete_clips(
    State(state): State<AppState>,
    request_user: RequestUser,
    Form(form): Form<SpeciesForm>,
) -> Response {
    let sci_name = form.sci_name.trim().to_owned();
    if sci_name.is_empty() {
        return rendered(state, None).await.into_response();
    }
    let name = sci_name.clone();
    let dir = state.recording_dir();

    let result = state
        .run_blocking(move |s| {
            let files = match s
                .with_read_db(|conn| birdnet_db::sqlite::species_clip_files(conn, &name))
            {
                Ok(files) => files,
                Err(e) => {
                    tracing::warn!(error = %e, species = %name, "clip list not read; nothing was changed");
                    return Err(ClipsRefused::List);
                }
            };
            let now = now_unix();
            let outcome = match s
                .with_db(|conn| birdnet_db::sqlite::prune_species_clips(conn, &name, now))
            {
                Ok(outcome) => outcome,
                Err(e) => {
                    // No file is touched: see the handler's doc comment.
                    tracing::warn!(error = %e, species = %name, "clips not marked; no file was removed");
                    return Err(ClipsRefused::Mark);
                }
            };

            let mut removed = 0usize;
            let mut failed = 0usize;
            for file in &files {
                match remove_clip(&dir, file) {
                    Ok(()) => removed += 1,
                    Err(e) => {
                        failed += 1;
                        tracing::warn!(file = %file, error = %e, "clip could not be removed");
                    }
                }
            }
            crate::audit::audit(
                s,
                Some(&request_user),
                "species.clips.delete",
                Some(&name),
                Some(&format!(
                    "marked={} files_removed={} failed={} locked_kept={}",
                    outcome.affected, removed, failed, outcome.locked_skipped
                )),
            );
            Ok((outcome, removed, failed))
        })
        .await;
    let (outcome, removed, failed) = match result {
        Ok(done) => done,
        Err(refused) => {
            return not_applied(&Toast::error(match refused {
                ClipsRefused::List => format!(
                    "The clips of {sci_name} were not deleted: the list of them could not be \
                     read from the database. Nothing was changed."
                ),
                ClipsRefused::Mark => format!(
                    "The clips of {sci_name} were not deleted: the database refused the \
                     change. No audio file was removed."
                ),
            }));
        }
    };
    tracing::info!(
        species = %sci_name,
        marked = outcome.affected,
        removed,
        failed,
        locked_kept = outcome.locked_skipped,
        "species clips reclaimed"
    );
    let mut note = outcome_note(&sci_name, outcome, "clips reclaimed");
    if failed > 0 {
        let _ = write!(
            note,
            " {failed} file(s) could not be removed — see the log."
        );
    }
    rendered(state, Some(&note)).await.into_response()
}

/// Which step of reclaiming clips the database refused.
enum ClipsRefused {
    /// The query for the species' clip files.
    List,
    /// Marking the rows reclaimed.
    Mark,
}

/// `POST /admin/species/manage/measure` — add up this species' clips on disk.
///
/// A separate action because it is `stat` per clip: for the species this page
/// exists for that is thousands of syscalls, and doing it for every species on
/// every page load would make the page unusable on the SD card it runs on.
async fn measure(State(state): State<AppState>, Form(form): Form<SpeciesForm>) -> Response {
    let sci_name = form.sci_name.trim().to_owned();
    if sci_name.is_empty() {
        return rendered(state, None).await.into_response();
    }
    let name = sci_name.clone();
    let dir = state.recording_dir();
    let measured = state
        .run_blocking(move |s| {
            let files =
                s.with_read_db(|conn| birdnet_db::sqlite::species_clip_files(conn, &name))?;
            let mut bytes = 0u64;
            let mut counted = 0usize;
            for file in &files {
                if let Some(path) = safe_clip_path(&dir, file)
                    && let Ok(meta) = std::fs::metadata(&path)
                {
                    bytes += meta.len();
                    counted += 1;
                }
            }
            Ok::<_, birdnet_db::sqlite::DbError>((bytes, counted))
        })
        .await;
    // "Using 0 B across 0 clips" is a measurement; a list that could not be
    // read is not one.
    let (bytes, counted) = match measured {
        Ok(measured) => measured,
        Err(e) => {
            tracing::warn!(error = %e, species = %sci_name, "species clips not listed");
            return not_applied(&Toast::error(format!(
                "{sci_name} could not be measured: its clips could not be listed from the database."
            )));
        }
    };

    let note = format!(
        "{} is using {} across {counted} clip(s) on disk.",
        escape_html(&sci_name),
        human_bytes(bytes)
    );
    rendered(state, Some(&note)).await.into_response()
}

/// Seconds since the Unix epoch, saturating rather than wrapping.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Resolve a stored `File_Name` under the recordings directory, refusing
/// anything that escapes it.
///
/// `File_Name` comes from the database, which on a migrated station holds
/// whatever BirdNET-Pi wrote there. This page **unlinks** what it resolves, so
/// a `..` reaching outside the recordings tree would delete an operator's
/// files. Checked here rather than trusted, the same way the spectrogram route
/// checks before it reads.
fn safe_clip_path(dir: &std::path::Path, file_name: &str) -> Option<std::path::PathBuf> {
    let candidate = dir.join(file_name);
    let canonical = candidate.canonicalize().ok()?;
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    canonical.starts_with(&root).then_some(canonical)
}

/// Remove one clip, refusing any path that leaves the recordings directory.
fn remove_clip(dir: &std::path::Path, file_name: &str) -> std::io::Result<()> {
    let Some(path) = safe_clip_path(dir, file_name) else {
        // Already gone is success — the mark is what matters, and retention
        // may have reclaimed the file first.
        if dir.join(file_name).exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "clip path resolves outside the recordings directory",
            ));
        }
        return Ok(());
    };
    std::fs::remove_file(path)
}

/// Say what happened, including what was deliberately kept.
fn outcome_note(sci_name: &str, outcome: BulkOutcome, what: &str) -> String {
    let mut note = format!("{}: {} {what}.", escape_html(sci_name), outcome.affected);
    if outcome.locked_skipped > 0 {
        let _ = write!(
            note,
            " {} locked detection(s) were kept — unlock them first if you meant to include them.",
            outcome.locked_skipped
        );
    }
    note
}

/// Bytes in the units a person reads.
fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.0} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// The table itself, re-rendered by every action.
fn table(rows: &[SpeciesUsage], note: Option<&str>) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(
        r#"<div class="section-title">Every species this station has recorded</div>
  <p class="hint">What each species has cost you, and the three ways to stop it. <strong>Exclude</strong> stops it being recorded from now on and changes nothing already stored. <strong>Delete detections</strong> removes its rows. <strong>Delete clips</strong> reclaims the audio but keeps the rows, so the record of what was heard survives. Detections you have <em>locked</em> are never touched by any of them.</p>"#,
    );
    if let Some(note) = note {
        let _ = write!(out, r#"<p class="empty-note mb">{note}</p>"#);
    }

    if rows.is_empty() {
        out.push_str(r#"<p class="empty-note mb">No detections yet.</p>"#);
        return out;
    }

    out.push_str(
        r#"<table class="thr-table"><thead><tr>
  <th class="cell-left">Species</th><th>Detections</th><th>Clips</th><th>Locked</th>
  <th>Last heard</th><th class="cell-left">Actions</th>
</tr></thead><tbody>"#,
    );
    for r in rows {
        let sci = escape_html(&r.sci_name);
        let com = escape_html(&r.com_name);
        let last = escape_html(&r.last_seen);
        let locked = if r.locked > 0 {
            format!(r#"<span class="bnb-pill">{}</span>"#, r.locked)
        } else {
            "—".to_owned()
        };
        let _ = write!(
            out,
            r##"<tr>
  <td>{com}<br><span class="hint">{sci}</span></td>
  <td class="cell-center">{detections}</td>
  <td class="cell-center">{clips}</td>
  <td class="cell-center">{locked}</td>
  <td class="cell-center mono">{last}</td>
  <td class="cell-right">
    <form hx-post="/admin/species/manage/measure" hx-target="#species-storage" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <button type="submit" class="bnb-btn">Measure</button>
    </form>
    <form hx-post="/admin/species/manage/exclude" hx-target="#species-storage" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <button type="submit" class="bnb-btn"
              data-confirm-action="hx-post"
              data-confirm-url="/admin/species/manage/exclude"
              data-confirm-title="Stop recording this species"
              data-confirm-body="Stop recording {com}? Everything already stored is kept; you can undo this under Species."
              data-confirm-confirm-label="Exclude">Exclude</button>
    </form>
    <form hx-post="/admin/species/manage/detections" hx-target="#species-storage" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <button type="submit" class="btn btn-danger del-btn"
              data-confirm-action="hx-post"
              data-confirm-url="/admin/species/manage/detections"
              data-confirm-title="Delete detections"
              data-confirm-body="Permanently delete {detections} detections of {com}? This cannot be undone. Locked detections are kept."
              data-confirm-confirm-label="Delete detections"
              data-confirm-style="danger">Delete detections</button>
    </form>
    <form hx-post="/admin/species/manage/clips" hx-target="#species-storage" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <button type="submit" class="btn btn-danger del-btn"
              data-confirm-action="hx-post"
              data-confirm-url="/admin/species/manage/clips"
              data-confirm-title="Delete clips"
              data-confirm-body="Delete the {clips} audio clips of {com}? The detections themselves are kept, so the record of what was heard survives. Clips of locked detections are kept."
              data-confirm-confirm-label="Delete clips"
              data-confirm-style="danger">Delete clips</button>
    </form>
  </td>
</tr>"##,
            detections = r.detections,
            clips = r.clips,
        );
    }
    out.push_str("</tbody></table>");
    out
}

#[cfg(test)]
mod tests {
    use super::{human_bytes, outcome_note, remove_clip, safe_clip_path, table};
    use birdnet_db::sqlite::{BulkOutcome, SpeciesUsage};

    fn usage(com: &str, sci: &str, detections: i64, clips: i64, locked: i64) -> SpeciesUsage {
        SpeciesUsage {
            com_name: com.to_owned(),
            sci_name: sci.to_owned(),
            detections,
            clips,
            locked,
            last_seen: "2026-09-07".to_owned(),
        }
    }

    // ── the table ───────────────────────────────────────────────────────

    /// The operator is being asked to delete their own records, so the row has
    /// to carry the numbers behind the decision.
    #[test]
    fn a_row_shows_what_the_species_has_cost() {
        let html = table(
            &[usage("Not A Bird", "Phantomus fictus", 4000, 3900, 0)],
            None,
        );
        assert!(html.contains("Not A Bird"), "{html}");
        assert!(html.contains("Phantomus fictus"), "{html}");
        assert!(html.contains("4000"), "the detection count is missing");
        assert!(html.contains("3900"), "the clip count is missing");
        assert!(
            html.contains("2026-09-07"),
            "the last-heard date is missing"
        );
    }

    /// **The count that must be visible before the button, not after it.** An
    /// operator who deletes 4 000 detections and finds 2 left will think the
    /// button is broken unless the page already told them about the lock.
    ///
    /// Observed failing with the `locked` cell hard-coded to `—`: the pill
    /// was absent and the assertion went red.
    #[test]
    fn a_locked_count_is_shown_before_the_operator_acts() {
        let html = table(&[usage("Rare Bird", "Rara avis", 10, 10, 2)], None);
        assert!(
            html.contains(r#"<span class="bnb-pill">2</span>"#),
            "the locked count must be on the row: {html}"
        );
        // Its counterpart: a species with none shows a dash, not a zero pill,
        // or the pill stops meaning anything.
        let html = table(&[usage("Blackbird", "Turdus merula", 10, 10, 0)], None);
        assert!(
            !html.contains(r#"<span class="bnb-pill">0</span>"#),
            "{html}"
        );
    }

    /// Every destructive action confirms first, and the confirmation says what
    /// it will and will not do — these delete an operator's own records.
    ///
    /// Observed failing with `data-confirm-action` removed from the detections
    /// button: the assertion counting three confirmations went red.
    #[test]
    fn every_destructive_action_confirms_and_says_what_survives() {
        let html = table(
            &[usage("Not A Bird", "Phantomus fictus", 4000, 3900, 0)],
            None,
        );
        assert_eq!(
            html.matches("data-confirm-action").count(),
            3,
            "exclude, delete detections and delete clips each need a confirmation: {html}"
        );
        assert!(
            html.contains("Everything already stored is kept"),
            "exclude must say it changes nothing already recorded: {html}"
        );
        assert!(
            html.contains("cannot be undone"),
            "deleting detections must say so: {html}"
        );
        assert!(
            html.contains("the record of what was heard survives"),
            "deleting clips must say the rows are kept: {html}"
        );
        assert!(html.contains("Locked detections are kept"), "{html}");
    }

    /// Every action posts the scientific name, because that is the key the
    /// exclusion list and the detections table are stored under. Posting the
    /// common name would silently act on nothing.
    #[test]
    fn every_action_carries_the_scientific_name() {
        let html = table(&[usage("Not A Bird", "Phantomus fictus", 1, 1, 0)], None);
        assert_eq!(
            html.matches(r#"name="sci_name" value="Phantomus fictus""#)
                .count(),
            4,
            "measure, exclude, detections and clips all take the scientific name: {html}"
        );
    }

    /// Names reach here from the model's label file and land in a table cell,
    /// an attribute value and the confirmation text.
    #[test]
    fn a_species_name_is_escaped_everywhere_it_lands() {
        let html = table(
            &[usage(
                r"<script>alert(1)</script>",
                r#"Evil" onload="x"#,
                1,
                1,
                0,
            )],
            None,
        );
        assert!(!html.contains("<script>alert(1)"), "{html}");
        assert!(!html.contains(r#"onload="x"#), "{html}");
    }

    /// An empty station says so rather than rendering a bare table header.
    #[test]
    fn a_station_with_no_detections_says_so() {
        let html = table(&[], None);
        assert!(html.contains("No detections yet"), "{html}");
        assert!(
            !html.contains("<tbody>"),
            "an empty table was rendered: {html}"
        );
    }

    // ── what the action reports ─────────────────────────────────────────

    /// The result has to name what was kept, or the operator is left with an
    /// unexplained discrepancy.
    ///
    /// Observed failing with the `locked_skipped > 0` branch removed: the note
    /// said "3998 detections removed" and nothing about the two that were not.
    #[test]
    fn the_result_says_what_was_kept_and_why() {
        let note = outcome_note(
            "Phantomus fictus",
            BulkOutcome {
                affected: 3998,
                locked_skipped: 2,
            },
            "detections removed",
        );
        assert!(note.contains("3998 detections removed"), "{note}");
        assert!(note.contains("2 locked detection(s) were kept"), "{note}");
        assert!(note.contains("unlock them first"), "{note}");

        // With nothing locked it does not mention locks at all.
        let clean = outcome_note(
            "Turdus merula",
            BulkOutcome {
                affected: 5,
                locked_skipped: 0,
            },
            "detections removed",
        );
        assert!(!clean.contains("locked"), "{clean}");
    }

    #[test]
    fn bytes_are_shown_in_units_a_person_reads() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_bytes(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    // ── the path guard ──────────────────────────────────────────────────

    /// **The gate that matters most in this file.** `File_Name` comes from the
    /// database, which on a migrated station holds whatever BirdNET-Pi wrote
    /// there, and this code *unlinks* what it resolves. A `..` that escaped
    /// the recordings directory would delete an operator's own files.
    ///
    /// Observed failing with the `canonical.starts_with(&root)` check removed:
    /// `safe_clip_path` returned the outside path and `remove_clip` deleted
    /// the file the assertion expects to survive.
    #[test]
    fn a_clip_path_that_escapes_the_recordings_directory_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let recordings = root.path().join("recordings");
        std::fs::create_dir_all(&recordings).expect("mkdir");

        // A file an operator would be horrified to lose, outside the tree.
        let precious = root.path().join("precious.db");
        std::fs::write(&precious, b"not a clip").expect("write");

        let escape = "../precious.db";
        assert!(
            safe_clip_path(&recordings, escape).is_none(),
            "a path leaving the recordings directory must not resolve"
        );
        assert!(
            remove_clip(&recordings, escape).is_err(),
            "removing an escaping path must be refused, not attempted"
        );
        assert!(
            precious.exists(),
            "a file outside the recordings directory was deleted"
        );
    }

    /// The counterpart: an ordinary clip inside the tree really is removed,
    /// or the guard above would pass against a function that deletes nothing.
    #[test]
    fn an_ordinary_clip_inside_the_tree_is_removed() {
        let recordings = tempfile::tempdir().expect("tempdir");
        let clip = recordings.path().join("By_Date").join("2026-09-07");
        std::fs::create_dir_all(&clip).expect("mkdir");
        let file = clip.join("wren.wav");
        std::fs::write(&file, b"audio").expect("write");

        let rel = "By_Date/2026-09-07/wren.wav";
        assert!(safe_clip_path(recordings.path(), rel).is_some());
        remove_clip(recordings.path(), rel).expect("removed");
        assert!(!file.exists(), "the clip should be gone");
    }

    /// A clip retention already reclaimed is not an error: the mark is what
    /// matters, and a failure here would make the whole action look broken.
    #[test]
    fn a_clip_that_is_already_gone_is_not_a_failure() {
        let recordings = tempfile::tempdir().expect("tempdir");
        remove_clip(recordings.path(), "By_Date/2026-01-01/gone.wav").expect("already gone is ok");
    }
}
