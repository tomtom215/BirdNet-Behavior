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
        .route("/admin/quality/phantoms", get(phantoms_partial))
        .route(
            "/admin/quality/phantoms/exclude",
            axum::routing::post(exclude_phantom),
        )
}

// ---------------------------------------------------------------------------
// Suspect species
// ---------------------------------------------------------------------------

/// The HTMX partial listing species whose detections look like artefacts.
async fn phantoms_partial(State(state): State<AppState>) -> Html<String> {
    // An in-memory read, taken before `state` is moved into the closure.
    let nearby = state.nearby();
    let loaded = tokio::task::spawn_blocking(move || load_phantoms(&state))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    Html(render_phantoms_or_error(loaded, nearby.as_deref()))
}

/// Read the per-species shapes and score them.
///
/// `Err` when the detections could not be read. It used to default to no
/// shapes, and an empty report renders "Nothing looks suspicious" — an
/// all-clear about the station's records given on a read that failed.
pub(crate) fn load_phantoms(
    state: &AppState,
) -> Result<Vec<birdnet_db::phantoms::PhantomReport>, String> {
    let shapes = state
        .with_read_db(birdnet_db::sqlite::species_shapes)
        .map_err(|e| e.to_string())?;
    let mut reports = birdnet_db::phantoms::report(&shapes);
    // The report is read by the operator and acted on by scientific name, so the
    // common name is the one field to put in the station's language.
    for r in &mut reports {
        r.shape.com_name = state
            .names()
            .name(&r.shape.sci_name, &r.shape.com_name)
            .to_owned();
    }
    Ok(reports)
}

/// The report, or what to say when it could not be read.
pub(crate) fn render_phantoms_or_error(
    loaded: Result<Vec<birdnet_db::phantoms::PhantomReport>, String>,
    nearby: Option<&birdnet_integrations::ebird::Snapshot>,
) -> String {
    match loaded {
        Ok(reports) => render_phantoms(&reports, nearby),
        Err(e) => {
            tracing::warn!(error = %e, "suspect-species report could not be read");
            r#"<div class="section-title">Species that may not be birds</div>
  <p class="hint">The detections could not be read, so this report could not be checked. The log has the details.</p>"#
                .to_owned()
        }
    }
}

/// Form carrying the species the operator chose to exclude.
#[derive(Debug, serde::Deserialize)]
struct ExcludeForm {
    /// Scientific name to add to the exclusion list.
    sci_name: String,
}

/// `POST /admin/quality/phantoms/exclude` — add one species to the exclusion
/// list and re-render the report.
///
/// Adds to the list the operator already has rather than inventing a second
/// mechanism: excluding a species is a thing this station already knows how to
/// do, and a phantom-specific filter would be a second place to look when a
/// bird stops appearing.
async fn exclude_phantom(
    State(state): State<AppState>,
    request_user: crate::auth_middleware::RequestUser,
    axum::Form(form): axum::Form<ExcludeForm>,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;

    let sci_name = form.sci_name.trim().to_owned();
    if !sci_name.is_empty() {
        let name = sci_name.clone();
        let result = state
            .run_blocking(move |s| {
                let result = crate::routes::admin::species::handler::add_to_exclude_list(s, &name);
                if result.is_ok() {
                    crate::audit::audit(
                        s,
                        Some(&request_user),
                        "species.exclude.add",
                        Some(&name),
                        Some("via=suspect-species"),
                    );
                }
                result
            })
            .await;
        // The report is not re-rendered over a list that was not changed: the
        // operator would read the unchanged row as an exclusion that failed
        // silently, which is the one thing worse than being told.
        if let Err(e) = result {
            tracing::warn!(error = %e, species = %sci_name, "species not excluded");
            return crate::routes::pages::toast::not_applied(
                &crate::routes::pages::toast::Toast::error(
                    e.toast(&sci_name, "added to the exclusion list"),
                ),
            );
        }
        tracing::info!(species = %sci_name, "species excluded from the suspect-species report");
    }
    // An in-memory read, taken before `state` is moved into the closure.
    let nearby = state.nearby();
    let loaded = tokio::task::spawn_blocking(move || load_phantoms(&state))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    Html(render_phantoms_or_error(loaded, nearby.as_deref())).into_response()
}

/// Render the suspect-species report.
pub(crate) fn render_phantoms(
    reports: &[birdnet_db::phantoms::PhantomReport],
    nearby: Option<&birdnet_integrations::ebird::Snapshot>,
) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(
        r#"<div class="section-title">Species that may not be birds</div>
  <p class="hint">Flagged by the <em>shape</em> of their detections, not by name: a repeated non-bird sound — a barking dog, a creaking gate — produces the same wrong species at the same confidence, on the same few days, and never survives review. This is a heuristic and it will sometimes be wrong about a genuinely scarce visitor, so nothing is filtered until you say so. Excluding a species adds it to the list under <a href="/admin/species">Species</a>, where you can undo it.</p>"#,
    );
    if let Some(snapshot) = nearby {
        let _ = write!(
            out,
            r#"<p class="hint">Where somebody else has reported a species {scope} in the last {back} days, eBird says so below — that is independent evidence the bird is real, and a strong reason not to exclude it. eBird saying nothing is <em>not</em> evidence against a species: it only means nobody nearby submitted a checklist with it, which is common wherever there are few birders.</p>"#,
            scope = escape_html(&snapshot.scope_label),
            back = snapshot.back_days,
        );
    }

    if reports.is_empty() {
        out.push_str(
            r#"<p class="empty-note mb">Nothing looks suspicious. Species are only considered once they have at least 10 detections, and are only listed when two independent signals agree.</p>"#,
        );
        return out;
    }

    out.push_str(
        r#"<table class="thr-table"><thead><tr><th class="cell-left">Species</th><th>Detections</th><th>Days</th><th>Confidence</th><th class="cell-left">Why</th><th></th></tr></thead><tbody>"#,
    );
    for r in reports {
        let sci = escape_html(&r.shape.sci_name);
        let com = escape_html(&r.shape.com_name);
        let total = r.shape.total;
        let days = r.shape.distinct_days;
        let lo = r.shape.min_confidence * 100.0;
        let hi = r.shape.max_confidence * 100.0;
        let why = r
            .signals
            .iter()
            .map(|s| escape_html(s.label()))
            .collect::<Vec<_>>()
            .join("; ");
        // Corroboration only ever argues *against* excluding. A species eBird
        // has nothing on renders exactly as it did before this existed.
        let seen = nearby.and_then(|s| s.reported(&r.shape.sci_name));
        let nearby_pill = seen.map_or_else(String::new, |s| {
            format!(
                r#"<br><span class="bnb-pill moss">eBird: reported nearby {date}</span>"#,
                date = escape_html(&s.last_seen)
            )
        });
        let confirm_body = seen.map_or_else(
            || format!("Stop recording {com}? Existing detections are kept; you can undo this under Species."),
            |s| format!(
                "Somebody reported {com} nearby on {date}, so this is probably a real bird. Stop recording it anyway? Existing detections are kept; you can undo this under Species.",
                date = escape_html(&s.last_seen)
            ),
        );
        let _ = write!(
            out,
            r##"<tr>
  <td>{com}<br><span class="hint">{sci}</span>{nearby_pill}</td>
  <td class="cell-center">{total}</td>
  <td class="cell-center">{days}</td>
  <td class="cell-center">{lo:.0}–{hi:.0}%</td>
  <td>{why}</td>
  <td class="cell-right">
    <form hx-post="/admin/quality/phantoms/exclude" hx-target="#phantoms-section" hx-swap="innerHTML" class="inline-form">
      <input type="hidden" name="sci_name" value="{sci}">
      <button type="submit" class="btn btn-danger del-btn"
              data-confirm-action="hx-post"
              data-confirm-url="/admin/quality/phantoms/exclude"
              data-confirm-title="Exclude species"
              data-confirm-body="{confirm_body}"
              data-confirm-confirm-label="Exclude"
              data-confirm-style="danger">Exclude</button>
    </form>
  </td>
</tr>"##
        );
    }
    out.push_str("</tbody></table>");
    out
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The standalone `/admin/quality` page GET folded into the Station **Data**
/// tab; its old URL permanently redirects there. The summary/trend HTMX
/// partials below keep their `/admin/quality/...` paths.
async fn quality_page() -> axum::response::Redirect {
    axum::response::Redirect::permanent("/station/data#quality")
}

/// Load the quality metrics and render the body (no document shell).
///
/// Shared with the Station **Data** tab
/// (`crate::routes::pages::homes::station_tabs`). Call from a blocking context —
/// `load_quality_data` runs several aggregate queries.
pub(crate) fn quality_body(state: &AppState) -> String {
    render_quality_page(&load_quality_data(state), state.names())
}

async fn quality_summary_partial(State(state): State<AppState>) -> Html<String> {
    let data = state.run_blocking(load_quality_data).await;
    Html(panel(&data.summary, "the summary statistics", |s| {
        render_summary_cards(s.as_ref())
    }))
}

async fn quality_trend_partial(State(state): State<AppState>) -> Html<String> {
    let data = state.run_blocking(load_quality_data).await;
    Html(panel(&data.trend, "the confidence trend", |t| {
        render_confidence_trend(t)
    }))
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

/// One panel's data, or the error that stopped it being read.
///
/// Per panel and not for the page, because the queries are independent and a
/// failure in one says nothing about the others. What they must not do is turn
/// into the default the panel would have drawn for an empty station: every
/// field used to be `.ok()` or `.unwrap_or_default()`, so a database that could
/// not be read drew "No detections in database.", a histogram of six zeros and
/// "Database quality looks good!".
type Loaded<T> = Result<T, birdnet_db::sqlite::DbError>;

struct QualityData {
    /// `None` is a station with nothing to summarise, not a failed read.
    summary: Loaded<Option<QualitySummary>>,
    conf_buckets: Loaded<[i64; 6]>,
    trend: Loaded<Vec<(String, f64)>>,
    by_hour: Loaded<Vec<(u8, i64, f64)>>,
    low_conf: Loaded<Vec<(String, String, i64, f64)>>,
    review_trend: Loaded<Vec<ReviewVerdictDay>>,
    model_vs_review: Loaded<Vec<ModelVsReviewRow>>,
}

/// Render one panel from what was loaded for it, or say it could not be.
fn panel<T>(loaded: &Loaded<T>, what: &str, render: impl FnOnce(&T) -> String) -> String {
    match loaded {
        Ok(data) => render(data),
        Err(e) => {
            tracing::warn!(error = %e, panel = what, "data quality panel could not be read");
            crate::routes::pages::error_states::inline(what)
        }
    }
}

/// The summary, or `None` for a station that has no detections to summarise.
///
/// `quality_summary` cannot tell those apart for the caller: over an empty
/// `detections_analytic` its `SUM` is `NULL`, which does not read as an
/// integer, so it returns an error on a station that has simply heard nothing.
/// The old `.ok()` turned that into "No detections in database." — right for
/// an empty station by accident, and the same sentence for a broken one. A
/// count settles which it is before the summary is asked for.
fn load_summary(conn: &rusqlite::Connection) -> Loaded<Option<QualitySummary>> {
    if birdnet_db::sqlite::analytic_detection_count(conn)? == 0 {
        return Ok(None);
    }
    quality_summary(conn).map(Some)
}

fn load_quality_data(state: &AppState) -> QualityData {
    state.with_read_db(|conn| QualityData {
        summary: load_summary(conn),
        conf_buckets: confidence_distribution(conn),
        trend: confidence_trend(conn, 30),
        by_hour: detection_quality_by_hour(conn),
        // Drawn, not matched: the common name is in the station's language.
        low_conf: low_confidence_species(conn, 0.60, 3).map(|rows| {
            rows.into_iter()
                .map(|(com, sci, count, avg)| {
                    (state.names().name(&sci, &com).to_owned(), sci, count, avg)
                })
                .collect()
        }),
        review_trend: review_verdict_trend(conn, 30),
        model_vs_review: model_vs_review_by_species(conn, 12),
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn render_quality_page(data: &QualityData, names: &birdnet_core::i18n::NameLocalizer) -> String {
    let summary_html = panel(&data.summary, "the summary statistics", |s| {
        render_summary_cards(s.as_ref())
    });
    let dist_html = panel(
        &data.conf_buckets,
        "the confidence distribution",
        render_confidence_distribution,
    );
    let trend_html = panel(&data.trend, "the confidence trend", |t| {
        render_confidence_trend(t)
    });
    let hour_html = panel(&data.by_hour, "the hourly profile", |h| {
        render_hourly_quality(h)
    });
    let low_conf_html = panel(&data.low_conf, "the low-confidence species", |l| {
        render_low_confidence_species(l)
    });
    let review_trend_html = panel(&data.review_trend, "the review verdict trend", |r| {
        render_review_trend(r)
    });
    let model_vs_review_html = panel(
        &data.model_vs_review,
        "the model-versus-human comparison",
        |m| render_model_vs_review(m, names),
    );
    let tuning_link = help_link(Topic::Tuning);
    // Skeletons for the partials that load via hx-trigger="load" (resolves
    // TODO(O-16-followup) for #quality-summary / #quality-trend).
    let summary_skeleton = skeletons::stat_row(6);
    let trend_skeleton = skeletons::hourly_bars(30);

    format!(
        r#"<header class="page-head q-head" data-screen-label="Data quality" data-om-validate>
  <div>
    <div class="bnb-eyebrow">Admin · data quality</div>
    <h2 class="display q-h1">
      Data quality
    </h2>
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
      <h2 class="sh-h">At-a-glance statistics</h2>
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
      <h2 class="sh-h">Daily average confidence</h2>
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
      <h2 class="sh-h">Human disagreement over 30 days</h2>
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
      <h2 class="sh-h">Consistently uncertain calls</h2>
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
  <div class="bar" data-style="height:{height_pct}%;background:{color};"></div>
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
            r#"<div class="trend-bar" data-style="height:{height_pct}%;background:{color};"
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
            r#"<div class="hour-bar" data-style="height:{height_pct}%;background:{color};"
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
      <div class="conf-fill" data-style="width:{bar_pct}%;background:{color};"></div>
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
  <span class="unreviewed" data-style="height:{u}%"></span>
  <span class="confirmed"  data-style="height:{c}%" class-suffix="approved"></span>
  <span class="approved"   data-style="height:{c}%"></span>
  <span class="rejected"   data-style="height:{r}%"></span>
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

fn render_model_vs_review(
    rows: &[ModelVsReviewRow],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
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
    <div class="bnb-mvr-bar model"><span class="fill" data-style="width:{model_w}%"></span><span class="label">model {model_pct:.0}%</span></div>
    <div class="bnb-mvr-bar human"><span class="fill" data-style="width:{human_w}%"></span><span class="label">human {human_pct:.0}%</span></div>
  </div>
  <div class="bnb-mvr-gap{gap_class}">Δ {gap_sign}{gap_abs:.0}%</div>
</li>"#,
            href = escape_html(&species_link),
            com = escape_html(names.name(&r.sci_name, &r.com_name)),
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

#[cfg(test)]
mod phantom_read_failure_tests {
    /// A station whose detections cannot be read is told so, not told that
    /// nothing looks suspicious. Observed failing with `load_phantoms`'s old
    /// `species_shapes(conn).unwrap_or_default()` restored (as an `Ok` of the
    /// empty report): the page said "Nothing looks suspicious".
    #[test]
    fn a_report_that_cannot_be_read_says_so() {
        // No migrations: the `detections` table does not exist.
        let state = crate::state::AppState::from_connection(
            rusqlite::Connection::open_in_memory().expect("db"),
            std::path::PathBuf::from(":memory:"),
        );
        let html = super::render_phantoms_or_error(super::load_phantoms(&state), None);
        assert!(html.contains("could not be read"), "{html}");
        assert!(!html.contains("Nothing looks suspicious"), "{html}");
    }
}

#[cfg(test)]
mod phantom_tests {
    use super::render_phantoms;
    use birdnet_db::phantoms::{PhantomReport, PhantomSignal, SpeciesShape};

    /// A report for one flagged species.
    fn report(sci: &str, com: &str, signals: Vec<PhantomSignal>) -> PhantomReport {
        PhantomReport {
            shape: SpeciesShape {
                sci_name: sci.to_owned(),
                com_name: com.to_owned(),
                total: 80,
                distinct_days: 2,
                min_confidence: 0.71,
                max_confidence: 0.73,
                confirmed: 0,
                rejected: 6,
            },
            signals,
        }
    }

    #[test]
    fn a_flagged_species_is_shown_with_its_reasons_and_its_numbers() {
        // The operator is being asked to remove a species from their own
        // record. A verdict with no evidence beside it is not actionable, and
        // the numbers are what let them overrule it.
        let html = render_phantoms(
            &[report(
                "Phantomus fictus",
                "Not A Bird",
                vec![
                    PhantomSignal::NeverConfirmed,
                    PhantomSignal::ConfinedToFewDays,
                ],
            )],
            None,
        );
        assert!(html.contains("Not A Bird"), "{html}");
        assert!(html.contains("Phantomus fictus"), "{html}");
        assert!(html.contains("every review rejected"), "{html}");
        assert!(html.contains("many detections, few days"), "{html}");
        assert!(
            html.contains("80"),
            "the detection count is missing: {html}"
        );
        assert!(
            html.contains("71–73%"),
            "the confidence band is missing: {html}"
        );
    }

    #[test]
    fn an_empty_report_says_so_rather_than_rendering_an_empty_table() {
        // A bare table header reads as a broken page. It also has to say what
        // "nothing" means, or a station whose species all sit below the
        // minimum looks like one with nothing to find.
        let html = render_phantoms(&[], None);
        assert!(html.contains("Nothing looks suspicious"), "{html}");
        assert!(
            !html.contains("<tbody>"),
            "an empty table was rendered: {html}"
        );
        assert!(
            html.contains("10 detections"),
            "the threshold is not explained: {html}"
        );
    }

    /// The report is swapped into `#phantoms-section`, which its callers
    /// open and close, so it must close nothing of its own. Observed failing
    /// with the old trailing `</div>` on both paths: one each, and on
    /// `/station/data` it closed the page's content column early.
    #[test]
    fn the_report_closes_only_what_it_opens() {
        for html in [
            render_phantoms(&[], None),
            render_phantoms(
                &[report(
                    "Phantomus fictus",
                    "Not A Bird",
                    vec![PhantomSignal::NeverConfirmed],
                )],
                None,
            ),
        ] {
            assert_eq!(
                html.matches("<div").count(),
                html.matches("</div>").count(),
                "{html}"
            );
        }
    }

    #[test]
    fn the_exclude_button_carries_the_scientific_name() {
        // The exclusion list is keyed on the scientific name; posting the
        // common name would silently exclude nothing.
        let html = render_phantoms(
            &[report(
                "Phantomus fictus",
                "Not A Bird",
                vec![
                    PhantomSignal::NeverConfirmed,
                    PhantomSignal::ConfinedToFewDays,
                ],
            )],
            None,
        );
        assert!(
            html.contains(r#"name="sci_name" value="Phantomus fictus""#),
            "{html}"
        );
    }

    #[test]
    fn excluding_is_confirmed_before_it_happens() {
        // It stops a species being recorded at all. The station's other
        // destructive actions all confirm first, and this is the one that is
        // easiest to press by accident while reading a table.
        let html = render_phantoms(
            &[report(
                "Phantomus fictus",
                "Not A Bird",
                vec![
                    PhantomSignal::NeverConfirmed,
                    PhantomSignal::ConfinedToFewDays,
                ],
            )],
            None,
        );
        assert!(html.contains("data-confirm-action"), "{html}");
        assert!(
            html.contains("Existing detections are kept"),
            "the confirmation does not say what it will and will not do: {html}"
        );
    }

    #[test]
    fn a_species_name_is_escaped_in_the_row_and_the_form() {
        // Names reach here from the model's label file, and land in a table
        // cell, an attribute value, and the confirmation text.
        let html = render_phantoms(
            &[report(
                r#"Evil" onload="x"#,
                r"<script>alert(1)</script>",
                vec![
                    PhantomSignal::NeverConfirmed,
                    PhantomSignal::ConfinedToFewDays,
                ],
            )],
            None,
        );
        assert!(!html.contains(r#"onload="x"#), "{html}");
        assert!(!html.contains("<script>alert"), "{html}");
    }
}

// ---------------------------------------------------------------------------
// G-27: eBird corroboration in the suspect-species report
// ---------------------------------------------------------------------------

#[cfg(test)]
mod nearby_phantom_tests {
    use super::render_phantoms;
    use birdnet_db::phantoms::{PhantomReport, PhantomSignal, SpeciesShape};
    use birdnet_integrations::ebird::{NearbySpecies, Snapshot};

    fn flagged(sci: &str) -> PhantomReport {
        PhantomReport {
            shape: SpeciesShape {
                sci_name: sci.to_owned(),
                com_name: "Not A Bird".to_owned(),
                total: 80,
                distinct_days: 2,
                min_confidence: 0.71,
                max_confidence: 0.73,
                confirmed: 0,
                rejected: 6,
            },
            signals: vec![
                PhantomSignal::NeverConfirmed,
                PhantomSignal::ConfinedToFewDays,
            ],
        }
    }

    fn snapshot_of(sci_normalised: &str) -> Snapshot {
        Snapshot {
            fetched_at: 1_800_000_000,
            scope: "geo:42.45,-76.48:25".to_owned(),
            scope_label: "within 25 km".to_owned(),
            back_days: 14,
            species: vec![NearbySpecies {
                sci_name: sci_normalised.to_owned(),
                com_name: "A Real Bird".to_owned(),
                last_seen: "2026-09-07".to_owned(),
                locality: "Hanshaw Rd. fields".to_owned(),
                how_many: Some(1),
            }],
        }
    }

    /// The most valuable thing eBird can say on this page: *do not exclude
    /// this one*. It must be visible in the row and repeated in the
    /// confirmation, because the confirmation is the last thing the operator
    /// reads before removing a species from their own record.
    ///
    /// Observed failing with the `seen` lookup hard-coded to `None`: neither
    /// the pill nor the warning appeared, and the confirmation was the plain
    /// one.
    #[test]
    fn a_flagged_species_somebody_reported_nearby_is_marked_and_its_warning_says_so() {
        let html = render_phantoms(
            &[flagged("Turdus merula")],
            Some(&snapshot_of("turdus merula")),
        );
        assert!(html.contains("eBird: reported nearby 2026-09-07"), "{html}");
        assert!(
            html.contains("probably a real bird"),
            "the confirmation does not warn: {html}"
        );
    }

    /// The discrimination counterpart. A species eBird has nothing on must
    /// render exactly as it did before this feature existed — no pill, and
    /// the plain confirmation — because eBird's silence is evidence of
    /// nothing at all.
    ///
    /// Without this gate, a version that marked every row would pass the one
    /// above.
    #[test]
    fn a_flagged_species_ebird_says_nothing_about_is_unchanged() {
        let with_snapshot = render_phantoms(
            &[flagged("Phantomus fictus")],
            Some(&snapshot_of("turdus merula")),
        );
        assert!(
            !with_snapshot.contains("eBird: reported nearby"),
            "{with_snapshot}"
        );
        assert!(
            !with_snapshot.contains("probably a real bird"),
            "{with_snapshot}"
        );
        assert!(
            with_snapshot.contains("Stop recording Not A Bird?"),
            "the plain confirmation is missing: {with_snapshot}"
        );
    }

    /// A station with no eBird key renders the report it always rendered,
    /// down to the absence of the explanatory paragraph.
    #[test]
    fn a_station_with_no_snapshot_renders_the_report_it_always_did() {
        let html = render_phantoms(&[flagged("Turdus merula")], None);
        assert!(!html.contains("eBird"), "{html}");
        assert!(html.contains("Stop recording Not A Bird?"), "{html}");
    }

    /// The explanatory paragraph has to say what silence means, or an
    /// operator reads an unmarked row as "eBird disagrees".
    #[test]
    fn the_explanation_says_that_silence_is_not_evidence() {
        let html = render_phantoms(
            &[flagged("Turdus merula")],
            Some(&snapshot_of("turdus merula")),
        );
        assert!(html.contains("is <em>not</em> evidence"), "{html}");
        assert!(html.contains("within 25 km"), "{html}");
        assert!(html.contains("14 days"), "{html}");
    }

    /// Matching is on the normalised scientific name, so a label file that
    /// spells a name with different case still corroborates.
    #[test]
    fn the_match_survives_a_difference_of_case() {
        let html = render_phantoms(
            &[flagged("TURDUS MERULA")],
            Some(&snapshot_of("turdus merula")),
        );
        assert!(html.contains("eBird: reported nearby"), "{html}");
    }
}
