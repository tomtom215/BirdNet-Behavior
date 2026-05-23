//! Kiosk mode: simplified auto-refreshing display for dedicated screens.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::Html;

use super::conf_class;
use crate::routes::pages::{escape_html, group_thousands, today_date_string};
use crate::state::AppState;

const KIOSK_HTML: &str = r#"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>BirdNet-Behavior · Kiosk</title>
<link rel="stylesheet" href="/static/css/app.css">
<style>
  body { padding:4vh 5vw; overflow:hidden; }
  .kiosk-head { display:flex; align-items:center; justify-content:center; gap:12px; margin-bottom:4vh; }
  .kiosk-head .title { font-family:var(--font-display); font-size:clamp(28px,4vw,52px); letter-spacing:-0.02em; }
  .stats { display:flex; gap:24px; justify-content:center; margin-bottom:4vh; flex-wrap:wrap; }
  .stat { background:var(--surface); border:0.5px solid var(--border); border-radius:var(--r-lg); padding:20px 36px; text-align:center; min-width:170px; box-shadow:var(--shadow-md); }
  .stat .value { font-family:var(--font-display); font-variant-numeric:tabular-nums; font-size:clamp(34px,5vw,64px); line-height:1; color:var(--moss); }
  .stat .label { font-size:11px; letter-spacing:0.1em; text-transform:uppercase; color:var(--fg-3); margin-top:10px; }
  .recent { max-width:1100px; margin:0 auto; max-height:calc(100vh - 34vh); overflow-y:auto; }
  .detection { display:flex; align-items:center; gap:18px; padding:14px 2px; border-bottom:0.5px solid var(--hairline); }
  .detection .name { font-weight:500; font-size:clamp(16px,1.6vw,22px); }
  .detection .sci { font-style:italic; color:var(--fg-3); font-size:13px; font-family:var(--font-mono); }
  .detection .time { color:var(--fg-3); font-size:13px; margin-left:auto; white-space:nowrap; font-family:var(--font-mono); }
</style>
</head>
<body>
<div class="kiosk-head">
  <svg width="32" height="32" viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="11" fill="none" stroke="currentColor" stroke-width="0.8" style="color:var(--fg)"></circle>
    <g stroke="currentColor" stroke-width="1.4" stroke-linecap="round" style="color:var(--fg)">
      <line x1="6" y1="12" x2="6" y2="12"></line><line x1="9" y1="9.5" x2="9" y2="14.5"></line>
      <line x1="12" y1="6" x2="12" y2="18"></line><line x1="15" y1="8" x2="15" y2="16"></line>
      <line x1="18" y1="10.5" x2="18" y2="13.5"></line>
    </g>
  </svg>
  <span class="title">BirdNet<span style="color:var(--fg-3)">Behavior</span></span>
</div>
<div id="kiosk-content"
     hx-get="/pages/kiosk-content"
     hx-trigger="load, every 30s"
     hx-swap="innerHTML">
  <p style="text-align:center;color:var(--fg-3);">Loading…</p>
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
                today_n = group_thousands(today_n),
                total = group_thousands(total),
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
