//! Data quality metrics dashboard.
//!
//! Provides a read-only admin page summarising the health of the detection
//! database with the following panels:
//!
//! - **Summary statistics** — total detections, species count, confidence
//!   mean/min/max, date range.
//! - **Confidence distribution** — visual bar chart across six buckets.
//! - **Daily confidence trend** — 30-day moving average bar chart.
//! - **Hourly quality profile** — detection count and mean confidence by
//!   hour-of-day (identifies noisy recording windows).
//! - **Low-confidence species** — ranked list of species whose average
//!   confidence falls below the configurable threshold (false-positive
//!   candidates).
//!
//! | Path | Method | Purpose |
//! |------|--------|---------|
//! | `/admin/quality` | GET | Full quality metrics page |
//! | `/admin/quality/summary` | GET | HTMX partial — summary stats |
//! | `/admin/quality/trend` | GET | HTMX partial — confidence trend |

use std::fmt::Write as _;

use axum::extract::State;
use axum::response::Html;
use axum::{Router, routing::get};

use birdnet_db::sqlite::{
    QualitySummary, confidence_distribution, confidence_trend, detection_quality_by_hour,
    low_confidence_species, quality_summary,
};

use crate::routes::pages::escape_html;
use crate::state::AppState;

/// Mount data quality routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/admin/quality", get(quality_page))
        .route("/admin/quality/summary", get(quality_summary_partial))
        .route("/admin/quality/trend", get(quality_trend_partial))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn quality_page(State(state): State<AppState>) -> Html<String> {
    let data = tokio::task::spawn_blocking(move || load_quality_data(&state))
        .await
        .unwrap_or_else(|_| QualityData::empty());

    Html(render_quality_page(&data))
}

async fn quality_summary_partial(State(state): State<AppState>) -> Html<String> {
    let data = tokio::task::spawn_blocking(move || load_quality_data(&state))
        .await
        .unwrap_or_else(|_| QualityData::empty());
    Html(render_summary_cards(&data.summary))
}

async fn quality_trend_partial(State(state): State<AppState>) -> Html<String> {
    let data = tokio::task::spawn_blocking(move || load_quality_data(&state))
        .await
        .unwrap_or_else(|_| QualityData::empty());
    Html(render_confidence_trend(&data.trend))
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

struct QualityData {
    summary: Option<QualitySummary>,
    conf_buckets: [i64; 6],
    trend: Vec<(String, f64)>,
    by_hour: Vec<(u8, i64, f64)>,
    low_conf: Vec<(String, String, i64, f64)>,
}

impl QualityData {
    fn empty() -> Self {
        Self {
            summary: None,
            conf_buckets: [0; 6],
            trend: Vec::new(),
            by_hour: Vec::new(),
            low_conf: Vec::new(),
        }
    }
}

fn load_quality_data(state: &AppState) -> QualityData {
    state.with_db(|conn| QualityData {
        summary: quality_summary(conn).ok(),
        conf_buckets: confidence_distribution(conn).unwrap_or([0; 6]),
        trend: confidence_trend(conn, 30).unwrap_or_default(),
        by_hour: detection_quality_by_hour(conn).unwrap_or_default(),
        // Species with avg confidence < 60%, seen ≥ 3 times
        low_conf: low_confidence_species(conn, 0.60, 3).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_quality_page(data: &QualityData) -> String {
    let summary_html = render_summary_cards(&data.summary);
    let dist_html = render_confidence_distribution(&data.conf_buckets);
    let trend_html = render_confidence_trend(&data.trend);
    let hour_html = render_hourly_quality(&data.by_hour);
    let low_conf_html = render_low_confidence_species(&data.low_conf);

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1.0">
  <title>Data Quality — BirdNet-Behavior Admin</title>
  <script src="/static/htmx.min.js"></script>
  <style>
    body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; margin:0; }}
    .container {{ max-width:960px; margin:0 auto; padding:2rem 1rem; }}
    nav {{ margin-bottom:2rem; }}
    nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; font-size:.9rem; }}
    nav a:hover {{ color:#38bdf8; }}
    h1 {{ font-size:1.5rem; font-weight:700; color:#f8fafc; margin-bottom:.25rem; }}
    .subtitle {{ color:#64748b; font-size:.875rem; margin-bottom:2rem; }}
    .card {{ background:#1e293b; border:1px solid #334155; border-radius:.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
    .card h2 {{ font-size:1.05rem; color:#38bdf8; margin:0 0 1rem; font-weight:600; }}
    .stat-grid {{ display:grid; grid-template-columns:repeat(auto-fit,minmax(140px,1fr)); gap:1rem; }}
    .stat-card {{ background:#0f172a; border:1px solid #1e293b; border-radius:.5rem; padding:1rem; text-align:center; }}
    .stat-value {{ font-size:1.5rem; font-weight:700; margin-bottom:.25rem; }}
    .stat-label {{ font-size:.75rem; color:#64748b; text-transform:uppercase; letter-spacing:.05em; }}
    .bar-chart {{ display:flex; align-items:flex-end; gap:.375rem; height:120px; padding:.5rem 0; }}
    .bar-wrap {{ display:flex; flex-direction:column; align-items:center; flex:1; gap:.25rem; }}
    .bar {{ border-radius:.25rem .25rem 0 0; min-height:2px; width:100%; transition:height .3s; }}
    .bar-label {{ font-size:.7rem; color:#64748b; white-space:nowrap; }}
    .bar-val {{ font-size:.75rem; color:#94a3b8; font-weight:600; }}
    .trend-bars {{ display:flex; align-items:flex-end; gap:2px; height:80px; overflow-x:auto; padding:.25rem 0; }}
    .trend-bar {{ min-width:8px; border-radius:.125rem .125rem 0 0; flex-shrink:0; }}
    table {{ width:100%; border-collapse:collapse; font-size:.875rem; }}
    th {{ text-align:left; color:#64748b; font-size:.75rem; text-transform:uppercase; font-weight:600;
           padding:.5rem .75rem; border-bottom:1px solid #334155; }}
    td {{ padding:.6rem .75rem; border-bottom:1px solid #1e293b; }}
    tr:last-child td {{ border-bottom:none; }}
    .hour-bars {{ display:grid; grid-template-columns:repeat(24, 1fr); gap:2px; align-items:end; height:80px; }}
    .hour-bar {{ border-radius:.125rem .125rem 0 0; }}
    .conf-meter {{ height:6px; border-radius:3px; background:#334155; overflow:hidden; }}
    .conf-fill {{ height:100%; border-radius:3px; }}
    .badge {{ display:inline-block; padding:.15rem .5rem; border-radius:.25rem; font-size:.75rem; font-weight:600; }}
    .badge-warn {{ background:#422006; color:#fbbf24; }}
    .badge-ok   {{ background:#14532d; color:#4ade80; }}
  </style>
</head>
<body>
<div class="container">
  <nav>
    <a href="/admin/overview">Overview</a>
    <a href="/admin/settings">Settings</a>
    <a href="/admin/rules">Rules</a>
    <a href="/admin/quality" style="color:#38bdf8;">Quality</a>
    <a href="/admin/notifications">Notifications</a>
    <a href="/admin/system">System</a>
  </nav>

  <h1>Data Quality</h1>
  <p class="subtitle">
    Detection database health metrics — confidence distributions, trend analysis,
    and potential false-positive species identification.
  </p>

  <div class="card">
    <h2>Summary Statistics</h2>
    <div id="quality-summary"
         hx-get="/admin/quality/summary"
         hx-trigger="load"
         hx-swap="innerHTML">
      {summary_html}
    </div>
  </div>

  <div class="card">
    <h2>Confidence Distribution</h2>
    <p style="font-size:.8rem;color:#64748b;margin-bottom:.75rem;">
      Detection counts across six confidence buckets. A healthy dataset should
      skew toward higher buckets (≥70%).
    </p>
    {dist_html}
  </div>

  <div class="card">
    <h2>30-Day Confidence Trend</h2>
    <p style="font-size:.8rem;color:#64748b;margin-bottom:.75rem;">
      Daily average confidence. Sudden drops may indicate equipment issues or
      adverse acoustic conditions.
    </p>
    <div id="quality-trend"
         hx-get="/admin/quality/trend"
         hx-trigger="load"
         hx-swap="innerHTML">
      {trend_html}
    </div>
  </div>

  <div class="card">
    <h2>Hourly Quality Profile</h2>
    <p style="font-size:.8rem;color:#64748b;margin-bottom:.75rem;">
      Detection counts (bars) and average confidence (colour intensity) by
      hour of day. Dawn (04–08) and dusk (18–22) windows typically have
      the most activity.
    </p>
    {hour_html}
  </div>

  <div class="card">
    <h2>Low-Confidence Species (&lt;60% avg, ≥3 detections)</h2>
    <p style="font-size:.8rem;color:#64748b;margin-bottom:.75rem;">
      Species with consistently low confidence scores are prime false-positive
      candidates. Consider raising their per-species threshold in
      <a href="/admin/species" style="color:#38bdf8;">Species settings</a>.
    </p>
    {low_conf_html}
  </div>
</div>
</body>
</html>"##,
        summary_html = summary_html,
        dist_html = dist_html,
        trend_html = trend_html,
        hour_html = hour_html,
        low_conf_html = low_conf_html,
    )
}

fn render_summary_cards(summary: &Option<QualitySummary>) -> String {
    let Some(s) = summary else {
        return r#"<p style="color:#64748b;">No detections in database.</p>"#.to_string();
    };

    let low_pct = if s.total_detections > 0 {
        (s.low_confidence_count * 100) / s.total_detections
    } else {
        0
    };

    let low_badge = if low_pct > 10 {
        format!(r#"<span class="badge badge-warn">{low_pct}% low-conf</span>"#)
    } else {
        format!(r#"<span class="badge badge-ok">{low_pct}% low-conf</span>"#)
    };

    format!(
        r#"<div class="stat-grid">
  <div class="stat-card">
    <div class="stat-value" style="color:#38bdf8;">{total}</div>
    <div class="stat-label">Total Detections</div>
  </div>
  <div class="stat-card">
    <div class="stat-value" style="color:#34d399;">{species}</div>
    <div class="stat-label">Species</div>
  </div>
  <div class="stat-card">
    <div class="stat-value" style="color:#a78bfa;">{avg:.1}%</div>
    <div class="stat-label">Avg Confidence</div>
  </div>
  <div class="stat-card">
    <div class="stat-value" style="color:#fb923c;">{min:.0}%–{max:.0}%</div>
    <div class="stat-label">Conf Range</div>
  </div>
  <div class="stat-card">
    <div class="stat-value" style="font-size:1rem;">{badge}</div>
    <div class="stat-label">Quality Flag</div>
  </div>
  <div class="stat-card">
    <div class="stat-value" style="font-size:1rem;color:#64748b;">{earliest}</div>
    <div class="stat-label">Earliest Detection</div>
  </div>
</div>"#,
        total = s.total_detections,
        species = s.distinct_species,
        avg = s.avg_confidence * 100.0,
        min = s.min_confidence * 100.0,
        max = s.max_confidence * 100.0,
        badge = low_badge,
        earliest = escape_html(&s.earliest_date),
    )
}

fn render_confidence_distribution(buckets: &[i64; 6]) -> String {
    let labels = ["<50%", "50–60%", "60–70%", "70–80%", "80–90%", "≥90%"];
    let colors = [
        "#ef4444", "#f97316", "#eab308", "#22c55e", "#3b82f6", "#8b5cf6",
    ];
    let max = *buckets.iter().max().unwrap_or(&1).max(&1);

    let mut html = String::from(r#"<div class="bar-chart">"#);
    for (i, (&count, (&label, &color))) in buckets
        .iter()
        .zip(labels.iter().zip(colors.iter()))
        .enumerate()
    {
        let _ = i;
        let height_pct = if max > 0 { (count * 100) / max } else { 0 };
        write!(
            html,
            r#"<div class="bar-wrap">
  <div class="bar-val">{count}</div>
  <div class="bar" style="height:{height_pct}%;background:{color};"></div>
  <div class="bar-label">{label}</div>
</div>"#
        )
        .unwrap_or_default();
    }
    html.push_str("</div>");
    html
}

fn render_confidence_trend(trend: &[(String, f64)]) -> String {
    if trend.is_empty() {
        return r#"<p style="color:#64748b;">No data for the last 30 days.</p>"#.to_string();
    }

    let max_conf = trend
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0_f64, f64::max)
        .max(0.01);

    let mut html =
        String::from(r#"<div class="trend-bars" title="Daily average confidence (last 30 days)">"#);
    for (date, conf) in trend {
        let height_pct = (conf / max_conf * 100.0) as u32;
        let color = conf_to_color(*conf);
        write!(
            html,
            r#"<div class="trend-bar" style="height:{height_pct}%;background:{color};"
                 title="{date}: {conf:.1}%"></div>"#,
            date = escape_html(date),
            conf = conf * 100.0,
            height_pct = height_pct,
            color = color,
        )
        .unwrap_or_default();
    }
    html.push_str("</div>");

    // Add a simple date range legend
    if let (Some((first, _)), Some((last, _))) = (trend.first(), trend.last()) {
        write!(
            html,
            r#"<div style="display:flex;justify-content:space-between;font-size:.75rem;color:#64748b;margin-top:.25rem;">
  <span>{}</span><span>{}</span>
</div>"#,
            escape_html(first),
            escape_html(last),
        )
        .unwrap_or_default();
    }
    html
}

fn render_hourly_quality(by_hour: &[(u8, i64, f64)]) -> String {
    if by_hour.is_empty() {
        return r#"<p style="color:#64748b;">No data yet.</p>"#.to_string();
    }

    let max_count = by_hour.iter().map(|(_, c, _)| *c).max().unwrap_or(1).max(1);

    // Build a 24-element lookup (hour → Option<(count, avg_conf)>)
    let mut hours_map = vec![None::<(i64, f64)>; 24];
    for &(h, cnt, conf) in by_hour {
        if (h as usize) < 24 {
            hours_map[h as usize] = Some((cnt, conf));
        }
    }

    let mut html = String::from(r#"<div class="hour-bars">"#);
    for (hour, maybe) in hours_map.iter().enumerate() {
        let (count, avg_conf, color) = maybe
            .map(|(c, a)| (c, a, conf_to_color(a)))
            .unwrap_or((0, 0.0, "#1e293b"));
        let height_pct = (count * 100) / max_count;
        write!(
            html,
            r#"<div class="hour-bar" style="height:{height_pct}%;background:{color};"
                 title="{hour:02}:00 — {count} detections, avg {conf:.0}%"></div>"#,
            height_pct = height_pct,
            color = color,
            hour = hour,
            count = count,
            conf = avg_conf * 100.0,
        )
        .unwrap_or_default();
    }
    html.push_str("</div>");

    // Hour axis labels
    html.push_str(
        r#"<div style="display:grid;grid-template-columns:repeat(24,1fr);gap:2px;font-size:.65rem;color:#64748b;margin-top:.25rem;">"#,
    );
    for h in 0u8..24 {
        if h % 6 == 0 {
            write!(
                html,
                r#"<span style="grid-column:span 6;text-align:left;">{h:02}h</span>"#
            )
            .unwrap_or_default();
        }
    }
    html.push_str("</div>");
    html
}

fn render_low_confidence_species(low: &[(String, String, i64, f64)]) -> String {
    if low.is_empty() {
        return r#"<p style="color:#4ade80;">
            No species with avg confidence &lt;60% (≥3 detections). Database quality looks good!
           </p>"#
            .to_string();
    }

    let mut html = String::from(
        r#"<table>
<thead>
  <tr>
    <th>Common Name</th>
    <th>Scientific Name</th>
    <th style="text-align:right">Detections</th>
    <th>Avg Confidence</th>
    <th>Recommendation</th>
  </tr>
</thead>
<tbody>"#,
    );

    for (com, sci, count, avg_conf) in low {
        let pct = avg_conf * 100.0;
        let rec = if pct < 40.0 {
            r#"<span class="badge badge-warn">Consider exclusion</span>"#
        } else {
            r#"<span class="badge badge-warn">Raise threshold</span>"#
        };
        let bar_pct = pct as u32;
        write!(
            html,
            r#"<tr>
  <td><strong>{com}</strong></td>
  <td style="color:#94a3b8;font-style:italic">{sci}</td>
  <td style="text-align:right">{count}</td>
  <td>
    <div class="conf-meter">
      <div class="conf-fill" style="width:{bar_pct}%;background:{color};"></div>
    </div>
    <span style="font-size:.8rem;color:#94a3b8;">{pct:.1}%</span>
  </td>
  <td>{rec}</td>
</tr>"#,
            com = escape_html(com),
            sci = escape_html(sci),
            count = count,
            bar_pct = bar_pct,
            color = conf_to_color(*avg_conf),
            pct = pct,
            rec = rec,
        )
        .unwrap_or_default();
    }

    html.push_str("</tbody></table>");
    html
}

/// Map a confidence value (0.0–1.0) to a CSS colour string.
fn conf_to_color(conf: f64) -> &'static str {
    if conf >= 0.90 {
        "#8b5cf6"
    } else if conf >= 0.80 {
        "#3b82f6"
    } else if conf >= 0.70 {
        "#22c55e"
    } else if conf >= 0.60 {
        "#eab308"
    } else if conf >= 0.50 {
        "#f97316"
    } else {
        "#ef4444"
    }
}
