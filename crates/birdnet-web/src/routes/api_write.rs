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

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::{
    Json, Router,
    routing::{get, post},
};
use birdnet_core::config::redact::{
    REDACTED, is_secret_key, redact_email_local_part, redact_url_credentials,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::routes::admin::settings::form::SETTINGS_FORM_KEYS;
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
    ("PUT", "/api/v2/settings"),
    ("POST", "/api/v2/control/restart"),
];

/// Read endpoints that live behind the same bearer gate.
///
/// `GET /api/v2/settings` is a read, so it is not in [`WRITE_ROUTES`] — the
/// CSRF guard has no interest in a `GET`. It is here rather than in
/// `public_routes()` because a station's settings are not public: the values
/// are redacted (by `redacted_settings`, private to this module, so it is
/// named rather than linked), but the *shape* of a station's configuration is
/// still not something to hand an anonymous visitor.
pub const READ_ROUTES: &[(&str, &str)] = &[("GET", "/api/v2/settings")];

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
        .route("/api/v2/settings", get(read_settings).put(write_settings))
        .route("/api/v2/control/restart", post(restart))
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

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// The station's settings, with every credential masked.
///
/// Applies the project's existing redaction rule rather than a second copy of
/// it: [`is_secret_key`] by key name, then [`redact_url_credentials`] and
/// [`redact_email_local_part`] by value shape. That composition is
/// `support::redacted_config`'s, and it moved into `birdnet-core` so both
/// callers share one definition — two copies of "which values are secret" is
/// the arrangement that once shipped an open `/admin` a diagnostic called
/// protected.
///
/// The value is **replaced, not dropped**, for the reason the support-bundle
/// module records: "this station has an SMTP password set" is information, and
/// an absent key reads identically to one that was never configured.
///
/// `apprise_url`, `notify_urls` and `heartbeat_url` are the interesting cases.
/// None of their *names* looks like a secret, and all three routinely carry one
/// in the value — `ntfy://user:pass@host`, and a heartbeat URL whose path
/// segment *is* the credential (`NT-16`). The by-shape half catches the first
/// two; the third is a bare token in a path and is caught by neither, which is
/// stated here rather than left for a reader to discover.
fn redacted_settings(raw: &std::collections::HashMap<String, String>) -> BTreeMap<String, String> {
    raw.iter()
        .map(|(k, v)| {
            let shown = if is_secret_key(k) {
                REDACTED.to_owned()
            } else {
                redact_email_local_part(&redact_url_credentials(v))
            };
            (k.clone(), shown)
        })
        .collect()
}

async fn read_settings(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    let raw = crate::routes::admin::settings::handler::load_all_settings(&state);
    let redacted = redacted_settings(&raw);
    let masked: Vec<&String> = redacted
        .iter()
        .filter(|(_, v)| v.as_str() == REDACTED)
        .map(|(k, _)| k)
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "settings": redacted,
            // Named so a caller can tell "this station has no SMTP password"
            // from "you are not allowed to read it".
            "redacted": masked,
            "writable_keys": SETTINGS_FORM_KEYS,
        })),
    )
}

/// Coerce one JSON scalar to the string the settings table stores.
///
/// Numbers and booleans are accepted because a JSON client will naturally send
/// `{"latitude": 51.5}` or `{"night_inhibit": true}`, and refusing those would
/// be a papercut with no safety value — every settings value is a string in the
/// database either way. Arrays, objects and `null` are refused: none of them
/// has an obvious string form, and guessing one would store something the
/// caller did not write.
fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

async fn write_settings(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let Some(object) = body.as_object() else {
        return bad_request("the body must be a JSON object of setting keys to values");
    };

    // Unknown keys are refused rather than ignored. A caller who misspells
    // `confidence_treshold` and gets a 200 has been told their change landed.
    let unknown: Vec<&String> = object
        .keys()
        .filter(|k| !SETTINGS_FORM_KEYS.contains(&k.as_str()))
        .collect();
    if !unknown.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "unknown setting keys",
                "unknown": unknown,
                "writable_keys": SETTINGS_FORM_KEYS,
            })),
        );
    }

    // The round-trip trap: `GET` returns `***REDACTED***` in place of every
    // secret, so a client that reads the whole object, edits one field and
    // writes it back would overwrite real credentials with the placeholder.
    // Refusing is the honest answer — silently skipping would mean "I set it
    // and nothing happened".
    let placeholders: Vec<&String> = object
        .iter()
        .filter(|(_, v)| v.as_str() == Some(REDACTED))
        .map(|(k, _)| k)
        .collect();
    if !placeholders.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "refusing to store the redaction placeholder over a real value; \
                          send only the keys you mean to change",
                "keys": placeholders,
            })),
        );
    }

    let mut strings = serde_json::Map::new();
    for (k, v) in object {
        let Some(s) = scalar_to_string(v) else {
            return bad_request(&format!(
                "{k} must be a string, number or boolean, not {}",
                match v {
                    Value::Null => "null",
                    Value::Array(_) => "an array",
                    _ => "an object",
                }
            ));
        };
        strings.insert(k.clone(), Value::String(s));
    }

    // Deserialise into the same form type the settings page posts, so the
    // normalisation, the category assignment and the only-write-what-changed
    // rule are the page's and not a second implementation of them.
    let Ok(form) = serde_json::from_value::<crate::routes::admin::settings::form::SettingsForm>(
        Value::Object(strings),
    ) else {
        return bad_request("the body could not be read as a settings payload");
    };

    let existing = crate::routes::admin::settings::handler::load_all_settings(&state);
    let items = crate::routes::admin::settings::handler::build_settings_items(&form, &existing);
    if items.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({ "updated": 0, "keys": [], "note": "every value already matched" })),
        );
    }
    let keys: Vec<&str> = items.iter().map(|(k, _, _)| *k).collect();

    let written = state.with_db(|conn| {
        birdnet_db::settings::ensure_settings_table(conn)?;
        let refs: Vec<(&str, &str, birdnet_db::settings::SettingsCategory)> =
            items.iter().map(|(k, v, c)| (*k, v.as_str(), *c)).collect();
        birdnet_db::settings::set_many(conn, &refs)?;
        Ok::<usize, birdnet_db::settings::SettingsError>(refs.len())
    });

    match written {
        Ok(n) => {
            // Names only, never values: a metadata field carrying
            // `birdweather_token=…` would put a credential in a table
            // `/admin/audit` renders.
            crate::audit::audit(
                &state,
                None,
                "settings.update",
                None,
                Some(&format!("{VIA_API} keys={}", keys.join(","))),
            );
            (StatusCode::OK, Json(json!({ "updated": n, "keys": keys })))
        }
        Err(e) => {
            tracing::error!(error = %e, "settings write from the API failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "the database refused the change" })),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Control
// ---------------------------------------------------------------------------

/// Restart the station.
///
/// Shares [`crate::routes::admin::system_controls::service::request_restart`]
/// with the admin page's button, so the systemd detection and the delayed
/// self-SIGTERM have one implementation. `503` rather than `200` when there is
/// no systemd to bring the process back: a caller that got a cheerful 200 and
/// then found the station gone would have been told the opposite of what
/// happened.
#[allow(clippy::unused_async)] // async required by axum's Handler trait
async fn restart(State(state): State<AppState>) -> (StatusCode, Json<Value>) {
    use crate::routes::admin::system_controls::service::{RestartOutcome, request_restart};

    // Before the decision, so the record exists even where the restart is
    // refused — and before the SIGTERM, so it survives the restart.
    crate::audit::audit(&state, None, "system.restart", None, Some(VIA_API));

    match request_restart() {
        RestartOutcome::Signalled => (
            StatusCode::OK,
            Json(json!({
                "restarting": true,
                "note": "SIGTERM sent; systemd Restart=always brings a fresh instance up"
            })),
        ),
        RestartOutcome::NotUnderSystemd => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "restarting": false,
                "error": "not running under systemd, so nothing would restart this process"
            })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        READ_ROUTES, REDACTED, WRITE_ROUTES, is_valid_date, is_valid_time, is_write_route,
        redacted_settings, scalar_to_string,
    };

    #[test]
    fn the_route_table_is_well_formed() {
        // What this checks is the *table*: every entry is a method this module
        // could mount, on a path under the API prefix, recognised by
        // `is_write_route`.
        //
        // It does not check the router — `axum::Router` exposes no route list
        // to assert against, and a first version of this test named itself
        // `the_route_table_is_the_router` while passing happily with
        // `.put(write_settings)` deleted from `router()`. That half is
        // `every_documented_route_is_mounted` in
        // `tests/the_api_can_change_the_station.rs`, which has a real
        // `AppState` and can send the request.
        assert!(!WRITE_ROUTES.is_empty());
        for (method, path) in WRITE_ROUTES {
            assert!(
                matches!(*method, "POST" | "PUT"),
                "{path} is documented with {method}, a method the router does not mount"
            );
            assert!(path.starts_with("/api/v2/"), "{path} is not under /api/v2");
            assert!(
                is_write_route(path),
                "{path} is not recognised by is_write_route"
            );
        }
    }

    #[test]
    fn a_read_route_is_not_a_write_route() {
        // `GET /api/v2/settings` shares a path with `PUT /api/v2/settings`, and
        // the CSRF guard keys on the path alone. That is safe only because a
        // `GET` never reaches the guard's mutating branch; what would not be
        // safe is `READ_ROUTES` growing a mutating method, so say so here.
        assert!(!READ_ROUTES.is_empty());
        for (method, path) in READ_ROUTES {
            assert_eq!(
                *method, "GET",
                "{path} is listed as a read but documented with {method}"
            );
            assert!(path.starts_with("/api/v2/"), "{path} is not under /api/v2");
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

    #[test]
    fn a_scalar_becomes_the_string_the_settings_table_stores() {
        assert_eq!(
            scalar_to_string(&serde_json::json!("0.7")).as_deref(),
            Some("0.7")
        );
        assert_eq!(
            scalar_to_string(&serde_json::json!(0.7)).as_deref(),
            Some("0.7")
        );
        assert_eq!(
            scalar_to_string(&serde_json::json!(4)).as_deref(),
            Some("4")
        );
        assert_eq!(
            scalar_to_string(&serde_json::json!(true)).as_deref(),
            Some("true")
        );
        // Refused, because none of these has a string form a caller would
        // recognise as the value they sent.
        assert!(scalar_to_string(&serde_json::json!(null)).is_none());
        assert!(scalar_to_string(&serde_json::json!([1, 2])).is_none());
        assert!(scalar_to_string(&serde_json::json!({"a": 1})).is_none());
    }

    #[test]
    fn settings_are_redacted_by_key_and_by_shape() {
        let raw: HashMap<String, String> = [
            // By key name.
            ("email_smtp_pass", "hunter2"),
            ("birdweather_token", "bw-live-abcdef"),
            // By value shape: nothing about `apprise_url` says "secret".
            ("apprise_url", "ntfy://alice:hunter2@ntfy.example/topic"),
            // Left alone.
            ("confidence_threshold", "0.7"),
            ("site_name", "Back Garden"),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();

        let out = redacted_settings(&raw);

        assert_eq!(out["email_smtp_pass"], REDACTED);
        assert_eq!(out["birdweather_token"], REDACTED);

        // The exact output, not just "the password is gone", because the two
        // by-shape rules compose in a way neither one describes on its own.
        // `redact_url_credentials` alone yields
        // `ntfy://alice:***REDACTED***@ntfy.example/topic`; running
        // `redact_email_local_part` over *that* sees `ntfy://alice:…` as an
        // address local part and replaces the whole thing, scheme included.
        // So it is the email rule, not the URL rule, that removes the password
        // here — and a first version of this test asserted only "does not
        // contain hunter2" and "does contain ntfy.example", both of which were
        // true for a reason it had not established. This is the support
        // bundle's composition unchanged (`support::redacted_env`); pinning it
        // means a change to either rule shows up here rather than silently
        // altering what a station discloses.
        assert_eq!(out["apprise_url"], "***@ntfy.example/topic");
        assert!(
            !out["apprise_url"].contains("hunter2"),
            "a credential inside a URL value survived: {}",
            out["apprise_url"]
        );

        // The counterpart: a blanket `REDACTED` for everything would satisfy
        // the assertions above and make the endpoint useless.
        assert_eq!(out["confidence_threshold"], "0.7");
        assert_eq!(out["site_name"], "Back Garden");

        // Every key survives, so a caller can tell "not set" from "not shown".
        assert_eq!(out.len(), raw.len());
    }
}
