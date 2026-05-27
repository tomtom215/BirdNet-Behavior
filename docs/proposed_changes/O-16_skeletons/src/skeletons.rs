//! Skeleton-loading helpers — shape-preserving placeholders for every htmx
//! swap target in the dashboard. See O-16 DIFF.md for the migration table.
//!
//! All helpers return `&'static str` or `String` (no allocations for fixed
//! shapes) so they're cheap to embed directly in template render output.
//!
//! Reduced-motion is honoured by the CSS — see `app.css` (.bnb-skel rules).
//! Each helper sets `aria-busy="true"` on the wrapper so a screen reader
//! announces the loading state exactly once.

use std::fmt::Write as _;

/// Eight feed rows. Use for `#today-results`, dashboard live feed first paint.
#[must_use]
pub fn feed_rows(n: usize) -> String {
    let n = n.clamp(1, 20);
    let mut out = String::from(r#"<div class="feed" aria-busy="true" aria-live="polite">"#);
    for _ in 0..n {
        out.push_str(
            r#"<div class="feed-row bnb-skel-feed-row">
  <span class="bnb-skel line" style="width:46px;"></span>
  <span class="bnb-skel avatar"></span>
  <div><span class="bnb-skel line" style="width:60%;"></span><span class="bnb-skel line" style="width:40%;margin-top:6px;"></span></div>
  <span class="bnb-skel" style="height:22px;border-radius:3px;"></span>
  <span class="bnb-skel" style="height:14px;border-radius:3px;"></span>
  <span class="bnb-skel" style="width:64px;height:22px;border-radius:6px;"></span>
</div>"#,
        );
    }
    out.push_str("</div>");
    out
}

/// 4-up (or 6-up) stat row matching `.stat-row` + `.stat-tile` shape.
#[must_use]
pub fn stat_row(n: usize) -> String {
    let n = n.clamp(2, 6);
    let mut out = format!(
        r#"<div class="bnb-skel-stat-row" style="--n:{n};" aria-busy="true" aria-label="Loading statistics">"#
    );
    for _ in 0..n {
        out.push_str(
            r#"<div>
  <span class="bnb-skel line" style="width:60%;"></span>
  <span class="bnb-skel line xl" style="width:50%;margin-top:8px;"></span>
  <span class="bnb-skel line" style="width:70%;margin-top:auto;"></span>
</div>"#,
        );
    }
    out.push_str("</div>");
    out
}

/// 24h day strip — a histogram-shaped placeholder plus a dot row underneath.
#[must_use]
pub fn day_strip() -> &'static str {
    r#"<div aria-busy="true" aria-label="Loading day strip">
  <div class="bnb-skel-bars" style="--n:24;height:62px;">
    <span style="height:30%"></span><span style="height:42%"></span><span style="height:58%"></span><span style="height:70%"></span><span style="height:62%"></span><span style="height:48%"></span><span style="height:38%"></span><span style="height:32%"></span><span style="height:28%"></span><span style="height:26%"></span><span style="height:24%"></span><span style="height:22%"></span><span style="height:24%"></span><span style="height:28%"></span><span style="height:36%"></span><span style="height:42%"></span><span style="height:52%"></span><span style="height:60%"></span><span style="height:54%"></span><span style="height:44%"></span><span style="height:34%"></span><span style="height:26%"></span><span style="height:20%"></span><span style="height:18%"></span>
  </div>
  <div style="display:grid;grid-template-columns:repeat(4,1fr);margin-top:8px;">
    <span class="bnb-skel line" style="width:38px;"></span>
    <span class="bnb-skel line" style="width:38px;justify-self:center;"></span>
    <span class="bnb-skel line" style="width:38px;justify-self:center;"></span>
    <span class="bnb-skel line" style="width:38px;justify-self:end;"></span>
  </div>
</div>"#
}

/// Polar plot square (dawn-chorus). Just a centred concentric ring.
#[must_use]
pub fn polar_plot() -> &'static str {
    r#"<div aria-busy="true" aria-label="Loading polar plot" style="aspect-ratio:1;max-width:480px;width:100%;margin:0 auto;position:relative;">
  <span class="bnb-skel box" style="position:absolute;inset:0;border-radius:50%;"></span>
  <span class="bnb-skel box" style="position:absolute;inset:14%;border-radius:50%;background:var(--surface);"></span>
  <span class="bnb-skel box" style="position:absolute;inset:32%;border-radius:50%;"></span>
  <span class="bnb-skel box" style="position:absolute;inset:46%;border-radius:50%;background:var(--surface);"></span>
</div>"#
}

/// Ridgeline placeholder — 8 stacked horizontal "ridges".
#[must_use]
pub fn ridgeline() -> &'static str {
    r#"<div aria-busy="true" aria-label="Loading ridgeline" style="display:flex;flex-direction:column;gap:6px;min-height:340px;">
  <span class="bnb-skel" style="height:34px;border-radius:18px 18px 4px 4px;"></span>
  <span class="bnb-skel" style="height:30px;border-radius:14px 14px 4px 4px;"></span>
  <span class="bnb-skel" style="height:38px;border-radius:18px 18px 4px 4px;"></span>
  <span class="bnb-skel" style="height:32px;border-radius:14px 14px 4px 4px;"></span>
  <span class="bnb-skel" style="height:40px;border-radius:18px 18px 4px 4px;"></span>
  <span class="bnb-skel" style="height:28px;border-radius:14px 14px 4px 4px;"></span>
  <span class="bnb-skel" style="height:36px;border-radius:18px 18px 4px 4px;"></span>
  <span class="bnb-skel" style="height:30px;border-radius:14px 14px 4px 4px;"></span>
</div>"#
}

/// Diversity bars under the ridgeline — 52 thin bars.
#[must_use]
pub fn diversity_bars() -> String {
    let mut out = String::from(
        r#"<div class="bnb-skel-bars" style="--n:52;height:60px;" aria-busy="true" aria-label="Loading weekly diversity">"#,
    );
    // Pre-baked heights so it looks like a real distribution.
    let h = [
        18, 22, 24, 28, 30, 36, 44, 56, 68, 78, 82, 86, 88, 82, 70, 62, 54, 48, 42, 38, 36, 32, 30,
        28, 28, 26, 24, 26, 28, 30, 32, 36, 40, 44, 50, 58, 64, 70, 74, 76, 72, 68, 60, 54, 46, 40,
        34, 28, 24, 22, 20, 18,
    ];
    for &p in &h {
        let _ = write!(out, r#"<span style="height:{p}%"></span>"#);
    }
    out.push_str("</div>");
    out
}

/// Per-species ribbons list (dawn chorus right rail). N rows.
#[must_use]
pub fn species_ribbons(n: usize) -> String {
    let n = n.clamp(2, 12);
    let mut out = String::from(
        r#"<div class="bnb-skel-list" aria-busy="true" aria-label="Loading species ribbons">"#,
    );
    for _ in 0..n {
        out.push_str(
            r#"<div class="bnb-skel-list-row">
  <span class="bnb-skel avatar"></span>
  <div><span class="bnb-skel line" style="width:55%;"></span><span class="bnb-skel" style="height:14px;border-radius:3px;margin-top:6px;"></span></div>
  <span class="bnb-skel" style="width:42px;height:42px;border-radius:50%;"></span>
</div>"#,
        );
    }
    out.push_str("</div>");
    out
}

/// 24-bar hourly chart (species circadian, hour×day mini-grid spine).
#[must_use]
pub fn hourly_bars(n: usize) -> String {
    let n = n.clamp(8, 96);
    let mut out = format!(
        r#"<div class="bnb-skel-bars" style="--n:{n};" aria-busy="true" aria-label="Loading hourly activity">"#
    );
    // Bell-curve heights.
    for i in 0..n {
        let t = (i as f64) / (n as f64);
        let h = ((t * std::f64::consts::PI).sin() * 0.85 + 0.10) * 100.0;
        let _ = write!(out, r#"<span style="height:{:.0}%"></span>"#, h);
    }
    out.push_str("</div>");
    out
}

/// 14-day trend bars (`pages/species-daily` shape).
#[must_use]
pub fn trend_line() -> &'static str {
    r#"<div aria-busy="true" aria-label="Loading trend">
  <div class="bnb-skel-bars" style="--n:14;height:120px;">
    <span style="height:18%"></span><span style="height:32%"></span><span style="height:24%"></span><span style="height:46%"></span><span style="height:58%"></span><span style="height:72%"></span><span style="height:68%"></span><span style="height:52%"></span><span style="height:62%"></span><span style="height:78%"></span><span style="height:86%"></span><span style="height:74%"></span><span style="height:64%"></span><span style="height:56%"></span>
  </div>
</div>"#
}

/// Generic list rows (n rows of avatar + name + count).
#[must_use]
pub fn list_rows(n: usize) -> String {
    let n = n.clamp(1, 20);
    let mut out = String::from(
        r#"<div class="bnb-skel-list" aria-busy="true" aria-label="Loading list">"#,
    );
    for _ in 0..n {
        out.push_str(
            r#"<div class="bnb-skel-list-row">
  <span class="bnb-skel avatar"></span>
  <div><span class="bnb-skel line" style="width:60%;"></span><span class="bnb-skel line" style="width:40%;margin-top:6px;"></span></div>
  <span class="bnb-skel line" style="width:42px;"></span>
</div>"#,
        );
    }
    out.push_str("</div>");
    out
}

/// Inline pill row (species-status, etc.) — n pill placeholders side by side.
#[must_use]
pub fn pill_row(n: usize) -> String {
    let n = n.clamp(1, 6);
    let mut out =
        String::from(r#"<div style="display:flex;gap:8px;" aria-busy="true" aria-hidden="true">"#);
    for _ in 0..n {
        out.push_str(r#"<span class="bnb-skel pill"></span>"#);
    }
    out.push_str("</div>");
    out
}

/// Hero card placeholder — photo aspect-4/3 box + caption + scrubber bar.
#[must_use]
pub fn hero_card() -> &'static str {
    r#"<div aria-busy="true" aria-label="Loading best detection">
  <span class="bnb-skel line" style="width:38%;"></span>
  <span class="bnb-skel box" style="aspect-ratio:4/3;width:100%;margin-top:10px;"></span>
  <span class="bnb-skel line" style="width:60%;margin-top:12px;"></span>
  <span class="bnb-skel" style="height:36px;width:100%;margin-top:10px;border-radius:6px;"></span>
</div>"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_carry_aria_busy() {
        assert!(feed_rows(3).contains("aria-busy=\"true\""));
        assert!(stat_row(4).contains("aria-busy=\"true\""));
        assert!(polar_plot().contains("aria-busy=\"true\""));
        assert!(ridgeline().contains("aria-busy=\"true\""));
    }

    #[test]
    fn feed_rows_bounds() {
        assert!(feed_rows(0).contains("aria-busy"));
        assert!(feed_rows(99).matches("class=\"feed-row").count() <= 20);
    }

    #[test]
    fn hourly_bars_emits_n_bars() {
        let html = hourly_bars(24);
        assert_eq!(html.matches("<span style=\"height:").count(), 24);
    }
}
