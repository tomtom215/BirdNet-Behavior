//! The equaliser's response curve, drawn from the same coefficients that
//! filter the audio.
//!
//! Not a sketch of what the filter is meant to do: every point is
//! [`EqChain::magnitude_db_at`], which evaluates `|H(e^{jω})|` from the
//! designed biquads. A curve computed any other way would eventually disagree
//! with the audio, and a picture that lies is worse than no picture — an
//! operator would trust it over their ears.
//!
//! Geometry-heavy SVG generation; the lint allows cover the benign coordinate
//! arithmetic, matching `routes::pages::viz`.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use birdnet_core::audio::eq::EqChain;

use crate::routes::pages::escape_html;

/// Lowest frequency on the axis. Below this is inaudible and, for a bird
/// station, is exactly the rumble the chain is usually there to remove — the
/// interesting part of a high-pass response is what happens *above* its corner.
const MIN_HZ: f64 = 20.0;

/// Highest frequency drawn, whatever the sample rate allows. `BirdNET`'s own
/// analysis band stops at 15 kHz, so there is nothing to see above this.
const MAX_HZ: f64 = 20_000.0;

/// Vertical extent, in decibels. Wide enough for a +12 dB bell and a −24 dB
/// notch shoulder without the curve leaving the box on a typical chain.
const DB_TOP: f64 = 18.0;
/// Bottom of the decibel axis. Anything past this is drawn clamped, with the
/// clipping visible rather than the line silently vanishing.
const DB_BOTTOM: f64 = -36.0;

/// Points sampled across the axis. Enough that a Q = 20 notch at 50 Hz — the
/// narrowest thing an operator is likely to write — still shows its null.
const SAMPLES: usize = 320;

const W: f64 = 560.0;
const H: f64 = 180.0;
const PAD_L: f64 = 34.0;
const PAD_R: f64 = 8.0;
const PAD_T: f64 = 8.0;
const PAD_B: f64 = 20.0;

/// Map a frequency to an x coordinate on a log axis.
fn x_of(hz: f64, top_hz: f64) -> f64 {
    let t = (hz.log10() - MIN_HZ.log10()) / (top_hz.log10() - MIN_HZ.log10());
    PAD_L + t.clamp(0.0, 1.0) * (W - PAD_L - PAD_R)
}

/// Map decibels to a y coordinate, clamped into the box.
fn y_of(db: f64) -> f64 {
    let t = (DB_TOP - db) / (DB_TOP - DB_BOTTOM);
    PAD_T + t.clamp(0.0, 1.0) * (H - PAD_T - PAD_B)
}

/// Render the chain's magnitude response as an inline SVG.
///
/// `sample_rate` sets the right-hand end of the axis: a 16 kHz source has an
/// 8 kHz Nyquist and nothing to show beyond it, and drawing to 20 kHz anyway
/// would imply a band the source does not have.
///
/// An empty chain draws the flat line rather than nothing, so clearing the
/// field visibly means "no filtering" instead of looking like a broken panel.
#[must_use]
pub fn render(chain: &EqChain, sample_rate: u32) -> String {
    let nyquist = f64::from(sample_rate) / 2.0;
    let top_hz = MAX_HZ.min(nyquist * 0.98).max(MIN_HZ * 10.0);

    let mut points = String::new();
    let mut peak_db = f64::NEG_INFINITY;
    let mut trough_db = f64::INFINITY;
    for i in 0..SAMPLES {
        let t = i as f64 / (SAMPLES - 1) as f64;
        let hz = 10.0_f64.powf(MIN_HZ.log10() + t * (top_hz.log10() - MIN_HZ.log10()));
        let db = chain.magnitude_db_at(hz as f32, sample_rate);
        if db.is_finite() {
            peak_db = peak_db.max(db);
            trough_db = trough_db.min(db);
        }
        let _ = write!(points, "{:.1},{:.1} ", x_of(hz, top_hz), y_of(db));
    }

    // Decade gridlines, plus the two corners an operator most often writes.
    let mut grid = String::new();
    for hz in [
        20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0,
    ] {
        if hz > top_hz {
            continue;
        }
        let x = x_of(hz, top_hz);
        let _ = write!(
            grid,
            r#"<line x1="{x:.1}" y1="{PAD_T}" x2="{x:.1}" y2="{:.1}" class="eqc-grid"/>"#,
            H - PAD_B
        );
        let label = if hz >= 1000.0 {
            format!("{:.0}k", hz / 1000.0)
        } else {
            format!("{hz:.0}")
        };
        let _ = write!(
            grid,
            r#"<text x="{x:.1}" y="{:.1}" class="eqc-xlabel">{label}</text>"#,
            H - PAD_B + 13.0
        );
    }
    for db in [12.0, 0.0, -12.0, -24.0] {
        let y = y_of(db);
        let cls = if db == 0.0 { "eqc-zero" } else { "eqc-grid" };
        let _ = write!(
            grid,
            r#"<line x1="{PAD_L}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" class="{cls}"/>"#,
            W - PAD_R
        );
        let _ = write!(
            grid,
            r#"<text x="{:.1}" y="{:.1}" class="eqc-ylabel">{db:+.0}</text>"#,
            PAD_L - 5.0,
            y + 3.0
        );
    }

    let summary = escape_html(&describe(chain, peak_db, trough_db, top_hz));
    format!(
        r#"<svg class="eq-curve" viewBox="0 0 {W:.0} {H:.0}" role="img" aria-label="{summary}">
  <title>{summary}</title>
  {grid}
  <polyline class="eqc-line" points="{points}"/>
</svg>"#
    )
}

/// One sentence describing the curve, for the `aria-label` and the panel.
///
/// Present because the SVG is the only rendering of this information and a
/// screen reader gets nothing from a polyline. The numbers are read off the
/// same samples the line is drawn from, so the two cannot disagree.
#[must_use]
pub fn describe(chain: &EqChain, peak_db: f64, trough_db: f64, top_hz: f64) -> String {
    if chain.is_empty() {
        return "No filtering: the source is passed through unchanged.".to_string();
    }
    let n = chain.stages().len();
    let plural = if n == 1 { "stage" } else { "stages" };
    format!(
        "{n} {plural}; response between {trough_db:.1} and {peak_db:+.1} dB \
         across 20 Hz to {:.0} kHz.",
        top_hz / 1000.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    /// Pull the polyline's points back out and turn them into (Hz, dB) pairs,
    /// inverting the two mappings. The point of the test suite here is that
    /// the *drawn* curve matches the filter, not that some intermediate
    /// number did — so everything is read off the rendered SVG.
    fn drawn_points(svg: &str, top_hz: f64) -> Vec<(f64, f64)> {
        let start = svg.find(r#"points=""#).expect("polyline present") + 8;
        let rest = &svg[start..];
        let end = rest.find('"').expect("points terminated");
        rest[..end]
            .split_whitespace()
            .map(|pair| {
                let (x, y) = pair.split_once(',').expect("x,y");
                let x: f64 = x.parse().expect("x is a number");
                let y: f64 = y.parse().expect("y is a number");
                let t = (x - PAD_L) / (W - PAD_L - PAD_R);
                let hz = 10.0_f64.powf(MIN_HZ.log10() + t * (top_hz.log10() - MIN_HZ.log10()));
                let u = (y - PAD_T) / (H - PAD_T - PAD_B);
                (hz, DB_TOP - u * (DB_TOP - DB_BOTTOM))
            })
            .collect()
    }

    fn db_near(points: &[(f64, f64)], hz: f64) -> f64 {
        points
            .iter()
            .min_by(|a, b| {
                (a.0 - hz)
                    .abs()
                    .partial_cmp(&(b.0 - hz).abs())
                    .expect("finite")
            })
            .expect("points")
            .1
    }

    /// The curve has to be the filter, not a drawing of one. Every assertion
    /// here is read back out of the rendered SVG and compared against
    /// `magnitude_db_at`, which is what the audio path uses.
    #[test]
    fn the_drawn_curve_is_the_filters_own_response() {
        let chain = EqChain::parse("highpass:120; peaking:4000:1:6").expect("parses");
        let svg = render(&chain, SR);
        let points = drawn_points(&svg, MAX_HZ.min(f64::from(SR) / 2.0 * 0.98));
        for hz in [30.0_f64, 120.0, 500.0, 4000.0, 12_000.0] {
            let drawn = db_near(&points, hz);
            let want = chain.magnitude_db_at(hz as f32, SR);
            // The nearest sampled point is not exactly `hz`, so allow the
            // curve's own slope between samples; 1.5 dB covers the steepest
            // stage this chain has at the sample spacing used.
            assert!(
                (drawn - want).abs() < 1.5,
                "at {hz} Hz the curve reads {drawn:.2} dB, the filter says {want:.2} dB"
            );
        }
    }

    /// A boost really goes up and a cut really goes down — the sign of the
    /// y-axis is the one thing a reader takes from the picture at a glance,
    /// and SVG's y grows downwards, so it is the easiest thing to invert.
    #[test]
    fn a_boost_is_drawn_above_the_zero_line_and_a_cut_below() {
        let boost = EqChain::parse("peaking:2000:1:9").expect("parses");
        let cut = EqChain::parse("peaking:2000:1:-9").expect("parses");
        let top = MAX_HZ.min(f64::from(SR) / 2.0 * 0.98);
        let up = db_near(&drawn_points(&render(&boost, SR), top), 2000.0);
        let down = db_near(&drawn_points(&render(&cut, SR), top), 2000.0);
        assert!(up > 6.0, "a +9 dB bell should draw near +9, got {up:.2}");
        assert!(
            down < -6.0,
            "a -9 dB bell should draw near -9, got {down:.2}"
        );
        assert!(
            y_of(9.0) < y_of(-9.0),
            "positive decibels sit higher on screen"
        );
    }

    /// The axis stops at the source's Nyquist. Drawing to 20 kHz on a 16 kHz
    /// source would show a band that source does not have, and the filter's
    /// response there is not defined.
    #[test]
    fn the_axis_ends_at_the_sources_nyquist() {
        let chain = EqChain::parse("highpass:120").expect("parses");
        let svg = render(&chain, 16_000);
        assert!(
            svg.contains(">5k<"),
            "5 kHz label expected below an 8 kHz Nyquist"
        );
        assert!(
            !svg.contains(">10k<") && !svg.contains(">20k<"),
            "no label may sit above the 8 kHz Nyquist:\n{svg}"
        );
        // ...and the full-rate curve does carry them, so the test above is
        // measuring the Nyquist cap rather than a label that never renders.
        let wide = render(&chain, 48_000);
        assert!(wide.contains(">10k<"), "48 kHz should reach 10 kHz");
    }

    /// An empty chain draws the flat line, not an empty box: clearing the
    /// field has to look like "no filtering" rather than a panel that broke.
    #[test]
    fn an_empty_chain_draws_a_flat_line_at_zero() {
        let svg = render(&EqChain::default(), SR);
        let points = drawn_points(&svg, MAX_HZ.min(f64::from(SR) / 2.0 * 0.98));
        assert!(points.len() > 100, "the line is drawn");
        for (hz, db) in &points {
            // Coordinates are written to one decimal place, and the y axis
            // packs 54 dB into 152 px, so a rounded pixel is worth about
            // 0.036 dB. That is the floor this can be held to, and it is two
            // orders of magnitude below anything visible.
            assert!(db.abs() < 0.05, "flat at {hz:.0} Hz, got {db:.3} dB");
        }
        assert!(svg.contains("No filtering"), "and says so in the label");
    }

    /// A response that runs off the bottom is clamped into the box rather than
    /// drawn outside it, where the browser would clip the path and the reader
    /// would see the line simply stop.
    #[test]
    fn a_response_past_the_axis_is_clamped_into_the_box() {
        let chain = EqChain::parse("highpass:2000:0.707:0:4").expect("parses");
        let svg = render(&chain, SR);
        let start = svg.find(r#"points=""#).expect("polyline") + 8;
        let rest = &svg[start..];
        let raw = &rest[..rest.find('"').expect("terminated")];
        for pair in raw.split_whitespace() {
            let (x, y) = pair.split_once(',').expect("x,y");
            let x: f64 = x.parse().expect("number");
            let y: f64 = y.parse().expect("number");
            assert!(
                (PAD_L - 0.05..=W - PAD_R + 0.05).contains(&x),
                "x {x} outside"
            );
            assert!(
                (PAD_T - 0.05..=H - PAD_B + 0.05).contains(&y),
                "y {y} outside"
            );
        }
    }

    /// The label is the only thing a screen reader gets, so it has to carry
    /// the numbers rather than say "a chart".
    #[test]
    fn the_label_reports_the_range_the_curve_covers() {
        let chain = EqChain::parse("peaking:2000:1:9").expect("parses");
        let svg = render(&chain, SR);
        assert!(svg.contains("1 stage"), "stage count, singular:\n{svg}");
        assert!(
            svg.contains("+9.0 dB") || svg.contains("+8.9 dB"),
            "peak:\n{svg}"
        );

        let two = EqChain::parse("peaking:2000:1:9; highpass:100").expect("parses");
        assert!(render(&two, SR).contains("2 stages"), "plural");
    }
}
