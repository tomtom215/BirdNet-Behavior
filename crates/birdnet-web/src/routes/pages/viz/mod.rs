//! Bespoke analytics visualizations rendered as inline SVG.
//!
//! Pure functions, styled by the design tokens in `static/css/app.css`, grouped
//! by visual family:
//!
//! - `matrix` — the co-occurrence intensity grid.
//! - `timeline` — Cartesian time-series charts (streamgraph, accumulation
//!   curve, migration ridgeline, the 24-hour day strip).
//! - `radial` — polar charts (the dawn-chorus circadian plot, the acoustic
//!   co-occurrence chord diagram) and the geometry helpers they share.
//!
//! Colours come from `super::atoms::species_color` so a species keeps the same
//! hue across every screen.

// The funnel is the one viz used only by the analytics-gated dawn card, so it
// is gated to match — keeping the slim `--no-default-features` build free of an
// unused renderer.
#[cfg(feature = "analytics")]
pub(crate) mod funnel;
pub(crate) mod matrix;
pub(crate) mod radial;
pub(crate) mod timeline;

#[cfg(feature = "analytics")]
pub(crate) use funnel::sequence_funnel;
pub(crate) use matrix::cooccurrence_matrix;
pub(crate) use radial::{chord_diagram, circadian_polar};
pub(crate) use timeline::{accumulation_curve, day_strip, ridgeline, streamgraph};

use crate::routes::pages::escape_html;

/// Shared "not enough data yet" placeholder returned by every chart when its
/// input is too small to plot.
pub(super) const EMPTY: &str =
    r#"<p class="bnb-meta viz-empty">Not enough data yet for this view.</p>"#;

/// Accessible `<title>` + `<desc>` for a chart's root `<svg role="img">`.
///
/// `name` is the chart's accessible name (and a native hover tooltip); `desc`
/// is a one-sentence, jargon-free description of what the chart encodes. Emit
/// the returned markup as the **first** children of the `<svg>` so assistive
/// technology announces the name and description instead of a bare image, and
/// drop the `aria-label` it replaces (the `<title>` is the accessible name).
pub(super) fn svg_a11y(name: &str, desc: &str) -> String {
    format!(
        "<title>{}</title><desc>{}</desc>",
        escape_html(name),
        escape_html(desc),
    )
}
