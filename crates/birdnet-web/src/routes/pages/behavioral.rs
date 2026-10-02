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
    let (gap_minutes, limit) = (params.gap_minutes, params.limit);
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
        Ok(Ok(sessions)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_bursts(&sessions, gap_minutes, limit),
        ),
        Ok(Err(e)) => analytics_error_html("activity sessions", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics sessions: task failed");
            super::error_states::failed_partial("this station's listening sessions")
        }
    }
}

/// The bursts table: sessions of two or more detections, newest first.
///
/// A "burst of singing" made of one detection lasting 0s is not a burst.
/// Sessionisation groups a species' detections by a `gap_minutes` gap, so a
/// sparse species yields singletons — structurally correct and semantically
/// empty, and a table of nothing but those reads as broken to anyone who looks
/// at it. Filter to real runs and say plainly when there are none.
///
/// Both counts on the card are of what is actually shown. The empty state
/// named "about 20 minutes" while the query grouped by 30, and the footnote
/// "Showing 20 of N sessions" counted the filtered-out singletons in N — and N
/// could never pass `limit`, because only the newest `limit` sessions are
/// fetched.
#[cfg(feature = "analytics")]
fn render_bursts(
    sessions: &[birdnet_behavioral::types::ActivitySession],
    gap_minutes: u32,
    limit: u32,
) -> String {
    const SHOWN: usize = 20;
    let bursts: Vec<_> = sessions.iter().filter(|s| s.detection_count > 1).collect();
    if bursts.is_empty() {
        let seen = sessions.len();
        return format!(
            r#"<p class="bh-muted">No bursts yet. {seen} single detections have been grouped so far, but none is part of a run — a burst needs at least two detections of one species within {gap_minutes} minutes of each other.</p>"#
        );
    }
    let mut html = String::from(
        r"<table><thead><tr><th>Species</th><th>Detections</th><th>Start</th><th>Duration</th></tr></thead><tbody>",
    );
    for s in bursts.iter().take(SHOWN) {
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
    if bursts.len() > SHOWN {
        let scope = if u32::try_from(sessions.len()).is_ok_and(|n| n >= limit) {
            format!(" found among the latest {limit} sessions")
        } else {
            String::new()
        };
        let _ = write!(
            html,
            r#"<p class="bh-note">Showing the {SHOWN} most recent of {n} bursts{scope}.</p>"#,
            n = bursts.len(),
        );
    }
    html
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
                r"<table><thead><tr><th>Species</th><th>Classification</th><th>Weeks heard</th><th>Day 1</th><th>Day 7</th><th>Day 30</th></tr></thead><tbody>",
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
                    r#"<tr><td>{sp}</td><td><span class="conf {cls}">{label}</span></td><td class="mono">{wk} of {of}</td><td>{d1}</td><td>{d7}</td><td>{d30}</td></tr>"#,
                    sp = escape_html(&r.species),
                    // The class is read from this, so it is shown beside it.
                    wk = r.weeks_present,
                    of = r.station_weeks,
                    d1 = find_rate(&r.retention_rates, 1),
                    d7 = find_rate(&r.retention_rates, 7),
                    d30 = find_rate(&r.retention_rates, 30),
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => analytics_error_html("return visits", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics retention: task failed");
            super::error_states::failed_partial("which birds came back")
        }
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
                    "SELECT Com_Name FROM detections_analytic ORDER BY rowid DESC LIMIT 1",
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
        Ok(Err(e)) => analytics_error_html("what sings next", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics next: task failed");
            super::error_states::failed_partial("what the station expects to hear next")
        }
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
/// the `window_funnel` window covers all of it: hours 4 to 8 *inclusive* are
/// five hours, 04:00–08:59. It was 240, so a run the headline counted as "in
/// order" (`sequence_count`, no gap limit) could fall short in the funnel.
#[cfg(feature = "analytics")]
const DAWN_FUNNEL_WINDOW_MINUTES: u32 = (DAWN_HOUR_END - DAWN_HOUR_START + 1) * 60;

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
        Ok(Err(e)) => analytics_error_html("the dawn sequence", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics dawn sequence: task failed");
            super::error_states::failed_partial("the order birds joined the dawn chorus")
        }
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_dawn_sequence_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Dawn sequence")
}

/// Derive the station's dawn "running order" from its own data: the most
/// prominent dawn-window voices (hours 4–8), ordered by when each typically
/// *starts* — the mean, over the mornings it was heard, of its first dawn
/// detection that morning. Returns up to three species; fewer than two means
/// there isn't enough dawn activity to read an order.
///
/// The card reads "your dawn tends to open …", which is a claim about who
/// starts first. It was ordered by the mean time of *all* of a species' dawn
/// song, so a bird up at 04:30 that sang on past eight was placed after one
/// that sang only from five to twenty past.
#[cfg(feature = "analytics")]
fn derive_dawn_sequence(conn: &rusqlite::Connection) -> Vec<String> {
    // Top five dawn voices by volume, then ordered by typical first-song time
    // so the sequence reads as the order the morning chorus opens in.
    const SQL: &str = "WITH dawn_rows AS (
            SELECT Date, Com_Name,
                   CAST(substr(Time, 1, 2) AS REAL) * 3600
                       + CAST(substr(Time, 4, 2) AS REAL) * 60
                       + CAST(substr(Time, 7, 2) AS REAL) AS secs
            FROM detections_analytic
            WHERE length(Time) >= 8
              AND CAST(substr(Time, 1, 2) AS INTEGER) BETWEEN 4 AND 8
        ),
        mornings AS (
            SELECT Com_Name, Date, COUNT(*) AS n, MIN(secs) AS first_secs
            FROM dawn_rows
            GROUP BY Com_Name, Date
        ),
        dawn AS (
            SELECT Com_Name, SUM(n) AS c, AVG(first_secs) AS opens_secs
            FROM mornings
            GROUP BY Com_Name
            HAVING c >= 10
        )
        SELECT Com_Name FROM (
            SELECT Com_Name, opens_secs FROM dawn ORDER BY c DESC LIMIT 5
        ) ORDER BY opens_secs ASC LIMIT 3";
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
                // The commands stay — this is a diagnostics table and the
                // person who can act on them reads it — but they no longer
                // lead. The first sentence is for the station's owner, who
                // was previously handed a build-time environment variable and
                // a path inside the source repository.
                r#"<tr><td colspan="2" class="bh-cell-note">The extra behaviour insights (activity sessions, return visits, what sings next) are not available on this station. Everything else keeps recording as normal.<br><span class="bh-muted-sm">For whoever set the station up: run <code>--refresh-extension</code> to fetch the extension from the community registry, or use a release that bundles it (<code>BIRDNET_BUNDLED_EXTENSION_FILE</code> at build time, or a copy vendored under <code>crates/birdnet-behavioral/vendor/</code>).</span></td></tr>"#,
            );
        }
    }
    if compiled && !configured {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">Analytics is on by default — restart the service to open the DuckDB file alongside the SQLite database.</td></tr>"#);
    } else if !compiled {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">This build does not include the extra behaviour insights.<br><span class="bh-muted-sm">For whoever set the station up: rebuild with default features, or <code>--features analytics</code>.</span></td></tr>"#);
    }
    html.push_str("</table>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

fn analytics_unavailable_html(
    feature: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    // These render inside the ordinary analytics cards, not the diagnostics
    // table, so they carry no command at all: the reader of a card about the
    // dawn chorus is not holding a terminal.
    let msg = format!(
        r#"<p class="bh-muted">{feature} isn't available on this station — the extra behaviour insights aren't switched on.</p>
<p class="bh-muted-sm">Everything else keeps recording as normal. Whoever set the station up can turn them on.</p>"#
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

/// What to put in a behavioural card that could not be filled.
///
/// This used to open, for every failure, with "The `duckdb-behavioral`
/// extension is required for {func}" followed by the raw
/// `AnalyticsError::to_string()`. The header sentence was simply wrong for
/// three of the four variants: a query error and a lock held by another
/// process both loaded the extension perfectly well. So the card told its
/// reader to install something that was already installed, and then showed
/// them `DuckDB error: Binder Error: ...` underneath.
///
/// `what` completes "We couldn't load {what}", so pass a noun phrase in the
/// reader's words.
#[cfg(feature = "analytics")]
fn analytics_error_html(
    what: &str,
    error: &birdnet_behavioral::connection::AnalyticsError,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    use birdnet_behavioral::connection::AnalyticsError;
    tracing::warn!(error = %error, "behavioural card could not be filled: {what}");
    let html = match error {
        // The one case where the old sentence was true. Said without naming a
        // build flag or an environment variable: the reader of this page is
        // not the person who compiles it.
        AnalyticsError::ExtensionLoad(_) => format!(
            r#"<p class="bh-muted">The extra behaviour insights aren't switched on for this station, so {what} can't be worked out here.</p>
<p class="bh-muted-sm">Everything else on the station keeps recording as normal.</p>"#
        ),
        // Transient and self-clearing: say so, because "try again" is the
        // whole of the advice and it actually works.
        AnalyticsError::Locked(_) => format!(
            r#"<p class="bh-muted">The analytics database is busy right now, so {what} isn't ready.</p>
<p class="bh-muted-sm">This usually clears in a moment — reload the page to try again.</p>"#
        ),
        _ => super::error_states::inline(what),
    };
    // Return 200 (not 503) so HTMX swaps this informative fragment into the
    // card; a non-2xx response leaves the "Loading..." placeholder stuck.
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

    fn session(n: u32) -> birdnet_behavioral::types::ActivitySession {
        birdnet_behavioral::types::ActivitySession {
            species: "Great Tit".into(),
            session_id: 1,
            detection_count: n,
            start_time: "2026-05-01 05:00:00".into(),
            end_time: "2026-05-01 05:10:00".into(),
            duration_secs: 600,
        }
    }

    /// The footnote counts the bursts, not the singletons filtered out of the
    /// table, and says when the list was cut at the fetch limit.
    #[test]
    fn the_bursts_footnote_counts_bursts() {
        let mut sessions: Vec<_> = (0..25).map(|_| session(3)).collect();
        sessions.extend((0..75).map(|_| session(1)));
        let html = super::render_bursts(&sessions, 30, 100);
        assert!(
            html.contains(
                "Showing the 20 most recent of 25 bursts found among the latest 100 sessions."
            ),
            "{html}"
        );
        // Counterpart: under the limit, no claim about a cut.
        let html = super::render_bursts(&sessions[..30], 30, 100);
        assert!(html.contains("of 25 bursts.</p>"), "{html}");
    }

    /// The empty state names the gap the grouping actually used.
    #[test]
    fn the_empty_bursts_message_names_the_real_gap() {
        let html = super::render_bursts(&[session(1), session(1)], 30, 100);
        assert!(html.contains("within 30 minutes"), "{html}");
        assert!(!html.contains("20 minutes"), "{html}");
    }

    /// The funnel's window spans the whole dawn filter it runs over.
    ///
    /// The filter is hours 4 to 8 *inclusive* — 04:00 to 08:59, five hours —
    /// and the funnel window was 240 minutes, so a morning whose run spanned
    /// the full dawn counted as "in order" in the headline (`sequence_count`,
    /// no gap limit) and fell short in the funnel drawn above it.
    #[test]
    fn the_funnel_window_covers_the_whole_dawn_filter() {
        assert_eq!(
            super::DAWN_FUNNEL_WINDOW_MINUTES,
            (super::DAWN_HOUR_END - super::DAWN_HOUR_START + 1) * 60
        );
    }

    /// The card says the dawn "opens" with the sequence, so it is ordered by
    /// when each voice *starts* on a typical morning, not by the mean time of
    /// all its song.
    ///
    /// The Robin is first up at 04:30 every morning and keeps singing until
    /// after eight; the Wren sings only from 05:00 to 05:20. By mean time the
    /// Wren came first; by who opens the morning, the Robin does.
    #[test]
    fn the_dawn_sequence_is_ordered_by_who_starts_first() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE detections_analytic (Date TEXT, Time TEXT, Com_Name TEXT);",
        )
        .expect("table");
        for day in 1..=5 {
            let date = format!("2026-05-{day:02}");
            let add = |time: &str, sp: &str| {
                conn.execute(
                    "INSERT INTO detections_analytic VALUES (?1, ?2, ?3)",
                    rusqlite::params![date, time, sp],
                )
                .expect("row");
            };
            add("04:30:00", "European Robin");
            for m in ["00", "10", "20", "30"] {
                add(&format!("08:{m}:00"), "European Robin");
            }
            for m in ["00", "05", "10", "20"] {
                add(&format!("05:{m}:00"), "Eurasian Wren");
            }
        }
        assert_eq!(
            super::derive_dawn_sequence(&conn),
            ["European Robin", "Eurasian Wren"]
        );
    }
}
