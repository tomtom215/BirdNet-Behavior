//! "Why isn't this bird heard here?" — the page that explains the station's
//! range filter (parity row P-3).
//!
//! The station asks a range model which species occur around its location in
//! each week of the year, and holds back any it scores below `SF_THRESH`. A
//! bird that is never heard is, sometimes, a bird the filter is dropping, and
//! until this page nothing said so: the filter is on a different thread, behind
//! a model file, and reports only a count to Prometheus.
//!
//! The page takes a name, finds the species the range model scores that match
//! it, and says in a sentence — with the model's real number and the real
//! threshold — whether the station admits each. Beneath, it lists the species
//! the station does admit this week. Every number is the filter's own decision
//! restated by [`crate::occurrence`]; nothing here recomputes it.
//!
//! When there is nothing to show — no location, no range model, no detection
//! daemon in this process, a model that would not run — the page says which, in
//! words. An empty list would read as "nothing occurs here".

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::occurrence::{
    DEFAULT_LIMIT, MODEL_WEEKS, OccurrenceReport, Selection, Unavailable, Verdict, parse_count,
    parse_week, percent_text, select, sentence,
};
use crate::state::AppState;

/// Most species the lookup shows in full before asking for a narrower name.
const MOST_ANSWERS: usize = 8;

/// The page's address.
pub const PATH: &str = "/species/occurrence";

/// Mount the page.
pub fn router() -> Router<AppState> {
    Router::new().route(PATH, get(occurrence_page))
}

/// Query for the page. Text throughout: a malformed value is explained on the
/// page, not answered with a bare `400`.
#[derive(Debug, Default, Deserialize)]
struct PageQuery {
    q: Option<String>,
    week: Option<String>,
    offset: Option<String>,
}

/// The twelve months, for the week selector.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// What a week of the model's 48-week year is called in the selector: its
/// number, its month, and which of that month's four it is. The model's year is
/// four weeks to a month by construction, so this is the model's own arithmetic,
/// not a calendar's.
fn week_label(week: u32) -> String {
    let month = MONTHS[((week - 1) / 4) as usize];
    format!("Week {week} · {month}, week {}", (week - 1) % 4 + 1)
}

/// The same week in a sentence: "week 22 (the second week of June)".
fn week_prose(week: u32) -> String {
    const MONTHS_FULL: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const ORDINALS: [&str; 4] = ["first", "second", "third", "fourth"];
    format!(
        "week {week} (the {} week of {})",
        ORDINALS[((week - 1) % 4) as usize],
        MONTHS_FULL[((week - 1) / 4) as usize]
    )
}

/// The `<select>` of weeks, `current` and `chosen` marked.
fn week_select(chosen: u32, current: u32) -> String {
    let mut out = String::from(r#"<select id="occ-week" name="week">"#);
    for week in 1..=MODEL_WEEKS {
        let selected = if week == chosen { " selected" } else { "" };
        let now = if week == current { " (this week)" } else { "" };
        let _ = write!(
            out,
            r#"<option value="{week}"{selected}>{}{now}</option>"#,
            week_label(week)
        );
    }
    out.push_str("</select>");
    out
}

async fn occurrence_page(
    State(state): State<AppState>,
    Query(params): Query<PageQuery>,
    headers: HeaderMap,
) -> Html<String> {
    let current = crate::routes::species::current_model_week();
    let mut notes: Vec<String> = Vec::new();
    let week = match parse_week(params.week.as_deref()) {
        Ok(w) => w.unwrap_or(current),
        Err(e) => {
            notes.push(e);
            current
        }
    };
    let offset = match parse_count("offset", params.offset.as_deref(), 0, usize::MAX) {
        Ok(n) => n.unwrap_or(0),
        Err(e) => {
            notes.push(e);
            0
        }
    };
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_owned);

    let body = match state.occurrence_source() {
        None => unavailable_card(
            "No range filter to ask",
            "This process is not running the detection daemon, so there is no range filter to ask.",
        ),
        Some(source) => {
            let report = state.run_blocking(move |_| source.report(week)).await;
            match report {
                Err(reason) => unavailable_for(&reason),
                Ok(report) => {
                    let names = state.names_handle();
                    let also: std::collections::HashSet<String> =
                        super::name_search::also_species(&names, query.as_deref())
                            .into_iter()
                            .collect();
                    results(&report, query.as_deref(), offset, &names, &also, &notes)
                }
            }
        }
    };

    let form = lookup_form(query.as_deref(), week, current);
    let content = format!(
        r#"<div class="page-head" data-screen-label="Range head">
  <div>
    <div class="bnb-eyebrow"><a href="/species">Species</a></div>
    <h1 class="display sp-h1">Why isn't a bird heard here?</h1>
    <p class="bnb-meta sp-lede">The station asks a range model which birds occur around where it is in each week of the year, and holds back any it scores below a threshold. Type a bird's name to see where it stands.</p>
  </div>
</div>
{form}
{body}"#
    );
    super::render_page_for_request(
        "Why isn't a bird heard here?",
        &content,
        "species",
        &headers,
    )
}

/// The lookup form: a GET, so every answer is a link that can be shared.
fn lookup_form(query: Option<&str>, week: u32, current: u32) -> String {
    let value = query.map(escape_html).unwrap_or_default();
    let weeks = week_select(week, current);
    format!(
        r#"<form class="occ-form" method="get" action="{PATH}" role="search" aria-label="Look up a bird">
  <div class="occ-field occ-grow"><label for="occ-q">Bird</label><input id="occ-q" type="search" name="q" value="{value}" placeholder="Common or scientific name" autocomplete="off"></div>
  <div class="occ-field"><label for="occ-week">Week of the year</label>{weeks}</div>
  <button class="bnb-btn primary" type="submit">Look up</button>
</form>"#
    )
}

/// A card that says why there is nothing to list.
fn unavailable_card(title: &str, why: &str) -> String {
    format!(
        r#"<section class="bnb-card pad occ-card" aria-labelledby="occ-none-h">
  <h2 class="occ-h" id="occ-none-h">{}</h2>
  <p class="bnb-meta">{}</p>
</section>"#,
        escape_html(title),
        escape_html(why)
    )
}

/// The card for each reason the filter cannot be read.
fn unavailable_for(reason: &Unavailable) -> String {
    match reason {
        Unavailable::NoLocation => unavailable_card(
            "This station has no location",
            "The range model is asked about a place, and this station has no latitude and longitude, so the filter is not removing any bird for being out of range. Set the location in Settings and restart the station, and this page will say what the filter does there.",
        ),
        Unavailable::NoModel => unavailable_card(
            "The range filter is off",
            "No range model is loaded, so the station does not filter species by where it is: every bird the classifier knows is a candidate, wherever it is. A bird that is never heard is not being held back by location. Installing the range model turns the filter on.",
        ),
        Unavailable::Failed(detail) => {
            tracing::error!(error = %detail, "the range model could not be run for the occurrence page");
            unavailable_card(
                "The range model could not be run",
                "The model file is there but would not give an answer. The station's log has the reason.",
            )
        }
    }
}

/// The answer and the list, for a report that exists.
fn results(
    report: &OccurrenceReport,
    query: Option<&str>,
    offset: usize,
    names: &birdnet_core::i18n::NameLocalizer,
    also: &std::collections::HashSet<String>,
    notes: &[String],
) -> String {
    let mut out = String::new();
    for note in notes {
        let _ = write!(
            out,
            r#"<p class="bnb-meta occ-note" role="status">{}</p>"#,
            escape_html(note)
        );
    }
    let threshold = percent_text(report.threshold);
    let place = format!(
        "{:.1}°{}, {:.1}°{}",
        report.latitude.abs(),
        if report.latitude < 0.0 { "S" } else { "N" },
        report.longitude.abs(),
        if report.longitude < 0.0 { "W" } else { "E" },
    );
    let _ = write!(
        out,
        r#"<p class="bnb-meta occ-basis">Around {place} in {week}, the range model scores {scored} species. This station admits a species when the model gives it {threshold} or more (or you have put it on the Always allowed list) and your include and exclude lists allow it.</p>"#,
        week = week_prose(report.week),
        scored = crate::routes::pages::group_thousands(
            i64::try_from(report.species.len()).unwrap_or(i64::MAX)
        ),
    );

    if let Some(q) = query {
        let picked = select(
            report,
            &Selection {
                q: Some(q.to_owned()),
                admitted: None,
            },
            also,
        );
        let _ = write!(
            out,
            r#"<section class="occ-answer" aria-labelledby="occ-answer-h"><h2 class="occ-h" id="occ-answer-h">“{}”</h2>"#,
            escape_html(q)
        );
        if picked.species.is_empty() {
            let _ = write!(
                out,
                r#"<div class="empty-state"><p class="display es-h">No bird by that name.</p><p class="bnb-meta es-sub">The range model scores no species whose common or scientific name contains “{}”. Try a shorter piece of the name, or the scientific name.</p></div>"#,
                escape_html(q)
            );
        } else {
            for score in picked.species.iter().take(MOST_ANSWERS) {
                out.push_str(&answer_card(score, report, names));
            }
            if picked.matched > MOST_ANSWERS {
                let _ = write!(
                    out,
                    r#"<p class="bnb-meta">{} more species match; type more of the name to narrow it.</p>"#,
                    picked.matched - MOST_ANSWERS
                );
            }
        }
        out.push_str("</section>");
    }

    out.push_str(&admitted_list(report, query, offset, names));
    out
}

/// One species' answer: its names, whether it is allowed, and why, in a sentence.
fn answer_card(
    score: &birdnet_core::inference::species_filter::OccurrenceScore,
    report: &OccurrenceReport,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let verdict = Verdict::of(score, report.threshold);
    let (pill, pill_class) = if verdict.is_admitted() {
        ("Allowed", "bnb-pill moss")
    } else {
        ("Held back", "bnb-pill dawn")
    };
    format!(
        r#"<article class="bnb-card pad occ-card">
  <div class="occ-card-head"><h3 class="occ-name">{name}</h3><span class="{pill_class}">{pill}</span></div>
  <p class="bnb-meta occ-sci"><i>{sci}</i></p>
  <p class="occ-why">{why}</p>
  <dl class="occ-facts"><div><dt>Range model, this week</dt><dd>{chance}</dd></div><div><dt>Station admits at</dt><dd>{threshold}</dd></div></dl>
</article>"#,
        name = escape_html(names.name(&score.scientific_name, &score.common_name)),
        sci = escape_html(&score.scientific_name),
        why = escape_html(&sentence(score, report)),
        chance = percent_text(score.probability),
        threshold = percent_text(report.threshold),
    )
}

/// The species the station admits, a page at a time.
fn admitted_list(
    report: &OccurrenceReport,
    query: Option<&str>,
    offset: usize,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let picked = select(
        report,
        &Selection {
            q: None,
            admitted: Some(true),
        },
        &std::collections::HashSet::new(),
    );
    let mut out = format!(
        r#"<section class="occ-admitted" aria-labelledby="occ-admitted-h"><h2 class="occ-h" id="occ-admitted-h">Birds this station admits in {}</h2><p class="bnb-meta occ-count">{} of the {} species the range model scores.</p>"#,
        week_prose(report.week),
        crate::routes::pages::group_thousands(i64::try_from(picked.matched).unwrap_or(i64::MAX)),
        crate::routes::pages::group_thousands(
            i64::try_from(report.species.len()).unwrap_or(i64::MAX)
        ),
    );
    if picked.species.is_empty() {
        out.push_str(
            r#"<div class="empty-state"><p class="display es-h">The filter admits no species this week.</p><p class="bnb-meta es-sub">Every bird the model scores is under the threshold, so the station records nothing. The threshold is Species Frequency Threshold in Settings; lowering it admits more.</p></div></section>"#,
        );
        return out;
    }
    out.push_str(r#"<ul class="occ-list">"#);
    for score in picked.species.iter().skip(offset).take(DEFAULT_LIMIT) {
        let _ = write!(
            out,
            r#"<li><span class="occ-li-name">{name}<span class="bnb-meta"> <i>{sci}</i></span></span><span class="bnb-meta occ-li-pct">{pct}</span></li>"#,
            name = escape_html(names.name(&score.scientific_name, &score.common_name)),
            sci = escape_html(&score.scientific_name),
            pct = percent_text(score.probability),
        );
    }
    out.push_str("</ul>");
    let shown_to = offset + DEFAULT_LIMIT;
    if shown_to < picked.matched {
        let q = query
            .map(|q| format!("&amp;q={}", simple_url_encode(q)))
            .unwrap_or_default();
        let _ = write!(
            out,
            r#"<p><a class="bnb-btn" href="{PATH}?week={week}{q}&amp;offset={shown_to}#occ-admitted-h">Show the next {DEFAULT_LIMIT}</a></p>"#,
            week = report.week,
        );
    }
    out.push_str("</section>");
    out
}

/// A pill linking a species page to its range answer, when the filter would
/// hold the species back this week; empty when it would not, or cannot say.
///
/// Blocking (it may run the model): call it from `run_blocking`.
pub fn held_back_pill(state: &AppState, common_name: &str) -> String {
    let Some(source) = state.occurrence_source() else {
        return String::new();
    };
    let Ok(report) = source.report(crate::routes::species::current_model_week()) else {
        return String::new();
    };
    let wanted = common_name.trim().to_lowercase();
    let Some(score) = report.species.iter().find(|s| {
        s.common_name.to_lowercase() == wanted || s.scientific_name.to_lowercase() == wanted
    }) else {
        return String::new();
    };
    if Verdict::of(score, report.threshold).is_admitted() {
        return String::new();
    }
    format!(
        r#"<a class="bnb-pill dawn" href="{PATH}?q={}">The range filter holds this species back this week. Why?</a>"#,
        simple_url_encode(&score.scientific_name)
    )
}
