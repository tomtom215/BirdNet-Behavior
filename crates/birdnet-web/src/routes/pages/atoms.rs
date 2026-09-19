//! Server-side render helpers for the design-system atoms.
//!
//! These mirror the React reference components from the original design
//! prototype (`SpeciesAvatar`, `ConfBar`, `Sparkline`, `MiniWaveform`) but emit plain
//! HTML/SVG strings styled by `static/css/app.css`. Colours are derived
//! deterministically from the species name so a given bird is always the same
//! hue without needing a persisted colour column.

use std::fmt::Write as _;

use super::escape_html;

/// Derive a 4-letter banding-style code from a common name.
///
/// Approximates the alpha codes birders use: two words → first two letters of
/// each (`Northern Cardinal` → `NOCA`); three words (hyphens count as splits)
/// → 1+1+2 (`Black-capped Chickadee` → `BCCH`); one word → first four letters.
#[must_use]
pub(crate) fn species_code(common: &str) -> String {
    let words: Vec<&str> = common
        .split(|c: char| c.is_whitespace() || c == '-' || c == '\'')
        .filter(|w| !w.is_empty())
        .collect();

    let take = |w: &str, n: usize| -> String {
        w.chars()
            .filter(char::is_ascii_alphabetic)
            .take(n)
            .collect::<String>()
    };

    let raw = match words.len() {
        0 => return "????".to_string(),
        1 => take(words[0], 4),
        2 => format!("{}{}", take(words[0], 2), take(words[1], 2)),
        3 => format!(
            "{}{}{}",
            take(words[0], 1),
            take(words[1], 1),
            take(words[2], 2)
        ),
        _ => words.iter().take(4).map(|w| take(w, 1)).collect(),
    };

    let mut code: String = raw.to_uppercase();
    while code.chars().count() < 4 {
        code.push('·');
    }
    code.chars().take(4).collect()
}

/// Stable hue (0–359) hashed from the species name (FNV-1a).
fn species_hue(name: &str) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    u32::try_from(hash % 360).unwrap_or(150)
}

/// Deterministic OKLCH colour for a species, usable in both themes.
///
/// # Use this for chips, not for charts
///
/// The hue is a hash, with nothing spacing it, and lightness and chroma are
/// constant. That is exactly right for an avatar chip, where one species is
/// shown on its own and all that matters is that it looks the same every time.
///
/// It is wrong wherever several species are shown **together**, because two
/// hashes can land arbitrarily close. Measured on the fixture's own
/// eight-species streamgraph: Northern Cardinal at hue 89 and American Robin at
/// 91 (2° apart), Mourning Dove at 120 and Tufted Titmouse at 123 (3°) — two
/// pairs indistinguishable on screen. Nor is that bad luck with those species:
/// drawing N hues uniformly from 360 with no spacing, the probability that
/// *some* pair lands within 25° — about where constant lightness and chroma
/// stop being separable — is 92.7 % at N=6, 99.6 % at N=8 and 100 % by N=12.
///
/// Constant `L = 62%` compounds it: with no lightness variation the palette
/// collapses in greyscale and for the common colour-vision deficiencies, where
/// two series differing only in hue are two identical greys.
///
/// Multi-series charts use [`series_color`] instead.
#[must_use]
pub(crate) fn species_color(name: &str) -> String {
    format!("oklch(62% 0.13 {})", species_hue(name))
}

/// Colour for series `index` of `total` in a multi-series chart.
///
/// Assigned by **rank within the chart** rather than by hashing the name, which
/// is what makes the colours separable: the hues are spread evenly around the
/// circle, so N series are always N maximally-distant hues instead of N
/// independent samples that may collide (see [`species_color`] for the
/// measured collision rates).
///
/// Two further properties the hash could not provide:
///
/// * **Alternating lightness.** Consecutive series step between two lightness
///   levels, so adjacent bands differ in *value* as well as hue. That is what
///   keeps them distinguishable in greyscale and under deuteranopia, where hue
///   alone conveys nothing.
///
/// Even spacing rather than a golden-angle walk. A first draft stepped
/// `index * 5 % total` to push early indices apart, which silently degenerates
/// whenever `total` shares a factor with the step: at `total == 5` every series
/// got hue 0, i.e. one colour for the whole chart. Even spacing has no such
/// case and is optimal anyway — for N colours on a circle, `360/N` apart is the
/// largest achievable minimum separation.
///
/// The trade is that a species' colour is stable *within* a chart but not
/// across charts with different membership. For a legend-bearing comparison
/// that is the right way round: telling this band from that one matters more
/// than recognising a colour from another page, and the chips (which do keep a
/// stable colour) carry that continuity.
#[must_use]
pub(crate) fn series_color(index: usize, total: usize) -> String {
    let n = total.max(1);
    let hue = (index % n) * 360 / n;
    // Alternate between a lighter and a darker level so adjacent series differ
    // in value, not only in hue.
    let lightness = if index.is_multiple_of(2) { 68 } else { 52 };
    format!("oklch({lightness}% 0.14 {hue})")
}

/// Circular avatar chip: the species' **photograph**, over its banding code in
/// its own hue. `size` is one of `""` (default 28px), `"sm"`, or `"lg"`.
///
/// # Why the code is still there
///
/// It is the fallback, and it is reached often. `/api/v2/species/image/{sci}/file`
/// answers 404 whenever the station has no picture for that bird — the image
/// cache is switched off (`--image-cache-dir ""`), Wikipedia has no photo on
/// the species page, the lookup failed, or an admin blacklisted the one it
/// found. The `<img>` opts into `data-hide-on-error`, so on any of those the
/// browser uncovers the coloured tile underneath and the row looks exactly as
/// it did before this function grew a second argument. Nothing is drawn in the
/// gap and no request is made twice.
///
/// `alt` is empty on purpose. Every one of the fourteen places this is called
/// prints the species' name in the same row, so a description here would make
/// a screen reader say the bird twice; the photo is decoration beside a name
/// that is already text. The `title` on the chip is unchanged.
///
/// `scientific` may be empty — an imported BirdNET-Pi database can carry a row
/// with no `Sci_Name` — and then no `<img>` is emitted at all, rather than a
/// request for `/api/v2/species/image//file` that could only 404.
#[must_use]
pub(crate) fn avatar(common: &str, scientific: &str, size: &str) -> String {
    let cls = if size.is_empty() {
        "bnb-avatar".to_string()
    } else {
        format!("bnb-avatar {size}")
    };
    let photo = if scientific.is_empty() {
        String::new()
    } else {
        format!(
            r#"<img src="/api/v2/species/image/{enc}/file" alt="" loading="lazy" decoding="async" class="bnb-avatar-img" data-hide-on-error>"#,
            enc = super::simple_url_encode(scientific),
        )
    };
    format!(
        r#"<span class="{cls}" data-style="--sp:{color}" title="{title}">{code}{photo}</span>"#,
        color = species_color(common),
        title = escape_html(common),
        code = species_code(common),
    )
}

/// Confidence bar with the design's colour thresholds:
/// `> 0.90` moss, `> 0.75` dawn, else neutral.
///
/// # Why it reads `97%` and not `0.97`
///
/// `value` is the model's score, which is a probability, and printing it raw
/// put a bare `0.97` beside nearly every bird in the app — the live feed, the
/// history rows, the recordings grid, the species pages, the detail page —
/// with nothing naming the scale or the quantity. The app already used percent
/// everywhere a reader was likely to be a stranger (the public share page) or
/// an analyst (`/admin/quality`), so the two most-read surfaces disagreed with
/// each other about how to write the same number.
///
/// The bar is also given a name. It is `role="img"` with an `aria-label`,
/// because the number alone — read aloud as "ninety-seven percent", next to a
/// bird — does not say what it measures.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(crate) fn conf_bar(value: f64) -> String {
    let v = value.clamp(0.0, 1.0);
    let cls = if v > 0.90 {
        "high"
    } else if v > 0.75 {
        "mid"
    } else {
        ""
    };
    let pct = (v * 100.0).round() as i64;
    format!(
        r#"<span class="bnb-conf {cls}" role="img" aria-label="Confidence {pct}% — how sure the identifier was"><span class="track"><span class="fill" data-style="width:{pct}%"></span></span><span class="val">{pct}%</span></span>"#,
    )
}

/// Deterministic mini call-waveform (24 bars, bell envelope) for a feed row.
/// `seed` keeps a given detection's bars stable across re-renders.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub(crate) fn waveform(seed: u64, bars: usize) -> String {
    let mut s = seed.wrapping_mul(9301).wrapping_add(49297);
    let mut next = || {
        s = (s.wrapping_mul(9301).wrapping_add(49297)) % 233_280;
        s as f64 / 233_280.0
    };
    let mut out = String::from(r#"<span class="waveform" aria-hidden="true">"#);
    for i in 0..bars {
        let t = i as f64 / bars as f64;
        let env = (t * std::f64::consts::PI).sin();
        let v = env.mul_add(0.55 + next() * 0.40, 0.25);
        let h = (v * 22.0).round().clamp(2.0, 22.0) as i64;
        let _ = write!(out, r#"<span data-style="height:{h}px"></span>"#);
    }
    out.push_str("</span>");
    out
}

/// Line + area sparkline SVG (zero-baselined), styled via `.bnb-spark`.
///
/// Two degenerate shapes are handled explicitly, because the arithmetic that
/// serves the general case renders both of them as nothing useful:
///
/// * **One sample.** Stepping `x` by `width / (n - 1)` is undefined at `n == 1`,
///   and the loop emitted a bare `M0,y` — a moveto with no segment after it,
///   which paints no pixels. `dashboard::stats` substitutes `vec![0]` for an
///   empty trend, so a station on its first day shipped an empty box. One
///   sample now draws a flat line across the full width, which is what a single
///   reading looks like.
/// * **No variation at all.** With every sample equal and non-zero,
///   `v / max == 1` puts the whole line at `y = 1`, hard against the top edge,
///   where it reads as a border rather than as data — the Today Top-species
///   rail is mostly flat series and looked like a stack of hairlines. A flat
///   non-zero series is drawn through the middle instead. A flat *zero* series
///   keeps its baseline at the bottom, which is the honest place for it.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn sparkline(data: &[i64], width: f64, height: f64, accent: Option<&str>) -> String {
    if data.is_empty() {
        return String::new();
    }
    let lo = data.iter().copied().min().unwrap_or(0);
    let hi = data.iter().copied().max().unwrap_or(0);
    let flat_non_zero = hi == lo && hi > 0;
    let max = hi.max(1) as f64;
    let n = data.len();
    let step = if n > 1 { width / (n - 1) as f64 } else { width };
    let y_of = |v: i64| -> f64 {
        if flat_non_zero {
            height / 2.0
        } else {
            (v as f64 / max).mul_add(-(height - 2.0), height) - 1.0
        }
    };

    let mut path = String::new();
    for (i, &v) in data.iter().enumerate() {
        let x = i as f64 * step;
        let y = y_of(v);
        let _ = write!(path, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" });
    }
    // A single sample has no second point to draw to, so give it one at the
    // right edge; the area below then has a non-zero width to close against.
    let last_x = if n > 1 {
        (n - 1) as f64 * step
    } else {
        let y = y_of(data[0]);
        let _ = write!(path, "L{width:.1},{y:.1}");
        width
    };
    let area = format!("{path} L{last_x:.1},{height} L0,{height} Z");
    let stroke = accent.unwrap_or("var(--moss)");

    format!(
        r#"<svg class="bnb-spark" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" aria-hidden="true"><path class="area" d="{area}" fill="{stroke}" fill-opacity="0.10"/><path class="line" d="{path}" stroke="{stroke}" fill="none" stroke-width="1.4"/></svg>"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banding_codes_match_convention() {
        assert_eq!(species_code("Northern Cardinal"), "NOCA");
        assert_eq!(species_code("Blue Jay"), "BLJA");
        assert_eq!(species_code("American Robin"), "AMRO");
        assert_eq!(species_code("Black-capped Chickadee"), "BCCH");
        assert_eq!(species_code("White-breasted Nuthatch"), "WBNU");
        assert_eq!(species_code("Mourning Dove"), "MODO");
    }

    #[test]
    fn banding_code_one_word_and_short() {
        assert_eq!(species_code("Dunnock"), "DUNN");
        assert_eq!(species_code("Ou").chars().count(), 4); // padded
    }

    #[test]
    fn species_color_is_stable_and_oklch() {
        let a = species_color("Northern Cardinal");
        assert_eq!(a, species_color("Northern Cardinal"));
        assert!(a.starts_with("oklch("));
    }

    /// Chart series must be separable — the property the hash palette lacked.
    ///
    /// Asserts the minimum pairwise hue separation is the best achievable for
    /// N colours (`360/N`), across every chart size the UI actually renders,
    /// and that consecutive series differ in lightness as well as hue so the
    /// palette survives greyscale and colour-vision deficiency.
    #[test]
    fn series_colours_are_maximally_separated() {
        fn hue_of(css: &str) -> i64 {
            css.trim_end_matches(')')
                .rsplit(' ')
                .next()
                .and_then(|h| h.parse().ok())
                .unwrap_or(-1)
        }
        fn lightness_of(css: &str) -> i64 {
            css.split('(')
                .nth(1)
                .and_then(|r| r.split('%').next())
                .and_then(|l| l.parse().ok())
                .unwrap_or(-1)
        }

        for total in 2..=16_usize {
            let hues: Vec<i64> = (0..total)
                .map(|i| hue_of(&series_color(i, total)))
                .collect();
            assert!(hues.iter().all(|h| *h >= 0), "hue must parse: {hues:?}");

            let mut min_sep = 360;
            for i in 0..total {
                for j in (i + 1)..total {
                    let d = (hues[i] - hues[j]).abs() % 360;
                    min_sep = min_sep.min(d.min(360 - d));
                }
            }
            let ideal = 360 / i64::try_from(total).unwrap();
            assert_eq!(
                min_sep, ideal,
                "at {total} series the closest pair is {min_sep}° apart; the best \
                 achievable is {ideal}°. The hash palette this replaced put \
                 Northern Cardinal and American Robin 2° apart."
            );
        }

        // Adjacent series differ in value, not only in hue — what keeps the
        // palette readable in greyscale and under deuteranopia.
        assert_ne!(
            lightness_of(&series_color(0, 8)),
            lightness_of(&series_color(1, 8))
        );
    }

    /// The degenerate case the first draft had.
    ///
    /// `index * 5 % total` collapsed to a single hue whenever `total` was a
    /// multiple of 5 — a five-species chart drawn entirely in one colour.
    #[test]
    fn a_five_series_chart_is_not_one_colour() {
        let colours: std::collections::BTreeSet<String> =
            (0..5).map(|i| series_color(i, 5)).collect();
        assert_eq!(colours.len(), 5, "every series needs its own colour");
    }

    #[test]
    fn conf_bar_thresholds() {
        assert!(conf_bar(0.95).contains("bnb-conf high"));
        assert!(conf_bar(0.80).contains("bnb-conf mid"));
        assert!(conf_bar(0.50).contains("bnb-conf "));
    }

    /// The number beside a bird has to say what it is.
    ///
    /// A bare `0.95` was printed on every detection in the app, on a scale
    /// nothing named, in a quantity nothing named. Both halves are asserted:
    /// the visible text is a percentage, and the bar carries an accessible
    /// name, because `95%` read aloud on its own is no better than `0.95`.
    #[test]
    fn a_confidence_reads_as_a_percentage_and_says_what_it_measures() {
        let bar = conf_bar(0.95);
        assert!(
            bar.contains(">95%<"),
            "the visible value must be percent: {bar}"
        );
        assert!(
            !bar.contains("0.95"),
            "the bare 0-to-1 form must be gone: {bar}"
        );
        assert!(
            bar.contains(r#"role="img""#) && bar.contains("Confidence 95%"),
            "the bar needs an accessible name: {bar}"
        );
    }

    /// Rounding has to stay on the visible number. `0.955` reading as `96%`
    /// while the bar fills to 95.5% would be two different answers in one
    /// control.
    #[test]
    fn the_bar_and_its_number_round_together() {
        let bar = conf_bar(0.955);
        assert!(bar.contains("width:96%"), "{bar}");
        assert!(bar.contains(">96%<"), "{bar}");
    }

    #[test]
    fn avatar_carries_code_and_color() {
        let a = avatar("Blue Jay", "Cyanocitta cristata", "lg");
        assert!(a.contains("BLJA"));
        assert!(a.contains("bnb-avatar lg"));
        assert!(a.contains("--sp:oklch"));
    }

    #[test]
    fn waveform_has_requested_bar_count() {
        let w = waveform(42, 24);
        assert_eq!(w.matches("<span data-style=\"height:").count(), 24);
    }

    #[test]
    fn sparkline_emits_path_or_empty() {
        assert!(sparkline(&[], 56.0, 16.0, None).is_empty());
        let s = sparkline(&[1, 3, 2, 5], 56.0, 16.0, None);
        assert!(s.contains("<svg") && s.contains("class=\"line\""));
    }

    /// Pull the `d` attribute of the `class="line"` path.
    fn line_path(svg: &str) -> String {
        let after = svg
            .split_once("class=\"line\" d=\"")
            .expect("sparkline draws a line path")
            .1;
        after
            .split_once('"')
            .expect("d is terminated")
            .0
            .to_string()
    }

    /// A sparkline handed one sample must draw something.
    ///
    /// Observed failing against the pre-fix renderer: with `n == 1` the loop
    /// emitted a single `M0.0,y` — a moveto with no drawing command after it,
    /// which paints nothing — and the area closed to `M0.0,y L0.0,h L0,h Z`,
    /// a zero-width triangle. `dashboard::stats` deliberately constructs this
    /// case (`if trend.is_empty() { trend = vec![0] }`), so a station on its
    /// first day shipped an empty 200x26 box where its Detections trend should
    /// be, and every species with exactly one day of history did the same in
    /// the Top-species rail.
    #[test]
    fn single_sample_sparkline_is_visible() {
        for v in [0_i64, 1, 4, 900] {
            let svg = sparkline(&[v], 200.0, 26.0, None);
            let d = line_path(&svg);
            assert!(
                d.contains('L'),
                "one sample (v = {v}) drew no line segment: d = {d:?}"
            );
            // The segment must span the box, not collapse onto x = 0.
            assert!(
                d.contains("200.0"),
                "one sample (v = {v}) did not reach the full width: d = {d:?}"
            );
        }
    }

    /// A series with no variation must not be pinned to the top of its box.
    ///
    /// Observed failing against the pre-fix renderer, whose normalisation put
    /// every sample of a perfectly flat series at `v / max == 1`, i.e. `y = 1`
    /// — hard against the top edge, where a 1.4px stroke reads as a rule or a
    /// border rather than as data. The Today Top-species rail is mostly flat
    /// series (4, 4, 4, 4), and in a screenshot they were indistinguishable
    /// from hairlines.
    #[test]
    fn flat_series_sits_off_the_top_edge() {
        let h = 26.0_f64;
        for v in [1_i64, 4, 4096] {
            let d = line_path(&sparkline(&[v, v, v, v], 200.0, h, None));
            for y in d
                .split(['M', 'L'])
                .filter(|s| !s.is_empty())
                .filter_map(|pt| pt.split(',').nth(1))
                .filter_map(|y| y.trim().parse::<f64>().ok())
            {
                assert!(
                    y > 2.0 && y < h - 2.0,
                    "flat series at v = {v} drew y = {y}, which is against an edge of the {h}-high box"
                );
            }
        }
    }

    /// The counterpart: a series that genuinely varies must still use the full
    /// height, so the fix above cannot be satisfied by flattening everything
    /// into the middle.
    #[test]
    fn varying_series_still_spans_the_box() {
        let h = 26.0_f64;
        let d = line_path(&sparkline(&[0, 5, 0, 5], 200.0, h, None));
        let ys: Vec<f64> = d
            .split(['M', 'L'])
            .filter(|s| !s.is_empty())
            .filter_map(|pt| pt.split(',').nth(1))
            .filter_map(|y| y.trim().parse::<f64>().ok())
            .collect();
        let lo = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            hi - lo > h * 0.7,
            "a 0..5..0..5 series only spanned {:.1} of {h}px",
            hi - lo
        );
    }
}
