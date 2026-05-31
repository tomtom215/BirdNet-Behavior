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
    ModelVsReviewRow, QualitySummary, ReviewVerdictDay, confidence_distribution, confidence_trend,
    detection_quality_by_hour, low_confidence_species, model_vs_review_by_species, quality_summary,
    review_verdict_trend,
};

use super::admin_shell;
use crate::routes::pages::escape_html;
use crate::routes::pages::help::{Topic, help_link};
use crate::routes::pages::skeletons;
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

    Html(admin_shell(
        "Data Quality",
        "quality",
        &render_quality_page(&data),
    ))
}

async fn quality_summary_partial(State(state): State<AppState>) -> Html<String> {
    let data = tokio::task::spawn_blocking(move || load_quality_data(&state))
        .await
        .unwrap_or_else(|_| QualityData::empty());
    Html(render_summary_cards(data.summary.as_ref()))
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
    review_trend: Vec<ReviewVerdictDay>,
    model_vs_review: Vec<ModelVsReviewRow>,
}

impl QualityData {
    const fn empty() -> Self {
        Self {
            summary: None,
            conf_buckets: [0; 6],
            trend: Vec::new(),
            by_hour: Vec::new(),
            low_conf: Vec::new(),
            review_trend: Vec::new(),
            model_vs_review: Vec::new(),
        }
    }
}

fn load_quality_data(state: &AppState) -> QualityData {
    state.with_db(|conn| QualityData {
        summary: quality_summary(conn).ok(),
        conf_buckets: confidence_distribution(conn).unwrap_or([0; 6]),
        trend: confidence_trend(conn, 30).unwrap_or_default(),
        by_hour: detection_quality_by_hour(conn).unwrap_or_default(),
        low_conf: low_confidence_species(conn, 0.60, 3).unwrap_or_default(),
        review_trend: review_verdict_trend(conn, 30).unwrap_or_default(),
        model_vs_review: model_vs_review_by_species(conn, 12).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn render_quality_page(data: &QualityData) -> String {
    let summary_html = render_summary_cards(data.summary.as_ref());
    let dist_html = render_confidence_distribution(&data.conf_buckets);
    let trend_html = render_confidence_trend(&data.trend);
    let hour_html = render_hourly_quality(&data.by_hour);
    let low_conf_html = render_low_confidence_species(&data.low_conf);
    let review_trend_html = render_review_trend(&data.review_trend);
    let model_vs_review_html = render_model_vs_review(&data.model_vs_review);
    let tuning_link = help_link(Topic::Tuning);
    // Skeletons for the partials that load via hx-trigger="load" (resolves
    // TODO(O-16-followup) for #quality-summary / #quality-trend).
    let summary_skeleton = skeletons::stat_row(6);
    let trend_skeleton = skeletons::hourly_bars(30);

    format!(
        r#"<header class="page-head q-head" data-screen-label="Data quality" data-om-validate>
  <div>
    <div class="bnb-eyebrow">Admin · data quality</div>
    <h1 class="display q-h1">
      Data quality
    </h1>
    <p class="bnb-meta q-lede">
      Detection database health — confidence distributions, trend analysis,
      and potential false positives.
    </p>
  </div>
</header>

<section class="bnb-card pad">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Summary</div>
      <h3>At-a-glance statistics</h3>
    </div>
  </div>
  <div id="quality-summary"
       hx-get="/admin/quality/summary"
       hx-trigger="load"
       hx-swap="innerHTML">
    {summary_skeleton}
    <div hidden>{summary_html}</div>
  </div>
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Confidence distribution</div>
      <h3>Six bucket histogram</h3>
    </div>
  </div>
  <p class="bnb-meta q-meta-mb">
    A healthy dataset skews toward higher buckets (≥70%).
  </p>
  {dist_html}
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">30-day trend</div>
      <h3>Daily average confidence</h3>
    </div>
  </div>
  <p class="bnb-meta q-meta-mb">
    Sudden drops may indicate equipment issues or adverse acoustic conditions.
  </p>
  <div id="quality-trend"
       hx-get="/admin/quality/trend"
       hx-trigger="load"
       hx-swap="innerHTML">
    {trend_skeleton}
    <div hidden>{trend_html}</div>
  </div>
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Hourly profile</div>
      <h3>When does activity peak?</h3>
    </div>
  </div>
  <p class="bnb-meta q-meta-mb">
    Detection counts (bars) and average confidence (colour intensity) by hour.
    Dawn (04–08) and dusk (18–22) typically run the busiest.
  </p>
  {hour_html}
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Review verdict trend</div>
      <h3>Human disagreement over 30 days</h3>
    </div>
  </div>
  <p class="bnb-meta q-meta-mb">
    Every reviewer verdict on a detection rolls up here. A rising red band
    means the model is firing detections you keep rejecting.
  </p>
  {review_trend_html}
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Model vs. human</div>
      <h3>Where do they disagree most?</h3>
    </div>
  </div>
  <p class="bnb-meta q-meta-mb">
    Per-species comparison of the classifier's mean confidence (top bar) vs.
    the share of reviewed calls that humans confirmed (bottom bar). Species
    sorted by the gap, biggest first — these are the most likely
    overconfident false positives.
  </p>
  {model_vs_review_html}
</section>

<section class="bnb-card pad q-section">
  <div class="section-header">
    <div>
      <div class="bnb-eyebrow">Low-confidence species</div>
      <h3>Consistently uncertain calls</h3>
    </div>
    {tuning_link}
  </div>
  <p class="bnb-meta q-meta-mb">
    Average confidence below 60% with at least 3 detections. Consider raising
    the per-species threshold in
    <a href="/admin/species" class="q-link">Species settings</a>.
  </p>
  {low_conf_html}
</section>"#
    )
}

fn render_summary_cards(summary: Option<&QualitySummary>) -> String {
    let Some(s) = summary else {
        return r#"<p class="q-empty">No detections in database.</p>"#.to_string();
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
    <div class="stat-value q-stat-value moss-ink">{total}</div>
    <div class="stat-label">Total Detections</div>
  </div>
  <div class="stat-card">
    <div class="stat-value q-stat-value moss">{species}</div>
    <div class="stat-label">Species</div>
  </div>
  <div class="stat-card">
    <div class="stat-value q-stat-value moss-ink">{avg:.1}%</div>
    <div class="stat-label">Avg Confidence</div>
  </div>
  <div class="stat-card">
    <div class="stat-value q-stat-value dawn">{min:.0}%–{max:.0}%</div>
    <div class="stat-label">Conf Range</div>
  </div>
  <div class="stat-card">
    <div class="stat-value q-stat-value sm">{badge}</div>
    <div class="stat-label">Quality Flag</div>
  </div>
  <div class="stat-card">
    <div class="stat-value q-stat-value sm muted">{earliest}</div>
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
        "var(--rare)",
        "var(--dawn)",
        "var(--dawn)",
        "var(--moss)",
        "var(--moss-ink)",
        "var(--moss-ink)",
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
        return r#"<p class="q-empty">No data for the last 30 days.</p>"#.to_string();
    }

    let max_conf = trend
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0_f64, f64::max)
        .max(0.01);

    let mut html =
        String::from(r#"<div class="trend-bars" title="Daily average confidence (last 30 days)">"#);
    for (date, conf) in trend {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let height_pct = (conf / max_conf * 100.0).clamp(0.0, 100.0) as u32;
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
            r#"<div class="q-trend-legend">
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
        return r#"<p class="q-empty">No data yet.</p>"#.to_string();
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
        let (count, avg_conf, color) = maybe.map_or((0, 0.0, "var(--surface)"), |(c, a)| {
            (c, a, conf_to_color(a))
        });
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
    html.push_str(r#"<div class="q-hour-axis">"#);
    for h in 0u8..24 {
        if h % 6 == 0 {
            write!(html, "<span>{h:02}h</span>").unwrap_or_default();
        }
    }
    html.push_str("</div>");
    html
}

fn render_low_confidence_species(low: &[(String, String, i64, f64)]) -> String {
    if low.is_empty() {
        return r#"<p class="q-empty ok">
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
    <th class="q-num">Detections</th>
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
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bar_pct = pct.clamp(0.0, 100.0) as u32;
        write!(
            html,
            r#"<tr>
  <td><strong>{com}</strong></td>
  <td class="q-sci">{sci}</td>
  <td class="q-num">{count}</td>
  <td>
    <div class="conf-meter">
      <div class="conf-fill" style="width:{bar_pct}%;background:{color};"></div>
    </div>
    <span class="q-conf-pct">{pct:.1}%</span>
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

// ---------------------------------------------------------------------------
// O-22 model-trust panels — review verdict trend + model-vs-human gap
// ---------------------------------------------------------------------------

fn render_review_trend(days: &[ReviewVerdictDay]) -> String {
    if days.is_empty() {
        return r#"<p class="bnb-meta">No detections in the last 30 days.</p>"#.to_string();
    }
    let max_total = days.iter().map(|d| d.total).max().unwrap_or(1).max(1);
    let mut html = String::from(r#"<div class="bnb-verdict-trend">"#);
    for d in days {
        let label = format!(
            "{} · {} total · {} confirmed · {} rejected · {} unreviewed",
            d.day, d.total, d.confirmed, d.rejected, d.unreviewed
        );
        #[allow(clippy::cast_precision_loss)]
        let scale = |n: i64| -> u32 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                ((n as f64) / (max_total as f64) * 100.0).clamp(0.0, 100.0) as u32
            }
        };
        let _ = write!(
            html,
            r#"<div class="col" title="{label}">
  <span class="unreviewed" style="height:{u}%"></span>
  <span class="confirmed"  style="height:{c}%" class-suffix="approved"></span>
  <span class="approved"   style="height:{c}%"></span>
  <span class="rejected"   style="height:{r}%"></span>
</div>"#,
            label = escape_html(&label),
            u = scale(d.unreviewed),
            c = scale(d.confirmed),
            r = scale(d.rejected),
        );
    }
    html.push_str("</div>");
    html.push_str(
        r#"<div class="bnb-verdict-legend">
  <span><i class="approved"></i> Confirmed</span>
  <span><i class="rejected"></i> Rejected</span>
  <span><i class="unreviewed"></i> Unreviewed</span>
</div>"#,
    );
    html
}

fn render_model_vs_review(rows: &[ModelVsReviewRow]) -> String {
    if rows.is_empty() {
        return r#"<p class="bnb-meta">Not enough reviewed detections yet. The panel needs at least 5 detections per species and 3 reviewer verdicts to compare model and human confidence.</p>"#.to_string();
    }
    let mut html = String::from(r#"<ul class="bnb-mvr q-mvr-list" role="list">"#);
    for r in rows {
        let model_pct = (r.model_avg * 100.0).clamp(0.0, 100.0);
        let human_pct = (r.human_avg * 100.0).clamp(0.0, 100.0);
        let gap = r.model_avg - r.human_avg;
        let gap_class = if gap.abs() >= 0.15 { "" } else { " small" };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let model_w = model_pct as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let human_w = human_pct as u32;
        let species_link = format!(
            "/quarantine?species={}",
            crate::routes::pages::simple_url_encode(&r.com_name)
        );
        let _ = write!(
            html,
            r#"<li class="bnb-mvr-row">
  <div class="bnb-mvr-name">
    <a href="{href}" class="q-mvr-link">{com}</a>
    <em>{sci} · {total} detections</em>
  </div>
  <div class="bnb-mvr-bars">
    <div class="bnb-mvr-bar model"><span class="fill" style="width:{model_w}%"></span><span class="label">model {model_pct:.0}%</span></div>
    <div class="bnb-mvr-bar human"><span class="fill" style="width:{human_w}%"></span><span class="label">human {human_pct:.0}%</span></div>
  </div>
  <div class="bnb-mvr-gap{gap_class}">Δ {gap_sign}{gap_abs:.0}%</div>
</li>"#,
            href = escape_html(&species_link),
            com = escape_html(&r.com_name),
            sci = escape_html(&r.sci_name),
            total = r.total,
            model_w = model_w,
            human_w = human_w,
            model_pct = model_pct,
            human_pct = human_pct,
            gap_class = gap_class,
            gap_sign = if gap >= 0.0 { "+" } else { "−" },
            gap_abs = (gap.abs() * 100.0),
        );
    }
    html.push_str("</ul>");
    html
}

/// Map a confidence value (0.0–1.0) to a design-token colour string.
fn conf_to_color(conf: f64) -> &'static str {
    if conf >= 0.85 {
        "var(--moss-ink)"
    } else if conf >= 0.70 {
        "var(--moss)"
    } else if conf >= 0.55 {
        "var(--dawn)"
    } else {
        "var(--rare)"
    }
}
