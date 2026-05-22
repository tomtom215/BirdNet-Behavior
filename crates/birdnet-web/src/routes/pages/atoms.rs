//! Server-side render helpers for the design-system atoms.
//!
//! These mirror the React reference components in
//! `design_handoff_birdnet_behavior/source/lib/components.jsx`
//! (`SpeciesAvatar`, `ConfBar`, `Sparkline`, `MiniWaveform`) but emit plain
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
#[must_use]
pub(crate) fn species_color(name: &str) -> String {
    format!("oklch(62% 0.13 {})", species_hue(name))
}

/// Circular avatar chip carrying the species' banding code in its own hue.
/// `size` is one of `""` (default 28px), `"sm"`, or `"lg"`.
#[must_use]
pub(crate) fn avatar(common: &str, size: &str) -> String {
    let cls = if size.is_empty() {
        "bnb-avatar".to_string()
    } else {
        format!("bnb-avatar {size}")
    };
    format!(
        r#"<span class="{cls}" style="--sp:{color}" title="{title}">{code}</span>"#,
        color = species_color(common),
        title = escape_html(common),
        code = species_code(common),
    )
}

/// Confidence bar (0–1) with the design's colour thresholds:
/// `> 0.90` moss, `> 0.75` dawn, else neutral.
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
        r#"<span class="bnb-conf {cls}"><span class="track"><span class="fill" style="width:{pct}%"></span></span><span class="val">{v:.2}</span></span>"#,
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
        let _ = write!(out, r#"<span style="height:{h}px"></span>"#);
    }
    out.push_str("</span>");
    out
}

/// Line + area sparkline SVG (max-normalised), styled via `.bnb-spark`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(crate) fn sparkline(data: &[i64], width: f64, height: f64, accent: Option<&str>) -> String {
    if data.is_empty() {
        return String::new();
    }
    let max = data.iter().copied().max().unwrap_or(1).max(1) as f64;
    let n = data.len();
    let step = if n > 1 { width / (n - 1) as f64 } else { width };

    let mut path = String::new();
    for (i, &v) in data.iter().enumerate() {
        let x = i as f64 * step;
        let y = (v as f64 / max).mul_add(-(height - 2.0), height) - 1.0;
        let _ = write!(path, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" });
    }
    let last_x = (n.saturating_sub(1)) as f64 * step;
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

    #[test]
    fn conf_bar_thresholds() {
        assert!(conf_bar(0.95).contains("bnb-conf high"));
        assert!(conf_bar(0.80).contains("bnb-conf mid"));
        assert!(conf_bar(0.50).contains("bnb-conf "));
        assert!(conf_bar(0.95).contains("0.95"));
    }

    #[test]
    fn avatar_carries_code_and_color() {
        let a = avatar("Blue Jay", "lg");
        assert!(a.contains("BLJA"));
        assert!(a.contains("bnb-avatar lg"));
        assert!(a.contains("--sp:oklch"));
    }

    #[test]
    fn waveform_has_requested_bar_count() {
        let w = waveform(42, 24);
        assert_eq!(w.matches("<span style=\"height:").count(), 24);
    }

    #[test]
    fn sparkline_emits_path_or_empty() {
        assert!(sparkline(&[], 56.0, 16.0, None).is_empty());
        let s = sparkline(&[1, 3, 2, 5], 56.0, 16.0, None);
        assert!(s.contains("<svg") && s.contains("class=\"line\""));
    }
}
