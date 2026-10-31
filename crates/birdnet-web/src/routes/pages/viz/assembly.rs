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

/// The proportions of one drawing. The figure is drawn twice, 560 units across for a card and 360
/// for a phone, where the first would be about 0.6 of a pixel a unit and every caption would be
/// enlarged onto its neighbour (or cut: "6 voices" became "6. voi…"); CSS shows one.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    w: f64,
    pad_l: f64,
    pad_r: f64,
    pad_t: f64,
    pad_b: f64,
    row: f64,
    narrow: bool,
}

/// The drawing for a card.
const WIDE: Layout = Layout {
    w: 560.0,
    pad_l: 66.0,
    pad_r: 54.0,
    pad_t: 42.0,
    pad_b: 50.0,
    row: 38.0,
    narrow: false,
};

/// The drawing for a phone: row numbers alone, a label under only some of the columns, and the
/// shorter words.
const NARROW: Layout = Layout {
    w: 360.0,
    pad_l: 24.0,
    pad_r: 42.0,
    pad_t: 46.0,
    pad_b: 42.0,
    row: 34.0,
    narrow: true,
};

/// The rungs that carry a label on the phone's drawing (the others are still columns, and still read).
const NARROW_LABELS: [u32; 7] = [1, 5, 12, 30, 60, 120, 300];

/// The geometry of one drawing, so a test can ask where a window or a depth falls.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    cols: usize,
    rows: usize,
    lay: Layout,
}

impl Frame {
    const fn new(cols: usize, rows: usize, lay: Layout) -> Self {
        Self { cols, rows, lay }
    }

    fn height(self) -> f64 {
        self.lay.pad_t + self.rows as f64 * self.lay.row + self.lay.pad_b
    }

    fn base(self) -> f64 {
        self.height() - self.lay.pad_b
    }

    fn col_w(self) -> f64 {
        (self.lay.w - self.lay.pad_l - self.lay.pad_r) / self.cols as f64
    }

    /// The left edge of column `i`.
    #[must_use]
    pub fn left(self, i: usize) -> f64 {
        self.lay.pad_l + i as f64 * self.col_w()
    }

    /// The middle of column `i`.
    #[must_use]
    pub fn x(self, i: usize) -> f64 {
        self.left(i) + self.col_w() / 2.0
    }

    /// The top edge of row `k` (1 is the row on the baseline; zero is the baseline itself).
    #[must_use]
    pub fn top(self, k: usize) -> f64 {
        self.base() - k.min(self.rows) as f64 * self.lay.row
    }

    /// The middle of row `k`.
    #[must_use]
    pub fn y(self, k: usize) -> f64 {
        self.top(k) + self.lay.row / 2.0
    }

    /// The largest disc: it leaves a margin inside its cell, so neighbours never touch.
    #[must_use]
    pub fn r_max(self) -> f64 {
        self.col_w().min(self.lay.row) / 2.0 - 2.5
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

/// The figure, drawn for a card and for a phone: a field of discs, the typical morning in bold,
/// the middle half as a band, the share of mornings that ever got to each level at the right.
/// `names` are the voices in the order they join, for the readings and the description.
#[must_use]
pub fn chorus_figure(c: &Chorus, names: &[String]) -> String {
    if !drawable(c) {
        return EMPTY.to_string();
    }
    let wide = draw(c, names, WIDE, "chorus-assembling", " viz-chart--wide");
    let narrow = draw(
        c,
        names,
        NARROW,
        "chorus-assembling-narrow",
        " viz-chart--narrow",
    );
    format!("{wide}{narrow}")
}

const fn drawable(c: &Chorus) -> bool {
    c.windows.len() >= 2 && c.voices >= 2 && c.mornings > 0 && c.reached.len() == c.voices
}

/// One drawing of the figure at `lay`'s proportions; `id` names it for the chart kit and prefixes
/// its gradient ids (two drawings on one page must not share an id), `class` is what CSS switches on.
fn draw(c: &Chorus, names: &[String], lay: Layout, id: &str, class: &str) -> String {
    let n = c.windows.len();
    let frame = Frame::new(n, c.voices, lay);
    let (h, base, right) = (frame.height(), frame.base(), lay.w - lay.pad_r);
    let (w, pad_l, pad_t) = (lay.w, lay.pad_l, lay.pad_t);
    let g = kit::id_safe(id);

    // The first cell the keyboard lands on: where the typical morning first reaches its top.
    let top = c.median(n - 1);
    let landing = (0..n).find(|&i| c.median(i) == top).unwrap_or(0);
    let start = (c.voices - top.max(1)) * n + landing;

    let mut s = format!(
        r#"<div class="ck as-fig{class}"><svg viewBox="0 0 {w} {h:.0}" width="100%" role="img" class="as-svg ck-in" data-ck="marks" data-ck-id="{g}" data-ck-grid="{n}" data-ck-start="{start}" data-ck-label="Left and right move between times allowed, up and down between numbers of voices.">"#,
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
        let label = if lay.narrow {
            k.to_string()
        } else {
            format!("{k} {}", if k == 1 { "voice" } else { "voices" })
        };
        let _ = write!(
            s,
            r#"<line class="ck-grid" x1="{pad_l}" y1="{y:.1}" x2="{right:.1}" y2="{y:.1}"/><text class="ck-lbl as-row" x="{lx:.1}" y="{ty:.1}" text-anchor="end">{label}</text>"#,
            y = frame.top(k),
            lx = pad_l - if lay.narrow { 6.0 } else { 8.0 },
            ty = frame.y(k) + 4.0,
        );
    }
    let _ = write!(
        s,
        r#"<line class="ck-axis" x1="{pad_l}" y1="{base:.1}" x2="{right:.1}" y2="{base:.1}"/>"#
    );
    for (i, &win) in c.windows.iter().enumerate() {
        if lay.narrow && !NARROW_LABELS.contains(&win) {
            continue;
        }
        let _ = write!(
            s,
            r#"<text class="ck-lbl mono" x="{x:.1}" y="{ty:.1}" text-anchor="middle">{label}</text>"#,
            x = frame.x(i),
            ty = base + 16.0,
            label = rung_label(win),
        );
    }
    let _ = write!(
        s,
        r#"<text class="ck-lbl as-axis" x="{mid:.1}" y="{ty:.1}" text-anchor="middle">{title}</text><text class="ck-lbl as-axis" x="{gx:.1}" y="{ay:.1}">by {last}</text>"#,
        mid = f64::midpoint(pad_l, right),
        ty = h - 8.0,
        title = if lay.narrow {
            "time allowed, first voice to last"
        } else {
            "time allowed from the first voice to the last"
        },
        gx = right + 6.0,
        ay = pad_t - 8.0,
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
                r#"<rect class="ck-hit" x="{x:.1}" y="{y:.1}" width="{cw:.1}" height="{row}" data-tip="{reading}" data-ck-i="{idx}"/>"#,
                x = frame.left(i),
                y = frame.top(k),
                cw = frame.col_w(),
                row = lay.row,
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
            x = right + 6.0,
            y = frame.y(k) + 4.0,
            pct = c.percent(k, n - 1),
        );
    }

    // One note, in the margin, for the typical morning's last climb, only where it fits.
    if top >= 2 {
        let text = format!(
            "{} {top} voices in {}",
            if lay.narrow {
                "typical:"
            } else {
                "typical morning:"
            },
            window_label(c.windows[landing])
        );
        let (px, py) = (frame.left(landing), frame.top(top));
        let nw = kit::note_width(&text);
        // In the margin above the grid, with a leader down the edge of the column where the line
        // climbs: a disc never reaches a column's edge, so the leader crosses none of them, and the
        // words are never written over a mark.
        let ny = pad_t - 16.0;
        if px - 10.0 - nw >= pad_l {
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

    /// The drawing for a card, as the figure draws it.
    fn wide(c: &Chorus, names: &[String]) -> String {
        draw(c, names, WIDE, "chorus-assembling", " viz-chart--wide")
    }

    /// The drawing for a phone.
    fn narrow(c: &Chorus, names: &[String]) -> String {
        draw(
            c,
            names,
            NARROW,
            "chorus-assembling-narrow",
            " viz-chart--narrow",
        )
    }

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
        // Half is not a third: six mornings with 3, 3, 2, 2, 1 and 1 voices at the second window.
        // Half of them (three) have two or more, a third of them (two) have three, so the typical
        // morning is two voices, the middle half runs from one to three.
        let six = Chorus::from_depths(
            &[1, 2],
            3,
            &[
                vec![1, 3],
                vec![1, 3],
                vec![1, 2],
                vec![1, 2],
                vec![1, 1],
                vec![1, 1],
            ],
        )
        .expect("rows");
        assert_eq!((six.lower(1), six.median(1), six.upper(1)), (1, 2, 3));
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
        let f = Frame::new(14, 6, WIDE);
        assert!((f.left(0) - WIDE.pad_l).abs() < 1e-9);
        assert!((f.left(14) - (WIDE.w - WIDE.pad_r)).abs() < 1e-9);
        assert!((f.left(5) - f.left(4) - f.col_w()).abs() < 1e-9);
        assert!(
            (f.top(0) - f.base()).abs() < 1e-9,
            "zero voices is the baseline"
        );
        assert!((f.top(1) - f.base() + WIDE.row).abs() < 1e-9);
        assert!(f.top(6) < f.top(5), "more voices are higher");
        assert!(
            (f.top(9) - f.top(6)).abs() < 1e-9,
            "never above the top row"
        );
        assert!(
            f.r_max() * 2.0 < f.col_w().min(WIDE.row),
            "a disc leaves room round it"
        );
        assert!((f.height() - (WIDE.pad_t + 6.0 * WIDE.row + WIDE.pad_b)).abs() < 1e-9);
    }

    /// Area is the share: a quarter of the mornings is half the radius of all of them, none is
    /// no disc, and a few are still a disc you can see.
    #[test]
    fn a_discs_area_is_the_share_of_mornings() {
        let f = Frame::new(6, 3, WIDE);
        let full = radius(f, 1.0).expect("a disc");
        assert!((full - f.r_max()).abs() < 1e-9);
        assert!((radius(f, 0.25).expect("a disc") - full / 2.0).abs() < 1e-9);
        assert!(radius(f, 0.0).is_none());
        assert!(
            (radius(f, 0.0001).expect("a disc") - 2.4).abs() < 1e-9,
            "a few are still visible"
        );
        let svg = wide(&four(), &names());
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
        Frame::new(c.windows.len(), c.voices, WIDE)
    }

    /// The bold line sits at the top of the highest row that at least half the mornings reached,
    /// in every column.
    #[test]
    fn the_typical_contour_is_where_half_the_mornings_stop() {
        let c = four();
        let f = f_of(&c);
        let svg = wide(&c, &names());
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
        let svg = wide(&c, &names());
        let h = f_of(&c).height();
        for (attr, hi) in [
            ("x", WIDE.w),
            ("x1", WIDE.w),
            ("x2", WIDE.w),
            ("cx", WIDE.w),
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
                assert!((0.0..=WIDE.w.max(h)).contains(&v), "path value {v} in {d}");
            }
        }
    }

    /// One reading for every cell, each saying how many mornings it is and what the typical
    /// morning is at that time; every index once, top row first, so the arrows walk it.
    #[test]
    fn every_cell_has_a_reading_and_an_index_the_arrows_can_walk() {
        let svg = wide(&four(), &names());
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
        let svg = wide(&four(), &names());
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
        let svg = wide(&c, &names());
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
        assert!(!wide(&flat, &names()).contains("ck-note"));
        // The typical morning has them all from the first column: no room to its left.
        let brisk = Chorus::from_depths(&windows(), 3, &[vec![3; 6], vec![3; 6]]).expect("rows");
        assert!(!wide(&brisk, &names()).contains("ck-note"));
        // A rise too early to leave room for the words is not squeezed in.
        let early = Chorus::from_depths(
            &windows(),
            3,
            &[vec![1, 3, 3, 3, 3, 3], vec![1, 3, 3, 3, 3, 3]],
        )
        .expect("rows");
        assert!(!wide(&early, &names()).contains("ck-note"));
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
        let svg = wide(&four(), &["<b>x</b>".into(), "B".into(), "C".into()]);
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
        let svg = wide(
            &c,
            &(1..=6).map(|k| format!("Voice {k}")).collect::<Vec<_>>(),
        );
        assert_eq!(svg.matches(r#"class="ck-hit""#).count(), 6 * 14);
        let h = f_of(&c).height();
        assert!(
            svg.contains(&format!("viewBox=\"0 0 {} {h:.0}\"", WIDE.w)),
            "{svg}"
        );
        assert!(svg.contains("ck-c6"));
    }

    /// A card and a phone each get a drawing of their own, with names of their own: two drawings
    /// that shared an id would be a defect on the page, and each carries the class CSS switches on.
    #[test]
    fn the_figure_is_drawn_for_a_card_and_for_a_phone() {
        let both = chorus_figure(&four(), &names());
        assert_eq!(both.matches("<svg").count(), 2, "{both}");
        assert_eq!(
            both.matches(r#"class="ck as-fig viz-chart--wide""#).count(),
            1
        );
        assert_eq!(
            both.matches(r#"class="ck as-fig viz-chart--narrow""#)
                .count(),
            1
        );
        assert!(both.contains(r#"data-ck-id="chorus-assembling""#));
        assert!(both.contains(r#"data-ck-id="chorus-assembling-narrow""#));
        let ids: Vec<&str> = both
            .split(" id=\"")
            .skip(1)
            .filter_map(|r| r.split('"').next())
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "an id is defined twice: {ids:?}");
        let (wide_svg, narrow_svg) = both
            .split_once(r#"<div class="ck as-fig viz-chart--narrow">"#)
            .expect("two drawings");
        assert!(
            wide_svg.contains("viewBox=\"0 0 560 ") && narrow_svg.contains("viewBox=\"0 0 360 ")
        );
    }

    /// The phone's drawing says the same with fewer words: row numbers without "voices", a label
    /// under only some of the columns, a shorter axis title. The readings are the same cells.
    #[test]
    fn the_phones_drawing_has_short_labels_and_the_same_readings() {
        let ladder = [1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300];
        let rows: Vec<Vec<u32>> = (0..10)
            .map(|_| (0..14).map(|i| 1 + (i / 5).min(2)).collect())
            .collect();
        let c = Chorus::from_depths(&ladder, 3, &rows).expect("rows");
        let (w, n) = (wide(&c, &names()), narrow(&c, &names()));
        let col_labels = |svg: &str| svg.matches(r#"class="ck-lbl mono" x="#).count();
        assert_eq!(col_labels(&w), 14);
        assert_eq!(
            col_labels(&n),
            NARROW_LABELS.len(),
            "only the named rungs are labelled on a phone"
        );
        for label in ["1m", "5m", "12m", "30m", "1h", "2h", "5h"] {
            assert!(n.contains(&format!(">{label}<")), "{label}");
        }
        assert!(!n.contains(">45m<") && w.contains(">45m<"));
        assert!(w.contains(">3 voices<") && !n.contains(">3 voices<") && n.contains(">3</text>"));
        assert!(
            n.contains("time allowed, first voice to last")
                && !n.contains("from the first voice to the last")
        );
        assert_eq!(
            w.matches(r#"class="ck-hit""#).count(),
            n.matches(r#"class="ck-hit""#).count()
        );
        // The same cells say the same things.
        assert!(w.contains("3 voices within 10 min") || n.contains("3 voices within 20 min"));
        let reading = |svg: &str, k: usize, i: usize| {
            let idx = (3 - k) * 14 + i;
            svg.split("data-tip=\"").skip(1).find_map(|c| {
                let (tip, rest) = c.split_once("\" data-ck-i=\"")?;
                rest.starts_with(&format!("{idx}\""))
                    .then(|| tip.to_owned())
            })
        };
        assert!(reading(&w, 2, 7).is_some_and(|t| t.contains("2 voices within 30 min")));
        assert_eq!(reading(&w, 2, 7), reading(&n, 2, 7));
    }

    /// Every mark of the phone's drawing is inside its picture, as the card's is, and no label's
    /// text is longer than the room its row or column gives it.
    #[test]
    fn the_phones_drawing_is_inside_its_picture_too() {
        let ladder = [1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300];
        let rows: Vec<Vec<u32>> = (0..12)
            .map(|m| (0..14).map(|i| 1 + ((i + m) / 4).min(5)).collect())
            .collect();
        let c = Chorus::from_depths(&ladder, 6, &rows).expect("rows");
        let names: Vec<String> = (1..=6).map(|k| format!("Voice {k}")).collect();
        for (svg, lay) in [(wide(&c, &names), WIDE), (narrow(&c, &names), NARROW)] {
            let h = Frame::new(14, 6, lay).height();
            for (attr, hi) in [
                ("x", lay.w),
                ("x1", lay.w),
                ("x2", lay.w),
                ("cx", lay.w),
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
                    assert!(
                        (0.0..=hi).contains(&v),
                        "{attr}={v} in a drawing {} wide",
                        lay.w
                    );
                }
            }
            // Right-hand figures leave the picture's edge room: "100%" at 11 units is under 30 wide.
            let gutter = lay.pad_r - 6.0;
            assert!(
                kit::text_width("100%", 11.0) < gutter,
                "{} < {gutter}",
                kit::text_width("100%", 11.0)
            );
        }
    }
}
