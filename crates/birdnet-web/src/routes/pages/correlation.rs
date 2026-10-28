//! Species co-occurrence correlation page and partials.
//!
//! Shows which species are commonly detected together — useful for
//! understanding mixed flocks, habitat associations, and observation patterns.
//!
//! | Path | Purpose |
//! |------|---------|
//! | (embedded)                         | Patterns home, "Who sings together" tab |
//! | `GET /pages/correlation-pairs`     | Top co-occurrence pairs (HTMX partial)   |
//! | `GET /pages/companion-species`     | Companion species for a trigger species  |

use std::fmt::Write as _;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::routing::get;
use serde::Deserialize;

use birdnet_db::sqlite::{companion_species, top_cooccurrence_pairs};

use super::escape_html;
use super::simple_url_encode;
use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

/// A pair needs this many shared five-minute blocks to be drawn. Below it, two
/// birds heard once, together, would read as perfectly associated.
const MIN_SHARED_BLOCKS: u32 = 3;

/// Fallback served (uncached) when a co-occurrence query errors.
const CORR_ERR: &str = r#"<p class="co-err">Co-occurrence data temporarily unavailable.</p>"#;

/// Mount correlation routes.
pub fn router() -> Router<AppState> {
    Router::new()
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
    #[serde(default, deserialize_with = "crate::routes::form::blank_is_none")]
    days: Option<u32>,
    species: Option<String>,
    /// How many of the strongest links the chord diagram draws (`0` = all of them).
    links: Option<usize>,
}

/// Links the chord diagram draws until the reader asks for more: a ring with every pair
/// of ten species on it is the tangle this chart used to be.
const DEFAULT_LINKS: usize = super::viz::LINK_CHOICES[0];

/// The most species either picture holds.
const MAX_SPECIES: usize = 10;

// ---------------------------------------------------------------------------
// Page content — embedded in the Patterns home ("Who sings together" tab)
// ---------------------------------------------------------------------------

/// The co-occurrence surface, rendered for embedding by `homes::patterns`.
pub(super) fn content() -> String {
    let ranges = r#"<div class="seg" id="range-controls" role="group" aria-label="Time range"><button type="button" class="btn active" data-days="30" aria-pressed="true">30 days</button><button type="button" class="btn" data-days="90" aria-pressed="false">90 days</button><button type="button" class="btn" data-days="180" aria-pressed="false">6 months</button><button type="button" class="btn" data-days="365" aria-pressed="false">1 year</button></div>"#;
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Analytics),
        title: "Who sings with <em>whom</em>",
        lead: "Species joined by a ribbon are often heard in the same five-minute block.",
        lede: "A wider, darker ribbon means the two birds share more of their time. Point at a name to see only its ribbons. It's correlation, not cause: they may just share the same good morning.",
        aside: ranges,
    });
    CORRELATION_CONTENT.replace("{{page_head}}", &head)
}

static CORRELATION_CONTENT: crate::seal::Sealed = crate::seal::Sealed::new(
    r##"{{page_head}}

<div class="bnb-card pad fig">
  <div class="section-header"><div><div class="bnb-eyebrow"><span class="fig-no" aria-hidden="true"></span>The acoustic network</div><h2 class="sh-h">Who connects to whom</h2></div><span class="bnb-pill">same 5-min block</span></div>
  <div class="pt-viz" id="acoustic-network" hx-get="/pages/acoustic-network?days=30" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading…</p>
  </div>
  <details class="pt-disc co-disc-matrix">
    <summary>See the numbers — the co-occurrence matrix</summary>
    <div id="cooccurrence-matrix" hx-get="/pages/cooccurrence-matrix" hx-include="#days-hidden" hx-trigger="toggle from:closest details once, intersect once" hx-swap="innerHTML">
      <span class="htmx-indicator bnb-meta">Loading…</span>
    </div>
  </details>
  <details class="pt-disc">
    <summary>See the numbers — strongest co-occurring pairs</summary>
    <div id="correlation-pairs" hx-get="/pages/correlation-pairs" hx-include="#days-hidden" hx-trigger="toggle from:closest details once, intersect once" hx-swap="innerHTML">
      <span class="htmx-indicator bnb-meta">Loading…</span>
    </div>
  </details>
</div>

<div class="bnb-card pad fig">
  <div class="section-header"><div><div class="bnb-eyebrow"><span class="fig-no" aria-hidden="true"></span>Lookup</div><h2 class="sh-h">Companion species</h2></div></div>
  <p class="bnb-meta co-meta-gap">Enter a species to see which others are commonly detected on the same day.</p>
  <div class="co-lookup-row">
    <input type="text" id="species-input" class="co-species-input"
           placeholder="e.g. European Robin"
           hx-get="/pages/companion-species"
           hx-trigger="keyup changed delay:400ms"
           hx-target="#companion-results"
           hx-include="#days-hidden"
           name="species">
    <!-- The range every lazily-loaded part of this tab asks for. Named `days`
         because that is what the server reads: `days-val` was ignored, so the
         companion lookup always answered for 30 days, and the two tables
         below fetched a hard-coded `?days=30` on first opening. -->
    <input type="hidden" name="days" id="days-hidden" value="30">
  </div>
  <div id="companion-results">
    <p class="bnb-meta">Type a species name above…</p>
  </div>
</div>

<script>
function loadDays(days, btn) {
  document.querySelectorAll('#range-controls .btn').forEach(function (b) {
    b.classList.remove('active');
    b.setAttribute('aria-pressed', 'false');
  });
  btn.classList.add('active');
  btn.setAttribute('aria-pressed', 'true');
  document.getElementById('days-hidden').value = days;
  htmx.ajax('GET', '/pages/cooccurrence-matrix?days=' + days, '#cooccurrence-matrix');
  htmx.ajax('GET', '/pages/acoustic-network?days=' + days, '#acoustic-network');
  htmx.ajax('GET', '/pages/correlation-pairs?days=' + days, '#correlation-pairs');
  const species = document.getElementById('species-input').value.trim();
  if (species) {
    htmx.ajax('GET', '/pages/companion-species?species=' + encodeURIComponent(species) + '&days=' + days, '#companion-results');
  }
}
document.getElementById('range-controls').addEventListener('click', function(e) {
  const btn = e.target.closest('button[data-days]');
  if (btn) loadDays(parseInt(btn.dataset.days, 10), btn);
});
</script>"##,
);

// ---------------------------------------------------------------------------
// GET /pages/correlation-pairs — top co-occurring pairs partial
// ---------------------------------------------------------------------------

fn compute_correlation_pairs(state: &AppState, days: u32) -> Option<String> {
    let pairs = state
        .with_read_db(|conn| top_cooccurrence_pairs(conn, days, 25, MIN_SHARED_BLOCKS))
        .ok()?;
    Some(render_pairs_table(&pairs, days, state.names()))
}

async fn correlation_pairs_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let html = cached_fragment(&state, format!("corr-pairs:{days}"), CORR_ERR, move |s| {
        compute_correlation_pairs(s, days)
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/cooccurrence-matrix — N×N intensity grid
// ---------------------------------------------------------------------------

fn compute_cooccurrence_matrix(state: &AppState, days: u32) -> Option<String> {
    let pairs = state
        .with_read_db(|conn| top_cooccurrence_pairs(conn, days, 120, MIN_SHARED_BLOCKS))
        .ok()?;
    Some(super::viz::cooccurrence_matrix(&build_graph(
        &pairs,
        MAX_SPECIES,
        state.names(),
    )))
}

async fn cooccurrence_matrix_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let html = cached_fragment(&state, format!("corr-matrix:{days}"), CORR_ERR, move |s| {
        compute_cooccurrence_matrix(s, days)
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/acoustic-network — chord diagram of the co-occurrence graph
// ---------------------------------------------------------------------------

fn compute_acoustic_network(state: &AppState, days: u32, links: usize) -> Option<String> {
    let pairs = state
        .with_read_db(|conn| top_cooccurrence_pairs(conn, days, 120, MIN_SHARED_BLOCKS))
        .ok()?;
    if pairs.is_empty() {
        return Some(super::empty_states::no_co_signal());
    }
    let graph = build_graph(&pairs, MAX_SPECIES, state.names());
    Some(super::viz::co_figure(
        &graph,
        links,
        "/pages/acoustic-network",
    ))
}

async fn acoustic_network_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let links = query.links.unwrap_or(DEFAULT_LINKS).min(200);
    let html = cached_fragment(
        &state,
        format!("corr-network:{days}:{links}"),
        CORR_ERR,
        move |s| compute_acoustic_network(s, days, links),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// Reduce co-occurrence pairs to the graph over the `max_species` most connected species.
///
/// Species are ranked by the sum of the overlap they take part in, busiest first, and that
/// rank is their colour slot in every picture. Strength is the pair's overlap scaled to the
/// strongest pair among those species, so the pictures read as a relative picture; the
/// overlap as measured and the blocks behind it travel along, because a reading must say
/// "38 % of the blocks either was heard in", not "0.71 of the strongest".
fn build_graph(
    pairs: &[birdnet_db::sqlite::SpeciesPair],
    max_species: usize,
    names: &birdnet_core::i18n::NameLocalizer,
) -> super::viz::CoGraph {
    use std::collections::HashMap;

    // Summed overlap each species participates in → connectedness ranking.
    let mut weight: HashMap<&str, f64> = HashMap::new();
    for p in pairs {
        *weight.entry(p.species_a.as_str()).or_insert(0.0) += p.overlap;
        *weight.entry(p.species_b.as_str()).or_insert(0.0) += p.overlap;
    }
    let mut ranked: Vec<(&str, f64)> = weight.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    let labels: Vec<&str> = ranked.iter().take(max_species).map(|(n, _)| *n).collect();

    let idx: HashMap<&str, usize> = labels.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let n = labels.len();
    let mut strength = vec![vec![0.0_f64; n]; n];
    let mut share = vec![vec![0.0_f64; n]; n];
    let mut blocks = vec![vec![0_i64; n]; n];
    let mut max_overlap = f64::MIN_POSITIVE;
    for p in pairs {
        if let (Some(&i), Some(&j)) = (idx.get(p.species_a.as_str()), idx.get(p.species_b.as_str()))
            && i != j
        {
            share[i][j] = p.overlap;
            share[j][i] = p.overlap;
            blocks[i][j] = p.shared_blocks;
            blocks[j][i] = p.shared_blocks;
            max_overlap = max_overlap.max(p.overlap);
        }
    }
    for (row, out) in share.iter().zip(strength.iter_mut()) {
        for (v, cell) in row.iter().zip(out.iter_mut()) {
            *cell = v / max_overlap;
        }
    }
    super::viz::CoGraph {
        names: labels
            .iter()
            .map(|l| names.name_of_stored(l).to_owned())
            .collect(),
        strength,
        share,
        blocks,
    }
}

fn render_pairs_table(
    pairs: &[birdnet_db::sqlite::SpeciesPair],
    _days: u32,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    if pairs.is_empty() {
        return super::empty_states::no_co_signal();
    }

    let max_overlap = pairs
        .iter()
        .map(|p| p.overlap)
        .fold(f64::MIN_POSITIVE, f64::max);

    let mut html = String::from(
        r"<table>
<thead>
  <tr>
    <th>Species A</th>
    <th>Species B</th>
    <th>5-min blocks together</th>
    <th>Share of their time</th>
  </tr>
</thead>
<tbody>",
    );

    for pair in pairs {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bar_pct = (pair.overlap / max_overlap * 100.0).round() as u64;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let share_pct = (pair.overlap * 100.0).round() as u64;
        let enc_a = simple_url_encode(&pair.species_a);
        let enc_b = simple_url_encode(&pair.species_b);
        let _ = write!(
            html,
            r#"<tr>
  <td><a class="species-link" href="/species/detail?name={enc_a}">{a}</a></td>
  <td><a class="species-link" href="/species/detail?name={enc_b}">{b}</a></td>
  <td>{blocks}</td>
  <td>
    <div class="co-bar-row">
      <div class="bar" data-style="width:{bar_pct}%;min-width:4px;"></div>
      <span class="co-bar-label">{share_pct}%</span>
    </div>
  </td>
</tr>"#,
            a = escape_html(names.name_of_stored(&pair.species_a)),
            b = escape_html(names.name_of_stored(&pair.species_b)),
            blocks = pair.shared_blocks,
        );
    }

    html.push_str("</tbody></table>");
    html
}

// ---------------------------------------------------------------------------
// GET /pages/companion-species — companion lookup partial
// ---------------------------------------------------------------------------

fn compute_companion(state: &AppState, species: &str, days: u32) -> Option<String> {
    let companions = state
        .with_read_db(|conn| {
            // Typed in the station's language, asked of the database in the
            // classifier's.
            let stored = super::name_search::stored_name_for_typed(conn, state.names(), species);
            companion_species(conn, &stored, days, 15)
        })
        .ok()?;
    Some(render_companion_table(&companions, state.names()))
}

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
                r#"<p class="co-hint">Type a species name above…</p>"#.to_string(),
            );
        }
    };
    let days = query.days.unwrap_or(30).min(365);
    let key = format!("corr-companion:{days}:{species}");
    let html = cached_fragment(&state, key, CORR_ERR, move |s| {
        compute_companion(s, &species, days)
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

fn render_companion_table(
    companions: &[birdnet_db::sqlite::FollowOn],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    if companions.is_empty() {
        return r#"<p class="co-hint">
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
    <div class="bar" data-style="width:{bar_pct}%;min-width:4px;"></div>
  </td>
</tr>"#,
            name = escape_html(names.name_of_stored(&c.companion)),
            days = c.shared_days,
            conf = conf_pct,
        );
    }

    html.push_str("</tbody></table>");
    html
}

/// Pre-compute and cache the default 30-day co-occurrence fragments so the
/// first visit (and each background refresh) is instant.
pub fn prewarm(state: &AppState) {
    let cache = state.analytics_cache();
    if let Some(h) = compute_cooccurrence_matrix(state, 30) {
        cache.put("corr-matrix:30".to_string(), h);
    }
    if let Some(h) = compute_acoustic_network(state, 30, DEFAULT_LINKS) {
        cache.put(format!("corr-network:30:{DEFAULT_LINKS}"), h);
    }
    if let Some(h) = compute_correlation_pairs(state, 30) {
        cache.put("corr-pairs:30".to_string(), h);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite::{FollowOn, SpeciesPair};

    #[test]
    fn render_pairs_table_empty() {
        let html = render_pairs_table(&[], 30, &birdnet_core::i18n::NameLocalizer::english());
        assert!(html.contains("Not enough overlap"));
    }

    #[test]
    fn render_pairs_table_with_data() {
        let pairs = vec![SpeciesPair {
            species_a: "Robin".into(),
            species_b: "Wren".into(),
            shared_blocks: 5,
            blocks_a: 10,
            blocks_b: 8,
            overlap: 5.0 / 13.0,
        }];
        let html = render_pairs_table(&pairs, 30, &birdnet_core::i18n::NameLocalizer::english());
        assert!(html.contains("Robin"));
        assert!(html.contains("Wren"));
        assert!(html.contains("<td>5</td>"), "{html}");
        assert!(html.contains(">38%<"), "5 of 13 blocks: {html}");
    }

    #[test]
    fn render_pairs_table_escapes_html() {
        let pairs = vec![SpeciesPair {
            species_a: "<script>alert(1)</script>".into(),
            species_b: "Wren".into(),
            shared_blocks: 1,
            blocks_a: 1,
            blocks_b: 1,
            overlap: 1.0,
        }];
        let html = render_pairs_table(&pairs, 30, &birdnet_core::i18n::NameLocalizer::english());
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    fn pair(a: &str, b: &str, shared: u32, overlap: f64) -> SpeciesPair {
        SpeciesPair {
            species_a: a.into(),
            species_b: b.into(),
            shared_blocks: i64::from(shared),
            blocks_a: 40,
            blocks_b: 40,
            overlap,
        }
    }

    /// The graph the pictures draw keeps what a reading needs: the overlap as measured (not
    /// only as scaled to the strongest pair) and the blocks behind it, and the colour slot of
    /// a species is its rank by connectedness.
    #[test]
    fn the_graph_keeps_the_measured_share_beside_the_scaled_one() {
        let pairs = vec![
            pair("Robin", "Wren", 30, 0.60),
            pair("Robin", "Jay", 15, 0.30),
            pair("Wren", "Jay", 6, 0.12),
            pair("Jay", "Owl", 3, 0.05),
        ];
        let g = build_graph(&pairs, 3, &birdnet_core::i18n::NameLocalizer::english());
        // Robin (0.90) and Wren (0.72) and Jay (0.47) are the three most connected; Owl is cut.
        assert_eq!(g.names, vec!["Robin", "Wren", "Jay"]);
        assert!(
            (g.strength[0][1] - 1.0).abs() < 1e-12,
            "the strongest pair is 1.0"
        );
        assert!((g.strength[0][2] - 0.5).abs() < 1e-12);
        assert!(
            (g.share[0][1] - 0.60).abs() < 1e-12,
            "the share is as measured"
        );
        assert_eq!(g.blocks[1][2], 6);
        for i in 0..3 {
            assert!(
                g.strength[i][i].abs() < 1e-12,
                "no species is linked to itself"
            );
            for j in 0..3 {
                assert!(
                    (g.strength[i][j] - g.strength[j][i]).abs() < 1e-12,
                    "symmetric"
                );
                assert_eq!(g.blocks[i][j], g.blocks[j][i]);
            }
        }
    }

    /// A pair outside the species kept does not stretch the scale: only the strongest link
    /// *drawn* is 1.0.
    #[test]
    fn a_pair_that_is_not_drawn_does_not_set_the_scale() {
        let pairs = vec![
            pair("A", "B", 10, 0.40),
            pair("A", "C", 8, 0.30),
            pair("B", "C", 8, 0.20),
            pair("X", "Y", 50, 0.45),
        ];
        // X and Y (0.45 each) are less connected in total than A (0.70), B (0.60), C (0.50)
        // and are cut at three, though their pair is stronger than A-B.
        let g = build_graph(&pairs, 3, &birdnet_core::i18n::NameLocalizer::english());
        assert_eq!(g.names, vec!["A", "B", "C"]);
        assert!((g.strength[0][1] - 1.0).abs() < 1e-12, "{:?}", g.strength);
    }

    /// The chord diagram's "links shown" is read from the request and bounded.
    #[test]
    fn the_default_is_the_first_choice_the_figure_offers() {
        assert_eq!(DEFAULT_LINKS, 12);
        assert_eq!(super::super::viz::LINK_CHOICES, [12, 24, 0]);
    }

    #[test]
    fn render_companion_table_empty() {
        let html = render_companion_table(&[], &birdnet_core::i18n::NameLocalizer::english());
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
        let html =
            render_companion_table(&companions, &birdnet_core::i18n::NameLocalizer::english());
        assert!(html.contains("Blue Tit"));
        assert!(html.contains("85%"));
    }
}
