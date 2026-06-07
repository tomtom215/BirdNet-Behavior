//! Bespoke analytics visualizations rendered as inline SVG.
//!
//! Pure functions, styled by the design tokens in `static/css/app.css`, grouped
//! by visual family:
//!
//! - [`matrix`] — the co-occurrence intensity grid.
//! - [`timeline`] — Cartesian time-series charts (streamgraph, accumulation
//!   curve, migration ridgeline, the 24-hour day strip).
//! - [`radial`] — polar charts (the dawn-chorus circadian plot, the acoustic
//!   co-occurrence chord diagram) and the geometry helpers they share.
//!
//! Colours come from `super::atoms::species_color` so a species keeps the same
//! hue across every screen.

pub(crate) mod matrix;
pub(crate) mod radial;
pub(crate) mod timeline;

pub(crate) use matrix::cooccurrence_matrix;
pub(crate) use radial::{chord_diagram, circadian_polar};
pub(crate) use timeline::{accumulation_curve, day_strip, ridgeline, streamgraph};

/// Shared "not enough data yet" placeholder returned by every chart when its
/// input is too small to plot.
pub(super) const EMPTY: &str =
    r#"<p class="bnb-meta viz-empty">Not enough data yet for this view.</p>"#;
