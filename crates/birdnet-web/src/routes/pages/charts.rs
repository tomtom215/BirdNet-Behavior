//! SVG chart rendering helpers.
//!
//! Produces inline SVG for embedding directly in HTMX partial responses.
//! All functions return a `String` that can be inserted directly into HTML.

use std::fmt::Write as _;

/// Render an SVG bar chart for hourly detection counts (0–23).
///
/// Returns a "no data" message if all counts are zero.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
pub(crate) fn render_hourly_chart(hours: &[birdnet_db::sqlite::HourlyCount]) -> String {
    let mut counts = [0_i64; 24];
    for h in hours {
        if let Ok(hour) = h.hour.parse::<usize>() {
            if hour < 24 {
                counts[hour] = h.count;
            }
        }
    }

    if counts.iter().all(|&c| c == 0) {
        return r#"<p style="color: var(--text-muted)">No detections today yet.</p>"#.to_string();
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);
    let chart_w = 700;
    let chart_h = 120;
    let bar_w = 25;
    let gap = 4;
    let left_pad = 5;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" style="width: 100%; height: auto; display: block;" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 20,
    );

    for (i, &count) in counts.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;
        let color = if count > 0 { "#38bdf8" } else { "#1e293b" };

        let _ = write!(
            svg,
            r#"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="{color}"/>"#,
        );

        if count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#94a3b8" font-size="9" font-family="sans-serif">{count}</text>"##,
                tx = x + bar_w / 2,
                ty = y - 3,
            );
        }

        if i % 3 == 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#64748b" font-size="9" font-family="sans-serif">{i:02}</text>"##,
                tx = x + bar_w / 2,
                ty = chart_h + 14,
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

/// Render an SVG bar chart for daily detection counts.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
pub(crate) fn render_daily_chart(days: &[birdnet_db::sqlite::DailyCount]) -> String {
    if days.is_empty() {
        return r#"<p style="color: var(--text-muted)">No detection data yet.</p>"#.to_string();
    }

    let max_count = days.iter().map(|d| d.count).max().unwrap_or(1).max(1);
    let chart_w = 280;
    let chart_h = 100;
    let bar_w = 32;
    let gap = 6;
    let left_pad = 5;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" style="width: 100%; height: auto; display: block;" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 22,
    );

    for (i, day) in days.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (day.count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;

        let _ = write!(
            svg,
            r##"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="#38bdf8"/>"##,
        );

        if day.count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#94a3b8" font-size="9" font-family="sans-serif">{count}</text>"##,
                tx = x + bar_w / 2,
                ty = y - 3,
                count = day.count,
            );
        }

        let date_label = day.date.get(5..).unwrap_or(&day.date);
        let _ = write!(
            svg,
            r##"<text x="{tx}" y="{ty}" text-anchor="middle" fill="#64748b" font-size="8" font-family="sans-serif">{label}</text>"##,
            tx = x + bar_w / 2,
            ty = chart_h + 14,
            label = super::escape_html(date_label),
        );
    }

    svg.push_str("</svg>");
    svg
}

/// Render an SVG horizontal bar chart for confidence distribution.
///
/// Buckets: `[0-50%, 50-60%, 60-70%, 70-80%, 80-90%, 90-100%]`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_lossless
)]
pub(crate) fn render_confidence_chart(buckets: &[i64; 6]) -> String {
    let total: i64 = buckets.iter().sum();
    if total == 0 {
        return r#"<p style="color: var(--text-muted)">No detection data yet.</p>"#.to_string();
    }

    let max_count = buckets.iter().copied().max().unwrap_or(1).max(1);
    let labels = ["<50%", "50-60%", "60-70%", "70-80%", "80-90%", "90-100%"];
    let colors = [
        "#64748b", "#f59e0b", "#eab308", "#84cc16", "#22c55e", "#10b981",
    ];

    let bar_h = 18;
    let gap = 6;
    let label_w = 55;
    let chart_w = 280;
    let max_bar_w = chart_w - label_w - 40;
    let svg_h = 6 * (bar_h + gap);

    let mut svg = format!(
        r#"<svg viewBox="0 0 {chart_w} {svg_h}" style="width: 100%; height: auto; display: block;" xmlns="http://www.w3.org/2000/svg">"#,
    );

    for (i, (&count, (&label, &color))) in buckets
        .iter()
        .zip(labels.iter().zip(colors.iter()))
        .enumerate()
    {
        let y = i as i32 * (bar_h + gap);
        let bar_w = if max_count > 0 {
            (count as f64 / max_count as f64 * max_bar_w as f64) as i32
        } else {
            0
        };

        let _ = write!(
            svg,
            r##"<text x="{lx}" y="{ly}" text-anchor="end" fill="#94a3b8" font-size="10" font-family="sans-serif" dominant-baseline="middle">{label}</text>"##,
            lx = label_w - 4,
            ly = y + bar_h / 2,
        );
        let _ = write!(
            svg,
            r#"<rect x="{label_w}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="{color}"/>"#,
        );
        if count > 0 {
            let _ = write!(
                svg,
                r##"<text x="{tx}" y="{ty}" fill="#94a3b8" font-size="9" font-family="sans-serif" dominant-baseline="middle">{count}</text>"##,
                tx = label_w + bar_w + 4,
                ty = y + bar_h / 2,
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_chart_empty() {
        let r = render_hourly_chart(&[]);
        assert!(r.contains("No detections today"));
    }

    #[test]
    fn daily_chart_empty() {
        let r = render_daily_chart(&[]);
        assert!(r.contains("No detection data"));
    }

    #[test]
    fn confidence_chart_empty() {
        let r = render_confidence_chart(&[0; 6]);
        assert!(r.contains("No detection data"));
    }

    #[test]
    fn confidence_chart_all_labels() {
        let svg = render_confidence_chart(&[5, 10, 20, 30, 25, 15]);
        assert!(svg.contains("<50%"));
        assert!(svg.contains("90-100%"));
    }
}
