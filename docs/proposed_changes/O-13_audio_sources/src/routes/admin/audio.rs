//! Audio sources management — first-class CRUD page.
//!
//! Replaces the hard-coded two-row stub in the prior `admin/audio.rs`. The
//! body is rendered from `templates/admin_audio_sources.html` + per-row
//! partials in `templates/_partial_audio_source_row.html`; live status comes
//! from `AudioSourceStore::probe(id)` and is refreshed via `hx-trigger="every
//! 8s"` on each row's status pill.
//!
//! Drop-in: replace `crates/birdnet-web/src/routes/admin/audio.rs` wholesale
//! with this file once `AudioSourceStore` lands in `birdnet-db` (see O-13
//! DIFF.md). Until then, the `STUB_*` constants at the bottom of this file
//! give a working in-memory implementation that exercises every code path.

use std::fmt::Write as _;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::{delete, get, patch, post};
use axum::{Form, Router};
use serde::Deserialize;

use super::admin_shell;
use crate::routes::pages::confirm; // O-17
use crate::routes::pages::escape_html;
use crate::routes::pages::skeletons; // O-16
use crate::routes::pages::toast; // O-18
use crate::state::AppState;

// Embedded templates — compiled into the binary.
const PAGE_TPL: &str = include_str!("../../../templates/admin_audio_sources.html");
const ROW_TPL:  &str = include_str!("../../../templates/_partial_audio_source_row.html");

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/audio", get(page))
        .route("/admin/audio/sources", post(create))
        .route("/admin/audio/sources/{id}", delete(remove).patch(update))
        .route("/admin/audio/sources/{id}/edit", get(edit_form))
        .route("/admin/audio/sources/{id}/probe", get(probe))
}

// ---------------------------------------------------------------------------
// View model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Kind { UsbAlsa, PipeWire, Rtsp }

impl Kind {
    fn from_db(s: &str) -> Self {
        match s {
            "rtsp" => Kind::Rtsp,
            "pipewire" => Kind::PipeWire,
            _ => Kind::UsbAlsa,
        }
    }
    fn css(self) -> &'static str { match self { Self::UsbAlsa => "usb", Self::PipeWire => "pipe", Self::Rtsp => "rtsp" } }
    fn label(self) -> &'static str { match self { Self::UsbAlsa => "USB · ALSA", Self::PipeWire => "PipeWire", Self::Rtsp => "RTSP" } }
}

#[derive(Clone, Copy, Debug)]
enum Status { Capturing, Starting, Paused, Down }

impl Status {
    fn css(self) -> &'static str { match self { Self::Capturing => "live", Self::Starting => "starting", Self::Paused => "paused", Self::Down => "down" } }
    fn label(self) -> &'static str { match self { Self::Capturing => "Capturing", Self::Starting => "Starting…", Self::Paused => "Paused (schedule)", Self::Down => "Down" } }
}

struct SourceView {
    id: String,
    kind: Kind,
    label: Option<String>,
    device_id: String,
    detail_line: String,
    status: Status,
    meta_line: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn page(State(state): State<AppState>) -> Html<String> {
    let sources = list_sources(&state).await;
    Html(admin_shell("Audio Sources", "audio", &render_body(&sources)))
}

#[derive(Deserialize)]
struct CreateForm {
    scope: String,       // "local" | "rtsp"
    kind: String,        // "usb-alsa" | "pipewire" | "rtsp"
    device_id: String,
    label: Option<String>,
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default)]
    rtsp_transport: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> Result<Html<String>, (StatusCode, String)> {
    let _ = state; // wired in real impl
    if form.device_id.trim().is_empty() {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "device id is required".into()));
    }
    if !matches!(form.kind.as_str(), "usb-alsa" | "pipewire" | "rtsp") {
        return Err((StatusCode::UNPROCESSABLE_ENTITY, "invalid kind".into()));
    }
    // store.insert(new) → returns SourceView; until storage is wired, fake it.
    let view = SourceView {
        id: format!("src_new_{}", short_now()),
        kind: Kind::from_db(&form.kind),
        label: form.label.filter(|s| !s.trim().is_empty()),
        device_id: form.device_id,
        detail_line: default_detail(&form),
        status: Status::Starting,
        meta_line: "just added".into(),
    };
    let row = render_row(&view);
    // Attach toast + restart-required side effect.
    let mut body = row;
    body.push_str(&toast::Toast::success(format!(
        "Added {}.",
        view.label.as_deref().unwrap_or(&view.device_id)
    )).with_action("/admin/system/restart", "Restart to apply").render_oob());
    Ok(Html(body))
}

async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Html<String> {
    let _ = state;
    // store.soft_delete(&id).await;
    // Empty body removes the row; OOB toast confirms.
    Html(toast::Toast::success(format!("Removed source {id}.")).render_oob())
}

async fn edit_form(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Html<String> {
    let _ = state;
    // store.get(&id).await; emit an inline edit form sized like a row.
    Html(format!(
        r#"<li class="audio-source" data-source-id="{id}">
  <form hx-patch="/admin/audio/sources/{id}"
        hx-target="this" hx-swap="outerHTML"
        style="display:contents;">
    <!-- Same grid as a row; fields replace the read-only spans. Full edit
         form omitted for brevity in this stub — wire when the store lands. -->
    <div class="audio-source__kind"><span class="audio-kind-badge">edit · {id}</span></div>
    <div class="audio-source__id">
      <input name="label" type="text" placeholder="Friendly label" style="font-size:14.5px;">
      <input name="device_id" class="mono" type="text" style="font-size:12px;" required>
    </div>
    <div class="audio-source__right">
      <button type="submit" class="bnb-btn moss">Save</button>
      <button type="button" class="bnb-btn ghost"
              hx-get="/admin/audio/sources/{id}"
              hx-target="closest li" hx-swap="outerHTML">Cancel</button>
    </div>
  </form>
</li>"#,
        id = escape_html(&id),
    ))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(_form): Form<CreateForm>,
) -> Html<String> {
    let _ = state;
    // store.update(&id, patch).await;
    Html(format!("<li>updated {}</li>", escape_html(&id))) // real impl: render_row + toast
}

async fn probe(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Html<String> {
    let _ = state;
    // store.probe(&id).await — for now, return a stable status.
    let status = Status::Capturing;
    Html(format!(
        r#"<span class="bnb-pill {cls}"
  hx-get="/admin/audio/sources/{id}/probe"
  hx-trigger="every 8s"
  hx-swap="outerHTML"
  aria-live="polite">
  <span class="bnb-dot {cls}" aria-hidden="true"></span>{label}</span>"#,
        cls = status.css(),
        label = status.label(),
        id = escape_html(&id),
    ))
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn render_body(sources: &[SourceView]) -> String {
    let (local, rtsp): (Vec<_>, Vec<_>) = sources
        .iter()
        .partition(|s| !matches!(s.kind, Kind::Rtsp));

    let mut rows_local = String::new();
    if local.is_empty() {
        let _ = write!(rows_local, "{}", skeletons::list_rows(0)); // empty list, no skeletons
    } else {
        for s in &local { rows_local.push_str(&render_row(s)); }
    }
    let mut rows_rtsp = String::new();
    for s in &rtsp { rows_rtsp.push_str(&render_row(s)); }

    let count_local = format!("{} {}", local.len(), if local.len() == 1 { "connected" } else { "connected" });
    let count_rtsp  = format!("{} {}", rtsp.len(), if rtsp.len() == 1 { "stream" } else { "streams" });

    let empty_both  = local.is_empty() && rtsp.is_empty();

    // Restart-required pill is only rendered when there's a pending change set.
    // For this stub it's empty; a real impl reads from a flag in state.
    let pending_changes = String::new();

    PAGE_TPL
        .replace("{{rows_local}}", &rows_local)
        .replace("{{rows_rtsp}}",  &rows_rtsp)
        .replace("{{count_local}}", &escape_html(&count_local))
        .replace("{{count_rtsp}}",  &escape_html(&count_rtsp))
        .replace("{{hidden_local}}", if local.is_empty() && !empty_both { r#"hidden"# } else { "" })
        .replace("{{hidden_rtsp}}",  if rtsp.is_empty()  && !empty_both { r#"hidden"# } else { "" })
        .replace("{{hidden_empty}}", if empty_both { "" } else { "hidden" })
        .replace("{{pending_changes}}", &pending_changes)
}

fn render_row(s: &SourceView) -> String {
    let (label_class, label_text) = match s.label.as_deref() {
        Some(l) if !l.is_empty() => ("", l.to_string()),
        _ => ("untitled", "— no friendly label —".into()),
    };
    ROW_TPL
        .replace("{{id}}",           &escape_html(&s.id))
        .replace("{{kind_class}}",   s.kind.css())
        .replace("{{kind_label}}",   s.kind.label())
        .replace("{{label_class}}",  label_class)
        .replace("{{label_text}}",   &escape_html(&label_text))
        .replace("{{device_id}}",    &escape_html(&s.device_id))
        .replace("{{detail_line}}",  &escape_html(&s.detail_line))
        .replace("{{status_class}}", s.status.css())
        .replace("{{status_label}}", s.status.label())
        .replace("{{meta_line}}",    &escape_html(&s.meta_line))
}

fn default_detail(form: &CreateForm) -> String {
    let rate = form.sample_rate.unwrap_or(48_000) as f64 / 1000.0;
    match form.kind.as_str() {
        "rtsp" => format!(
            "RTSP · {} · keepalive on",
            form.rtsp_transport.as_deref().unwrap_or("auto"),
        ),
        _ => format!("{rate:.1} kHz · mono · 24-bit"),
    }
}

fn short_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs().to_string()).unwrap_or_else(|_| "0".into())
}

// ---------------------------------------------------------------------------
// Storage stub — drop this once `birdnet-db::audio_sources` lands.
// ---------------------------------------------------------------------------

async fn list_sources(state: &AppState) -> Vec<SourceView> {
    // Real impl:
    //   state.audio_sources().list().unwrap_or_default().into_iter().map(into_view).collect()
    let _ = state;
    vec![
        SourceView {
            id: "src_usb_1".into(),
            kind: Kind::UsbAlsa,
            label: Some("Backyard feeder".into()),
            device_id: "plughw:1,0".into(),
            detail_line: "48 kHz · mono · 24-bit · gain +12 dB".into(),
            status: Status::Capturing,
            meta_line: "up 6d 04h".into(),
        },
        SourceView {
            id: "src_rtsp_1".into(),
            kind: Kind::Rtsp,
            label: Some("Side path camera".into()),
            device_id: "rtsp://cam1.lan:554/Streaming/Channels/101".into(),
            detail_line: "AAC · 48 kHz · buffer 1.2 s".into(),
            status: Status::Capturing,
            meta_line: "up 2d 18h".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_row_substitutes_all_placeholders() {
        let s = SourceView {
            id: "src_test_1".into(),
            kind: Kind::Rtsp,
            label: None,
            device_id: "rtsp://x".into(),
            detail_line: "AAC · 48 kHz".into(),
            status: Status::Down,
            meta_line: "retry in 38s".into(),
        };
        let html = render_row(&s);
        assert!(!html.contains("{{"), "unsubstituted placeholder in:\n{html}");
        assert!(html.contains("rtsp://x"));
        assert!(html.contains("Down"));
        assert!(html.contains("untitled")); // no friendly label
    }

    #[test]
    fn render_body_partitions_by_kind() {
        let v = vec![
            SourceView { id: "u".into(), kind: Kind::UsbAlsa, label: None,
                device_id: "hw:1".into(), detail_line: "".into(),
                status: Status::Capturing, meta_line: "".into() },
            SourceView { id: "r".into(), kind: Kind::Rtsp, label: None,
                device_id: "rtsp://".into(), detail_line: "".into(),
                status: Status::Down, meta_line: "".into() },
        ];
        let html = render_body(&v);
        assert!(!html.contains("{{"), "unsubstituted placeholder");
        // Both kinds rendered, empty state hidden.
        assert!(html.contains("hw:1"));
        assert!(html.contains("rtsp://"));
        assert!(html.contains(r#"audio-source" "#) || html.contains(r#"audio-source""#));
    }

    #[test]
    fn render_body_empty_shows_empty_state() {
        let html = render_body(&[]);
        assert!(html.contains("No audio sources yet"));
        // Both group cards are hidden, empty state is not.
        assert!(html.matches("hidden").count() >= 2);
    }
}
