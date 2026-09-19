//! The species-accumulation curve partial.
//!
//! The pre-spine `/life-list` page folded into the Species home's **Life list**
//! view ([`super::species_pages`]); `/life-list` now permanently redirects there
//! (see [`crate::routes::redirects`]). This module keeps the one HTMX partial
//! that other surfaces still embed — the cumulative life-list growth curve, used
//! by the Patterns **Trends** tab (`templates/timeseries.html`).
//!
//! | Path                       | Purpose                                  |
//! |----------------------------|------------------------------------------|
//! | `GET /pages/life-accumulation` | Cumulative species-accumulation curve |

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use crate::state::AppState;

/// Mount the accumulation-curve partial.
pub fn router() -> Router<AppState> {
    Router::new().route("/pages/life-accumulation", get(life_accumulation_partial))
}

/// HTMX partial: cumulative species-accumulation curve (life-list growth).
async fn life_accumulation_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_read_db(|conn| {
            // `?`, not `.unwrap_or_default()`. This is the only query the
            // partial makes, so defaulting it turned a database that could not
            // be read into an empty map, an empty curve, and
            // `accumulation_curve`'s "Not enough data yet for this view." —
            // a failure rendered as a statement about the reader's birds, and
            // to the person least able to shrug it off: a life list is the one
            // record nobody wants to be told is empty.
            let first_seen = birdnet_db::sqlite::species_first_seen(conn)?;
            let mut monthly: std::collections::BTreeMap<String, u32> =
                std::collections::BTreeMap::new();
            for date in first_seen.values() {
                // `get(..7)` rather than `date[..7]`: a multibyte char straddling
                // byte 7 would make the slice panic, and `panic = "abort"` turns
                // that into a process crash. The `YYYY-MM` prefix is the key.
                if let Some(month) = date.get(..7) {
                    *monthly.entry(month.to_string()).or_default() += 1;
                }
            }
            Ok::<_, birdnet_db::sqlite::DbError>(monthly)
        })
    })
    .await;

    // See `error_states::failed_partial` for why this is a 200.
    let monthly = match result {
        Ok(Ok(monthly)) => monthly,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "life accumulation: query failed");
            return super::error_states::failed_partial("your life list's growth over time");
        }
        Err(e) => {
            tracing::warn!(error = %e, "life accumulation: task failed");
            return super::error_states::failed_partial("your life list's growth over time");
        }
    };
    let mut cum: i64 = 0;
    let points: Vec<(String, i64)> = monthly
        .iter()
        .map(|(month, &c)| {
            cum += i64::from(c);
            (month.get(2..).unwrap_or(month).to_string(), cum)
        })
        .collect();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        super::viz::accumulation_curve(&points),
    )
}
