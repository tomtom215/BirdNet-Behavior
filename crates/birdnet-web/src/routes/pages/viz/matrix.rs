//! The species co-occurrence intensity grid.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign
//! `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::{EMPTY, svg_a11y};
use crate::routes::pages::atoms::species_code;
use crate::routes::pages::escape_html;

/// Symmetric N×N intensity grid. `m[i][j]` is a normalised 0–1 strength;
/// the diagonal is drawn as a neutral cell.
#[must_use]
pub fn cooccurrence_matrix(labels: &[String], m: &[Vec<f64>]) -> String {
    let n = labels.len();
    if n < 2 {
        return EMPTY.to_string();
    }
    let cell = 30_i32;
    let gutter = 92_i32; // room for row labels (left) and rotated col labels (top)
    let size_w = gutter + n as i32 * cell + 8;
    let size_h = gutter + n as i32 * cell + 8;

    let mut svg = format!(
        r#"<div class="viz-scroll"><svg width="{size_w}" height="{size_h}" viewBox="0 0 {size_w} {size_h}" role="img">"#
    );
    svg.push_str(&svg_a11y(
        "Species co-occurrence matrix",
        "A grid of species pairs; a darker cell means those two species were detected in the same five-minute window more often.",
    ));

    // Column labels (rotated) + row labels.
    for (i, name) in labels.iter().enumerate() {
        let code = species_code(name);
        let title = escape_html(name);
        let x = gutter + i as i32 * cell + cell / 2;
        let y = gutter + i as i32 * cell + cell / 2;
        // column header, rotated -45°
        let _ = write!(
            svg,
            r#"<text class="mono" x="{x}" y="{cy}" transform="rotate(-45 {x} {cy})" text-anchor="start" font-size="9" fill="var(--fg-3)"><title>{title}</title>{code}</text>"#,
            cy = gutter - 6,
        );
        // row label
        let _ = write!(
            svg,
            r#"<text class="mono" x="{lx}" y="{y}" text-anchor="end" dominant-baseline="middle" font-size="9" fill="var(--fg-3)"><title>{title}</title>{code}</text>"#,
            lx = gutter - 8,
        );
    }

    // Cells.
    for (i, row) in m.iter().enumerate().take(n) {
        for (j, &v) in row.iter().enumerate().take(n) {
            let x = gutter + j as i32 * cell;
            let y = gutter + i as i32 * cell;
            if i == j {
                let _ = write!(
                    svg,
                    r#"<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="3" fill="var(--surface-2)"/>"#,
                    w = cell - 3,
                );
                continue;
            }
            let op = 0.06 + v.clamp(0.0, 1.0) * 0.9;
            let a = escape_html(&labels[i]);
            let b = escape_html(&labels[j]);
            let pct = (v * 100.0).round() as i64;
            let _ = write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{w}" height="{w}" rx="3" fill="var(--moss)" fill-opacity="{op:.2}"><title>{a} × {b} — {pct}%</title></rect>"#,
                w = cell - 3,
            );
        }
    }

    svg.push_str("</svg></div>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_empty_for_small_input() {
        assert!(cooccurrence_matrix(&[], &[]).contains("Not enough data"));
        assert!(cooccurrence_matrix(&["A".into()], &[vec![1.0]]).contains("Not enough data"));
    }

    #[test]
    fn matrix_renders_cells_and_labels() {
        let labels = vec!["Blue Jay".to_string(), "Northern Cardinal".to_string()];
        let m = vec![vec![1.0, 0.5], vec![0.5, 1.0]];
        let svg = cooccurrence_matrix(&labels, &m);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("BLJA"));
        assert!(svg.contains("NOCA"));
        assert!(svg.contains("fill-opacity"));
        // Accessible name + description (role="img" needs an alt; the bare
        // aria-label was replaced by the richer <title>/<desc> pair).
        assert!(svg.contains("<title>Species co-occurrence matrix</title>"));
        assert!(svg.contains("<desc>A grid of species pairs;"));
        assert!(!svg.contains("aria-label"));
    }
}
