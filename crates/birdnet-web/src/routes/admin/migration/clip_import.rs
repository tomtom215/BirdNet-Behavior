//! Bringing BirdNET-Pi's recordings across, from the browser.
//!
//! The database import copies rows; the recording each row names stayed in
//! BirdNET-Pi's clip tree. This is the step that brings them, so an imported
//! detection plays and has a spectrogram. The work is
//! [`birdnet_migrate::clips`]; this module asks the questions of the database
//! and the disk that the library cannot, shows the dry run, and runs it with
//! progress.
//!
//! # Why it checks the disk against the purge threshold
//!
//! Copying a few gigabytes onto a station whose disk is at 80 % can take it
//! past the percentage at which the disk manager starts deleting clips. The
//! manager would then remove the oldest clips of the busiest species, which is
//! to say the history just brought across, and no setting the owner chose would
//! have said so. The dry run compares the usage after the copy with the
//! threshold and refuses when it would cross it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Form;
use axum::extract::State;
use axum::response::Html;
use birdnet_migrate::clips::{self, ClipError, DiskRoom, Mode, Options, Report, Wanted};
use birdnet_migrate::progress::{MigrationProgress, MigrationStage, ProgressHandle};
use serde::Deserialize;

use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

/// Rows read from the database at a time.
const PAGE: i64 = 5_000;

/// What the form posts.
#[derive(Debug, Deserialize)]
pub(super) struct ClipForm {
    /// BirdNET-Pi's clip tree, or a folder that holds it.
    clips_source: String,
    /// `copy`, `hardlink`, `move` or `leave`.
    clips_mode: String,
}

/// The imported detections that name a clip, read a page at a time so a
/// station with years of history does not hold them all in memory.
struct Pager {
    state: AppState,
    after: i64,
    page: VecDeque<Wanted>,
    done: bool,
}

impl Pager {
    const fn new(state: AppState) -> Self {
        Self {
            state,
            after: 0,
            page: VecDeque::new(),
            done: false,
        }
    }
}

impl Iterator for Pager {
    type Item = Wanted;

    fn next(&mut self) -> Option<Wanted> {
        if self.page.is_empty() && !self.done {
            let after = self.after;
            let read = self
                .state
                .with_read_db(|conn| clips::wanted_page(conn, after, PAGE));
            match read {
                Ok(rows) if !rows.is_empty() => {
                    self.after = rows.last().map_or(after, |(id, _)| *id);
                    self.page = rows.into_iter().map(|(_, w)| w).collect();
                }
                Ok(_) => self.done = true,
                Err(e) => {
                    // Stop rather than skip: a half-read list would report the
                    // rest as "nothing to bring".
                    tracing::warn!(error = %e, "could not read the imported detections");
                    self.done = true;
                }
            }
        }
        self.page.pop_front()
    }
}

/// The question the dry run and the run both ask, answered once.
struct Setup {
    options: Options,
}

fn setup(state: &AppState, form: &ClipForm, dry_run: bool) -> Result<Setup, String> {
    let mode = Mode::parse(&form.clips_mode)
        .ok_or_else(|| "Choose what to do with the recordings.".to_owned())?;
    let source = form.clips_source.trim();
    if source.is_empty() {
        return Err(
            "Enter the folder BirdNET-Pi keeps its recordings in, for example \
                    /home/pi/BirdSongs."
                .to_owned(),
        );
    }
    let source = PathBuf::from(source);
    if !source.is_absolute() {
        return Err(format!(
            "{} is not a full path. Start it with a slash.",
            source.display()
        ));
    }
    let dest = state.recording_dir();
    let (threshold, retention_days) = state.with_read_db(|conn| {
        let get = |key: &str| {
            birdnet_db::settings::get_or(conn, key, "")
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
        };
        (
            get("purge_threshold").unwrap_or(95.0),
            get("clip_retention_days").unwrap_or(0.0),
        )
    });
    let room = std::fs::create_dir_all(&dest)
        .ok()
        .and_then(|()| birdnet_core::audio::capture::disk_usage(&dest).ok())
        .map(|u| DiskRoom {
            used_bytes: u.used_bytes,
            available_bytes: u.available_bytes,
            purge_threshold_percent: threshold,
        });
    #[allow(clippy::cast_possible_truncation)]
    let retention_cutoff = (retention_days >= 1.0)
        .then(|| {
            birdnet_core::civil::shift_datetime(
                &crate::routes::pages::today_date_string(),
                "00:00:00",
                -retention_days.floor() * 86_400.0,
            )
        })
        .flatten()
        .map(|(date, _)| date);
    Ok(Setup {
        options: Options {
            source,
            dest,
            mode,
            dry_run,
            room,
            retention_cutoff,
        },
    })
}

const fn describe(mode: Mode) -> &'static str {
    match mode {
        Mode::Copy => "copied",
        Mode::Hardlink => "linked",
        Mode::Move => "moved",
        Mode::Leave => "found",
    }
}

fn report_html(report: &Report, mode: Mode, dry_run: bool) -> String {
    use clips::human_bytes;
    use std::fmt::Write as _;
    let verb = describe(mode);
    let mut s = String::new();
    let _ = write!(
        s,
        "<p><strong>{}</strong> recording{} would be {verb} ({}); {} are already here and {} \
         could not be found.</p>",
        report.done,
        if report.done == 1 { "" } else { "s" },
        human_bytes(report.bytes),
        report.already_there,
        report.missing,
    );
    if !dry_run {
        s = s.replace("would be", "were");
    }
    if report.conflicts > 0 {
        let _ = write!(
            s,
            r#"<p class="hint">{} already have a file of that name with a different size; they are left alone.</p>"#,
            report.conflicts
        );
    }
    if report.unsafe_names > 0 {
        let _ = write!(
            s,
            r#"<p class="hint">{} detections name a file that is not a plain file name; they are skipped.</p>"#,
            report.unsafe_names
        );
    }
    if report.older_than_retention > 0 {
        let _ = write!(
            s,
            r#"<p class="hint"><strong>{} of these are older than the number of days this station keeps clips for.</strong> The next cleanup would delete them again; raise <em>Keep clip audio</em> in the Settings first if you want them.</p>"#,
            report.older_than_retention
        );
    }
    if !report.missing_sample.is_empty() {
        let mut list = String::new();
        for n in &report.missing_sample {
            let _ = write!(list, "<li><code>{}</code></li>", escape_html(n));
        }
        let _ = write!(
            s,
            r#"<details class="preview-details"><summary class="preview-summary">Some of the recordings that were not found</summary><ul class="steps">{list}</ul><p class="hint">Usually a clip BirdNET-Pi's own cleanup already deleted, or a folder that is somewhere else.</p></details>"#
        );
    }
    if !report.failed_sample.is_empty() {
        let mut list = String::new();
        for n in &report.failed_sample {
            let _ = write!(list, "<li>{}</li>", escape_html(n));
        }
        let _ = write!(
            s,
            r#"<p class="hint"><strong>{} could not be {verb}:</strong></p><ul class="steps">{list}</ul>"#,
            report.failed
        );
    }
    s
}

/// The form card.
#[must_use]
pub(super) fn form(default_source: &str) -> String {
    format!(
        r##"<div class="card" id="clip-import">
    <div class="card-head">Your recordings</div>
    <p class="hint mb-sm">
      The history import brings the detections but not the sound. This brings the recordings, so
      imported detections play and show a spectrogram. It only takes the recordings that imported
      detections name, never overwrites a file, and can be run again to carry on where it stopped.
    </p>
    <form id="clips-form">
      <label for="clips-source">Where BirdNET-Pi keeps its recordings</label>
      <input type="text" id="clips-source" name="clips_source" value="{src}"
             placeholder="/home/pi/BirdSongs" autocomplete="off" spellcheck="false">
      <p class="hint">A folder on this device: <code>~/BirdSongs</code>, or the <code>Extracted</code> or
        <code>By_Date</code> folder inside it. If BirdNET-Pi runs on another machine, copy the folder here first.</p>
      <label for="clips-mode" class="mt-sm">What to do with them</label>
      <select id="clips-mode" name="clips_mode">
        <option value="copy" selected>Copy them (BirdNET-Pi keeps its own)</option>
        <option value="hardlink">Link them (no extra space; same disk only)</option>
        <option value="move">Move them (BirdNET-Pi loses them)</option>
        <option value="leave">Only count them; change nothing</option>
      </select>
      <div class="actions center wrap">
        <button type="button" class="btn btn-secondary"
                hx-post="/admin/migrate/clips/preview" hx-include="#clips-form"
                hx-target="#clips-result" hx-swap="innerHTML">Check first</button>
        <span class="hint">Looks at the disk and the folder; changes nothing.</span>
      </div>
    </form>
    <div id="clips-result" class="mt" aria-live="polite"></div>
  </div>"##,
        src = escape_html(default_source)
    )
}

fn err(message: &str) -> Html<String> {
    Html(format!(
        r#"<div class="card result-card err" role="alert"><div class="result-title err">{}</div></div>"#,
        escape_html(message)
    ))
}

/// `POST /admin/migrate/clips/preview`: the dry run.
pub(super) async fn preview_handler(
    State(state): State<AppState>,
    Form(form): Form<ClipForm>,
) -> Html<String> {
    let result = state
        .run_blocking(move |state| {
            let setup = setup(state, &form, true)?;
            let mode = setup.options.mode;
            let factory_state = state.clone();
            let wanted = move || -> Box<dyn Iterator<Item = Wanted>> {
                Box::new(Pager::new(factory_state.clone()))
            };
            clips::run(&setup.options, &wanted, None)
                .map(|report| {
                    (
                        report,
                        mode,
                        form.clips_source.clone(),
                        form.clips_mode.clone(),
                    )
                })
                .map_err(|e: ClipError| e.to_string())
        })
        .await;
    match result {
        Err(message) => err(&message),
        Ok((report, mode, source, mode_word)) => {
            use std::fmt::Write as _;
            let mut html = String::from(r#"<div class="card result-card ok">"#);
            html.push_str(
                r#"<div class="result-title ok">What bringing the recordings across would do</div><p class="hint"><strong>Nothing has been changed yet.</strong></p>"#,
            );
            html.push_str(&report_html(&report, mode, true));
            if mode == Mode::Leave || report.done == 0 {
                html.push_str(r#"<p class="hint">There is nothing to bring across.</p></div>"#);
            } else {
                let confirm = if mode == Mode::Move {
                    r#" data-confirm-body="Move these recordings out of BirdNET-Pi's folder? BirdNET-Pi will no longer have them. There is no undo." data-confirm-action="hx-post""#
                } else {
                    ""
                };
                let _ = write!(
                    html,
                    r##"<div class="actions center"><button class="btn btn-primary"
                      hx-post="/admin/migrate/clips/run"
                      hx-vals='{{"clips_source": "{src}", "clips_mode": "{mode}"}}'
                      hx-target="#clips-result" hx-swap="innerHTML"{confirm}>Bring them across</button></div></div>"##,
                    src = escape_html(&source).replace('\'', "&#39;"),
                    mode = escape_html(&mode_word),
                );
            }
            Html(html)
        }
    }
}

/// `POST /admin/migrate/clips/run`: do it, in the background, with progress.
pub(super) async fn run_handler(
    State(state): State<AppState>,
    migration_state: Arc<Mutex<Option<ProgressHandle>>>,
    Form(form): Form<ClipForm>,
) -> Html<String> {
    let progress = ProgressHandle::new();
    {
        let mut guard = migration_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.as_ref().is_some_and(|p| {
            !p.snapshot().is_terminal() && p.snapshot().stage != MigrationStage::Pending
        }) {
            return err("Another import is still running. Wait for it to finish.");
        }
        *guard = Some(progress.clone());
    }
    let setup_state = state.clone();
    let setup_form = ClipForm {
        clips_source: form.clips_source.clone(),
        clips_mode: form.clips_mode.clone(),
    };
    let prepared = state
        .run_blocking(move |state| setup(state, &setup_form, false))
        .await;
    let setup = match prepared {
        Ok(s) => s,
        Err(message) => {
            progress.fail(message.clone());
            return err(&message);
        }
    };
    let mode = setup.options.mode;
    tokio::task::spawn_blocking(move || {
        progress.update(MigrationProgress {
            stage: MigrationStage::Importing,
            rows_imported: 0,
            rows_total: 0,
            message: "Looking for the recordings…".to_owned(),
            error: None,
        });
        let factory_state = setup_state.clone();
        let wanted = move || -> Box<dyn Iterator<Item = Wanted>> {
            Box::new(Pager::new(factory_state.clone()))
        };
        match clips::run(&setup.options, &wanted, Some(&progress)) {
            Ok(report) => {
                tracing::info!(
                    done = report.done,
                    already_there = report.already_there,
                    missing = report.missing,
                    failed = report.failed,
                    "BirdNET-Pi recordings brought across"
                );
                let verb = describe(mode);
                let message = format!(
                    "{} recording{} {verb} ({}); {} were already here, {} not found, {} failed.",
                    report.done,
                    if report.done == 1 { "" } else { "s" },
                    clips::human_bytes(report.bytes),
                    report.already_there,
                    report.missing,
                    report.failed,
                );
                progress.update(MigrationProgress {
                    stage: MigrationStage::Complete,
                    rows_imported: report.done,
                    rows_total: report.done,
                    message,
                    error: None,
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "bringing the recordings across failed");
                progress.fail(e.to_string());
            }
        }
    });
    toast::with(
        Html(super::render::clips_started()),
        Toast::warn("Bringing the recordings across — see progress below.").sticky(),
    )
}
