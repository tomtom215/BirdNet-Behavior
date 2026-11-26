//! The classifier panel: is the station able to identify birds with the models
//! it is set up for, and if not, one button to fix it.
//!
//! Shown in two places that must agree, so they share one renderer: the setup
//! wizard's "Species model" step and the Settings → Capture tab. Both poll
//! `GET /admin/classifier/panel` (see `static/classifier-panel.js`), which
//! returns the panel's own markup, so the words live here once and the script
//! only swaps them in.
//!
//! The panel reports the truth about the *running* station: "Listening" only
//! when the detection engine is up with a classifier. Files on disk that this
//! start did not load are "Installed — restart to start listening", not green.
//! Below the headline it lists every catalogue file the station is set up for
//! — the classifier, a second classifier, the occurrence filter — with what the
//! running station is doing with each, and its button installs or repairs all
//! of those that need it (MDL-15).

use std::fmt::Write as _;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, IntoResponse as _, Response};
use axum::routing::{get, post};

use crate::classifier_setup::{Phase, Snapshot, StartError};
use crate::routes::pages::escape_html;
use crate::state::AppState;

/// The panel and its install action, behind the admin gate.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/classifier/panel", get(panel))
        .route("/admin/classifier/install", post(install))
        .route("/admin/system/boot-id", get(boot))
}

/// `GET /admin/system/boot-id` — which process is answering.
///
/// A restart is "done" when a different process answers, and the old process
/// goes on answering for a moment after the button is pressed, so the page
/// compares this with what it saw before.
async fn boot() -> Response {
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
        ],
        boot_id(),
    )
        .into_response()
}

/// The script that drives the panel and the restart buttons.
pub(crate) static CONTROLS_SCRIPT: crate::seal::Sealed =
    crate::seal::Sealed::new(r#"<script src="/static/setup-controls.js" defer></script>"#);

/// The panel inside the element the script swaps it in and out of, with the
/// script. For the Capture tab; the setup wizard embeds the same two parts.
#[must_use]
///
/// `show_paths` is `false` for a viewer, whose panel names no file path
/// ([`for_reader`]).
pub fn host_html(state: &AppState, show_paths: bool) -> String {
    format!(
        r#"<div class="bnb-card pad" data-cls-host>{panel}</div>{script}"#,
        panel = render_panel(&for_reader(snapshot_of(state), show_paths)),
        script = CONTROLS_SCRIPT.as_str(),
    )
}

/// `GET /admin/classifier/panel` — the current panel, as markup.
async fn panel(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Response {
    no_store(Html(render_panel(&for_reader(
        snapshot_of(&state),
        request_user.is_admin(),
    ))))
}

/// The panel's snapshot for this station as it is now: the detection
/// engine's liveness and the model state its start published.
#[must_use]
pub fn snapshot_of(state: &AppState) -> Snapshot {
    state
        .classifier_setup()
        .snapshot(state.detection_daemon_running(), &state.models_state())
}

/// What a viewer's copy of a snapshot keeps.
///
/// The panel is part of the Capture tab, which a read-only account may open, and
/// some of its fields are filesystem paths or text that carries one: the
/// configured model file that is not there, the reason an install failed (an
/// I/O error names the path it could not write), why the detection engine did
/// not start and what the station says about each file. None is anything a
/// viewer can act on, and each says where the station keeps its files. The
/// administrator's panel is unchanged.
#[must_use]
pub fn for_reader(mut snap: Snapshot, show_paths: bool) -> Snapshot {
    if !show_paths {
        snap.missing_path = None;
        snap.diagnostics_link = false;
        if snap.failure.is_some() {
            snap.failure = Some(
                if snap.phase == Phase::DidNotStart {
                    "Station health says why."
                } else {
                    "The download did not finish."
                }
                .to_owned(),
            );
        }
        for file in &mut snap.files {
            if file.note.is_some() {
                file.note = Some("Station health says why.".to_owned());
            }
        }
        if let birdnet_core::inference::model_state::FilterState::Off { why, .. } =
            &mut snap.published_filter
        {
            "Station health says why.".clone_into(why);
        }
    }
    snap
}

/// `POST /admin/classifier/install` — start installing, and return the panel.
///
/// Answers `200` with the panel in every case, including a refusal, because
/// htmx never swaps the body of a `4xx` and the script that calls this would
/// otherwise show nothing at all when the click was refused.
async fn install(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
) -> Response {
    state
        .run_blocking(move |state| {
            let started = state.classifier_setup().begin_install(
                state.detection_daemon_running(),
                &state.models_state(),
                crate::classifier_setup::install_from_catalogue,
            );
            let note = match started {
                Ok(ids) => {
                    crate::audit::audit(
                        state,
                        Some(&request_user),
                        "classifier.install",
                        Some(&ids.join(",")),
                        None,
                    );
                    None
                }
                Err(StartError::AlreadyRunning) => None,
                Err(e) => Some(refusal(e)),
            };
            let mut html = render_panel(&snapshot_of(state));
            if let Some(note) = note {
                let _ = write!(
                    html,
                    r#"<p class="cls-note" role="alert">{}</p>"#,
                    escape_html(note)
                );
            }
            no_store(Html(html))
        })
        .await
}

/// What a refused install says, in words for the person who clicked.
const fn refusal(e: StartError) -> &'static str {
    match e {
        StartError::WebOnly => {
            "This station was started without its detection engine, so installing a classifier would not change anything."
        }
        StartError::NotNeeded => {
            "Every model file this station is set up for is already in place, or the configuration names its own, so nothing was installed."
        }
        StartError::NoLayout => {
            "This station cannot say where to put a classifier, so nothing was installed."
        }
        StartError::Thread => "The download could not be started. Try again.",
        StartError::AlreadyRunning => "An install is already running.",
    }
}

fn no_store(html: Html<String>) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], html).into_response()
}

/// The panel's markup for one snapshot.
///
/// The root carries `data-cls-state`, so the script knows when to poll.
#[must_use]
pub fn render_panel(snap: &Snapshot) -> String {
    let (tone, pill, headline, detail) = words(snap);
    let progress = snap.progress.as_ref().map_or_else(String::new, |p| {
        let line = if p.file.is_empty() {
            "Starting the download…".to_owned()
        } else {
            format!(
                "{file} — file {step} of {steps}, {done} of {total}",
                file = escape_html(&p.file),
                step = p.step,
                steps = p.steps,
                done = megabytes(p.done),
                total = megabytes(p.total),
            )
        };
        format!(
            r#"<progress class="cls-bar" max="100" value="{pct}" aria-label="Download progress">{pct}%</progress><p class="bnb-meta" role="status">{line}</p>"#,
            pct = p.percent(),
        )
    });
    format!(
        r#"<div class="cls-panel" data-cls-panel data-cls-state="{state}" data-tone="{tone}">
<div class="cls-head"><span class="bnb-pill {pill_class}">{pill}</span><strong class="cls-name">{headline}</strong></div>
<p class="cls-detail">{detail}</p>{progress}{files}
<div class="cls-actions">{actions}</div>
</div>"#,
        state = snap.phase.as_str(),
        pill_class = tone_class(tone),
        files = files_list(snap),
        actions = actions(snap),
    )
}

/// The `bnb-pill` modifier for a tone.
fn tone_class(tone: &str) -> &'static str {
    match tone {
        "ok" => "moss",
        "warn" => "dawn",
        "bad" => "rare",
        "info" => "info",
        _ => "",
    }
}

/// The panel's one button: install or repair every file that needs it, or,
/// with nothing to fetch and something installed since the start, restart.
fn actions(snap: &Snapshot) -> String {
    let fetch = !matches!(
        snap.phase,
        Phase::Installing | Phase::WebOnly | Phase::Unavailable
    ) && snap.to_install().next().is_some();
    if fetch {
        let label = if snap.phase == Phase::Failed {
            "Try again".to_owned()
        } else if snap.repairs() {
            format!("Repair ({})", megabytes(snap.download_bytes))
        } else {
            format!("Install ({})", megabytes(snap.download_bytes))
        };
        return format!(
            r#"<button type="button" class="bnb-btn primary" data-cls-install>{label}</button>"#
        );
    }
    let restart =
        snap.phase == Phase::NeedsRestart || (snap.phase == Phase::Ready && snap.awaits_restart());
    if restart {
        return r#"<button type="button" class="bnb-btn primary" data-cls-restart>Restart the station now</button>"#
            .to_owned();
    }
    String::new()
}

/// One line per catalogue file the station is set up for: what it is for,
/// and what the running station is doing with it. Not drawn for a station
/// whose only file is the classifier the headline is already about, or while
/// the headline is about something else entirely.
fn files_list(snap: &Snapshot) -> String {
    use crate::classifier_setup::{PartState, Role};
    if snap.files.len() < 2
        || matches!(
            snap.phase,
            Phase::Installing | Phase::WebOnly | Phase::Unavailable
        )
    {
        return String::new();
    }
    let mut out = String::from(r#"<ul class="cls-files" aria-label="Model files">"#);
    for f in &snap.files {
        let role = match &f.role {
            Role::Primary => "classifier".to_owned(),
            Role::Second(id) => format!("second classifier \u{201c}{}\u{201d}", escape_html(id)),
            Role::Filter => "species filter".to_owned(),
        };
        let (class, word) = match f.state {
            PartState::InUse => ("moss", "In use"),
            PartState::Installed => ("dawn", "Installed — restart to use it"),
            PartState::Missing => ("dawn", "Not installed"),
            PartState::Damaged => ("rare", "Damaged"),
            PartState::NotUsed => ("dawn", "Not used"),
        };
        let note = f.note.as_deref().map_or_else(String::new, |n| {
            format!(
                r#"<span class="cls-file-note">{}</span>"#,
                escape_html(&crate::model_conditions::capitalize_first(n))
            )
        });
        let _ = write!(
            out,
            r#"<li><span class="cls-file-name">{name}</span> <span class="bnb-meta">{role} · {size}</span> <span class="bnb-pill {class}">{word}</span>{note}</li>"#,
            name = escape_html(f.name),
            size = megabytes(f.bytes),
        );
    }
    out.push_str("</ul>");
    out
}

/// What the Ready headline's detail says about the occurrence filter: what
/// the running station is doing with it, never what the configuration names
/// (MDL-04, MDL-05).
fn filter_sentence(snap: &Snapshot) -> String {
    use crate::classifier_setup::PartState;
    let Some(filter) = snap.filter() else {
        // A filter the operator named, or none: what the station did with it.
        use birdnet_core::inference::model_state::{FilterOff, FilterState};
        return match &snap.published_filter {
            FilterState::Active { .. } => "The local species filter is on: species that do not \
                                           occur near the station's location are held back."
                .to_owned(),
            FilterState::Off {
                reason: FilterOff::NotConfigured,
                ..
            } => "No local species filter is set up, so every species the model knows stays \
                  possible wherever the station is."
                .to_owned(),
            FilterState::Off { why, .. } => format!(
                "The local species filter is off. {}",
                escape_html(&crate::model_conditions::capitalize_first(why))
            ),
        };
    };
    match filter.state {
        PartState::InUse => "The local species filter is on: species that do not occur near the \
                             station's location are held back."
            .to_owned(),
        PartState::Installed => "The local species filter is installed; restart the station once \
                                 to switch it on."
            .to_owned(),
        PartState::Missing => "The local species filter is not installed, so every species the \
                               model knows stays possible wherever the station is. The button \
                               below installs it."
            .to_owned(),
        PartState::Damaged => "The local species filter's file is damaged, so it is off. The \
                               button below repairs it."
            .to_owned(),
        PartState::NotUsed => format!(
            "The local species filter is off. {}",
            escape_html(
                &filter
                    .note
                    .as_deref()
                    .map_or_else(String::new, crate::model_conditions::capitalize_first)
            )
        ),
    }
}

/// `(tone, pill, headline, detail)` for a snapshot. All text here is authored
/// or escaped: nothing a visitor sent reaches it, but a download error can
/// contain a URL.
fn words(snap: &Snapshot) -> (&'static str, &'static str, String, String) {
    let name = escape_html(&snap.name);
    let size = megabytes(snap.download_bytes);
    match snap.phase {
        Phase::Unavailable => (
            "muted",
            "Not available",
            "The model's status cannot be read on this page".to_owned(),
            r#"<a href="/station">Station health</a> lists the models the detector last reported."#
                .to_owned(),
        ),
        Phase::Missing if snap.repairs() => (
            "bad",
            "Damaged",
            "The bird-identifying model is damaged".to_owned(),
            format!(
                "A model file on this station is not the file it should be, so the station \
                 cannot use it. A repair downloads {size} again; every file is checked against \
                 a fingerprint built into the program before it is used."
            ),
        ),
        Phase::Missing => (
            "warn",
            "Not installed",
            "The bird-identifying model is not installed yet".to_owned(),
            format!(
                "Without it the station records nothing. {list} {are} {size}; the station needs \
                 an internet connection this once, and every file is checked against a \
                 fingerprint built into the program before it is used.",
                list = escape_html(&install_list(snap)),
                are = if snap.to_install().count() > 1 {
                    "are"
                } else {
                    "is"
                },
            ),
        ),
        Phase::Installing => (
            "info",
            "Installing…",
            "Downloading the bird-identifying model".to_owned(),
            "You can leave this page open or come back; the download carries on in the station."
                .to_owned(),
        ),
        Phase::Failed => (
            "bad",
            "Install failed",
            "The model could not be installed".to_owned(),
            format!(
                "{} Nothing half-downloaded is used. Check the station's internet connection and \
                 free disk space, then try again.",
                escape_html(snap.failure.as_deref().unwrap_or("The download did not finish."))
            ),
        ),
        Phase::NeedsRestart => (
            "warn",
            "Installed — restart to start listening",
            format!("{name} is installed"),
            "The station loads its model when it starts, so it is not listening yet. Restart it \
             once and it will begin."
                .to_owned(),
        ),
        Phase::Ready => (
            "ok",
            "Listening",
            format!(
                "Identifying birds with {}",
                escape_html(snap.in_use.as_deref().unwrap_or("the installed model"))
            ),
            filter_sentence(snap),
        ),
        Phase::DidNotStart => (
            "bad",
            "Not started",
            "The detection engine did not start".to_owned(),
            format!(
                "Nothing is being identified. {}",
                escape_html(&crate::model_conditions::capitalize_first(
                    snap.failure.as_deref().unwrap_or("Station health says why.")
                ))
            ),
        ),
        Phase::Stopped => (
            "bad",
            "Stopped",
            "The detection engine started and has stopped".to_owned(),
            r#"Nothing is being identified. <a href="/station">Station health</a> says why."#
                .to_owned(),
        ),
        Phase::WebOnly => (
            "muted",
            "Pages only",
            "This station is running without its detection engine".to_owned(),
            "It was started with <code>--web-only</code>, so it shows pages and history but does \
             not listen. Start it without that option to identify birds."
                .to_owned(),
        ),
        Phase::Configured => (
            "warn",
            "Set in the configuration file",
            "This station is set to use its own model file".to_owned(),
            snap.missing_path.as_deref().map_or_else(
                || {
                    if snap.diagnostics_link {
                        r#"The detection engine did not start with it. <a href="/admin/doctor">Diagnostics</a> says why."#
                            .to_owned()
                    } else {
                        "The detection engine did not start with it. An administrator can see why under Diagnostics."
                            .to_owned()
                    }
                },
                |p| {
                    format!(
                        "The file <code>{}</code> named in the configuration is not there, so \
                         nothing is being identified. Put it there, or remove the setting.",
                        escape_html(p)
                    )
                },
            ),
        ),
    }
}

/// The files an install would fetch, by name, as a sentence's subject.
fn install_list(snap: &Snapshot) -> String {
    let names: Vec<&str> = snap.to_install().map(|f| f.name).collect();
    match names.as_slice() {
        [] => snap.name.clone(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `"84 MB"`, rounded up so a figure never understates a download.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

/// Which process rendered a panel: constant for the life of the process and
/// different after a restart.
fn boot_id() -> &'static str {
    use std::sync::OnceLock;
    static BOOT: OnceLock<String> = OnceLock::new();
    BOOT.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("{nanos:x}-{}", std::process::id())
    })
}
