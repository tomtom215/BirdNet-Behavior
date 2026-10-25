//! SVG chart rendering helpers.
//!
//! Produces inline SVG for embedding directly in HTMX partial responses.
//! All functions return a `String` that can be inserted directly into HTML.

/// Render a column chart for hourly detection counts (0–23): a bar an hour on an axis from
/// zero, the busiest hour labelled, a tooltip for every hour (including the empty ones).
///
/// `id` names the chart on its page (its gradients and its entrance animation are keyed by
/// it, so two charts on one page need two ids). Returns a "no data" message if all counts
/// are zero. The drawing is [`super::viz::bars::hourly`].
pub(crate) fn render_hourly_chart(id: &str, hours: &[birdnet_db::sqlite::HourlyCount]) -> String {
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
    super::viz::bars::hourly(&super::viz::bars::Hourly {
        id,
        name: "Detections by hour of the day",
        desc: "Detections for each hour of the day; the busiest hour is labelled and every hour has a tooltip.",
        unit: "detections",
        counts,
        fill: super::viz::bars::Fill::Slot(0),
        width: 700.0,
        plot_h: 120.0,
        solar: None,
        now_h: None,
        top: None,
        temps: &[],
        table: false,
    })
}

/// Render a column chart for daily detection counts: a bar a day on an axis from zero, the
/// busiest day labelled, the average as a hairline, a tooltip for every day.
///
/// A run of days is drawn as a run of days: a day with no row is a zero between the first
/// date and `end` (today), not a gap the bars close over. `effort` is the station's
/// recording-effort ledger for those dates (see `pages::listening`): a day with no audio in
/// it is hatched and says so; `None`, or a ledger with nothing in the period, claims nothing.
pub(crate) fn render_daily_chart(
    id: &str,
    days: &[birdnet_db::sqlite::DailyCount],
    end: Option<&str>,
    effort: Option<&std::collections::BTreeMap<String, f64>>,
) -> String {
    daily_chart(id, days, end, effort, Layout::WIDE)
}

/// [`render_daily_chart`] drawn twice: the wide drawing for a card, and a narrower, taller one
/// for a phone. A drawing is scaled to its card, and the wide one is 700 units across: on a
/// 390 px phone that is 0.44 of a pixel a unit, a plot 48 px tall, labels the text-fitting
/// script has to double so that they stand on the axis and over the bars, and an average label
/// that was placed in units and is written across the tallest bar once enlarged. The narrow one
/// is 360 units, so it is drawn at about its own size on a phone and nothing is enlarged. CSS
/// (`.viz-chart--wide` / `.viz-chart--narrow`) shows one of them; the other is `display: none`,
/// which takes it out of the tab order and the accessibility tree.
///
/// `id` names the wide one; the narrow one is `{id}-narrow`.
pub(crate) fn render_daily_chart_pair(
    id: &str,
    days: &[birdnet_db::sqlite::DailyCount],
    end: Option<&str>,
    effort: Option<&std::collections::BTreeMap<String, f64>>,
) -> String {
    if days.is_empty() {
        return render_daily_chart(id, days, end, effort);
    }
    let narrow_id = format!("{id}-narrow");
    format!(
        "{}{}",
        daily_chart(
            id,
            days,
            end,
            effort,
            Layout::WIDE.shown(" viz-chart--wide")
        ),
        daily_chart(
            &narrow_id,
            days,
            end,
            effort,
            Layout::NARROW.shown(" viz-chart--narrow")
        ),
    )
}

/// How big a daily chart is drawn, in viewBox units, and what its outer `div` is called.
#[derive(Clone, Copy)]
struct Layout {
    width: f64,
    plot_h: f64,
    class: &'static str,
}

impl Layout {
    const WIDE: Self = Self {
        width: 700.0,
        plot_h: 110.0,
        class: "",
    };
    /// Close to the width of a card on a 390 px phone, so the scale is about one.
    const NARROW: Self = Self {
        width: 360.0,
        plot_h: 120.0,
        class: "",
    };

    const fn shown(self, class: &'static str) -> Self {
        Self { class, ..self }
    }
}

fn daily_chart(
    id: &str,
    days: &[birdnet_db::sqlite::DailyCount],
    end: Option<&str>,
    effort: Option<&std::collections::BTreeMap<String, f64>>,
    layout: Layout,
) -> String {
    if days.is_empty() {
        return r#"<p class="cht-empty">No detection data yet.</p>"#.to_string();
    }
    let filled = fill_days(days, end);
    let dates: Vec<String> = filled.iter().map(|d| d.0.clone()).collect();
    let today = dates.last().cloned().unwrap_or_default();
    let listened = effort.and_then(|e| super::listening::listened_on(e, &dates, &today));
    super::viz::bars::daily(&super::viz::bars::Daily {
        id,
        name: "Detections by day",
        desc: "Detections for each day; the busiest day is labelled, the average is a thin line and every day has a tooltip.",
        unit: "detections",
        days: &filled,
        slot: 0,
        width: layout.width,
        plot_h: layout.plot_h,
        listened: listened.as_deref(),
        mean_line: true,
        future_from: None,
        weekday_labels: false,
        table: false,
        class: layout.class,
    })
}

/// One `(date, count)` per calendar day from the first date to `end` (or the last date),
/// with zero for a day that has no row. Rows that are not dates are kept as they came.
pub(crate) fn fill_days(
    days: &[birdnet_db::sqlite::DailyCount],
    end: Option<&str>,
) -> Vec<(String, i64)> {
    use super::viz::kit::{epoch_day, shift_date};
    let first = days.first().and_then(|d| epoch_day(&d.date));
    let last_row = days.last().and_then(|d| epoch_day(&d.date));
    let last = end.and_then(epoch_day).into_iter().chain(last_row).max();
    let (Some(first), Some(last)) = (first, last) else {
        return days.iter().map(|d| (d.date.clone(), d.count)).collect();
    };
    // A station that has run for years must not be asked for a thousand bars by a bad row.
    if last - first > 400 {
        return days.iter().map(|d| (d.date.clone(), d.count)).collect();
    }
    let by_date: std::collections::HashMap<&str, i64> =
        days.iter().map(|d| (d.date.as_str(), d.count)).collect();
    (first..=last)
        .filter_map(|day| {
            let (y, m, d) = shift_date("1970-01-01", day)?;
            let iso = format!("{y:04}-{m:02}-{d:02}");
            let n = by_date.get(iso.as_str()).copied().unwrap_or(0);
            Some((iso, n))
        })
        .collect()
}

/// Render a horizontal bar chart for confidence distribution.
///
/// Buckets: `[0-50%, 50-60%, 60-70%, 70-80%, 80-90%, 90-100%]`.
pub(crate) fn render_confidence_chart(buckets: &[i64; 6], floor: f64) -> String {
    let total: i64 = buckets.iter().sum();
    if total == 0 {
        return r#"<p class="cht-empty">No detection data yet.</p>"#.to_string();
    }

    let labels = ["<50%", "50-60%", "60-70%", "70-80%", "80-90%", "90-100%"];
    // Coloured by the one confidence rule (`pages::confidence`), at the middle
    // of each bucket, and grey below the station's own cut-off. This used to be
    // a rainbow of UI colours (grey, the alarm red, two ambers, two greens) cut at
    // 50/60/70/80/90 whatever the station's cut-off was, so "70-80%" was amber
    // here and a different colour on every other page for the same bird, and the
    // alarm red was spent on detections the station had accepted.
    let cut = f64::from(crate::routes::pages::confidence::cutoff_percent(floor));
    let rows: Vec<super::viz::bars::Row<'_>> = [45.0_f64, 55.0, 65.0, 75.0, 85.0, 95.0]
        .iter()
        .zip(labels)
        .zip(buckets)
        .map(|((&mid, label), &count)| super::viz::bars::Row {
            label,
            count,
            fill: if mid < cut {
                "var(--fg-4)".to_string()
            } else {
                format!(
                    "var(--conf-{})",
                    crate::routes::pages::confidence::tier_of_percent(mid, floor).class()
                )
            },
        })
        .collect();
    super::viz::bars::distribution(
        "confidence",
        "Detections by confidence",
        "How many detections fall in each confidence band; bands below the station's cut-off are grey.",
        "detections",
        &rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_chart_empty() {
        let r = render_hourly_chart("t", &[]);
        assert!(r.contains("No detections today"));
    }

    #[test]
    fn daily_chart_empty() {
        let r = render_daily_chart("t", &[], None, None);
        assert!(r.contains("No detection data"));
    }

    #[test]
    fn confidence_chart_empty() {
        let r = render_confidence_chart(&[0; 6], 0.70);
        assert!(r.contains("No detection data"));
    }

    #[test]
    fn confidence_chart_all_labels() {
        let svg = render_confidence_chart(&[5, 10, 20, 30, 25, 15], 0.70);
        assert!(svg.contains("&lt;50%"), "{svg}");
        assert!(svg.contains("90-100%"));
    }

    /// The bars are the colours every other surface gives the same score, at
    /// the station's own cut-off, and grey below it. The chart used to spend
    /// the alarm red on the 50-60% bucket and paint 70-80% amber whatever the
    /// station accepted.
    #[test]
    fn the_confidence_chart_uses_the_one_confidence_rule() {
        let fills = |floor: f64| -> Vec<String> {
            render_confidence_chart(&[5, 10, 20, 30, 25, 15], floor)
                .split(r#"<path d=""#)
                .skip(1)
                .map(|s| {
                    let rest = s.split_once(r#" fill=""#).expect("a bar has a fill").1;
                    rest.split('"').next().unwrap_or("").to_string()
                })
                .collect()
        };
        // Cut-off 70: buckets mid 45/55/65 are below it; 75 is within ten points
        // of it (borderline); 85 is sure; 95 is very sure.
        assert_eq!(
            fills(0.70),
            [
                "var(--fg-4)",
                "var(--fg-4)",
                "var(--fg-4)",
                "var(--conf-low)",
                "var(--conf-mid)",
                "var(--conf-high)"
            ]
        );
        // Cut-off 50: borderline ends at 60, so 65 is no longer a worry.
        assert_eq!(
            fills(0.50),
            [
                "var(--fg-4)",
                "var(--conf-low)",
                "var(--conf-mid)",
                "var(--conf-mid)",
                "var(--conf-mid)",
                "var(--conf-high)"
            ]
        );
        let svg = render_confidence_chart(&[5, 10, 20, 30, 25, 15], 0.70);
        assert!(
            !svg.contains("var(--rare)"),
            "the alarm colour is not a bucket"
        );
    }

    /// The x-extent `(left, right)` of every painted bar: the first `M` and the last `L` of its path.
    fn bar_spans(svg: &str) -> Vec<(f64, f64)> {
        svg.split(r#"<path class="ck-grow" d=""#)
            .skip(1)
            .map(|frag| {
                let d = frag.split('"').next().expect("a path");
                let nums = |seg: &str| -> Vec<f64> {
                    seg.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                        .filter_map(|t| t.parse().ok())
                        .collect()
                };
                let xs: Vec<f64> = d
                    .split(['M', 'L', 'Q', ' '])
                    .filter(|t| t.contains(','))
                    .filter_map(|t| nums(t).first().copied())
                    .collect();
                (
                    xs.iter().copied().fold(f64::INFINITY, f64::min),
                    xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            })
            .collect()
    }

    fn view_box(svg: &str) -> (f64, f64, f64, f64) {
        let vb = svg
            .split_once("viewBox=\"")
            .expect("viewBox")
            .1
            .split_once('"')
            .expect("end")
            .0;
        let v: Vec<f64> = vb
            .split_whitespace()
            .map(|t| t.parse().expect("number"))
            .collect();
        (v[0], v[1], v[2], v[3])
    }

    fn daily(n: usize, f: impl Fn(usize) -> i64) -> Vec<birdnet_db::sqlite::DailyCount> {
        (0..n)
            .map(|i| birdnet_db::sqlite::DailyCount {
                date: super::super::viz::kit::shift_date(
                    "2026-01-01",
                    i64::try_from(i).unwrap_or(0),
                )
                .map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))
                .expect("a date"),
                count: f(i),
            })
            .collect()
    }

    /// Every bar `render_daily_chart` draws is inside the `viewBox` it declares, for one
    /// day and for ninety.
    ///
    /// Observed failing against the pre-fix renderer, which hard-coded `chart_w = 280`
    /// regardless of how many days it was handed while stepping `x` by 38 a day: the
    /// 14-day species chart lost its most recent week outside the box. Kept for the new
    /// renderer because the failure is one of arithmetic and could come back with any
    /// change to the slot width.
    #[test]
    fn daily_chart_bars_fit_inside_their_view_box() {
        for n in [1_usize, 2, 7, 14, 30, 90] {
            let days = daily(n, |i| i64::try_from(i % 17).unwrap_or(0) + 1);
            let svg = render_daily_chart("t", &days, None, None);
            let (vx, _, vw, _) = view_box(&svg);
            let spans = bar_spans(&svg);
            assert_eq!(spans.len(), n, "n = {n}: every day must draw a bar");
            for (i, (l, r)) in spans.iter().enumerate() {
                assert!(
                    *l >= vx && *r <= vx + vw,
                    "n = {n}: bar {i} spans {l}..{r} but the viewBox is {vx}..{}",
                    vx + vw
                );
            }
        }
    }

    /// The species page draws its 14-day chart twice, for a card and for a phone, and each
    /// drawing stands on its own: its bars are inside its own box, it has its own ids (a
    /// duplicated gradient or `data-ck-id` is a defect `qa.mjs` counts), and the narrow one is
    /// the narrower and the taller (it is there to be drawn at about its own size on a phone).
    ///
    /// Observed failing against the single wide drawing: with `render_daily_chart` in place of
    /// the pair there is no `viz-chart--narrow` to split on, and the first assertion fails.
    #[test]
    fn the_pair_is_two_whole_drawings_with_their_own_names() {
        let days = daily(14, |i| i64::try_from(i % 7).unwrap_or(0) + 1);
        let pair = render_daily_chart_pair("species-daily", &days, None, None);
        let (wide, narrow) = pair
            .split_once(r#"<div class="viz-chart ck viz-chart--narrow">"#)
            .expect("a narrow drawing after the wide one");
        assert!(wide.starts_with(r#"<div class="viz-chart ck viz-chart--wide">"#));
        assert!(wide.contains(r#"data-ck-id="species-daily""#));
        assert!(narrow.contains(r#"data-ck-id="species-daily-narrow""#));
        assert!(!narrow.contains(r#"data-ck-id="species-daily""#));
        // Every id the wide one defines is absent from the narrow one.
        for part in wide.split(r#" id=""#).skip(1) {
            let id = part.split('"').next().unwrap_or_default();
            assert!(
                !narrow.contains(&format!(r#" id="{id}""#)),
                "id {id:?} is defined twice on the page"
            );
        }
        for (name, one) in [("wide", wide), ("narrow", narrow)] {
            let (vx, _, vw, _) = view_box(one);
            let spans = bar_spans(one);
            assert_eq!(spans.len(), 14, "{name}: every day draws a bar");
            assert!(
                spans.iter().all(|(l, r)| *l >= vx && *r <= vx + vw),
                "{name}: a bar leaves its viewBox"
            );
        }
        let (_, _, ww, wh) = view_box(wide);
        let (_, _, nw, nh) = view_box(narrow);
        assert!(
            nw < ww && nh > wh,
            "narrow {nw}x{nh} against wide {ww}x{wh}"
        );
    }

    /// With nothing to draw the pair is the one sentence, not two.
    #[test]
    fn the_pair_of_an_empty_run_is_one_sentence() {
        let pair = render_daily_chart_pair("t", &[], None, None);
        assert_eq!(pair.matches("cht-empty").count(), 1);
    }

    /// The counterpart: proving bars *fit* is worthless if the renderer could satisfy it
    /// by drawing them on top of each other.
    #[test]
    fn daily_chart_bars_do_not_overlap() {
        let svg = render_daily_chart("t", &daily(14, |_| 5), None, None);
        let spans = bar_spans(&svg);
        assert_eq!(spans.len(), 14);
        for pair in spans.windows(2) {
            assert!(
                pair[1].0 >= pair[0].1,
                "bar at {} overlaps the one ending at {}",
                pair[1].0,
                pair[0].1
            );
        }
    }

    /// The busiest bar's number is inside the drawing, with room for its glyphs, however
    /// tall the bar. (The first version of this chart wrote it at y = -3, outside the box,
    /// so the busiest day, the one a reader looks for, was the one whose number never showed.)
    #[test]
    fn the_busiest_count_is_drawn_inside_the_chart() {
        for (name, svg) in [
            (
                "daily",
                render_daily_chart(
                    "t",
                    &daily(3, |i| i64::try_from(i).unwrap_or(0) + 1),
                    None,
                    None,
                ),
            ),
            (
                "hourly",
                render_hourly_chart(
                    "t",
                    &[
                        birdnet_db::sqlite::HourlyCount {
                            hour: "06".into(),
                            count: 2,
                        },
                        birdnet_db::sqlite::HourlyCount {
                            hour: "07".into(),
                            count: 7,
                        },
                    ],
                ),
            ),
        ] {
            let (_, vy, _, vh) = view_box(&svg);
            let label = svg
                .split("<text ")
                .find(|t| t.contains("ck-annot ck-halo"))
                .unwrap_or_else(|| panic!("{name}: the busiest is labelled"));
            let y: f64 = label
                .split("y=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .and_then(|v| v.parse().ok())
                .expect("label has a y");
            assert!(
                y - 10.0 >= vy && y <= vy + vh,
                "{name}: the label's baseline y = {y} leaves no room in {vy}..{}",
                vy + vh
            );
        }
    }

    /// A run of days is a run of days: a day with no row is a zero, and the run reaches today.
    #[test]
    fn days_without_a_row_are_zeros_and_the_run_reaches_today() {
        let rows = vec![
            birdnet_db::sqlite::DailyCount {
                date: "2026-10-15".into(),
                count: 4,
            },
            birdnet_db::sqlite::DailyCount {
                date: "2026-10-17".into(),
                count: 9,
            },
        ];
        let f = fill_days(&rows, Some("2026-10-19"));
        assert_eq!(
            f,
            vec![
                ("2026-10-15".to_string(), 4),
                ("2026-10-16".to_string(), 0),
                ("2026-10-17".to_string(), 9),
                ("2026-10-18".to_string(), 0),
                ("2026-10-19".to_string(), 0)
            ]
        );
        // An end date earlier than the data, or not a date, does not truncate or invent.
        assert_eq!(fill_days(&rows, Some("2026-10-01")).len(), 3);
        assert_eq!(fill_days(&rows, Some("not a date")).len(), 3);
    }

    /// A row that is not a date (an import) is carried as it came, not dropped.
    #[test]
    fn a_row_that_is_not_a_date_is_kept() {
        let rows = vec![birdnet_db::sqlite::DailyCount {
            date: "garbage".into(),
            count: 3,
        }];
        assert_eq!(
            fill_days(&rows, Some("2026-10-19")),
            vec![("garbage".to_string(), 3)]
        );
    }
}
