//! Detection detail page.
//!
//! Shows a single detection with:
//! - Inline audio player
//! - Spectrogram image (generated from the WAV file)
//! - Species information card
//! - Links to related detections

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/detections/detail", get(detection_detail_page))
}

#[derive(Debug, Deserialize)]
pub struct DetectionDetailQuery {
    /// Date (YYYY-MM-DD).
    date: Option<String>,
    /// Time (HH:MM:SS).
    time: Option<String>,
    /// Common name filter (optional, used to disambiguate if multiple species at same time).
    name: Option<String>,
}

async fn detection_detail_page(
    State(state): State<AppState>,
    Query(query): Query<DetectionDetailQuery>,
) -> Result<Html<String>, StatusCode> {
    let date = query.date.unwrap_or_default();
    let time = query.time.unwrap_or_default();
    let com_name = query.name.unwrap_or_default();

    if date.is_empty() || time.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let date2 = date.clone();
    let time2 = time.clone();
    let com2 = com_name.clone();

    let detection = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| find_detection(conn, &date2, &time2, &com2))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(det) = detection else {
        return Ok(Html(not_found_page(&date, &time)));
    };

    Ok(Html(render_detail_page(&det)))
}

// ---------------------------------------------------------------------------
// DB query
// ---------------------------------------------------------------------------

fn find_detection(
    conn: &rusqlite::Connection,
    date: &str,
    time: &str,
    com_name: &str,
) -> Option<birdnet_db::sqlite::DetectionRow> {
    use rusqlite::params;
    use birdnet_db::sqlite::DetectionRow;

    if com_name.is_empty() {
        conn.query_row(
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name
             FROM detections WHERE Date = ?1 AND Time = ?2 LIMIT 1",
            params![date, time],
            |row| Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            }),
        ).ok()
    } else {
        conn.query_row(
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name
             FROM detections WHERE Date = ?1 AND Time = ?2 AND Com_Name = ?3 LIMIT 1",
            params![date, time, com_name],
            |row| Ok(DetectionRow {
                date: row.get(0)?,
                time: row.get(1)?,
                sci_name: row.get(2)?,
                com_name: row.get(3)?,
                confidence: row.get(4)?,
                lat: row.get(5)?,
                lon: row.get(6)?,
                cutoff: row.get(7)?,
                week: row.get(8)?,
                sens: row.get(9)?,
                overlap: row.get(10)?,
                file_name: row.get(11)?,
            }),
        ).ok()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_detail_page(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let conf_pct = det.confidence * 100.0;
    let conf_color = if conf_pct >= 80.0 { "#34d399" } else if conf_pct >= 50.0 { "#fbbf24" } else { "#f87171" };
    let enc_name = simple_url_encode(&det.com_name);
    let enc_sci = simple_url_encode(&det.sci_name);

    let audio_section = build_audio_section(det);
    let meta = build_meta_rows(det);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{com_name} — {date} {time} — BirdNet-Behavior</title>
  <link rel="stylesheet" href="/static/style.css">
  <style>
    body {{ background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; }}
    .container {{ max-width:900px; margin:0 auto; padding:2rem 1rem; }}
    nav a {{ color:#94a3b8; text-decoration:none; margin-right:1.5rem; }}
    nav a:hover {{ color:#38bdf8; }}
    .card {{ background:#1e293b; border:1px solid #334155; border-radius:0.75rem; padding:1.5rem; margin-bottom:1.5rem; }}
    .section-title {{ font-size:1.1rem; font-weight:600; color:#38bdf8; margin-bottom:1rem; border-bottom:1px solid #334155; padding-bottom:0.5rem; }}
    table {{ width:100%; border-collapse:collapse; }}
    td {{ padding:0.4rem 0.75rem; border-bottom:1px solid #1e293b; font-size:0.9rem; }}
    tr:last-child td {{ border-bottom:none; }}
    td:first-child {{ color:#94a3b8; width:35%; }}
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem; padding:1rem 0; border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/species/detail?name={enc_name}">↩ {com_name_esc}</a>
  </nav>

  <h1 style="font-size:1.5rem;font-weight:700;margin-bottom:0.5rem;color:#f1f5f9;">
    {com_name_esc}
  </h1>
  <p style="color:#64748b;margin-bottom:1.5rem;font-style:italic;">{sci_name_esc}</p>

  <div class="card">
    <div class="section-title">Detection Details</div>
    <table>
      <tr><td>Date</td><td>{date_esc}</td></tr>
      <tr><td>Time</td><td>{time_esc}</td></tr>
      <tr><td>Confidence</td><td><strong style="color:{conf_color};">{conf_pct:.1}%</strong></td></tr>
      {meta}
    </table>
  </div>

  {audio_section}

  <div class="card">
    <div class="section-title">Related</div>
    <p><a href="/species/detail?name={enc_name}" style="color:#38bdf8;">
      All detections of {com_name_esc} →
    </a></p>
    <p><a href="/api/v2/images/{enc_sci}" style="color:#38bdf8;">
      Species photo (Wikipedia) →
    </a></p>
  </div>
</div>
</body>
</html>"#,
        com_name = escape_html(&det.com_name),
        com_name_esc = escape_html(&det.com_name),
        sci_name_esc = escape_html(&det.sci_name),
        date = escape_html(&det.date),
        date_esc = escape_html(&det.date),
        time = escape_html(&det.time),
        time_esc = escape_html(&det.time),
    )
}

fn build_audio_section(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let Some(ref fname) = det.file_name else { return String::new() };
    if fname.is_empty() { return String::new(); }

    let basename = std::path::Path::new(fname)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let safe = escape_html(&basename);
    format!(
        r#"<div class="card">
  <div class="section-title">Recording</div>
  <audio controls style="width:100%;margin-bottom:1rem;">
    <source src="/api/v2/recordings/{safe}" type="audio/wav">
    Your browser does not support audio playback.
  </audio>
  <img src="/api/v2/spectrogram/{safe}"
       alt="Spectrogram"
       style="width:100%;border-radius:0.5rem;border:1px solid #334155;"
       onerror="this.style.display='none'">
</div>"#
    )
}

fn build_meta_rows(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let mut out = String::new();
    if let (Some(lat), Some(lon)) = (det.lat, det.lon) {
        let _ = write!(out, "<tr><td>Location</td><td>{lat:.4}°N, {lon:.4}°E</td></tr>");
    }
    if let Some(sens) = det.sens {
        let _ = write!(out, "<tr><td>Sensitivity</td><td>{sens:.2}</td></tr>");
    }
    if let Some(overlap) = det.overlap {
        let _ = write!(out, "<tr><td>Overlap</td><td>{overlap:.1}s</td></tr>");
    }
    if let Some(cutoff) = det.cutoff {
        let _ = write!(out, "<tr><td>Cutoff</td><td>{:.0}%</td></tr>", cutoff * 100.0);
    }
    out
}

fn not_found_page(date: &str, time: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Not Found</title></head>
<body style="background:#0f172a;color:#e2e8f0;font-family:system-ui;padding:2rem;">
  <h1>Detection not found</h1>
  <p>No detection found for date=<code>{date}</code> time=<code>{time}</code>.</p>
  <a href="/" style="color:#38bdf8;">← Back to dashboard</a>
</body>
</html>"#,
        date = escape_html(date),
        time = escape_html(time),
    )
}
