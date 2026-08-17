//! Behavioral analytics HTMX partials (requires duckdb-behavioral extension).

// `analytics_config_partial` uses `write!` macro on a String regardless of
// whether the analytics feature is enabled — the surrounding `if let Some(...)
// = ext_status` branch is unreachable without the feature, but the macro still
// needs the trait in scope to compile.
use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use super::{ANALYTICS_PAGE_HTML, escape_html};
use crate::state::AppState;

/// Mount the behavioral analytics page and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/analytics-sessions", get(analytics_sessions_partial))
        .route(
            "/pages/analytics-retention",
            get(analytics_retention_partial),
        )
        .route("/pages/analytics-next", get(analytics_next_partial))
        .route(
            "/pages/analytics-dawn-sequence",
            get(analytics_dawn_sequence_partial),
        )
        .route("/pages/analytics-config", get(analytics_config_partial))
}

/// The behavioral-analytics surface, rendered for embedding by
/// `homes::patterns` ("Behavior" tab).
pub(super) fn content() -> String {
    ANALYTICS_PAGE_HTML.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    )
}

/// HTMX partial: activity sessions table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_sessions_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Activity sessions");
    }
    let params = birdnet_behavioral::types::SessionizeParams::default();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.sessionize(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(sessions)) => {
            // A "burst of singing" made of one detection lasting 0s is not a
            // burst. Sessionisation groups a species' detections by a 20-minute
            // gap, so a sparse species yields singletons — structurally correct
            // and semantically empty, and the panel rendered a table of nothing
            // but those, which reads as broken to anyone who looks at it.
            //
            // Filter to real runs and say plainly when there are none, rather
            // than showing rows that undermine the reader's trust in the rest
            // of the page.
            let bursts: Vec<_> = sessions.iter().filter(|s| s.detection_count > 1).collect();
            if bursts.is_empty() {
                let seen = sessions.len();
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    format!(
                        r#"<p class="bh-muted">No bursts yet. {seen} single detections have been                            grouped so far, but none is part of a run — a burst needs at least two                            detections of one species within about 20 minutes.</p>"#
                    ),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Species</th><th>Detections</th><th>Start</th><th>Duration</th></tr></thead><tbody>",
            );
            for s in bursts.iter().take(20) {
                let duration = format_duration(s.duration_secs);
                let _ = write!(
                    html,
                    r"<tr><td>{sp}</td><td>{c}</td><td>{st}</td><td>{d}</td></tr>",
                    sp = escape_html(&s.species),
                    c = s.detection_count,
                    st = escape_html(&s.start_time),
                    d = duration,
                );
            }
            html.push_str("</tbody></table>");
            if sessions.len() > 20 {
                let _ = write!(
                    html,
                    r#"<p class="bh-note">Showing 20 of {} sessions.</p>"#,
                    sessions.len()
                );
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("sessions", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading sessions</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_sessions_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Activity sessions")
}

/// HTMX partial: species retention table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_retention_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Species retention");
    }
    let params = birdnet_behavioral::types::RetentionParams::default();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.retention(&params))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(retention)) => {
            if retention.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="bh-muted">No retention data yet.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Species</th><th>Classification</th><th>Day 1</th><th>Day 7</th><th>Day 30</th></tr></thead><tbody>",
            );
            for r in &retention {
                let (label, cls) = match r.classification {
                    birdnet_behavioral::types::ResidencyType::Resident => ("Resident", "high"),
                    birdnet_behavioral::types::ResidencyType::Regular => ("Regular", "mid"),
                    birdnet_behavioral::types::ResidencyType::Migrant => ("Migrant", "low"),
                    birdnet_behavioral::types::ResidencyType::Rarity => ("Rarity", "low"),
                };
                let _ = write!(
                    html,
                    r#"<tr><td>{sp}</td><td><span class="conf {cls}">{label}</span></td><td>{d1}</td><td>{d7}</td><td>{d30}</td></tr>"#,
                    sp = escape_html(&r.species),
                    d1 = find_rate(&r.retention_rates, 1),
                    d7 = find_rate(&r.retention_rates, 7),
                    d30 = find_rate(&r.retention_rates, 30),
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("retention", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading retention</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_retention_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Species retention")
}

/// HTMX partial: next-species predictions.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_next_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Next species predictions");
    }
    let trigger_result = tokio::task::spawn_blocking({
        let s = state.clone();
        move || {
            s.with_db(|conn| {
                conn.query_row(
                    "SELECT Com_Name FROM detections ORDER BY rowid DESC LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            })
        }
    })
    .await;

    let Ok(Some(trigger)) = trigger_result else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p class="bh-muted">No detections yet.</p>"#.to_string(),
        );
    };

    let display = trigger.clone();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics(|adb| adb.next_species(&trigger, 60, 5))
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok(predictions)) => {
            if predictions.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    format!(
                        r#"<p class="bh-muted">No predictions for <strong>{}</strong> yet.</p>"#,
                        escape_html(&display)
                    ),
                );
            }
            let mut html = format!(
                r#"<p class="bh-after">After <strong>{}</strong>:</p><table><thead><tr><th>Species</th><th>Probability</th><th>Observed</th></tr></thead><tbody>"#,
                escape_html(&display),
            );
            for p in &predictions {
                // The trigger species appears among its own follow-ons — the
                // same bird calling again — which under a heading that reads
                // "which tends to turn up **next**" is confusing rather than
                // informative. Label it instead of dropping it: that it sings
                // again is a real fact about the species, just not a
                // *succession* fact.
                let self_follow = p.predicted_species == display;
                let pct = p.probability * 100.0;
                let cls = if pct >= 50.0 {
                    "high"
                } else if pct >= 20.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr><td>{sp}{note}</td><td><span class="conf {cls}">{pct:.0}%</span></td><td>{f}</td></tr>"#,
                    sp = escape_html(&p.predicted_species),
                    note = if self_follow {
                        r#" <span class="bnb-meta">(calls again)</span>"#
                    } else {
                        ""
                    },
                    f = p.frequency
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => extension_error_html("next_species", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading predictions</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_next_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Next species predictions")
}

/// Dawn window the sequence card analyses (hours of day, inclusive).
#[cfg(feature = "analytics")]
const DAWN_HOUR_START: u32 = 4;
#[cfg(feature = "analytics")]
const DAWN_HOUR_END: u32 = 8;
/// Funnel window (minutes) — a morning's run may span the whole dawn window, so
/// the `window_funnel` window covers hours 4–8.
#[cfg(feature = "analytics")]
const DAWN_FUNNEL_WINDOW_MINUTES: u32 = 240;

/// HTMX partial: the dawn "running order" — how often the morning's leading
/// voices sing in sequence (`sequence_count`, v0.8.0) plus the step timing of a
/// recent run (`sequence_match_events`, v0.8.0).
///
/// The sequence is derived from the station's own dawn-window data rather than
/// hard-coded, so the card is meaningful regardless of geography — the REST
/// defaults are European, but a North-American dawn opens with entirely
/// different birds.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_dawn_sequence_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Dawn sequence");
    }
    let result = tokio::task::spawn_blocking(move || {
        let sequence = state.with_db(derive_dawn_sequence);
        // sequence_count / sequence_match_events need 2..=32 steps; fewer than
        // two prominent dawn voices means there's no order to read yet.
        if sequence.len() < 2 {
            return Ok(None);
        }
        let params = birdnet_behavioral::types::PatternParams {
            species_sequence: sequence.clone(),
            max_gap_minutes: None,
            hour_start: DAWN_HOUR_START,
            hour_end: DAWN_HOUR_END,
        };
        let funnel_params = birdnet_behavioral::types::FunnelParams {
            species_sequence: sequence.clone(),
            window_minutes: DAWN_FUNNEL_WINDOW_MINUTES,
            hour_start: DAWN_HOUR_START,
            hour_end: DAWN_HOUR_END,
        };
        state
            .with_analytics(|adb| {
                // Both run the same NFA pattern over the same params: how *often*
                // the ordered run completed (sequence_count) and the per-step
                // timestamps (sequence_match_events). Sharing the pattern means a
                // counted full-match day always has a full set of step times to
                // show, so the headline and the morning we surface stay aligned.
                let counts = adb.sequence_count(&params)?;
                let events = adb.sequence_match_events(&params)?;
                // window_funnel over the same sequence: how far each morning got
                // down the chain, aggregated into the funnel picture.
                let funnel = adb.funnel(&funnel_params)?;
                Ok((sequence, counts, events, funnel))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
            .map(Some)
    })
    .await;

    match result {
        Ok(Ok(Some((sequence, counts, events, funnel)))) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_dawn_sequence(&sequence, &counts, &events, &funnel),
        ),
        Ok(Ok(None)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p class="bh-muted">Not enough dawn activity yet to read a running order — give the mornings a little longer.</p>"#.to_string(),
        ),
        Ok(Err(e)) => extension_error_html("dawn sequence", &e.to_string()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading dawn sequence</p>".to_string(),
        ),
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_dawn_sequence_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Dawn sequence")
}

/// Derive the station's dawn "running order" from its own data: the most
/// prominent dawn-window voices (hours 4–8), ordered by the mean time of day
/// they sing — earliest first. Returns up to three species; fewer than two
/// means there isn't enough dawn activity to read an order.
#[cfg(feature = "analytics")]
fn derive_dawn_sequence(conn: &rusqlite::Connection) -> Vec<String> {
    // Top five dawn voices by volume, then ordered by mean time-of-day so the
    // sequence reads as the natural progression of the morning chorus.
    const SQL: &str = "WITH dawn AS (
            SELECT Com_Name,
                   COUNT(*) AS c,
                   AVG(CAST(substr(Time, 1, 2) AS REAL) * 3600
                       + CAST(substr(Time, 4, 2) AS REAL) * 60
                       + CAST(substr(Time, 7, 2) AS REAL)) AS avg_secs
            FROM detections
            WHERE length(Time) >= 8
              AND CAST(substr(Time, 1, 2) AS INTEGER) BETWEEN 4 AND 8
            GROUP BY Com_Name
            HAVING c >= 10
        )
        SELECT Com_Name FROM (
            SELECT Com_Name, avg_secs FROM dawn ORDER BY c DESC LIMIT 5
        ) ORDER BY avg_secs ASC LIMIT 3";
    let Ok(mut stmt) = conn.prepare(SQL) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}

/// Trim a `DuckDB` timestamp string (`2026-04-12 05:58:23`) to `HH:MM`.
#[cfg(feature = "analytics")]
fn step_time_hhmm(ts: &str) -> &str {
    ts.split([' ', 'T'])
        .nth(1)
        .and_then(|t| t.get(..5))
        .unwrap_or(ts)
}

/// The most recent morning that completed the whole ordered sequence (a full
/// set of step times), to show step-by-step. `events` arrive date-descending,
/// so `find` yields the most recent. Days that reached only a partial in-order
/// prefix aren't a full run, so they're skipped — the card only calls this when
/// `sequence_count` already found a full match, so one always exists.
#[cfg(feature = "analytics")]
fn best_progression(
    full_len: usize,
    events: &[birdnet_behavioral::types::PatternMatchEvents],
) -> Option<&birdnet_behavioral::types::PatternMatchEvents> {
    events.iter().find(|e| e.step_times.len() == full_len)
}

/// Per-step "mornings that reached this step" counts from the per-day funnel
/// results: step `k` (1-based) is the number of days whose `steps_completed`
/// reached at least `k`. The result is non-increasing — the funnel shape.
#[cfg(feature = "analytics")]
fn funnel_step_counts(
    funnel: &[birdnet_behavioral::types::ChorusFunnel],
    total_steps: usize,
) -> Vec<u64> {
    let total = u32::try_from(total_steps).unwrap_or(u32::MAX);
    (1..=total)
        .map(|k| {
            u64::try_from(funnel.iter().filter(|f| f.steps_completed >= k).count()).unwrap_or(0)
        })
        .collect()
}

/// Render the dawn-sequence card body from the derived sequence, its per-day
/// occurrence counts (`sequence_count`) and step timings (`sequence_match_events`).
#[cfg(feature = "analytics")]
fn render_dawn_sequence(
    sequence: &[String],
    counts: &[birdnet_behavioral::types::PatternCount],
    events: &[birdnet_behavioral::types::PatternMatchEvents],
    funnel: &[birdnet_behavioral::types::ChorusFunnel],
) -> String {
    let chain = sequence
        .iter()
        .map(|s| escape_html(s))
        .collect::<Vec<_>>()
        .join(" \u{2192} ");

    let total_occ: u64 = counts.iter().map(|c| c.count).sum();
    let match_days = counts.iter().filter(|c| c.count > 0).count();
    let total_days = counts.len();
    let best = counts.iter().map(|c| c.count).max().unwrap_or(0);

    let mut html =
        format!(r#"<p class="bh-after">Your dawn tends to open <strong>{chain}</strong>.</p>"#);

    // Lead with the picture (the Patterns idiom): the funnel of how many
    // mornings reach each step of the run. Omitted — never an empty chart — when
    // nothing reached even the first step.
    let step_counts = funnel_step_counts(funnel, sequence.len());
    if step_counts.first().is_some_and(|c| *c > 0) {
        let steps: Vec<(String, u64)> = sequence.iter().cloned().zip(step_counts).collect();
        html.push_str(r#"<p class="bnb-meta">Mornings reaching each step:</p>"#);
        html.push_str(&super::viz::sequence_funnel(&steps));
    }

    if total_occ == 0 {
        html.push_str(
            r#"<p class="bh-muted">All heard at dawn, but not yet in that exact order on a single morning.</p>"#,
        );
        return html;
    }

    let _ = write!(
        html,
        r#"<p class="bnb-meta">In order on <strong>{match_days}</strong> of <strong>{total_days}</strong> mornings · <strong>{total_occ}</strong> runs in total · up to <strong>{best}</strong> in one morning.</p>"#
    );

    if let Some(ev) = best_progression(sequence.len(), events) {
        let _ = write!(
            html,
            r#"<p class="bh-after">A recent morning — <strong>{date}</strong>:</p><table class="pt-tbl"><thead><tr><th>Voice</th><th>First heard</th></tr></thead><tbody>"#,
            date = escape_html(&ev.date),
        );
        // step_times[i] pairs with species_sequence[i] for the completed steps;
        // zip stops at the shorter, so partial runs show only what fired.
        for (sp, t) in ev.species_sequence.iter().zip(ev.step_times.iter()) {
            let _ = write!(
                html,
                "<tr><td>{sp}</td><td>{t}</td></tr>",
                sp = escape_html(sp),
                t = escape_html(step_time_hhmm(t)),
            );
        }
        html.push_str("</tbody></table>");
    }

    // The most recent mornings, tucked under a disclosure (the Patterns
    // "see the numbers" idiom) so the card leads with the headline.
    html.push_str(
        r#"<details class="pt-disc"><summary>Recent mornings</summary><div><table class="pt-tbl"><thead><tr><th>Morning</th><th>In-order runs</th></tr></thead><tbody>"#,
    );
    for c in counts.iter().take(7) {
        let _ = write!(
            html,
            "<tr><td>{date}</td><td>{count}</td></tr>",
            date = escape_html(&c.date),
            count = c.count,
        );
    }
    html.push_str("</tbody></table></div></details>");

    html
}

async fn analytics_config_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();
    let db_path = escape_html(&state.db_path().display().to_string());
    let version = env!("CARGO_PKG_VERSION");

    // Pull the live extension status from the AnalyticsDb when one is open. The
    // three flags — compiled / active / extension-loaded — measure independent
    // truths, so they're shown as three distinct rows instead of one ambiguous
    // "Connected" pill.
    #[cfg(feature = "analytics")]
    let ext_status: Option<(bool, Option<String>, Option<String>)> = state.with_analytics(|db| {
        (
            db.extension_loaded(),
            db.duckdb_version(),
            db.extension_version(),
        )
    });
    #[cfg(not(feature = "analytics"))]
    let ext_status: Option<(bool, Option<String>, Option<String>)> = None;

    let mut html = format!(
        r#"<table class="bh-config-table"><tr><td class="bh-key">Version</td><td>{version}</td></tr>
<tr><td class="bh-key">SQLite Database</td><td><code>{db_path}</code></td></tr>
<tr><td class="bh-key">Analytics Compiled</td><td>{compiled}</td></tr>
<tr><td class="bh-key">Analytics Active</td><td>{configured}</td></tr>"#,
    );
    if let Some((loaded, duckdb_v, ext_v)) = ext_status {
        let loaded_str = if loaded { "true" } else { "false" };
        let duckdb_v_str = escape_html(duckdb_v.as_deref().unwrap_or("unknown"));
        let ext_v_str = escape_html(ext_v.as_deref().unwrap_or("\u{2014}"));
        let _ = write!(
            html,
            "<tr><td class=\"bh-key\">DuckDB</td><td><code>{duckdb_v_str}</code></td></tr>\
             <tr><td class=\"bh-key\">Behavioral extension</td><td><code>{ext_v_str}</code> \u{00b7} loaded: <strong>{loaded_str}</strong></td></tr>"
        );
        if !loaded {
            html.push_str(
                r#"<tr><td colspan="2" class="bh-cell-note">Extension not loaded — sessions, retention and next-species queries are unavailable. Run <code>--refresh-extension</code> to fetch from the community registry, or restart with a release that bundles the extension binary (sets <code>BIRDNET_BUNDLED_EXTENSION_FILE</code> at build time, or vendors a copy under <code>crates/birdnet-behavioral/vendor/</code>).</td></tr>"#,
            );
        }
    }
    if compiled && !configured {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">Analytics is on by default — restart the service to open the DuckDB file alongside the SQLite database.</td></tr>"#);
    } else if !compiled {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">Rebuild with default features (or <code>--features analytics</code>) to enable.</td></tr>"#);
    }
    html.push_str("</table>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

fn analytics_unavailable_html(
    feature: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let msg = if cfg!(feature = "analytics") {
        format!(
            r#"<p class="bh-muted">{feature} requires DuckDB analytics. Start with <code>--analytics-db</code>.</p>"#
        )
    } else {
        format!(
            r#"<p class="bh-muted">{feature} requires the analytics feature. Rebuild with <code>--features analytics</code>.</p>"#
        )
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

#[cfg(feature = "analytics")]
fn extension_error_html(
    func: &str,
    error: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    let html = format!(
        r#"<p class="bh-muted">The <code>duckdb-behavioral</code> extension is required for {func}.</p>
<p class="bh-muted-sm">{error}</p>"#,
        error = escape_html(error),
    );
    // Return 200 (not 503) so HTMX swaps this informative fragment into the
    // card; a non-2xx response leaves the "Loading…" placeholder stuck.
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(feature = "analytics")]
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(feature = "analytics")]
fn find_rate(rates: &[birdnet_behavioral::types::RetentionRate], days: u32) -> String {
    rates
        .iter()
        .find(|r| r.days == days)
        .map_or_else(|| "—".to_string(), |r| format!("{:.0}%", r.rate * 100.0))
}

#[cfg(all(test, feature = "analytics"))]
mod tests {
    use super::funnel_step_counts;
    use birdnet_behavioral::types::ChorusFunnel;

    fn cf(steps_completed: u32) -> ChorusFunnel {
        ChorusFunnel {
            date: "2026-06-21".into(),
            steps_completed,
            total_steps: 3,
            matched_species: Vec::new(),
        }
    }

    #[test]
    fn step_counts_are_non_increasing() {
        // Three mornings reached step >=1, two reached >=2, one reached >=3.
        let funnel = vec![cf(3), cf(2), cf(1)];
        assert_eq!(funnel_step_counts(&funnel, 3), vec![3, 2, 1]);
    }

    #[test]
    fn none_reaching_first_step_is_all_zero() {
        let funnel = vec![cf(0), cf(0)];
        assert_eq!(funnel_step_counts(&funnel, 3), vec![0, 0, 0]);
    }

    #[test]
    fn zero_total_steps_is_empty() {
        assert_eq!(funnel_step_counts(&[], 0), Vec::<u64>::new());
    }
}
