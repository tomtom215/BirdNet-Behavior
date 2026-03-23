//! Data management: clear detections and extracted recordings.

use axum::extract::State;
use axum::response::Html;

use crate::state::AppState;

pub(super) async fn clear_detections(State(state): State<AppState>) -> Html<String> {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let det = conn.execute("DELETE FROM detections", []);
            let notif = conn.execute("DELETE FROM notification_log", []);
            match (det, notif) {
                (Ok(d), Ok(n)) => Ok(format!(
                    "Cleared {d} detections and {n} notification log entries."
                )),
                (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            }
        })
    })
    .await;

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p style="color:#4ade80;">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(
            r#"<p style="color:#f87171;">Failed to clear data: {e}</p>"#
        )),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}

pub(super) async fn clear_extracted(State(state): State<AppState>) -> Html<String> {
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

    match result {
        Ok(Ok(msg)) => Html(format!(r#"<p style="color:#4ade80;">{msg}</p>"#)),
        Ok(Err(e)) => Html(format!(r#"<p style="color:#f87171;">Failed: {e}</p>"#)),
        Err(e) => Html(format!(
            r#"<p style="color:#f87171;">Internal error: {e}</p>"#
        )),
    }
}
