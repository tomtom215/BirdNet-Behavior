//! Health badge and disk status HTMX partials.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/health-badge", get(health_badge_partial))
        .route("/pages/disk-status", get(disk_status_partial))
        .route("/pages/analytics-status", get(analytics_status_partial))
}

async fn health_badge_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::quick_check(conn).unwrap_or(false))
    })
    .await;

    let (dot_class, label) = match result {
        Ok(true) => ("ok", "Healthy"),
        Ok(false) => ("err", "Degraded"),
        Err(_) => ("err", "Error"),
    };

    let html = format!(r#"<span class="dot {dot_class}"></span> {label}"#);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

async fn disk_status_partial(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let db_path = state.db_path().to_path_buf();

    let result = tokio::task::spawn_blocking(move || {
        let dir = db_path.parent().filter(|p| !p.as_os_str().is_empty());
        let dir = dir.unwrap_or_else(|| std::path::Path::new("."));
        birdnet_core::audio::capture::disk_usage(dir)
    })
    .await;

    match result {
        Ok(Ok(usage)) => {
            let pct = usage.used_percent();
            let css_class = if usage.is_critical() {
                "err"
            } else if usage.is_low() {
                "warn"
            } else {
                "ok"
            };

            #[allow(clippy::cast_precision_loss)]
            let avail_gb = usage.available_bytes as f64 / 1_073_741_824.0;

            let html = format!(
                r#"<div class="stat-card">
    <div class="value"><span class="dot {css_class}"></span> {pct:.0}%</div>
    <div class="label">Disk Used ({avail_gb:.1} GB free)</div>
</div>"#,
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<div class="stat-card"><div class="value">--</div><div class="label">Disk Status</div></div>"#.to_string(),
        ),
    }
}

async fn analytics_status_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();

    let (status, css_class) = if configured {
        ("Active", "ok")
    } else if compiled {
        ("Not Configured", "warn")
    } else {
        ("Not Compiled", "err")
    };

    let hint = if configured {
        "DuckDB behavioral analytics are active."
    } else if compiled {
        "Start with <code>--analytics-db</code> to enable."
    } else {
        "Rebuild with <code>--features analytics</code> to enable."
    };

    let html = format!(
        r#"<div class="value"><span class="dot {css_class}"></span> {status}</div>
<div class="label">Analytics Engine</div>
<p style="color:var(--text-muted);font-size:0.8rem;margin-top:0.5rem;">{hint}</p>"#,
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}
