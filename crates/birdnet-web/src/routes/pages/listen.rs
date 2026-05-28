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
//! `audio_sources` plus a `(default)` entry that maps to the legacy
//! single-string `state.audio_source()` path. On a fresh station with
//! no `audio_sources` rows but a configured `state.audio_source()`, the
//! selector still works via the default option — `/stream` itself now
//! resolves the default via the first enabled `audio_sources` row,
//! reading `state.audio_source()` only as a final fallback.

use axum::Router;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;

use birdnet_db::audio_sources::{AudioSource, AudioSourceStore, SourceKind};

use super::{escape_html, render_page};
use crate::state::AppState;

const PAGE_HTML: &str = include_str!("../../../templates/listen.html");

pub fn router() -> Router<AppState> {
    Router::new().route("/listen", get(page))
}

async fn page(State(state): State<AppState>) -> Html<String> {
    let sources = state
        .with_db(|conn| AudioSourceStore::list(conn).ok().unwrap_or_default())
        .into_iter()
        .filter(|s| s.disabled_at.is_none())
        .collect::<Vec<_>>();

    let has_default = state.audio_source().is_some();
    let options = render_source_options(&sources, has_default);

    // Trickle skeleton — reuse the feed_rows shape used on the dashboard.
    let trickle_skel = super::skeletons::feed_rows(6);

    let body = PAGE_HTML
        .replace("{{source_options}}", &options)
        .replace("{{skel_trickle}}", &trickle_skel);

    // active_nav = "today" so the existing topnav highlight lands on
    // the closest concept — "listen" isn't a separate top-level nav
    // entry, the link sits inside the audio-sources admin row.
    render_page("Listen now", &body, "today")
}

/// Render the `<option>` set for the source selector. The first option
/// is the default-source path; configured `audio_sources` rows follow,
/// labelled by `label` (or `device_id` when no label is set).
fn render_source_options(sources: &[AudioSource], has_default: bool) -> String {
    let mut out = String::new();
    if has_default || sources.is_empty() {
        // Use an empty value to map to "no source_id query" → the
        // legacy `state.audio_source()` path in /stream.
        out.push_str(r#"<option value="">— default audio source —</option>"#);
    }
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
                r#"<option value="{id}">{glyph} {label} ({kind})</option>"#,
                id = escape_html(&s.id),
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
    fn render_source_options_empty_yields_default_only() {
        let html = render_source_options(&[], false);
        // No configured rows AND no `state.audio_source()` → still
        // render the default option so the selector isn't empty (the
        // play button will fail gracefully in that case).
        assert!(html.contains(r#"value="""#));
        assert!(html.contains("default audio source"));
    }

    #[test]
    fn render_source_options_lists_each_active_row() {
        let sources = vec![
            sample("src_usb_1", SourceKind::UsbAlsa, Some("Garden mic"), false),
            sample("src_rtsp_1", SourceKind::Rtsp, Some("Front camera"), false),
            sample("src_pw_1", SourceKind::PipeWire, None, false),
        ];
        let html = render_source_options(&sources, true);
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
        let html = render_source_options(&sources, false);
        assert!(html.contains("Front &lt;cam&gt;"));
        assert!(!html.contains("<cam>"));
    }
}
