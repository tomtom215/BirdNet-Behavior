//! The plate, written as one inline SVG.
//!
//! Everything a plate needs is in the markup this returns: paper, plate-mark, title block, one
//! hatched oval per species with its Latin name inside, a caption under each, a numbered key
//! and a colophon. It is complete with no script. `static/plate.js` only improves it, by
//! fetching each species' picture and drawing it into the oval.
//!
//! Geometry is in the SVG's own units (1600 across), so the whole sheet scales to any screen
//! and every position is an attribute: no inline style, which the page's CSP forbids.

use std::fmt::Write as _;

use birdnet_integrations::species_images::PlateIndex;

use super::layout::{self, Placement, Rect};
use crate::routes::pages::{escape_html, group_thousands};

/// The sheet is this many units across; its height follows the screen's shape.
pub const SHEET_W: f64 = 1600.0;
/// Inset of the plate-mark (the double rule) from the edge of the sheet.
const MARK: f64 = 34.0;
/// How far the figures keep in from the plate-mark.
const PAD: f64 = 56.0;
/// Species drawn as pictures on a landscape sheet; the rest are in the key.
const MAX_FIGURES_WIDE: usize = 18;
/// Species drawn as pictures on a portrait sheet.
const MAX_FIGURES_TALL: usize = 12;
/// Species listed in the key before "and N more kinds".
const MAX_KEY: usize = 40;
/// Height of one key row.
const KEY_ROW: f64 = 24.0;

/// A species heard on the plate's day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    /// Scientific name, as stored (the picture is looked up by it).
    pub sci: String,
    /// Common name, in the station's language.
    pub com: String,
    /// Detections that day.
    pub count: u64,
    /// Local time of the first detection, `HH:MM`.
    pub first: String,
    /// Local time of the latest detection, `HH:MM`.
    pub last: String,
}

/// Which day the plate shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    /// Today so far.
    Today,
    /// Yesterday, shown in the small hours before anything has been heard today.
    Yesterday,
}

/// What a plate is drawn from.
pub struct Input<'a> {
    /// Species heard, most heard first.
    pub heard: &'a [Heard],
    /// Which day it is.
    pub day: Day,
    /// The day, `YYYY-MM-DD`.
    pub date: &'a str,
    /// Draw the lamplight plate (dark paper, light ink).
    pub night: bool,
    /// Width over height of the screen showing it.
    pub aspect: f64,
    /// Turns the packing, so each day's plate differs and one day's is stable.
    pub seed: u32,
    /// Only the most recently heard species, large.
    pub latest_only: bool,
    /// A line of credit the operator keeps beside their artwork (`CREDITS.txt`), if any.
    pub credit: Option<&'a str>,
    /// Draw the paper's fibres (a filter; the Pi can do without).
    pub paper: bool,
}

/// `1` to `3999` in Roman numerals; `0` is empty.
#[must_use]
pub fn roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, glyph) in TABLE {
        while n >= value {
            out.push_str(glyph);
            n -= value;
        }
    }
    out
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// `(year, month, day)` of a `YYYY-MM-DD` string.
fn ymd(date: &str) -> Option<(u32, u32, u32)> {
    let y = date.get(0..4)?.parse().ok()?;
    let m: u32 = date.get(5..7)?.parse().ok()?;
    let d: u32 = date.get(8..10)?.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// "Wednesday 28 October" for `2026-10-28`; the date itself when it does not parse.
#[must_use]
pub fn long_date(date: &str) -> String {
    let Some((y, m, d)) = ymd(date) else {
        return date.to_owned();
    };
    // 1970-01-01 was a Thursday, the fourth day of a week that starts on Monday.
    let weekday =
        usize::try_from((birdnet_core::civil::days_from_civil(y, m, d) + 3).rem_euclid(7))
            .unwrap_or(0);
    format!("{} {d} {}", WEEKDAYS[weekday], MONTHS[(m - 1) as usize])
}

/// The day of the year, 1 to 366, which is the plate's number: one plate a day.
#[must_use]
pub fn day_of_year(date: &str) -> u32 {
    ymd(date).map_or(1, |(y, m, d)| {
        let days = birdnet_core::civil::days_from_civil(y, m, d)
            - birdnet_core::civil::days_from_civil(y, 1, 1)
            + 1;
        u32::try_from(days).unwrap_or(1)
    })
}

/// Width of an average character as a share of the font size, by the face the plate sets it in.
///
/// Measured from a rendered plate (1920 by 1080, Instrument Serif italic and Inter Tight), not
/// guessed: "Poecile atricapillus" at 15 units was 97 units wide, and "BLACK-CAPPED CHICKADEE" at
/// 11.5 with its 0.2 em letter-spacing was 180. A name is cut only where it would run into the
/// next figure, so these err a little wide.
#[derive(Debug, Clone, Copy)]
pub enum Face {
    /// The italic serif the Latin names are set in.
    Serif,
    /// The tracked capitals of the common names.
    Caps,
    /// The plain sans of the colophon.
    Sans,
}

impl Face {
    const fn em(self) -> f64 {
        match self {
            Self::Serif => 0.38,
            Self::Caps => 0.8,
            Self::Sans => 0.52,
        }
    }
}

/// A stand-in for the width of `text` set at `size` units in `face`. Good enough to decide where
/// a name must be shortened, and the same on every machine.
fn est_width(text: &str, size: f64, face: Face) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = text.chars().count() as f64;
    n * size * face.em()
}

/// `text` cut to fit `max` units at `size` in `face`, ending in an ellipsis when it was cut.
#[must_use]
pub fn fit(text: &str, size: f64, max: f64, face: Face) -> String {
    if est_width(text, size, face) <= max {
        return text.to_owned();
    }
    let keep = (max / (size * face.em())).floor().max(2.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let keep = keep as usize;
    let mut s: String = text.chars().take(keep.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// A title for the day, from when the last bird was heard.
fn title(heard: &[Heard], day: Day) -> &'static str {
    if heard.is_empty() {
        return "Nothing heard yet";
    }
    if day == Day::Yesterday {
        return "Heard yesterday";
    }
    let hour: u32 = heard
        .iter()
        .map(|h| h.last.as_str())
        .max()
        .and_then(|t| t.get(0..2))
        .and_then(|h| h.parse().ok())
        .unwrap_or(12);
    match hour {
        0..=4 => "Heard in the night",
        5..=11 => "Heard this morning",
        12..=16 => "Heard this afternoon",
        _ => "Heard this evening",
    }
}

/// The sheet's height for a screen of this shape; the shape is held to what a plate can take.
fn sheet_h(aspect: f64) -> f64 {
    SHEET_W / aspect.clamp(0.5, 2.4)
}

/// A figure number for key and oval, `1`-based.
const fn no(i: usize) -> usize {
    i + 1
}

/// Everything about the plate that changes it, hashed: the page asks whether anything did.
#[must_use]
pub fn digest(input: &Input<'_>) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in input.heard {
        (&s.sci, &s.com, s.count, &s.first, &s.last).hash(&mut h);
    }
    (
        input.day == Day::Today,
        input.date,
        input.night,
        // Screens of nearly the same shape share a plate.
        (input.aspect.clamp(0.5, 2.4) * 20.0).round().to_bits(),
        input.seed,
        input.latest_only,
        input.credit,
        input.paper,
    )
        .hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Where the parts of the sheet are, worked out once from the screen's shape and what was heard.
struct Sheet {
    /// Width of the sheet in its own units.
    w: f64,
    /// Height of the sheet.
    h: f64,
    /// A portrait sheet: two key columns, fewer figures.
    tall: bool,
    /// Height of the title block.
    head_h: f64,
    /// Key columns.
    cols: usize,
    /// Rows in the key.
    key_rows: usize,
    /// Species listed in the key.
    listed: usize,
    /// Height of the key, including its rule.
    key_h: f64,
    /// The area the figures are placed in.
    free: Rect,
}

/// Height of the foot (the colophon line).
const FOOT_H: f64 = 34.0;
/// Font size of a common name under its figure.
const COM_SIZE: f64 = 11.5;
/// The size a Latin name is assumed to be set at when its caption's room is reckoned; the real
/// size follows the oval (15 to 26), and the oval's own width covers the larger ones.
const SCI_ROOM_SIZE: f64 = 18.0;

impl Sheet {
    fn of(input: &Input<'_>) -> Self {
        let aspect = input.aspect.clamp(0.5, 2.4);
        let (w, h) = (SHEET_W, sheet_h(aspect));
        let tall = aspect < 1.0;
        let head_h = if tall { 208.0 } else { 168.0 };
        // The key's columns: two on a portrait sheet, six on an ultra-wide one so the key does
        // not take the height the figures need, four otherwise.
        let cols = if tall {
            2
        } else if aspect >= 2.0 {
            6
        } else {
            4
        };
        let listed = input.heard.len().min(MAX_KEY);
        let key_rows = listed.div_ceil(cols);
        #[allow(clippy::cast_precision_loss)]
        let key_h = if listed == 0 {
            0.0
        } else {
            key_rows as f64 * KEY_ROW + 40.0
        };
        let free = Rect {
            x: MARK + PAD,
            y: MARK + head_h,
            w: w - 2.0 * (MARK + PAD),
            h: h - MARK - head_h - key_h - FOOT_H - MARK,
        };
        Self {
            w,
            h,
            tall,
            head_h,
            cols,
            key_rows,
            listed,
            key_h,
            free,
        }
    }
}

/// The width a species' caption needs: its common name in tracked capitals or its Latin name
/// in italic, whichever is wider, between the layout's floor and ceiling. The layout keeps
/// neighbours off a box this wide, so the whole name can be printed.
fn caption_room(bird: &Heard) -> f64 {
    est_width(&bird.com.to_uppercase(), COM_SIZE, Face::Caps)
        .max(est_width(&bird.sci, SCI_ROOM_SIZE, Face::Serif))
        .clamp(layout::CAPTION_MIN_W, layout::CAPTION_MAX_W)
}

/// The figures to draw and where: every species up to the sheet's limit, sized by how often it
/// was heard, or in the latest mode the one heard last, large. The layout may leave out the
/// rarest when they do not fit, so what comes back is what was *placed*.
fn place_figures(input: &Input<'_>, sheet: &Sheet) -> Vec<Placement> {
    let limit = if sheet.tall {
        MAX_FIGURES_TALL
    } else {
        MAX_FIGURES_WIDE
    };
    let free = sheet.free;
    if input.latest_only {
        let Some((latest, _)) = input
            .heard
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.last.cmp(&b.1.last).then(b.0.cmp(&a.0)))
        else {
            return Vec::new();
        };
        // One bird, as large as the free area allows, in the middle of it.
        let width = (free.w * 0.62).min((free.h - layout::CAPTION_H) / layout::ASPECT);
        return vec![Placement {
            index: latest,
            cx: free.x + free.w / 2.0,
            cy: free.y + (free.h - layout::CAPTION_H) / 2.0,
            w: width,
            h: width * layout::ASPECT,
            cap_w: caption_room(&input.heard[latest]),
        }];
    }
    let drawn: Vec<usize> = (0..input.heard.len().min(limit)).collect();
    let counts: Vec<u64> = drawn.iter().map(|&i| input.heard[i].count).collect();
    let rooms: Vec<f64> = drawn
        .iter()
        .map(|&i| caption_room(&input.heard[i]))
        .collect();
    layout::place(&counts, &rooms, free, input.seed)
        .into_iter()
        .map(|mut p| {
            p.index = drawn[p.index];
            p
        })
        .collect()
}

/// The `<svg>` element's opening, the filters and patterns, the paper and the plate-mark.
fn write_frame(svg: &mut String, input: &Input<'_>, sheet: &Sheet, heading: &str) {
    let (w, h) = (sheet.w, sheet.h);
    let _ = writeln!(
        svg,
        r#"<svg class="plate-svg" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w:.0} {h:.0}" preserveAspectRatio="xMidYMid meet" role="img" aria-labelledby="plate-t plate-d" data-hash="{hash}" data-night="{night}" data-paper="{paper}">
<title id="plate-t">{heading}</title>
<desc id="plate-d">{desc}</desc>
<defs>
  <filter id="plate-fibre" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency=".9 .05" numOctaves="3" seed="4"/><feColorMatrix values="0 0 0 0 .45  0 0 0 0 .34  0 0 0 0 .2  0 0 0 .55 -.18"/></filter>
  <filter id="plate-foxing" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency=".012" numOctaves="2" seed="9"/><feColorMatrix values="0 0 0 0 .55  0 0 0 0 .38  0 0 0 0 .18  0 0 0 1.1 -.45"/></filter>
  <pattern id="plate-hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(-35)"><line x1="0" y1="0" x2="0" y2="6" class="plate-hatch-line"/></pattern>
</defs>
<rect class="plate-paper" width="{w:.0}" height="{h:.0}"/>
<rect class="plate-fibre" width="{w:.0}" height="{h:.0}"/>
<rect class="plate-foxing" width="{w:.0}" height="{h:.0}"/>
<rect class="plate-mark" x="{m:.0}" y="{m:.0}" width="{mw:.0}" height="{mh:.0}"/>
<rect class="plate-mark-inner" x="{m2:.0}" y="{m2:.0}" width="{mw2:.0}" height="{mh2:.0}"/>"#,
        hash = digest(input),
        night = u8::from(input.night),
        paper = u8::from(input.paper),
        heading = escape_html(heading),
        desc = escape_html(&describe(input)),
        m = MARK,
        mw = w - 2.0 * MARK,
        mh = h - 2.0 * MARK,
        m2 = MARK + 6.0,
        mw2 = w - 2.0 * (MARK + 6.0),
        mh2 = h - 2.0 * (MARK + 6.0),
    );
}

/// The plate's number, its title, the line under it and the rule with its lozenges.
fn write_title(svg: &mut String, input: &Input<'_>, sheet: &Sheet, heading: &str) {
    let cx = sheet.w / 2.0;
    let total: u64 = input.heard.iter().map(|s| s.count).sum();
    let first = input.heard.iter().map(|s| s.first.as_str()).min();
    let last = input.heard.iter().map(|s| s.last.as_str()).max();
    let sub = sub_line(input.date, first.zip(last), input.heard.len(), total);
    let rule_y = MARK + sheet.head_h - 8.0;
    let _ = writeln!(
        svg,
        r#"<text class="plate-no" x="{cx:.0}" y="{y1:.0}" text-anchor="middle">Plate {plate_no}</text>
<text class="plate-title" x="{cx:.0}" y="{y2:.0}" text-anchor="middle">{heading}</text>
<text class="plate-sub" x="{cx:.0}" y="{y3:.0}" text-anchor="middle">{sub}</text>
<line class="plate-rule" x1="{rx0:.0}" y1="{rule_y:.0}" x2="{rx1:.0}" y2="{rule_y:.0}"/>
<path class="plate-lozenge" d="M{rx0l:.0},{rule_y:.0} l7,-7 l7,7 l-7,7 z M{rx1r:.0},{rule_y:.0} l-7,-7 l-7,7 l7,7 z"/>"#,
        plate_no = roman(day_of_year(input.date)),
        y1 = MARK + 34.0,
        y2 = MARK + 104.0,
        y3 = MARK + 142.0,
        rx0 = cx - 60.0,
        rx1 = cx + 60.0,
        rx0l = cx - 74.0,
        rx1r = cx + 74.0,
        heading = escape_html(heading),
    );
}

/// An oval for each placed species with its caption, or the empty plate's one waiting oval.
fn write_figures(svg: &mut String, input: &Input<'_>, sheet: &Sheet, placements: &[Placement]) {
    if input.heard.is_empty() {
        let free = sheet.free;
        let (ow, oh) = (free.w.min(560.0), free.w.min(560.0) * layout::ASPECT);
        let (ox, oy) = (free.x + free.w / 2.0, free.y + free.h / 2.0);
        let _ = writeln!(
            svg,
            r#"<g class="plate-empty"><ellipse class="plate-cart" cx="{ox:.0}" cy="{oy:.0}" rx="{rx:.0}" ry="{ry:.0}"/>
<text class="plate-sci" x="{ox:.0}" y="{ty:.0}" text-anchor="middle">The first bird of the day will appear here.</text></g>"#,
            rx = ow / 2.0,
            ry = oh / 2.0,
            ty = oy + 8.0,
        );
    }
    for p in placements {
        let bird = &input.heard[p.index];
        let (left, top) = (p.cx - p.w / 2.0, p.cy - p.h / 2.0);
        let cap = p.caption();
        let sci_size = (p.w / 14.0).clamp(15.0, 26.0);
        let _ = writeln!(
            svg,
            r#"<g class="plate-fig" data-sci="{sci_attr}" data-state="none"{work_attr}>
<title>{com}: {count} {detections}</title>
<ellipse class="plate-cart" cx="{cx:.1}" cy="{cy:.1}" rx="{rx:.1}" ry="{ry:.1}"/>
{cart_text}<image class="plate-pic" x="{left:.1}" y="{top:.1}" width="{pw:.1}" height="{ph:.1}" preserveAspectRatio="xMidYMid slice"/>
<circle class="plate-num-ring" cx="{nx:.1}" cy="{ny:.1}" r="14"/>
<text class="plate-num" x="{nx:.1}" y="{nty:.1}" text-anchor="middle">{number}</text>
<text class="plate-sci" x="{cx:.1}" y="{sy:.1}" text-anchor="middle" font-size="{sci_size:.1}">{sci}</text>
<text class="plate-com" x="{cx:.1}" y="{cy2:.1}" text-anchor="middle">{com_up}</text>
</g>"#,
            sci_attr = escape_html(&bird.sci),
            // The book this species' public-domain plate is from, if the index has one: the
            // script names the books it drew in the foot, and has no index of its own.
            work_attr = PlateIndex::embedded()
                .plate_for(&bird.sci)
                .map_or_else(String::new, |(_, work)| {
                    format!(r#" data-work="{}""#, escape_html(&work.citation()))
                }),
            com = escape_html(&bird.com),
            count = group_thousands(i64::try_from(bird.count).unwrap_or(i64::MAX)),
            detections = if bird.count == 1 {
                "detection"
            } else {
                "detections"
            },
            cx = p.cx,
            cy = p.cy,
            rx = p.w / 2.0,
            ry = p.h / 2.0,
            cart_text = cartouche_text(&bird.sci, p),
            pw = p.w,
            ph = p.h,
            number = no(p.index),
            nx = left + 6.0,
            ny = top + 10.0,
            nty = top + 15.0,
            sy = cap.y + sci_size + 2.0,
            sci = escape_html(&fit(&bird.sci, sci_size, cap.w, Face::Serif)),
            cy2 = cap.y + sci_size + 22.0,
            com_up = escape_html(&fit(&bird.com.to_uppercase(), COM_SIZE, cap.w, Face::Caps)),
        );
    }
}

/// The numbered key at the foot: every species up to [`MAX_KEY`], a number where it has a figure
/// and a dot where it does not.
fn write_key(svg: &mut String, input: &Input<'_>, sheet: &Sheet, placements: &[Placement]) {
    if sheet.listed == 0 {
        return;
    }
    let drawn: std::collections::HashSet<usize> = placements.iter().map(|p| p.index).collect();
    let key_top = sheet.h - MARK - FOOT_H - sheet.key_h + 12.0;
    let col_w = (sheet.w - 2.0 * (MARK + PAD)) / f64::from(u32::try_from(sheet.cols).unwrap_or(4));
    let _ = writeln!(
        svg,
        r#"<line class="plate-rule plate-key-rule" x1="{x0:.0}" y1="{key_top:.0}" x2="{x1:.0}" y2="{key_top:.0}"/>
<g class="plate-key">"#,
        x0 = MARK + PAD,
        x1 = sheet.w - MARK - PAD,
    );
    for (i, bird) in input.heard.iter().take(MAX_KEY).enumerate() {
        let (col, row) = (i / sheet.key_rows, i % sheet.key_rows);
        let x = MARK + PAD + col_w * f64::from(u32::try_from(col).unwrap_or(0));
        let y = key_top + 26.0 + KEY_ROW * f64::from(u32::try_from(row).unwrap_or(0));
        let mark = if drawn.contains(&i) {
            no(i).to_string()
        } else {
            "·".to_owned()
        };
        let _ = writeln!(
            svg,
            r#"<text class="plate-key-n" x="{x:.1}" y="{y:.1}">{mark}</text><text class="plate-key-sci" x="{xs:.1}" y="{y:.1}">{sci}</text><text class="plate-key-c" x="{xc:.1}" y="{y:.1}" text-anchor="end">{count}</text>"#,
            xs = x + 26.0,
            xc = x + col_w - 22.0,
            sci = escape_html(&fit(&bird.sci, 17.0, col_w - 26.0 - 74.0, Face::Serif)),
            count = group_thousands(i64::try_from(bird.count).unwrap_or(i64::MAX)),
        );
    }
    if input.heard.len() > MAX_KEY {
        let more = input.heard.len() - MAX_KEY;
        let _ = writeln!(
            svg,
            r#"<text class="plate-key-more" x="{x:.1}" y="{y:.1}">and {more} more {kinds}</text>"#,
            x = MARK + PAD,
            y = key_top + 26.0 + KEY_ROW * f64::from(u32::try_from(sheet.key_rows).unwrap_or(1))
                - 2.0,
            kinds = if more == 1 { "kind" } else { "kinds" },
        );
    }
    svg.push_str("</g>\n");
}

/// The foot: what the pictures are, and where the idea came from, then the closing tag.
fn write_colophon(svg: &mut String, input: &Input<'_>, sheet: &Sheet) {
    let colophon = input.credit.map_or_else(
        || {
            "Engraved by the station from photographs · size follows the number of detections"
                .to_owned()
        },
        |c| format!("Art: {c}"),
    );
    // The room the foot has: the script rewrites it from what was drawn and measures against this.
    // Four fifths of the width: "Plate layout after fugleramme" takes the right hand end, and what
    // is left of the sheet is room for the books, the credits address and the sentence about size.
    let room = (sheet.w - 2.0 * (MARK + PAD)) * 0.8;
    let _ = write!(
        svg,
        r#"<text class="plate-colophon" id="plate-colophon" data-max="{room:.0}" data-art="{art}" x="{x0:.0}" y="{fy:.0}">{colophon}</text>
<text class="plate-colophon" x="{x1:.0}" y="{fy:.0}" text-anchor="end">Plate layout after fugleramme</text>
</svg>"#,
        x0 = MARK + PAD,
        x1 = sheet.w - MARK - PAD,
        fy = sheet.h - MARK - 16.0,
        art = escape_html(input.credit.unwrap_or_default()),
        colophon = escape_html(&fit(&colophon, 13.0, room, Face::Sans)),
    );
}

/// Render the plate.
#[must_use]
pub fn render(input: &Input<'_>) -> String {
    let sheet = Sheet::of(input);
    let placements = place_figures(input, &sheet);
    let heading = title(input.heard, input.day);
    let mut svg = String::with_capacity(16 * 1024);
    write_frame(&mut svg, input, &sheet, heading);
    write_title(&mut svg, input, &sheet, heading);
    write_figures(&mut svg, input, &sheet, &placements);
    write_key(&mut svg, input, &sheet, &placements);
    write_colophon(&mut svg, input, &sheet);
    svg
}

/// A time as SVG text: canonical `HH:MM` in a `tspan` that `display-format.js` re-prints in the
/// reader's choice of 12 or 24 hours, as every other page does.
fn clock_tspan(hhmm: &str) -> String {
    format!(
        "<tspan {}>{}</tspan>",
        crate::display::clock_attr(hhmm),
        escape_html(hhmm)
    )
}

/// The line under the title, as markup for a `<text>`: the date, the span of the day's singing
/// and what was heard.
fn sub_line(date: &str, span: Option<(&str, &str)>, kinds: usize, total: u64) -> String {
    let date = escape_html(&long_date(date));
    match span {
        Some((first, last)) => format!(
            "{date} · first {}, latest {} · {kinds} {}, {} detections",
            clock_tspan(first),
            clock_tspan(last),
            if kinds == 1 { "kind" } else { "kinds" },
            group_thousands(i64::try_from(total).unwrap_or(i64::MAX)),
        ),
        None => format!("{date} · the station is listening"),
    }
}

/// A sentence for the screen reader: what is on the plate, in words.
fn describe(input: &Input<'_>) -> String {
    if input.heard.is_empty() {
        return "Nothing has been heard yet; the plate fills as birds are identified.".to_owned();
    }
    let mut s = format!(
        "{} on {}: ",
        title(input.heard, input.day),
        long_date(input.date)
    );
    for (i, h) in input.heard.iter().take(MAX_KEY).enumerate() {
        if i > 0 {
            s.push_str("; ");
        }
        let _ = write!(s, "{} ({}), {} detections", h.com, h.sci, h.count);
    }
    s
}

/// The Latin name inside an oval, set on two lines at the space, so the plate says what is
/// there before (or without) its picture. Hidden by CSS when a picture arrives.
fn cartouche_text(sci: &str, p: &Placement) -> String {
    let size = (p.w / 11.0).clamp(14.0, 34.0);
    let (genus, species) = sci.split_once(' ').unwrap_or((sci, ""));
    let max = p.w * 0.8;
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<text class="plate-cart-name" x="{cx:.1}" y="{y1:.1}" text-anchor="middle" font-size="{size:.1}">{g}</text>"#,
        cx = p.cx,
        y1 = if species.is_empty() {
            p.cy + size / 3.0
        } else {
            p.cy - size * 0.15
        },
        g = escape_html(&fit(genus, size, max, Face::Serif)),
    );
    if !species.is_empty() {
        let _ = write!(
            out,
            r#"<text class="plate-cart-name" x="{cx:.1}" y="{y2:.1}" text-anchor="middle" font-size="{size:.1}">{s}</text>"#,
            cx = p.cx,
            y2 = p.cy + size * 1.05,
            s = escape_html(&fit(species, size, max, Face::Serif)),
        );
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(n: usize) -> Vec<Heard> {
        (0..n)
            .map(|i| Heard {
                sci: format!("Genus species{i}"),
                com: format!("Bird number {i}"),
                count: u64::try_from(100 / (i + 1)).unwrap_or(1).max(1),
                first: "06:12".into(),
                last: format!("08:{:02}", i % 60),
            })
            .collect()
    }

    fn input(heard: &[Heard]) -> Input<'_> {
        Input {
            heard,
            day: Day::Today,
            date: "2026-10-28",
            night: false,
            aspect: 1.6,
            seed: 301,
            latest_only: false,
            credit: None,
            paper: true,
        }
    }

    #[test]
    fn roman_numerals_for_every_day_of_a_year() {
        assert_eq!(roman(0), "");
        assert_eq!(roman(1), "I");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(49), "XLIX");
        assert_eq!(roman(301), "CCCI");
        assert_eq!(roman(366), "CCCLXVI");
        assert_eq!(roman(1994), "MCMXCIV");
    }

    #[test]
    fn the_plate_is_numbered_by_the_day_of_the_year() {
        assert_eq!(day_of_year("2026-01-01"), 1);
        assert_eq!(day_of_year("2026-10-28"), 301);
        assert_eq!(day_of_year("2024-12-31"), 366);
        assert_eq!(day_of_year("garbage"), 1);
    }

    #[test]
    fn dates_are_written_the_way_a_plate_would() {
        assert_eq!(long_date("2026-10-28"), "Wednesday 28 October");
        assert_eq!(long_date("2024-02-29"), "Thursday 29 February");
        assert_eq!(long_date("nope"), "nope");
    }

    #[test]
    fn a_long_name_is_cut_to_the_room_it_has_and_a_short_one_is_not() {
        assert_eq!(
            fit("Turdus merula", 20.0, 400.0, Face::Serif),
            "Turdus merula"
        );
        let cut = fit("Pseudoscopelus cordilluminatus", 20.0, 100.0, Face::Serif);
        assert!(cut.ends_with('…') && cut.chars().count() <= 14, "{cut}");
        assert_eq!(fit("ab", 20.0, 1.0, Face::Serif).chars().count(), 2);
        // The tracked capitals are wider than the italic serif, so the same room holds fewer.
        let name = "BLACK-CAPPED CHICKADEE";
        assert_eq!(fit(name, 11.5, 210.0, Face::Caps), name);
        assert!(fit(name, 11.5, 150.0, Face::Caps).ends_with('…'));
    }

    /// A figure whose species has a public-domain plate says which book it is from, so the script
    /// can name the books it actually drew in the foot; a species with none says nothing.
    #[test]
    fn a_species_with_a_plate_names_its_work_and_one_without_does_not() {
        let mut h = heard(2);
        h[0].sci = "Turdus merula".into();
        let svg = render(&input(&h));
        let (_, work) = PlateIndex::embedded()
            .plate_for("Turdus merula")
            .expect("the index has a blackbird plate");
        let figure = |sci: &str| {
            svg.split("<g class=\"plate-fig\"")
                .find(|f| f.contains(&format!("data-sci=\"{sci}\"")))
                .and_then(|f| f.split('>').next())
                .unwrap_or_default()
                .to_owned()
        };
        assert!(
            figure("Turdus merula")
                .contains(&format!("data-work=\"{}\"", escape_html(&work.citation()))),
            "{}",
            figure("Turdus merula")
        );
        assert!(
            !figure("Genus species1").contains("data-work"),
            "{}",
            figure("Genus species1")
        );
    }

    /// The foot is rewritten by the script from what was drawn, so it carries what the script needs:
    /// an id, the room it has, and the operator's credit for their own art.
    #[test]
    fn the_foot_carries_what_the_script_rewrites_it_from() {
        let h = heard(1);
        let mut input = input(&h);
        input.credit = Some("Mina \"Wren\" Ortiz, linocut");
        let svg = render(&input);
        let foot = svg
            .split("<text class=\"plate-colophon\"")
            .nth(1)
            .and_then(|f| f.split('>').next())
            .unwrap_or_default();
        assert!(foot.contains("id=\"plate-colophon\""), "{foot}");
        assert!(foot.contains("data-max=\""), "{foot}");
        assert!(
            foot.contains("data-art=\"Mina &quot;Wren&quot; Ortiz, linocut\""),
            "{foot}"
        );
        assert!(
            svg.contains("Art: Mina &quot;Wren&quot; Ortiz, linocut"),
            "no script: the art is credited"
        );
    }

    #[test]
    fn an_empty_station_gets_an_empty_plate_not_an_error() {
        let svg = render(&input(&[]));
        assert!(svg.contains("Nothing heard yet"), "{svg}");
        assert!(svg.contains("The first bird of the day will appear here."));
        assert!(!svg.contains("plate-key\""), "no key for no birds");
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    }

    #[test]
    fn every_species_drawn_has_a_figure_a_number_and_a_key_row() {
        let h = heard(5);
        let svg = render(&input(&h));
        assert_eq!(svg.matches("class=\"plate-fig\"").count(), 5);
        assert_eq!(svg.matches("class=\"plate-key-sci\"").count(), 5);
        for i in 0..5 {
            assert!(
                svg.contains(&format!("data-sci=\"Genus species{i}\"")),
                "{i}"
            );
        }
    }

    #[test]
    fn a_busy_day_draws_the_busiest_and_lists_the_rest_in_the_key() {
        let h = heard(30);
        let svg = render(&input(&h));
        assert!(svg.matches("class=\"plate-fig\"").count() <= MAX_FIGURES_WIDE);
        assert_eq!(svg.matches("class=\"plate-key-sci\"").count(), 30);
        // Undrawn species carry a dot where the number would be.
        assert!(svg.contains(">·</text>"), "{svg}");
        // More than the key can hold says how many more.
        let svg = render(&input(&heard(55)));
        assert!(svg.contains("and 15 more kinds"), "{svg}");
    }

    #[test]
    fn the_latest_mode_draws_one_bird_the_one_heard_last() {
        let mut h = heard(4);
        h[2].last = "09:59".into();
        let mut i = input(&h);
        i.latest_only = true;
        let svg = render(&i);
        assert_eq!(svg.matches("class=\"plate-fig\"").count(), 1);
        assert!(svg.contains("data-sci=\"Genus species2\""), "{svg}");
    }

    #[test]
    fn names_from_the_database_are_escaped() {
        let h = vec![Heard {
            sci: "A\"><script>x</script>".into(),
            com: "B & <i>c</i>".into(),
            count: 3,
            first: "06:00".into(),
            last: "06:01".into(),
        }];
        let svg = render(&input(&h));
        assert!(!svg.contains("<script>") && !svg.contains("<i>"), "{svg}");
        assert!(
            svg.contains("B &amp; &lt;i&gt;c&lt;/i&gt;") || svg.contains("B &amp;"),
            "{svg}"
        );
    }

    #[test]
    fn the_plate_changes_its_digest_when_the_birds_do_and_not_otherwise() {
        let h = heard(3);
        let a = digest(&input(&h));
        assert_eq!(a, digest(&input(&h)), "same birds, same digest");
        let mut h2 = h.clone();
        h2[0].count += 1;
        assert_ne!(a, digest(&input(&h2)), "one more detection redraws it");
        let mut night = input(&h);
        night.night = true;
        assert_ne!(a, digest(&night), "dusk redraws it");
        let mut shaped = input(&h);
        shaped.aspect = 1.61;
        assert_eq!(
            a,
            digest(&shaped),
            "a screen a hair different in shape does not"
        );
    }

    #[test]
    fn the_sheet_follows_the_screens_shape_within_limits() {
        assert!((sheet_h(1.6) - 1000.0).abs() < 1e-9);
        assert!(
            (sheet_h(0.1) - 3200.0).abs() < 1e-9,
            "never taller than 1 : 2"
        );
        assert!(
            (sheet_h(10.0) - SHEET_W / 2.4).abs() < 1e-9,
            "never wider than 2.4 : 1"
        );
    }

    #[test]
    fn every_figure_and_caption_stays_on_the_sheet() {
        for aspect in [0.56, 0.75, 1.0, 1.6, 1.78, 2.2] {
            let h = heard(14);
            let mut i = input(&h);
            i.aspect = aspect;
            let svg = render(&i);
            let (w, hh) = (SHEET_W, sheet_h(aspect));
            for frag in svg.split("<ellipse class=\"plate-cart\"").skip(1) {
                let num = |key: &str| -> f64 {
                    frag.split(&format!("{key}=\""))
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .and_then(|s| s.parse().ok())
                        .expect(key)
                };
                let (cx, cy, rx, ry) = (num("cx"), num("cy"), num("rx"), num("ry"));
                assert!(
                    cx - rx >= MARK && cx + rx <= w - MARK,
                    "{aspect}: x {cx} ± {rx}"
                );
                assert!(
                    cy - ry >= MARK && cy + ry <= hh - MARK,
                    "{aspect}: y {cy} ± {ry} of {hh}"
                );
            }
        }
    }

    /// The text of every element with this class, in document order.
    fn texts(svg: &str, class: &str) -> Vec<String> {
        svg.split(&format!("class=\"{class}\""))
            .skip(1)
            .filter_map(|rest| {
                let body = rest.split_once('>')?.1;
                Some(body.split_once('<')?.0.to_owned())
            })
            .collect()
    }

    /// The key promises a figure for every number it prints. On a very wide screen with a full
    /// key the key takes the height the figures would have had and the layout leaves the rarest
    /// out; the key used to number every species under the figure limit all the same, so six of
    /// its numbers pointed at nothing.
    #[test]
    fn the_key_numbers_the_figures_that_were_drawn_and_dots_the_rest() {
        let h = heard(40);
        let mut i = input(&h);
        i.aspect = 2.4;
        let svg = render(&i);
        let mut figures = texts(&svg, "plate-num");
        assert!(
            figures.len() < MAX_FIGURES_WIDE,
            "this shape must leave figures out for the test to mean anything; drew {}",
            figures.len()
        );
        let marks = texts(&svg, "plate-key-n");
        assert_eq!(marks.len(), 40);
        let mut numbered: Vec<String> = marks.iter().filter(|m| *m != "·").cloned().collect();
        numbered.sort();
        figures.sort();
        assert_eq!(numbered, figures, "every number in the key has a figure");
        assert_eq!(marks.len() - numbered.len(), 40 - figures.len());
    }

    /// Times are canonical and marked, so the wall display follows the reader's 12 or 24 hour
    /// choice like every other page; the first draft put a rendered `<span>` through
    /// `escape_html` and would have printed its markup.
    #[test]
    fn times_are_marked_for_the_readers_clock_and_never_printed_as_markup() {
        let h = heard(3);
        let svg = render(&input(&h));
        assert!(
            svg.contains(r#"first <tspan data-clk="06:12">06:12</tspan>"#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"latest <tspan data-clk="08:02">08:02</tspan>"#),
            "{svg}"
        );
        assert!(
            !svg.contains("&lt;span") && !svg.contains("&lt;tspan"),
            "{svg}"
        );
    }
}
