//! Audio sources management — first-class CRUD page (O-13).
//!
//! Replaces the prior hard-coded two-row stub. The page body is rendered
//! from `templates/admin_audio_sources.html` + per-row partials in
//! `templates/_partial_audio_source_row.html`; live status comes from
//! `probe(id)` which reads the supervisor-published
//! `birdnet_audio_source_up{source=<row.id>}` gauge.
//!
//! ## O-13 wiring status
//!
//! Both the initial page render and the per-row `/probe` poll resolve
//! status through the metrics gauge `birdnet_audio_source_up{source=<row.id>}`
//! the capture supervisor publishes each reconcile tick — honest
//! `Capturing` / `Down`, never a synthetic "first row up" stub (#102, #107).
//!
//! The `audio_sources` table is now the sole source of truth: the legacy
//! single-string `state.audio_source()` fallback (and the `with_audio_source`
//! builder that fed it from CLI/env) was retired in O-13. Migration 15 still
//! seeds the table from a pre-existing `settings.audio_source` on upgrade.

use std::fmt::Write as _;

use axum::Form;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use birdnet_db::audio_sources::{
    AudioSource, AudioSourceError, AudioSourcePatch, AudioSourceStore, NewAudioSource,
    PipelineFlags, RtspTransport, SourceKind,
};

use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

// Embedded templates.
const PAGE_TPL: &str = include_str!("../../../templates/admin_audio_sources.html");
const ROW_TPL: &str = include_str!("../../../templates/_partial_audio_source_row.html");

/// Mount the audio sources CRUD admin routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/audio", get(page))
        .route("/admin/audio/sources", post(create))
        .route(
            "/admin/audio/sources/{id}",
            get(row).delete(remove).patch(update),
        )
        .route("/admin/audio/sources/{id}/edit", get(edit_form))
        .route("/admin/audio/sources/{id}/probe", get(probe))
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
// `Starting` and `Paused` are part of the documented status palette
// (see _partial_audio_source_row.html). The metrics-driven path
// currently only emits `Capturing` / `Down`; the supervisor would
// need a separate "starting" / "scheduled-pause" state to drive the
// other two pills. They stay in the enum so the partial's status
// keys are exhaustively typed.
#[allow(dead_code)]
enum Status {
    Capturing,
    Starting,
    Paused,
    Down,
}

impl Status {
    const fn css(self) -> &'static str {
        match self {
            Self::Capturing => "live",
            Self::Starting => "starting",
            Self::Paused => "paused",
            Self::Down => "down",
        }
    }
    const fn label(self) -> &'static str {
        match self {
            Self::Capturing => "Capturing",
            Self::Starting => "Starting…",
            Self::Paused => "Paused (schedule)",
            Self::Down => "Down",
        }
    }
}

const fn kind_css(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::UsbAlsa => "usb",
        SourceKind::PipeWire => "pipe",
        SourceKind::Rtsp => "rtsp",
    }
}

/// Human-readable badge text for a source kind.
///
/// `pub(crate)` so the onboarding wizard's microphone step names the operator's
/// real hardware with the same words the Capture tab uses, instead of carrying
/// its own copy that could drift.
pub(crate) const fn kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::UsbAlsa => "USB · ALSA",
        SourceKind::PipeWire => "PipeWire",
        SourceKind::Rtsp => "RTSP",
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The standalone `/admin/audio` page GET folded into the Station **Capture**
/// tab; its old URL permanently redirects there. The POST/probe/partial
/// endpoints below keep their `/admin/audio/...` paths.
async fn page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/capture#audio")
}

/// Render the audio-sources management body (no document shell).
///
/// Shared between the standalone `/admin/audio` page and the Station **Capture**
/// tab (`crate::routes::pages::homes::station_tabs`). First paint resolves each
/// pill through the same metrics-driven path the per-row `/probe` poll uses, so
/// the initial status matches the 8 s self-poll.
pub(crate) fn sources_body(state: &AppState) -> String {
    let sources = state.with_db(AudioSourceStore::list).unwrap_or_else(|err| {
        tracing::error!(error = %err, "audio_sources list failed");
        Vec::new()
    });
    render_body(&sources, |row| daemon_status(row, state))
}

#[derive(Deserialize)]
struct CreateForm {
    #[serde(default)]
    scope: String,
    kind: String,
    device_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default)]
    rtsp_transport: Option<String>,
    /// Start of the per-source quiet window, `HH:MM` in the station's **local**
    /// time. Empty (with `quiet_end` also empty) clears the window.
    #[serde(default)]
    quiet_start: Option<String>,
    /// End of the per-source quiet window, `HH:MM` local. Half-open: the source
    /// is paused from `quiet_start` up to but not including `quiet_end`.
    #[serde(default)]
    quiet_end: Option<String>,
    /// Per-source conditioning toggles. `Some("1")` when ticked, `None` when
    /// the operator unticked it *or* when the form does not carry the control
    /// at all — see [`ToggleSet::from_form`] for how those are told apart.
    #[serde(default)]
    high_pass: Option<String>,
    #[serde(default)]
    dc_removal: Option<String>,
    #[serde(default)]
    agc: Option<String>,
    #[serde(default)]
    rtsp_keepalive: Option<String>,
    /// Hidden companion field, always submitted by the edit form. Its presence
    /// is what says "this submission came from a form that carries the four
    /// checkboxes", so an unticked box means *off* rather than *absent*.
    /// Without it, a PATCH from any other form would silently clear all four.
    #[serde(default)]
    pipeline_present: Option<String>,
}

/// The four conditioning toggles as submitted, or `None` when the form did not
/// carry them.
///
/// An unchecked HTML checkbox submits nothing at all, so "off" and "not on this
/// form" look identical in the payload. The hidden `pipeline_present` marker
/// disambiguates them, which matters because the create form and the edit form
/// are different shapes and a PATCH that guessed wrong would turn a source's
/// conditioning off without the operator touching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// The four booleans mirror `PipelineFlags` one-for-one. Collapsing them into an
// enum or a bitflag here would only add a translation layer between the form
// and the storage type, and the checkbox names are the wire format.
#[allow(clippy::struct_excessive_bools)]
struct ToggleSet {
    high_pass: bool,
    dc_removal: bool,
    agc: bool,
    rtsp_keepalive: bool,
}

impl ToggleSet {
    fn from_form(
        marker: Option<&str>,
        high_pass: Option<&str>,
        dc_removal: Option<&str>,
        agc: Option<&str>,
        rtsp_keepalive: Option<&str>,
    ) -> Option<Self> {
        marker?;
        Some(Self {
            high_pass: high_pass.is_some(),
            dc_removal: dc_removal.is_some(),
            agc: agc.is_some(),
            rtsp_keepalive: rtsp_keepalive.is_some(),
        })
    }

    const fn into_flags(self) -> PipelineFlags {
        PipelineFlags {
            high_pass: self.high_pass,
            dc_removal: self.dc_removal,
            agc: self.agc,
            rtsp_keepalive: self.rtsp_keepalive,
        }
    }
}

/// What the form's two quiet-window fields mean together.
#[derive(Debug, Clone, PartialEq, Eq)]
enum QuietChoice {
    /// Neither field was submitted at all — the form does not carry the
    /// control, so leave whatever is stored alone. Distinct from `Clear`.
    Absent,
    /// Both submitted and blank: remove the window.
    Clear,
    /// Both submitted and valid.
    Set(String, String),
}

/// Interpret the form's `quiet_start` / `quiet_end` pair.
///
/// Blank means "no quiet window", and both fields have to agree about that: a
/// window with only one end is not a window, and silently dropping the half the
/// operator did fill in would look like the setting had been saved. `HH:MM` is
/// validated here rather than left to the SQL `CHECK`, so a typo comes back as a
/// toast naming the field instead of a 500.
fn parse_quiet_choice(start: Option<&str>, end: Option<&str>) -> Result<QuietChoice, &'static str> {
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(QuietChoice::Absent);
    };
    let (start, end) = (start.trim(), end.trim());
    match (start.is_empty(), end.is_empty()) {
        (true, true) => Ok(QuietChoice::Clear),
        (false, false) => {
            if !is_hhmm(start) || !is_hhmm(end) {
                return Err("Quiet window times must be HH:MM (24-hour), e.g. 22:00.");
            }
            if start == end {
                return Err(
                    "A quiet window that starts and ends at the same time is empty —                      clear both fields to remove it.",
                );
            }
            Ok(QuietChoice::Set(start.to_owned(), end.to_owned()))
        }
        _ => Err("Set both ends of the quiet window, or clear both to remove it."),
    }
}

/// Whether `s` is a 24-hour `HH:MM`.
fn is_hhmm(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else {
        return false;
    };
    if h.len() != 2 || m.len() != 2 {
        return false;
    }
    matches!((h.parse::<u32>(), m.parse::<u32>()), (Ok(h), Ok(m)) if h < 24 && m < 60)
}

async fn create(State(state): State<AppState>, Form(form): Form<CreateForm>) -> Response {
    let _ = form.scope;
    let device_id = form.device_id.trim().to_string();
    if device_id.is_empty() {
        return validation_response("Device id is required.");
    }
    let kind: SourceKind = match form.kind.parse() {
        Ok(k) => k,
        Err(_) => return validation_response("Unknown source kind."),
    };

    // Fool-proofing: refuse to add the same physical source twice. The DB only
    // enforces uniqueness on the synthetic `id` (freshly generated here), so
    // without this an operator who can't tell the save registered just clicks
    // "Save" again and silently ends up with the same mic/stream listed twice.
    let already_configured = state
        .with_db(AudioSourceStore::list)
        .unwrap_or_default()
        .iter()
        .any(|s| s.kind.as_str() == kind.as_str() && s.device_id == device_id);
    if already_configured {
        return validation_response(&format!(
            "“{device_id}” is already configured — see the list below."
        ));
    }

    let mut new = NewAudioSource::defaults(synth_id(kind), kind, device_id);
    new.label = form.label.and_then(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    if let Some(rate) = form.sample_rate {
        // Constrained by the SQL CHECK; the form select limits it to safe values.
        new.sample_rate = rate;
    }
    if let Some(transport) = form.rtsp_transport.as_deref() {
        match transport.parse::<RtspTransport>() {
            Ok(t) => new.rtsp_transport = t,
            Err(_) => return validation_response("Unknown RTSP transport."),
        }
    }
    if let Some(toggles) = ToggleSet::from_form(
        form.pipeline_present.as_deref(),
        form.high_pass.as_deref(),
        form.dc_removal.as_deref(),
        form.agc.as_deref(),
        form.rtsp_keepalive.as_deref(),
    ) {
        new.pipeline = toggles.into_flags();
    }
    match parse_quiet_choice(form.quiet_start.as_deref(), form.quiet_end.as_deref()) {
        Ok(QuietChoice::Set(a, b)) => new.schedule_quiet = Some((a, b)),
        Ok(QuietChoice::Absent | QuietChoice::Clear) => {}
        Err(msg) => return validation_response(msg),
    }

    let result = state.with_db(|conn| conn.insert(&new));
    match result {
        Ok(row) => {
            let mut body = render_row(&row, daemon_status(&row, &state));
            // Refresh the section totals so adding the first (or Nth) source
            // visibly updates the "N mics / N streams" header, not just the list.
            body.push_str(&count_oobs(&state));
            body.push_str(
                &Toast::success(format!(
                    "Added {}.",
                    row.label.as_deref().unwrap_or(&row.device_id)
                ))
                .with_action("/admin/system/restart", "Restart to apply")
                .render_oob(),
            );
            Html(body).into_response()
        }
        Err(AudioSourceError::Conflict(_)) => validation_response(
            "A source with that id already exists. Retry — a new id will be generated.",
        ),
        Err(AudioSourceError::Invalid(msg)) => validation_response(&msg),
        Err(e) => {
            tracing::error!(error = %e, "audio source insert failed");
            internal_response("Could not add the source.")
        }
    }
}

async fn remove(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let result = state.with_db(|conn| conn.soft_delete(&id));
    match result {
        Ok(()) => {
            // The row's hx-swap removes it from the list; refresh both count
            // chips via OOB so the header totals drop in step.
            let mut body = count_oobs(&state);
            body.push_str(
                &Toast::success("Source removed.")
                    .with_action("/admin/system/restart", "Restart to apply")
                    .render_oob(),
            );
            Html(body).into_response()
        }
        Err(AudioSourceError::NotFound(_)) => {
            toast::oob_only(Toast::warn("Source already removed.")).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "audio source soft-delete failed");
            internal_response("Could not remove the source.")
        }
    }
}

async fn edit_form(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let result = state.with_db(|conn| conn.get(&id));
    let row = match result {
        Ok(Some(row)) => row,
        Ok(None) => return not_found_row(&id),
        Err(e) => {
            tracing::error!(error = %e, "audio source get failed");
            return internal_response("Could not load that source.");
        }
    };
    Html(render_edit_form(&row)).into_response()
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<CreateForm>,
) -> Response {
    let mut patch = AudioSourcePatch::default();
    let device_id = form.device_id.trim().to_string();
    if !device_id.is_empty() {
        patch.device_id = Some(device_id);
    }
    let label = form.label.map(|s| {
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    if let Some(l) = label {
        patch.label = Some(l);
    }
    if let Some(rate) = form.sample_rate {
        patch.sample_rate = Some(rate);
    }
    if let Some(transport) = form.rtsp_transport.as_deref() {
        match transport.parse::<RtspTransport>() {
            Ok(t) => patch.rtsp_transport = Some(t),
            Err(_) => return validation_response("Unknown RTSP transport."),
        }
    }
    if let Some(toggles) = ToggleSet::from_form(
        form.pipeline_present.as_deref(),
        form.high_pass.as_deref(),
        form.dc_removal.as_deref(),
        form.agc.as_deref(),
        form.rtsp_keepalive.as_deref(),
    ) {
        patch.pipeline = Some(toggles.into_flags());
    }
    match parse_quiet_choice(form.quiet_start.as_deref(), form.quiet_end.as_deref()) {
        Ok(QuietChoice::Set(a, b)) => patch.schedule_quiet = Some(Some((a, b))),
        Ok(QuietChoice::Clear) => patch.schedule_quiet = Some(None),
        Ok(QuietChoice::Absent) => {}
        Err(msg) => return validation_response(msg),
    }

    let result = state.with_db(|conn| conn.update(&id, &patch));
    match result {
        Ok(row) => {
            let mut body = render_row(&row, daemon_status(&row, &state));
            body.push_str(
                &Toast::success("Source updated.")
                    .with_action("/admin/system/restart", "Restart to apply")
                    .render_oob(),
            );
            Html(body).into_response()
        }
        Err(AudioSourceError::NotFound(_)) => not_found_row(&id),
        Err(AudioSourceError::Invalid(msg)) => validation_response(&msg),
        Err(e) => {
            tracing::error!(error = %e, "audio source update failed");
            internal_response("Could not update the source.")
        }
    }
}

async fn probe(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let row = match state.with_db(|conn| conn.get(&id)) {
        Ok(Some(row)) => row,
        Ok(None) => return not_found_row(&id),
        Err(e) => {
            tracing::error!(error = %e, "audio source probe failed");
            return internal_response("Could not probe that source.");
        }
    };
    let status = daemon_status(&row, &state);
    Html(render_status_pill(&row.id, status)).into_response()
}

/// Return the read-only row for one source. Used by the edit form's **Cancel**
/// button to restore the row view. (The previous Cancel fetched `/probe` with
/// `hx-swap="none"`, which fetched the status pill but swapped nothing, leaving
/// the edit form stuck open — Cancel appeared to do nothing.)
async fn row(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match state.with_db(|conn| conn.get(&id)) {
        Ok(Some(src)) => Html(render_row(&src, daemon_status(&src, &state))).into_response(),
        Ok(None) => not_found_row(&id),
        Err(e) => {
            tracing::error!(error = %e, "audio source get failed");
            internal_response("Could not load that source.")
        }
    }
}

// ---------------------------------------------------------------------------
// Daemon-side probe — reads metrics::source_up(row.id), the per-source
// liveness the capture supervisor publishes.
// ---------------------------------------------------------------------------

/// Map an `audio_sources` row to a status pill from the
/// `birdnet_audio_source_up{source}` gauge.
///
/// The capture supervisor publishes per-source liveness keyed by `row.id`;
/// the handler reads the same label back, so the pill reflects the actual
/// reconcile state — `Capturing` when up, `Down` when the subprocess is dead /
/// backing off, or when no gauge has been published yet (supervisor not
/// started, or the source not yet reconciled). `row.disabled_at`
/// short-circuits to `Down` ahead of the gauge.
fn daemon_status(row: &AudioSource, state: &AppState) -> Status {
    if row.disabled_at.is_some() {
        return Status::Down;
    }
    match state.metrics().source_up(&row.id) {
        Some(true) => Status::Capturing,
        _ => Status::Down,
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// "0 mics" / "1 mic" / "3 streams" — the count-chip text for a section header,
/// shared by the initial render and the out-of-band refresh after an add/remove.
fn count_text(n: usize, singular: &str, plural: &str) -> String {
    format!("{n} {}", if n == 1 { singular } else { plural })
}

/// One section count chip as an out-of-band swap, so an add/remove can refresh
/// "N mics" / "N streams" in place without re-rendering (and re-wiring) the
/// whole panel. The `id` matches the chip in `admin_audio_sources.html`.
fn oob_count(span_id: &str, n: usize, singular: &str, plural: &str) -> String {
    format!(
        r#"<span id="{span_id}" class="bnb-meta mono" hx-swap-oob="true">{}</span>"#,
        escape_html(&count_text(n, singular, plural))
    )
}

/// Both count chips recomputed from the live source list — appended to the
/// add/remove responses so the header totals stay truthful the instant a source
/// is added or removed.
fn count_oobs(state: &AppState) -> String {
    let sources = state.with_db(AudioSourceStore::list).unwrap_or_default();
    let (local, rtsp): (Vec<&AudioSource>, Vec<&AudioSource>) = sources
        .iter()
        .partition(|s| !matches!(s.kind, SourceKind::Rtsp));
    format!(
        "{}{}",
        oob_count("aas-count-local", local.len(), "mic", "mics"),
        oob_count("aas-count-rtsp", rtsp.len(), "stream", "streams"),
    )
}

fn render_body(sources: &[AudioSource], status_for: impl Fn(&AudioSource) -> Status) -> String {
    let (local, rtsp): (Vec<&AudioSource>, Vec<&AudioSource>) = sources
        .iter()
        .partition(|s| !matches!(s.kind, SourceKind::Rtsp));

    let mut rows_local = String::new();
    for s in &local {
        rows_local.push_str(&render_row(s, status_for(s)));
    }
    let mut rows_rtsp = String::new();
    for s in &rtsp {
        rows_rtsp.push_str(&render_row(s, status_for(s)));
    }

    // Both sections are always rendered (never hidden): hiding a section when
    // its kind was empty stranded operators who had a mic but no stream — the
    // "Add stream" form lived inside the hidden RTSP section, so it was
    // unreachable. The counts below are refreshed in place via `count_oobs`.
    PAGE_TPL
        .replace("{{rows_local}}", &rows_local)
        .replace("{{rows_rtsp}}", &rows_rtsp)
        .replace(
            "{{count_local}}",
            &escape_html(&count_text(local.len(), "mic", "mics")),
        )
        .replace(
            "{{count_rtsp}}",
            &escape_html(&count_text(rtsp.len(), "stream", "streams")),
        )
        .replace("{{pending_changes}}", "")
}

fn render_row(s: &AudioSource, status: Status) -> String {
    let (label_class, label_text) = match s.label.as_deref() {
        Some(l) if !l.is_empty() => ("", l.to_string()),
        _ => ("untitled", "— no friendly label —".to_string()),
    };
    let label_raw = s.label.as_deref().unwrap_or("");
    // `{{label}}` only appears in the template's documentation comment;
    // substitute it for the raw value to keep the no-placeholder
    // invariant honest after render.
    ROW_TPL
        .replace("{{id}}", &escape_html(&s.id))
        .replace("{{kind_class}}", kind_css(s.kind))
        .replace("{{kind_label}}", kind_label(s.kind))
        .replace("{{label_class}}", label_class)
        .replace("{{label_text}}", &escape_html(&label_text))
        .replace("{{label}}", &escape_html(label_raw))
        .replace("{{device_id}}", &escape_html(&s.device_id))
        .replace("{{detail_line}}", &escape_html(&detail_for(s)))
        .replace("{{status_class}}", status.css())
        .replace("{{status_label}}", status.label())
        .replace("{{meta_line}}", &escape_html(&meta_for(s)))
}

/// The replacement pill returned by `/probe`, and therefore the one that keeps
/// polling from the second swap onwards.
///
/// It carries `hx-target="this"` for the same reason the template's pill does:
/// it lands back inside `<li class="audio-source" hx-target="this">`, and an
/// inherited `"this"` resolves to the declaring `<li>`, not to the pill. Omit it
/// here and the row survives exactly one swap before the next poll replaces the
/// whole row with a bare status span.
fn render_status_pill(id: &str, status: Status) -> String {
    format!(
        r#"<span class="bnb-pill {cls}"
  hx-get="/admin/audio/sources/{id}/probe"
  hx-trigger="every 8s"
  hx-target="this"
  hx-swap="outerHTML"
  aria-live="polite">
  <span class="bnb-dot {cls}" aria-hidden="true"></span>{label}</span>"#,
        cls = status.css(),
        label = status.label(),
        id = escape_html(id),
    )
}

fn render_edit_form(row: &AudioSource) -> String {
    let label = row.label.clone().unwrap_or_default();
    // Checkbox state for the four per-source conditioning toggles. An unchecked
    // HTML checkbox submits *nothing*, which is what lets `Option<String>` in
    // the form struct tell "operator cleared it" from "form does not carry the
    // control" — the same distinction the quiet window already relies on.
    let checked = |on: bool| if on { " checked" } else { "" };
    let high_pass_checked = checked(row.pipeline.high_pass);
    let dc_removal_checked = checked(row.pipeline.dc_removal);
    let agc_checked = checked(row.pipeline.agc);
    let rtsp_keepalive_checked = checked(row.pipeline.rtsp_keepalive);
    // `("", "")` when no window is set — an `<input type="time">` with an empty
    // value renders as the blank "--:--" the operator needs in order to say
    // "none", which is why both ends are submitted even when unset.
    let quiet = row
        .schedule_quiet
        .as_ref()
        .map_or(("", ""), |(a, b)| (a.as_str(), b.as_str()));
    format!(
        r#"<li class="audio-source" data-source-id="{id}">
  <form hx-patch="/admin/audio/sources/{id}"
        hx-target="closest li"
        hx-swap="outerHTML"
        class="aud-edit-form">
    <input type="hidden" name="kind" value="{kind}">
    <div class="audio-source__kind">
      <span class="audio-kind-badge {kind_class}">
        <span class="audio-kind-glyph" aria-hidden="true"></span>
        {kind_label}
      </span>
    </div>
    <div class="audio-source__id aud-edit-id">
      <input name="label" type="text" placeholder="Friendly label" value="{label}"
             class="aud-edit-label">
      <input name="device_id" class="mono aud-edit-device" type="text" value="{device_id}" required>
    </div>
    <div class="aud-edit-quiet">
      <span class="bnb-eyebrow">Quiet window</span>
      <label class="sr-only" for="q-start-{id}">Quiet from</label>
      <input id="q-start-{id}" name="quiet_start" type="time" value="{quiet_start}"
             class="aud-edit-time">
      <span class="bnb-meta" aria-hidden="true">→</span>
      <label class="sr-only" for="q-end-{id}">Quiet until</label>
      <input id="q-end-{id}" name="quiet_end" type="time" value="{quiet_end}"
             class="aud-edit-time">
      <p class="hint">This source stops recording between these times, in the station's
        local time. Leave both blank for none.</p>
    </div>
    <div class="aud-edit-pipeline">
      <input type="hidden" name="pipeline_present" value="1">
      <span class="bnb-eyebrow">Signal conditioning</span>
      <label><input type="checkbox" name="high_pass" value="1"{high_pass_checked}>
        High-pass &mdash; cut wind rumble below 120&nbsp;Hz</label>
      <label><input type="checkbox" name="dc_removal" value="1"{dc_removal_checked}>
        Remove DC offset</label>
      <label><input type="checkbox" name="agc" value="1"{agc_checked}>
        Automatic gain control</label>
      <label><input type="checkbox" name="rtsp_keepalive" value="1"{rtsp_keepalive_checked}>
        Drop a stalled RTSP stream after 10&nbsp;s so it restarts</label>
      <p class="hint">Applied to this source before analysis. The first three also
        shape what you hear on the live stream; the last one affects RTSP only.</p>
    </div>
    <div class="audio-source__right aud-edit-right">
      <button type="submit" class="bnb-btn moss">Save</button>
      <button type="button" class="bnb-btn ghost"
              hx-get="/admin/audio/sources/{id}"
              hx-target="closest li"
              hx-swap="outerHTML">Cancel</button>
    </div>
  </form>
</li>"#,
        id = escape_html(&row.id),
        kind = row.kind.as_str(),
        kind_class = kind_css(row.kind),
        kind_label = kind_label(row.kind),
        label = escape_html(&label),
        device_id = escape_html(&row.device_id),
        quiet_start = escape_html(quiet.0),
        quiet_end = escape_html(quiet.1),
        high_pass_checked = high_pass_checked,
        dc_removal_checked = dc_removal_checked,
        agc_checked = agc_checked,
        rtsp_keepalive_checked = rtsp_keepalive_checked,
    )
}

/// One-line technical summary of a source: rate, channels, depth, gain.
///
/// `pub(crate)` for the same reason as [`kind_label`] — the onboarding
/// wizard shows the operator's real capture settings rather than a
/// plausible-looking constant.
pub(crate) fn detail_for(s: &AudioSource) -> String {
    let rate_khz = f64::from(s.sample_rate) / 1000.0;
    let mut out = format!(
        "{rate_khz:.1} kHz · {channels} · {bit}-bit",
        channels = s.channels.as_str(),
        bit = s.bit_depth,
    );
    // Stereo is the one channel setting that can quietly cost detections, so
    // it says so where the operator chose it. Both channels are kept and the
    // decoder averages them to the mono BirdNET wants; for a *spaced* pair that
    // average is a comb filter rather than a noise reduction — measured through
    // this project's decode path, a wavefront reaching the capsules half a
    // period apart loses about 66 dB, a quarter period costs 3 dB, and the
    // notches move with the bird's direction. Left or Right avoids it by
    // selecting a channel instead of mixing.
    if matches!(s.channels, birdnet_db::audio_sources::Channels::Stereo) {
        out.push_str(
            " · both channels are averaged to mono; on a spaced pair that can \
             cancel signal — pick Left or Right unless the capsules are together",
        );
    }
    if (s.gain_db - 0.0).abs() >= 0.05 {
        let sign = if s.gain_db >= 0.0 { "+" } else { "" };
        let _ = write!(out, " · gain {sign}{:.0} dB", s.gain_db);
    }
    if matches!(s.kind, SourceKind::Rtsp) {
        let _ = write!(out, " · {} transport", s.rtsp_transport.as_str());
    }
    // A quiet window is the one setting here that makes a source *stop*, so it
    // has to be legible without opening the edit form — a source that looks
    // configured and is silent every night is otherwise indistinguishable from
    // one that has failed.
    if let Some((from, to)) = s.schedule_quiet.as_ref() {
        let _ = write!(out, " · quiet {from}–{to} local");
    }
    out
}

fn meta_for(s: &AudioSource) -> String {
    s.disabled_at.as_ref().map_or_else(
        || format!("added {}", s.created_at),
        |ts| format!("disabled {ts}"),
    )
}

fn synth_id(kind: SourceKind) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    // A process-local sequence guarantees uniqueness even for two sources added
    // within the same second. The old `src_<kind>_<secs>` form collided then, and
    // the collision surfaced to the operator as a baffling "Retry — a new id will
    // be generated" toast on a perfectly valid second add. The timestamp stays
    // for human readability; the counter is what actually guarantees uniqueness.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let prefix = match kind {
        SourceKind::UsbAlsa => "usb",
        SourceKind::PipeWire => "pw",
        SourceKind::Rtsp => "rtsp",
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("src_{prefix}_{secs}_{seq}")
}

// ---------------------------------------------------------------------------
// Error rendering
// ---------------------------------------------------------------------------

fn validation_response(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        toast::oob_only(Toast::warn(message)),
    )
        .into_response()
}

fn internal_response(message: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        toast::oob_only(Toast::error(message)),
    )
        .into_response()
}

fn not_found_row(id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        toast::oob_only(Toast::warn(format!("Source {id} no longer exists."))),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::audio_sources::{Channels, PipelineFlags};
    use tempfile::TempDir;

    fn fixture() -> (TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("birds.db");
        let _conn = birdnet_db::sqlite::open_or_create(&db_path).expect("open db");
        let state = AppState::new(db_path).expect("state");
        (dir, state)
    }

    fn insert_one(state: &AppState, id: &str, kind: SourceKind, device_id: &str) -> AudioSource {
        let new = NewAudioSource::defaults(id, kind, device_id);
        state
            .with_db(|conn| conn.insert(&new))
            .expect("insert succeeds")
    }

    #[test]
    fn render_body_empty_shows_both_add_sections() {
        let html = render_body(&[], |_| Status::Down);
        // Both add-affordances are always present, even from a blank slate, so an
        // operator can add either a mic or a stream (the old layout hid the RTSP
        // section until one existed, stranding anyone who had only a mic).
        assert!(html.contains("Add a microphone"));
        assert!(html.contains("Add an RTSP stream"));
        // Zeroed counts, and no separate (contradictory) empty-state card.
        assert!(html.contains("0 mics"));
        assert!(html.contains("0 streams"));
        assert!(!html.contains("No audio sources yet"));
        assert!(!html.contains("{{"));
    }

    #[test]
    fn render_body_partitions_by_kind() {
        let (_d, state) = fixture();
        insert_one(&state, "src_u", SourceKind::UsbAlsa, "hw:1,0");
        insert_one(&state, "src_r", SourceKind::Rtsp, "rtsp://x/y");
        let sources = state.with_db(AudioSourceStore::list).unwrap();
        let html = render_body(&sources, |_| Status::Down);
        assert!(!html.contains("{{"));
        assert!(html.contains("hw:1,0"));
        assert!(html.contains("rtsp://x/y"));
        // Both sections always render; the counts reflect one of each.
        assert!(html.contains("1 mic"));
        assert!(html.contains("1 stream"));
    }

    #[test]
    fn count_oobs_reports_per_kind_totals() {
        let (_d, state) = fixture();
        insert_one(&state, "src_u", SourceKind::UsbAlsa, "hw:1,0");
        insert_one(&state, "src_u2", SourceKind::UsbAlsa, "hw:2,0");
        insert_one(&state, "src_r", SourceKind::Rtsp, "rtsp://x/y");
        let oob = count_oobs(&state);
        assert!(oob.contains(r#"id="aas-count-local""#));
        assert!(oob.contains(r#"id="aas-count-rtsp""#));
        assert!(oob.contains(r#"hx-swap-oob="true""#));
        assert!(oob.contains("2 mics"));
        assert!(oob.contains("1 stream"));
    }

    /// End to end through the real handlers: a quiet window set on the form
    /// reaches the database, is cleared by blanking both ends, and a half-filled
    /// form is refused.
    ///
    /// The parser unit tests above cannot show this. `schedule_quiet` had a
    /// column, a parser and supervisor enforcement, and *nothing wrote it* —
    /// every construction site in the tree passed `None` — so the feature was
    /// reachable only by direct SQL. This is the gate that the wiring exists.
    #[tokio::test]
    async fn a_quiet_window_set_on_the_form_reaches_the_database() {
        let (_d, state) = fixture();
        let base = || CreateForm {
            high_pass: None,
            dc_removal: None,
            agc: None,
            rtsp_keepalive: None,
            pipeline_present: None,
            scope: String::new(),
            kind: "usb-alsa".to_string(),
            device_id: "plughw:2,0".to_string(),
            label: None,
            sample_rate: None,
            rtsp_transport: None,
            quiet_start: None,
            quiet_end: None,
        };

        let mut form = base();
        form.quiet_start = Some("22:00".to_string());
        form.quiet_end = Some("06:00".to_string());
        assert_eq!(
            create(State(state.clone()), Form(form)).await.status(),
            StatusCode::OK
        );

        let stored = |state: &crate::state::AppState| {
            state
                .with_db(birdnet_db::audio_sources::AudioSourceStore::list)
                .unwrap_or_default()
                .into_iter()
                .find(|s| s.device_id == "plughw:2,0")
                .expect("source exists")
                .schedule_quiet
        };
        assert_eq!(
            stored(&state),
            Some(("22:00".to_string(), "06:00".to_string())),
            "the window the operator typed must be what the supervisor reads"
        );

        let id = state
            .with_db(birdnet_db::audio_sources::AudioSourceStore::list)
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.device_id == "plughw:2,0")
            .expect("source")
            .id;

        // Half a window is refused, and leaves the stored one untouched.
        let mut half = base();
        half.quiet_start = Some("23:00".to_string());
        half.quiet_end = Some(String::new());
        let refused = update(State(state.clone()), Path(id.clone()), Form(half)).await;
        assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            stored(&state),
            Some(("22:00".to_string(), "06:00".to_string())),
            "a refused edit must not have changed anything"
        );

        // Both blank clears it.
        let mut clear = base();
        clear.quiet_start = Some(String::new());
        clear.quiet_end = Some(String::new());
        assert_eq!(
            update(State(state.clone()), Path(id), Form(clear))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            stored(&state),
            None,
            "blanking both ends removes the window"
        );
    }

    #[tokio::test]
    async fn create_rejects_duplicate_device() {
        let (_d, state) = fixture();
        let form = || CreateForm {
            high_pass: None,
            dc_removal: None,
            agc: None,
            rtsp_keepalive: None,
            pipeline_present: None,
            scope: String::new(),
            kind: "usb-alsa".to_string(),
            device_id: "plughw:1,0".to_string(),
            label: None,
            sample_rate: None,
            rtsp_transport: None,
            quiet_start: None,
            quiet_end: None,
        };
        let first = create(State(state.clone()), Form(form())).await;
        assert_eq!(first.status(), StatusCode::OK, "first add succeeds");
        let second = create(State(state.clone()), Form(form())).await;
        assert_eq!(
            second.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "the same device cannot be added twice"
        );
        let count = state.with_db(AudioSourceStore::list).unwrap().len();
        assert_eq!(count, 1, "only one row persisted");
    }

    #[tokio::test]
    async fn row_endpoint_returns_row_or_not_found() {
        let (_d, state) = fixture();
        insert_one(&state, "src_u", SourceKind::UsbAlsa, "hw:1,0");
        let found = row(State(state.clone()), Path("src_u".to_string())).await;
        assert_eq!(found.status(), StatusCode::OK);
        let missing = row(State(state.clone()), Path("nope".to_string())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn render_body_initial_paint_reflects_metrics_gauge() {
        // First paint must agree with the per-row `/probe` poll: a source
        // whose supervisor gauge reads "up" renders `Capturing` immediately,
        // not a legacy heuristic. Mirrors what `page()` passes to render_body.
        let (_d, state) = fixture();
        insert_one(&state, "src_live", SourceKind::UsbAlsa, "hw:1,0");
        insert_one(&state, "src_dead", SourceKind::Rtsp, "rtsp://x/y");
        state.metrics().set_source_up("src_live", true);
        state.metrics().set_source_up("src_dead", false);
        let sources = state.with_db(AudioSourceStore::list).unwrap();
        let html = render_body(&sources, |row| daemon_status(row, &state));
        assert!(
            html.contains("Capturing"),
            "live row should paint Capturing"
        );
        assert!(html.contains("Down"), "dead row should paint Down");
    }

    #[test]
    fn render_row_substitutes_all_placeholders() {
        let source = AudioSource {
            id: "src_test_1".to_string(),
            kind: SourceKind::Rtsp,
            device_id: "rtsp://x".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Tcp,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28 12:00:00".to_string(),
            updated_at: "2026-05-28 12:00:00".to_string(),
        };
        let html = render_row(&source, Status::Down);
        assert!(
            !html.contains("{{"),
            "unsubstituted placeholder in:\n{html}"
        );
        assert!(html.contains("rtsp://x"));
        assert!(html.contains("Down"));
        assert!(html.contains("untitled"));
        assert!(html.contains("RTSP"));
    }

    #[test]
    fn render_row_includes_label_when_set() {
        let source = AudioSource {
            id: "src_x".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "hw:1,0".to_string(),
            label: Some("Backyard feeder".to_string()),
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 12.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        };
        let html = render_row(&source, Status::Capturing);
        assert!(html.contains("Backyard feeder"));
        // The "untitled" class is only added in the no-label case. The
        // template's doc comment mentions the word, so match on the
        // attribute instead of a substring.
        assert!(html.contains(r#"class="audio-source__label ""#));
        assert!(!html.contains(r#"class="audio-source__label untitled""#));
        assert!(html.contains("gain +12 dB"));
        assert!(html.contains("Capturing"));
    }

    #[test]
    fn render_status_pill_html_is_self_polling() {
        let html = render_status_pill("src_a", Status::Capturing);
        assert!(html.contains(r#"hx-get="/admin/audio/sources/src_a/probe""#));
        assert!(html.contains(r#"hx-trigger="every 8s""#));
        assert!(html.contains("Capturing"));
    }

    /// The poll must replace the PILL, never the row.
    ///
    /// `hx-target` is inherited, and the enclosing `<li>` declares
    /// `hx-target="this"`. htmx resolves an inherited `"this"` to the element
    /// that declares the attribute, so a pill without its own `hx-target`
    /// swapped the probe response over the entire row — the microphone
    /// disappeared from the admin page ~8 s after load, on a station whose mic
    /// was down, which is exactly when an operator is looking at it.
    #[test]
    fn status_pill_targets_itself_not_the_enclosing_row() {
        let pill = render_status_pill("src_a", Status::Down);
        assert!(
            pill.contains(r#"hx-target="this""#),
            "the /probe replacement pill must target itself, got: {pill}"
        );

        // Same for the pill the row template ships, which is the one that runs
        // the first poll after a page load.
        let source = AudioSource {
            id: "src_a".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "plughw:CARD=PRO,DEV=0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-08-10".to_string(),
            updated_at: "2026-08-10".to_string(),
        };
        let row = render_row(&source, Status::Down);
        let pill_start = row
            .find(r#"<span class="bnb-pill"#)
            .expect("row renders a status pill");
        let pill_end = row[pill_start..].find('>').expect("pill tag closes") + pill_start;
        let pill_tag = &row[pill_start..pill_end];
        assert!(
            pill_tag.contains(r#"hx-target="this""#),
            "the row template's pill must target itself, got: {pill_tag}"
        );
    }

    // ---- quiet windows -------------------------------------------------
    //
    // The columns, the parser and the supervisor's enforcement all shipped
    // together; the *form* did not. Every construction site in the tree passed
    // `schedule_quiet: None`, so the only way to set one was direct SQL against
    // the database — which for an enclosure near a road or a neighbour is the
    // difference between a usable feature and a schema comment.

    #[test]
    fn a_quiet_window_round_trips_through_the_form() {
        assert_eq!(
            parse_quiet_choice(Some("22:00"), Some("06:00")),
            Ok(QuietChoice::Set("22:00".into(), "06:00".into()))
        );
        // Whitespace from a hand-edited form field is not a typo.
        assert_eq!(
            parse_quiet_choice(Some(" 22:00 "), Some(" 06:00 ")),
            Ok(QuietChoice::Set("22:00".into(), "06:00".into()))
        );
    }

    /// Both blank clears the window; neither field present leaves it alone.
    ///
    /// The distinction matters: a form that does not carry the control at all
    /// must not wipe a window the operator set elsewhere, while a form that
    /// carries it with both ends blank is the operator saying "none".
    #[test]
    fn blank_clears_and_absent_leaves_alone() {
        assert_eq!(
            parse_quiet_choice(Some(""), Some("")),
            Ok(QuietChoice::Clear)
        );
        assert_eq!(
            parse_quiet_choice(Some("  "), Some("")),
            Ok(QuietChoice::Clear)
        );
        assert_eq!(parse_quiet_choice(None, None), Ok(QuietChoice::Absent));
        assert_eq!(
            parse_quiet_choice(Some("22:00"), None),
            Ok(QuietChoice::Absent),
            "a partial form is not a half-window"
        );
    }

    /// Half a window is not a window. Accepting it would store something the
    /// supervisor cannot act on while the UI showed the value the operator
    /// typed, which reads as "saved".
    #[test]
    fn one_blank_end_is_rejected() {
        assert!(parse_quiet_choice(Some("22:00"), Some("")).is_err());
        assert!(parse_quiet_choice(Some(""), Some("06:00")).is_err());
    }

    #[test]
    fn malformed_and_empty_windows_are_rejected() {
        for (a, b) in [
            ("25:00", "06:00"),
            ("22:00", "06:60"),
            ("2200", "0600"),
            ("22:0", "06:00"),
            ("22:00:00", "06:00"),
            ("", "x"),
        ] {
            assert!(
                parse_quiet_choice(Some(a), Some(b)).is_err(),
                "{a:?}..{b:?} should be rejected"
            );
        }
        // Start == end is an empty window, not a 24-hour one.
        assert!(parse_quiet_choice(Some("06:00"), Some("06:00")).is_err());
    }

    /// The edit form has to render the control, or the setting stays
    /// unreachable no matter what the parser does.
    #[test]
    fn the_edit_form_exposes_the_quiet_window() {
        let mut source = AudioSource {
            id: "src_x".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "hw:1,0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        };
        let html = render_edit_form(&source);
        assert!(html.contains(r#"name="quiet_start""#), "no start field");
        assert!(html.contains(r#"name="quiet_end""#), "no end field");
        assert!(
            html.contains(r#"name="quiet_start" type="time" value=""#),
            "an unset window must render blank so the operator can leave it unset"
        );

        source.schedule_quiet = Some(("22:00".to_string(), "06:00".to_string()));
        let html = render_edit_form(&source);
        assert!(html.contains(r#"value="22:00""#), "{html}");
        assert!(html.contains(r#"value="06:00""#));
    }

    /// A source that goes quiet every night must say so on its row. Otherwise a
    /// silent source is indistinguishable from a broken one.
    #[test]
    fn the_row_summary_names_a_quiet_window() {
        let source = AudioSource {
            id: "src_x".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "hw:1,0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: Some(("22:00".to_string(), "06:00".to_string())),
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        };
        let detail = detail_for(&source);
        assert!(detail.contains("quiet 22:00–06:00 local"), "got {detail:?}");
        // …and says nothing when there is no window.
        let mut none = source;
        none.schedule_quiet = None;
        assert!(!detail_for(&none).contains("quiet"));
    }

    #[test]
    fn render_edit_form_carries_existing_values() {
        let source = AudioSource {
            id: "src_x".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "hw:1,0".to_string(),
            label: Some("Backyard feeder".to_string()),
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        };
        let html = render_edit_form(&source);
        assert!(html.contains(r#"value="Backyard feeder""#));
        assert!(html.contains(r#"value="hw:1,0""#));
        assert!(html.contains(r#"name="kind" value="usb-alsa""#));
    }

    #[test]
    fn real_probe_reads_metrics_gauge_by_row_id() {
        // Build a state, publish a per-row gauge value via the metrics
        // API, and confirm the probe handler returns the matching pill.
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        let state =
            crate::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));

        let row = AudioSource {
            id: "src_garden".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "hw:1,0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        };

        // No gauge published yet → Down (the supervisor hasn't reconciled
        // this source).
        assert!(matches!(daemon_status(&row, &state), Status::Down));

        // Publish gauge: row is up.
        state.metrics().set_source_up("src_garden", true);
        assert!(matches!(daemon_status(&row, &state), Status::Capturing));

        // Publish gauge: row went down (subprocess died, supervisor's
        // next reconcile would clear).
        state.metrics().set_source_up("src_garden", false);
        assert!(matches!(daemon_status(&row, &state), Status::Down));

        // Disabled row short-circuits to Down regardless of the gauge.
        let disabled = AudioSource {
            disabled_at: Some("2026-05-28".to_string()),
            ..row
        };
        state.metrics().set_source_up("src_garden", true);
        assert!(matches!(daemon_status(&disabled, &state), Status::Down));
    }

    #[test]
    fn detail_for_rtsp_includes_transport() {
        let s = AudioSource {
            id: "x".to_string(),
            kind: SourceKind::Rtsp,
            device_id: "rtsp://x".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Tcp,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: None,
            created_at: "x".to_string(),
            updated_at: "x".to_string(),
        };
        let detail = detail_for(&s);
        assert!(detail.contains("48.0 kHz"));
        assert!(detail.contains("tcp transport"));
    }

    #[test]
    fn synth_id_uses_kind_prefix() {
        let id = synth_id(SourceKind::Rtsp);
        assert!(id.starts_with("src_rtsp_"));
    }
}

// ── the conditioning toggles reach the operator and the daemon ──────────
//
// These four flags were stored, defaulted and round-tripped by the store, and
// the edit form never rendered a control for any of them — so the only way to
// change one was direct SQL. The audio path never read them either; that half
// is fixed in `birdnet-core`. This half is the form.
#[cfg(test)]
mod pipeline_toggle_tests {
    use super::*;
    use birdnet_db::audio_sources::Channels;

    fn source_with(pipeline: PipelineFlags) -> AudioSource {
        AudioSource {
            id: "mic".to_string(),
            kind: SourceKind::UsbAlsa,
            device_id: "plughw:1,0".to_string(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 16,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline,
            disabled_at: None,
            created_at: "2026-05-28".to_string(),
            updated_at: "2026-05-28".to_string(),
        }
    }

    /// The gate for the missing controls: the edit form must carry all four.
    /// Fails before the fix — the form had no checkbox at all.
    #[test]
    fn the_edit_form_renders_every_toggle() {
        let html = render_edit_form(&source_with(PipelineFlags::default()));
        for field in ["high_pass", "dc_removal", "agc", "rtsp_keepalive"] {
            assert!(
                html.contains(&format!(r#"name="{field}""#)),
                "the edit form has no control for {field}"
            );
        }
        assert!(
            html.contains(r#"name="pipeline_present""#),
            "the hidden marker must be present or an unticked box reads as absent"
        );
    }

    /// A stored value has to come back checked, or saving the form would
    /// silently clear whatever was set.
    #[test]
    fn stored_state_is_reflected_in_the_checkboxes() {
        let on = render_edit_form(&source_with(PipelineFlags {
            high_pass: true,
            dc_removal: false,
            agc: true,
            rtsp_keepalive: false,
        }));
        // Exactly the two that are on carry `checked`.
        assert_eq!(
            on.matches(" checked").count(),
            2,
            "two toggles are on, so exactly two boxes are checked"
        );
        assert!(on.contains(r#"name="high_pass" value="1" checked"#));
        assert!(on.contains(r#"name="dc_removal" value="1">"#));
    }

    /// An unchecked box submits nothing, so "off" and "this form has no such
    /// control" arrive identically. The hidden marker is what separates them.
    #[test]
    fn absent_marker_means_leave_the_stored_flags_alone() {
        assert_eq!(
            ToggleSet::from_form(None, Some("1"), None, None, None),
            None,
            "without the marker the submission must not touch the flags"
        );
    }

    /// The counterpart: with the marker, an unticked box really does mean off.
    /// Without this the guard above would be satisfied by never applying the
    /// toggles at all.
    #[test]
    fn present_marker_applies_every_box_including_the_unticked_ones() {
        let set = ToggleSet::from_form(Some("1"), Some("1"), None, None, Some("1"))
            .expect("marker present");
        assert_eq!(
            set.into_flags(),
            PipelineFlags {
                high_pass: true,
                dc_removal: false,
                agc: false,
                rtsp_keepalive: true,
            }
        );
    }
}
