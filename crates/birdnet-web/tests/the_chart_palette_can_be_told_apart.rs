//! The eight chart colours are told apart by a person who cannot see red, in
//! both themes, on the surface a chart is drawn on.
//!
//! Before this, a chart's colours came from `series_color(rank, total)`: the
//! hue wheel cut into `total` equal slices, lightness alternating 68/52 at
//! chroma 0.14. Alternating lightness kept neighbours apart, which was the right
//! instinct; but two of the eight were outside sRGB (so the browser painted
//! something else), three were under 3:1 against the card, nothing had been
//! checked for red-green colour blindness, and the dark theme reused the same
//! lightness, which is above the dark band: a "green" series was a lime that
//! glowed against the grey text beside it. A palette is not something to judge
//! by eye: whether two colours are confusable is arithmetic, so this does the
//! arithmetic.
//!
//! # What is held
//!
//! For `--chart-1` … `--chart-8`, in the light and in the dark block of
//! `app.css`, on that theme's card:
//!
//! 1. **Lightness band** (OKLCH L): 0.43–0.77 light, 0.48–0.67 dark. Outside it
//!    a mark is a smudge on paper or a glare on night.
//! 2. **Chroma floor** (OKLCH C ≥ 0.10): below it a hue reads as grey.
//! 3. **Adjacent separation under colour blindness** (`OKLab` ΔE ×100 under the
//!    Machado 2009 protan and deutan simulations) ≥ 8 for every neighbouring
//!    pair, which is the pairing a stacked area or a ring chart puts side by side.
//! 4. **Adjacent separation for everyone** ≥ 15, unsimulated.
//! 5. **Contrast against the card** ≥ 3:1 (WCAG 1.4.11) — except the slots in
//!    [`NEEDS_A_LABEL`], whose lightness is what makes them read as the colour
//!    they are (amber at 3:1 is brown). Every chart that uses them prints the
//!    name beside the mark and offers a table.
//! 6. The tokens are inside sRGB, so the number computed here is the number on
//!    the screen and not what a browser's gamut mapping makes of it.
//!
//! The arithmetic is checked against the reference palette in the dataviz
//! method's own documentation, whose published figures this must reproduce, so a
//! slip in the colour maths shows up as that test failing and not as a palette
//! that is quietly graded by the wrong ruler.
//!
//! A second gate holds the other half: a chart that does not read these tokens
//! is not graded by any of this. Every chart renderer is scanned for a mark
//! painted in a UI colour (`--moss`, `--dawn`, `--rare`, `--info`, the accent
//! triple) or a colour literal.
// Colour arithmetic is r, g, b, l, m, s, a, b by every convention there is.
#![allow(
    clippy::many_single_char_names,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::similar_names
)]

use std::collections::BTreeMap;
use std::path::Path;

fn read(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        match rest[open + 2..].find("*/") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// `--name: value` pairs of the first block whose selector is `selector`.
fn token_block(css: &str, selector: &str) -> BTreeMap<String, String> {
    let needle = format!("{selector} {{");
    let at = css
        .find(&needle)
        .unwrap_or_else(|| panic!("no block `{selector}`"));
    let start = at + needle.len();
    let end = css[start..]
        .find("\n}")
        .map_or_else(|| panic!("block never closes"), |i| start + i);
    let mut out = BTreeMap::new();
    let mut depth = 0_i32;
    let mut cur = String::new();
    let mut flush = |cur: &mut String| {
        let d = cur.trim().to_string();
        cur.clear();
        if let Some(rest) = d.strip_prefix("--")
            && let Some((k, v)) = rest.split_once(':')
        {
            out.insert(format!("--{}", k.trim()), v.trim().to_string());
        }
    };
    for ch in css[start..end].chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ';' if depth == 0 => flush(&mut cur),
            _ => cur.push(ch),
        }
    }
    flush(&mut cur);
    out
}

// ── colour maths ────────────────────────────────────────────────────────────

type Rgb = [f64; 3];

fn encode(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

fn decode(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// `oklch(56% 0.12 150)` → linear sRGB, unclamped (so a colour outside the
/// gamut shows as a channel outside 0–1).
fn oklch_linear(l: f64, c: f64, h: f64) -> Rgb {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = 0.396_337_777_4_f64.mul_add(a, 0.215_803_757_3_f64.mul_add(b, l));
    let m_ = (-0.105_561_345_8_f64).mul_add(a, (-0.063_854_172_8_f64).mul_add(b, l));
    let s_ = (-0.089_484_177_5_f64).mul_add(a, (-1.291_485_548_0_f64).mul_add(b, l));
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    [
        4.076_741_662_1_f64.mul_add(l3, (-3.307_711_591_3_f64).mul_add(m3, 0.230_969_929_2 * s3)),
        (-1.268_438_004_6_f64).mul_add(l3, 2.609_757_401_1_f64.mul_add(m3, -0.341_319_396_5 * s3)),
        (-0.004_196_086_3_f64)
            .mul_add(l3, (-0.703_418_614_7_f64).mul_add(m3, 1.707_614_701_0 * s3)),
    ]
}

/// Parse `oklch(56% 0.12 150)` into `(L 0–1, C, h°)`.
fn parse_oklch(v: &str) -> Option<(f64, f64, f64)> {
    let inner = v.trim().strip_prefix("oklch(")?.strip_suffix(')')?;
    let mut it = inner.split_whitespace();
    let l = it.next()?.trim_end_matches('%').parse::<f64>().ok()? / 100.0;
    let c = it.next()?.parse::<f64>().ok()?;
    let h = it.next()?.parse::<f64>().ok()?;
    Some((l, c, h))
}

/// A colour as the screen has it: eight bits a channel.
#[derive(Clone, Copy, Debug)]
struct Colour {
    srgb8: [u8; 3],
    in_gamut: bool,
}

impl Colour {
    fn from_oklch(v: &str) -> Self {
        let (l, c, h) = parse_oklch(v).unwrap_or_else(|| panic!("not an oklch() colour: `{v}`"));
        let lin = oklch_linear(l, c, h);
        let in_gamut = lin.iter().all(|x| (-0.002..=1.002).contains(x));
        Self {
            srgb8: lin.map(|x| (encode(x) * 255.0).round() as u8),
            in_gamut,
        }
    }

    fn from_hex(h: &str) -> Self {
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("hex");
        Self {
            srgb8: [p(1), p(3), p(5)],
            in_gamut: true,
        }
    }

    fn linear(self) -> Rgb {
        self.srgb8.map(|c| decode(f64::from(c) / 255.0))
    }

    fn luminance(self) -> f64 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn oklab(self) -> [f64; 3] {
        oklab_of(self.linear())
    }

    /// `(L, C)` in OKLCH.
    fn lc(self) -> (f64, f64) {
        let [l, a, b] = self.oklab();
        (l, a.hypot(b))
    }
}

fn oklab_of([r, g, b]: Rgb) -> [f64; 3] {
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// Machado, Oliveira & Fernandes (2009), severity 1.0, on linear RGB.
const PROTAN: [[f64; 3]; 3] = [
    [0.152_286, 1.052_583, -0.204_868],
    [0.114_503, 0.786_281, 0.099_216],
    [-0.003_882, -0.048_116, 1.051_998],
];
const DEUTAN: [[f64; 3]; 3] = [
    [0.367_322, 0.860_646, -0.227_968],
    [0.280_085, 0.672_501, 0.047_413],
    [-0.011_820, 0.042_940, 0.968_881],
];

fn simulate(c: Colour, m: &[[f64; 3]; 3]) -> Rgb {
    let [r, g, b] = c.linear();
    m.map(|row| (row[0] * r + row[1] * g + row[2] * b).clamp(0.0, 1.0))
}

/// `OKLab` distance ×100, optionally through a colour-blindness simulation.
fn delta_e(a: Colour, b: Colour, sim: Option<&[[f64; 3]; 3]>) -> f64 {
    let (la, lb) = sim.map_or_else(
        || (a.oklab(), b.oklab()),
        |m| (oklab_of(simulate(a, m)), oklab_of(simulate(b, m))),
    );
    100.0 * ((la[0] - lb[0]).powi(2) + (la[1] - lb[1]).powi(2) + (la[2] - lb[2]).powi(2)).sqrt()
}

fn contrast(a: Colour, b: Colour) -> f64 {
    let (x, y) = (a.luminance(), b.luminance());
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

// ── the gates ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Mode {
    Light,
    Dark,
}

impl Mode {
    const fn band(self) -> (f64, f64) {
        match self {
            Self::Light => (0.43, 0.77),
            Self::Dark => (0.48, 0.67),
        }
    }
}

const CHROMA_FLOOR: f64 = 0.10;
const CVD_TARGET: f64 = 8.0;
const NORMAL_FLOOR: f64 = 15.0;
const CONTRAST_MIN: f64 = 3.0;

/// Slots (1-based) allowed to sit below 3:1 on the light card, with the reason.
/// They must be named, and every chart that draws them must label the mark.
const NEEDS_A_LABEL: &[(usize, &str)] = &[(
    7,
    "amber: darkened to 3:1 it reads as brown, and a chart of birds should not \
     turn its seventh species brown; the legend, the band's <title> and the \
     table view carry the identity",
)];

struct Verdict {
    problems: Vec<String>,
    worst_cvd: f64,
    worst_normal: f64,
}

fn grade(palette: &[Colour], mode: Mode, card: Colour, relief: &[usize]) -> Verdict {
    let mut problems = Vec::new();
    let (lo, hi) = mode.band();
    for (i, c) in palette.iter().enumerate() {
        let slot = i + 1;
        if !c.in_gamut {
            problems.push(format!(
                "slot {slot} is outside sRGB; the browser will map it"
            ));
        }
        let (l, ch) = c.lc();
        if !(lo..=hi).contains(&l) {
            problems.push(format!("slot {slot}: L {l:.3} outside {lo}–{hi}"));
        }
        if ch < CHROMA_FLOOR {
            problems.push(format!(
                "slot {slot}: chroma {ch:.3} < {CHROMA_FLOOR} reads as grey"
            ));
        }
        let cr = contrast(*c, card);
        if cr < CONTRAST_MIN && !relief.contains(&slot) {
            problems.push(format!(
                "slot {slot}: {cr:.2}:1 against the card, needs {CONTRAST_MIN}:1"
            ));
        }
    }
    let mut worst_cvd = f64::INFINITY;
    let mut worst_normal = f64::INFINITY;
    for (i, w) in palette.windows(2).enumerate() {
        let (a, b) = (i + 1, i + 2);
        for (name, m) in [("protan", &PROTAN), ("deutan", &DEUTAN)] {
            let d = delta_e(w[0], w[1], Some(m));
            worst_cvd = worst_cvd.min(d);
            if d < CVD_TARGET {
                problems.push(format!(
                    "slots {a} and {b} are {d:.1} apart under {name} (< {CVD_TARGET})"
                ));
            }
        }
        let d = delta_e(w[0], w[1], None);
        worst_normal = worst_normal.min(d);
        if d < NORMAL_FLOOR {
            problems.push(format!(
                "slots {a} and {b} are {d:.1} apart (< {NORMAL_FLOOR}) for full colour vision"
            ));
        }
    }
    Verdict {
        problems,
        worst_cvd,
        worst_normal,
    }
}

/// What a theme resolves each token to: the light block, with the dark block
/// laid over it for dark.
fn theme(mode: Mode) -> BTreeMap<String, String> {
    let css = strip_comments(&read("static/css/app.css"));
    let mut map = token_block(&css, ":root,\n:root[data-theme=\"light\"]");
    if matches!(mode, Mode::Dark) {
        map.extend(token_block(&css, ":root[data-theme=\"dark\"]"));
    }
    map
}

/// A token as a colour, following `var(--other)` aliases.
fn colour(map: &BTreeMap<String, String>, key: &str) -> Colour {
    let mut v = map
        .get(key)
        .unwrap_or_else(|| panic!("{key} is not defined"))
        .as_str();
    for _ in 0..6 {
        let Some(name) = v.strip_prefix("var(").and_then(|r| r.strip_suffix(')')) else {
            break;
        };
        v = map
            .get(name.trim())
            .unwrap_or_else(|| panic!("{name} is not defined"))
            .as_str();
    }
    Colour::from_oklch(v)
}

fn chart_tokens(map: &BTreeMap<String, String>) -> Vec<Colour> {
    (1..=8)
        .map(|n| colour(map, &format!("--chart-{n}")))
        .collect()
}

#[test]
fn the_light_chart_palette_is_distinguishable() {
    let light = theme(Mode::Light);
    let card = colour(&light, "--card-top");
    let relief: Vec<usize> = NEEDS_A_LABEL.iter().map(|(s, _)| *s).collect();
    let v = grade(&chart_tokens(&light), Mode::Light, card, &relief);
    assert!(
        v.problems.is_empty(),
        "light chart palette:\n  {}",
        v.problems.join("\n  ")
    );
}

#[test]
fn the_dark_chart_palette_is_distinguishable() {
    let dark = theme(Mode::Dark);
    let card = colour(&dark, "--card-top");
    let v = grade(&chart_tokens(&dark), Mode::Dark, card, &[]);
    assert!(
        v.problems.is_empty(),
        "dark chart palette:\n  {}",
        v.problems.join("\n  ")
    );
}

/// A slot exempted from the contrast gate must still be one the page labels.
#[test]
fn only_the_named_slots_are_exempt_and_each_says_why() {
    for (slot, why) in NEEDS_A_LABEL {
        assert!((1..=8).contains(slot));
        assert!(why.len() > 40, "slot {slot}: say why, in a sentence");
    }
    assert!(
        NEEDS_A_LABEL.len() <= 2,
        "more than two exempt slots is a palette that is too pale"
    );
}

// ── the ruler is checked against a published one ────────────────────────────

/// The dataviz method's reference palette, with the figures its documentation
/// publishes for it: worst adjacent CVD ΔE 9.1 light / 8.4 dark, worst adjacent
/// normal-vision ΔE 19.6 light / 19.3 dark. If this arithmetic does not
/// reproduce them, it is not the same ruler, and nothing above it means much.
#[test]
fn the_arithmetic_reproduces_the_published_reference_figures() {
    let light = [
        "#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948",
    ];
    let dark = [
        "#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300", "#9085e9", "#e66767",
    ];
    for (name, set, cvd, normal, card) in [
        ("light", light, 9.1, 19.6, "#fcfcfb"),
        ("dark", dark, 8.4, 19.3, "#1a1a19"),
    ] {
        let pal: Vec<Colour> = set.iter().map(|h| Colour::from_hex(h)).collect();
        let v = grade(
            &pal,
            if name == "light" {
                Mode::Light
            } else {
                Mode::Dark
            },
            Colour::from_hex(card),
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        assert!(
            (v.worst_cvd - cvd).abs() < 0.15,
            "{name}: worst adjacent CVD ΔE is {:.2}, the reference documents {cvd}",
            v.worst_cvd
        );
        assert!(
            (v.worst_normal - normal).abs() < 0.15,
            "{name}: worst adjacent normal ΔE is {:.2}, the reference documents {normal}",
            v.worst_normal
        );
    }
}

/// The counterpart: the grader finds what it is for. Two neighbours that differ
/// only in lightness, one that is grey, one too pale for a dark card.
#[test]
fn the_grader_reports_confusable_grey_and_out_of_band_colours() {
    let green = Colour::from_oklch("oklch(56% 0.12 150)");
    let near_green = Colour::from_oklch("oklch(60% 0.12 150)");
    let grey = Colour::from_oklch("oklch(56% 0.02 150)");
    let neon = Colour::from_oklch("oklch(78% 0.18 150)");
    let card = Colour::from_oklch("oklch(100% 0 0)");

    let v = grade(&[green, near_green], Mode::Light, card, &[]);
    assert!(
        v.problems.iter().any(|p| p.contains("slots 1 and 2")),
        "{:?}",
        v.problems
    );

    let v = grade(&[grey], Mode::Light, card, &[]);
    assert!(
        v.problems.iter().any(|p| p.contains("reads as grey")),
        "{:?}",
        v.problems
    );

    // The dark theme's old `--moss` as a chart colour.
    let v = grade(
        &[neon],
        Mode::Dark,
        Colour::from_oklch("oklch(20% 0.01 250)"),
        &[],
    );
    assert!(
        v.problems.iter().any(|p| p.contains("outside 0.48")),
        "{:?}",
        v.problems
    );

    // And a clean pair must stay clean: green beside a rose.
    let rose = Colour::from_oklch("oklch(60% 0.18 15)");
    let blue = Colour::from_oklch("oklch(58% 0.14 250)");
    let v = grade(&[green, blue, rose], Mode::Light, card, &[]);
    assert!(v.problems.is_empty(), "{:?}", v.problems);
}

/// What the charts drew before: the hue wheel cut into `n` equal slices with
/// lightness alternating 68 / 52 at chroma 0.14 (`atoms::series_color`). Alternating
/// lightness did keep neighbours apart; what it did not do is stay inside sRGB
/// (two of the eight are out of gamut at that chroma, so the browser painted
/// something else), clear 3:1 against the card (three of the eight do not), or
/// step down for the dark theme (it reused the same lightness, which is above the
/// dark band, which is the glow). Held here as data, to show the ruler would have
/// caught it.
#[test]
fn the_hue_sweep_it_replaced_would_not_have_passed() {
    let sweep: Vec<Colour> = (0..8)
        .map(|i| {
            let l = if i % 2 == 0 { 68 } else { 52 };
            Colour::from_oklch(&format!("oklch({l}% 0.14 {})", i * 360 / 8))
        })
        .collect();
    let card = Colour::from_oklch("oklch(100% 0 0)");
    let v = grade(&sweep, Mode::Light, card, &[]);
    assert!(
        v.problems.iter().any(|p| p.contains("outside sRGB")),
        "the old sweep leaves the gamut: {:?}",
        v.problems
    );
    assert!(
        v.problems.iter().any(|p| p.contains("against the card")),
        "and has marks under 3:1: {:?}",
        v.problems
    );
    let dark_card = Colour::from_oklch("oklch(20% 0.01 250)");
    let v = grade(&sweep, Mode::Dark, dark_card, &[]);
    assert!(
        v.problems.iter().any(|p| p.contains("outside 0.48")),
        "and sits above the dark band: {:?}",
        v.problems
    );
}

/// `--chart-other` is what the ninth series and everything after it is drawn in;
/// a ninth hue would be a ninth thing to tell apart.
#[test]
fn the_everything_else_colour_is_a_neutral_that_can_be_seen() {
    for mode in [Mode::Light, Mode::Dark] {
        let map = theme(mode);
        let other = colour(&map, "--chart-other");
        let card = colour(&map, "--card-top");
        let (_, chroma) = other.lc();
        assert!(
            chroma < 0.03,
            "--chart-other has chroma {chroma:.3}: it is a hue, not a neutral"
        );
        assert!(
            contrast(other, card) >= CONTRAST_MIN,
            "--chart-other is {:.2}:1 against the card",
            contrast(other, card)
        );
    }
}

/// The sequential ramp: lightness goes one way, the first step is not an empty
/// cell, the last is not lost on the card.
#[test]
fn the_heat_ramp_runs_one_way_and_its_ends_can_be_seen() {
    for (mode, name, darker_means_more) in
        [(Mode::Light, "light", true), (Mode::Dark, "dark", false)]
    {
        let map = theme(mode);
        let stops: Vec<Colour> = ["--heat-lo", "--heat-mid", "--heat-hi"]
            .iter()
            .map(|k| colour(&map, k))
            .collect();
        assert!(
            stops.iter().all(|c| c.in_gamut),
            "{name}: a heat stop is outside sRGB"
        );
        for w in stops.windows(2) {
            let (a, b) = (w[0].lc().0, w[1].lc().0);
            let step = if darker_means_more { a - b } else { b - a };
            assert!(
                step >= 0.12,
                "{name}: the ramp must move {} by at least 0.12 a step (moved {step:.3})",
                if darker_means_more {
                    "darker"
                } else {
                    "lighter"
                }
            );
        }
        let first_step = delta_e(stops[0], colour(&map, "--surface-2"), None);
        assert!(
            first_step >= 8.0,
            "{name}: a cell with one detection is only {first_step:.1} from an empty cell; it reads as none"
        );
        let card = colour(&map, "--card-top");
        assert!(
            contrast(stops[2], card) >= CONTRAST_MIN,
            "{name}: the top of the ramp is {:.2}:1 against the card",
            contrast(stops[2], card)
        );
    }
}

// ── A chart does not borrow a UI colour ─────────────────────────────────────

/// The files that draw a chart, relative to the crate root.
const CHART_RENDERERS: &[&str] = &[
    "src/routes/pages/viz/funnel.rs",
    "src/routes/pages/viz/matrix.rs",
    "src/routes/pages/viz/radial.rs",
    "src/routes/pages/viz/timeline.rs",
    "src/routes/pages/charts.rs",
    "src/routes/pages/heatmap.rs",
    "src/routes/pages/dawn_chorus.rs",
    "src/routes/pages/dashboard/heatmap_widget.rs",
    "src/routes/pages/atoms.rs",
];

/// UI colours a mark must not be painted in. `--moss` is a success and a link,
/// `--dawn` a warning, `--rare` an alarm, `--info` a notice; `--accent-rgb` is
/// the old moss as a number triple. A chart that reads them is not graded by
/// anything above, and in dark `--moss` is the lime that glows.
const UI_COLOURS: &[&str] = &[
    "var(--moss",
    "var(--dawn",
    "var(--rare",
    "var(--info",
    "--accent-rgb",
];

/// (file, substring, why): a use that is the thing itself and not a series.
const NOT_A_SERIES: &[(&str, &str, &str)] = &[
    (
        "src/routes/pages/dawn_chorus.rs",
        "var(--dawn)",
        "the sunrise marker is the sun",
    ),
    (
        "src/routes/pages/dawn_chorus.rs",
        "var(--dawn-ink",
        "the sunset marker and the moon-phase arc are the sun and the moon",
    ),
    (
        "src/routes/pages/viz/timeline.rs",
        "var(--dawn-ink",
        "the sunrise and sunset glyphs on the day strip are the sun",
    ),
];

/// Non-test, non-comment source of `rel`.
fn production_source(rel: &str) -> String {
    let text = read(rel);
    let text = text.split("#[cfg(test)]").next().unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn borrowed_ui_colours(rel: &str, src: &str) -> Vec<String> {
    let clip = |l: &str| l.trim().chars().take(90).collect::<String>();
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        for ui in UI_COLOURS {
            if !line.contains(ui) {
                continue;
            }
            let excused = NOT_A_SERIES
                .iter()
                .any(|(f, sub, _)| *f == rel && line.contains(sub));
            if !excused {
                out.push(format!("{rel}: `{ui}` in `{}`", clip(line)));
            }
        }
        // A literal colour in a mark is the same fault by another route.
        let literal = ["fill=\"oklch(", "stroke=\"oklch(", "fill=\"#", "stroke=\"#"]
            .iter()
            .any(|p| line.contains(p));
        if literal {
            out.push(format!(
                "{rel}: a colour literal on line {} `{}`",
                n + 1,
                clip(line)
            ));
        }
    }
    out
}

#[test]
fn a_chart_mark_is_painted_in_the_chart_palette_and_nowhere_else() {
    let mut found = Vec::new();
    for rel in CHART_RENDERERS {
        found.extend(borrowed_ui_colours(rel, &production_source(rel)));
    }
    assert!(
        found.is_empty(),
        "{} chart mark(s) read a UI colour instead of --chart-N / --heat-*:\n  {}",
        found.len(),
        found.join("\n  ")
    );
}

/// The counterpart: the scanner finds a borrowed colour, a literal, and lets the
/// two named exceptions through.
#[test]
fn the_borrowing_scanner_finds_what_it_is_for() {
    let bad = "<rect fill=\"var(--moss)\"/>\n<path stroke=\"oklch(60% 0.1 20)\"/>\nbackground:rgba(var(--accent-rgb),.2)";
    let v = borrowed_ui_colours("x.rs", bad);
    assert!(v.iter().any(|m| m.contains("var(--moss")), "{v:?}");
    assert!(v.iter().any(|m| m.contains("colour literal")), "{v:?}");
    assert!(v.iter().any(|m| m.contains("accent-rgb")), "{v:?}");

    let ok = "<circle fill=\"var(--dawn)\"/> <rect fill=\"var(--chart-1)\"/> <text fill=\"var(--fg-3)\"/>";
    assert!(borrowed_ui_colours("src/routes/pages/dawn_chorus.rs", ok).is_empty());
    // The same line in a file that is not excused is a finding.
    assert!(!borrowed_ui_colours("src/routes/pages/charts.rs", ok).is_empty());
}

// ── the plate chart style ───────────────────────────────────────────────────

/// The token a rule paints with: `prop: var(--name)` inside the first `selector {` block of `css`.
fn var_in_rule(css: &str, selector: &str, prop: &str) -> String {
    let open = format!("{selector} {{");
    let at = css
        .find(&open)
        .unwrap_or_else(|| panic!("app.css has no rule `{selector}`"));
    let body = &css[at + open.len()..];
    let body = &body[..body.find('}').expect("an unclosed rule")];
    let value = body
        .split(';')
        .find_map(|d| d.trim().strip_prefix(&format!("{prop}:")))
        .unwrap_or_else(|| panic!("`{selector}` does not set {prop}"));
    value
        .trim()
        .strip_prefix("var(")
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or_else(|| panic!("`{selector}` sets {prop} to {value}, not a token"))
        .trim()
        .to_owned()
}

/// Under Display → Charts → Plate a series is its own colour with strokes cut across it, and a stroke is
/// a graphic a reader needs (it is what tells two neighbours apart when their colours are confused), so
/// it has to be seen against every colour it crosses: WCAG 1.4.11, 3:1. The stroke colour is read from
/// the stylesheet, so what is graded is what ships.
#[test]
fn a_plate_stroke_can_be_seen_on_every_colour_it_crosses() {
    let css = strip_comments(&read("static/css/app.css"));
    for (mode, selector) in [
        (Mode::Light, r#":root[data-illustration="plate"]"#),
        (
            Mode::Dark,
            r#":root[data-illustration="plate"][data-theme="dark"]"#,
        ),
    ] {
        let map = theme(mode);
        let ink = colour(&map, &var_in_rule(&css, selector, "--hx-ink"));
        for n in 1..=8 {
            let series = colour(&map, &format!("--chart-{n}"));
            assert!(
                contrast(ink, series) >= CONTRAST_MIN,
                "{selector}: the stroke is {:.2}:1 on --chart-{n}, needs {CONTRAST_MIN}:1",
                contrast(ink, series)
            );
        }
        // The grey series draws its own, quieter stroke.
        let other_ink = colour(&map, &var_in_rule(&css, ".hx-other line", "stroke"));
        let other = colour(&map, "--chart-other");
        assert!(
            contrast(other_ink, other) >= CONTRAST_MIN,
            "{selector}: the grey series' stroke is {:.2}:1 on --chart-other, needs {CONTRAST_MIN}:1",
            contrast(other_ink, other)
        );
    }
}

/// The counterpart: the choices this one replaced do fail, so the check above can fail. A light stroke on
/// the light theme (paper on `--chart-7`, a pale amber) and a light stroke on the dark theme (the text
/// colour on the dark series) were the first two attempts and measured 2.09:1 and 2.89:1.
#[test]
fn the_strokes_that_were_tried_and_dropped_would_fail_the_same_check() {
    let light = theme(Mode::Light);
    let paper_on_amber = contrast(colour(&light, "--paper"), colour(&light, "--chart-7"));
    assert!(
        paper_on_amber < CONTRAST_MIN,
        "paper on --chart-7 in light is {paper_on_amber:.2}:1"
    );
    let dark = theme(Mode::Dark);
    let worst = (1..=8)
        .map(|n| {
            contrast(
                colour(&dark, "--fg"),
                colour(&dark, &format!("--chart-{n}")),
            )
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        worst < CONTRAST_MIN,
        "the text colour on the dark series is at best {worst:.2}:1 at its worst slot"
    );
}
