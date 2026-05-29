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
use axum::http::{HeaderMap, StatusCode, header};
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
        .route(
            "/pages/cooccurrence-matrix",
            get(cooccurrence_matrix_partial),
        )
        .route("/pages/acoustic-network", get(acoustic_network_partial))
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

async fn correlation_page(headers: HeaderMap) -> Html<String> {
    let body = CORRELATION_CONTENT.replace(
        "{{help_link}}",
        &super::help::help_link(super::help::Topic::Analytics),
    );
    super::render_page_for_request("Species Co-occurrence", &body, "analytics", &headers)
}

const CORRELATION_CONTENT: &str = r##"<div class="page-head">
  <div>
    <div class="bnb-eyebrow">Behavioral analytics</div>
    <h1 class="display" style="font-size:34px;">Who sings with whom</h1>
    {{help_link}}
    <p class="bnb-meta" style="margin-top:4px;">Which species are detected together most often.</p>
  </div>
  <div class="seg" id="range-controls">
    <button class="btn active" onclick="loadDays(30, this)">30 days</button>
    <button class="btn" onclick="loadDays(90, this)">90 days</button>
    <button class="btn" onclick="loadDays(180, this)">6 months</button>
    <button class="btn" onclick="loadDays(365, this)">1 year</button>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">The yard's social graph</div><h3>Co-occurrence matrix</h3></div></div>
  <div id="cooccurrence-matrix" hx-get="/pages/cooccurrence-matrix?days=30" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading…</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">The acoustic network</div><h3>Who connects to whom</h3></div><span class="bnb-pill">ρ ≥ 0.20</span></div>
  <p class="bnb-meta" style="margin:4px 0 12px;">The same data as the matrix, drawn as ribbons — thicker links co-occur more often, and each species' arc length is its total connectedness in the soundscape.</p>
  <div id="acoustic-network" hx-get="/pages/acoustic-network?days=30" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading…</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Strongest pairs</div><h3>Top co-occurring species</h3></div></div>
  <div id="correlation-pairs" hx-get="/pages/correlation-pairs?days=30" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading…</p>
  </div>
</div>

<div class="bnb-card pad">
  <div class="section-header"><div><div class="bnb-eyebrow">Lookup</div><h3>Companion species</h3></div></div>
  <p class="bnb-meta" style="margin:4px 0 12px;">Enter a species to see which others are commonly detected on the same day.</p>
  <div style="display:flex;gap:.75rem;margin-bottom:1rem;">
    <input type="text" id="species-input" style="width:280px;"
           placeholder="e.g. European Robin"
           hx-get="/pages/companion-species"
           hx-trigger="keyup changed delay:400ms"
           hx-target="#companion-results"
           hx-include="[name='days-val']"
           name="species">
    <input type="hidden" name="days-val" id="days-hidden" value="30">
  </div>
  <div id="companion-results">
    <p class="bnb-meta">Type a species name above…</p>
  </div>
</div>

<script>
function loadDays(days, btn) {
  document.querySelectorAll('#range-controls .btn').forEach(b => b.classList.remove('active'));
  btn.classList.add('active');
  document.getElementById('days-hidden').value = days;
  htmx.ajax('GET', '/pages/cooccurrence-matrix?days=' + days, '#cooccurrence-matrix');
  htmx.ajax('GET', '/pages/acoustic-network?days=' + days, '#acoustic-network');
  htmx.ajax('GET', '/pages/correlation-pairs?days=' + days, '#correlation-pairs');
  const species = document.getElementById('species-input').value.trim();
  if (species) {
    htmx.ajax('GET', '/pages/companion-species?species=' + encodeURIComponent(species) + '&days=' + days, '#companion-results');
  }
}
</script>"##;

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
            "<p style='color:var(--rare)'>Error loading co-occurrence data</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /pages/cooccurrence-matrix — N×N intensity grid
// ---------------------------------------------------------------------------

async fn cooccurrence_matrix_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| top_cooccurrence_pairs(conn, days, 120, 1))
    })
    .await;

    match result {
        Ok(Ok(pairs)) => {
            let (labels, matrix) = build_matrix(&pairs, 10);
            let html = super::viz::cooccurrence_matrix(&labels, &matrix);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading matrix</p>".to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// GET /pages/acoustic-network — chord diagram of the co-occurrence graph
// ---------------------------------------------------------------------------

async fn acoustic_network_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| top_cooccurrence_pairs(conn, days, 120, 1))
    })
    .await;

    match result {
        Ok(Ok(pairs)) => {
            // Fewer arcs read more clearly as a chord than the 10-wide matrix.
            let (labels, matrix) = build_matrix(&pairs, 9);
            let html = super::viz::chord_diagram(&labels, &matrix);
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading network</p>".to_string(),
        ),
    }
}

/// Reduce co-occurrence pairs to a square matrix over the `max_species` most
/// connected species. Cell strength is shared-days normalised to the global
/// maximum so the grid reads as a relative heat-map.
#[allow(clippy::cast_precision_loss)]
fn build_matrix(
    pairs: &[birdnet_db::sqlite::SpeciesPair],
    max_species: usize,
) -> (Vec<String>, Vec<Vec<f64>>) {
    use std::collections::HashMap;

    // Total shared-days each species participates in → connectedness ranking.
    let mut weight: HashMap<&str, i64> = HashMap::new();
    for p in pairs {
        *weight.entry(p.species_a.as_str()).or_insert(0) += p.co_occurrence_days;
        *weight.entry(p.species_b.as_str()).or_insert(0) += p.co_occurrence_days;
    }
    let mut ranked: Vec<(&str, i64)> = weight.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let labels: Vec<String> = ranked
        .iter()
        .take(max_species)
        .map(|(n, _)| (*n).to_string())
        .collect();

    let idx: std::collections::HashMap<&str, usize> = labels
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let n = labels.len();
    let mut matrix = vec![vec![0.0_f64; n]; n];
    let mut max_co = 1.0_f64;
    for p in pairs {
        if let (Some(&i), Some(&j)) = (idx.get(p.species_a.as_str()), idx.get(p.species_b.as_str()))
        {
            let v = p.co_occurrence_days as f64;
            matrix[i][j] = v;
            matrix[j][i] = v;
            max_co = max_co.max(v);
        }
    }
    for row in &mut matrix {
        for cell in row.iter_mut() {
            *cell /= max_co;
        }
    }
    (labels, matrix)
}

fn render_pairs_table(pairs: &[birdnet_db::sqlite::SpeciesPair], _days: u32) -> String {
    if pairs.is_empty() {
        return super::empty_states::no_co_signal();
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
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
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
      <span style="font-size:.75rem;color:var(--fg-3);">{days} days</span>
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
                r#"<p style="color:var(--fg-3);font-size:.875rem;">Type a species name above…</p>"#
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
            "<p style='color:var(--rare)'>Error loading companion species</p>".to_string(),
        ),
    }
}

fn render_companion_table(companions: &[birdnet_db::sqlite::FollowOn]) -> String {
    if companions.is_empty() {
        return r#"<p style="color:var(--fg-3);font-size:.875rem;">
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
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let bar_pct = (c.shared_days as f64 / max_days as f64 * 100.0).round() as u64;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
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
        assert!(html.contains("Not enough overlap"));
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
        assert!(html.contains('5'));
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
