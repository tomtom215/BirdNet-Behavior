//! Bespoke analytics visualizations rendered as inline SVG.
//!
//! Pure functions, styled by the design tokens in `static/css/app.css`:
//! - [`cooccurrence_matrix`] — who-sings-with-whom intensity grid
//! - [`streamgraph`] — centred stacked species activity over a window
//! - [`circadian_polar`] — the dawn-chorus 24-hour polar plot
//!
//! Colours come from [`super::atoms::species_color`] so a species keeps the
//! same hue across every screen.
//!
//! These are geometry-heavy SVG generators; the lint allows below cover the
//! pervasive, benign `f64`/`i32` coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::atoms::{species_code, species_color};
use super::escape_html;

const EMPTY: &str = r#"<p class="bnb-meta" style="text-align:center;padding:1.5rem;">Not enough data yet for this view.</p>"#;

/// (x, y) on a circle centred at (`cx`, `cy`).
fn polar(cx: f64, cy: f64, r: f64, ang: f64) -> (f64, f64) {
    (cx + r * ang.cos(), cy + r * ang.sin())
}

/// Hour-of-day (0–24, fractional ok) → angle with midnight at the top.
fn hour_angle(h: f64) -> f64 {
    (h / 24.0) * std::f64::consts::TAU - std::f64::consts::FRAC_PI_2
}

// ───────────────────────────── co-occurrence matrix ────────────────────────

/// Symmetric N×N intensity grid. `m[i][j]` is a normalised 0–1 strength;
/// the diagonal is drawn as a neutral cell.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn cooccurrence_matrix(labels: &[String], m: &[Vec<f64>]) -> String {
    let n = labels.len();
    if n < 2 {
        return EMPTY.to_string();
    }
    let cell = 30_i32;
    let gutter = 92_i32; // room for row labels (left) and rotated col labels (top)
    let size_w = gutter + n as i32 * cell + 8;
    let size_h = gutter + n as i32 * cell + 8;

    let mut svg = format!(
        r#"<div style="overflow-x:auto;"><svg width="{size_w}" height="{size_h}" viewBox="0 0 {size_w} {size_h}" role="img" aria-label="Species co-occurrence matrix">"#
    );

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

// ──────────────────────────────── streamgraph ──────────────────────────────

/// Centred stacked-area ("themeriver") of per-day counts. Each series is
/// `(common_name, daily_counts)`; all vectors should share a length.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn streamgraph(series: &[(String, Vec<i64>)]) -> String {
    let days = series.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    if series.is_empty() || days < 2 {
        return EMPTY.to_string();
    }

    let w = 760.0_f64;
    let h = 220.0_f64;
    let mid = h / 2.0;
    let step = w / (days - 1) as f64;

    // Per-day total → vertical scale so the fattest day fills ~85% of height.
    let mut max_total = 1.0_f64;
    for d in 0..days {
        let total: i64 = series
            .iter()
            .map(|(_, v)| v.get(d).copied().unwrap_or(0))
            .sum();
        max_total = max_total.max(total as f64);
    }
    let scale = (h * 0.85) / max_total;
    let val_y = |v: f64| mid - v * scale;

    let mut svg = format!(
        r#"<div style="overflow-x:auto;"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="none" role="img" aria-label="Activity streamgraph" style="display:block;">"#
    );

    // Stack bands from a centred baseline.
    for (name, counts) in series {
        let color = species_color(name);
        let mut top = String::new();
        let mut bottom: Vec<(f64, f64)> = Vec::with_capacity(days);
        for d in 0..days {
            let total: f64 = series
                .iter()
                .map(|(_, v)| v.get(d).copied().unwrap_or(0))
                .sum::<i64>() as f64;
            let below: f64 = series
                .iter()
                .take_while(|(n2, _)| n2 != name)
                .map(|(_, v)| v.get(d).copied().unwrap_or(0))
                .sum::<i64>() as f64;
            let val = counts.get(d).copied().unwrap_or(0) as f64;
            let lower = below - total / 2.0;
            let upper = lower + val;
            let x = d as f64 * step;
            let _ = write!(
                top,
                "{}{x:.1},{y:.1} ",
                if d == 0 { "M" } else { "L" },
                y = val_y(upper)
            );
            bottom.push((x, val_y(lower)));
        }
        let mut path = top;
        for (x, y) in bottom.iter().rev() {
            let _ = write!(path, "L{x:.1},{y:.1} ");
        }
        path.push('Z');
        let _ = write!(
            svg,
            r#"<path d="{path}" fill="{color}" fill-opacity="0.82" stroke="{color}" stroke-width="0.5" stroke-opacity="0.5"><title>{n}</title></path>"#,
            n = escape_html(name),
        );
    }

    svg.push_str("</svg></div>");
    // Legend.
    svg.push_str(r#"<div style="display:flex;gap:12px;flex-wrap:wrap;margin-top:10px;">"#);
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta" style="display:inline-flex;align-items:center;gap:5px;"><span style="width:9px;height:9px;border-radius:2px;background:{c};display:inline-block;"></span>{n}</span>"#,
            c = species_color(name),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");
    svg
}

// ─────────────────────────── dawn-chorus polar ─────────────────────────────

/// 24-hour polar plot. Each series is `(common_name, [hourly_value; 24])`;
/// values are normalised against the global maximum so heights compare.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn circadian_polar(series: &[(String, [f64; 24])]) -> String {
    if series.is_empty() {
        return EMPTY.to_string();
    }
    let global_max = series
        .iter()
        .flat_map(|(_, v)| v.iter())
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let size = 320.0_f64;
    let cx = size / 2.0;
    let cy = size / 2.0;
    let ir = 30.0_f64;
    let or = size / 2.0 - 30.0;

    let mut svg = format!(
        r#"<svg width="{size:.0}" height="{size:.0}" viewBox="0 0 {size:.0} {size:.0}" role="img" aria-label="Dawn chorus circadian plot" style="max-width:100%;display:block;margin:0 auto;">"#
    );

    // Dawn wedge (05:00–08:00) + dusk wedge (17:00–20:00).
    for (start, end, tok) in [(5.0, 8.0, "var(--dawn)"), (17.0, 20.0, "var(--night)")] {
        let (x0, y0) = polar(cx, cy, or, hour_angle(start));
        let (x1, y1) = polar(cx, cy, or, hour_angle(end));
        let _ = write!(
            svg,
            r#"<path d="M{cx:.1},{cy:.1} L{x0:.1},{y0:.1} A{or:.1},{or:.1} 0 0 1 {x1:.1},{y1:.1} Z" fill="{tok}" fill-opacity="0.12"/>"#,
        );
    }

    // Hour rings + spokes.
    for r in [ir, f64::midpoint(ir, or), or] {
        let _ = write!(
            svg,
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" fill="none" stroke="var(--hairline)" stroke-width="0.5"/>"#,
        );
    }
    for (h, lbl) in [(0.0, "12a"), (6.0, "6a"), (12.0, "12p"), (18.0, "6p")] {
        let (lx, ly) = polar(cx, cy, or + 12.0, hour_angle(h));
        let _ = write!(
            svg,
            r#"<text class="mono" x="{lx:.1}" y="{ly:.1}" text-anchor="middle" dominant-baseline="middle" font-size="9" fill="var(--fg-4)">{lbl}</text>"#,
        );
    }

    // Per-species closed polar areas.
    for (name, hours) in series {
        let color = species_color(name);
        let mut path = String::new();
        for (h, &v) in hours.iter().enumerate() {
            let r = ir + (v / global_max) * (or - ir);
            let (x, y) = polar(cx, cy, r, hour_angle(h as f64));
            let _ = write!(path, "{}{x:.1},{y:.1} ", if h == 0 { "M" } else { "L" });
        }
        path.push('Z');
        let _ = write!(
            svg,
            r#"<path d="{path}" fill="{color}" fill-opacity="0.16" stroke="{color}" stroke-width="1.4" stroke-opacity="0.85"><title>{n}</title></path>"#,
            n = escape_html(name),
        );
    }

    svg.push_str("</svg>");
    // Legend.
    svg.push_str(r#"<div style="display:flex;gap:12px;flex-wrap:wrap;justify-content:center;margin-top:8px;">"#);
    for (name, _) in series {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta" style="display:inline-flex;align-items:center;gap:5px;"><span style="width:9px;height:9px;border-radius:50%;background:{c};display:inline-block;"></span>{n}</span>"#,
            c = species_color(name),
            n = escape_html(name),
        );
    }
    svg.push_str("</div>");
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
    }

    #[test]
    fn streamgraph_empty_and_basic() {
        assert!(streamgraph(&[]).contains("Not enough data"));
        let series = vec![
            ("Blue Jay".to_string(), vec![1, 3, 2, 5, 4]),
            ("American Robin".to_string(), vec![0, 2, 4, 3, 1]),
        ];
        let svg = streamgraph(&series);
        assert!(svg.contains("<svg") && svg.contains("<path"));
        assert!(svg.contains("Blue Jay"));
    }

    #[test]
    fn polar_empty_and_basic() {
        assert!(circadian_polar(&[]).contains("Not enough data"));
        let mut h = [0.0_f64; 24];
        h[6] = 5.0;
        h[7] = 8.0;
        let svg = circadian_polar(&[("Northern Cardinal".to_string(), h)]);
        assert!(svg.contains("<svg") && svg.contains("12a") && svg.contains("6a"));
    }
}
