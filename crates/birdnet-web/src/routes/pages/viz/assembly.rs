//! The chorus assembling: how fast the dawn voices come together, as a field of discs.
//!
//! The question is "once the dawn starts, how long before it is a chorus?". A column is a time
//! allowed, from the first voice to the last; a row is how many of the station's usual dawn
//! voices (in their usual order) have joined; and each disc's *area* is the share of mornings
//! that had at least that many voices together within that time. So assembly reads as discs
//! growing up and to the right: the opener is a full row, and each row above it fills in later.
//!
//! The bold line round the cells is the typical morning: the most voices that at least half of
//! the mornings had, at each time allowed. The pale band is the middle half of mornings (from
//! the most that three in four had to the most that one in four had). A row is the colour of the
//! voice that completes it, the same positional colours the running order uses for its lanes.
//!
//! The depths are the extension's own: `window_funnel` asked at each of a ladder of windows
//! (`birdnet_behavioral::types::ASSEMBLY_WINDOWS`). Nothing is fitted or smoothed, and the
//! columns are the rungs of the ladder, evenly spaced, so no scale hides a stretch of time. The
//! step *times* of `window_funnel_events` are not used: it reports each step as the latest
//! occurrence of that voice before the next, so a gap between two of its steps depends on a
//! tie-break and not on the birds.
//!
//! Geometry-heavy SVG generation; the lint allows cover the pervasive, benign `f64`/`usize`
//! coordinate arithmetic.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
#![allow(clippy::many_single_char_names)]

use std::fmt::Write as _;

use super::heat::disc;
use super::kit::{self, slot_class, tip};
use super::{EMPTY, svg_a11y};

/// What the figure is made of: how many mornings had at least `k` voices within each window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chorus {
    /// The windows in minutes, strictly increasing.
    pub windows: Vec<u32>,
    /// How many voices are followed (the top row).
    pub voices: usize,
    /// How many mornings are counted: those on which the first voice was heard at all.
    pub mornings: usize,
    /// `reached[k - 1][i]`: how many counted mornings had at least `k` voices in order within
    /// `windows[i]`. Rises with `i` and falls with `k`; `reached[0]` is `mornings` throughout.
    pub reached: Vec<Vec<usize>>,
}

impl Chorus {
    /// The figure's input from the engine's depths, one row per morning. `None` when there is
    /// nothing to follow: fewer than two windows, fewer than two voices, or no counted morning.
    ///
    /// A morning counts when its first voice was heard in the hours looked at (any depth of one
    /// or more); a depth above `voices` is cut to it, and a row with the wrong number of depths
    /// is left out rather than guessed at.
    #[must_use]
    pub fn from_depths(windows: &[u32], voices: usize, mornings: &[Vec<u32>]) -> Option<Self> {
        if windows.len() < 2 || voices < 2 {
            return None;
        }
        let counted: Vec<Vec<usize>> = mornings
            .iter()
            .filter(|d| d.len() == windows.len())
            .map(|d| {
                d.iter()
                    .map(|&k| (k as usize).min(voices))
                    .collect::<Vec<_>>()
            })
            .filter(|d| d.iter().any(|&k| k >= 1))
            .collect();
        if counted.is_empty() {
            return None;
        }
        let reached = (1..=voices)
            .map(|k| {
                (0..windows.len())
                    .map(|i| counted.iter().filter(|d| d[i] >= k).count())
                    .collect()
            })
            .collect();
        Some(Self {
            windows: windows.to_vec(),
            voices,
            mornings: counted.len(),
            reached,
        })
    }

    /// The most voices that at least `num / den` of the mornings had within `windows[i]`.
    fn at_least(&self, i: usize, num: usize, den: usize) -> usize {
        (1..=self.voices)
            .rev()
            .find(|&k| self.reached[k - 1][i] * den >= self.mornings * num)
            .unwrap_or(0)
    }

    /// The typical morning's depth at `windows[i]`: the most voices that at least half of the
    /// mornings had.
    #[must_use]
    pub fn median(&self, i: usize) -> usize {
        self.at_least(i, 1, 2)
    }

    /// The lower edge of the middle half: the most voices that at least three in four mornings had.
    #[must_use]
    pub fn lower(&self, i: usize) -> usize {
        self.at_least(i, 3, 4)
    }

    /// The upper edge of the middle half: the most voices that at least one morning in four had.
    #[must_use]
    pub fn upper(&self, i: usize) -> usize {
        self.at_least(i, 1, 4)
    }

    /// The index of the first window by which at least half of the mornings had `k` voices.
    #[must_use]
    pub fn half_by(&self, k: usize) -> Option<usize> {
        let row = self.reached.get(k.checked_sub(1)?)?;
        row.iter().position(|&n| n * 2 >= self.mornings)
    }

    /// The share, 0 to 100, of mornings with at least `k` voices within `windows[i]`.
    #[must_use]
    pub fn percent(&self, k: usize, i: usize) -> f64 {
        self.reached[k - 1][i] as f64 * 100.0 / self.mornings as f64
    }
}

/// `5 min`, `1 h`, `1 h 30 min`: a window for a sentence or a tooltip.
#[must_use]
pub fn window_label(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The compact form of a rung under its column: `5m`, `1h`, `90m`.
fn rung_label(minutes: u32) -> String {
    if minutes >= 60 && minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}m")
    }
}

const W: f64 = 560.0;
const PAD_L: f64 = 66.0;
const PAD_R: f64 = 54.0;
const PAD_T: f64 = 42.0;
const PAD_B: f64 = 50.0;
const ROW: f64 = 38.0;

/// The geometry of one drawing, so a test can ask where a window or a depth falls.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    cols: usize,
    rows: usize,
}

impl Frame {
    const fn new(cols: usize, rows: usize) -> Self {
        Self { cols, rows }
    }

    fn height(self) -> f64 {
        PAD_T + self.rows as f64 * ROW + PAD_B
    }

    fn base(self) -> f64 {
        self.height() - PAD_B
    }

    fn col_w(self) -> f64 {
        (W - PAD_L - PAD_R) / self.cols as f64
    }

    /// The left edge of column `i`.
    #[must_use]
    pub fn left(self, i: usize) -> f64 {
        PAD_L + i as f64 * self.col_w()
    }

    /// The middle of column `i`.
    #[must_use]
    pub fn x(self, i: usize) -> f64 {
        self.left(i) + self.col_w() / 2.0
    }

    /// The top edge of row `k` (1 is the row on the baseline; zero is the baseline itself).
    #[must_use]
    pub fn top(self, k: usize) -> f64 {
        self.base() - k.min(self.rows) as f64 * ROW
    }

    /// The middle of row `k`.
    #[must_use]
    pub fn y(self, k: usize) -> f64 {
        self.top(k) + ROW / 2.0
    }

    /// The largest disc: it leaves a margin inside its cell, so neighbours never touch.
    #[must_use]
    pub fn r_max(self) -> f64 {
        self.col_w().min(ROW) / 2.0 - 2.5
    }
}

/// A disc's radius for `share` (0 to 1) of the mornings: area follows the share, so a quarter of
/// the mornings is half the radius. Nothing is drawn for none, and a few are never smaller than
/// something that can be seen and pointed at.
fn radius(frame: Frame, share: f64) -> Option<f64> {
    (share > 0.0).then(|| (frame.r_max() * share.clamp(0.0, 1.0).sqrt()).max(2.4))
}

/// The corners of the contour at the top of row `rows[i]` in every column: along the cell, then
/// down or up to the next column's level, so the line follows the cell edges.
fn contour(frame: Frame, levels: &[usize]) -> Vec<(f64, f64)> {
    let mut pts = Vec::with_capacity(levels.len() * 2);
    for (i, &k) in levels.iter().enumerate() {
        let y = frame.top(k);
        pts.push((frame.left(i), y));
        pts.push((frame.left(i + 1), y));
    }
    pts
}

/// A path along `pts`, as an open line.
fn line(pts: &[(f64, f64)]) -> String {
    let mut d = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(d, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" });
    }
    d
}

/// The figure: a field of discs, the typical morning in bold, the middle half as a band, the
/// share of mornings that ever got to each level at the right. `names` are the voices in the
/// order they join, for the readings and the description.
#[must_use]
pub fn chorus_figure(c: &Chorus, names: &[String]) -> String {
    if c.windows.len() < 2 || c.voices < 2 || c.mornings == 0 || c.reached.len() != c.voices {
        return EMPTY.to_string();
    }
    let n = c.windows.len();
    let frame = Frame::new(n, c.voices);
    let (h, base, right) = (frame.height(), frame.base(), W - PAD_R);
    let g = kit::id_safe("chorus-assembling");

    // The first cell the keyboard lands on: where the typical morning first reaches its top.
    let top = c.median(n - 1);
    let landing = (0..n).find(|&i| c.median(i) == top).unwrap_or(0);
    let start = (c.voices - top.max(1)) * n + landing;

    let mut s = format!(
        r#"<div class="ck as-fig"><svg viewBox="0 0 {W} {h:.0}" width="100%" role="img" class="as-svg ck-in" data-ck="marks" data-ck-id="{g}" data-ck-grid="{n}" data-ck-start="{start}" data-ck-label="Left and right move between times allowed, up and down between numbers of voices.">"#,
    );
    s.push_str(&svg_a11y(
        "How fast the chorus comes together",
        &format!(
            "A grid of discs. Each column is a time allowed, from {} to {}; each row is how many of the station's usual dawn voices ({}) were heard, in that order, within that time. A disc's area is the share of {} mornings that had at least that many. The same numbers are in the table below.",
            window_label(c.windows[0]),
            window_label(c.windows[n - 1]),
            names.join(", "),
            c.mornings,
        ),
    ));
    let classes: Vec<String> = (0..c.voices).map(slot_class).collect();
    let _ = write!(
        s,
        "<defs>{}</defs>",
        kit::halo_gradients(&format!("{g}-hg"), &classes)
    );

    // Row labels, and a rule between rows: the rows are the thing the eye follows.
    for k in 1..=c.voices {
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{PAD_L}" y1="{y:.1}" x2="{right:.1}" y2="{y:.1}"/><text class="ck-lbl as-row" x="{lx:.1}" y="{ty:.1}" text-anchor="end">{k} {noun}</text>"#,
            y = frame.top(k),
            lx = PAD_L - 8.0,
            ty = frame.y(k) + 4.0,
            noun = if k == 1 { "voice" } else { "voices" },
        );
    }
    let _ = write!(
        s,
        r#"<line class="ck-axis" x1="{PAD_L}" y1="{base:.1}" x2="{right:.1}" y2="{base:.1}"/>"#
    );
    for (i, &w) in c.windows.iter().enumerate() {
        let _ = write!(
            s,
            r#"<text class="ck-lbl mono" x="{x:.1}" y="{ty:.1}" text-anchor="middle">{label}</text>"#,
            x = frame.x(i),
            ty = base + 16.0,
            label = rung_label(w),
        );
    }
    let _ = write!(
        s,
        r#"<text class="ck-lbl as-axis" x="{mid:.1}" y="{ty:.1}" text-anchor="middle">time allowed from the first voice to the last</text><text class="ck-lbl as-axis" x="{gx:.1}" y="{ay:.1}">by {last}</text>"#,
        mid = f64::midpoint(PAD_L, right),
        ty = h - 8.0,
        gx = right + 8.0,
        ay = PAD_T - 8.0,
        last = window_label(c.windows[n - 1]),
    );

    // The middle half of mornings: from the bottom of its lowest row to the top of its highest, in each column.
    let upper: Vec<usize> = (0..n).map(|i| c.upper(i)).collect();
    let lower: Vec<usize> = (0..n).map(|i| c.lower(i)).collect();
    let mut band = contour(frame, &upper);
    let mut under: Vec<(f64, f64)> = (0..n)
        .flat_map(|i| {
            let y = frame.top(lower[i].saturating_sub(1));
            [(frame.left(i), y), (frame.left(i + 1), y)]
        })
        .collect();
    under.reverse();
    band.extend(under);
    let _ = write!(s, r#"<path class="as-band" d="{} Z"/>"#, line(&band));

    // The field: one disc per cell, its halo under it, the typical morning's contour over them.
    let (mut halos, mut discs, mut hits) = (String::new(), String::new(), String::new());
    // Top row first, left to right: the order of the indices the arrow keys walk.
    for k in (1..=c.voices).rev() {
        for i in 0..n {
            let idx = (c.voices - k) * n + i;
            let class = slot_class(k - 1);
            let share = c.reached[k - 1][i] as f64 / c.mornings as f64;
            let (cx, cy) = (frame.x(i), frame.y(k));
            if let Some(r) = radius(frame, share) {
                disc(
                    &mut halos,
                    &mut discs,
                    &format!("{g}-hg"),
                    &class,
                    (cx, cy),
                    r,
                    idx,
                );
            } else {
                let _ = write!(
                    discs,
                    r#"<circle class="ck-none" cx="{cx:.1}" cy="{cy:.1}" r="1.2" data-ck-i="{idx}"/>"#
                );
            }
            let who = names
                .iter()
                .take(k)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" \u{2192} ");
            let reading = tip(
                &format!(
                    "{k} {} within {}",
                    if k == 1 { "voice" } else { "voices" },
                    window_label(c.windows[i])
                ),
                &[
                    (
                        "Mornings",
                        &format!(
                            "{} of {} ({:.0}%)",
                            c.reached[k - 1][i],
                            c.mornings,
                            c.percent(k, i)
                        ),
                        None,
                    ),
                    ("Typical morning", &format!("{} voices", c.median(i)), None),
                ],
                (!who.is_empty()).then_some(who.as_str()),
            );
            let _ = write!(
                hits,
                r#"<rect class="ck-hit" x="{x:.1}" y="{y:.1}" width="{cw:.1}" height="{ROW}" data-tip="{reading}" data-ck-i="{idx}"/>"#,
                x = frame.left(i),
                y = frame.top(k),
                cw = frame.col_w(),
            );
        }
    }
    s.push_str(&halos);
    s.push_str(&discs);

    let median: Vec<usize> = (0..n).map(|i| c.median(i)).collect();
    let _ = write!(
        s,
        r#"<path class="as-med" d="{}"/>"#,
        line(&contour(frame, &median))
    );

    // The share of mornings that ever got to each level, at the right: the funnel in one column.
    for k in 1..=c.voices {
        let _ = write!(
            s,
            r#"<text class="ck-lbl mono as-share" x="{x:.1}" y="{y:.1}">{pct:.0}%</text>"#,
            x = right + 8.0,
            y = frame.y(k) + 4.0,
            pct = c.percent(k, n - 1),
        );
    }

    // One note, in the margin, for the typical morning's last climb, only where it fits.
    if top >= 2 {
        let text = format!(
            "typical morning: {top} voices in {}",
            window_label(c.windows[landing])
        );
        let (px, py) = (frame.left(landing), frame.top(top));
        let nw = kit::note_width(&text);
        // In the margin above the grid, with a leader down the edge of the column where the line
        // climbs: a disc never reaches a column's edge, so the leader crosses none of them, and the
        // words are never written over a mark.
        let ny = PAD_T - 16.0;
        if px - 10.0 - nw >= PAD_L {
            let _ = write!(
                s,
                r#"<path class="ck-lead" d="M{px:.1},{y0:.1} V{y1:.1} H{lx:.1}"/><text class="ck-note" x="{tx:.1}" y="{ny:.1}" text-anchor="end">{t}</text>"#,
                y0 = py - 2.0,
                y1 = ny + 3.0,
                lx = px - 6.0,
                tx = px - 10.0,
                t = crate::routes::pages::escape_html(&text),
            );
        }
    }

    s.push_str(&hits);
    s.push_str("</svg></div>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows() -> Vec<u32> {
        vec![1, 2, 5, 10, 30, 60]
    }

    fn names() -> Vec<String> {
        vec!["Robin".into(), "Blackbird".into(), "Wren".into()]
    }

    /// Four mornings, three voices.
    fn four() -> Chorus {
        Chorus::from_depths(
            &windows(),
            3,
            &[
                vec![1, 2, 3, 3, 3, 3], // quick: all three within 5 min
                vec![1, 1, 2, 3, 3, 3], // all three within 10
                vec![1, 1, 1, 1, 2, 3], // all three within 60
                vec![1, 1, 1, 1, 1, 1], // only the opener, ever
            ],
        )
        .expect("four mornings")
    }

    fn radii(svg: &str) -> Vec<f64> {
        svg.split(r#"<circle class="ck-disc "#)
            .skip(1)
            .filter_map(|c| {
                let r = c.split(" r=\"").nth(1)?;
                r.split('"').next()?.parse().ok()
            })
            .collect()
    }

    #[test]
    fn a_morning_counts_when_the_first_voice_was_heard_and_a_row_of_the_wrong_length_does_not() {
        let c = Chorus::from_depths(
            &windows(),
            3,
            &[
                vec![0; 6], // none of the voices: not a morning of the chorus
                vec![1; 6],
                vec![1, 2, 2, 2, 2, 9], // a depth above the top is cut to it
                vec![1, 2],             // the wrong number of windows: left out
            ],
        )
        .expect("two counted");
        assert_eq!(c.mornings, 2);
        assert_eq!(
            c.reached[2],
            vec![0, 0, 0, 0, 0, 1],
            "the cut depth is three"
        );
        assert!(Chorus::from_depths(&windows(), 1, &[vec![1; 6]]).is_none());
        assert!(Chorus::from_depths(&[5], 3, &[vec![1]]).is_none());
        assert!(Chorus::from_depths(&windows(), 3, &[]).is_none());
    }

    /// `reached` is the funnel: it falls as the number of voices asked for rises, and it rises as
    /// the allowance does. The first row is every counted morning at every window.
    #[test]
    fn the_counts_rise_with_the_window_and_fall_with_the_voices() {
        let c = four();
        assert_eq!(c.mornings, 4);
        assert_eq!(c.reached[0], vec![4; 6], "the opener is every morning");
        assert_eq!(c.reached[1], vec![0, 1, 2, 2, 3, 3]);
        assert_eq!(c.reached[2], vec![0, 0, 1, 2, 2, 3]);
        for k in 1..c.voices {
            for i in 0..c.windows.len() {
                assert!(c.reached[k][i] <= c.reached[k - 1][i], "k={k} i={i}");
                if i > 0 {
                    assert!(c.reached[k][i] >= c.reached[k][i - 1], "k={k} i={i}");
                }
            }
        }
    }

    /// The typical morning is the most voices that at least half the mornings had; the middle
    /// half is between the most that three in four had and the most that one in four had.
    #[test]
    fn the_typical_morning_and_the_middle_half_are_the_quantiles_they_say() {
        let c = four();
        // At 10 min: depths are 3, 3, 1, 1.
        assert_eq!(c.median(3), 3, "half have all three");
        assert_eq!(c.lower(3), 1, "three in four only all have one");
        assert_eq!(c.upper(3), 3);
        assert_eq!(
            (c.lower(0), c.median(0), c.upper(0)),
            (1, 1, 1),
            "at 1 min nobody has two"
        );
        assert_eq!(c.half_by(2), Some(2), "within 5 min: 2 of 4");
        assert_eq!(c.half_by(3), Some(3));
        // One morning in four is not half.
        let few = Chorus::from_depths(
            &windows(),
            3,
            &[vec![1, 1, 1, 1, 1, 3], vec![1; 6], vec![1; 6], vec![1; 6]],
        )
        .expect("rows");
        assert_eq!(few.half_by(3), None);
        assert!((few.percent(3, 5) - 25.0).abs() < 1e-9);
    }

    /// For every window the three are in order, whatever the data: the band is never inside out.
    #[test]
    fn the_middle_half_always_holds_the_typical_morning() {
        for seed in 0..40_u64 {
            let rows: Vec<Vec<u32>> = (0..9_u64)
                .map(|m| {
                    let mut d = 1;
                    (0..6)
                        .map(|i| {
                            let r = (seed * 31 + m * 17 + i * 7) % 5;
                            if r < 2 && d < 4 {
                                d += 1;
                            }
                            d
                        })
                        .collect()
                })
                .collect();
            let c = Chorus::from_depths(&windows(), 4, &rows).expect("rows");
            for i in 0..6 {
                assert!(
                    c.lower(i) <= c.median(i) && c.median(i) <= c.upper(i),
                    "seed {seed} i {i}"
                );
            }
        }
    }

    #[test]
    fn a_window_is_said_in_minutes_and_hours() {
        assert_eq!(window_label(1), "1 min");
        assert_eq!(window_label(45), "45 min");
        assert_eq!(window_label(60), "1 h");
        assert_eq!(window_label(90), "1 h 30 min");
        assert_eq!(window_label(300), "5 h");
        assert_eq!(rung_label(60), "1h");
        assert_eq!(rung_label(5), "5m");
        assert_eq!(rung_label(90), "90m");
    }

    /// Columns are equal bands edge to edge; row 1 sits on the baseline and each row above is one
    /// step higher; and nothing is wider than its cell.
    #[test]
    fn the_cells_tile_the_plot() {
        let f = Frame::new(14, 6);
        assert!((f.left(0) - PAD_L).abs() < 1e-9);
        assert!((f.left(14) - (W - PAD_R)).abs() < 1e-9);
        assert!((f.left(5) - f.left(4) - f.col_w()).abs() < 1e-9);
        assert!(
            (f.top(0) - f.base()).abs() < 1e-9,
            "zero voices is the baseline"
        );
        assert!((f.top(1) - f.base() + ROW).abs() < 1e-9);
        assert!(f.top(6) < f.top(5), "more voices are higher");
        assert!(
            (f.top(9) - f.top(6)).abs() < 1e-9,
            "never above the top row"
        );
        assert!(
            f.r_max() * 2.0 < f.col_w().min(ROW),
            "a disc leaves room round it"
        );
        assert!((f.height() - (PAD_T + 6.0 * ROW + PAD_B)).abs() < 1e-9);
    }

    /// Area is the share: a quarter of the mornings is half the radius of all of them, none is
    /// no disc, and a few are still a disc you can see.
    #[test]
    fn a_discs_area_is_the_share_of_mornings() {
        let f = Frame::new(6, 3);
        let full = radius(f, 1.0).expect("a disc");
        assert!((full - f.r_max()).abs() < 1e-9);
        assert!((radius(f, 0.25).expect("a disc") - full / 2.0).abs() < 1e-9);
        assert!(radius(f, 0.0).is_none());
        assert!(
            (radius(f, 0.0001).expect("a disc") - 2.4).abs() < 1e-9,
            "a few are still visible"
        );
        let svg = chorus_figure(&four(), &names());
        let r = radii(&svg);
        // Row 1 is every morning at every window: six full discs.
        assert_eq!(
            r.iter()
                .filter(|&&x| (x - f_of(&four()).r_max()).abs() < 0.06)
                .count(),
            6,
            "{r:?}"
        );
        // A quarter of the mornings (1 of 4) at 2 min for two voices, half the radius.
        assert!(
            r.iter()
                .any(|&x| (x - f_of(&four()).r_max() / 2.0).abs() < 0.06),
            "{r:?}"
        );
        // No share, no disc: three voices within 1 min is nobody, a small mark and not a disc.
        assert!(svg.contains(r#"class="ck-none""#));
    }

    fn f_of(c: &Chorus) -> Frame {
        Frame::new(c.windows.len(), c.voices)
    }

    /// The bold line sits at the top of the highest row that at least half the mornings reached,
    /// in every column.
    #[test]
    fn the_typical_contour_is_where_half_the_mornings_stop() {
        let c = four();
        let f = f_of(&c);
        let svg = chorus_figure(&c, &names());
        let d = svg
            .split(r#"<path class="as-med" d=""#)
            .nth(1)
            .and_then(|r| r.split('"').next())
            .expect("a contour");
        let pts: Vec<(f64, f64)> = d
            .split(['M', 'L'])
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (x, y) = p.split_once(',').expect("a pair");
                (x.parse().expect("x"), y.parse().expect("y"))
            })
            .collect();
        assert_eq!(pts.len(), 12, "two corners a column");
        for i in 0..6 {
            let want = f.top(c.median(i));
            assert!(
                (pts[2 * i].1 - want).abs() < 0.06 && (pts[2 * i + 1].1 - want).abs() < 0.06,
                "column {i}"
            );
            assert!(
                (pts[2 * i].0 - f.left(i)).abs() < 0.06
                    && (pts[2 * i + 1].0 - f.left(i + 1)).abs() < 0.06
            );
        }
    }

    #[test]
    fn everything_drawn_is_inside_the_picture() {
        let c = four();
        let svg = chorus_figure(&c, &names());
        let h = f_of(&c).height();
        for (attr, hi) in [
            ("x", W),
            ("x1", W),
            ("x2", W),
            ("cx", W),
            ("y", h),
            ("y1", h),
            ("y2", h),
            ("cy", h),
        ] {
            for r in svg.split(&format!(" {attr}=\"")).skip(1) {
                let v: f64 = r
                    .split('"')
                    .next()
                    .and_then(|t| t.parse().ok())
                    .expect("number");
                assert!((0.0..=hi).contains(&v), "{attr}={v}");
            }
        }
        for d in svg
            .split(" d=\"")
            .skip(1)
            .filter_map(|r| r.split('"').next())
        {
            for tok in d
                .split(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
                .filter(|t| !t.is_empty())
            {
                let v: f64 = tok.parse().expect("number");
                assert!((0.0..=W.max(h)).contains(&v), "path value {v} in {d}");
            }
        }
    }

    /// One reading for every cell, each saying how many mornings it is and what the typical
    /// morning is at that time; every index once, top row first, so the arrows walk it.
    #[test]
    fn every_cell_has_a_reading_and_an_index_the_arrows_can_walk() {
        let svg = chorus_figure(&four(), &names());
        assert_eq!(
            svg.matches(r#"class="ck-hit""#).count(),
            18,
            "3 rows x 6 windows"
        );
        assert!(
            svg.contains(r#"data-ck-grid="6""#),
            "a line of six: up and down move between rows"
        );
        let idx: Vec<usize> = svg
            .split(r#"class="ck-hit""#)
            .skip(1)
            .map(|c| {
                let r = c.split("data-ck-i=\"").nth(1).expect("an index");
                r.split('"').next().expect("v").parse().expect("n")
            })
            .collect();
        assert_eq!(
            idx,
            (0..18).collect::<Vec<_>>(),
            "top row first, left to right"
        );
        // Three voices within 10 min: 2 of 4 mornings, a typical morning of 3 voices, and who they are.
        assert!(svg.contains("3 voices within 10 min"), "{svg}");
        assert!(svg.contains("2 of 4 (50%)"), "{svg}");
        assert!(
            svg.contains("Robin \u{2192} Blackbird \u{2192} Wren"),
            "{svg}"
        );
        // The keyboard lands where the typical morning first reaches its top (3 voices at 10 min).
        assert!(svg.contains(r#"data-ck-start="3""#), "{svg}");
        // The right-hand column: the share that ever got to each level, by the widest window.
        assert!(svg.contains(">100%<") && svg.contains(">75%<"), "{svg}");
        // The baseline is the axis, drawn once.
        assert_eq!(svg.matches(r#"class="ck-axis""#).count(), 1);
    }

    /// A row is the colour of the voice that completes it, and each colour has the halo that
    /// belongs to it.
    #[test]
    fn a_row_is_the_colour_of_the_voice_that_completes_it() {
        let svg = chorus_figure(&four(), &names());
        for k in 1..=3 {
            assert!(
                svg.contains(&format!(r#"class="ck-disc ck-c{k}""#)),
                "row {k}"
            );
            assert!(
                svg.contains(&format!(r#"id="chorus-assembling-hg-ck-c{k}""#)),
                "halo {k}"
            );
        }
        assert!(!svg.contains("ck-c4"), "only as many colours as voices");
    }

    /// The note is written only where it fits left of the rise, and says the same as the numbers.
    #[test]
    fn the_note_names_the_typical_morning_and_is_left_out_when_there_is_no_room() {
        let ladder = [1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300];
        // Every morning has all three voices from the hour on: the rise is far enough along.
        let late: Vec<Vec<u32>> = (0..6)
            .map(|_| {
                ladder
                    .iter()
                    .enumerate()
                    .map(|(i, _)| if i >= 9 { 3 } else { 1 })
                    .collect()
            })
            .collect();
        let c = Chorus::from_depths(&ladder, 3, &late).expect("rows");
        let svg = chorus_figure(&c, &names());
        assert!(svg.contains("typical morning: 3 voices in 1 h"), "{svg}");
        // In the margin above the grid, never over a mark: its baseline is above the first row.
        let note_y: f64 = svg
            .split(r#"class="ck-note" x=""#)
            .nth(1)
            .and_then(|r| r.split("y=\"").nth(1))
            .and_then(|r| r.split('"').next())
            .and_then(|v| v.parse().ok())
            .expect("the note's y");
        assert!(note_y < f_of(&c).top(c.voices), "{note_y}");
        // Only the opener, ever: nothing to say in the margin.
        let flat = Chorus::from_depths(&windows(), 3, &[vec![1; 6], vec![1; 6]]).expect("rows");
        assert!(!chorus_figure(&flat, &names()).contains("ck-note"));
        // The typical morning has them all from the first column: no room to its left.
        let brisk = Chorus::from_depths(&windows(), 3, &[vec![3; 6], vec![3; 6]]).expect("rows");
        assert!(!chorus_figure(&brisk, &names()).contains("ck-note"));
        // A rise too early to leave room for the words is not squeezed in.
        let early = Chorus::from_depths(
            &windows(),
            3,
            &[vec![1, 3, 3, 3, 3, 3], vec![1, 3, 3, 3, 3, 3]],
        )
        .expect("rows");
        assert!(!chorus_figure(&early, &names()).contains("ck-note"));
    }

    #[test]
    fn nothing_to_draw_is_the_placeholder_and_names_are_escaped() {
        let empty = Chorus {
            windows: windows(),
            voices: 3,
            mornings: 0,
            reached: vec![],
        };
        assert_eq!(chorus_figure(&empty, &[]), EMPTY);
        let svg = chorus_figure(&four(), &["<b>x</b>".into(), "B".into(), "C".into()]);
        assert!(!svg.contains("<b>x"), "{svg}");
        assert!(svg.contains("&lt;b&gt;x&lt;/b&gt;"));
    }

    /// Six voices and the whole ladder: the largest the card draws, and still every mark inside it.
    #[test]
    fn the_largest_figure_is_still_inside_its_picture() {
        let ladder = [1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300];
        let rows: Vec<Vec<u32>> = (0..40_u32)
            .map(|m| {
                ladder
                    .iter()
                    .enumerate()
                    .map(|(i, _)| 1 + ((i as u32 + m) / 3).min(5))
                    .collect()
            })
            .collect();
        let c = Chorus::from_depths(&ladder, 6, &rows).expect("rows");
        let svg = chorus_figure(
            &c,
            &(1..=6).map(|k| format!("Voice {k}")).collect::<Vec<_>>(),
        );
        assert_eq!(svg.matches(r#"class="ck-hit""#).count(), 6 * 14);
        let h = f_of(&c).height();
        assert!(
            svg.contains(&format!("viewBox=\"0 0 {W} {h:.0}\"")),
            "{svg}"
        );
        assert!(svg.contains("ck-c6"));
    }
}
