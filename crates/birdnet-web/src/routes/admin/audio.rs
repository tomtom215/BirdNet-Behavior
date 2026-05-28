//! Audio sources management — first-class CRUD page (O-13).
//!
//! Replaces the prior hard-coded two-row stub. The page body is rendered
//! from `templates/admin_audio_sources.html` + per-row partials in
//! `templates/_partial_audio_source_row.html`; live status comes from
//! `probe(id)` which today returns a stable `Capturing` for any row the
//! audio daemon knows about and `Down` otherwise.
//!
//! ## O-13 wiring status
//!
//! **Stage 1 (shipped, this PR's sibling)**: the capture pipeline in
//! `src/capture.rs` resolves sources from `audio_sources` rows when the
//! table is non-empty, falling back to the CLI/config path otherwise.
//! See `capture::resolve_sources_from_db`. `state.audio_source()` is
//! retained for back-compat consumers (livestream, the live-spectrogram
//! producer, the listen-now page's "default" option) — those keep
//! reading the legacy single-string value while operators migrate.
//!
//! **TODO(O-13-followup)** — still open on the daemon side:
//!
//! 1. `probe(id)` is intentionally synthetic in this PR: it returns
//!    `Capturing` for the first non-disabled row and `Down` for the
//!    others, because the per-source `is_capturing` flag does not exist
//!    in `birdnet-core` yet. The replacement is a daemon-side metrics
//!    handle keyed on `audio_source.id`. The handler is otherwise wired
//!    end-to-end so the swap is one function body.
//!
//! 2. The seed migration's `settings.audio_source` cross-reference and
//!    the `with_audio_source` builder stay in place during this
//!    transition; they can be retired once every consumer of
//!    `state.audio_source()` (livestream `/stream` default-source path,
//!    live-spectrogram producer, listen-now "default" option, and a
//!    handful of test fixtures) has been migrated to read the
//!    `audio_sources` table directly.

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

use super::admin_shell;
use crate::routes::pages::escape_html;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

// Embedded templates.
const PAGE_TPL: &str = include_str!("../../../templates/admin_audio_sources.html");
const ROW_TPL: &str = include_str!("../../../templates/_partial_audio_source_row.html");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/audio", get(page))
        .route("/admin/audio/sources", post(create))
        .route(
            "/admin/audio/sources/{id}",
            axum::routing::delete(remove).patch(update),
        )
        .route("/admin/audio/sources/{id}/edit", get(edit_form))
        .route("/admin/audio/sources/{id}/probe", get(probe))
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
// `Starting` and `Paused` are part of the documented status palette
// (see _partial_audio_source_row.html). The synthetic `daemon_status`
// only returns `Capturing` / `Down` today; the other two will be
// emitted once the audio daemon's per-source signals are wired —
// see TODO(O-13-followup) above.
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

async fn page(State(state): State<AppState>) -> Html<String> {
    let sources = state.with_db(AudioSourceStore::list).unwrap_or_else(|err| {
        tracing::error!(error = %err, "audio_sources list failed");
        Vec::new()
    });
    let active_daemon = state.audio_source().map(ToString::to_string);
    Html(admin_shell(
        "Audio Sources",
        "audio",
        &render_body(&sources, active_daemon.as_deref()),
    ))
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
            let mut body = render_row(&row, daemon_status(&row, state.audio_source()));
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
        Ok(()) => toast::oob_only(
            Toast::success("Source removed.")
                .with_action("/admin/system/restart", "Restart to apply"),
        )
        .into_response(),
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
            let mut body = render_row(&row, daemon_status(&row, state.audio_source()));
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
    let status = daemon_status(&row, state.audio_source());
    Html(render_status_pill(&row.id, status)).into_response()
}

// ---------------------------------------------------------------------------
// Daemon-side probe (synthetic until follow-up wires it for real)
// ---------------------------------------------------------------------------

/// Maps an `audio_sources` row to a status pill. Until the audio daemon
/// exposes per-source liveness, this is synthetic: the first row whose
/// `device_id` matches the daemon's single-string config reports
/// `Capturing`; every other row reports `Down`. Rows with a quiet
/// schedule that the daemon understands would report `Paused`; today the
/// schedule is stored but not consulted, so this synthesizes
/// `Capturing` whenever the daemon is reading the row.
fn daemon_status(row: &AudioSource, daemon_source: Option<&str>) -> Status {
    if row.disabled_at.is_some() {
        return Status::Down;
    }
    if daemon_source.is_some_and(|s| s == row.device_id) {
        Status::Capturing
    } else {
        Status::Down
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn render_body(sources: &[AudioSource], daemon_source: Option<&str>) -> String {
    let (local, rtsp): (Vec<&AudioSource>, Vec<&AudioSource>) = sources
        .iter()
        .partition(|s| !matches!(s.kind, SourceKind::Rtsp));

    let mut rows_local = String::new();
    for s in &local {
        rows_local.push_str(&render_row(s, daemon_status(s, daemon_source)));
    }
    let mut rows_rtsp = String::new();
    for s in &rtsp {
        rows_rtsp.push_str(&render_row(s, daemon_status(s, daemon_source)));
    }

    let count_local = format!(
        "{} {}",
        local.len(),
        if local.len() == 1 { "mic" } else { "mics" }
    );
    let count_rtsp = format!(
        "{} {}",
        rtsp.len(),
        if rtsp.len() == 1 { "stream" } else { "streams" }
    );

    let empty_both = local.is_empty() && rtsp.is_empty();

    PAGE_TPL
        .replace("{{rows_local}}", &rows_local)
        .replace("{{rows_rtsp}}", &rows_rtsp)
        .replace("{{count_local}}", &escape_html(&count_local))
        .replace("{{count_rtsp}}", &escape_html(&count_rtsp))
        .replace(
            "{{hidden_local}}",
            if local.is_empty() && !empty_both {
                "hidden"
            } else {
                ""
            },
        )
        .replace(
            "{{hidden_rtsp}}",
            if rtsp.is_empty() && !empty_both {
                "hidden"
            } else {
                ""
            },
        )
        .replace("{{hidden_empty}}", if empty_both { "" } else { "hidden" })
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
        style="display:contents;">
    <input type="hidden" name="kind" value="{kind}">
    <div class="audio-source__kind">
      <span class="audio-kind-badge {kind_class}">
        <span class="audio-kind-glyph" aria-hidden="true"></span>
        {kind_label}
      </span>
    </div>
    <div class="audio-source__id" style="display:flex;flex-direction:column;gap:6px;">
      <input name="label" type="text" placeholder="Friendly label" value="{label}"
             style="font-size:14.5px;padding:6px 8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--bg-2);color:var(--fg);">
      <input name="device_id" class="mono" type="text" value="{device_id}" required
             style="font-size:12px;padding:6px 8px;border-radius:6px;border:0.5px solid var(--border-2);background:var(--bg-2);color:var(--fg);">
    </div>
    <div class="audio-source__right" style="display:inline-flex;gap:8px;">
      <button type="submit" class="bnb-btn moss">Save</button>
      <button type="button" class="bnb-btn ghost"
              hx-get="/admin/audio/sources/{id}/probe"
              hx-target="closest li"
              hx-swap="none">Cancel</button>
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
    fn render_body_empty_shows_empty_state() {
        let html = render_body(&[], None);
        assert!(html.contains("No audio sources yet"));
        // Both group cards are hidden, empty state is not.
        assert!(html.contains("hidden"));
    }

    #[test]
    fn render_body_partitions_by_kind() {
        let (_d, state) = fixture();
        insert_one(&state, "src_u", SourceKind::UsbAlsa, "hw:1,0");
        insert_one(&state, "src_r", SourceKind::Rtsp, "rtsp://x/y");
        let sources = state.with_db(AudioSourceStore::list).unwrap();
        let html = render_body(&sources, None);
        assert!(!html.contains("{{"));
        assert!(html.contains("hw:1,0"));
        assert!(html.contains("rtsp://x/y"));
        // Both visible — neither group is hidden.
        // Empty state IS hidden though.
        assert!(html.contains(r#"<section class="bnb-card pad" hidden>"#));
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
    fn daemon_status_reflects_active_source() {
        let s = AudioSource {
            id: "src_a".to_string(),
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
        assert!(matches!(
            daemon_status(&s, Some("hw:1,0")),
            Status::Capturing
        ));
        assert!(matches!(daemon_status(&s, Some("hw:2,0")), Status::Down));
        assert!(matches!(daemon_status(&s, None), Status::Down));

        let mut disabled = s;
        disabled.disabled_at = Some("2026-05-28".to_string());
        assert!(matches!(
            daemon_status(&disabled, Some("hw:1,0")),
            Status::Down
        ));
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
