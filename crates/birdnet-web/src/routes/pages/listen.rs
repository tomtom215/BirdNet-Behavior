//! Listen-now page (the maintainer's standing request).
//!
//! Mounts:
//!   GET /listen   — full page (audio playback + spectrogram + trickle)
//!
//! Composes three already-wired surfaces:
//!
//! * **Audio** — `<audio src="/stream?source_id=…">`, where the page's
//!   source selector populates the query. `/stream` resolves the id via
//!   the `audio_sources` table (see [`crate::routes::livestream`])
//!   so listening to a per-source mic just works without restarting
//!   the daemon.
//! * **Spectrogram** — the same `/api/v2/ws/spectrogram` consumer
//!   shipped on the dashboard (#98); the in-page script is a narrowed
//!   copy. Producer side is the global capture-pipeline watcher, so the
//!   canvas shows whichever source is feeding the watch dir — not
//!   strictly the listen-now selection. A per-source spectrogram
//!   producer is the natural follow-up once the capture pipeline
//!   iterates `audio_sources` rows (O-13).
//! * **Trickle** — `/pages/detections` (the dashboard live feed
//!   handler) polled every 10 s. Empty DB → `empty_states::quiet_yard()`
//!   via the existing partial.
//!
//! Source-selector population: lists every non-disabled row from
//! `audio_sources` plus a `(default)` entry that maps to `/stream` with no
//! `source_id` (resolving to the first enabled row). On a station with no
//! `audio_sources` rows, the selector shows a disabled "no audio sources
//! configured" placeholder — the legacy single-string `state.audio_source()`
//! fallback was retired in O-13.

use axum::Router;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Html;
use axum::routing::get;

use birdnet_db::audio_sources::{AudioSource, AudioSourceStore, SourceKind};

use super::{escape_html, render_page_for_request};
use crate::state::AppState;

const PAGE_HTML: &str = include_str!("../../../templates/listen.html");

pub fn router() -> Router<AppState> {
    Router::new().route("/listen", get(page))
}

/// `?source=` deep link (the Today signal card's picker lands here with a
/// source already chosen).
#[derive(serde::Deserialize)]
struct ListenParams {
    source: Option<String>,
}

async fn page(
    State(state): State<AppState>,
    Query(params): Query<ListenParams>,
    headers: HeaderMap,
) -> Html<String> {
    let sources = state
        .with_db(|conn| AudioSourceStore::list(conn).ok().unwrap_or_default())
        .into_iter()
        .filter(|s| s.disabled_at.is_none())
        .collect::<Vec<_>>();

    // The "— default audio source —" option maps to /stream with no
    // source_id, which resolves to the first enabled `audio_sources` row.
    let options = render_options(&sources, params.source.as_deref());

    // Trickle skeleton — reuse the feed_rows shape used on the dashboard.
    let trickle_skel = super::skeletons::feed_rows(6);

    let body = PAGE_HTML
        .replace("{{source_options}}", &options)
        .replace("{{skel_trickle}}", &trickle_skel);

    // "listen" highlights the Live-audio entry in the More menu / mobile sheet
    // (the {{nav_listen}} slot); it is not a top-level tab.
    render_page_for_request("Listen now", &body, "listen", &headers)
}

/// The source-selector `<option>` set for any page offering per-source live
/// audio (this page and the Today home's signal card). Filters disabled rows
/// itself so callers can hand over the raw store listing.
pub(super) fn source_options(sources: &[AudioSource]) -> String {
    let enabled: Vec<AudioSource> = sources
        .iter()
        .filter(|s| s.disabled_at.is_none())
        .cloned()
        .collect();
    render_options(&enabled, None)
}

/// Render the `<option>` set for the source selector. With at least one
/// configured `audio_sources` row, the first option is the default-source
/// path (empty value → first enabled row) and the rows follow, labelled by
/// `label` (or `device_id` when no label). With no rows, a single disabled
/// "no audio sources configured" placeholder is rendered instead.
fn render_options(sources: &[AudioSource], selected: Option<&str>) -> String {
    let mut out = String::new();
    if sources.is_empty() {
        // No rows → /stream has no source to resolve (503). Show a disabled
        // placeholder rather than a "default" option that can't play.
        out.push_str(
            r#"<option value="" disabled selected>— no audio sources configured —</option>"#,
        );
        return out;
    }
    // Empty value maps to "no source_id" → the first enabled row in /stream.
    out.push_str(r#"<option value="">— default audio source —</option>"#);
    for s in sources {
        let label_display = s.label.clone().unwrap_or_else(|| s.device_id.clone());
        let kind_glyph = match s.kind {
            SourceKind::UsbAlsa => "🎙",
            SourceKind::PipeWire => "🔊",
            SourceKind::Rtsp => "📡",
        };
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!(
                r#"<option value="{id}"{sel}>{glyph} {label} ({kind})</option>"#,
                id = escape_html(&s.id),
                sel = if selected == Some(s.id.as_str()) {
                    " selected"
                } else {
                    ""
                },
                glyph = kind_glyph,
                label = escape_html(&label_display),
                kind = s.kind.as_str(),
            ),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_source_options(sources: &[AudioSource]) -> String {
        render_options(sources, None)
    }
    use birdnet_db::audio_sources::{Channels, PipelineFlags, RtspTransport};

    fn sample(id: &str, kind: SourceKind, label: Option<&str>, disabled: bool) -> AudioSource {
        AudioSource {
            id: id.to_string(),
            kind,
            device_id: format!("dev:{id}"),
            label: label.map(str::to_string),
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 16,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
            disabled_at: if disabled {
                Some("2026-01-01".to_string())
            } else {
                None
            },
            created_at: "2026-05-01".to_string(),
            updated_at: "2026-05-01".to_string(),
        }
    }

    #[test]
    fn render_source_options_empty_yields_no_sources_notice() {
        let html = render_source_options(&[]);
        // No configured rows → a disabled placeholder, not a playable default.
        assert!(html.contains("no audio sources configured"));
        assert!(html.contains("disabled"));
        assert!(!html.contains("— default audio source —"));
    }

    #[test]
    fn render_source_options_lists_each_active_row() {
        let sources = vec![
            sample("src_usb_1", SourceKind::UsbAlsa, Some("Garden mic"), false),
            sample("src_rtsp_1", SourceKind::Rtsp, Some("Front camera"), false),
            sample("src_pw_1", SourceKind::PipeWire, None, false),
        ];
        let html = render_source_options(&sources);
        assert!(html.contains(r#"value="src_usb_1""#));
        assert!(html.contains("Garden mic"));
        assert!(html.contains(r#"value="src_rtsp_1""#));
        assert!(html.contains("Front camera"));
        // PipeWire row falls back to its device_id when no label.
        assert!(html.contains(r#"value="src_pw_1""#));
        assert!(html.contains("dev:src_pw_1"));
        // Kind suffix renders for each row.
        assert!(html.contains("(usb-alsa)"));
        assert!(html.contains("(rtsp)"));
        assert!(html.contains("(pipewire)"));
    }

    #[test]
    fn render_source_options_html_escapes_labels() {
        // A label like `Front <camera>` could break out of the option text.
        let sources = vec![sample(
            "src_x",
            SourceKind::Rtsp,
            Some("Front <cam>"),
            false,
        )];
        let html = render_source_options(&sources);
        assert!(html.contains("Front &lt;cam&gt;"));
        assert!(!html.contains("<cam>"));
    }
}
