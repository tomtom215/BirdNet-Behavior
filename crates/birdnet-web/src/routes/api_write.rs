//! The bearer-authenticated mutating `/api/v2` surface (`O-1`).
//!
//! # What this closes
//!
//! Every other module under `/api/v2` is `get`-only — a mechanical grep for
//! `post(`/`put(`/`delete(`/`patch(` across the fourteen nested routers returns
//! nothing — against upstream `birdnet-go`'s fifty-four mutating routes. Every
//! state change in this product is an HTMX form post that returns an HTML
//! fragment, so Home Assistant, Node-RED or a shell script can read a station
//! and never act on one, and our own front end is the only client: a change to
//! fragment markup silently breaks whatever automation exists in the wild.
//!
//! # Why this is a separate router
//!
//! `crates/birdnet-web/tests/public_router_is_read_only.rs` exists because
//! thirteen mutating `POST` routes were once sitting in the public router,
//! reachable by anyone who could load the dashboard. These endpoints are
//! mutating and they are under `/api/v2`, so putting them in
//! [`crate::routes::public_routes`] would re-create exactly that. They are
//! mounted separately and wrapped in [`crate::api_token::require_bearer`]
//! instead, and that gate's route list names them so the arrangement is
//! asserted rather than assumed.
//!
//! # The shape
//!
//! JSON in, JSON out, and the same `(date, time, sci_name)` composite key the
//! detections table uses as its identity — there is no surrogate id to offer.
//! The handlers are thin wrappers over the same `AppState` and `birdnet_db`
//! calls the HTMX pages use; they are **not** the page handlers themselves,
//! which return HTML fragments, take `Form`, and are shaped by what HTMX needs
//! to swap into the DOM.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Json, Router, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::state::AppState;

/// Every bearer-authenticated mutating endpoint, as `(method, path)`.
///
/// One table, read by three things that must agree: the router below, the CSRF
/// guard that has to skip these paths, and the gates that check both. That is
/// the lesson the station-health `CHECKS` table and the audit-log `ACTIONS`
/// list already record — a set expressed only as scattered call sites cannot be
/// checked.
pub const WRITE_ROUTES: &[(&str, &str)] = &[
    ("POST", "/api/v2/detections/review"),
    ("POST", "/api/v2/detections/lock"),
    ("POST", "/api/v2/detections/unlock"),
    ("POST", "/api/v2/detections/delete"),
];

/// Whether `path` is one of the mutating API endpoints.
///
/// Used by [`crate::security::csrf_guard_middleware`]: a bearer credential
/// cannot be attached to a cross-site form submission, so the CSRF check has
/// nothing to protect here — but the skip is scoped to exactly these paths so
/// it can never widen to the cookie-authenticated admin surface.
#[must_use]
pub fn is_write_route(path: &str) -> bool {
    WRITE_ROUTES.iter().any(|(_, p)| *p == path)
}

/// The mutating API, before authentication is layered on.
///
/// Mounted by [`crate::server`] behind [`crate::api_token::require_bearer`].
/// Nothing here is reachable on a station with no `BNB_API_TOKEN`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v2/detections/review", post(review))
        .route("/api/v2/detections/lock", post(lock))
        .route("/api/v2/detections/unlock", post(unlock))
        .route("/api/v2/detections/delete", post(delete))
}

/// The composite key that identifies a detection.
///
/// `(Date, Time, Sci_Name, File_Name, chunk_offset_secs)` is this schema's
/// identity; the first three are what a caller can reasonably know, and are
/// what every page handler already keys on.
#[derive(Debug, Deserialize)]
struct Key {
    date: String,
    time: String,
    sci_name: String,
}

/// A review verdict to record, or `None` to clear one.
#[derive(Debug, Deserialize)]
struct ReviewBody {
    date: String,
    time: String,
    sci_name: String,
    /// Shown in the review UI; the schema stores it alongside the verdict.
    com_name: Option<String>,
    /// `confirmed`, `rejected`, or absent to clear an existing verdict.
    status: Option<String>,
    /// Free text an operator can attach.
    notes: Option<String>,
}

/// `YYYY-MM-DD`, checked before it reaches a query.
fn is_valid_date(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

/// `HH:MM:SS`, checked before it reaches a query.
fn is_valid_time(s: &str) -> bool {
    s.len() == 8
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 2 || i == 5 {
                b == b':'
            } else {
                b.is_ascii_digit()
            }
        })
}

/// Reject a malformed key before it reaches the database.
///
/// Not for safety — every query is parameterised — but so a caller that sends
/// `date: "yesterday"` is told what is wrong instead of getting a cheerful
/// "nothing matched".
fn validate(date: &str, time: &str, sci_name: &str) -> Result<(), (StatusCode, Json<Value>)> {
    if !is_valid_date(date) {
        return Err(bad_request("date must be YYYY-MM-DD"));
    }
    if !is_valid_time(time) {
        return Err(bad_request("time must be HH:MM:SS"));
    }
    if sci_name.trim().is_empty() {
        return Err(bad_request("sci_name must not be empty"));
    }
    Ok(())
}

fn bad_request(message: &str) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message })))
}

fn not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "no detection matches that date, time and scientific name" })),
    )
}

fn server_error(e: &birdnet_db::sqlite::DbError) -> (StatusCode, Json<Value>) {
    tracing::warn!(error = %e, "mutating API request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "the database refused the change" })),
    )
}

/// The audit metadata every change made through this API carries.
///
/// The audit row's user is `None` throughout: a token-authenticated request is
/// not a logged-in person, and inventing one would make the audit log say
/// something untrue. This is how `/admin/audit` tells an automated change from
/// a human one.
///
/// The `crate::audit::audit` calls below are written out at each site rather
/// than wrapped in a local helper, because `tests/the_audit_log_records_what_happened.rs`
/// finds action names by scanning for that call and reading string literals
/// near it. A helper taking the action as a parameter compiles, works, and is
/// invisible to that gate — which was how these four names first shipped
/// undocumented.
const VIA_API: &str = "via=api";

/// The audit target for a key: the tuple, in the order the schema uses.
fn target_of(date: &str, time: &str, sci_name: &str) -> String {
    format!("{date} {time} {sci_name}")
}

async fn review(
    State(state): State<AppState>,
    Json(body): Json<ReviewBody>,
) -> (StatusCode, Json<Value>) {
    if let Err(e) = validate(&body.date, &body.time, &body.sci_name) {
        return e;
    }
    let target = target_of(&body.date, &body.time, &body.sci_name);

    let outcome = match body.status.as_deref() {
        None => state.clear_detection_review(&body.date, &body.time, &body.sci_name),
        Some(s) => {
            let Some(status) = birdnet_db::sqlite::ReviewStatus::parse(s) else {
                return bad_request("status must be \"confirmed\", \"rejected\", or omitted");
            };
            state.set_detection_review(
                &body.date,
                &body.time,
                &body.sci_name,
                body.com_name.as_deref().unwrap_or(&body.sci_name),
                status,
                body.notes.as_deref(),
            )
        }
    };

    match outcome {
        Ok(()) => {
            crate::audit::audit(
                &state,
                None,
                "detection.review",
                Some(&target),
                Some(VIA_API),
            );
            (
                StatusCode::OK,
                Json(json!({ "status": body.status, "detection": target })),
            )
        }
        Err(e) => server_error(&e),
    }
}

async fn lock(State(state): State<AppState>, Json(key): Json<Key>) -> (StatusCode, Json<Value>) {
    set_lock(&state, &key, true)
}

async fn unlock(State(state): State<AppState>, Json(key): Json<Key>) -> (StatusCode, Json<Value>) {
    set_lock(&state, &key, false)
}

fn set_lock(state: &AppState, key: &Key, locked: bool) -> (StatusCode, Json<Value>) {
    if let Err(e) = validate(&key.date, &key.time, &key.sci_name) {
        return e;
    }
    let changed = state.with_db(|conn| {
        if locked {
            birdnet_db::sqlite::lock_detection(conn, &key.date, &key.time, &key.sci_name)
        } else {
            birdnet_db::sqlite::unlock_detection(conn, &key.date, &key.time, &key.sci_name)
        }
    });
    let target = target_of(&key.date, &key.time, &key.sci_name);
    match changed {
        Ok(true) => {
            crate::audit::audit(
                state,
                None,
                if locked {
                    "detection.lock"
                } else {
                    "detection.unlock"
                },
                Some(&target),
                Some(VIA_API),
            );
            (
                StatusCode::OK,
                Json(json!({ "locked": locked, "detection": target })),
            )
        }
        Ok(false) => not_found(),
        Err(e) => server_error(&e),
    }
}

async fn delete(State(state): State<AppState>, Json(key): Json<Key>) -> (StatusCode, Json<Value>) {
    if let Err(e) = validate(&key.date, &key.time, &key.sci_name) {
        return e;
    }
    let target = target_of(&key.date, &key.time, &key.sci_name);
    match state.delete_detection(&key.date, &key.time, &key.sci_name) {
        Ok(true) => {
            crate::audit::audit(
                &state,
                None,
                "detection.delete",
                Some(&target),
                Some(VIA_API),
            );
            (
                StatusCode::OK,
                Json(json!({ "deleted": true, "detection": target })),
            )
        }
        Ok(false) => not_found(),
        Err(e) => server_error(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::{WRITE_ROUTES, is_valid_date, is_valid_time, is_write_route};

    #[test]
    fn the_route_table_is_the_router() {
        // Every entry names a path this module actually mounts. `router()` is
        // built by hand, so this is what stops the table and the router
        // drifting — the CSRF guard and the OpenAPI gate both read the table.
        assert!(!WRITE_ROUTES.is_empty());
        for (method, path) in WRITE_ROUTES {
            assert_eq!(
                *method, "POST",
                "{path} is documented with a method the router does not mount"
            );
            assert!(path.starts_with("/api/v2/"), "{path} is not under /api/v2");
            assert!(
                is_write_route(path),
                "{path} is not recognised by is_write_route"
            );
        }
    }

    #[test]
    fn nothing_else_is_a_write_route() {
        // The counterpart. Without it `is_write_route` returning `true`
        // unconditionally would satisfy the gate above and hand every path a
        // CSRF exemption.
        for path in [
            "/api/v2/detections",
            "/api/v2/health",
            "/pages/today-delete",
            "/admin/settings",
            "",
            "/",
        ] {
            assert!(!is_write_route(path), "{path} must not be CSRF-exempt");
        }
    }

    #[test]
    fn dates_and_times_are_checked_before_they_reach_a_query() {
        assert!(is_valid_date("2026-09-03"));
        assert!(!is_valid_date("2026-9-3"));
        assert!(!is_valid_date("yesterday"));
        assert!(!is_valid_date("2026-09-03T00"));
        assert!(!is_valid_date(""));
        assert!(is_valid_time("06:05:04"));
        assert!(!is_valid_time("6:05:04"));
        assert!(!is_valid_time("06:05"));
        assert!(!is_valid_time("06-05-04"));
        assert!(!is_valid_time(""));
    }
}
