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
    use birdnet_db::sqlite::DetectionRow;
    use rusqlite::params;

    if com_name.is_empty() {
        conn.query_row(
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, correlation_id
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
                correlation_id: row.get(12)?,
            }),
        ).ok()
    } else {
        conn.query_row(
            "SELECT Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, correlation_id
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
                correlation_id: row.get(12)?,
            }),
        ).ok()
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render_detail_page(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let conf_pct = det.confidence * 100.0;
    let conf_color = if conf_pct >= 80.0 {
        "#34d399"
    } else if conf_pct >= 50.0 {
        "#fbbf24"
    } else {
        "#f87171"
    };
    let enc_name = simple_url_encode(&det.com_name);
    let enc_sci = simple_url_encode(&det.sci_name);

    let audio_section = build_audio_section(det);
    let meta = build_meta_rows(det);
    let correlation_section = build_correlation_section(det);

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

  {correlation_section}

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
    let Some(ref fname) = det.file_name else {
        return String::new();
    };
    if fname.is_empty() {
        return String::new();
    }

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

/// Render the per-row correlation-id card with a "Copy" affordance.
///
/// The `correlation_id` is the per-file ID the detection daemon stamps on
/// every log line, DB write, and notification for one audio file
/// (migration 12). Surfacing it on the row's detail page closes the
/// log → row traceability loop: an operator who clicks a suspicious
/// detection can copy the id and run `journalctl -u birdnet | grep <id>`
/// to pull the exact decode/infer/notify slice the daemon emitted for
/// that file.
///
/// Returns an empty string when the row pre-dates migration 12
/// (BirdNET-Pi-imported rows, quarantine-approve rows) so the card
/// doesn't render an empty "Correlation ID: " line.
fn build_correlation_section(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let Some(ref id) = det.correlation_id else {
        return String::new();
    };
    if id.is_empty() {
        return String::new();
    }
    let safe = escape_html(id);
    // Vanilla JS clipboard copy with a clear fallback affordance. We
    // ship inline because the admin pages don't have a shared script
    // bundle yet and a single 4-line script is cheaper than wiring one.
    // The button intentionally shows the ID inline so an operator on a
    // browser without clipboard access can read it directly.
    format!(
        r#"<div class="card">
  <div class="section-title">Operator: Daemon Log Trace</div>
  <p style="margin-bottom:0.75rem;color:#94a3b8;font-size:0.85rem;">
    Every event the detection daemon emitted for this audio file is
    tagged with the correlation ID below.
    Run <code>journalctl -u birdnet | grep {safe}</code> to see the
    exact decode/infer/notify slice that produced this row.
  </p>
  <div style="display:flex;align-items:center;gap:0.5rem;flex-wrap:wrap;">
    <code id="correlation-id"
          style="background:#0f172a;padding:0.5rem 0.75rem;border-radius:0.375rem;
                 font-family:ui-monospace,Menlo,monospace;color:#a7f3d0;">{safe}</code>
    <button type="button" id="copy-correlation-id"
            style="background:#1e293b;color:#e2e8f0;border:1px solid #334155;
                   border-radius:0.375rem;padding:0.5rem 0.75rem;cursor:pointer;"
            onclick="(function(){{
              const el=document.getElementById('correlation-id');
              const txt=el.textContent;
              if(navigator.clipboard){{navigator.clipboard.writeText(txt);}}
              else{{const r=document.createRange();r.selectNode(el);
                    window.getSelection().removeAllRanges();
                    window.getSelection().addRange(r);
                    document.execCommand('copy');}}
              const b=document.getElementById('copy-correlation-id');
              b.textContent='Copied!';
              setTimeout(()=>{{b.textContent='Copy';}}, 1500);
            }})()">Copy</button>
  </div>
</div>"#
    )
}

fn build_meta_rows(det: &birdnet_db::sqlite::DetectionRow) -> String {
    let mut out = String::new();
    if let (Some(lat), Some(lon)) = (det.lat, det.lon) {
        let _ = write!(
            out,
            "<tr><td>Location</td><td>{lat:.4}°N, {lon:.4}°E</td></tr>"
        );
    }
    if let Some(sens) = det.sens {
        let _ = write!(out, "<tr><td>Sensitivity</td><td>{sens:.2}</td></tr>");
    }
    if let Some(overlap) = det.overlap {
        let _ = write!(out, "<tr><td>Overlap</td><td>{overlap:.1}s</td></tr>");
    }
    if let Some(cutoff) = det.cutoff {
        let _ = write!(
            out,
            "<tr><td>Cutoff</td><td>{:.0}%</td></tr>",
            cutoff * 100.0
        );
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

#[cfg(test)]
mod tests {
    use super::build_correlation_section;
    use birdnet_db::sqlite::DetectionRow;

    fn row_with_id(id: Option<&str>) -> DetectionRow {
        DetectionRow {
            date: "2026-05-19".into(),
            time: "09:00:00".into(),
            sci_name: "Pica pica".into(),
            com_name: "Eurasian Magpie".into(),
            confidence: 0.95,
            lat: None,
            lon: None,
            cutoff: None,
            week: None,
            sens: None,
            overlap: None,
            file_name: None,
            correlation_id: id.map(str::to_owned),
        }
    }

    #[test]
    fn correlation_section_empty_when_id_is_none() {
        // Rows that pre-date migration 12 (BirdNET-Pi imports,
        // quarantine-approve writes) have no correlation_id. The
        // section must not render then — otherwise the operator sees
        // an empty grey card with no actionable content.
        assert_eq!(build_correlation_section(&row_with_id(None)), "");
    }

    #[test]
    fn correlation_section_empty_when_id_is_empty_string() {
        // Defensive: an empty string id is treated the same as None.
        assert_eq!(build_correlation_section(&row_with_id(Some(""))), "");
    }

    #[test]
    fn correlation_section_renders_the_id() {
        // The id appears in the code element so an operator can read
        // it on a browser without clipboard access.
        let html = build_correlation_section(&row_with_id(Some("a1b2c3d4")));
        assert!(html.contains("a1b2c3d4"), "id not present in rendered html");
        assert!(
            html.contains("journalctl"),
            "operator command hint not rendered"
        );
        assert!(
            html.contains("id=\"correlation-id\""),
            "id node not present"
        );
        assert!(html.contains("Copy"), "copy affordance not rendered");
    }

    #[test]
    fn correlation_section_escapes_html_in_id() {
        // The id is rendered into a `code` element and into the
        // `journalctl ... grep <id>` command snippet. Pin escaping
        // so a maliciously crafted DB row (or BirdNET-Pi import) can't
        // break out into the page's script context.
        let html = build_correlation_section(&row_with_id(Some("<script>alert(1)</script>")));
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "raw script tag should be escaped"
        );
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "expected HTML-encoded id"
        );
    }
}
