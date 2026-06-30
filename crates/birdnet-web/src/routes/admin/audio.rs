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
    RtspTransport, SourceKind,
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

const fn kind_label(kind: SourceKind) -> &'static str {
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
    axum::response::Redirect::permanent("/station/capture")
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

fn render_status_pill(id: &str, status: Status) -> String {
    format!(
        r#"<span class="bnb-pill {cls}"
  hx-get="/admin/audio/sources/{id}/probe"
  hx-trigger="every 8s"
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
    )
}

fn detail_for(s: &AudioSource) -> String {
    let rate_khz = f64::from(s.sample_rate) / 1000.0;
    let mut out = format!(
        "{rate_khz:.1} kHz · {channels} · {bit}-bit",
        channels = s.channels.as_str(),
        bit = s.bit_depth,
    );
    if (s.gain_db - 0.0).abs() >= 0.05 {
        let sign = if s.gain_db >= 0.0 { "+" } else { "" };
        let _ = write!(out, " · gain {sign}{:.0} dB", s.gain_db);
    }
    if matches!(s.kind, SourceKind::Rtsp) {
        let _ = write!(out, " · {} transport", s.rtsp_transport.as_str());
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
    use std::time::{SystemTime, UNIX_EPOCH};
    let prefix = match kind {
        SourceKind::UsbAlsa => "usb",
        SourceKind::PipeWire => "pw",
        SourceKind::Rtsp => "rtsp",
    };
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("src_{prefix}_{secs}")
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

    #[tokio::test]
    async fn create_rejects_duplicate_device() {
        let (_d, state) = fixture();
        let form = || CreateForm {
            scope: String::new(),
            kind: "usb-alsa".to_string(),
            device_id: "plughw:1,0".to_string(),
            label: None,
            sample_rate: None,
            rtsp_transport: None,
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
