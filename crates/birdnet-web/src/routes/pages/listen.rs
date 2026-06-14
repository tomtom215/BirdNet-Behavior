//! Live-audio source picker `<option>` rendering.
//!
//! The standalone `/listen` page folded into the Recordings home's **Live**
//! view in the v3 spine (`/recordings?view=live`); `/listen`, `/livestream`
//! and `/live` now permanently redirect there. What survives here is the bit
//! two surfaces still share — the source-selector `<option>` set — so the
//! Today signal card ([`super::today`]) and the Recordings Live view
//! ([`super::recordings`]) render an identical picker from one place.
//!
//! Population: every non-disabled row from `audio_sources`, preceded by a
//! `— default audio source —` entry that maps to `/stream` with no
//! `source_id` (resolving to the first enabled row). A station with no
//! `audio_sources` rows gets a single disabled "no audio sources configured"
//! placeholder instead.

use birdnet_db::audio_sources::{AudioSource, SourceKind};

use super::escape_html;

/// The source-selector `<option>` set, with no row pre-selected. Filters
/// disabled rows itself so callers can hand over the raw store listing.
pub(super) fn source_options(sources: &[AudioSource]) -> String {
    source_options_for(sources, None)
}

/// The source-selector `<option>` set with `selected` (an `audio_sources.id`)
/// pre-selected when present — the Recordings Live `?source=` deep link lands
/// here with a source already chosen.
pub(super) fn source_options_for(sources: &[AudioSource], selected: Option<&str>) -> String {
    let enabled: Vec<AudioSource> = sources
        .iter()
        .filter(|s| s.disabled_at.is_none())
        .cloned()
        .collect();
    render_options(&enabled, selected)
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
    fn source_options_for_marks_the_selected_row() {
        let sources = vec![
            sample("src_a", SourceKind::UsbAlsa, Some("A"), false),
            sample("src_b", SourceKind::Rtsp, Some("B"), false),
        ];
        let html = source_options_for(&sources, Some("src_b"));
        assert!(html.contains(r#"value="src_b" selected"#));
        assert!(!html.contains(r#"value="src_a" selected"#));
    }

    #[test]
    fn source_options_for_filters_disabled_rows() {
        let sources = vec![
            sample("src_on", SourceKind::UsbAlsa, Some("On"), false),
            sample("src_off", SourceKind::Rtsp, Some("Off"), true),
        ];
        let html = source_options_for(&sources, None);
        assert!(html.contains(r#"value="src_on""#));
        assert!(!html.contains(r#"value="src_off""#));
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
