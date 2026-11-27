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

use birdnet_db::sqlite::{ListenedFrom, companion_species, cooccurrence_strata};

use super::escape_html;
use super::simple_url_encode;
use crate::analytics_cache::cached_fragment;
use crate::state::AppState;

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
        lead: "Which birds are heard in the same five-minute block more often, or less often, than chance.",
        lede: "Chance is worked out within each hour of the day, so two birds that both sing at dawn are not counted as together for that alone. A coloured cell stands out from chance; an empty one does not. It is an association, not a cause, and exploratory: they may share a habitat, a weather or a disturbance.",
        aside: ranges,
    });
    CORRELATION_CONTENT.replace("{{page_head}}", &head)
}

static CORRELATION_CONTENT: crate::seal::Sealed = crate::seal::Sealed::new(
    r##"{{page_head}}

<div class="bnb-card pad fig">
  <div class="section-header"><div><div class="bnb-eyebrow"><span class="fig-no" aria-hidden="true"></span>The acoustic network</div><h2 class="sh-h">Who connects to whom</h2></div><span class="bnb-pill">against chance</span></div>
  <div class="pt-viz" id="acoustic-network" hx-get="/pages/acoustic-network?days=30" hx-trigger="load" hx-swap="innerHTML">
    <p class="bnb-meta">Loading…</p>
  </div>
  <details class="pt-disc">
    <summary>See the numbers: every pair against chance</summary>
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
// GET /pages/correlation-pairs — every pair against chance, as a table
// ---------------------------------------------------------------------------

fn compute_correlation_pairs(state: &AppState, days: u32) -> Option<String> {
    let (g, stored) = co_graph(state, days, "co-occurrence table")?;
    Some(render_pairs_table(&g, &stored, 25))
}

async fn correlation_pairs_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let html = cached_fragment(&state, format!("corr-pairs:{days}"), CORR_ERR, move |s| {
        compute_correlation_pairs(s, days).map(with_as_of)
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/cooccurrence-matrix — the ladder on its own
// ---------------------------------------------------------------------------

fn compute_cooccurrence_matrix(state: &AppState, days: u32) -> Option<String> {
    let (g, _) = co_graph(state, days, "co-occurrence ladder")?;
    Some(super::viz::cooccurrence_matrix(&g))
}

async fn cooccurrence_matrix_partial(
    State(state): State<AppState>,
    Query(query): Query<CorrelationQuery>,
) -> impl axum::response::IntoResponse {
    let days = query.days.unwrap_or(30).min(365);
    let html = cached_fragment(&state, format!("corr-matrix:{days}"), CORR_ERR, move |s| {
        compute_cooccurrence_matrix(s, days).map(with_as_of)
    })
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// GET /pages/acoustic-network — the figure: the ladder, the key and the folded ring
// ---------------------------------------------------------------------------

fn compute_acoustic_network(state: &AppState, days: u32, links: usize) -> Option<String> {
    let (g, _) = co_graph(state, days, "co-occurrence chord")?;
    if g.n() < 2 {
        return Some(super::empty_states::no_co_signal());
    }
    Some(super::viz::co_figure(&g, links, "/pages/acoustic-network"))
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
        move |s| compute_acoustic_network(s, days, links).map(with_as_of),
    )
    .await;
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

// ---------------------------------------------------------------------------
// The graph, with its chance level (CHT-08)
// ---------------------------------------------------------------------------

/// The co-occurrence graph of the window: counts by hour from the database, the chance level
/// from the permutation null. `None` when the read fails (never an empty graph in its place).
fn co_graph(state: &AppState, days: u32, what: &str) -> Option<(super::viz::CoGraph, Vec<String>)> {
    let strata = state
        .with_read_db(|conn| cooccurrence_strata(conn, days, MAX_SPECIES))
        .ok()?;
    let slots = super::viz::kit::station_slots_for(state, what)?;
    let g = build_graph(&strata, state.names(), slots);
    Some((g, strata.names))
}

/// What the method line says about where the listened blocks came from.
const fn listened_words(from: Option<ListenedFrom>) -> &'static str {
    match from {
        Some(ListenedFrom::Ledger) => {
            " The blocks the station listened to are read from its hourly listening ledger."
        }
        Some(ListenedFrom::DetectionHours) => {
            " This station has no hourly listening ledger for the period, so every hour with a detection is taken as listened in full, and a quiet hour it did listen to is not counted."
        }
        None => "",
    }
}

/// Test every pair of the species in `strata` against chance and build the graph the pictures
/// draw.
///
/// A pair's test shuffles one bird's blocks among the blocks the station listened to in the
/// same clock hour (`birdnet_behavioral::null::stratified_shuffle`), so each bird keeps its
/// amount of time and its daily rhythm, and two dawn singers are not "together" for both
/// singing at dawn. Every pair is tested, with no floor on shared blocks: a pair that never
/// shares a block is a result too (it may be "less often than chance"). The pairs are tested
/// at once and marked by Benjamini–Hochberg at `FDR_Q`. Each species keeps the colour it owns
/// on the station (`slots`).
#[allow(clippy::many_single_char_names)] // i, j, n, p, r: a pair, the species, the p-values
fn build_graph(
    strata: &birdnet_db::sqlite::CoStrata,
    names: &birdnet_core::i18n::NameLocalizer,
    slots: super::viz::kit::SpeciesSlots,
) -> super::viz::CoGraph {
    use birdnet_behavioral::null::{
        self, FDR_Q, NULL_ROUNDS, Rng, Stratum, Verdict, benjamini_hochberg, stratified_shuffle,
    };

    let n = strata.names.len();
    let mut tests = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            let st: Vec<Stratum> = (0..strata.listened.len())
                .map(|h| Stratum {
                    listened: strata.listened[h],
                    a: strata.blocks[i][h],
                    b: strata.blocks[j][h],
                    shared: strata.shared[i][j][h],
                })
                .collect();
            // The stream is named by the pair, in name order, so a pair's draws do not depend
            // on where it stands in the list.
            let (x, y) = if strata.names[i] <= strata.names[j] {
                (i, j)
            } else {
                (j, i)
            };
            let mut rng = Rng::for_key(&format!("co|{}|{}", strata.names[x], strata.names[y]));
            tests.push((i, j, stratified_shuffle(&st, NULL_ROUNDS, &mut rng)));
        }
    }
    let p: Vec<f64> = tests.iter().map(|t| t.2.p_two_sided()).collect();
    let marked = benjamini_hochberg(&p, FDR_Q);

    let mut strength = vec![vec![0.0_f64; n]; n];
    let mut share = vec![vec![0.0_f64; n]; n];
    let mut blocks = vec![vec![0_i64; n]; n];
    let mut expected = vec![vec![0.0_f64; n]; n];
    let mut verdict = vec![vec![Verdict::Chance; n]; n];
    let mut top = f64::MIN_POSITIVE;
    for ((i, j, r), &stands_out) in tests.iter().zip(&marked) {
        let (i, j) = (*i, *j);
        let v = Verdict::from_null(r, stands_out);
        let shared = strata.total_shared(i, j);
        let either = strata.total(i) + strata.total(j) - shared;
        #[allow(clippy::cast_precision_loss)]
        let jaccard = if either > 0 {
            shared as f64 / either as f64
        } else {
            0.0
        };
        let shared = i64::try_from(shared).unwrap_or(i64::MAX);
        for (a, b) in [(i, j), (j, i)] {
            share[a][b] = jaccard;
            blocks[a][b] = shared;
            expected[a][b] = r.expected;
            verdict[a][b] = v;
        }
        if v == Verdict::More
            && let Some(l) = r.lift()
        {
            strength[i][j] = l.ln();
            strength[j][i] = l.ln();
            top = top.max(l.ln());
        }
    }
    for row in &mut strength {
        for cell in row.iter_mut() {
            *cell /= top;
        }
    }
    let tested = tests.len();
    super::viz::CoGraph {
        names: strata
            .names
            .iter()
            .map(|l| names.name_of_stored(l).to_owned())
            .collect(),
        slots,
        strength,
        share,
        blocks,
        expected,
        verdict,
        method: format!(
            "{}{}",
            null::method_words(
                "one bird's five-minute blocks are put back at random among the blocks the station listened to in the same hour of the day",
                tested,
            ),
            listened_words(strata.listened_from),
        ),
    }
}

/// Every pair tested, the ones that stand out first (furthest from chance first), then the
/// rest by blocks shared: the table behind the ladder.
fn render_pairs_table(g: &super::viz::CoGraph, stored: &[String], limit: usize) -> String {
    use super::viz::cograph::{Verdict, lift_words};
    let n = g.n();
    if n < 2 {
        return super::empty_states::no_co_signal();
    }
    let mut rows: Vec<(usize, usize)> = (0..n)
        .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
        .collect();
    let away = |&(i, j): &(usize, usize)| -> f64 {
        if g.verdict(i, j) == Verdict::Chance {
            -1.0
        } else {
            g.lift(i, j)
                .map_or(0.0, |l| l.max(f64::MIN_POSITIVE).ln().abs())
        }
    };
    rows.sort_by(|x, y| {
        away(y)
            .total_cmp(&away(x))
            .then(g.blocks[y.0][y.1].cmp(&g.blocks[x.0][x.1]))
            .then(x.cmp(y))
    });
    rows.truncate(limit);

    let mut html = String::from(
        r"<table>
<thead>
  <tr>
    <th>Species A</th>
    <th>Species B</th>
    <th>5-min blocks together</th>
    <th>Chance gives</th>
    <th>Against chance</th>
  </tr>
</thead>
<tbody>",
    );
    for (i, j) in rows {
        let verdict = g.verdict(i, j);
        let against = g.lift(i, j).map_or_else(
            || super::viz::matrix::verdict_words(verdict).to_owned(),
            |l| {
                format!(
                    "{}, {}",
                    lift_words(l),
                    super::viz::matrix::verdict_words(verdict)
                )
            },
        );
        let _ = write!(
            html,
            r#"<tr>
  <td><a class="species-link" href="/species/detail?name={enc_a}">{a}</a></td>
  <td><a class="species-link" href="/species/detail?name={enc_b}">{b}</a></td>
  <td>{blocks}</td>
  <td>{chance:.1}</td>
  <td>{against}</td>
</tr>"#,
            enc_a = simple_url_encode(stored.get(i).map_or("", String::as_str)),
            enc_b = simple_url_encode(stored.get(j).map_or("", String::as_str)),
            a = escape_html(&g.names[i]),
            b = escape_html(&g.names[j]),
            blocks = g.blocks[i][j],
            chance = g.expected[i][j],
            against = escape_html(&against),
        );
    }
    html.push_str("</tbody></table>");
    let _ = write!(
        html,
        r#"<p class="bnb-meta">{}</p>"#,
        escape_html(&g.method)
    );
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
        compute_companion(s, &species, days).map(with_as_of)
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

    // Ranked by the share of their days the two have in common, not by how many: the commonest
    // residents share most of anyone's days and were every species' first companion (UIC-04).
    // The bar is that share itself, not a length relative to the first row.
    let trigger = companions
        .first()
        .map_or("", |c| names.name_of_stored(&c.trigger));
    let mut html = format!(
        r#"<p class="co-hint">Ranked by the share of their days the two have in common: the days both were heard, out of the days either was. So a bird heard every day is not first merely for being common: it shares all of {who}'s days, and many more besides.</p>
<table>
<thead>
  <tr>
    <th>Companion species</th>
    <th>Heard on the same day</th>
    <th>How sure (average)</th>
    <th>Share of their days</th>
  </tr>
</thead>
<tbody>"#,
        who = escape_html(trigger),
    );

    for c in companions {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        let share_pct = (c.overlap.clamp(0.0, 1.0) * 100.0).round() as u64;
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
    {share_pct}%<div class="bar" data-style="width:{share_pct}%;min-width:4px;"></div>
  </td>
</tr>"#,
            name = escape_html(names.name_of_stored(&c.companion)),
            days = companion_days_text(c, trigger),
            conf = conf_pct,
        );
    }

    html.push_str("</tbody></table>");
    html
}

/// "8 of the Nutcracker's 10 days": the shared days, out of the trigger's, in the station's
/// language and escaped.
pub(super) fn companion_days_text(c: &birdnet_db::sqlite::FollowOn, trigger_shown: &str) -> String {
    format!(
        "{} of the {}’s {} day{}",
        c.shared_days,
        escape_html(trigger_shown),
        c.trigger_days,
        if c.trigger_days == 1 { "" } else { "s" }
    )
}

/// Pre-compute and cache the default 30-day co-occurrence fragments so the
/// first visit (and each background refresh) is instant.
pub fn prewarm(state: &AppState) {
    let cache = state.analytics_cache();
    if let Some(h) = compute_acoustic_network(state, 30, DEFAULT_LINKS).map(with_as_of) {
        cache.put(format!("corr-network:30:{DEFAULT_LINKS}"), h);
    }
    if let Some(h) = compute_correlation_pairs(state, 30).map(with_as_of) {
        cache.put("corr-pairs:30".to_string(), h);
    }
}

/// A figure from the analytics cache ends with the line that says how old it is (UIC-23).
#[allow(clippy::needless_pass_by_value)]
fn with_as_of(html: String) -> String {
    html + crate::analytics_cache::AS_OF_LINE
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_db::sqlite::FollowOn;

    fn english() -> birdnet_core::i18n::NameLocalizer {
        birdnet_core::i18n::NameLocalizer::english()
    }

    /// A station that listened to every hour of the last 30 days, and the species placed in
    /// five-minute blocks by `place(day, hour, slot) -> names`.
    fn station(place: impl Fn(u32, u32, u32) -> Vec<&'static str>) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let mut rows = Vec::new();
        let mut effort = Vec::new();
        for back in 1..=30_u32 {
            let d = format!("DATE('now', 'localtime', '-{back} days')");
            for h in 0..24_u32 {
                effort.push(format!("({d},{h},'mic',3600,3600)"));
                for s in 0..12_u32 {
                    for name in place(back, h, s) {
                        rows.push(format!(
                            "({d},'{h:02}:{m:02}:00','{name} sp','{name}',0.9)",
                            m = s * 5 + 1
                        ));
                    }
                }
            }
        }
        conn.execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES {};
             INSERT INTO recording_effort_hourly (date, hour, source, seconds_analysed, seconds_up) VALUES {};",
            rows.join(","),
            effort.join(",")
        ))
        .unwrap();
        conn
    }

    /// A fixed pseudo-random draw in `0..1000` for a block and a bird: the fixture's own dice,
    /// so it is the same on every run.
    fn dice(day: u32, hour: u32, slot: u32, bird: u32) -> u32 {
        let mut x = u64::from(day) * 1_000_003
            + u64::from(hour) * 10_007
            + u64::from(slot) * 101
            + u64::from(bird) * 7_919;
        x ^= x >> 13;
        x = x.wrapping_mul(0x5bd1_e995_9e37_79b9);
        x ^= x >> 29;
        u32::try_from(x % 1000).unwrap()
    }

    fn figure(conn: &rusqlite::Connection) -> (crate::routes::pages::viz::CoGraph, String) {
        let strata = cooccurrence_strata(conn, 30, MAX_SPECIES).unwrap();
        let g = build_graph(
            &strata,
            &english(),
            crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        let html = crate::routes::pages::viz::co_figure(&g, 12, "/pages/acoustic-network");
        (g, html)
    }

    /// Five birds, each in its own share of the dawn and morning blocks, independently of the
    /// others: they share many blocks because they share hours, and none more or less than
    /// those hours give.
    fn independent(day: u32, hour: u32, slot: u32) -> Vec<&'static str> {
        const BIRDS: [(&str, u32); 5] = [
            ("Robin", 450),
            ("Wren", 350),
            ("Blackbird", 300),
            ("Dunnock", 250),
            ("Chaffinch", 200),
        ];
        if !(4..=9).contains(&hour) {
            return Vec::new();
        }
        BIRDS
            .iter()
            .zip(0_u32..)
            .filter(|((_, p), k)| dice(day, hour, slot, *k) < *p)
            .map(|((n, _), _)| *n)
            .collect()
    }

    /// CHT-08: birds that are only heard in the same hours are not drawn as together. Every
    /// cell is empty and the figure says nothing stands out, though every pair shares dozens of
    /// blocks (which the old figure drew as its strongest ribbons).
    #[test]
    fn birds_heard_only_in_the_same_hours_draw_nothing() {
        let conn = station(independent);
        let (g, html) = figure(&conn);
        assert_eq!(g.n(), 5);
        assert!(
            g.blocks[0][1] > 30,
            "the pairs do share blocks: {:?}",
            g.blocks
        );
        assert_eq!(g.standing_out(), (0, 0), "{:?}", g.verdict);
        assert!(html.contains(crate::routes::pages::viz::matrix::NOTHING_STANDS_OUT));
        assert!(
            !html.contains("ck-fill ck-dp") && !html.contains("ck-fill ck-dn"),
            "no cell is coloured: {html}"
        );
        assert!(html.contains("999 times (seed 20261127)"), "{html}");
        assert!(html.contains("hourly listening ledger"), "{html}");
    }

    /// Counterpart: a bird that comes in with the Robin, in the same blocks far more often
    /// than their hours give, is the one pair marked, and the darkest.
    #[test]
    fn a_planted_follower_is_the_pair_that_stands_out() {
        let conn = station(|day, hour, slot| {
            let mut birds = independent(day, hour, slot);
            if birds.contains(&"Robin") && dice(day, hour, slot, 99) < 400 {
                birds.push("Nuthatch");
            }
            birds
        });
        let (g, html) = figure(&conn);
        let (more, _) = g.standing_out();
        assert_eq!(more, 1, "{:?}", g.verdict);
        let links = g.ranked_links();
        let top = (g.names[links[0].a].as_str(), g.names[links[0].b].as_str());
        assert!(
            top == ("Robin", "Nuthatch") || top == ("Nuthatch", "Robin"),
            "{top:?}"
        );
        assert!(!html.contains(crate::routes::pages::viz::matrix::NOTHING_STANDS_OUT));
        assert!(html.contains("Together more than chance"), "{html}");
    }

    /// The table holds every pair tested, the ones that stand out first, with what chance gives.
    #[test]
    fn the_table_puts_what_stands_out_first_beside_what_chance_gives() {
        let conn = station(|day, hour, slot| {
            let mut birds = independent(day, hour, slot);
            if birds.contains(&"Robin") && dice(day, hour, slot, 99) < 400 {
                birds.push("Nuthatch");
            }
            birds
        });
        let strata = cooccurrence_strata(&conn, 30, MAX_SPECIES).unwrap();
        let g = build_graph(
            &strata,
            &english(),
            crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        let html = render_pairs_table(&g, &strata.names, 25);
        assert_eq!(html.matches("<tr>").count(), 1 + 15, "6 species, 15 pairs");
        let first = html.split("<tbody>").nth(1).unwrap();
        let first = first.split("</tr>").next().unwrap();
        assert!(
            first.contains("Nuthatch") && first.contains("Robin"),
            "{first}"
        );
        assert!(first.contains("together more often than chance"), "{first}");
        assert!(html.contains("<th>Chance gives</th>"), "{html}");
        assert!(html.contains("within chance"), "{html}");
        assert!(
            html.contains("href=\"/species/detail?name=Nuthatch\""),
            "{html}"
        );
    }

    #[test]
    fn the_table_escapes_names_and_says_when_there_is_nothing() {
        let empty = birdnet_db::sqlite::CoStrata::default();
        let g = build_graph(
            &empty,
            &english(),
            crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        assert!(render_pairs_table(&g, &[], 25).contains("Not enough overlap"));
        let strata = birdnet_db::sqlite::CoStrata {
            names: vec!["<script>alert(1)</script>".into(), "Wren".into()],
            listened: vec![12; 24],
            blocks: vec![vec![1; 24], vec![1; 24]],
            shared: vec![
                vec![vec![0; 24], vec![1; 24]],
                vec![vec![1; 24], vec![0; 24]],
            ],
            listened_from: Some(ListenedFrom::DetectionHours),
        };
        let g = build_graph(
            &strata,
            &english(),
            crate::routes::pages::viz::kit::SpeciesSlots::default(),
        );
        let html = render_pairs_table(&g, &strata.names, 25);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("no hourly listening ledger"), "{html}");
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
            trigger_days: 10,
            companion_days: 12,
            overlap: 8.0 / 14.0,
            avg_confidence: 0.85,
        }];
        let html =
            render_companion_table(&companions, &birdnet_core::i18n::NameLocalizer::english());
        assert!(html.contains("Blue Tit"));
        assert!(html.contains("85%"));
        // UIC-04: what the rank is made of, said in words: the shared days out of the trigger's,
        // and the share (8 of the 14 days either was heard), not a length relative to the top row.
        assert!(html.contains("8 of the Robin’s 10 days"), "{html}");
        assert!(
            html.contains("57%<div class=\"bar\" data-style=\"width:57%"),
            "{html}"
        );
        assert!(html.contains("<th>Share of their days</th>"), "{html}");
        assert!(!html.contains("Against the most"), "{html}");
    }
}
