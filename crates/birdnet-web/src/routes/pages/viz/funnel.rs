//! Funnel chart: step-by-step drop-off of an ordered species sequence.
//!
//! Visualizes the v0.8.0 `window_funnel` result for the dawn "running order" —
//! how many mornings reach each successive step of the sequence. Each bar's
//! width is the count of mornings that got that far, so the chart narrows as the
//! chorus progresses, the classic funnel shape. Labels sit on the card surface
//! (not over the coloured bars) so text contrast is unaffected.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::{EMPTY, svg_a11y};
use crate::routes::pages::atoms::series_color;
use crate::routes::pages::escape_html;

/// Render a funnel of `(species, count)` steps in funnel order — the first step
/// is the widest and counts never increase down the chain. `count[i]` is how
/// many observations reached step `i`.
///
/// Returns the shared [`EMPTY`] placeholder for fewer than two steps or an
/// all-zero funnel (nothing reached even the first step).
#[must_use]
pub fn sequence_funnel(steps: &[(String, u64)]) -> String {
    if steps.len() < 2 {
        return EMPTY.to_string();
    }
    let max = steps.iter().map(|(_, c)| *c).max().unwrap_or(0);
    let first = steps[0].1;
    if max == 0 {
        return EMPTY.to_string();
    }

    let w = 520.0_f64;
    let row_h = 46.0_f64;
    let bar_h = 16.0_f64;
    let pad_top = 4.0_f64;
    let h = pad_top + steps.len() as f64 * row_h;

    let mut svg = format!(
        r#"<svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="viz-svg-block">"#
    );
    svg.push_str(&svg_a11y(
        "Dawn running-order funnel",
        "How many mornings reach each step of the dawn sequence; each bar's width is the count of mornings that got that far, narrowing as the chorus progresses.",
    ));

    for (i, (name, count)) in steps.iter().enumerate() {
        let y = pad_top + i as f64 * row_h;
        let label_y = y + 12.0;
        let bar_y = y + 18.0;
        let frac = *count as f64 / max as f64;
        // Keep a 2px sliver visible for any non-zero step so it never vanishes.
        let bar_w = if *count == 0 {
            0.0
        } else {
            (frac * w).max(2.0)
        };
        let pct = *count as f64 / first.max(1) as f64 * 100.0;
        let color = series_color(i, steps.len());

        // Species (left) and "count · pct%" (right) on the surface — readable
        // regardless of the bar colour below them.
        let _ = write!(
            svg,
            r#"<text x="0" y="{label_y:.1}" fill="var(--fg)" font-size="13" font-weight="500">{n}</text>"#,
            n = escape_html(name),
        );
        let _ = write!(
            svg,
            r#"<text x="{w:.0}" y="{label_y:.1}" fill="var(--fg-3)" font-size="12" text-anchor="end">{count} · {pct:.0}%</text>"#,
        );
        // Faint full-width track, then the coloured bar over it.
        let _ = write!(
            svg,
            r#"<rect x="0" y="{bar_y:.1}" width="{w:.0}" height="{bar_h:.0}" rx="4" fill="var(--surface-2)"/>"#,
        );
        let _ = write!(
            svg,
            r#"<rect x="0" y="{bar_y:.1}" width="{bar_w:.1}" height="{bar_h:.0}" rx="4" fill="{color}"><title>{n}: {count}</title></rect>"#,
            n = escape_html(name),
        );
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps() -> Vec<(String, u64)> {
        vec![
            ("American Robin".to_string(), 20),
            ("Song Sparrow".to_string(), 12),
            ("Northern Cardinal".to_string(), 5),
        ]
    }

    #[test]
    fn renders_accessible_svg_with_all_steps() {
        let svg = sequence_funnel(&steps());
        assert!(svg.contains(r#"role="img""#));
        assert!(svg.contains("<title>Dawn running-order funnel</title>"));
        assert!(svg.contains("<desc>"));
        assert!(svg.contains("American Robin"));
        assert!(svg.contains("Song Sparrow"));
        assert!(svg.contains("Northern Cardinal"));
        // First step is 100% of itself; later steps are proportions of it.
        assert!(svg.contains("20 · 100%"));
        assert!(svg.contains("12 · 60%"));
        assert!(svg.contains("5 · 25%"));
    }

    #[test]
    fn bars_narrow_monotonically() {
        // The coloured bars carry an `oklch` fill (tracks use a token); pull the
        // width attribute immediately preceding each and confirm it never grows.
        let svg = sequence_funnel(&steps());
        let widths: Vec<f64> = svg
            .match_indices(r#"fill="oklch"#)
            .filter_map(|(idx, _)| {
                let before = &svg[..idx];
                let wstart = before.rfind(r#"width=""#)? + r#"width=""#.len();
                let wend = before[wstart..].find('"')? + wstart;
                before[wstart..wend].parse::<f64>().ok()
            })
            .collect();
        assert_eq!(widths.len(), 3, "one coloured bar per step");
        assert!(
            widths[0] >= widths[1] && widths[1] >= widths[2],
            "funnel must narrow: {widths:?}"
        );
    }

    #[test]
    fn too_few_steps_is_empty() {
        assert_eq!(sequence_funnel(&[]), EMPTY);
        assert_eq!(sequence_funnel(&[("Solo".to_string(), 9)]), EMPTY);
    }

    #[test]
    fn all_zero_is_empty() {
        let z = vec![("A".to_string(), 0), ("B".to_string(), 0)];
        assert_eq!(sequence_funnel(&z), EMPTY);
    }
}
