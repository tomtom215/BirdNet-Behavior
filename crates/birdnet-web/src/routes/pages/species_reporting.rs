//! `GET /pages/species-reporting?name=…`: on what share of the days the station listened was
//! this species heard, through the year (CHT-16 / G-01; the "birder's bar chart").
//!
//! It lives on the species page, beside "Through the year", because it answers that page's
//! question about one bird ("in which weeks can I expect it?") and needs the species named; the
//! Patterns tabs compare species with each other. The calendar above it counts detections per
//! day; this divides by the listening, so a week the station was off is not a week the bird was
//! away, and one loud morning is one day, not a hundred detections.
//!
//! The figure: 48 parts of the year (four a month, `birdnet_behavioral::reporting`), each with
//! the share of its listened days on which the species was heard as a dot, its 95 % Wilson
//! interval as a quiet bar behind the dot, a ring for a part with too few listened days to say
//! (`viz::uncertainty`), and a hatch where the station did not listen at all. It is drawn twice,
//! for a card and for a phone, and carries a table of every number, a one-sentence reading
//! above it and a Method panel below.

#[cfg(feature = "analytics")]
use axum::extract::{Query, State};
use axum::{Router, routing::get};
#[cfg(feature = "analytics")]
use serde::Deserialize;

use crate::state::AppState;

/// The query: the species' stored common name.
#[cfg(feature = "analytics")]
#[derive(Deserialize)]
pub(super) struct ReportingQuery {
    name: Option<String>,
}

/// The route this partial answers.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/species-reporting", get(species_reporting_partial))
}

#[cfg(not(feature = "analytics"))]
async fn species_reporting_partial() -> axum::response::Html<String> {
    axum::response::Html(UNAVAILABLE.to_owned())
}

/// What the card says on a station without the analytics engine.
const UNAVAILABLE: &str = r#"<p class="cht-empty">This figure is worked out by the analytics engine, which isn't switched on for this station.</p>"#;

#[cfg(feature = "analytics")]
async fn species_reporting_partial(
    State(state): State<AppState>,
    Query(query): Query<ReportingQuery>,
) -> impl axum::response::IntoResponse {
    use axum::http::{StatusCode, header};
    let ok = |html: String| (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html);
    let Some(name) = query.name else {
        return ok("<p>No species specified.</p>".to_owned());
    };
    if !state.has_analytics() {
        return ok(UNAVAILABLE.to_owned());
    }
    let names = state.names_handle();
    let stored = name.clone();
    let floor = birdnet_timeseries::queries::trend::EFFORT_FLOOR_SHARE;
    let result = state
        .run_blocking(move |s| s.with_analytics_read(|adb| adb.reporting_frequency(&stored, floor)))
        .await;
    match result {
        None => ok(UNAVAILABLE.to_owned()),
        Some(Ok(rf)) => ok(render::figure(&rf, names.name_of_stored(&name), floor)),
        Some(Err(e)) => {
            tracing::warn!(error = %e, "species reporting frequency: query failed");
            super::error_states::failed_partial("how often this species is heard through the year")
        }
    }
}

#[cfg(feature = "analytics")]
pub mod render {
    //! The figure, its reading, its table and its Method panel, as pure functions of the counts.

    use std::fmt::Write as _;

    use birdnet_behavioral::reporting::{
        PERIODS_PER_YEAR, ReportingFrequency, ReportingPeriod, period_label,
    };

    use super::super::escape_html;
    use super::super::viz::uncertainty::{
        self, Interval, interval_words, percent, too_few, too_few_words,
    };
    use super::super::viz::{kit, svg_a11y};

    /// What `n` counts, singular and plural.
    const UNIT: (&str, &str) = ("listened day", "listened days");

    /// The month names on the axis.
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    /// The id the figure's table, gradients and kit identity are built from.
    const ID: &str = "sp-report";

    /// What one part of the year is, for drawing and saying.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum Cell {
        /// The station listened on no day of it: no value.
        NotListening,
        /// It listened on `n` days, fewer than the floor, and heard the bird on `k`.
        TooFew {
            /// Days heard.
            k: u32,
            /// Days listened.
            n: u32,
        },
        /// Heard on `k` of `n` listened days, with the share's interval.
        Share {
            /// Days heard.
            k: u32,
            /// Days listened.
            n: u32,
            /// The Wilson interval of `k` of `n`.
            ci: Interval,
        },
    }

    impl Cell {
        /// The cell for one period's counts, if any.
        pub fn of(p: Option<&ReportingPeriod>) -> Self {
            match p {
                None => Self::NotListening,
                Some(p) if p.listened_days == 0 => Self::NotListening,
                Some(p) if too_few(p.listened_days) => Self::TooFew {
                    k: p.heard_days,
                    n: p.listened_days,
                },
                Some(p) => Interval::share(p.heard_days, p.listened_days).map_or(
                    Self::NotListening,
                    |ci| Self::Share {
                        k: p.heard_days,
                        n: p.listened_days,
                        ci,
                    },
                ),
            }
        }
    }

    /// The 48 cells of the year.
    pub fn cells(rf: &ReportingFrequency) -> Vec<Cell> {
        (0..PERIODS_PER_YEAR)
            .map(|i| Cell::of(rf.period(i)))
            .collect()
    }

    /// The part of the year the reading names: of the parts with enough listening, the one
    /// whose interval's lower bound is highest (the share the data are surest is high, not
    /// the one that happens to be highest on three days), the earliest on a tie.
    pub fn surest(cells: &[Cell]) -> Option<(usize, u32, u32, Interval)> {
        let mut best: Option<(usize, u32, u32, Interval)> = None;
        for (i, c) in cells.iter().enumerate() {
            if let Cell::Share { k, n, ci } = *c
                && best.is_none_or(|(_, _, _, b)| ci.lo > b.lo + 1e-12)
            {
                best = Some((i, k, n, ci));
            }
        }
        best
    }

    /// The one-sentence reading above the figure, as authored markup (names escaped): the
    /// whole year's days heard of days listened, then the part of the year the data are surest
    /// it is heard in, then how many parts with enough listening it was not heard in at all.
    pub fn reading(cells: &[Cell]) -> String {
        let judged: Vec<(u32, u32)> = cells
            .iter()
            .filter_map(|c| match *c {
                Cell::Share { k, n, .. } => Some((k, n)),
                _ => None,
            })
            .collect();
        let Some((i, k, n, ci)) = surest(cells) else {
            return "Too few listened days in any part of the year to say when it is heard."
                .to_owned();
        };
        if k == 0 {
            return format!(
                "Not heard on any listened day in the {} parts of the year with enough listening to say.",
                judged.len()
            );
        }
        let (all_k, all_n) = cells.iter().fold((0_u32, 0_u32), |(a, b), c| match *c {
            Cell::Share { k, n, .. } | Cell::TooFew { k, n } => (a + k, b + n),
            Cell::NotListening => (a, b),
        });
        let silent = judged.iter().filter(|(k, _)| *k == 0).count();
        let mut s = format!(
            "Heard on {all_k} of {all_n} listened days ({}); most surely {}, on {k} of {n} ({})",
            percent(f64::from(all_k) / f64::from(all_n.max(1))),
            escape_html(&period_label(i)),
            interval_words(ci),
        );
        if silent > 0 {
            let _ = write!(
                s,
                "; not heard at all in {silent} of the {} parts of the year with enough listening to say.",
                judged.len()
            );
        } else {
            s.push('.');
        }
        s
    }

    /// A drawing's dimensions, in viewBox units.
    struct Frame {
        w: f64,
        h: f64,
        left: f64,
        right: f64,
        top: f64,
        /// The plot's baseline (0 %).
        base: f64,
        /// The strip under the plot where a "too few" ring sits, on no scale.
        few_y: f64,
        /// The month labels' baseline.
        month_y: f64,
        ci_w: f64,
        dot_r: f64,
        font: f64,
        /// Suffix for ids, so the card's and the phone's drawing never share one.
        tag: &'static str,
        class: &'static str,
    }

    const WIDE: Frame = Frame {
        w: 700.0,
        h: 236.0,
        left: 40.0,
        right: 8.0,
        top: 10.0,
        base: 190.0,
        few_y: 202.0,
        month_y: 226.0,
        ci_w: 3.0,
        dot_r: 3.8,
        font: 11.0,
        tag: "w",
        class: " viz-chart--wide",
    };

    const NARROW: Frame = Frame {
        w: 360.0,
        h: 226.0,
        left: 34.0,
        right: 4.0,
        top: 10.0,
        base: 180.0,
        few_y: 192.0,
        month_y: 216.0,
        ci_w: 2.0,
        dot_r: 2.6,
        font: 10.0,
        tag: "n",
        class: " viz-chart--narrow",
    };

    /// A period's index as a coordinate. Indices are below 48, so nothing is lost.
    fn at(i: usize) -> f64 {
        f64::from(u8::try_from(i).unwrap_or(u8::MAX))
    }

    impl Frame {
        fn col_w(&self) -> f64 {
            (self.w - self.left - self.right) / at(PERIODS_PER_YEAR)
        }
        fn x(&self, i: usize) -> f64 {
            (at(i) + 0.5).mul_add(self.col_w(), self.left)
        }
        fn y(&self, share: f64) -> f64 {
            share
                .clamp(0.0, 1.0)
                .mul_add(self.top - self.base, self.base)
        }
    }

    /// What the kit says for one part of the year.
    fn tip(i: usize, c: Cell) -> String {
        let label = period_label(i);
        match c {
            Cell::NotListening => kit::tip(&label, &[], Some("The station was not listening")),
            Cell::TooFew { k, n } => {
                let heard = format!("{k} of {n}");
                kit::tip(
                    &label,
                    &[("Days heard", &heard, None)],
                    Some(&too_few_words(n, UNIT)),
                )
            }
            Cell::Share { k, n, ci } => {
                let heard = format!("{k} of {n} listened days");
                let share = percent(f64::from(k) / f64::from(n));
                let range = format!("{} to {}", percent(ci.lo), percent(ci.hi));
                kit::tip(
                    &label,
                    &[
                        ("Heard", &heard, None),
                        ("Share", &share, None),
                        ("95% interval", &range, None),
                    ],
                    None,
                )
            }
        }
    }

    /// One drawing of the figure. `name` is text; [`svg_a11y`] escapes it.
    fn drawing(cells: &[Cell], f: &Frame, name: &str, start: usize) -> String {
        let id = format!("{ID}-{}", f.tag);
        let mut s = String::with_capacity(24 * 1024);
        let _ = write!(
            s,
            r#"<div class="viz-chart ck{class}"><svg width="100%" viewBox="0 0 {w:.0} {h:.0}" role="img" class="viz-svg-block ck-in" data-ck="marks" data-ck-id="{id}" data-ck-dim="index" data-ck-start="{start}" data-ck-label="Left and right read each part of the year.">"#,
            class = f.class,
            w = f.w,
            h = f.h,
        );
        s.push_str(&svg_a11y(
            &format!("How often {name} is heard through the year"),
            "For each quarter of each month, the share of the days the station listened on which this species was heard, with its 95% interval; a ring where there were too few listened days to say, and a hatch where the station did not listen.",
        ));
        let _ = write!(s, "<defs>{}</defs>", kit::nodata_pattern(&id));
        // Rules at 0, 25, 50, 75 and 100 % with their labels.
        for q in 0..=4 {
            let share = f64::from(q) / 4.0;
            let y = f.y(share);
            let _ = write!(
                s,
                r#"<line class="{cls}" x1="{x0:.1}" y1="{y:.1}" x2="{x1:.1}" y2="{y:.1}"/><text class="mono ck-lbl" x="{lx:.1}" y="{ty:.1}" text-anchor="end" font-size="{fs}">{t}</text>"#,
                cls = if q == 0 { "ck-axis" } else { "ck-grid" },
                x0 = f.left,
                x1 = f.w - f.right,
                lx = f.left - 5.0,
                ty = y + f.font * 0.35,
                fs = f.font,
                t = percent(share),
            );
        }
        // Month boundaries and names.
        for (m, month) in MONTHS.iter().enumerate() {
            let x0 = f.x(m * 4) - f.col_w() / 2.0;
            if m > 0 {
                let _ = write!(
                    s,
                    r#"<line class="ck-grid" x1="{x0:.1}" y1="{t:.1}" x2="{x0:.1}" y2="{b:.1}"/>"#,
                    t = f.top,
                    b = f.base,
                );
            }
            let _ = write!(
                s,
                r#"<text class="ck-lbl" x="{x:.1}" y="{y:.1}" text-anchor="middle" font-size="{fs}">{month}</text>"#,
                x = x0 + 2.0 * f.col_w(),
                y = f.month_y,
                fs = f.font,
            );
        }
        // The marks: hatch, interval, estimate, ring; each under a hit box with its reading.
        let cw = f.col_w();
        for (i, c) in cells.iter().enumerate() {
            let x = f.x(i);
            match *c {
                Cell::NotListening => {
                    let _ = write!(
                        s,
                        r#"<rect x="{x0:.1}" y="{t:.1}" width="{w:.1}" height="{h:.1}" fill="url(#{id}-nd)" data-ck-i="{i}"/>"#,
                        x0 = x - cw / 2.0,
                        t = f.top,
                        w = cw,
                        h = f.base - f.top,
                    );
                }
                Cell::TooFew { .. } => {
                    s.push_str(&uncertainty::few_mark(x, f.few_y, f.dot_r, i));
                }
                Cell::Share { k, n, ci } => {
                    s.push_str(&uncertainty::whisker(x, f.y(ci.lo), f.y(ci.hi), f.ci_w, i));
                    s.push_str(&uncertainty::estimate(
                        x,
                        f.y(f64::from(k) / f64::from(n)),
                        f.dot_r,
                        i,
                    ));
                }
            }
            let _ = write!(
                s,
                r#"<rect class="ck-hit" x="{x0:.1}" y="{t:.1}" width="{w:.1}" height="{h:.1}" data-ck-i="{i}" data-tip="{tip}"/>"#,
                x0 = x - cw / 2.0,
                t = f.top,
                w = cw,
                h = f.few_y + f.dot_r + 2.0 - f.top,
                tip = tip(i, *c),
            );
        }
        s.push_str("</svg></div>");
        s
    }

    /// The table that carries every number the drawing does.
    fn table(cells: &[Cell]) -> String {
        let mut s = format!(
            r#"<details class="viz-table"><summary>Show as a table</summary><div class="viz-table-scroll"><table id="{ID}-data"><thead><tr><th scope="col">Part of the year</th><th scope="col" class="num">Listened days</th><th scope="col" class="num">Days heard</th><th scope="col" class="num">Share</th><th scope="col">95% interval</th></tr></thead><tbody>"#
        );
        for (i, c) in cells.iter().enumerate() {
            let label = escape_html(&period_label(i));
            match *c {
                Cell::NotListening => {
                    let _ = write!(
                        s,
                        r#"<tr><th scope="row">{label}</th><td class="num">0</td><td class="num">—</td><td class="num">—</td><td>not listening</td></tr>"#
                    );
                }
                Cell::TooFew { k, n } => {
                    let _ = write!(
                        s,
                        r#"<tr><th scope="row">{label}</th><td class="num">{n}</td><td class="num">{k}</td><td class="num">—</td><td>too few to say</td></tr>"#
                    );
                }
                Cell::Share { k, n, ci } => {
                    let _ = write!(
                        s,
                        r#"<tr><th scope="row">{label}</th><td class="num">{n}</td><td class="num">{k}</td><td class="num">{share}</td><td>{lo} to {hi}</td></tr>"#,
                        share = percent(f64::from(k) / f64::from(n)),
                        lo = percent(ci.lo),
                        hi = percent(ci.hi),
                    );
                }
            }
        }
        s.push_str("</tbody></table></div></details>");
        s
    }

    /// How long the typical day is, said in hours.
    fn hours(seconds: f64) -> String {
        format!("{:.1} hours", seconds / 3600.0)
    }

    /// The whole card body for `rf`. `name` is the species as the station shows it; `floor`
    /// the listened-day floor as a share of the typical day.
    pub fn figure(rf: &ReportingFrequency, name: &str, floor: f64) -> String {
        if !rf.has_ledger {
            return r#"<p class="cht-empty">The station keeps no listening ledger yet, so there is nothing to divide by: a share of the days it listened cannot be worked out.</p>"#.to_owned();
        }
        if rf.heard_days == 0 {
            return r#"<p class="cht-empty">Never heard by this station, so there is no year to draw for it.</p>"#.to_owned();
        }
        if rf.periods.is_empty() {
            return format!(
                r#"<p class="cht-empty">The listening ledger holds no day with at least {} of the station's typical day recorded, so there is nothing to divide by yet.</p>"#,
                percent(floor)
            );
        }
        let cells = cells(rf);
        let start = surest(&cells).map_or(0, |(i, ..)| i);
        let mut s = super::super::frame::takeaway(&reading(&cells));
        s.push_str(&drawing(&cells, &WIDE, name, start));
        s.push_str(&drawing(&cells, &NARROW, name, start));
        s.push_str(&uncertainty::key("listened days"));
        s.push_str(
            r#"<p class="bnb-meta ck-unc-key"><span class="ck-unc-item"><span class="ck-unc-sw ck-unc-sw-off" aria-hidden="true"></span>hatched: the station was not listening</span></p>"#,
        );
        s.push_str(&table(&cells));
        let years = match (&rf.first_listened, &rf.last_listened) {
            (Some(a), Some(b)) if rf.years > 1 => format!(
                "{a} to {b}; where that holds a part of the year more than once, the years' listened days are added together"
            ),
            (Some(a), Some(b)) => format!("{a} to {b}"),
            _ => "none".to_owned(),
        };
        let typical = rf
            .typical_seconds
            .map_or_else(|| "not known".to_owned(), hours);
        let asked = [
            ("Species", name.to_owned()),
            (
                "A listened day",
                format!(
                    "a date with at least {} of the station's typical day of audio in the listening ledger (typical day: {typical}, summed over microphones)",
                    percent(floor)
                ),
            ),
            (
                "Parts of the year",
                "four a month: days 1–7, 8–14, 15–21 and 22 to the month's end".to_owned(),
            ),
            ("Listened days counted", years),
            (
                "Days heard but not counted",
                format!(
                    "{} of {} (the ledger does not count them as listened)",
                    rf.heard_days_not_counted, rf.heard_days
                ),
            ),
        ];
        s.push_str(&super::super::frame::method(&super::super::frame::Method {
            computed: "Plain SQL over the analytics copy of the detections and the listening ledger; no extension function. The interval is the Wilson score interval, worked out on the page.",
            asked: &asked,
            note: "Each dot is the share of the listened days in that part of the year on which this species was detected at least once; detections rejected in review are left out. The bar behind it is that share's 95% Wilson interval, which treats each listened day as a separate trial. Days in a row are alike (a bird here today is likely here tomorrow), so the bar is the narrowest the uncertainty can be, not its full width; it is at its most honest where several years are pooled. Fewer than three listened days: too few to say, drawn as a ring with no value; no listened day: hatched, never zero. Days before the ledger began are left out of both sides, not counted as silent. Years are pooled, which assumes the season repeats. A share of days is not abundance: one bird singing daily and fifty read the same. A false positive pushes the share up: one misidentified detection makes its day a day heard, which is a seventh of a seven-day part of the year, and matters most where the true share is near zero. The reading names the part whose interval starts highest, not the highest dot.",
            json: &[],
            csv: &[("Download the table as CSV", "sp-report-data", "reporting-frequency")],
        }));
        s
    }
}

#[cfg(all(test, feature = "analytics"))]
mod tests {
    use birdnet_behavioral::reporting::{ReportingFrequency, ReportingPeriod};

    use super::render::{Cell, cells, figure, reading};

    fn rf(periods: &[(usize, u32, u32)]) -> ReportingFrequency {
        ReportingFrequency {
            has_ledger: true,
            periods: periods
                .iter()
                .map(|&(period, listened_days, heard_days)| ReportingPeriod {
                    period,
                    listened_days,
                    heard_days,
                })
                .collect(),
            heard_days: periods.iter().map(|p| p.2).sum::<u32>().max(1),
            heard_days_not_counted: 0,
            first_listened: Some("2025-01-01".to_owned()),
            last_listened: Some("2025-12-31".to_owned()),
            years: 1,
            typical_seconds: Some(86_400.0),
        }
    }

    /// The text of the table row for `label`.
    fn row<'a>(html: &'a str, label: &str) -> &'a str {
        let at = html
            .find(&format!(r#"<th scope="row">{label}</th>"#))
            .unwrap_or_else(|| panic!("a row for {label}"));
        let end = html[at..].find("</tr>").expect("row end") + at;
        &html[at..end]
    }

    /// Three days heard of seven listened reads 3 of 7, 43 %, with the Wilson bounds worked by
    /// hand in `birdnet_behavioral::interval` (0.1582198, 0.7495416): 16 % to 75 %.
    #[test]
    fn three_of_seven_reads_three_of_seven_with_its_wilson_bounds() {
        let r = rf(&[(16, 7, 3)]); // 1–7 May
        match cells(&r)[16] {
            Cell::Share { k, n, ci } => {
                assert_eq!((k, n), (3, 7));
                assert!((ci.lo - 0.158_219_8).abs() < 5e-7, "{ci:?}");
                assert!((ci.hi - 0.749_541_6).abs() < 5e-7, "{ci:?}");
            }
            other => panic!("1–7 May is a share: {other:?}"),
        }
        let html = figure(&r, "Robin", 0.5);
        let tr = row(&html, "1–7 May");
        assert!(
            tr.contains(r#"<td class="num">7</td><td class="num">3</td><td class="num">43%</td><td>16% to 75%</td>"#),
            "{tr}"
        );
        assert!(
            html.contains("Heard&#9;3 of 7 listened days"),
            "the tip says it"
        );
        assert!(
            html.contains("95% interval&#9;16% to 75%"),
            "the tip says the bounds"
        );
    }

    /// A part of the year with no listening has no value: not a dot at zero, not "0%", but a
    /// hatch and the words. Counterpart: a part listened to and never heard *is* 0%, with an
    /// interval, because that is a finding.
    #[test]
    fn no_listening_shows_no_value_and_a_silent_listened_week_shows_zero() {
        let r = rf(&[(16, 7, 3), (17, 7, 0)]);
        let html = figure(&r, "Robin", 0.5);
        let off = row(&html, "8–14 Apr");
        assert!(
            off.contains("not listening") && !off.contains("0%"),
            "{off}"
        );
        assert!(
            html.contains("8–14 Apr&#10;The station was not listening"),
            "the tip says the station was off"
        );
        let silent = row(&html, "8–14 May");
        assert!(
            silent.contains(r#"<td class="num">0%</td><td>0% to 35%</td>"#),
            "{silent}"
        );
    }

    /// Fewer than three listened days: words, a ring, no value. Counterpart: three is drawn.
    #[test]
    fn two_listened_days_are_too_few_and_three_are_not() {
        let r = rf(&[(16, 2, 2), (20, 3, 3)]);
        let html = figure(&r, "Robin", 0.5);
        let few = row(&html, "1–7 May");
        assert!(
            few.contains("too few to say") && !few.contains("100%"),
            "{few}"
        );
        assert!(html.contains("2 listened days: too few to say"));
        let three = row(&html, "1–7 Jun");
        assert!(three.contains("100%"), "{three}");
        assert_eq!(
            html.matches(r#"class="ck-few""#).count(),
            2,
            "one ring per drawing"
        );
    }

    /// The documented empty states, each in words and none a picture of zeros.
    #[test]
    fn a_species_never_heard_has_the_documented_empty_state() {
        let mut never = rf(&[(16, 7, 0)]);
        never.heard_days = 0;
        let html = figure(&never, "Robin", 0.5);
        assert!(html.contains("Never heard by this station"), "{html}");
        assert!(!html.contains("<svg"), "no figure for a bird never heard");
        let mut none = rf(&[]);
        none.has_ledger = false;
        assert!(figure(&none, "Robin", 0.5).contains("no listening ledger"));
    }

    /// The reading names the part whose interval starts highest: 7 of 7 (lower bound 65 %)
    /// over 3 of 3 (lower bound 44 %), though both are 100 %.
    #[test]
    fn the_reading_names_the_surest_part_not_the_luckiest() {
        let r = rf(&[(16, 3, 3), (20, 7, 7), (30, 7, 0)]);
        let s = reading(&cells(&r));
        assert!(
            s.starts_with("Heard on 10 of 17 listened days (59%); most surely 1–7 Jun, on 7 of 7"),
            "{s}"
        );
        assert!(s.contains("not heard at all in 1 of the 3 parts"), "{s}");
    }
}
