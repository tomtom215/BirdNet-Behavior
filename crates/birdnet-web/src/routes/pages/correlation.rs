//! Species co-occurrence correlation page and partials.
//!
//! Shows which species are commonly detected together — useful for
//! understanding mixed flocks, habitat associations, and observation patterns.
//!
//! | Path | Purpose |
//! |------|---------|
//! | `GET /correlation`                 | Full correlation page                    |
//! | `GET /pages/correlation-pairs`     | Top co-occurrence pairs (HTMX partial)   |
//! | `GET /pages/companion-species`     | Companion species for a trigger species  |

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::Html;
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{companion_species, top_cooccurrence_pairs};

use super::escape_html;
use super::simple_url_encode;
use crate::state::AppState;

/// Mount correlation routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/correlation", get(correlation_page))
        .route("/pages/correlation-pairs", get(correlation_pairs_partial))
        .route("/pages/companion-species", get(companion_partial))
}

#[derive(Deserialize)]
struct CorrelationQuery {
    days: Option<u32>,
    species: Option<String>,
}

// ---------------------------------------------------------------------------
// GET /correlation — full page
// ---------------------------------------------------------------------------

async fn correlation_page() -> Html<String> {
    Html(CORRELATION_PAGE.to_string())
}

const CORRELATION_PAGE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width,initial-scale=1.0">
  <title>Species Correlation — BirdNet-Behavior</title>
  <script src="/static/htmx.min.js"></script>
  <style>
    body { background:#0f172a; color:#e2e8f0; font-family:system-ui,sans-serif; margin:0; }
    .container { max-width:1000px; margin:0 auto; padding:2rem 1rem; }
    nav a { color:#94a3b8; text-decoration:none; margin-right:1.5rem; font-size:.9rem; }
    nav a:hover, nav a.active { color:#38bdf8; }
    h1 { font-size:1.5rem; font-weight:700; color:#f1f5f9; margin-bottom:.5rem; }
    .subtitle { color:#64748b; font-size:.875rem; margin-bottom:2rem; }
    .card { background:#1e293b; border:1px solid #334155; border-radius:.75rem;
            padding:1.5rem; margin-bottom:1.5rem; }
    .section-title { font-size:1rem; font-weight:600; color:#38bdf8;
                     margin-bottom:1rem; }
    table { width:100%; border-collapse:collapse; font-size:.875rem; }
    th { text-align:left; padding:.5rem .75rem; color:#64748b; font-weight:500;
         border-bottom:1px solid #334155; font-size:.75rem; text-transform:uppercase; }
    td { padding:.5rem .75rem; border-bottom:1px solid #1e293b; }
    tr:hover td { background:#334155; }
    .bar { height:8px; background:#0ea5e9; border-radius:4px; }
    .controls { display:flex; gap:.75rem; margin-bottom:1.5rem; flex-wrap:wrap;
                align-items:center; }
    .btn { padding:.4rem 1rem; border-radius:.375rem; border:1px solid #334155;
           background:#1e293b; color:#e2e8f0; cursor:pointer; font-size:.875rem; }
    .btn.active, .btn:hover { background:#0ea5e9; border-color:#0ea5e9; color:#fff; }
    input[type="text"] { background:#0f172a; border:1px solid #334155; border-radius:.375rem;
                          padding:.4rem .75rem; color:#e2e8f0; font-size:.875rem;
                          outline:none; width:280px; }
    input[type="text"]:focus { border-color:#38bdf8; }
    a.species-link { color:#38bdf8; text-decoration:none; }
    a.species-link:hover { text-decoration:underline; }
    .tag { display:inline-block; padding:2px 8px; border-radius:9999px; font-size:.7rem;
           font-weight:600; background:#0c4a6e; color:#7dd3fc; }
  </style>
</head>
<body>
<div class="container">
  <nav style="margin-bottom:2rem;padding:1rem 0;border-bottom:1px solid #334155;">
    <a href="/">Dashboard</a>
    <a href="/species">Species</a>
    <a href="/heatmap">Heatmap</a>
    <a href="/analytics">Analytics</a>
    <a href="/correlation" class="active">Correlation</a>
    <a href="/admin">Admin</a>
  </nav>

  <h1>Species Co-occurrence</h1>
  <p class="subtitle">Which species are detected together most often?</p>

  <div class="controls">
    <button class="btn active" onclick="loadDays(30, this)">30 days</button>
    <button class="btn" onclick="loadDays(90, this)">90 days</button>
    <button class="btn" onclick="loadDays(180, this)">6 months</button>
    <button class="btn" onclick="loadDays(365, this)">1 year</button>
  </div>

  <!-- Top pairs -->
  <div class="card">
    <div class="section-title">Top Co-occurring Species Pairs</div>
    <div id="correlation-pairs"
         hx-get="/pages/correlation-pairs?days=30"
         hx-trigger="load"
         hx-swap="innerHTML">
      <p style="color:#64748b;">Loading…</p>
    </div>
  </div>

  <!-- Companion lookup -->
  <div class="card">
    <div class="section-title">Companion Species Lookup</div>
    <p style="font-size:.875rem;color:#94a3b8;margin-bottom:1rem;">
      Enter a species to see which others are commonly detected on the same day.
    </p>
    <div style="display:flex;gap:.75rem;margin-bottom:1rem;">
      <input type="text" id="species-input"
             placeholder="e.g. European Robin"
             hx-get="/pages/companion-species"
             hx-trigger="keyup changed delay:400ms"
             hx-target="#companion-results"
             hx-include="[name='days-val']"
             name="species">
      <input type="hidden" name="days-val" id="days-hidden" value="30">
    </div>
    <div id="companion-results">
      <p style="color:#64748b;font-size:.875rem;">Type a species name above…</p>
    </div>
  </div>
</div>

<script>
let currentDays = 30;
function loadDays(days, btn) {
  currentDays = days;
  document.querySelectorAll('.btn').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  document.getElementById('days-hidden').value = days;
  htmx.ajax('GET', '/pages/correlation-pairs?days=' + days, '#correlation-pairs');
  const species = document.getElementById('species-input').value.trim();
  if (species) {
    htmx.ajax('GET', '/pages/companion-species?species=' + encodeURIComponent(species) + '&days=' + days, '#companion-results');
  }
}
</script>
</body>
</html>"##;

// ---------------------------------------------------------------------------
// GET /pages/correlation-pairs — top co-occurring pairs partial
// ---------------------------------------------------------------------------

async fn correlation_pairs_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| top_cooccurrence_pairs(conn, days, 25, 2))
    })
    .await;

    match result {
        Ok(Ok(pairs)) => {
            let html = render_pairs_table(&pairs, days);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p style='color:#f87171'>Error loading co-occurrence data</p>".to_string(),
        ),
    }
}

fn render_pairs_table(pairs: &[birdnet_db::sqlite::SpeciesPair], days: u32) -> String {
    if pairs.is_empty() {
        return format!(
            r#"<p style="color:#64748b;text-align:center;padding:1.5rem;">
               No co-occurring pairs found in the last {days} days.
               Try extending the time window.
             </p>"#
        );
    }

    let max_days = pairs
        .iter()
        .map(|p| p.co_occurrence_days)
        .max()
        .unwrap_or(1);

    let mut html = String::from(
        r"<table>
<thead>
  <tr>
    <th>Species A</th>
    <th>Species B</th>
    <th>Shared Days</th>
    <th>Co-occurrence</th>
  </tr>
</thead>
<tbody>",
    );

    for pair in pairs {
        #[allow(clippy::cast_precision_loss)]
        let bar_pct = (pair.co_occurrence_days as f64 / max_days as f64 * 100.0).round() as u64;
        let enc_a = simple_url_encode(&pair.species_a);
        let enc_b = simple_url_encode(&pair.species_b);
        let _ = write!(
            html,
            r#"<tr>
  <td><a class="species-link" href="/species/detail?name={enc_a}">{a}</a></td>
  <td><a class="species-link" href="/species/detail?name={enc_b}">{b}</a></td>
  <td>{days}</td>
  <td>
    <div style="display:flex;align-items:center;gap:.5rem;">
      <div class="bar" style="width:{bar_pct}%;min-width:4px;"></div>
      <span style="font-size:.75rem;color:#64748b;">{days} days</span>
    </div>
  </td>
</tr>"#,
            a = escape_html(&pair.species_a),
            b = escape_html(&pair.species_b),
            days = pair.co_occurrence_days,
        );
    }

    html.push_str("</tbody></table>");
    html
}

// ---------------------------------------------------------------------------
// GET /pages/companion-species — companion lookup partial
// ---------------------------------------------------------------------------

async fn companion_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let species = match query.species.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_owned(),
        _ => {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                r#"<p style="color:#64748b;font-size:.875rem;">Type a species name above…</p>"#
                    .to_string(),
            );
        }
    };

    let days = query.days.unwrap_or(30).min(365);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| companion_species(conn, &species, days, 15))
    })
    .await;

    match result {
        Ok(Ok(companions)) => {
            let html = render_companion_table(&companions);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p style='color:#f87171'>Error loading companion species</p>".to_string(),
        ),
    }
}

fn render_companion_table(companions: &[birdnet_db::sqlite::FollowOn]) -> String {
    if companions.is_empty() {
        return r#"<p style="color:#64748b;font-size:.875rem;">
          No companion species found. Try a different name or extend the time window.
        </p>"#
            .to_string();
    }

    let max_days = companions.iter().map(|c| c.shared_days).max().unwrap_or(1);

    let mut html = String::from(
        r"<table>
<thead>
  <tr>
    <th>Companion Species</th>
    <th>Shared Days</th>
    <th>Avg Confidence</th>
    <th>Co-occurrence</th>
  </tr>
</thead>
<tbody>",
    );

    for c in companions {
        #[allow(clippy::cast_precision_loss)]
        let bar_pct = (c.shared_days as f64 / max_days as f64 * 100.0).round() as u64;
        let conf_pct = (c.avg_confidence * 100.0).round() as u32;
        let enc = simple_url_encode(&c.companion);
        let _ = write!(
            html,
            r#"<tr>
  <td><a class="species-link" href="/species/detail?name={enc}">{name}</a></td>
  <td>{days}</td>
  <td>{conf}%</td>
  <td>
    <div class="bar" style="width:{bar_pct}%;min-width:4px;"></div>
  </td>
</tr>"#,
            name = escape_html(&c.companion),
            days = c.shared_days,
            conf = conf_pct,
        );
    }

    html.push_str("</tbody></table>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite::{FollowOn, SpeciesPair};

    #[test]
    fn render_pairs_table_empty() {
        let html = render_pairs_table(&[], 30);
        assert!(html.contains("No co-occurring"));
    }

    #[test]
    fn render_pairs_table_with_data() {
        let pairs = vec![SpeciesPair {
            species_a: "Robin".into(),
            species_b: "Wren".into(),
            co_occurrence_days: 5,
            count_a: 10,
            count_b: 8,
        }];
        let html = render_pairs_table(&pairs, 30);
        assert!(html.contains("Robin"));
        assert!(html.contains("Wren"));
        assert!(html.contains("5"));
    }

    #[test]
    fn render_pairs_table_escapes_html() {
        let pairs = vec![SpeciesPair {
            species_a: "<script>alert(1)</script>".into(),
            species_b: "Wren".into(),
            co_occurrence_days: 1,
            count_a: 1,
            count_b: 1,
        }];
        let html = render_pairs_table(&pairs, 30);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn render_companion_table_empty() {
        let html = render_companion_table(&[]);
        assert!(html.contains("No companion"));
    }

    #[test]
    fn render_companion_table_with_data() {
        let companions = vec![FollowOn {
            trigger: "Robin".into(),
            companion: "Blue Tit".into(),
            shared_days: 8,
            avg_confidence: 0.85,
        }];
        let html = render_companion_table(&companions);
        assert!(html.contains("Blue Tit"));
        assert!(html.contains("85%"));
    }
}
