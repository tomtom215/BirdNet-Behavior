//! Permanent redirects from pre-spine routes to their v3-spine homes.
//!
//! "301 every old path to its new home — never 404 a veteran's bookmark"
//! (v3 handoff §04). Every standalone page that folded into a home keeps its
//! old URL working: RSS readers, kiosk bookmarks, BirdNET-Pi muscle memory
//! and deep links from old notifications all land on the equivalent view.
//!
//! Axum's `Redirect::permanent` emits `308 Permanent Redirect` — the
//! method-preserving modern form of 301, matching the pre-existing `/live`
//! redirect's behaviour.
//!
//! `/quarantine` is deliberately **not** here: the handoff keeps the review
//! page reachable for bulk triage (Today's review nudge links to it), so a
//! redirect would orbit it out of existence.

use axum::Router;
use axum::response::Redirect;
use axum::routing::get;

use crate::state::AppState;

/// `(old path, new home)` — the single source of truth for the legacy-route
/// map. Tests iterate this table, so adding a row is automatically covered.
pub const LEGACY_ROUTES: &[(&str, &str)] = &[
    // Today home: the dashboard/today merge — Today IS the home now.
    ("/today", "/"),
    // Patterns home: the six analytics surfaces.
    ("/heatmap", "/patterns"),
    ("/analytics/dawn-chorus", "/patterns?tab=dawn"),
    ("/migration", "/patterns?tab=migration"),
    ("/correlation", "/patterns?tab=together"),
    ("/timeseries", "/patterns?tab=trends"),
    ("/analytics", "/patterns?tab=behavior"),
    // Reports home: the three look-backs.
    ("/weekly", "/reports"),
    ("/year-in-review", "/reports?tab=year"),
    ("/history", "/reports?tab=history"),
    // Station home: the public health dashboard.
    ("/system", "/station"),
];

/// Mount every legacy route as a permanent redirect to its home.
pub fn router() -> Router<AppState> {
    let mut router = Router::new();
    for (old, new) in LEGACY_ROUTES {
        router = router.route(old, get(|| async { Redirect::permanent(new) }));
    }
    router
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use tower::ServiceExt as _;

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    #[tokio::test]
    async fn every_legacy_route_redirects_to_its_home() {
        for (old, new) in LEGACY_ROUTES {
            let app = router().with_state(test_state());
            let res = app
                .oneshot(
                    Request::builder()
                        .uri(*old)
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("router responds");
            assert_eq!(
                res.status(),
                StatusCode::PERMANENT_REDIRECT,
                "{old} should be a permanent redirect"
            );
            let loc = res
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            assert_eq!(loc, *new, "{old} should land on {new}");
        }
    }

    #[test]
    fn legacy_paths_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (old, _) in LEGACY_ROUTES {
            assert!(seen.insert(*old), "duplicate legacy route: {old}");
        }
    }
}
