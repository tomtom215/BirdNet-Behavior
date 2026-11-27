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
//!
//! # The usual range (CHT-06)
//!
//! "Was today unusual?" needs a picture of usual, on the chart and not in a table two folds
//! away. The detections chart can draw a band behind the days: for each day, the 10th to
//! 90th percentile (nearest rank) of the [`RANGE_WINDOW`] listened days before it, from
//! [`expected_ranges`]; a day outside it is marked and its readout says which side. It is a
//! percentile band and not mean ± z·sd because daily counts are overdispersed: a z-score
//! over-flags a busy season. It is descriptive, not a confidence interval, and the chart says
//! so: about one ordinary day in five falls outside it by chance, and neighbouring days are
//! not independent (a run of days outside it is one event, not several). A day with fewer
//! than [`RANGE_MIN_DAYS`] listened days before it has no band: too few to say.
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
    /// The usual range of each day, from [`expected_ranges`]; `None` draws no band.
    pub expected: Option<&'a [Option<(f64, f64)>]>,
}

/// How many listened days before a day its usual range is made of.
pub const RANGE_WINDOW: usize = 30;

/// Fewer listened days than this before a day and it has no usual range: too few to say.
pub const RANGE_MIN_DAYS: usize = 14;

/// The usual range of each day of `values` from index `from` on: the 10th and 90th
/// percentile (nearest rank) of the last [`RANGE_WINDOW`] listened days strictly before it.
///
/// `listened` says which days count (`None`: all of them). A day that was not listened to
/// has no range, and is not part of any other day's; a day with fewer than
/// [`RANGE_MIN_DAYS`] listened days before it has none either.
#[must_use]
pub fn expected_ranges(
    values: &[f64],
    listened: Option<&[bool]>,
    from: usize,
) -> Vec<Option<(f64, f64)>> {
    let on = |i: usize| listened.is_none_or(|l| l.get(i).copied().unwrap_or(false));
    (from..values.len())
        .map(|i| {
            if !on(i) {
                return None;
            }
            let mut window: Vec<f64> = (0..i)
                .rev()
                .filter(|&j| on(j) && values[j].is_finite())
                .take(RANGE_WINDOW)
                .map(|j| values[j])
                .collect();
            if window.len() < RANGE_MIN_DAYS {
                return None;
            }
            window.sort_by(f64::total_cmp);
            Some((nearest_rank(&window, 0.10), nearest_rank(&window, 0.90)))
        })
        .collect()
}

/// The `q` quantile of a sorted sample by nearest rank: the `ceil(q n)`-th value.
fn nearest_rank(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    let k = ((q * n as f64).ceil() as usize).clamp(1, n);
    sorted[k - 1]
}

/// Which side of its usual range a day's value is on, if it is outside it.
fn outside(value: f64, range: Option<(f64, f64)>) -> Option<&'static str> {
    let (lo, hi) = range?;
    if value > hi {
        Some("Above")
    } else if value < lo {
        Some("Below")
    } else {
        None
    }
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
        .chain(o.expected.into_iter().flatten().flatten().map(|r| r.1))
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
        series.push((capitalise(o.smooth_name), "var(--chart-annot)".to_string()));
    } else {
        series[0].1 = "var(--chart-annot)".to_string();
    }
    let has_prior = !prior.is_empty();
    if has_prior {
        series.push(("Before".to_string(), "var(--chart-axis)".to_string()));
    }
    let label_of =
        |iso: &str| kit::civil_label(iso, 0).unwrap_or_else(|| (iso.to_string(), iso.to_string()));
    let expected_at = |i: usize| -> Option<(f64, f64)> {
        if off(i) {
            return None;
        }
        o.expected.and_then(|e| e.get(i).copied().flatten())
    };
    let has_band = (0..n).any(|i| expected_at(i).is_some());
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
            let range = expected_at(i);
            let note = if off(i) {
                Some("The station was not listening".to_string())
            } else {
                let side = outside(p.value, range).zip(range).map(|(side, (lo, hi))| {
                    format!("{side} the usual range ({} to {})", fmt(lo), fmt(hi))
                });
                let before = prior_at(i).map(|q| format!("Compared with {}", label_of(&q.date).1));
                match (side, before) {
                    (Some(a), Some(b)) => Some(format!("{a}. {b}")),
                    (a, b) => a.or(b),
                }
            };
            Col {
                label: label_of(&p.date).1,
                values,
                note,
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
        r#"<defs><linearGradient id="{g}-area" class="ck-gr ck-cink" x1="0" y1="0" x2="0" y2="1"><stop class="a" offset="0" stop-opacity=".30"/><stop class="a" offset="1" stop-opacity="0"/></linearGradient></defs>"#
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

    // The usual range, behind the days: one band per run of days that have one.
    if has_band {
        let mut run: Vec<(f64, f64, f64)> = Vec::new();
        let flush = |run: &mut Vec<(f64, f64, f64)>, svg: &mut String| {
            if !run.is_empty() {
                svg.push_str(&super::uncertainty::band(run, slot / 2.0));
                run.clear();
            }
        };
        for i in 0..n {
            match expected_at(i) {
                Some((lo, hi)) => run.push((x_at(i), y.at(lo), y.at(hi))),
                None => flush(&mut run, &mut svg),
            }
        }
        flush(&mut run, &mut svg);
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
            r#"<path class="ck-area" d="{curve} L{xb:.1},{base:.1} L{xa:.1},{base:.1} Z" fill="url(#{g}-area)"/><path class="ck-line ck-cink ck-drawn" d="{curve}" pathLength="1"/>"#,
        );
    }
    if !o.bars {
        for (i, p) in points.iter().enumerate() {
            if off(i) || !p.value.is_finite() {
                continue;
            }
            let _ = write!(
                svg,
                r#"<circle class="ck-ring ck-cink ck-fadein" cx="{:.1}" cy="{:.1}" r="3"/>"#,
                x_at(i),
                y.at(p.value)
            );
        }
    }

    // A day outside its usual range: a small open diamond at its value, over everything but
    // the names of the extremes. What it says is in the readout ("Above the usual range").
    for (i, p) in points.iter().enumerate() {
        if outside(p.value, expected_at(i)).is_some() {
            let (cx, cy) = (x_at(i), y.at(p.value));
            let _ = write!(
                svg,
                r#"<path class="ck-out" d="M{cx:.1},{t:.1} L{r:.1},{cy:.1} L{cx:.1},{b:.1} L{l:.1},{cy:.1} Z"/>"#,
                t = cy - 4.5,
                b = cy + 4.5,
                r = cx + 4.5,
                l = cx - 4.5,
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
        r#"<line class="ck-cursor" x1="{pad_l:.1}" y1="{pad_t:.1}" x2="{pad_l:.1}" y2="{base:.1}" visibility="hidden"/><circle class="ck-dot ck-cink" cx="{pad_l:.1}" cy="{base:.1}" r="4" visibility="hidden"/>"#,
    );
    svg.push_str("</svg></div>");

    // The key: what each mark is.
    svg.push_str(r#"<div class="ck-scale ck-trendkey">"#);
    if o.bars && !o.smooth_name.is_empty() {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta"><i class="ck-keyline ck-cink"></i> {}</span><span class="bnb-meta"><i class="ck-keybar"></i> Each day</span>"#,
            escape_html(o.smooth_name),
        );
    } else {
        let _ = write!(
            svg,
            r#"<span class="bnb-meta"><i class="ck-keyline ck-cink"></i> {}</span>"#,
            escape_html(&capitalise(o.unit))
        );
    }
    if has_prior {
        svg.push_str(r#"<span class="bnb-meta"><i class="ck-keyline ck-keyghost"></i> The period before</span>"#);
    }
    if has_band {
        svg.push_str(r#"<span class="bnb-meta"><i class="ck-keyband" aria-hidden="true"></i> Usual range</span><span class="bnb-meta"><i class="ck-keyout" aria-hidden="true"></i> Outside it</span>"#);
    }
    if (0..n).any(off) {
        svg.push_str(r#"<span class="bnb-meta"><i class="ck-off" aria-hidden="true"></i> Not listening</span>"#);
    }
    svg.push_str("</div>");
    if has_band {
        let flagged = (0..n)
            .filter(|&i| outside(points[i].value, expected_at(i)).is_some())
            .count();
        let judged = (0..n).filter(|&i| expected_at(i).is_some()).count();
        let _ = write!(
            svg,
            r#"<p class="bnb-meta ck-method">Usual range: the middle 80% (10th to 90th percentile) of the {RANGE_WINDOW} listened days before each day; a day with fewer than {RANGE_MIN_DAYS} before it has none. {flagged} of {judged} days fall outside it. It describes ordinary variation and is not a confidence interval: about one ordinary day in five falls outside it by chance, and neighbouring days are not independent, so a run of days outside it is one event, not several.</p>"#,
        );
    }
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

    /// N25-06: a trend is one total, not a species, so it wears no species' colour. It was
    /// `ck-cink` and `--chart-1`, the colour of the station's most heard bird.
    #[test]
    fn a_trend_wears_no_species_colour() {
        for bars in [true, false] {
            let svg = trend_chart(&trend(&[33.0, 31.0, 32.0, 34.0, 33.0]), &opts(&[], bars));
            assert!(svg.contains("ck-line"), "precondition: the line is drawn");
            assert_eq!(
                super::super::kit::species_colours_in(&svg),
                Vec::<String>::new()
            );
        }
    }

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
            expected: None,
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
            svg.matches(r#"class="ck-line ck-cink ck-drawn""#).count(),
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
            svg.matches(r#"<circle class="ck-ring ck-cink ck-fadein""#)
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
            svg.matches(r#"class="ck-line ck-cink ck-drawn""#).count(),
            2,
            "two runs, not one line through a zero: {svg}"
        );
        // Nothing is drawn at the off day: not a dot, not a point of the line.
        assert_eq!(
            svg.matches(r#"<circle class="ck-ring ck-cink ck-fadein""#)
                .count(),
            4,
            "{svg}"
        );
        // Whole days listened to: one line, as before.
        o.listened = None;
        assert_eq!(
            trend_chart(&pts, &o)
                .matches(r#"class="ck-line ck-cink ck-drawn""#)
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

    // -----------------------------------------------------------------------
    // CHT-06: the usual range, on the chart it is about
    // -----------------------------------------------------------------------

    /// A steady station: Poisson-like days around 30 (a fixed linear congruential sequence, so
    /// the test is the same every run), 120 of them, every one listened to.
    fn steady(n: usize) -> Vec<f64> {
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        (0..n)
            .map(|_| {
                // Sum of 30 coin flips weighted to a mean of 30: variance near the mean, as a
                // count's is.
                let mut v = 0.0;
                for _ in 0..60 {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    if (state >> 33).is_multiple_of(2) {
                        v += 1.0;
                    }
                }
                v
            })
            .collect()
    }

    /// CHT-06. On a steady station the band is the middle 80 % of the 30 listened days before
    /// each day, so about one day in five falls outside it by chance, and no more: the share
    /// flagged stays under a quarter. A planted day at five times the usual is flagged.
    #[test]
    fn a_steady_station_flags_about_its_nominal_share_and_a_planted_day_is_flagged() {
        let mut v = steady(120);
        v[100] = 150.0;
        let ranges = expected_ranges(&v, None, 0);
        assert_eq!(ranges.len(), 120);
        assert!(
            ranges[..RANGE_MIN_DAYS].iter().all(Option::is_none),
            "too few days before them to say"
        );
        assert!(ranges[RANGE_MIN_DAYS].is_some(), "and enough from then on");
        let judged: Vec<(f64, (f64, f64))> = v
            .iter()
            .zip(&ranges)
            .filter_map(|(x, r)| r.map(|r| (*x, r)))
            .collect();
        let outside = judged
            .iter()
            .filter(|(x, (lo, hi))| x < lo || x > hi)
            .count();
        let share = outside as f64 / judged.len() as f64;
        assert!(share <= 0.25, "{outside} of {} days flagged", judged.len());
        assert!(
            share >= 0.05,
            "a band that never misses is too wide: {share}"
        );
        let (lo, hi) = ranges[100].expect("a range");
        assert!(150.0 > hi && lo > 15.0, "{lo}..{hi}");
    }

    /// A day the station was off is not part of anyone's usual range: the window reaches back
    /// past an outage for its 30 listened days, and an off day gets no range of its own.
    #[test]
    fn the_usual_range_is_made_of_listened_days_only() {
        let mut v = vec![30.0; 80];
        let mut listened = vec![true; 80];
        for i in 40..60 {
            v[i] = 0.0;
            listened[i] = false;
        }
        let ranges = expected_ranges(&v, Some(&listened), 0);
        assert_eq!(ranges[50], None, "an off day has no range");
        assert_eq!(ranges[60], Some((30.0, 30.0)), "the zeros are not in it");
        // `from` keeps only the days shown.
        assert_eq!(expected_ranges(&v, Some(&listened), 50).len(), 30);
    }

    /// The band is drawn behind the days, a day outside it is marked and says which side, the
    /// key names the band, and the method is printed with its caveat. Without ranges there is
    /// none of it (counterpart).
    #[test]
    fn the_band_and_the_days_outside_it_are_drawn_and_named() {
        let pts = trend(&[30.0, 31.0, 29.0, 90.0, 30.0]);
        let ranges = [
            None,
            Some((25.0, 35.0)),
            Some((25.0, 35.0)),
            Some((25.0, 35.0)),
            Some((25.0, 35.0)),
        ];
        let mut o = opts(&[], true);
        o.expected = Some(&ranges);
        let svg = trend_chart(&pts, &o);
        assert_eq!(svg.matches(r#"<path class="ck-band""#).count(), 1, "{svg}");
        assert_eq!(
            svg.matches(r#"class="ck-out"#).count(),
            1,
            "one day outside"
        );
        assert!(svg.contains("Above the usual range (25 to 35)"), "{svg}");
        assert!(svg.contains("Usual range"));
        assert!(svg.contains("not a confidence interval"));
        let plain = trend_chart(&pts, &opts(&[], true));
        assert!(!plain.contains("ck-band") && !plain.contains("ck-out"));
        assert!(!plain.contains("Usual range"));
    }
}
