//! "Some of this came from somewhere else."
//!
//! # Why a banner
//!
//! `birdnet-migrate` warns *before* an import that the source is 340 km away
//! and on another clock, and it deliberately does not block — merging two sites
//! is a legitimate thing to want, and only the operator can say whether two
//! coordinates are one station whose GPS fix moved or two sites a county apart.
//!
//! But once they say yes, every location- and hour-dependent analytic reads the
//! union as one station: the heat map, the dawn chorus, phenology, "first of
//! year", life-list firsts, the species-richness curves. Migration 25 recorded
//! which rows came from where, and until now nothing read it — so a chart could
//! not be judged, because nothing on it said part of it came from elsewhere.
//! For a research station that is the difference between a dataset you can cite
//! and one you cannot, and it is not detectable after the fact.
//!
//! This is the smallest thing that makes it detectable *while looking at the
//! chart*: one line, on the screens where provenance changes the reading,
//! rendering nothing at all on the overwhelming majority of stations that have
//! imported nothing or imported only their own history.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use crate::state::AppState;

use super::escape_html;

/// Mount the provenance-note partial.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/provenance-note", get(provenance_note_partial))
}

/// The markup for a station's import provenance, or an empty string.
///
/// Pure so the wording is testable without a server. Empty means "nothing to
/// say", and the caller renders nothing — a banner that says "no imported data"
/// on every station would be noise that trains people to ignore the one that
/// matters.
fn render_note(batches: &[birdnet_db::sqlite::ImportBatch], imported_rows: i64) -> String {
    if imported_rows <= 0 || batches.is_empty() {
        return String::new();
    }
    let elsewhere: Vec<&birdnet_db::sqlite::ImportBatch> =
        batches.iter().filter(|b| b.is_different_site()).collect();

    // Same-site imports are the common case — an operator moving their own
    // BirdNET-Pi history across — and saying "this is merged" about a station's
    // own past would be alarming about nothing.
    if elsewhere.is_empty() {
        return String::new();
    }

    let rows = crate::routes::pages::group_thousands(imported_rows);
    let detail = if elsewhere.len() == 1 {
        let b = elsewhere[0];
        let name = b
            .source_label
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("an unnamed source");
        let km = b.distance_km.unwrap_or_default();
        let clock = if b.applied_shift_secs == 0 {
            ", with no clock correction applied"
        } else {
            ", with a clock correction applied"
        };
        format!(
            "{} — recorded about {km:.0} km away{clock}",
            escape_html(name)
        )
    } else {
        format!("{} different sites", elsewhere.len())
    };

    format!(
        r#"<aside class="bnb-card pad prov-note" role="note">
  <span class="bnb-pill dawn"><span class="bnb-dot"></span> Merged history</span>
  <p class="prov-note__body">These charts include <b>{rows}</b> imported detections from {detail}.
     Sunrise, habitat and the species pool all differ between sites, so hour-of-day and
     location-based readings here describe more than one place.</p>
  <p class="bnb-meta"><a href="/station/data">See what was imported</a></p>
</aside>"#
    )
}

async fn provenance_note_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let html = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let batches = birdnet_db::sqlite::list_import_batches(conn).unwrap_or_default();
            let rows = birdnet_db::sqlite::imported_detection_count(conn).unwrap_or(0);
            render_note(&batches, rows)
        })
    })
    .await
    .unwrap_or_default();
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// The markup an analytic page embeds to pull the note in.
///
/// `hx-trigger="load"` and nothing else: provenance changes only when an import
/// runs, so polling it would be pure cost.
#[must_use]
pub const fn slot() -> &'static str {
    r#"<div hx-get="/pages/provenance-note" hx-trigger="load" hx-swap="innerHTML"></div>"#
}

#[cfg(test)]
mod tests {
    use super::render_note;
    use birdnet_db::sqlite::ImportBatch;

    fn batch(label: &str, km: Option<f64>, shift: i64, rows: i64) -> ImportBatch {
        ImportBatch {
            id: 1,
            imported_at: "2026-08-19 10:00:00".to_string(),
            source_kind: "birdnet-pi".to_string(),
            source_label: Some(label.to_string()),
            distance_km: km,
            applied_shift_secs: shift,
            row_count: rows,
            notes: None,
        }
    }

    /// The overwhelmingly common import is a station's own history moving
    /// across from BirdNET-Pi. Saying "merged history" about that would be a
    /// false alarm on nearly every station that ever imports, which is how a
    /// banner gets ignored by the time it matters.
    #[test]
    fn a_same_site_import_says_nothing() {
        assert_eq!(
            render_note(&[batch("My old Pi", Some(0.3), 0, 5_000)], 5_000),
            ""
        );
        // Unlocated sources cannot be shown to be elsewhere either.
        assert_eq!(render_note(&[batch("A CSV export", None, 0, 900)], 900), "");
    }

    #[test]
    fn a_station_with_no_imports_says_nothing() {
        assert_eq!(render_note(&[], 0), "");
        // Batches recorded but every imported row since deleted.
        assert_eq!(render_note(&[batch("Coastal", Some(341.0), 0, 900)], 0), "");
    }

    /// A genuinely different site is named, with the distance and whether the
    /// clocks were reconciled — the two facts that decide whether an
    /// hour-of-day chart can be read at all.
    #[test]
    fn a_different_site_is_named_with_its_distance_and_clock() {
        let html = render_note(&[batch("Coastal site", Some(341.0), -21_600, 900)], 900);
        assert!(html.contains("Coastal site"), "{html}");
        assert!(html.contains("341 km"), "{html}");
        assert!(html.contains("with a clock correction applied"), "{html}");
        assert!(html.contains("900"), "the row count must be shown: {html}");
    }

    /// An import left on the source station's clock is the dangerous one: the
    /// hours are in another timezone and nothing in the data says so.
    #[test]
    fn an_uncorrected_clock_is_called_out_as_such() {
        let html = render_note(&[batch("Coastal site", Some(341.0), 0, 900)], 900);
        assert!(html.contains("no clock correction applied"), "{html}");
    }

    /// A source label is operator-supplied text and reaches the page.
    #[test]
    fn the_source_label_is_escaped() {
        let html = render_note(
            &[batch("<script>alert(1)</script>", Some(341.0), 0, 900)],
            900,
        );
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    /// The link has to go somewhere real. `/station/data` is the tab that
    /// hosts the importer, so it is where "see what was imported" belongs.
    #[test]
    fn the_note_links_to_the_data_tab() {
        let html = render_note(&[batch("Coastal site", Some(341.0), 0, 900)], 900);
        assert!(html.contains(r#"href="/station/data""#), "{html}");
    }

    #[test]
    fn several_different_sites_are_counted_rather_than_listed() {
        let mut a = batch("North", Some(120.0), 0, 100);
        a.id = 1;
        let mut b = batch("South", Some(300.0), 0, 200);
        b.id = 2;
        let html = render_note(&[a, b], 300);
        assert!(html.contains("2 different sites"), "{html}");
    }
}
