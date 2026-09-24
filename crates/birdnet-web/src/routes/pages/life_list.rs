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
    let this_month = super::today_date_string()
        .get(..7)
        .unwrap_or_default()
        .to_string();
    let points = accumulation_points(&monthly, &this_month);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html")],
        super::viz::accumulation_curve(&points),
    )
}

/// The cumulative curve's points: every month from the first species through
/// `this_month` (`YYYY-MM`), labelled `YY-MM`.
///
/// Every month, not only those that added a species (ANA14a): a plateau is
/// part of the record, and without it the x-axis is not time. Through this
/// month, so the curve ends at the present rather than at the last addition.
fn accumulation_points(
    monthly: &std::collections::BTreeMap<String, u32>,
    this_month: &str,
) -> Vec<(String, i64)> {
    let parse = |m: &str| -> Option<(i32, u32)> {
        let (y, mo) = m.split_once('-')?;
        Some((y.parse().ok()?, mo.parse().ok()?))
    };
    let (Some(first), Some(last)) = (
        monthly.keys().next().and_then(|m| parse(m)),
        monthly.keys().next_back().and_then(|m| parse(m)),
    ) else {
        return Vec::new();
    };
    let end = parse(this_month).map_or(last, |now| now.max(last));
    let mut out = Vec::new();
    let mut cum: i64 = 0;
    let (mut y, mut mo) = first;
    while (y, mo) <= end {
        let key = format!("{y:04}-{mo:02}");
        cum += i64::from(monthly.get(&key).copied().unwrap_or(0));
        out.push((key.get(2..).unwrap_or(&key).to_string(), cum));
        (y, mo) = if mo == 12 { (y + 1, 1) } else { (y, mo + 1) };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::accumulation_points;

    /// `ANA14a`: a month with no new species is a flat step, not a missing one.
    ///
    /// The curve had one point per month that *added* a species, so January
    /// then June drew as two adjacent points — five months of plateau erased,
    /// and the x-axis no longer time. It also stopped at the last addition
    /// rather than at the present.
    #[test]
    fn the_curve_has_every_month_through_this_one() {
        let monthly = [("2026-01".to_string(), 3), ("2026-04".to_string(), 2)]
            .into_iter()
            .collect();
        let points = accumulation_points(&monthly, "2026-06");
        let expected: Vec<(String, i64)> = [
            ("26-01", 3),
            ("26-02", 3),
            ("26-03", 3),
            ("26-04", 5),
            ("26-05", 5),
            ("26-06", 5),
        ]
        .into_iter()
        .map(|(m, n)| (m.to_string(), n))
        .collect();
        assert_eq!(points, expected);
    }

    /// Counterparts: across a year boundary, and with nothing to draw.
    #[test]
    fn the_curve_crosses_a_year_and_is_empty_without_data() {
        let monthly = [("2025-11".to_string(), 1), ("2026-01".to_string(), 1)]
            .into_iter()
            .collect();
        let months: Vec<String> = accumulation_points(&monthly, "2026-01")
            .into_iter()
            .map(|(m, _)| m)
            .collect();
        assert_eq!(months, ["25-11", "25-12", "26-01"]);
        assert!(accumulation_points(&std::collections::BTreeMap::new(), "2026-01").is_empty());
    }
}
