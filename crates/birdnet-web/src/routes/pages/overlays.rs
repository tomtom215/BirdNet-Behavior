//! Signal-context overlays for time-axis charts. See O-23 DIFF.md.
//!
//! Three renderers, three concerns. Each emits SVG sized to the caller's
//! coordinate system; callers paint the host chart and then drop our SVG
//! groups in afterwards. Renderers never `format!` colours directly — they
//! lean on the CSS classes added in `app.css.append` so theming works.
//!
//! Moon phase is computed locally (Conway approximation). Weather pulls
//! pre-aggregated rows from a `WeatherStore`. SPL is design-only here.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Moon phase — Conway approximation, ±1 day accurate. No network.
// ---------------------------------------------------------------------------

/// Phase as a 0.0..1.0 fraction of the synodic month.
/// 0 = new moon, 0.5 = full moon, 1.0 wraps back to new.
#[must_use]
pub fn moon_phase_at(unix_seconds: i64) -> f32 {
    const SYNODIC_DAYS: f64 = 29.530_588_853_0;
    // Reference: a known new moon at 2000-01-06 18:14 UTC = 947_182_440 sec.
    let days_since_ref = (unix_seconds - 947_182_440) as f64 / 86_400.0;
    let mut phase = (days_since_ref / SYNODIC_DAYS).fract();
    if phase < 0.0 { phase += 1.0; }
    phase as f32
}

/// One of the four cardinal phase buckets — useful for icon glyphs and labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoonCardinal { New, WaxingHalf, Full, WaningHalf }

impl MoonCardinal {
    #[must_use]
    pub fn from_phase(p: f32) -> Self {
        let p = if p < 0.0 { 0.0 } else if p > 1.0 { 1.0 } else { p };
        match (p * 4.0).round() as i32 % 4 {
            0 => MoonCardinal::New,
            1 => MoonCardinal::WaxingHalf,
            2 => MoonCardinal::Full,
            _ => MoonCardinal::WaningHalf,
        }
    }
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            MoonCardinal::New => "new moon",
            MoonCardinal::WaxingHalf => "first quarter",
            MoonCardinal::Full => "full moon",
            MoonCardinal::WaningHalf => "last quarter",
        }
    }
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            MoonCardinal::New => "○",
            MoonCardinal::WaxingHalf => "◐",
            MoonCardinal::Full => "●",
            MoonCardinal::WaningHalf => "◑",
        }
    }
}

/// Small text-glyph badge for the day-strip header.
#[must_use]
pub fn moon_badge(unix_seconds: i64) -> String {
    let p = moon_phase_at(unix_seconds);
    let c = MoonCardinal::from_phase(p);
    let pct = (p * 100.0).round() as i32;
    format!(
        r#"<span class="bnb-signal bnb-signal--moon"
              data-phase="{}"
              aria-label="moon · {} · {}%">
  <span class="bnb-signal__glyph" aria-hidden="true">{}</span>
  <span class="bnb-signal__label">{}</span>
</span>"#,
        c.label().replace(' ', "-"),
        c.label(),
        pct,
        c.glyph(),
        c.label(),
    )
}

// ---------------------------------------------------------------------------
// Weather — overlay band sized to the host chart.
// ---------------------------------------------------------------------------

/// One hour of weather data. `WeatherStore::range(...)` returns these.
#[derive(Clone, Copy)]
pub struct WeatherSample {
    pub hour: u8,                 // 0..23 local time
    pub temp_c: Option<f32>,
    pub precip_mm: Option<f32>,
    pub wind_kt: Option<f32>,
}

/// Render a weather strip sized to `width` × `height`, anchored at (0,0).
/// Caller transforms / nests as needed (e.g. translate(0, chart_height-22)).
#[must_use]
pub fn weather_band(samples: &[WeatherSample], width: f64, height: f64) -> String {
    if samples.is_empty() {
        return format!(
            r#"<g class="bnb-signal bnb-signal--weather" data-state="empty">
  <rect x="0" y="0" width="{width}" height="{height}" class="bnb-signal__placeholder"/>
</g>"#,
            width = width, height = height
        );
    }
    // Temperature wave: poly-line across samples, normalized to the band height.
    let (t_min, t_max) = samples.iter().filter_map(|s| s.temp_c).fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(lo, hi), v| (lo.min(v), hi.max(v)),
    );
    let span = (t_max - t_min).max(1.0);
    let step = width / samples.len().max(1) as f64;

    let mut out = String::new();
    let _ = write!(
        out,
        r#"<g class="bnb-signal bnb-signal--weather" aria-label="hourly weather">
  <rect x="0" y="0" width="{width}" height="{height}" class="bnb-signal__bg"/>"#,
        width = width, height = height
    );

    // Temperature wave path
    out.push_str(r#"<path class="bnb-signal__temp" d=""#);
    for (i, s) in samples.iter().enumerate() {
        let x = i as f64 * step + step / 2.0;
        let t = s.temp_c.unwrap_or(t_min);
        let y = height - (((t - t_min) / span) as f64) * (height - 4.0) - 2.0;
        let _ = write!(out, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" });
    }
    out.push_str(r#""/>"#);

    // Precipitation droplets — one tick per hour with any precip.
    for (i, s) in samples.iter().enumerate() {
        let mm = s.precip_mm.unwrap_or(0.0);
        if mm <= 0.0 { continue; }
        let x = i as f64 * step + step / 2.0;
        let h = (mm.min(6.0) as f64 / 6.0) * height;
        let _ = write!(
            out,
            r#"<line class="bnb-signal__precip" x1="{x:.1}" y1="{y1:.1}" x2="{x:.1}" y2="{y2:.1}"/>"#,
            x = x, y1 = height - h, y2 = height
        );
    }

    out.push_str("</g>");
    out
}

// ---------------------------------------------------------------------------
// SPL (design-only — needs audio-daemon write before this renders real data)
// ---------------------------------------------------------------------------

/// Render an SPL band placeholder sized to `width` × `height`. The renderer
/// is finished; the data source is not. Until the audio daemon writes
/// `spl_minutes`, the band shows a quiet "no signal" line.
#[must_use]
pub fn spl_band(samples: Option<&[(u32, f32)]>, width: f64, height: f64) -> String {
    let Some(s) = samples else {
        return format!(
            r#"<g class="bnb-signal bnb-signal--spl" data-state="empty">
  <line x1="0" y1="{mid:.1}" x2="{w}" y2="{mid:.1}" class="bnb-signal__quiet"/>
  <title>SPL data not available on this station</title>
</g>"#,
            mid = height / 2.0, w = width
        );
    };
    if s.is_empty() {
        return spl_band(None, width, height);
    }
    let max = s.iter().map(|(_, v)| *v).fold(0.0_f32, f32::max).max(40.0);
    let min = 20.0_f32;
    let span = max - min;
    let step = width / s.len() as f64;

    let mut out = String::from(
        r#"<g class="bnb-signal bnb-signal--spl" aria-label="ambient SPL">"#,
    );
    out.push_str(r#"<path class="bnb-signal__spl" d=""#);
    for (i, (_, v)) in s.iter().enumerate() {
        let x = i as f64 * step + step / 2.0;
        let frac = ((v - min) / span).clamp(0.0, 1.0) as f64;
        let y = height - frac * (height - 4.0) - 2.0;
        let _ = write!(out, "{}{x:.1},{y:.1}", if i == 0 { "M" } else { "L" });
    }
    out.push_str(r#""/></g>"#);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_phase_is_bounded() {
        for ts in [0_i64, 1_700_000_000, 2_000_000_000] {
            let p = moon_phase_at(ts);
            assert!((0.0..1.0).contains(&p), "phase out of range: {p} for ts {ts}");
        }
    }

    #[test]
    fn cardinal_partitions_into_four_buckets() {
        let labels: Vec<_> = (0..=8).map(|i| MoonCardinal::from_phase(i as f32 / 8.0).label()).collect();
        // Strictly: 0,1,2,3 → new, waxing, full, waning, then wraps.
        assert_eq!(labels[0], "new moon");
        assert_eq!(labels[4], "full moon");
        assert_eq!(labels[8], "new moon");
    }

    #[test]
    fn weather_band_renders_path_when_samples_present() {
        let samples: Vec<_> = (0..24).map(|h| WeatherSample {
            hour: h as u8,
            temp_c: Some(10.0 + (h as f32) * 0.3),
            precip_mm: if h == 12 { Some(2.0) } else { None },
            wind_kt: Some(5.0),
        }).collect();
        let svg = weather_band(&samples, 1380.0, 22.0);
        assert!(svg.contains(r#"class="bnb-signal__temp""#));
        assert!(svg.contains(r#"class="bnb-signal__precip""#));
        assert!(!svg.contains(r#"data-state="empty""#));
    }

    #[test]
    fn weather_band_renders_placeholder_when_empty() {
        let svg = weather_band(&[], 1380.0, 22.0);
        assert!(svg.contains(r#"data-state="empty""#));
        assert!(svg.contains(r#"class="bnb-signal__placeholder""#));
    }

    #[test]
    fn spl_band_renders_quiet_line_when_no_data() {
        let svg = spl_band(None, 1380.0, 22.0);
        assert!(svg.contains(r#"class="bnb-signal__quiet""#));
    }
}
