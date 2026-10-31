//! The Trends charts: detections per day with its moving average, and species richness.
//!
//! "Is your yard getting busier?" is a question about a *direction*, and a column of
//! figures makes the reader do the subtraction. These draw the direction, and keep
//! three things honest that a plain line chart quietly drops:
//!
//! * the **daily values stay visible** under the smoothed line (thin columns, neutral),
//!   so how noisy the days were is part of the picture and the average is not mistaken
//!   for the data;
//! * the **previous period** is a ghost line on the same axis, aligned day for day, so
//!   "busier" means busier than something you can see;
//! * the **extremes** are named on the chart (the lowest and the highest day), and the
//!   axis starts at zero.
//!
//! A day the station was not listening is hatched (see `pages::listening`).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]

use std::fmt::Write as _;

use super::kit::{self, Col, Linear};
use super::{EMPTY, svg_a11y};
use crate::routes::pages::escape_html;

/// One day of a trend.
pub struct TrendPoint {
    /// ISO date, `YYYY-MM-DD`.
    pub date: String,
    /// The day's value.
    pub value: f64,
    /// A smoothed value for the same day, if there is one (a moving average).
    pub smooth: Option<f64>,
}

/// What a trend chart is told besides its points.
pub struct TrendOpts<'a> {
    /// `data-ck-id` and the prefix of the chart's gradient ids; unique on a page.
    pub id: &'a str,
    /// What is counted, plural (`detections`, `species`).
    pub unit: &'a str,
    /// What the smoothed line is called (`7-day average`); empty when there is none.
    pub smooth_name: &'a str,
    /// Draw the days as thin columns under a smoothed line (true), or as the line itself (false).
    pub bars: bool,
    /// The same number of days just before, for the ghost line; may be empty.
    pub prior: &'a [TrendPoint],
    /// Whether the station was listening each day; `None` claims nothing.
    pub listened: Option<&'a [bool]>,
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        kit::thousands(v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// A day-by-day trend as a picture, above the table it summarises.
///
/// With `bars`, each day is a thin neutral column and the smoothed series is the one
/// line in the chart's colour with a soft area beneath it; without, the series is the
/// line itself (species richness has no smoothing) with a dot a day. The previous period
/// is a ghost line; the busiest and quietest days are named; a crosshair and tooltip give
/// the day, its value, the average and the day it is compared with.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn trend_chart(points: &[TrendPoint], o: &TrendOpts<'_>) -> String {
    if points.len() < 2 {
        return EMPTY.to_string();
    }
    let n = points.len();
    let off = |i: usize| o.listened.is_some_and(|l| l.get(i).is_some_and(|on| !on));
    // The prior period lines up with the current one from the end.
    let prior: Vec<&TrendPoint> = o
        .prior
        .iter()
        .rev()
        .take(n)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let prior_at = |i: usize| -> Option<&TrendPoint> {
        let off = n - prior.len();
        i.checked_sub(off).and_then(|k| prior.get(k).copied())
    };
    let line_of = |p: &TrendPoint| if o.bars { p.smooth } else { Some(p.value) };

    let top_data = points
        .iter()
        .flat_map(|p| [p.value, p.smooth.unwrap_or(0.0)])
        .chain(
            prior
                .iter()
                .flat_map(|p| [p.value, p.smooth.unwrap_or(0.0)]),
        )
        .filter(|v| v.is_finite())
        .fold(0.0_f64, f64::max);
    let top = kit::count_ceiling(top_data);
    let rules = kit::count_ticks(top, 4);
    let widest = rules
        .iter()
        .map(|&v| kit::text_width(&kit::compact(v), 10.0))
        .fold(0.0_f64, f64::max);

    let w = 640.0_f64;
    let (pad_l, pad_r, pad_t, pad_b) = (kit::axis_margin(widest, w), 12.0, 14.0, 24.0);
    let h = 200.0_f64;
    let plot_w = w - pad_l - pad_r;
    let plot_h = h - pad_t - pad_b;
    let slot = plot_w / n as f64;
    let x_at = |i: usize| pad_l + (i as f64 + 0.5) * slot;
    let y = Linear::new((0.0, top), (pad_t + plot_h, pad_t));
    let base = pad_t + plot_h;
    let g = kit::id_safe(&format!("g-{}", o.id));

    // The columns the readout reads: day, value, average, previous.
    let mut series: Vec<(String, String)> =
        vec![(capitalise(o.unit), "var(--chart-other)".to_string())];
    if o.bars && !o.smooth_name.is_empty() {
        series.push((capitalise(o.smooth_name), "var(--chart-1)".to_string()));
    } else {
        series[0].1 = "var(--chart-1)".to_string();
    }
    let has_prior = !prior.is_empty();
    if has_prior {
        series.push(("Before".to_string(), "var(--chart-axis)".to_string()));
    }
    let label_of =
        |iso: &str| kit::civil_label(iso, 0).unwrap_or_else(|| (iso.to_string(), iso.to_string()));
    let cols: Vec<Col> = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let mut values = vec![if off(i) { None } else { Some(p.value) }];
            if o.bars && !o.smooth_name.is_empty() {
                values.push(p.smooth);
            }
            if has_prior {
                values.push(prior_at(i).and_then(line_of));
            }
            Col {
                label: label_of(&p.date).1,
                values,
                note: if off(i) {
                    Some("The station was not listening".to_string())
                } else {
                    prior_at(i).map(|q| format!("Compared with {}", label_of(&q.date).1))
                },
            }
        })
        .collect();

    // The line is drawn in runs of days the station was listening: on a day it was off there is
    // no value to draw, so the line stops there rather than diving to a zero it never measured.
    let mut runs: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
    for (i, p) in points.iter().enumerate() {
        match line_of(p).filter(|v| v.is_finite()) {
            Some(v) if !off(i) => runs.last_mut().expect("a run").push((x_at(i), y.at(v))),
            _ => {
                if runs.last().is_some_and(|r| !r.is_empty()) {
                    runs.push(Vec::new());
                }
            }
        }
    }
    runs.retain(|r| !r.is_empty());
    let ys: Vec<serde_json::Value> = points
        .iter()
        .map(|p| {
            line_of(p)
                .filter(|v| v.is_finite())
                .map_or(serde_json::Value::Null, |v| {
                    serde_json::json!((y.at(v) * 10.0).round() / 10.0)
                })
        })
        .collect();

    let mut svg = String::with_capacity(10 * 1024);
    let _ = write!(
        svg,
        r#"<div class="viz-chart ck"><div class="viz-scroll"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="viz-svg-block viz-trend ck-in" data-ck-id="{id}" data-ck-label="Left and right read each day."{attrs} data-ck-ys="{ys}">"#,
        id = escape_html(o.id),
        attrs = kit::cols_attrs(&cols, &series, x_at(0), x_at(n - 1), false, o.unit, None),
        ys = escape_html(&serde_json::Value::Array(ys).to_string()),
    );
    svg.push_str(&svg_a11y(
        "Daily trend",
        &format!(
            "{} per day, {n} days, as {}{}.",
            o.unit,
            if o.bars {
                "daily columns with a smoothed line over them"
            } else {
                "a line"
            },
            if has_prior {
                ", with the period before as a faint line"
            } else {
                ""
            },
        ),
    ));
    // The day columns fade from the neutral; a chart that is only a line has none.
    if o.bars {
        svg.push_str(&kit::gradient_defs(&g, &["ck-cother".to_string()]));
    }
    if (0..n).any(off) {
        let _ = write!(svg, "<defs>{}</defs>", kit::nodata_pattern(&g));
    }
    // Reverse the area's fade: strong at the line, gone at the baseline.
    let _ = write!(
        svg,
        r#"<defs><linearGradient id="{g}-area" class="ck-gr ck-c1" x1="0" y1="0" x2="0" y2="1"><stop class="a" offset="0" stop-opacity=".30"/><stop class="a" offset="1" stop-opacity="0"/></linearGradient></defs>"#
    );

    for &v in &rules {
        let yy = y.at(v);
        if v > 0.0 {
            let _ = write!(
                svg,
                r#"<line class="ck-grid" x1="{pad_l:.1}" y1="{yy:.1}" x2="{x2:.1}" y2="{yy:.1}"/>"#,
                x2 = pad_l + plot_w
            );
        }
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{tx:.1}" y="{ty:.1}" text-anchor="end" font-size="10">{l}</text>"#,
            tx = pad_l - 6.0,
            ty = yy + 3.5,
            l = kit::compact(v),
        );
    }
    for i in (0..n).filter(|&i| off(i)) {
        let _ = write!(
            svg,
            r#"<rect x="{x:.1}" y="{pad_t:.1}" width="{slot:.1}" height="{plot_h:.1}" fill="url(#{g}-nd)"/>"#,
            x = x_at(i) - slot / 2.0,
        );
    }

    // The days themselves, thin and neutral, when there is a smoothed line over them.
    if o.bars {
        let bw = (slot * 0.5).clamp(2.0, 14.0);
        svg.push_str("<g>");
        for (i, p) in points.iter().enumerate() {
            if p.value <= 0.0 || off(i) {
                continue;
            }
            let d = kit::bar_path(
                x_at(i) - bw / 2.0,
                y.at(p.value),
                bw,
                base - y.at(p.value),
                (bw / 2.0).min(3.0),
            );
            let _ = write!(
                svg,
                r#"<path class="ck-grow" d="{d}" fill="url(#{g}-ck-cother)" data-style="--i:{i}"/>"#
            );
        }
        svg.push_str("</g>");
    }
    let _ = write!(
        svg,
        r#"<line class="ck-axis" x1="{pad_l:.1}" y1="{base:.1}" x2="{x2:.1}" y2="{base:.1}"/>"#,
        x2 = pad_l + plot_w
    );

    // The period before, behind the line it is compared with.
    if has_prior {
        let pp: Vec<(f64, f64)> = (0..n)
            .filter_map(|i| {
                prior_at(i)
                    .and_then(line_of)
                    .filter(|v| v.is_finite())
                    .map(|v| (x_at(i), y.at(v)))
            })
            .collect();
        if pp.len() >= 2 {
            let _ = write!(
                svg,
                r#"<path class="ck-ghost" d="{}"/>"#,
                kit::monotone_path(&pp, false)
            );
        }
    }
    // Area, then line, one run at a time.
    for run in runs.iter().filter(|r| r.len() >= 2) {
        let curve = kit::monotone_path(run, false);
        let (xa, xb) = (run[0].0, run[run.len() - 1].0);
        let _ = write!(
            svg,
            r#"<path class="ck-area" d="{curve} L{xb:.1},{base:.1} L{xa:.1},{base:.1} Z" fill="url(#{g}-area)"/><path class="ck-line ck-c1 ck-drawn" d="{curve}" pathLength="1"/>"#,
        );
    }
    if !o.bars {
        for (i, p) in points.iter().enumerate() {
            if off(i) || !p.value.is_finite() {
                continue;
            }
            let _ = write!(
                svg,
                r#"<circle class="ck-ring ck-c1 ck-fadein" cx="{:.1}" cy="{:.1}" r="3"/>"#,
                x_at(i),
                y.at(p.value)
            );
        }
    }

    // The extremes of the *days* (not of the average), named where they happened.
    let counted: Vec<(usize, f64)> = points
        .iter()
        .enumerate()
        .filter(|(i, p)| !off(*i) && p.value.is_finite())
        .map(|(i, p)| (i, p.value))
        .collect();
    if counted.len() >= 3 {
        let hi = counted
            .iter()
            .copied()
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .unwrap_or(counted[0]);
        let lo = counted
            .iter()
            .copied()
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
            .unwrap_or(counted[0]);
        let mut placed: Vec<(f64, f64)> = Vec::new(); // (x_start, x_end) of labels, so two never overlap
        for (tag, (i, v), above) in [("High", hi, true), ("Low", lo, false)] {
            if (hi.1 - lo.1).abs() < f64::EPSILON && tag == "Low" {
                continue;
            }
            let text = format!(
                "{tag} {} \u{b7} {}",
                kit::compact(v),
                label_of(&points[i].date).0
            );
            let tw = kit::note_width(&text);
            let x = x_at(i);
            let (anchor, x0, x1) = if x + tw / 2.0 > pad_l + plot_w {
                ("end", x - tw, x)
            } else if x - tw / 2.0 < pad_l {
                ("start", x, x + tw)
            } else {
                ("middle", x - tw / 2.0, x + tw / 2.0)
            };
            let yy = if above {
                y.at(v) - 8.0
            } else {
                (y.at(v) + 16.0).min(base - 4.0)
            };
            if placed.iter().any(|&(a, b)| x0 < b + 6.0 && x1 > a - 6.0) && !above {
                continue; // the legend and the tooltip carry it
            }
            placed.push((x0, x1));
            let _ = write!(
                svg,
                r#"<circle class="ck-ring-annot ck-fadein" cx="{x:.1}" cy="{cy:.1}" r="3"/><text class="ck-note ck-halo ck-fadein" x="{x:.1}" y="{yy:.1}" text-anchor="{anchor}">{t}</text>"#,
                cy = y.at(v),
                t = escape_html(&text),
            );
        }
    }

    // First, middle and last date.
    for (n_i, i) in super::timeline::axis_columns(n, 6).into_iter().enumerate() {
        let (short, _) = label_of(&points[i].date);
        let cols_n = super::timeline::axis_columns(n, 6).len();
        let (anchor, x) = if n_i == 0 {
            ("start", pad_l)
        } else if n_i + 1 == cols_n {
            ("end", pad_l + plot_w)
        } else {
            ("middle", x_at(i))
        };
        let _ = write!(
            svg,
            r#"<text class="mono ck-lbl" x="{x:.1}" y="{y:.1}" text-anchor="{anchor}" font-size="10">{l}</text>"#,
            y = h - 6.0,
            l = escape_html(&short),
        );
    }
    // The crosshair and its marker.
    let _ = write!(
        svg,
        r#"<line class="ck-cursor" x1="{pad_l:.1}" y1="{pad_t:.1}" x2="{pad_l:.1}" y2="{base:.1}" visibility="hidden"/><circle class="ck-dot ck-c1" cx="{pad_l:.1}" cy="{base:.1}" r="4" visibility="hidden"/>"#,
    );
    svg.push_str("</svg></div>");

    // The key: what each mark is.
    svg.push_str(r#"<div class="ck-scale ck-trendkey">"#);
    if o.bars && !o.smooth_name.is_empty() {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta"><i class="ck-keyline ck-c1"></i> {}</span><span class="bnb-meta"><i class="ck-keybar"></i> Each day</span>"#,
            escape_html(o.smooth_name),
        );
    } else {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta"><i class="ck-keyline ck-c1"></i> {}</span>"#,
            escape_html(&capitalise(o.unit))
        );
    }
    if has_prior {
        svg.push_str(r#"<span class="bnb-meta"><i class="ck-keyline ck-keyghost"></i> The period before</span>"#);
    }
    if (0..n).any(off) {
        svg.push_str(r#"<span class="bnb-meta"><i class="ck-off" aria-hidden="true"></i> Not listening</span>"#);
    }
    svg.push_str("</div>");
    // Click-free text equivalent of the picture.
    let first = points.first().map_or(0.0, |p| p.value);
    let last = points.last().map_or(0.0, |p| p.value);
    let _ = write!(
        svg,
        r#"<p class="sr-only viz-readout">{}</p></div>"#,
        escape_html(&format!(
            "{} {} per day over {n} days, from {} on {} to {} on {}.",
            capitalise(o.unit),
            "",
            fmt(first),
            label_of(&points[0].date).1,
            fmt(last),
            label_of(&points[n - 1].date).1
        )),
    );
    svg
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_uppercase().collect::<String>() + c.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trend(values: &[f64]) -> Vec<TrendPoint> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| TrendPoint {
                date: format!("2026-10-{:02}", i + 6),
                value: *v,
                smooth: if i >= 2 { Some(*v - 1.0) } else { None },
            })
            .collect()
    }

    fn opts(prior: &[TrendPoint], bars: bool) -> TrendOpts<'_> {
        TrendOpts {
            id: "t",
            unit: if bars { "detections" } else { "species" },
            smooth_name: if bars { "7-day average" } else { "" },
            bars,
            prior,
            listened: None,
        }
    }

    /// The question is about a direction, so: an axis from zero with round rules, the
    /// first and last dates, a tooltip for every day, and the smoothed series as the one
    /// line in the chart's colour over neutral columns.
    #[test]
    fn the_trend_chart_has_an_axis_dates_and_one_coloured_line() {
        let svg = trend_chart(&trend(&[33.0, 31.0, 32.0, 34.0, 33.0]), &opts(&[], true));
        for label in [">0<", ">20<", ">40<"] {
            assert!(svg.contains(label), "axis {label}: {svg}");
        }
        for date in [">6 Oct<", ">10 Oct<"] {
            assert!(svg.contains(date), "{date}: {svg}");
        }
        assert!(svg.contains("&quot;Tue 6 Oct&quot;,[33.0,null]"), "{svg}");
        assert_eq!(
            svg.matches(r#"fill="url(#g-t-ck-cother)""#).count(),
            5,
            "five day columns: {svg}"
        );
        assert_eq!(
            svg.matches(r#"class="ck-line ck-c1 ck-drawn""#).count(),
            1,
            "one line: {svg}"
        );
        assert!(
            !svg.contains("oklch(") && !svg.contains("fill-opacity"),
            "{svg}"
        );
    }

    /// Without smoothing the series is the line, with a dot a day.
    #[test]
    fn a_trend_without_bars_is_a_line_with_a_dot_per_day() {
        let svg = trend_chart(&trend(&[13.0, 12.0, 14.0]), &opts(&[], false));
        assert!(!svg.contains("ck-cother"), "{svg}");
        assert_eq!(
            svg.matches(r#"<circle class="ck-ring ck-c1 ck-fadein""#)
                .count(),
            3,
            "a dot a day: {svg}"
        );
        assert!(trend_chart(&trend(&[1.0]), &opts(&[], true)).contains("Not enough data"));
    }

    /// The previous period is a ghost line behind the current one, aligned from the
    /// end, and the tooltip says which day each point is compared with.
    #[test]
    fn the_previous_period_is_a_ghost_aligned_day_for_day() {
        let now = trend(&[10.0, 12.0, 14.0, 16.0]);
        let before: Vec<TrendPoint> = (0..4)
            .map(|i| TrendPoint {
                date: format!("2026-09-{:02}", i + 20),
                value: 8.0 + f64::from(i),
                smooth: Some(8.0 + f64::from(i)),
            })
            .collect();
        let svg = trend_chart(&now, &opts(&before, true));
        assert_eq!(svg.matches(r#"class="ck-ghost""#).count(), 1, "{svg}");
        assert!(svg.contains("Compared with Sun 20 Sep"), "{svg}");
        assert!(svg.contains("The period before"), "{svg}");
        // A shorter prior lines up with the *latest* days; the earliest have nothing to compare to.
        let short = trend_chart(&now, &opts(&before[2..], true));
        assert!(
            short.contains("&quot;Tue 6 Oct&quot;,[10.0,null,null]]"),
            "no prior column for the first days: {short}"
        );
        assert!(short.contains("Compared with Tue 22 Sep"), "{short}");
    }

    /// The days' own extremes are named, once each, and a flat series names neither twice.
    #[test]
    fn the_busiest_and_quietest_days_are_named() {
        let svg = trend_chart(&trend(&[30.0, 12.0, 45.0, 20.0, 25.0]), &opts(&[], true));
        assert!(svg.contains("High 45 \u{b7} 8 Oct"), "{svg}");
        assert!(svg.contains("Low 12 \u{b7} 7 Oct"), "{svg}");
        let flat = trend_chart(&trend(&[5.0, 5.0, 5.0, 5.0]), &opts(&[], true));
        assert_eq!(
            flat.matches("High 5").count() + flat.matches("Low 5").count(),
            1,
            "{flat}"
        );
    }

    /// A day the station was off is hatched, has no value in its tooltip and says why: it is not a zero.
    #[test]
    fn a_day_the_station_was_off_is_not_a_zero() {
        let pts = trend(&[10.0, 0.0, 14.0, 16.0]);
        let listened = [true, false, true, true];
        let mut o = opts(&[], true);
        o.listened = Some(&listened);
        let svg = trend_chart(&pts, &o);
        assert!(svg.contains("&quot;Wed 7 Oct&quot;,[null,"), "{svg}");
        assert!(svg.contains("The station was not listening"), "{svg}");
        assert!(svg.contains("-nd)"), "{svg}");
    }

    /// The line stops where the station was off and starts again after it: it does not dive to a
    /// zero that was never measured and climb out of it.
    #[test]
    fn the_line_breaks_where_the_station_was_off() {
        let pts = trend(&[10.0, 12.0, 0.0, 14.0, 16.0]);
        let listened = [true, true, false, true, true];
        let mut o = opts(&[], false);
        o.listened = Some(&listened);
        let svg = trend_chart(&pts, &o);
        assert_eq!(
            svg.matches(r#"class="ck-line ck-c1 ck-drawn""#).count(),
            2,
            "two runs, not one line through a zero: {svg}"
        );
        // Nothing is drawn at the off day: not a dot, not a point of the line.
        assert_eq!(
            svg.matches(r#"<circle class="ck-ring ck-c1 ck-fadein""#)
                .count(),
            4,
            "{svg}"
        );
        // Whole days listened to: one line, as before.
        o.listened = None;
        assert_eq!(
            trend_chart(&pts, &o)
                .matches(r#"class="ck-line ck-c1 ck-drawn""#)
                .count(),
            1
        );
    }

    /// A huge or tiny day neither overflows the axis nor makes it unreadable.
    #[test]
    fn extreme_values_keep_the_axis_readable() {
        let svg = trend_chart(
            &trend(&[1_600_000.0, 3.0, 2.0, 900_000.0]),
            &opts(&[], true),
        );
        assert!(svg.contains(">2M<"), "{svg}");
        assert!(!svg.contains("NaN") && !svg.contains("inf"), "{svg}");
        let tiny = trend_chart(&trend(&[0.0, 0.0, 1.0]), &opts(&[], true));
        assert!(tiny.contains(">1<") && tiny.contains(">0<"), "{tiny}");
    }
}
