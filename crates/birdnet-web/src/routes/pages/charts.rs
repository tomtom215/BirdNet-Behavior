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
        if let Ok(hour) = h.hour.parse::<usize>()
            && hour < 24
        {
            counts[hour] = h.count;
        }
    }

    if counts.iter().all(|&c| c == 0) {
        return r#"<p class="cht-empty">No detections today yet.</p>"#.to_string();
    }

    let max_count = counts.iter().copied().max().unwrap_or(1).max(1);
    let chart_w = 700;
    let chart_h = 120;
    let bar_w = 25;
    let gap = 4;
    let left_pad = 5;

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" class="cht-svg" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 20,
    );

    for (i, &count) in counts.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;
        let color = if count > 0 {
            "var(--moss)"
        } else {
            "var(--surface-2)"
        };

        let _ = write!(
            svg,
            r#"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="{color}"/>"#,
        );

        if count > 0 {
            let _ = write!(
                svg,
                r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-3)" font-size="9" font-family="sans-serif">{count}</text>"#,
                tx = x + bar_w / 2,
                ty = y - 3,
            );
        }

        if i % 3 == 0 {
            let _ = write!(
                svg,
                r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-4)" font-size="9" font-family="sans-serif">{i:02}</text>"#,
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
        return r#"<p class="cht-empty">No detection data yet.</p>"#.to_string();
    }

    let max_count = days.iter().map(|d| d.count).max().unwrap_or(1).max(1);
    let chart_h = 100;
    let bar_w = 32;
    let gap = 6;
    let left_pad = 5;
    // Derived from the data, not fixed. This used to be a hard-coded 280 while
    // `x` stepped by `bar_w + gap` per day, so the caller's 14-day request drew
    // six bars outside the viewBox and the browser clipped them without a word.
    // The SVG carries no intrinsic width, so `.bnb-card svg { max-width: 100% }`
    // scales whatever we declare here down into the card.
    let chart_w = left_pad + days.len() as i32 * (bar_w + gap);

    let mut svg = format!(
        r#"<svg viewBox="0 0 {svg_w} {svg_h}" class="cht-svg" xmlns="http://www.w3.org/2000/svg">"#,
        svg_w = chart_w,
        svg_h = chart_h + 22,
    );

    for (i, day) in days.iter().enumerate() {
        let x = left_pad + i as i32 * (bar_w + gap);
        let bar_h = (day.count as f64 / max_count as f64 * chart_h as f64) as i32;
        let y = chart_h - bar_h;

        let _ = write!(
            svg,
            r#"<rect x="{x}" y="{y}" width="{bar_w}" height="{bar_h}" rx="2" fill="var(--moss)"/>"#,
        );

        if day.count > 0 {
            let _ = write!(
                svg,
                r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-3)" font-size="9" font-family="sans-serif">{count}</text>"#,
                tx = x + bar_w / 2,
                ty = y - 3,
                count = day.count,
            );
        }

        let date_label = day.date.get(5..).unwrap_or(&day.date);
        let _ = write!(
            svg,
            r#"<text x="{tx}" y="{ty}" text-anchor="middle" fill="var(--fg-4)" font-size="8" font-family="sans-serif">{label}</text>"#,
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
        return r#"<p class="cht-empty">No detection data yet.</p>"#.to_string();
    }

    let max_count = buckets.iter().copied().max().unwrap_or(1).max(1);
    let labels = ["<50%", "50-60%", "60-70%", "70-80%", "80-90%", "90-100%"];
    let colors = [
        "var(--fg-4)",
        "var(--rare)",
        "var(--dawn-ink)",
        "var(--dawn)",
        "var(--moss-ink)",
        "var(--moss)",
    ];

    let bar_h = 18;
    let gap = 6;
    let label_w = 55;
    let chart_w = 280;
    let max_bar_w = chart_w - label_w - 40;
    let svg_h = 6 * (bar_h + gap);

    let mut svg = format!(
        r#"<svg viewBox="0 0 {chart_w} {svg_h}" class="cht-svg" xmlns="http://www.w3.org/2000/svg">"#,
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
            r#"<text x="{lx}" y="{ly}" text-anchor="end" fill="var(--fg-3)" font-size="10" font-family="sans-serif" dominant-baseline="middle">{label}</text>"#,
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
                r#"<text x="{tx}" y="{ty}" fill="var(--fg-3)" font-size="9" font-family="sans-serif" dominant-baseline="middle">{count}</text>"#,
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

    /// Extract `(x, width)` for every `<rect>` that carries an explicit `x`.
    ///
    /// The background rect, where one exists, has no `x`, so it is skipped and
    /// only the data bars are returned.
    fn bar_boxes(svg: &str) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        for frag in svg.split("<rect ").skip(1) {
            let attr = |name: &str| -> Option<i64> {
                let key = format!("{name}=\"");
                let rest = frag.split_once(&key)?.1;
                rest.split_once('"')?.0.parse().ok()
            };
            if let (Some(x), Some(w)) = (attr("x"), attr("width")) {
                out.push((x, w));
            }
        }
        out
    }

    fn view_box_width(svg: &str) -> i64 {
        let vb = svg
            .split_once("viewBox=\"")
            .expect("chart has a viewBox")
            .1
            .split_once('"')
            .expect("viewBox is terminated")
            .0;
        vb.split_whitespace()
            .nth(2)
            .expect("viewBox has a width")
            .parse()
            .expect("viewBox width is an integer")
    }

    /// Every bar `render_daily_chart` draws must fit inside the `viewBox` it
    /// declares.
    ///
    /// Observed failing against the pre-fix renderer, which hard-coded
    /// `chart_w = 280` regardless of how many days it was handed while
    /// stepping `x` by `bar_w + gap` = 38 per day. The caller
    /// (`species_pages::species_daily_partial`) asks for 14 days, so bar 7
    /// ended at x = 303 and bars 8..=13 started at 309..=537 — entirely
    /// outside the 280-wide viewBox. A species heard on nine or more distinct
    /// days silently lost its most recent week, and the skeleton it replaced
    /// (`skeletons::trend_line`) draws 14 bars, so the card visibly shrank on
    /// swap. With `n = 14` the old code failed this at the first bar past
    /// index 6; `n = 1` and `n = 7` passed even then, which is why only the
    /// 14-day case proves the fix.
    #[test]
    fn daily_chart_bars_fit_inside_their_view_box() {
        for n in [1_usize, 2, 7, 14, 30, 90] {
            let days: Vec<_> = (0..n)
                .map(|i| birdnet_db::sqlite::DailyCount {
                    date: format!("2026-01-{:02}", (i % 28) + 1),
                    count: i64::try_from(i % 17).unwrap_or(0) + 1,
                })
                .collect();
            let svg = render_daily_chart(&days);
            let vb_w = view_box_width(&svg);
            let boxes = bar_boxes(&svg);
            assert_eq!(boxes.len(), n, "n = {n}: every day must draw a bar");
            for (i, (x, w)) in boxes.iter().enumerate() {
                assert!(
                    x + w <= vb_w,
                    "n = {n}: bar {i} spans {x}..{} but the viewBox is only {vb_w} wide",
                    x + w
                );
                assert!(*x >= 0, "n = {n}: bar {i} starts at a negative x ({x})");
            }
        }
    }

    /// The counterpart to the gate above: proving bars *fit* is worthless if
    /// the renderer could satisfy it by drawing them on top of each other.
    #[test]
    fn daily_chart_bars_do_not_overlap() {
        let days: Vec<_> = (0..14)
            .map(|i| birdnet_db::sqlite::DailyCount {
                date: format!("2026-01-{:02}", i + 1),
                count: 5,
            })
            .collect();
        let boxes = bar_boxes(&render_daily_chart(&days));
        for pair in boxes.windows(2) {
            let (x0, w0) = pair[0];
            let (x1, _) = pair[1];
            assert!(
                x1 >= x0 + w0,
                "bar at {x1} overlaps the one ending at {}",
                x0 + w0
            );
        }
    }
}
