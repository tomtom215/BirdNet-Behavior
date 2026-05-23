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

use super::atoms::conf_bar;
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
        return Ok(not_found_page(&date, &time));
    };

    Ok(render_detail_page(&det))
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

fn render_detail_page(det: &birdnet_db::sqlite::DetectionRow) -> Html<String> {
    let enc_name = simple_url_encode(&det.com_name);
    let enc_sci = simple_url_encode(&det.sci_name);
    let com = escape_html(&det.com_name);
    let sci = escape_html(&det.sci_name);
    let date = escape_html(&det.date);
    let time = escape_html(&det.time);

    let audio_section = build_audio_section(det);
    let meta = build_meta_rows(det);
    let correlation_section = build_correlation_section(det);
    let conf = conf_bar(det.confidence);

    // Public, HMAC-signed share link for this detection (O-07). The button
    // copies an absolute `/r/<token>` URL built from the page's own origin.
    let token = crate::routes::share::issue_token_for(&det.date, &det.time, &det.com_name);
    let share_path = format!("/r/{token}");
    let share_button = format!(
        r#"<button type="button" class="bnb-btn" title="Copy a public share link" onclick="(function(b){{var u=location.origin+'{share_path}';if(navigator.clipboard){{navigator.clipboard.writeText(u).then(function(){{b.textContent='Link copied';setTimeout(function(){{b.textContent='Share clip';}},1500);}});}}else{{window.prompt('Copy this link:',u);}}}})(this)">Share clip</button>"#
    );

    let content = format!(
        r#"<div class="page-head">
  <div>
    <div class="bnb-eyebrow">Detection · {date} {time}</div>
    <h1 class="display" style="font-size:40px;line-height:1.05;">{com}</h1>
    <p class="mono" style="color:var(--fg-3);font-style:italic;margin-top:4px;">{sci}</p>
  </div>
  <div style="display:flex;gap:8px;flex-wrap:wrap;">
    <a class="bnb-btn" href="/species/detail?name={enc_name}">All detections →</a>
    {share_button}
  </div>
</div>

<div class="grid-2">
  <div>
    {audio_section}
    <div class="bnb-card pad">
      <div class="section-header"><div><div class="bnb-eyebrow">Details</div><h3>This detection</h3></div>{conf}</div>
      <table>
        <tr><td class="bnb-meta">Date</td><td>{date}</td></tr>
        <tr><td class="bnb-meta">Time</td><td>{time}</td></tr>
        {meta}
      </table>
    </div>
    {correlation_section}
  </div>
  <div class="bnb-card pad">
    <div class="section-header"><div><div class="bnb-eyebrow">Related</div><h3>Explore</h3></div></div>
    <p style="margin-bottom:8px;"><a href="/species/detail?name={enc_name}">All detections of {com} →</a></p>
    <p><a href="/api/v2/species/image/{enc_sci}/file">Species photo (Wikipedia) →</a></p>
  </div>
</div>"#
    );

    super::render_page(&format!("{com} · {date} {time}"), &content, "today")
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
        r#"<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Recording</div><h3>The 3-second clip</h3></div></div>
  <img src="/api/v2/spectrogram/{safe}"
       alt="Spectrogram"
       style="width:100%;border-radius:var(--r-md);border:0.5px solid var(--border);display:block;margin-bottom:12px;"
       onerror="this.style.display='none'">
  <audio controls style="width:100%;">
    <source src="/api/v2/recordings/{safe}" type="audio/wav">
    Your browser does not support audio playback.
  </audio>
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
        r#"<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Operator</div><h3>Daemon log trace</h3></div></div>
  <p class="bnb-meta" style="margin-bottom:0.75rem;">
    Every event the detection daemon emitted for this audio file is
    tagged with the correlation ID below.
    Run <code>journalctl -u birdnet | grep {safe}</code> to see the
    exact decode/infer/notify slice that produced this row.
  </p>
  <div style="display:flex;align-items:center;gap:0.5rem;flex-wrap:wrap;">
    <code id="correlation-id"
          style="background:var(--surface-2);padding:0.5rem 0.75rem;border-radius:var(--r-sm);
                 font-family:var(--font-mono);color:var(--moss-ink);">{safe}</code>
    <button type="button" id="copy-correlation-id" class="bnb-btn"
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

fn not_found_page(date: &str, time: &str) -> Html<String> {
    let content = format!(
        r#"<div class="empty-state">
  <h1 class="display" style="font-size:32px;">Detection not found</h1>
  <p class="bnb-meta" style="margin-top:8px;">No detection found for date <code>{date}</code> time <code>{time}</code>.</p>
  <p style="margin-top:16px;"><a class="bnb-btn" href="/">← Back to dashboard</a></p>
</div>"#,
        date = escape_html(date),
        time = escape_html(time),
    );
    super::render_page("Detection not found", &content, "today")
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
