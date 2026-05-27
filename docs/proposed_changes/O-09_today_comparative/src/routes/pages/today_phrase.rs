//! Today's "comparative phrase" partial — answers *how* today compares to baseline.
//!
//! Drop this into `crates/birdnet-web/src/routes/pages/today.rs` and add the
//! route in that file's `router()`:
//!
//! ```rust,ignore
//! .route("/pages/today-phrase", get(today_phrase_partial))
//! ```
//!
//! Pure read; no schema changes; uses only `detections` table.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

use crate::routes::pages::{escape_html, today_date_string};
use crate::state::AppState;

/// Tier breakpoints — percentiles of the rolling 30-day count distribution.
///
/// Tuned for "feels right" rather than strict statistics. Adjust to taste.
const TIERS: &[(f64, &str, &str)] = &[
    (0.10, "quiet",     "fg-3"),       // bottom 10%
    (0.35, "calm",      "fg-2"),
    (0.65, "steady",    "fg"),         // middle band — no accent
    (0.85, "busy",      "moss-ink"),
    (0.97, "loud",      "moss-ink"),
    (1.01, "record",    "rare"),       // top 3%  — uses rare hue
];

pub async fn today_phrase_partial(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let today = today_date_string();
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM detections WHERE Date = ?1",
                [&today],
                |r| r.get(0),
            )?;
            let today_species: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT Com_Name) FROM detections WHERE Date = ?1",
                [&today],
                |r| r.get(0),
            )?;
            // 30-day baseline (excluding today).
            let baseline: Vec<i64> = {
                let mut stmt = conn.prepare(
                    "SELECT COUNT(*) FROM detections \
                     WHERE Date < ?1 AND Date >= date(?1, '-30 days') \
                     GROUP BY Date ORDER BY Date",
                )?;
                let rows = stmt.query_map([&today], |r| r.get::<_, i64>(0))?;
                rows.filter_map(Result::ok).collect()
            };
            Ok::<_, rusqlite::Error>((today_count, today_species, baseline))
        })
    })
    .await;

    let (count, species, baseline) = match result {
        Ok(Ok(v)) => v,
        _ => {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                static_fallback().to_string(),
            );
        }
    };

    // Compute percentile of today's count within the baseline.
    let pct = percentile(&baseline, count);
    let (verb, color) = tier_for(pct);
    let time_phrase = morning_or_day();

    let html = format!(
        r#"<h1 class="display" style="font-size:48px;line-height:1.05;letter-spacing:-0.02em;">
A <em style="color:var(--{color})">{verb}</em> {time_phrase}.
</h1>
<p class="bnb-meta" style="margin-top:6px;">
  <span class="mono tabular">{count}</span> detections ·
  <span class="mono tabular">{species}</span> species ·
  {pct_str} vs your last 30 days.
</p>"#,
        verb = escape_html(verb),
        pct_str = percentile_phrase(pct),
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        html,
    )
}

fn static_fallback() -> &'static str {
    r#"<h1 class="display" style="font-size:48px;line-height:1.05;letter-spacing:-0.02em;">
You're listening.
</h1>
<p class="bnb-meta" style="margin-top:6px;">Detections roll in below.</p>"#
}

/// 0..1 percentile of `value` within `samples`. Empty samples → 0.5 (middle).
fn percentile(samples: &[i64], value: i64) -> f64 {
    if samples.is_empty() {
        return 0.5;
    }
    let below = samples.iter().filter(|&&v| v < value).count() as f64;
    below / samples.len() as f64
}

fn tier_for(pct: f64) -> (&'static str, &'static str) {
    for (bound, verb, color) in TIERS {
        if pct <= *bound {
            return (verb, color);
        }
    }
    ("record", "rare")
}

fn percentile_phrase(pct: f64) -> String {
    let n = (pct * 100.0).round() as i32;
    match n {
        0..=5    => "well below typical".into(),
        6..=25   => format!("{n}th percentile"),
        26..=74  => "right around typical".into(),
        75..=89  => format!("{n}th percentile"),
        90..=98  => format!("{n}th percentile — well above typical"),
        _        => "your busiest day yet".into(),
    }
}

fn morning_or_day() -> &'static str {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hour = ((secs / 3600) % 24) as u32;
    match hour {
        4..=10 => "morning",
        11..=15 => "midday",
        16..=20 => "evening",
        _ => "night",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentile_basic() {
        let s = vec![10, 20, 30, 40, 50];
        assert!((percentile(&s, 35) - 0.6).abs() < 0.01);
        assert_eq!(percentile(&s, 5), 0.0);
        assert_eq!(percentile(&s, 100), 1.0);
    }
    #[test]
    fn tier_boundaries() {
        assert_eq!(tier_for(0.05).0, "quiet");
        assert_eq!(tier_for(0.50).0, "steady");
        assert_eq!(tier_for(0.99).0, "record");
    }
}
