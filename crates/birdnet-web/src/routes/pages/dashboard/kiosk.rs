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
<link rel="stylesheet" href="/static/css/app.css?v={{version}}">
<style>
  body { padding:4vh 5vw; overflow:hidden; }
  .kiosk-head { display:flex; align-items:center; justify-content:center; gap:12px; margin-bottom:4vh; }
  .kiosk-head .title { font-family:var(--font-display); font-size:clamp(28px,4vw,52px); letter-spacing:-0.02em; margin:0; font-weight:400; }
  .stats { display:flex; gap:24px; justify-content:center; margin-bottom:4vh; flex-wrap:wrap; }
  .stat { background:var(--surface); border:0.5px solid var(--border); border-radius:var(--r-lg); padding:20px 36px; text-align:center; min-width:170px; box-shadow:var(--shadow-md); }
  .stat .value { font-family:var(--font-display); font-variant-numeric:tabular-nums; font-size:clamp(34px,5vw,64px); line-height:1; color:var(--moss); }
  .stat .label { font-size:11px; letter-spacing:0.1em; text-transform:uppercase; color:var(--fg-3); margin-top:10px; }
  .recent { max-width:1100px; margin:0 auto; max-height:calc(100vh - 34vh); overflow-y:auto; }
  .detection { display:flex; align-items:center; gap:18px; padding:14px 2px; border-bottom:0.5px solid var(--hairline); }
  .detection .name { font-weight:500; font-size:clamp(16px,1.6vw,22px); }
  .detection .sci { font-style:italic; color:var(--fg-3); font-size:13px; font-family:var(--font-mono); }
  .detection .time { color:var(--fg-3); font-size:13px; margin-left:auto; white-space:nowrap; font-family:var(--font-mono); }
  /* Quiet escape hatch: a wall display is a dead end without one. Dimmed so
     it never competes with the display content; brightens on hover/focus
     for the operator standing at the screen. ESC works too (script below). */
  .kiosk-exit { position:fixed; top:14px; right:18px; color:var(--fg-3);
                font-size:13px; text-decoration:none; transition:opacity .2s; }
  .kiosk-exit:hover, .kiosk-exit:focus-visible { opacity:1; }
</style>
</head>
<body>
<!-- A wall display is still a document: it had no <main>, so nothing on it
     was inside a landmark, and no <h1>, so it had no accessible title beyond
     the <title> element. The heading is visually the existing wordmark. -->
<header class="kiosk-head">
  <a class="kiosk-exit" href="/" aria-label="Exit kiosk mode">Exit&nbsp;✕</a>
  <svg width="32" height="32" viewBox="0 0 24 24" aria-hidden="true">
    <circle cx="12" cy="12" r="11" fill="none" stroke="currentColor" stroke-width="0.8" class="ki-fg"></circle>
    <g stroke="currentColor" stroke-width="1.4" stroke-linecap="round" class="ki-fg">
      <line x1="6" y1="12" x2="6" y2="12"></line><line x1="9" y1="9.5" x2="9" y2="14.5"></line>
      <line x1="12" y1="6" x2="12" y2="18"></line><line x1="15" y1="8" x2="15" y2="16"></line>
      <line x1="18" y1="10.5" x2="18" y2="13.5"></line>
    </g>
  </svg>
  <h1 class="title">BirdNet<span class="ki-fg3">Behavior</span></h1>
</header>
<main id="kiosk-content"
     hx-get="/pages/kiosk-content"
     hx-trigger="load, every 30s"
     hx-swap="innerHTML">
  <p class="ki-loading">Loading…</p>
</main>
<script src="/static/htmx.min.js"></script>
<script>
  // ESC leaves kiosk mode — the keyboard counterpart of the corner link.
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') { window.location.href = '/'; }
  });
</script>
</body>
</html>"#;

pub(super) async fn kiosk_page() -> Html<String> {
    Html(crate::routes::pages::with_asset_version(KIOSK_HTML))
}

pub(super) async fn kiosk_content_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let result = tokio::task::spawn_blocking(move || {
        // A kiosk hangs on a wall and nobody is reading a log beside it, so a
        // failed read must not paint three zeroes and an empty list. Each of
        // these is a claim about the reader's birds; all four propagate.
        state.with_db(|conn| {
            let today = today_date_string();
            let total = birdnet_db::sqlite::detection_count(conn)?;
            let today_count = birdnet_db::sqlite::todays_detection_count(
                conn,
                &today,
                None,
                birdnet_db::sqlite::TodayFilter::All,
            )?;
            let species = birdnet_db::sqlite::species_count(conn)?;
            let recent = birdnet_db::sqlite::recent_detections(conn, 15)?;
            Ok::<_, birdnet_db::sqlite::DbError>((total, today_count, species, recent))
        })
    })
    .await;

    match result {
        Ok(Ok((total, today_n, species_n, recent))) => {
            let mut html = String::with_capacity(4096);
            let _ = write!(
                html,
                r#"<div class="stats">
  <div class="stat"><div class="value">{today_n}</div><div class="label">Today</div></div>
  <div class="stat"><div class="value">{total}</div><div class="label">Total</div></div>
  <div class="stat"><div class="value">{species_n}</div><div class="label">Species</div></div>
</div>
<div class="recent" tabindex="0" role="group" aria-label="Recent detections">"#,
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
        // 200, not 500: this partial is polled by the kiosk page, and htmx
        // discards a 5xx body, so a 500 leaves the wall display showing the
        // last good numbers for ever. The error state has to be swapped in.
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "kiosk: query failed");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                crate::routes::pages::error_states::inline("the station's totals"),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "kiosk: task failed");
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                crate::routes::pages::error_states::inline("the station's totals"),
            )
        }
    }
}
