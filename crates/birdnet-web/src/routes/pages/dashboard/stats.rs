//! Dashboard stat-row HTMX partials.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};

use crate::routes::pages::atoms::sparkline;
use crate::routes::pages::{escape_html, group_thousands, today_count, today_date_string};
use crate::state::AppState;

/// Distinct species seen today, rejected detections excluded.
///
/// Was an inlined `FROM detections` query. Every other number on this tile row
/// reads `detections_analytic`, so this one contradicted its own neighbours by
/// exactly the number of species whose only detections that day had been
/// rejected.
fn species_today(
    conn: &rusqlite::Connection,
    today: &str,
) -> Result<i64, birdnet_db::sqlite::DbError> {
    birdnet_db::sqlite::analytic_species_count_for_date(conn, today)
}

/// HTMX partial: the four headline stat tiles (Detections / Species / Today / Last hour).
pub(super) async fn stats_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    // Every tile below is a count of the reader's own birds, so every one of
    // them propagates. They were all `.unwrap_or(0)` *inside* the closure,
    // which returned a plain tuple — so the `else` arm beneath could only ever
    // fire on a task panic, and a database that could not be read rendered
    // "Detections 0 · Species 0 · Today 0 · Last hour 0" as an ordinary
    // dashboard. This is the first screen anyone opens.
    //
    // `daily` stays defaulted: it draws the 12-day sparkline, which is
    // decoration beside the numbers rather than a claim of its own.
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today = today_date_string();
            let total = birdnet_db::sqlite::analytic_detection_count(conn)?;
            let species = birdnet_db::sqlite::species_count(conn)?;
            let today_n = today_count(conn)?;
            let last_hour = birdnet_db::sqlite::last_hour_count(conn)?;
            let species_today_n = species_today(conn, &today)?;
            let daily = birdnet_db::sqlite::daily_counts(conn, 12).unwrap_or_default();
            Ok::<_, birdnet_db::sqlite::DbError>((
                total,
                species,
                today_n,
                last_hour,
                species_today_n,
                daily,
            ))
        })
    })
    .await;

    let (total, species, today_n, last_hour, species_today_n, daily) = match result {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "dashboard stats: query failed");
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                crate::routes::pages::error_states::inline("today's totals"),
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "dashboard stats: task failed");
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                crate::routes::pages::error_states::inline("today's totals"),
            );
        }
    };

    // Oldest → newest daily counts for the sparkline.
    let mut trend: Vec<i64> = daily.iter().rev().map(|d| d.count).collect();
    if trend.is_empty() {
        trend = vec![0];
    }
    let spark = sparkline(&trend, 200.0, 26.0, None);

    let mut html = String::new();
    // Tile 1 — Detections (all-time) with 12-day sparkline.
    let _ = write!(
        html,
        r#"<div class="stat-tile"><span class="label">Detections</span>
             <div><div class="value tabular">{total}</div><div class="sub">all time</div></div>
             <div class="ds-spark">{spark}</div></div>"#,
        total = group_thousands(total),
    );
    // Tile 2 — Species (all-time unique).
    let _ = write!(
        html,
        r#"<div class="stat-tile"><span class="label">Species</span>
             <div><div class="value tabular">{species}</div><div class="sub">{species_today_n} active today</div></div></div>"#,
        species = group_thousands(species),
        species_today_n = group_thousands(species_today_n),
    );
    // Tile 3 — Today.
    let _ = write!(
        html,
        r#"<div class="stat-tile"><span class="label">Today</span>
             <div><div class="value tabular">{today_n}</div><div class="sub">{date}</div></div></div>"#,
        today_n = group_thousands(today_n),
        date = escape_html(&today_date_string()),
    );
    // Tile 4 — Last hour (dawn accent for the live-ish number).
    let _ = write!(
        html,
        r#"<div class="stat-tile"><span class="label ds-label-live">Last hour <span class="bnb-dot live"></span></span>
             <div><div class="value tabular ds-last-hour">{last_hour}</div><div class="sub">rolling 60 min</div></div></div>"#,
        last_hour = group_thousands(last_hour),
    );

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: the hero "hearing N species today" pill body.
pub(super) async fn hero_status_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| species_today(conn, &today_date_string()))
    })
    .await;
    // "hearing 0 species today" is a sentence about the reader's garden. If
    // the count could not be read, the pill says nothing rather than saying
    // that.
    let body = match result {
        Ok(Ok(n)) => format!("hearing {n} species today"),
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "hero status: species count failed");
            "species count unavailable".to_string()
        }
        Err(e) => {
            tracing::warn!(error = %e, "hero status: task failed");
            "species count unavailable".to_string()
        }
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}
