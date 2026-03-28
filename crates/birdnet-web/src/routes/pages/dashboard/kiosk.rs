//! Kiosk mode: simplified auto-refreshing display for dedicated screens.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Html;

use super::conf_class;
use crate::routes::pages::{escape_html, today_date_string};
use crate::state::AppState;

const KIOSK_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet Kiosk</title>
<style>
  * { margin:0; padding:0; box-sizing:border-box; }
  body { background:#0f172a; color:#e2e8f0; font-family:system-ui,-apple-system,sans-serif; padding:1rem; }
  h1 { font-size:1.5rem; margin-bottom:1rem; text-align:center; color:#89b4fa; }
  .stats { display:flex; gap:1rem; justify-content:center; margin-bottom:1rem; flex-wrap:wrap; }
  .stat { background:#1e293b; border-radius:8px; padding:0.75rem 1.5rem; text-align:center; min-width:120px; }
  .stat .value { font-size:2rem; font-weight:700; color:#89b4fa; }
  .stat .label { font-size:0.75rem; color:#94a3b8; text-transform:uppercase; }
  .recent { max-height:calc(100vh - 12rem); overflow-y:auto; }
  .detection { display:flex; align-items:center; gap:1rem; padding:0.5rem 0; border-bottom:1px solid #1e293b; }
  .detection .name { font-weight:600; font-size:1.1rem; }
  .detection .sci { font-style:italic; color:#94a3b8; font-size:0.85rem; }
  .detection .conf { padding:0.15rem 0.5rem; border-radius:4px; font-size:0.8rem; font-weight:600; }
  .conf.high { background:#166534; color:#4ade80; }
  .conf.mid { background:#854d0e; color:#facc15; }
  .conf.low { background:#991b1b; color:#fca5a5; }
  .detection .time { color:#94a3b8; font-size:0.85rem; margin-left:auto; white-space:nowrap; }
</style>
</head>
<body>
<h1>BirdNet-Behavior</h1>
<div id="kiosk-content"
     hx-get="/pages/kiosk-content"
     hx-trigger="load, every 30s"
     hx-swap="innerHTML">
  <p style="text-align:center;color:#94a3b8;">Loading...</p>
</div>
<script src="/static/htmx.min.js"></script>
</body>
</html>"#;

pub(super) async fn kiosk_page() -> Html<String> {
    Html(KIOSK_HTML.to_string())
}

pub(super) async fn kiosk_content_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let today = today_date_string();
            let total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
            let today_count =
                birdnet_db::sqlite::todays_detection_count(conn, &today, None).unwrap_or(0);
            let species = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
            let recent = birdnet_db::sqlite::recent_detections(conn, 15).unwrap_or_default();
            (total, today_count, species, recent)
        })
    })
    .await;

    match result {
        Ok((total, today_n, species_n, recent)) => {
            let mut html = String::with_capacity(4096);
            let _ = write!(
                html,
                r#"<div class="stats">
  <div class="stat"><div class="value">{today_n}</div><div class="label">Today</div></div>
  <div class="stat"><div class="value">{total}</div><div class="label">Total</div></div>
  <div class="stat"><div class="value">{species_n}</div><div class="label">Species</div></div>
</div>
<div class="recent">"#,
            );

            for d in &recent {
                let conf_pct = d.confidence * 100.0;
                let cls = conf_class(conf_pct);
                let _ = write!(
                    html,
                    r#"<div class="detection">
  <div><div class="name">{com}</div><div class="sci">{sci}</div></div>
  <span class="conf {cls}">{conf_pct:.0}%</span>
  <span class="time">{time} &middot; {date}</span>
</div>"#,
                    com = escape_html(&d.com_name),
                    sci = escape_html(&d.sci_name),
                    time = escape_html(&d.time),
                    date = escape_html(&d.date),
                );
            }

            html.push_str("</div>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading kiosk data</p>".to_string(),
        ),
    }
}
