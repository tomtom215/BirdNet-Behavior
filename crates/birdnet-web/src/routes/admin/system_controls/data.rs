//! Data management: clear detections and extracted recordings.

use axum::extract::State;
use axum::response::Html;

use crate::auth_middleware::RequestUser;
use crate::routes::pages::toast::{self, Toast};
use crate::state::AppState;

pub(super) async fn clear_detections(
    State(state): State<AppState>,
    request_user: RequestUser,
) -> Html<String> {
    // Recorded before the work, not after. This deletes the entire detection
    // history; if the process dies mid-delete there is no "after" to record
    // from, and a station whose history vanished with nothing in the audit log
    // is indistinguishable from one that was never used.
    crate::audit::audit(
        &state,
        Some(&request_user),
        "data.detections.clear",
        None,
        None,
    );
    let state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        // `state.clear_detections`, not a bare `DELETE`: the analytics copy is
        // derived but incremental, so clearing only SQLite left every
        // behavioural and time-series dashboard rendering the whole history
        // beside a dashboard reporting zero detections.
        let det = state.clear_detections().map_err(|e| e.to_string())?;
        let notif = state
            .with_db(|conn| conn.execute("DELETE FROM notification_log", []))
            .map_err(|e| e.to_string())?;
        Ok::<String, String>(format!(
            "Cleared {det} detections and {notif} notification log entries."
        ))
    })
    .await;

    // O-18: toast the destructive outcome.
    match result {
        Ok(Ok(msg)) => toast::with(
            Html(format!(r#"<p class="ctl-ok">{msg}</p>"#)),
            Toast::success(msg),
        ),
        Ok(Err(e)) => {
            let (body, note) = fault("Your detections", &e);
            toast::with(body, note)
        }
        Err(e) => {
            let (body, note) = fault("Your detections", &e);
            toast::with(body, note)
        }
    }
}

/// Report a fault without handing the operator the raw error.
///
/// `Internal error: <io::Error>` and `Failed: <DbError>` were what these
/// controls said, on actions that delete the reader's records. The detail goes
/// to the log; the page says what happened and what state the station is in.
///
/// It deliberately does **not** say "nothing was removed". Clearing detections
/// is two steps — `clear_detections()` and then the notification log — and
/// clearing clips walks a directory file by file, so a failure part way through
/// leaves some of it gone. A comforting sentence that is false for the case the
/// reader is actually in is worse than the raw error it replaced.
///
/// `what` completes "{what} could not be …", so pass the thing in the
/// operator's words ("Your detections").
fn fault<E: std::fmt::Display>(what: &str, err: &E) -> (Html<String>, Toast) {
    tracing::error!(error = %err, "{what}: could not be cleared");
    let msg = format!(
        "{what} could not be fully cleared. Some may already have been removed, \
         so check before trying again. If the station's disk is full, free some \
         space first."
    );
    (
        Html(format!(r#"<p class="ctl-err">{msg}</p>"#)),
        Toast::error(msg),
    )
}

pub(super) async fn clear_extracted(
    State(state): State<AppState>,
    request_user: RequestUser,
) -> Html<String> {
    crate::audit::audit(
        &state,
        Some(&request_user),
        "data.recordings.clear",
        None,
        None,
    );
    let rec_dir = state.recording_dir();

    let result = tokio::task::spawn_blocking(move || {
        if !rec_dir.exists() {
            return Ok::<String, String>("No extracted recordings directory found.".to_string());
        }
        let mut removed = 0u64;
        let mut errors = 0u64;
        if let Ok(entries) = std::fs::read_dir(&rec_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    match std::fs::remove_file(&path) {
                        Ok(()) => removed += 1,
                        Err(_) => errors += 1,
                    }
                } else if path.is_dir() {
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => removed += 1,
                        Err(_) => errors += 1,
                    }
                }
            }
        }
        if errors > 0 {
            Ok(format!("Removed {removed} items ({errors} errors)."))
        } else {
            Ok(format!(
                "Removed {removed} items from recordings directory."
            ))
        }
    })
    .await;

    // O-18: toast the outcome.
    match result {
        Ok(Ok(msg)) => toast::with(
            Html(format!(r#"<p class="ctl-ok">{msg}</p>"#)),
            Toast::success(msg),
        ),
        Ok(Err(e)) => {
            let (body, note) = fault("The saved clips", &e);
            toast::with(body, note)
        }
        Err(e) => {
            let (body, note) = fault("The saved clips", &e);
            toast::with(body, note)
        }
    }
}
