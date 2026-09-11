//! Species list page, species detail page, and all species HTMX partials.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::atoms::{avatar, conf_bar, sparkline, species_code, species_color};
use super::charts::{render_daily_chart, render_hourly_chart};
use super::{SPECIES_DETAIL_HTML, escape_html, simple_url_encode};
use crate::state::AppState;

#[derive(Deserialize)]
pub(super) struct SpeciesQuery {
    pub name: Option<String>,
}

/// The Species home query: which view, plus the List/Photos filter + search.
#[derive(Debug, Default, Deserialize)]
pub(super) struct HomeParams {
    view: Option<String>,
    filter: Option<String>,
    q: Option<String>,
    /// Which taxonomic rank `taxon` names: `class`, `order` or `genus`.
    rank: Option<String>,
    /// The value at that rank, e.g. `Piciformes`. Ignored without a `rank`.
    taxon: Option<String>,
}

/// The taxonomic ranks the species pages can browse by (`G-15`).
///
/// Three, and deliberately not four: the classifier's label file states a
/// class and an order and nothing between them, so there is no family to
/// browse. See [`birdnet_web::state::Taxon`](crate::state::Taxon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rank {
    /// `Aves`, `Insecta`, … — what tells a bird from a bush-cricket.
    Class,
    /// `Piciformes`, `Strigiformes`, … — from the label file's column.
    Order,
    /// The first word of the binomial.
    Genus,
}

impl Rank {
    /// The rank named by a query parameter, or `None` for anything else.
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "class" => Some(Self::Class),
            "order" => Some(Self::Order),
            "genus" => Some(Self::Genus),
            _ => None,
        }
    }

    /// The query-parameter spelling, which is also the URL's.
    const fn key(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Order => "order",
            Self::Genus => "genus",
        }
    }

    /// This rank's value for a species, if the station has its taxonomy.
    fn of(self, taxon: &crate::state::Taxon) -> Option<&str> {
        match self {
            Self::Class => taxon.class.as_deref(),
            Self::Order => taxon.order.as_deref(),
            Self::Genus => taxon.genus.as_deref(),
        }
    }
}

/// A chosen `rank = value` pair, e.g. order = Piciformes.
#[derive(Debug, Clone)]
pub(super) struct TaxonFilter {
    rank: Rank,
    value: String,
}

impl TaxonFilter {
    /// Read one out of the query string. Both halves are required, and an
    /// unknown rank name is no filter rather than an empty page.
    fn from_params(params: &HomeParams) -> Option<Self> {
        let rank = Rank::parse(params.rank.as_deref()?.trim())?;
        let value = params.taxon.as_deref()?.trim();
        (!value.is_empty()).then(|| Self {
            rank,
            value: value.to_owned(),
        })
    }

    /// Whether a species sits at this rank and value.
    ///
    /// A species the station has no taxonomy for does **not** match. That is
    /// the honest answer — nothing says it belongs — and it is why the chips
    /// are built from the species that do have one, so a chip always leads
    /// somewhere.
    fn admits(&self, state: &AppState, scientific_name: &str) -> bool {
        state
            .taxon(scientific_name)
            .and_then(|t| self.rank.of(t))
            .is_some_and(|v| v.eq_ignore_ascii_case(&self.value))
    }

    /// The `&rank=…&taxon=…` tail for a link that keeps this filter.
    fn query_tail(&self) -> String {
        format!(
            "&amp;rank={}&amp;taxon={}",
            self.rank.key(),
            simple_url_encode(&self.value)
        )
    }
}

/// Mount the species list, species detail, and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/species", get(species_page))
        .route("/species/detail", get(species_detail_page))
        .route("/pages/species-summary", get(species_summary_partial))
        .route("/pages/species-hourly", get(species_hourly_partial))
        .route("/pages/species-detections", get(species_detections_partial))
        .route("/pages/species-daily", get(species_daily_partial))
        .route("/pages/species-info", get(species_info_partial))
        .route("/pages/species-companions", get(species_companions_partial))
        .route("/pages/species-hero", get(species_hero_partial))
        .route("/pages/species-status", get(species_status_partial))
}

/// The Species home (`/species?view=list|photos|lifelist`).
///
/// Folds the three pre-spine destinations — `/species` (List), `/gallery`
/// (Photos) and `/life-list` (Life list) — into one home with a view switcher,
/// filter chips and search. `/gallery` and `/life-list` permanently redirect
/// here (see [`crate::routes::redirects`]).
async fn species_page(
    State(state): State<AppState>,
    Query(params): Query<HomeParams>,
    headers: HeaderMap,
) -> Html<String> {
    let view = match params.view.as_deref() {
        Some("photos") => "photos",
        Some("lifelist") => "lifelist",
        _ => "list",
    };
    // The Life list answers a different question (every species ever), so the
    // List/Photos filter chips don't apply there.
    let filter = if params.filter.as_deref() == Some("week") {
        "week"
    } else {
        "all"
    };
    let search = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // The Life list counts every species ever, so a taxonomy filter there
    // would silently change what "life list" means.
    let taxon = (view != "lifelist")
        .then(|| TaxonFilter::from_params(&params))
        .flatten();

    let st = state.clone();
    let s2 = search.clone();
    let t2 = taxon.clone();
    let body = tokio::task::spawn_blocking(move || match view {
        "photos" => photos_view(&st, filter, s2.as_deref(), t2.as_ref()),
        "lifelist" => lifelist_view(&st),
        _ => list_view(&st, filter, s2.as_deref(), t2.as_ref()),
    })
    .await
    .unwrap_or_default();

    let help = super::help::help_link(super::help::Topic::Species);
    let head = format!(
        r#"<div class="page-head" data-screen-label="Species head">
  <div>
    <div class="bnb-eyebrow"><span>Species</span>{help}</div>
    <h1 class="display sp-h1">Who you've heard</h1>
    <p class="bnb-meta sp-lede">Every bird your station has identified — browse the list, the photos, or your growing life list.</p>
  </div>
</div>"#
    );
    let content = format!(
        "{head}{controls}{body}",
        controls = controls(view, filter, search.as_deref(), taxon.as_ref())
    );
    super::render_page_for_request("Species", &content, "species", &headers)
}

/// The three Species views, in switcher order: `(key, glyph + label)`.
const VIEWS: &[(&str, &str)] = &[
    ("list", "▤ List"),
    ("photos", "▦ Photos"),
    ("lifelist", "✦ Life list"),
];

/// The List/Photos filter chips. Per-species "rare"/"migratory" metadata has no
/// honest source today (the station records detections, not range/status), so
/// only the two real chips ship; the rest are deferred (Wave D).
const FILTERS: &[(&str, &str)] = &[("all", "All"), ("week", "This week")];

/// The controls row: view switcher (`sp-seg`) · filter chips (`sp-chips`, List
/// and Photos only) · search (a GET form, so every view is bookmarkable).
fn controls(view: &str, filter: &str, search: Option<&str>, taxon: Option<&TaxonFilter>) -> String {
    // A labelled group of navigation links (each loads a full page), with
    // aria-current marking the active view — not an ARIA tablist, which would
    // require role="tab" children and JS-controlled tabpanels.
    let mut seg = String::from(r#"<div class="sp-seg" role="group" aria-label="View">"#);
    for (key, label) in VIEWS {
        let active = if *key == view { " active" } else { "" };
        let cur = if *key == view {
            r#" aria-current="page""#
        } else {
            ""
        };
        let _ = write!(
            seg,
            r#"<a class="sp-seg-link{active}" href="/species?view={key}{tail}"{cur}>{label}</a>"#,
            tail = taxon.map(TaxonFilter::query_tail).unwrap_or_default(),
        );
    }
    seg.push_str("</div>");

    // The filter chips and search only make sense on the List/Photos grids.
    let (chips, search_form) = if view == "lifelist" {
        (String::new(), String::new())
    } else {
        let mut c = String::from(r#"<div class="sp-chips">"#);
        for (key, label) in FILTERS {
            let active = if *key == filter { " active" } else { "" };
            let _ = write!(
                c,
                r#"<a class="sp-chip{active}" href="/species?view={view}&amp;filter={key}{tail}">{label}</a>"#,
                tail = taxon.map(TaxonFilter::query_tail).unwrap_or_default(),
            );
        }
        c.push_str("</div>");
        let val = search.map(escape_html).unwrap_or_default();
        // The taxonomy filter rides along as hidden fields, or searching from
        // inside "only the woodpeckers" would quietly drop the woodpeckers.
        let keep = taxon.map_or_else(String::new, |t| {
            format!(
                r#"<input type="hidden" name="rank" value="{}"><input type="hidden" name="taxon" value="{}">"#,
                t.rank.key(),
                escape_html(&t.value),
            )
        });
        let form = format!(
            r#"<span class="sp-search"><span class="ico" aria-hidden="true">⌕</span><form method="get" action="/species" role="search"><input type="hidden" name="view" value="{view}"><input type="hidden" name="filter" value="{filter}">{keep}<input type="search" name="q" value="{val}" placeholder="Find a species…" aria-label="Find a species"></form></span>"#
        );
        (c, form)
    };
    format!(r#"<div class="sp-controls">{seg}{chips}{search_form}</div>"#)
}

/// How many order chips to offer before the row stops being browsable.
const MAX_ORDER_CHIPS: usize = 12;

/// One chip per taxonomic order among the species on this page (`G-15`).
///
/// Built from *this station's* species rather than the classifier's 11 560, so
/// a garden with 40 birds gets a handful of orders and not 75 — and every chip
/// is guaranteed to lead somewhere, because the species behind it are the ones
/// counted. Ordered by species count, so the orders an operator actually has
/// come first, and capped at [`MAX_ORDER_CHIPS`].
///
/// Empty when fewer than two orders are represented, which covers both the
/// station with no classifier label file (no taxonomy at all, so no orders) and
/// the one whose species all sit in a single order — a chip row offering the
/// only choice there is is furniture, not navigation. A separate
/// `has_taxonomy()` early-out stood here until a mutation of it survived every
/// gate: this one already subsumes it.
///
/// Order and not family: see [`crate::state::Taxon`]. Genus is reachable from
/// each species' own detail page, where "the other *Dryobates* here" is the
/// question; 2 907 genera is not a chip row.
fn order_chips(
    state: &AppState,
    species: &[birdnet_db::sqlite::SpeciesCount],
    view: &str,
    filter: &str,
    search: Option<&str>,
    active: Option<&TaxonFilter>,
) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for s in species {
        if let Some(order) = state.taxon(&s.sci_name).and_then(|t| t.order.as_deref()) {
            *counts.entry(order).or_default() += 1;
        }
    }
    if counts.len() < 2 {
        return String::new();
    }
    // Count descending, then name, so the row is stable between requests.
    let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked.truncate(MAX_ORDER_CHIPS);

    let keep = search.map_or_else(String::new, |q| format!("&amp;q={}", simple_url_encode(q)));
    let base = format!("/species?view={view}&amp;filter={filter}{keep}");
    let showing_all = active.is_none_or(|t| t.rank != Rank::Order);
    let mut out = format!(
        r#"<div class="sp-chips" role="group" aria-label="Taxonomic order"><a class="sp-chip{a}" href="{base}">All orders</a>"#,
        a = if showing_all { " active" } else { "" },
    );
    for (order, n) in ranked {
        let on =
            active.is_some_and(|t| t.rank == Rank::Order && t.value.eq_ignore_ascii_case(order));
        let _ = write!(
            out,
            r#"<a class="sp-chip{a}" href="{base}&amp;rank=order&amp;taxon={enc}">{name} <span class="bnb-meta">{n}</span></a>"#,
            a = if on { " active" } else { "" },
            enc = simple_url_encode(order),
            name = escape_html(order),
        );
    }
    out.push_str("</div>");
    out
}

/// The **List** view: `sp-count` headline + the `sp-table` (rank · avatar ·
/// 14-day sparkline · count · Avg confidence), every row a link to its detail.
fn list_view(
    state: &AppState,
    filter: &str,
    search: Option<&str>,
    taxon: Option<&TaxonFilter>,
) -> String {
    let (mut species, sparks) = state.with_db(|conn| {
        let species = search.map_or_else(
            || birdnet_db::sqlite::top_species(conn, 500).unwrap_or_default(),
            |q| birdnet_db::sqlite::search_species(conn, q, 500).unwrap_or_default(),
        );
        let sparks = birdnet_db::sqlite::species_sparklines(conn, 14).unwrap_or_default();
        (species, sparks)
    });
    if filter == "week" {
        species.retain(|s| active_this_week(sparks.get(&s.com_name)));
    }
    // Built before the filter narrows the list, so choosing a chip does not
    // make the other chips disappear.
    let chips = order_chips(state, &species, "list", filter, search, taxon);
    if let Some(t) = taxon {
        species.retain(|s| t.admits(state, &s.sci_name));
    }

    let total: i64 = species.iter().map(|s| s.count).sum();
    let mut rows = String::new();
    for (i, s) in species.iter().enumerate() {
        let color = species_color(&s.com_name);
        let spark = sparks
            .get(&s.com_name)
            .map(|d| sparkline(d, 84.0, 22.0, Some(&color)))
            .unwrap_or_default();
        let enc = simple_url_encode(&s.com_name);
        let _ = write!(
            rows,
            r#"<tr><td class="sp-rank">{rank}</td><td><a class="sp-cell" href="/species/detail?name={enc}"><span class="sp-cell-av">{av}</span><span class="sp-cell-tx"><span class="sp-nm">{name}</span><span class="sp-sci">{sci}</span></span></a></td><td>{spark}</td><td class="sp-num">{count}</td><td>{conf}</td></tr>"#,
            rank = i + 1,
            av = avatar(&s.com_name, ""),
            name = escape_html(&s.com_name),
            sci = escape_html(&s.sci_name),
            count = format_count(s.count),
            conf = conf_bar(s.avg_confidence),
        );
    }

    let count_line = species_count_line(species.len(), filter, total);
    if species.is_empty() {
        return format!("{chips}{count_line}{}", empty_note(search));
    }
    format!(
        r#"{chips}{count_line}<div class="bnb-card pad"><table class="sp-table"><thead><tr><th class="sp-rank">#</th><th>Species</th><th>14-day</th><th>Detections</th><th>Avg confidence</th></tr></thead><tbody>{rows}</tbody></table></div>"#
    )
}

/// The **Photos** view: the gallery grid (`sp-grid` of `sp-photo-card`s) with
/// Wikipedia thumbnails over the gradient banding-code fallback.
fn photos_view(
    state: &AppState,
    filter: &str,
    search: Option<&str>,
    taxon: Option<&TaxonFilter>,
) -> String {
    let (mut species, sparks) = state.with_db(|conn| {
        let species = search.map_or_else(
            || birdnet_db::sqlite::top_species(conn, 200).unwrap_or_default(),
            |q| birdnet_db::sqlite::search_species(conn, q, 200).unwrap_or_default(),
        );
        let sparks = birdnet_db::sqlite::species_sparklines(conn, 14).unwrap_or_default();
        (species, sparks)
    });
    if filter == "week" {
        species.retain(|s| active_this_week(sparks.get(&s.com_name)));
    }
    let chips = order_chips(state, &species, "photos", filter, search, taxon);
    if let Some(t) = taxon {
        species.retain(|s| t.admits(state, &s.sci_name));
    }

    let mut cards = String::new();
    for s in &species {
        let color = species_color(&s.com_name);
        let code = species_code(&s.com_name);
        let enc = simple_url_encode(&s.com_name);
        let enc_sci = simple_url_encode(&s.sci_name);
        let _ = write!(
            cards,
            r#"<a class="sp-photo-card" href="/species/detail?name={enc}"><div class="bnb-card"><div class="bnb-photo sp-photo"><div class="ga-thumb-bg" data-style="background:color-mix(in oklch, {color} 15%, var(--surface))"><span class="display ga-code" data-style="--sp:{color}">{code}</span></div><img src="/api/v2/species/image/{enc_sci}/file" alt="{name}" loading="lazy" class="ga-img" data-hide-on-error></div><div class="sp-photo-meta"><div class="nm">{name}</div><div class="sub">{count} detections</div></div></div></a>"#,
            name = escape_html(&s.com_name),
            count = format_count(s.count),
        );
    }
    let count_line = format!(
        r#"<div class="sp-count"><b>{n}</b> species{wk} · click any card for the full detail</div>"#,
        n = species.len(),
        wk = if filter == "week" {
            " · active this week"
        } else {
            ""
        },
    );
    if species.is_empty() {
        return format!("{chips}{count_line}{}", empty_note(search));
    }
    format!(r#"{chips}{count_line}<div class="sp-grid">{cards}</div>"#)
}

/// The **Life list** view: the big counters, the accumulation curve, and the
/// "New to the list" recent firsts. Every species the station has ever heard.
fn lifelist_view(state: &AppState) -> String {
    let (species_total, det_total, active_days, points, firsts) = state.with_db(|conn| {
        let species_total = birdnet_db::sqlite::species_count(conn).unwrap_or(0);
        let det_total = birdnet_db::sqlite::detection_count(conn).unwrap_or(0);
        let active_days = birdnet_db::sqlite::distinct_detection_dates(conn).map_or(0, |v| v.len());
        let first_seen = birdnet_db::sqlite::species_first_seen(conn).unwrap_or_default();
        let points = accumulation_points(&first_seen);
        let new_count = new_this_year(&first_seen);
        // Most-recent firsts: scientific-name keyed first-seen, joined to common
        // names via the top-species list (which carries both).
        let mut named: Vec<(String, String, String)> =
            birdnet_db::sqlite::top_species(conn, 10_000)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| {
                    first_seen
                        .get(&s.sci_name)
                        .map(|d| (s.com_name, s.sci_name, d.clone()))
                })
                .collect();
        named.sort_by(|a, b| b.2.cmp(&a.2));
        named.truncate(6);
        (
            species_total,
            det_total,
            active_days,
            points,
            (new_count, named),
        )
    });
    let (new_count, named) = firsts;

    let curve = super::viz::accumulation_curve(&points);
    let mut firsts_html = String::new();
    for (com, sci, date) in &named {
        let enc = simple_url_encode(com);
        let _ = write!(
            firsts_html,
            r#"<a class="sp-first-row" href="/species/detail?name={enc}">{av}<div class="sp-cell-tx"><div class="sp-nm">{name}</div><div class="sp-sci">{sci}</div></div><span class="when">{date}</span></a>"#,
            av = avatar(com, ""),
            name = escape_html(com),
            sci = escape_html(sci),
            date = escape_html(date),
        );
    }

    format!(
        r#"<div class="sp-life-head">
  <div>
    <div class="sp-life-stat">
      <div><div class="v moss">{species_total}</div><div class="l">species all-time</div></div>
      <div><div class="v">{active_days}</div><div class="l">active days</div></div>
      <div><div class="v">{new_count}</div><div class="l">new this year</div></div>
    </div>
    <p class="bnb-meta sp-life-lede">Every species your station has ever heard — {det} detections in all. The curve climbs fast at first, then each new bird gets rarer, and more exciting.</p>
  </div>
  <div class="bnb-card pad"><div class="bnb-eyebrow">Your growing list</div><div class="sd-viz">{curve}</div></div>
</div>
<div class="bnb-card pad"><div class="section-header"><div><div class="bnb-eyebrow">Most recent</div><h3>New to the list</h3></div></div><div class="sp-firsts">{firsts_html}</div></div>"#,
        det = format_count(det_total),
    )
}

/// Whether a species' 14-day sparkline shows any activity in the last 7 days.
fn active_this_week(spark: Option<&Vec<i64>>) -> bool {
    spark.is_some_and(|d| d.iter().rev().take(7).sum::<i64>() > 0)
}

/// Build the cumulative-species accumulation points (`(label, cum)`), binned by
/// month, mirroring the pre-spine life-list page.
fn accumulation_points(
    first_seen: &std::collections::HashMap<String, String>,
) -> Vec<(String, i64)> {
    let mut monthly: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for date in first_seen.values() {
        if let Some(month) = date.get(..7) {
            *monthly.entry(month.to_string()).or_default() += 1;
        }
    }
    let mut cum: i64 = 0;
    monthly
        .iter()
        .map(|(month, &c)| {
            cum += i64::from(c);
            (month.get(2..).unwrap_or(month).to_string(), cum)
        })
        .collect()
}

/// Count of species whose first-ever detection falls in the current year.
fn new_this_year(first_seen: &std::collections::HashMap<String, String>) -> usize {
    let year_prefix = super::today_date_string()
        .get(..4)
        .unwrap_or("")
        .to_string();
    if year_prefix.is_empty() {
        return 0;
    }
    first_seen
        .values()
        .filter(|d| d.starts_with(&year_prefix))
        .count()
}

/// The `sp-count` headline for the list view.
fn species_count_line(n: usize, filter: &str, total: i64) -> String {
    let scope = if filter == "week" {
        " · active this week"
    } else {
        ""
    };
    format!(
        r#"<div class="sp-count"><b>{n}</b> species{scope} · {total} detections all-time</div>"#,
        total = format_count(total),
    )
}

/// An honest empty state for a search / filter that matched nothing.
fn empty_note(search: Option<&str>) -> String {
    let what = search.map_or_else(
        || "No species match this filter yet.".to_string(),
        |q| format!("No species match “{}”.", escape_html(q)),
    );
    format!(r#"<div class="bnb-card pad bnb-meta">{what}</div>"#)
}

/// Group a count with thousands separators (e.g. `3142` → `3,142`).
fn format_count(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(char::from(*b));
    }
    if n < 0 { format!("-{out}") } else { out }
}

async fn species_detail_page(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
    headers: HeaderMap,
) -> Html<String> {
    let Some(name) = query.name else {
        return super::render_page_for_request(
            "Species",
            "<p>No species specified.</p>",
            "species",
            &headers,
        );
    };

    let com_name = name.clone();
    let sci_name = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            // Raw `detections` on purpose. This resolves a display name, not a
            // number: a species whose every detection has been rejected is gone
            // from the species list and every aggregate, but a link to its page
            // may still exist. Reading the analytic view here would render that
            // page nameless rather than empty-but-explained.
            conn.query_row(
                "SELECT Sci_Name FROM detections WHERE Com_Name = ?1 LIMIT 1",
                [&com_name],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default()
        })
    })
    .await
    .unwrap_or_default();

    let encoded = simple_url_encode(&name);
    let content = SPECIES_DETAIL_HTML
        .replace("{{species_name}}", &escape_html(&name))
        .replace("{{scientific_name}}", &escape_html(&sci_name))
        .replace("{{species_encoded}}", &encoded)
        // Skeleton placeholders (O-16) shown until the htmx swap targets load.
        .replace("{{skel_species_status}}", &super::skeletons::pill_row(3))
        .replace("{{skel_hero}}", super::skeletons::hero_card())
        .replace("{{skel_species_stats}}", &super::skeletons::stat_row(4))
        .replace("{{skel_circadian}}", &super::skeletons::hourly_bars(24))
        .replace("{{skel_trend}}", super::skeletons::trend_line())
        .replace("{{skel_detections}}", &super::skeletons::list_rows(5));
    super::render_page_for_request(&name, &content, "species", &headers)
}

async fn species_summary_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_summary(conn, &name))
    })
    .await;

    match result {
        Ok(Ok(Some(summary))) => {
            let conf_pct = summary.avg_confidence * 100.0;
            let html = format!(
                r#"<div class="stat-card"><div class="value">{c}</div><div class="label">Detections</div></div>
<div class="stat-card"><div class="value">{conf_pct:.0}%</div><div class="label">Avg Confidence</div></div>
<div class="stat-card"><div class="value">{f}</div><div class="label">First Seen</div></div>
<div class="stat-card"><div class="value">{l}</div><div class="label">Last Seen</div></div>"#,
                c = summary.count,
                f = escape_html(&summary.first_seen),
                l = escape_html(&summary.last_seen),
            );
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Ok(None)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p class="spp-muted">Species not found.</p>"#.to_string(),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading summary</p>".to_string(),
        ),
    }
}

async fn species_hourly_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_hourly_activity(conn, &name))
    })
    .await;
    match result {
        Ok(Ok(hours)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_hourly_chart(&hours),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

async fn species_daily_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_daily_counts(conn, &name, 14))
    })
    .await;
    match result {
        Ok(Ok(days)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            render_daily_chart(&days),
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading chart</p>".to_string(),
        ),
    }
}

async fn species_detections_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::detections_by_species(conn, &name, 20))
    })
    .await;

    match result {
        Ok(Ok(detections)) => {
            if detections.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="spp-muted">No detections found.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Confidence</th><th>Time</th><th>Date</th></tr></thead><tbody>",
            );
            for d in &detections {
                let conf_pct = d.confidence * 100.0;
                let cls = if conf_pct >= 80.0 {
                    "high"
                } else if conf_pct >= 50.0 {
                    "mid"
                } else {
                    "low"
                };
                let _ = write!(
                    html,
                    r#"<tr><td><span class="conf {cls}">{conf_pct:.0}%</span></td><td>{t}</td><td>{dt}</td></tr>"#,
                    t = escape_html(&d.time),
                    dt = escape_html(&d.date),
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading detections</p>".to_string(),
        ),
    }
}

async fn species_info_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };

    let com_name = name.clone();
    let state_clone = state.clone();
    let sci_name = tokio::task::spawn_blocking(move || {
        state_clone.with_db(|conn| {
            conn.query_row(
                "SELECT Sci_Name FROM detections WHERE Com_Name = ?1 LIMIT 1",
                [&com_name],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_default()
        })
    })
    .await
    .unwrap_or_default();

    let mut html = String::new();

    // Species photos are cached by *scientific* name so the gallery,
    // species-detail, and detection-detail pages share one entry per bird
    // (falling back to the common name only if the scientific lookup failed).
    let img_key = if sci_name.is_empty() {
        name.clone()
    } else {
        sci_name.clone()
    };

    // The /file image route is cache-only, so warm this species' photo in the
    // background (non-blocking) on first view — a later view then shows it.
    if let Some(cache) = state.image_cache()
        && !img_key.is_empty()
        && cache.get_cached(&img_key).is_none()
    {
        let key_bg = img_key.clone();
        tokio::spawn(async move {
            let _ = cache.get_image(&key_bg).await;
        });
    }

    if let Some(cache) = state.image_cache()
        && let Some(image) = cache.get_cached(&img_key)
    {
        if image.cached_path.is_some() {
            let enc = simple_url_encode(&img_key);
            let _ = write!(
                html,
                r#"<img src="/api/v2/species/image/{enc}/file" alt="{alt}" class="spp-info-img" />"#,
                alt = escape_html(&name),
            );
        }
        if let Some(desc) = &image.description {
            let _ = write!(html, r#"<p class="spp-desc">{}</p>"#, escape_html(desc));
        }
        if let Some(url) = &image.wiki_url {
            let _ = write!(
                html,
                r#"<p><a href="{}" target="_blank" rel="noopener">View on Wikipedia</a></p>"#,
                escape_html(url),
            );
        }
    }

    if html.is_empty() {
        html = format!(
            r#"<p class="spp-muted">No additional info for <em>{}</em>.</p>
<p class="spp-muted-sm">Enable <code>--image-cache-dir</code> to fetch species images.</p>"#,
            escape_html(&name),
        );
    }

    // The taxonomy line, and the way back to "everything else like this"
    // (`G-15`). Only what the label file states, and only when it states it.
    html.push_str(&taxonomy_line(&state, &sci_name));

    // Add species info links (eBird/AllAboutBirds) — always shown
    let info_site = state.info_site();
    if info_site != "none" {
        let encoded_com = simple_url_encode(&name);
        match info_site {
            "allaboutbirds" => {
                let _ = write!(
                    html,
                    r#"<p class="spp-mt"><a href="https://www.allaboutbirds.org/guide/{encoded_com}" target="_blank" rel="noopener" class="spp-link">View on All About Birds</a></p>"#,
                );
            }
            _ => {
                // Default to eBird.
                html.push_str(&ebird_link(
                    state.ebird_species_code(&sci_name),
                    if sci_name.is_empty() {
                        &name
                    } else {
                        &sci_name
                    },
                ));
            }
        }
    }

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// The taxonomy line of the species panel: class · order · genus, each a link
/// back to the species list filtered to it (`G-15`).
///
/// Empty when the station has no classifier label file, or has one that says
/// nothing about this species. There is no family here because the label file
/// has no family column — see [`crate::state::Taxon`] — and inventing one from
/// the genus would put a guess beside two stated facts.
fn taxonomy_line(state: &AppState, scientific_name: &str) -> String {
    if scientific_name.is_empty() {
        return String::new();
    }
    let Some(taxon) = state.taxon(scientific_name) else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    for rank in [Rank::Class, Rank::Order, Rank::Genus] {
        if let Some(value) = rank.of(taxon) {
            parts.push(format!(
                r#"<a href="/species?view=list&amp;filter=all&amp;rank={key}&amp;taxon={enc}" class="spp-link">{name}</a>"#,
                key = rank.key(),
                enc = simple_url_encode(value),
                name = escape_html(value),
            ));
        }
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(r#"<p class="spp-mt bnb-meta">{}</p>"#, parts.join(" · "))
}

/// The "View on eBird" line of the species panel (NP-1).
///
/// eBird's species pages are keyed on the six-letter eBird species code
/// (`https://ebird.org/species/zothaw`), not on the scientific name: the link
/// this replaced put the name in the path and every one of them 404'd. The
/// code comes from the geomodel's label file, so a station without that file
/// has none — and then this says so, with the flag that supplies it, rather
/// than linking to a page that is not there.
fn ebird_link(code: Option<&str>, shown_name: &str) -> String {
    code.map_or_else(
        || {
            format!(
                r#"<p class="spp-muted-sm">No eBird link: this station has no eBird species code for <em>{}</em>. The geomodel's label file supplies the codes — see <code>--metadata-labels</code>.</p>"#,
                escape_html(shown_name),
            )
        },
        |code| {
            format!(
                r#"<p class="spp-mt"><a href="https://ebird.org/species/{}" target="_blank" rel="noopener" class="spp-link">View on eBird</a></p>"#,
                simple_url_encode(code),
            )
        },
    )
}

/// HTMX partial: status pills (detection count, first/last heard, mean
/// confidence) shown under the species headline on the detail page.
async fn species_status_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            String::new(),
        );
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::species_summary(conn, &name))
    })
    .await;

    let html = match result {
        Ok(Ok(Some(s))) => {
            let conf_pct = s.avg_confidence * 100.0;
            format!(
                r#"<span class="bnb-pill moss"><span class="bnb-dot"></span> {count} detections</span>
<span class="bnb-pill">First heard {first}</span>
<span class="bnb-pill">Last heard {last}</span>
<span class="bnb-pill">avg {conf_pct:.0}% confidence</span>"#,
                count = s.count,
                first = escape_html(&s.first_seen),
                last = escape_html(&s.last_seen),
            )
        }
        Ok(Ok(None)) => r#"<span class="bnb-pill">No detections yet</span>"#.to_string(),
        _ => String::new(),
    };
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: "best detection" hero card — the highest-confidence clip for
/// the species, with the reference photo, spectrogram, and an audio scrubber.
async fn species_hero_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            String::new(),
        );
    };

    let lookup_name = name.clone();
    let state_clone = state.clone();
    let best = tokio::task::spawn_blocking(move || {
        state_clone.with_db(|conn| {
            // Shares `CLIP_AVAILABLE` with every other play-button surface, so
            // this page cannot end up offering audio the Recordings browser
            // knows has been reclaimed.
            let sql = format!(
                "SELECT Date, Time, Confidence, File_Name \
                 FROM detections_analytic \
                 WHERE Com_Name = ?1 AND {clip} \
                 ORDER BY Confidence DESC LIMIT 1",
                clip = birdnet_db::sqlite::CLIP_AVAILABLE,
            );
            conn.query_row(&sql, [&lookup_name], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .ok()
        })
    })
    .await
    .ok()
    .flatten();

    let Some((date, time, conf, file_name)) = best else {
        let html = r#"<div class="bnb-eyebrow spp-mb8">Best detection</div>
<div class="bnb-photo spp-photo-empty" data-caption="no clip yet"></div>
<p class="bnb-meta spp-mt8">No recording captured for this species yet.</p>"#;
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            html.to_string(),
        );
    };

    let basename = std::path::Path::new(&file_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or(file_name);
    let safe_file = escape_html(&basename);
    let time_short = time.get(0..5).unwrap_or(&time);
    let conf_pct = conf * 100.0;

    // The hero is the *recording* — the spectrogram and audio of the loudest
    // call. The species reference photo lives in the "About this species" card
    // below, so it isn't shown (cropped, and a second time) on the same page.
    let html = format!(
        r#"<div class="bnb-eyebrow spp-mb8">Best detection</div>
<img src="/api/v2/spectrogram/{safe_file}" alt="Spectrogram of the loudest detected call" data-hide-on-error class="spp-spectrogram" />
<audio controls preload="metadata" class="spp-audio"><source src="/api/v2/recordings/{safe_file}" type="audio/wav"></audio>
<div class="bnb-meta mono spp-mt8">{conf_pct:.0}% confidence · {date} {time_short}</div>"#,
    );

    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

/// HTMX partial: companion species (co-occurrence).
async fn species_companions_partial(
    State(state): State<AppState>,
    Query(query): Query<SpeciesQuery>,
) -> impl axum::response::IntoResponse {
    let Some(name) = query.name else {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>No species specified.</p>".to_string(),
        );
    };

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| birdnet_db::sqlite::companion_species(conn, &name, 30, 10))
    })
    .await;

    match result {
        Ok(Ok(companions)) => {
            if companions.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="spp-muted">No companion species data yet.</p>"#.to_string(),
                );
            }
            let mut html = String::from(
                r"<table><thead><tr><th>Companion</th><th>Co-occurrence Days</th></tr></thead><tbody>",
            );
            for c in &companions {
                let enc = simple_url_encode(&c.companion);
                let _ = write!(
                    html,
                    r#"<tr><td><a href="/species/detail?name={enc}" class="spp-inherit">{name}</a></td><td>{count}</td></tr>"#,
                    name = escape_html(&c.companion),
                    count = c.shared_days,
                );
            }
            html.push_str("</tbody></table>");
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/html")],
            "<p>Error loading companion species</p>".to_string(),
        ),
    }
}

// ── browsing by taxonomic rank (G-15) ───────────────────────────────────
//
// The gates below were written against the pre-feature page and observed
// failing: `HomeParams` had no `rank`/`taxon`, `AppState` no taxonomy, and the
// two grid views no way to narrow to a rank, so each of these asserts
// something that did not exist. Where a gate could be satisfied by a filter
// that simply drops everything, or by a chip row that always renders, the
// counterpart is here too.
#[cfg(test)]
mod taxonomy_browsing_tests {
    use super::*;
    use crate::state::Taxon;
    use birdnet_db::sqlite::SpeciesCount;

    fn taxon(class: &str, order: &str, genus: &str) -> Taxon {
        Taxon {
            class: Some(class.to_owned()),
            order: Some(order.to_owned()),
            genus: Some(genus.to_owned()),
        }
    }

    /// Four species over three orders, so a filter that keeps everything and
    /// one that keeps nothing both fail.
    fn state_with_taxonomy() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")).with_taxonomy([
            (
                "Dryobates villosus",
                taxon("Aves", "Piciformes", "Dryobates"),
            ),
            (
                "Dryobates pubescens",
                taxon("Aves", "Piciformes", "Dryobates"),
            ),
            ("Strix varia", taxon("Aves", "Strigiformes", "Strix")),
            (
                "Tettigonia viridissima",
                taxon("Insecta", "Orthoptera", "Tettigonia"),
            ),
        ])
    }

    fn species() -> Vec<SpeciesCount> {
        [
            ("Hairy Woodpecker", "Dryobates villosus"),
            ("Downy Woodpecker", "Dryobates pubescens"),
            ("Barred Owl", "Strix varia"),
            ("Great Green Bush-Cricket", "Tettigonia viridissima"),
        ]
        .into_iter()
        .map(|(com, sci)| SpeciesCount {
            com_name: com.to_owned(),
            sci_name: sci.to_owned(),
            count: 1,
            avg_confidence: 0.9,
        })
        .collect()
    }

    fn params(rank: Option<&str>, taxon: Option<&str>) -> HomeParams {
        HomeParams {
            view: None,
            filter: None,
            q: None,
            rank: rank.map(str::to_owned),
            taxon: taxon.map(str::to_owned),
        }
    }

    #[test]
    fn a_rank_and_a_value_select_exactly_the_species_at_that_rank() {
        let state = state_with_taxonomy();
        for (rank, value, expected) in [
            (
                "order",
                "Piciformes",
                vec!["Dryobates villosus", "Dryobates pubescens"],
            ),
            ("order", "Strigiformes", vec!["Strix varia"]),
            (
                "genus",
                "Dryobates",
                vec!["Dryobates villosus", "Dryobates pubescens"],
            ),
            ("class", "Insecta", vec!["Tettigonia viridissima"]),
            (
                "class",
                "Aves",
                vec!["Dryobates villosus", "Dryobates pubescens", "Strix varia"],
            ),
        ] {
            let filter = TaxonFilter::from_params(&params(Some(rank), Some(value)))
                .unwrap_or_else(|| panic!("{rank}={value} must parse"));
            let kept: Vec<&str> = species()
                .iter()
                .filter(|s| filter.admits(&state, &s.sci_name))
                .map(|s| s.sci_name.clone())
                .map(|s| Box::leak(s.into_boxed_str()) as &str)
                .collect();
            assert_eq!(kept, expected, "for {rank}={value}");
        }
    }

    /// The value is matched case-insensitively — a chip's own link is exact,
    /// but a bookmarked or hand-typed URL is not.
    #[test]
    fn the_value_is_matched_case_insensitively() {
        let state = state_with_taxonomy();
        let filter = TaxonFilter::from_params(&params(Some("order"), Some("piciFORMES"))).unwrap();
        assert!(filter.admits(&state, "Dryobates villosus"));
        assert!(!filter.admits(&state, "Strix varia"));
    }

    /// A rank the page does not know, a missing half of the pair, or a blank
    /// value is **no filter**, not an empty page. A station whose URL carries
    /// `rank=family` must still show its species.
    #[test]
    fn an_unusable_rank_or_value_is_no_filter_rather_than_an_empty_list() {
        for (rank, value) in [
            (Some("family"), Some("Picidae")),
            (Some("order"), None),
            (None, Some("Piciformes")),
            (Some("order"), Some("   ")),
            (None, None),
        ] {
            assert!(
                TaxonFilter::from_params(&params(rank, value)).is_none(),
                "rank={rank:?} taxon={value:?} must not become a filter"
            );
        }
    }

    /// A species the station has no taxonomy for does not match any rank. The
    /// counterpart to the selection gate: without this, a filter that returned
    /// `true` on a missing taxon would still pass the tests above for every
    /// species that *does* have one.
    #[test]
    fn a_species_with_no_taxonomy_matches_no_rank() {
        let state = state_with_taxonomy();
        let filter = TaxonFilter::from_params(&params(Some("order"), Some("Piciformes"))).unwrap();
        assert!(!filter.admits(&state, "Corvus corax"));
    }

    /// The chips are built from the species on the page, in count order, and
    /// the active one is marked.
    #[test]
    fn the_chip_row_offers_the_orders_this_station_actually_has() {
        let state = state_with_taxonomy();
        let html = order_chips(&state, &species(), "list", "all", None, None);

        assert!(html.contains("Piciformes"), "{html}");
        assert!(html.contains("Strigiformes"), "{html}");
        assert!(html.contains("Orthoptera"), "{html}");
        assert!(
            !html.contains("Passeriformes"),
            "only the orders on the page, not the classifier's 75: {html}"
        );
        // Two woodpeckers put Piciformes first; the other two tie and sort by
        // name, so Orthoptera precedes Strigiformes.
        let pos = |needle: &str| {
            html.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {html}"))
        };
        assert!(pos("Piciformes") < pos("Orthoptera"), "{html}");
        assert!(pos("Orthoptera") < pos("Strigiformes"), "{html}");
        assert!(
            html.contains(r#"<a class="sp-chip active" href="/species?view=list&amp;filter=all">All orders</a>"#),
            "with no filter chosen, All orders is the active chip: {html}"
        );

        let active = TaxonFilter::from_params(&params(Some("order"), Some("Piciformes"))).unwrap();
        let chosen = order_chips(&state, &species(), "list", "all", None, Some(&active));
        assert!(
            chosen.contains(r#"&amp;rank=order&amp;taxon=Strigiformes">Strigiformes"#),
            "the other orders stay reachable once one is chosen: {chosen}"
        );
        assert!(
            !chosen.contains(r#"<a class="sp-chip active" href="/species?view=list&amp;filter=all">All orders</a>"#),
            "All orders is no longer the active chip: {chosen}"
        );
    }

    /// No taxonomy, or only one order, means no chip row: a control offering
    /// the only choice there is is furniture.
    #[test]
    fn there_is_no_chip_row_without_a_choice_to_make() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let bare = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        assert_eq!(
            order_chips(&bare, &species(), "list", "all", None, None),
            "",
            "no taxonomy means no orders, so no row"
        );

        let state = state_with_taxonomy();
        let one_order: Vec<SpeciesCount> = species()
            .into_iter()
            .filter(|s| s.sci_name.starts_with("Dryobates"))
            .collect();
        assert_eq!(
            order_chips(&state, &one_order, "list", "all", None, None),
            "",
            "both woodpeckers are Piciformes, so there is nothing to choose"
        );
    }

    /// A search term survives a chip click, and the chosen order survives a
    /// search — each used to be dropped by the other's link.
    #[test]
    fn a_search_and_a_chosen_order_each_survive_the_other() {
        let state = state_with_taxonomy();
        let html = order_chips(&state, &species(), "list", "all", Some("wood pecker"), None);
        assert!(
            html.contains("&amp;q=wood%20pecker&amp;rank=order"),
            "a chip link keeps the search: {html}"
        );

        let active = TaxonFilter::from_params(&params(Some("order"), Some("Piciformes"))).unwrap();
        let controls = controls("list", "all", Some("owl"), Some(&active));
        assert!(
            controls.contains(r#"<input type="hidden" name="rank" value="order">"#)
                && controls.contains(r#"<input type="hidden" name="taxon" value="Piciformes">"#),
            "the search form keeps the order: {controls}"
        );
        assert!(
            controls.contains("/species?view=photos&amp;rank=order&amp;taxon=Piciformes"),
            "so does the view switcher: {controls}"
        );
    }

    /// The detail panel's taxonomy line links each rank back to the list, and
    /// says nothing at all when the station has no taxonomy for the species.
    #[test]
    fn the_detail_panel_links_each_rank_back_to_the_list() {
        let state = state_with_taxonomy();
        let line = taxonomy_line(&state, "Dryobates villosus");
        for (rank, value) in [
            ("class", "Aves"),
            ("order", "Piciformes"),
            ("genus", "Dryobates"),
        ] {
            assert!(
                line.contains(&format!("rank={rank}&amp;taxon={value}")),
                "{rank} must link to the filtered list: {line}"
            );
        }
        assert!(
            !line.contains("family"),
            "the label file states no family, so the panel must not show one: {line}"
        );
        assert_eq!(taxonomy_line(&state, "Corvus corax"), "");
        assert_eq!(taxonomy_line(&state, ""), "");
    }
}
