//! Dashboard stat-row HTMX partials.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};

use crate::routes::pages::atoms::sparkline;
use crate::routes::pages::{escape_html, group_thousands, today_count, today_date_string};
use crate::state::AppState;

/// Distinct species seen today (helper inlined — no dedicated DB fn exists).
fn species_today(conn: &rusqlite::Connection, today: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(DISTINCT Com_Name) FROM detections WHERE Date = ?1",
        [today],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// HTMX partial: the four headline stat tiles (Detections / Species / Today / Last hour).
pub(super) async fn stats_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today = today_date_string();
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let today_n = today_count(conn);
            let last_hour = birdnet_db::sqlite::last_hour_count(conn).unwrap_or(0);
            let species_today_n = species_today(conn, &today);
            let daily = birdnet_db::sqlite::daily_counts(conn, 12).unwrap_or_default();
            (total, species, today_n, last_hour, species_today_n, daily)
        })
    })
    .await;

    let Ok((total, species, today_n, last_hour, species_today_n, daily)) = result else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading stats</p>".to_string(),
        );
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
    let n = result.unwrap_or(0);
    let body = format!("hearing {n} species today");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], body)
}
