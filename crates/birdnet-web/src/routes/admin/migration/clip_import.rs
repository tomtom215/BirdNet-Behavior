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
    /// BirdNET-Pi's folder, or its `disk_check_exclude.txt`, when the list of
    /// clips its owner protected is not where the standard install keeps it.
    /// Blank: look for it beside the recordings.
    #[serde(default)]
    pi_folder: String,
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
    /// File names on BirdNET-Pi's list of protected clips; empty when there is
    /// none, or when nothing is being brought across.
    protected: Vec<String>,
    /// Whether a list was found at all, for the sentence the page prints.
    protected_found: bool,
}

/// BirdNET-Pi's protected-clips list: from the folder the person named, else
/// from beside the recordings.
fn protected_list(
    source: &std::path::Path,
    pi_folder: &str,
) -> Result<Option<Vec<String>>, String> {
    let pi_folder = pi_folder.trim();
    if !pi_folder.is_empty() {
        let path = PathBuf::from(pi_folder);
        if !path.is_absolute() {
            return Err(format!(
                "{} is not a full path. Start it with a slash.",
                path.display()
            ));
        }
        return clips::read_protected_list(&path).map(Some);
    }
    clips::find_protected_list(source)
        .map(|found| clips::read_protected_list(&found))
        .transpose()
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
    let list = if mode == Mode::Leave {
        None
    } else {
        protected_list(&source, &form.pi_folder)?
    };
    Ok(Setup {
        protected_found: list.is_some(),
        protected: list.unwrap_or_default(),
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
      <label for="clips-pi" class="mt-sm">Where BirdNET-Pi is installed <span class="muted">(optional)</span></label>
      <input type="text" id="clips-pi" name="pi_folder" value=""
             placeholder="/home/pi/BirdNET-Pi" autocomplete="off" spellcheck="false">
      <p class="hint">Only to find the clips you protected in BirdNET-Pi
        (<code>scripts/disk_check_exclude.txt</code>); they are locked here so this station's cleanup keeps them.
        Left blank, it is looked for beside the recordings.</p>
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

/// The sentence about BirdNET-Pi's protected clips, and (when `apply`) the locking.
///
/// BirdNET-Pi keeps a list of clips its owner protected from its purge. Here
/// the same thing is a locked detection, which the disk manager and the bulk
/// actions leave alone. Without this the imported protected clips arrive as
/// ordinary ones and the first cleanup may delete them.
fn protection_line(state: &AppState, setup: &Setup, apply: bool) -> Result<String, String> {
    if setup.options.mode == Mode::Leave {
        return Ok(String::new());
    }
    if !setup.protected_found {
        return Ok(r#"<p class="hint">No list of protected clips was found beside the recordings, so none will be locked. If you protected clips in BirdNET-Pi, put its folder in <em>Where BirdNET-Pi is installed</em> above.</p>"#.to_owned());
    }
    let names = &setup.protected;
    // A preview only reads the detections: its temp table lives in the
    // connection's own temporary database, which a read-only connection may
    // write. Only applying sets `is_locked`, and that takes the writer.
    let p = if apply {
        state.with_db(|conn| clips::protect_imported(conn, names, true))
    } else {
        state.with_read_db(|conn| clips::protect_imported(conn, names, false))
    }
    .map_err(|e| format!("The protected clips could not be checked: {e}"))?;
    let unmatched = p
        .listed
        .saturating_sub(usize::try_from(p.matched).unwrap_or(usize::MAX));
    let tail = if unmatched > 0 {
        format!(
            " {unmatched} name{} on BirdNET-Pi's list match no imported detection (import the history first, then run this again).",
            if unmatched == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    Ok(format!(
        r#"<p class="hint"><strong>{} protected clip{} will be locked</strong> so this station's cleanup never deletes {}; {} on BirdNET-Pi's list.{tail}</p>"#,
        p.newly_locked,
        if p.newly_locked == 1 { "" } else { "s" },
        if p.newly_locked == 1 { "it" } else { "them" },
        p.listed,
    ))
}

/// The `hx-vals` attribute value for the Bring-them-across button: a JSON
/// object, built by a serialiser and then escaped for a single-quoted HTML
/// attribute. (It was a format string, so a folder name holding `"` or `\`
/// made the JSON invalid and the button posted nothing.)
fn hx_vals(source: &str, mode: &str, pi_folder: &str) -> String {
    let json = serde_json::json!({
        "clips_source": source,
        "clips_mode": mode,
        "pi_folder": pi_folder,
    })
    .to_string();
    escape_html(&json).replace('\'', "&#39;")
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
            let report =
                clips::run(&setup.options, &wanted, None).map_err(|e: ClipError| e.to_string())?;
            let protection = protection_line(state, &setup, false)?;
            Ok::<_, String>((
                report,
                mode,
                form.clips_source.clone(),
                form.clips_mode.clone(),
                form.pi_folder,
                protection,
            ))
        })
        .await;
    match result {
        Err(message) => err(&message),
        Ok((report, mode, source, mode_word, pi_folder, protection)) => {
            use std::fmt::Write as _;
            let mut html = String::from(r#"<div class="card result-card ok">"#);
            html.push_str(
                r#"<div class="result-title ok">What bringing the recordings across would do</div><p class="hint"><strong>Nothing has been changed yet.</strong></p>"#,
            );
            html.push_str(&report_html(&report, mode, true));
            html.push_str(&protection);
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
                      hx-vals='{vals}'
                      hx-target="#clips-result" hx-swap="innerHTML"{confirm}>Bring them across</button></div></div>"##,
                    vals = hx_vals(&source, &mode_word, &pi_folder),
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
    let Some(progress) = super::claim_slot(&migration_state) else {
        return err(super::ALREADY_RUNNING);
    };
    let setup_state = state.clone();
    let setup_form = ClipForm {
        clips_source: form.clips_source.clone(),
        clips_mode: form.clips_mode.clone(),
        pi_folder: form.pi_folder.clone(),
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
        let worker_progress = progress.clone();
        super::run_guarded(&worker_progress, move || {
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
                    // The recordings are in place; now the locks. A failure
                    // here does not undo the copy, and says so.
                    let locked = if mode == Mode::Leave || !setup.protected_found {
                        String::new()
                    } else {
                        match setup_state
                            .with_db(|conn| clips::protect_imported(conn, &setup.protected, true))
                        {
                            Ok(p) => format!(
                                " {} protected clip{} locked.",
                                p.newly_locked,
                                if p.newly_locked == 1 { "" } else { "s" }
                            ),
                            Err(e) => {
                                tracing::warn!(error = %e, "could not lock the protected clips");
                                " The clips you protected in BirdNET-Pi could not be locked; run this again."
                                    .to_owned()
                            }
                        }
                    };
                    let message = format!(
                        "{} recording{} {verb} ({}); {} were already here, {} not found, {} failed.{locked}",
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
    });
    toast::with(
        Html(super::render::clips_started()),
        Toast::warn("Bringing the recordings across — see progress below.").sticky(),
    )
}

#[cfg(test)]
mod tests {
    use super::hx_vals;

    /// What the browser does with the attribute: decode the entities, parse JSON.
    fn as_the_browser_reads_it(attr: &str) -> serde_json::Value {
        let decoded = attr
            .replace("&#x27;", "'")
            .replace("&#39;", "'")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        serde_json::from_str(&decoded).expect("valid JSON")
    }

    #[test]
    fn a_folder_with_quotes_and_backslashes_still_posts_what_was_checked() {
        for source in [
            "/home/pi/BirdSongs",
            "/srv/it's \"quoted\" \\ folder <b>&",
            "/tmp/trailing\\",
        ] {
            let attr = hx_vals(source, "copy", "/home/pi/BirdNET-Pi");
            assert!(!attr.contains('\''), "would end the attribute: {attr}");
            let v = as_the_browser_reads_it(&attr);
            assert_eq!(v["clips_source"], source);
            assert_eq!(v["clips_mode"], "copy");
            assert_eq!(v["pi_folder"], "/home/pi/BirdNET-Pi");
        }
    }
}
