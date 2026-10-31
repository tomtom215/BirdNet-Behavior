//! Behavioral analytics HTMX partials (requires duckdb-behavioral extension).

// `analytics_config_partial` uses `write!` macro on a String regardless of
// whether the analytics feature is enabled — the surrounding `if let Some(...)
// = ext_status` branch is unreachable without the feature, but the macro still
// needs the trait in scope to compile.
use std::fmt::Write as _;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::{Router, routing::get};
#[cfg(feature = "analytics")]
use rusqlite::OptionalExtension as _;

use super::{ANALYTICS_PAGE_HTML, ANALYTICS_SEQUENCES_HTML, escape_html};
use crate::state::AppState;

/// Mount the behavioral analytics page and HTMX partial routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/pages/analytics-sessions", get(analytics_sessions_partial))
        .route(
            "/pages/analytics-retention",
            get(analytics_retention_partial),
        )
        .route("/pages/analytics-next", get(analytics_next_partial))
        .route("/pages/analytics-answers", get(analytics_answers_partial))
        .route("/pages/analytics-assembly", get(analytics_assembly_partial))
        .route(
            "/pages/analytics-dawn-sequence",
            get(analytics_dawn_sequence_partial),
        )
        .route("/pages/analytics-config", get(analytics_config_partial))
}

/// The analytics status pill both How tabs carry at the right of their heading.
const STATUS_PILL: &str = r#"<div class="bnb-card pad an-status" hx-get="/pages/analytics-status" hx-trigger="load" hx-swap="innerHTML"><div class="value">--</div><div class="label">Analytics Status</div></div>"#;

/// The "Visits" tab (`?tab=behavior`): how long the birds sing in one go and who keeps coming
/// back, rendered for embedding by `homes::patterns`. The engine's ordered questions are on the
/// "Sequences" tab beside it.
pub(super) fn content() -> String {
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Analytics),
        title: "How your birds <em>behave</em>",
        lead: "The research tier: every term explained in plain English.",
        lede: "These come from the behavioural analytics engine; you never need them to enjoy your station, but they're here when you're curious. This tab is about visits: how long the birds sing in one go, and who keeps coming back. Who follows whom is on <a href=\"/patterns?tab=sequences\">Sequences</a>.",
        aside: STATUS_PILL,
    });
    let gap = birdnet_behavioral::types::SessionizeParams::default()
        .gap_minutes
        .to_string();
    ANALYTICS_PAGE_HTML
        .replace("{{page_head}}", &head)
        .replace("{{session_gap}}", &gap)
}

/// The "Sequences" tab (`?tab=sequences`): the order things happen in: who opens the dawn and how
/// fast the chorus builds, who is heard next, who answers whom. Rendered for embedding by
/// `homes::patterns`.
pub(super) fn sequences_content() -> String {
    let head = super::frame::page_head(&super::frame::Head {
        eyebrow: "Patterns",
        help: &super::help::help_link(super::help::Topic::Analytics),
        title: "Who follows <em>whom</em>",
        lead: "The order things happen in.",
        lede: "Who opens your dawn and how fast the chorus builds, which bird is heard next, and who answers whom. These come from the behavioural analytics engine; you never need them to enjoy your station. How long the birds sing in one go is on <a href=\"/patterns?tab=behavior\">Visits</a>.",
        aside: STATUS_PILL,
    });
    ANALYTICS_SEQUENCES_HTML.replace("{{page_head}}", &head)
}

/// The Method panel of a Behavior card whose numbers come from the engine's functions.
///
/// `computed` names them (a comma-separated list for a card that uses several), `version` is the
/// extension's, as the engine reported it while it answered. Every statement a caller puts in
/// `asked` and `note` is one the query behind the card makes true.
#[cfg(feature = "analytics")]
fn method_panel(
    computed: &str,
    version: Option<&str>,
    asked: &[(&str, String)],
    note: &str,
    json: &[(&str, String)],
    csv: &[(&str, &str, &str)],
) -> String {
    let computed = super::frame::behavioral(computed, version);
    super::frame::method(&super::frame::Method {
        computed: &computed,
        asked,
        note,
        json,
        csv,
    })
}

/// The REST address of `route` with `query` (already URL-encoded), under the API's base.
#[cfg(feature = "analytics")]
fn api(route: &str, query: &str) -> String {
    format!("/api/v2/analytics/{route}?{query}")
}

/// A comma-separated, URL-encoded species list for the REST routes, or `None` when a name has a
/// comma in it: the routes read a list split at commas, so such a name cannot be said.
#[cfg(feature = "analytics")]
fn species_param(stored: &[String]) -> Option<String> {
    if stored.iter().any(|s| s.contains(',')) {
        return None;
    }
    Some(
        stored
            .iter()
            .map(|s| super::simple_url_encode(s))
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// HTMX partial: activity sessions table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_sessions_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Activity sessions");
    }
    let params = birdnet_behavioral::types::SessionizeParams::default();
    let (gap_minutes, limit) = (params.gap_minutes, params.limit);
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics_read(|adb| {
                adb.sessionize(&params)
                    .map(|rows| (rows, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok((sessions, version))) => {
            let mut html = render_bursts(&sessions, gap_minutes, limit, &names);
            html.push_str(&method_panel(
                "sessionize",
                version.as_deref(),
                &[
                    ("Quiet gap that ends a burst", format!("{gap_minutes} minutes")),
                    ("Grouped by", "species".to_owned()),
                    ("Sessions asked for", format!("the {limit} newest")),
                ],
                "A session is a run of detections of one species with no gap longer than the quiet gap. A burst is a session of at least two detections; a session of one is not drawn. The table lists the newest twenty bursts.",
                &[("The numbers as JSON", api("sessions", &format!("gap={gap_minutes}&limit={limit}")))],
                &[("Download the table as CSV", "bt-table", "bursts-of-singing")],
            ));
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => analytics_error_html("activity sessions", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics sessions: task failed");
            super::error_states::failed_partial("this station's listening sessions")
        }
    }
}

/// A `DuckDB` timestamp (`2026-10-21 06:23:55`, with an optional fraction and either a space
/// or a `T`) as its date and the seconds after midnight.
#[cfg(feature = "analytics")]
fn split_timestamp(ts: &str) -> Option<(String, u32)> {
    let (date, time) = ts.split_once([' ', 'T'])?;
    if date.len() != 10 {
        return None;
    }
    let mut parts = time.split(':');
    let h: u32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let sec: u32 = parts.next()?.split('.').next()?.parse().ok()?;
    (h < 24 && m < 60 && sec < 61).then(|| (date.to_owned(), h * 3600 + m * 60 + sec))
}

/// The bursts as a timeline, then the table of them: sessions of two or more detections,
/// newest first.
///
/// A "burst of singing" made of one detection lasting 0s is not a burst.
/// Sessionisation groups a species' detections by a `gap_minutes` gap, so a
/// sparse species yields singletons — structurally correct and semantically
/// empty, and a table of nothing but those reads as broken to anyone who looks
/// at it. Filter to real runs and say plainly when there are none.
///
/// Both counts on the card are of what is actually shown. The empty state
/// named "about 20 minutes" while the query grouped by 30, and the footnote
/// "Showing 20 of N sessions" counted the filtered-out singletons in N — and N
/// could never pass `limit`, because only the newest `limit` sessions are
/// fetched.
///
/// The timeline draws the newest seven days that have a burst, each burst where it began and
/// as long as it lasted (`viz::bursts`); the table under it lists the newest twenty with the
/// same numbers. A burst whose timestamps do not parse is left off the timeline and stays in
/// the table, so nothing is silently dropped from the numbers.
#[cfg(feature = "analytics")]
fn render_bursts(
    sessions: &[birdnet_behavioral::types::ActivitySession],
    gap_minutes: u32,
    limit: u32,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    use super::viz::bursts::{Burst, burst_timeline};
    const SHOWN: usize = 20;
    const DAYS: usize = 7;
    let bursts: Vec<_> = sessions.iter().filter(|s| s.detection_count > 1).collect();
    if bursts.is_empty() {
        let seen = sessions.len();
        return format!(
            r#"<p class="bh-muted">No bursts yet. {seen} single detections have been grouped so far, but none is part of a run — a burst needs at least two detections of one species within {gap_minutes} minutes of each other.</p>"#
        );
    }

    // Colours by how many bursts a species had in what is shown, busiest first; past the
    // eighth they share the neutral.
    let mut tally: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for b in &bursts {
        *tally.entry(b.species.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, usize)> = tally.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let slot_of = |species: &str| {
        ranked
            .iter()
            .position(|(n, _)| *n == species)
            .map_or(super::atoms::CHART_SLOTS, |i| {
                i.min(super::atoms::CHART_SLOTS)
            })
    };
    let mut legend: Vec<(String, usize)> = ranked
        .iter()
        .take(super::atoms::CHART_SLOTS)
        .enumerate()
        .map(|(i, (n, _))| (names.name_of_stored(n).to_owned(), i))
        .collect();
    if ranked.len() > super::atoms::CHART_SLOTS {
        legend.push(("Other species".to_owned(), super::atoms::CHART_SLOTS));
    }
    let drawn: Vec<Burst> = bursts
        .iter()
        .filter_map(|s| {
            let (date, start) = split_timestamp(&s.start_time)?;
            let (end_date, end) = split_timestamp(&s.end_time)?;
            Some(Burst {
                species: names.name_of_stored(&s.species).to_owned(),
                slot: slot_of(&s.species),
                end_secs: if end_date == date {
                    end.max(start)
                } else {
                    86_399
                },
                start_secs: start,
                date,
                detections: s.detection_count,
            })
        })
        .collect();

    let mut html = String::new();
    html.push_str(&burst_timeline(&drawn, &legend, DAYS));
    html.push_str(
        r#"<details class="pt-disc"><summary>See the numbers — the bursts</summary><div class="viz-table-scroll"><table id="bt-table"><thead><tr><th>Species</th><th>Detections</th><th>Start</th><th>Duration</th></tr></thead><tbody>"#,
    );
    for s in bursts.iter().take(SHOWN) {
        let duration = format_duration(s.duration_secs);
        let _ = write!(
            html,
            r"<tr><td>{sp}</td><td>{c}</td><td>{st}</td><td>{d}</td></tr>",
            sp = escape_html(names.name_of_stored(&s.species)),
            c = s.detection_count,
            st = escape_html(&s.start_time),
            d = duration,
        );
    }
    html.push_str("</tbody></table></div>");
    if bursts.len() > SHOWN {
        let scope = if u32::try_from(sessions.len()).is_ok_and(|n| n >= limit) {
            format!(" found among the latest {limit} sessions")
        } else {
            String::new()
        };
        let _ = write!(
            html,
            r#"<p class="bh-note">Showing the {SHOWN} most recent of {n} bursts{scope}.</p>"#,
            n = bursts.len(),
        );
    }
    html.push_str("</details>");
    html
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_sessions_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Activity sessions")
}

/// HTMX partial: species retention table.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_retention_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Species retention");
    }
    let params = birdnet_behavioral::types::RetentionParams::default();
    let (intervals, min_detections) = (params.intervals.clone(), params.min_detections);
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics_read(|adb| {
                adb.retention(&params)
                    .map(|rows| (rows, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok((retention, version))) => {
            if retention.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    r#"<p class="bh-muted">No retention data yet.</p>"#.to_string(),
                );
            }
            let mut html = render_retention(&retention, &names);
            let days = intervals
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            html.push_str(&method_panel(
                "retention",
                version.as_deref(),
                &[
                    ("Days after a day heard", days),
                    ("Species listed", format!("those with at least {min_detections} detections")),
                ],
                "For every day a species was heard, whether it was heard again within each number of days; a cell is the share of its days for which it was. The weeks and the kind of visitor (resident, regular, migrant, rarity) are counted from the weeks the species was heard in, not from these rates.",
                &[("The numbers as JSON", api("retention", &format!("min_detections={min_detections}")))],
                &[("Download the table as CSV", "rt-data", "who-keeps-coming-back")],
            ));
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => analytics_error_html("return visits", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics retention: task failed");
            super::error_states::failed_partial("which birds came back")
        }
    }
}

/// The rate of `days` in `rates`, as a number in `0..=1`.
#[cfg(feature = "analytics")]
fn rate_at(rates: &[birdnet_behavioral::types::RetentionRate], days: u32) -> Option<f64> {
    rates.iter().find(|r| r.days == days).map(|r| r.rate)
}

/// The day intervals the retention table shows, with how each is said.
#[cfg(feature = "analytics")]
const RETURN_AFTER: [(u32, &str); 6] = [
    (1, "1 d"),
    (2, "2 d"),
    (3, "3 d"),
    (7, "1 wk"),
    (14, "2 wk"),
    (30, "1 mo"),
];

/// Who keeps coming back, as a ledger of how often each species returns: for every species the
/// share of the days it was heard that it was heard again after a day, two, three, a week, a
/// fortnight and a month, each a cell on the sequential ramp that also prints its number, and
/// the weeks it was heard of the weeks the station listened, as a bar. The species are in
/// groups by how much of the station's time they were present (resident, regular, migrant,
/// rarity), which is what the classification is and what the groups say under their names.
///
/// The ledger is a table, so it is also its own alternative: every number is a cell, read by
/// row and column, and the ramp only helps the eye find the pattern (a resident is a row of
/// full cells; a bird passing through is a row that fades).
#[cfg(feature = "analytics")]
fn render_retention(
    retention: &[birdnet_behavioral::types::SpeciesRetention],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    use birdnet_behavioral::types::ResidencyType;
    // How often a species is here is a description, not a grade, so none of these is red: this
    // drew a migrant — a normal thing for a bird to be — in the colour of a fault.
    let groups = [
        (
            ResidencyType::Resident,
            "Resident",
            "moss",
            "heard in more than 70% of the weeks",
        ),
        (
            ResidencyType::Regular,
            "Regular",
            "",
            "heard in 30 to 70% of them",
        ),
        (
            ResidencyType::Migrant,
            "Migrant",
            "dawn",
            "heard in fewer: passing through",
        ),
        (
            ResidencyType::Rarity,
            "Rarity",
            "dawn",
            "every detection within one week",
        ),
    ];
    let mut html = String::from(
        r#"<div class="viz-table-scroll rt-scroll"><table id="rt-data" class="rt-table"><thead><tr><th scope="col">Species</th><th scope="col">Weeks heard</th>"#,
    );
    for (_, label) in RETURN_AFTER {
        let _ = write!(html, r#"<th scope="col" class="rt-h">{label}</th>"#);
    }
    html.push_str("</tr></thead>");
    for (class, label, cls, said) in groups {
        let rows: Vec<_> = retention
            .iter()
            .filter(|r| r.classification == class)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let _ = write!(
            html,
            r#"<tbody><tr class="rt-group"><th scope="colgroup" colspan="{span}"><span class="bnb-pill {cls}">{label}</span> <span class="bnb-meta">{count} · {said}</span></th></tr>"#,
            span = RETURN_AFTER.len() + 2,
            count = rows.len(),
        );
        for r in rows {
            let _ = write!(
                html,
                r#"<tr><th scope="row" class="rt-name">{sp}</th><td class="rt-weeks"><span class="rt-bar" aria-hidden="true"><i data-style="width:{w:.0}%"></i></span><span class="mono rt-wk">{wk} of {of}</span></td>"#,
                sp = escape_html(names.name_of_stored(&r.species)),
                w = f64::from(r.weeks_present) / f64::from(r.station_weeks.max(1)) * 100.0,
                wk = r.weeks_present,
                of = r.station_weeks,
            );
            for (days, _) in RETURN_AFTER {
                match rate_at(&r.retention_rates, days) {
                    Some(rate) => {
                        // Seven even steps from 0 to 100%: a rate is a proportion, so (unlike a
                        // count) its ramp is linear.
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let step = ((rate * 7.0).ceil() as usize).clamp(1, 7);
                        let class = if rate <= 0.0 {
                            "rt-zero".to_owned()
                        } else {
                            format!("ck-m{step}")
                        };
                        let _ = write!(
                            html,
                            r#"<td class="rt-cell {class}">{:.0}%</td>"#,
                            rate * 100.0
                        );
                    }
                    None => html.push_str(r#"<td class="rt-cell rt-none">—</td>"#),
                }
            }
            html.push_str("</tr>");
        }
        html.push_str("</tbody>");
    }
    html.push_str("</table></div>");
    html.push_str(
        r#"<div class="ck-scale rt-key"><span class="ck-steps" aria-hidden="true"><i class="ck-sw ck-m1"></i><i class="ck-sw ck-m2"></i><i class="ck-sw ck-m3"></i><i class="ck-sw ck-m4"></i><i class="ck-sw ck-m5"></i><i class="ck-sw ck-m6"></i><i class="ck-sw ck-m7"></i></span><span class="bnb-meta">0% to 100% of the days a species was heard that it was heard again that much later.</span></div>"#,
    );
    html
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_retention_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Species retention")
}

/// HTMX partial: the species heard just before and just after the most
/// recently detected one.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_next_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Next species predictions");
    }
    let trigger_result = tokio::task::spawn_blocking({
        let s = state.clone();
        move || {
            s.with_read_db(|conn| {
                // The most recent detection. Not `ORDER BY rowid`: this is a
                // view, and the bundled SQLite (3.53) has no rowid on a view —
                // "no such column: rowid". The error was swallowed by `.ok()`
                // into "No detections yet", so the card never showed a
                // prediction on any station.
                conn.query_row(
                    "SELECT Com_Name FROM detections_analytic \
                     ORDER BY detected_at_utc DESC NULLS LAST LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
            })
        }
    })
    .await;

    let trigger = match trigger_result {
        Ok(Ok(Some(trigger))) => trigger,
        Ok(Ok(None)) => {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                r#"<p class="bh-muted">No detections yet.</p>"#.to_string(),
            );
        }
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "analytics next: could not read the latest detection");
            return super::error_states::failed_partial("what the station expects to hear next");
        }
        Err(e) => {
            tracing::warn!(error = %e, "analytics next: task failed");
            return super::error_states::failed_partial("what the station expects to hear next");
        }
    };

    let display = trigger.clone();
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics_read(|adb| {
                let before = adb.previous_species(&trigger, 60, 5)?;
                let after = adb.next_species(&trigger, 60, 5)?;
                Ok((before, after, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;

    match result {
        Ok(Ok((before, after, version))) => {
            if before.is_empty() && after.is_empty() {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/html")],
                    format!(
                        r#"<p class="bh-muted">No predictions for <strong>{}</strong> yet.</p>"#,
                        escape_html(names.name_of_stored(&display))
                    ),
                );
            }
            let before: Vec<_> = before
                .iter()
                .map(|p| (p.predicted_species.as_str(), p.probability, p.frequency))
                .collect();
            let after: Vec<_> = after
                .iter()
                .map(|p| (p.predicted_species.as_str(), p.probability, p.frequency))
                .collect();
            let mut html = render_neighbour_figure(&display, &before, &after, &names);
            let who = super::simple_url_encode(&display);
            html.push_str(&method_panel(
                "sequence_next_node",
                version.as_deref(),
                &[
                    ("Bird looked at", names.name_of_stored(&display).to_owned()),
                    ("What that is", "the species of the most recent detection".to_owned()),
                    ("Sessions", "runs of detections with no gap longer than 60 minutes".to_owned()),
                    ("Neighbours listed", "up to 5 on each side".to_owned()),
                ],
                "For each session in which the bird was heard, the species heard just before its first detection and just after it. The probability is the share of those sessions; birds heard in the same recording as the bird are not counted as before or after it.",
                &[
                    ("After, as JSON", api("next-species", &format!("after={who}&window=60&limit=5"))),
                    ("Before, as JSON", api("previous-species", &format!("before={who}&window=60&limit=5"))),
                ],
                &[
                    ("Download “before” as CSV", "nb-before", "heard-before"),
                    ("Download “after” as CSV", "nb-after", "heard-after"),
                ],
            ));
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => analytics_error_html("what sings next", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics next: task failed");
            super::error_states::failed_partial("what the station expects to hear next")
        }
    }
}

/// The follow-on card: a flow with the species in the middle (wide screens), the same
/// neighbours as ranked bars (narrow ones, where a flow's names would be drawn too small), and
/// the tables, which hold every number, under a disclosure.
///
/// Each of `before` and `after` is `(species, probability, sessions)` rows.
#[cfg(feature = "analytics")]
fn render_neighbour_figure(
    trigger: &str,
    before: &[(&str, f64, u64)],
    after: &[(&str, f64, u64)],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    use super::viz::flow::{Neighbour, neighbours_flow};
    let to_neighbours = |rows: &[(&str, f64, u64)]| -> Vec<Neighbour> {
        rows.iter()
            .map(|&(species, probability, frequency)| Neighbour {
                name: names.name_of_stored(species).to_owned(),
                probability,
                frequency,
                is_trigger: species == trigger,
            })
            .collect()
    };
    let (b, a) = (to_neighbours(before), to_neighbours(after));
    let shown = names.name_of_stored(trigger);
    let mut html = String::from(r#"<div class="fl-fig">"#);
    html.push_str(&neighbours_flow(shown, &b, &a));
    html.push_str(r#"<div class="fl-lists">"#);
    for (side, rows) in [("Before", &b), ("After", &a)] {
        let _ = write!(
            html,
            r#"<h3 class="fl-list-h">{side} <b>{}</b></h3>"#,
            escape_html(shown)
        );
        if rows.is_empty() {
            html.push_str(r#"<p class="bh-muted">Nothing yet.</p>"#);
            continue;
        }
        html.push_str(r#"<ol class="co-rank">"#);
        let pmax = rows
            .iter()
            .map(|n| n.probability)
            .fold(f64::MIN_POSITIVE, f64::max);
        for n in rows {
            let _ = write!(
                html,
                r#"<li class="co-rank-row"><span class="co-rank-names">{name}{again}</span><span class="co-rank-bar"><i class="ck-sw ck-q{step}" data-style="width:{w:.0}%"></i></span><b class="co-rank-v">{pct:.0}%</b></li>"#,
                name = escape_html(&n.name),
                again = if n.is_trigger {
                    r#" <span class="bnb-meta">(calls again)</span>"#
                } else {
                    ""
                },
                step = super::viz::cograph::share_step(n.probability / pmax),
                w = (n.probability / pmax * 100.0).max(3.0),
                pct = n.probability * 100.0,
            );
        }
        html.push_str("</ol>");
    }
    html.push_str("</div>");
    html.push_str(
        r#"<details class="pt-disc"><summary>See the numbers — before and after</summary><div>"#,
    );
    render_neighbours(&mut html, "Before", trigger, before, names);
    render_neighbours(&mut html, "After", trigger, after, names);
    html.push_str("</div></details></div>");
    html
}

/// One "Before"/"After" table of the follow-on card: `(species, probability,
/// sessions)` rows for the species heard adjacent to `trigger`.
#[cfg(feature = "analytics")]
fn render_neighbours(
    html: &mut String,
    side: &str,
    trigger: &str,
    rows: &[(&str, f64, u64)],
    names: &birdnet_core::i18n::NameLocalizer,
) {
    let _ = write!(
        html,
        r#"<p class="bh-after">{side} <strong>{}</strong>:</p>"#,
        escape_html(names.name_of_stored(trigger)),
    );
    if rows.is_empty() {
        html.push_str(r#"<p class="bh-muted">Nothing yet.</p>"#);
        return;
    }
    html.push_str("<table class=\"bh-neighbours\" data-fits-phone id=\"nb-");
    html.push_str(&side.to_lowercase());
    html.push_str(
        "\"><thead><tr><th>Species</th><th>Probability</th><th>Observed</th></tr></thead><tbody>",
    );
    for &(species, probability, frequency) in rows {
        // The trigger appears among its own follow-ons — the same bird calling
        // again — which under a heading that reads "which tends to turn up
        // **next**" is confusing rather than informative. Label it instead of
        // dropping it: that it sings again is a real fact about the species,
        // just not a *succession* fact. It cannot precede its own first
        // detection, so the label only ever appears under "After".
        let self_follow = species == trigger;
        let pct = probability * 100.0;
        // A follow-on probability is a likelihood, not a verdict: a bird that
        // turns up next one time in five is not a warning, and this drew those
        // pills red (`low`). Green for a likely successor, neutral otherwise.
        let cls = if pct >= 50.0 { "moss" } else { "" };
        let _ = write!(
            html,
            r#"<tr><td>{sp}{note}</td><td><span class="bnb-pill {cls}">{pct:.0}%</span></td><td>{frequency}</td></tr>"#,
            sp = escape_html(names.name_of_stored(species)),
            note = if self_follow {
                r#" <span class="bnb-meta">(calls again)</span>"#
            } else {
                ""
            },
        );
    }
    html.push_str("</tbody></table>");
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_next_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Next species predictions")
}

/// How far back "who answers whom" looks, in days, and how many birds it compares.
#[cfg(feature = "analytics")]
const ANSWERS_DAYS: i64 = 30;
#[cfg(feature = "analytics")]
const ANSWERS_SPECIES: usize = 8;

/// HTMX partial: who is heard within a minute of whom, among the birds heard most lately.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_answers_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Who answers whom");
    }
    let Some(from) = super::date_days_ago(ANSWERS_DAYS) else {
        return super::error_states::failed_partial("who answers whom");
    };
    let from_date = from.clone();
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        state
            .with_analytics_read(|adb| {
                adb.answers(&from, 1, ANSWERS_SPECIES)
                    .map(|a| (a, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
    })
    .await;
    match result {
        Ok(Ok((a, version))) => {
            let shown: Vec<String> = a
                .species
                .iter()
                .map(|s| names.name_of_stored(s).to_owned())
                .collect();
            let window = u32::try_from(ANSWERS_DAYS).unwrap_or(30);
            let mut html = super::behavioral_answers::render(&a, &shown, window);
            if a.species.len() >= 2 {
                html.push_str(&method_panel(
                    "sequence_count",
                    version.as_deref(),
                    &[
                        ("Birds compared", format!("the {} heard most in the last {ANSWERS_DAYS} days", a.species.len())),
                        ("Counted since", from_date.clone()),
                        ("Replies", "heard within a minute after, on the same day".to_owned()),
                    ],
                    "A cell counts how many times the row bird was heard and then the column bird within a minute, without overlap: one bird singing three times before one reply is one exchange. Two birds in the same recording are not a reply, and the grid is not symmetric.",
                    &[("The numbers as JSON", api("answers", &format!("from={from_date}&within=1&top={ANSWERS_SPECIES}")))],
                    &[("Download the table as CSV", "aw-table", "who-answers-whom")],
                ));
            }
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Err(e)) => analytics_error_html("who answers whom", &e),
        Err(e) => {
            tracing::warn!(error = %e, "analytics answers: task failed");
            super::error_states::failed_partial("who answers whom")
        }
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_answers_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Who answers whom")
}

/// HTMX partial: how fast the dawn chorus comes together (`window_funnel` at a ladder of windows).
///
/// The voices are the station's own: its six regular dawn voices of the last season, in the order
/// they typically open (see [`derive_dawn_voices`]), so the card means something wherever the
/// station is. With fewer than three there is no chorus to watch, and the card says so.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_assembly_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    use super::behavioral_assembly as asm;
    if !state.has_analytics() {
        return analytics_unavailable_html("The chorus assembling");
    }
    let Some(since) = super::date_days_ago(asm::DAYS) else {
        return super::error_states::failed_partial("the chorus assembling");
    };
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        // A failed read is an error on the card, not "no chorus yet": that sentence is a claim
        // about the station's data.
        let voices = state
            .with_read_db(|conn| derive_dawn_voices(conn, Some(&since), asm::VOICES, asm::VOICES))
            .map_err(|e| {
                birdnet_behavioral::connection::AnalyticsError::InvalidData(format!(
                    "reading the dawn detections: {e}"
                ))
            })?;
        if voices.len() < asm::MIN_VOICES {
            return Ok(None);
        }
        let params = birdnet_behavioral::types::AssemblyParams {
            species_sequence: voices.clone(),
            windows_minutes: birdnet_behavioral::types::ASSEMBLY_WINDOWS.to_vec(),
            hour_start: DAWN_HOUR_START,
            hour_end: DAWN_HOUR_END,
            since: since.clone(),
        };
        state
            .with_analytics_read(|adb| {
                let rows = adb.assembly(&params)?;
                Ok((voices, params, rows, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
            .map(Some)
    })
    .await;

    match result {
        Ok(Ok(Some((voices, params, rows, version)))) => {
            let shown: Vec<String> = voices
                .iter()
                .map(|s| names.name_of_stored(s).to_owned())
                .collect();
            let html = asm::render(&asm::Card {
                stored: &voices,
                shown: &shown,
                rows: &rows,
                windows: &params.windows_minutes,
                hours: (DAWN_HOUR_START, DAWN_HOUR_END),
                since: &params.since,
                version: version.as_deref(),
            });
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Ok(None)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            format!(
                r#"<p class="bh-muted">Fewer than {} voices sing often enough at dawn here to hear a chorus assemble: give the mornings a little longer.</p>"#,
                asm::MIN_VOICES
            ),
        ),
        Ok(Err(e)) => analytics_error_html("the chorus assembling", &e),
        Err(e) => {
            tracing::warn!(error = %e, "analytics assembly: task failed");
            super::error_states::failed_partial("how fast the chorus comes together")
        }
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_assembly_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("The chorus assembling")
}

/// Dawn window the sequence card analyses (hours of day, inclusive).
#[cfg(feature = "analytics")]
const DAWN_HOUR_START: u32 = 4;
#[cfg(feature = "analytics")]
const DAWN_HOUR_END: u32 = 8;
/// Funnel window (minutes) — a morning's run may span the whole dawn window, so
/// the `window_funnel` window covers all of it: hours 4 to 8 *inclusive* are
/// five hours, 04:00–08:59. It was 240, so a run the headline counted as "in
/// order" (`sequence_count`, no gap limit) could fall short in the funnel.
#[cfg(feature = "analytics")]
const DAWN_FUNNEL_WINDOW_MINUTES: u32 = (DAWN_HOUR_END - DAWN_HOUR_START + 1) * 60;

/// HTMX partial: the dawn "running order" — how often the morning's leading
/// voices sing in sequence (`sequence_count`, v0.8.0) plus the step timing of a
/// recent run (`sequence_match_events`, v0.8.0).
///
/// The sequence is derived from the station's own dawn-window data rather than
/// hard-coded, so the card is meaningful regardless of geography — the REST
/// defaults are European, but a North-American dawn opens with entirely
/// different birds.
#[cfg(feature = "analytics")]
pub(super) async fn analytics_dawn_sequence_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    if !state.has_analytics() {
        return analytics_unavailable_html("Dawn sequence");
    }
    let names = state.names_handle();
    let result = tokio::task::spawn_blocking(move || {
        // A failed read is an error on the card, not "not enough dawn
        // activity yet" — that sentence is a claim about the station's data.
        let sequence = state.with_read_db(derive_dawn_sequence).map_err(|e| {
            birdnet_behavioral::connection::AnalyticsError::InvalidData(format!(
                "reading the dawn detections: {e}"
            ))
        })?;
        // sequence_count / sequence_match_events need 2..=32 steps; fewer than
        // two prominent dawn voices means there's no order to read yet.
        if sequence.len() < 2 {
            return Ok(None);
        }
        let params = birdnet_behavioral::types::PatternParams {
            species_sequence: sequence.clone(),
            max_gap_minutes: None,
            hour_start: DAWN_HOUR_START,
            hour_end: DAWN_HOUR_END,
        };
        let funnel_params = birdnet_behavioral::types::FunnelParams {
            species_sequence: sequence.clone(),
            window_minutes: DAWN_FUNNEL_WINDOW_MINUTES,
            hour_start: DAWN_HOUR_START,
            hour_end: DAWN_HOUR_END,
        };
        state
            .with_analytics_read(|adb| {
                // Both run the same NFA pattern over the same params: how *often*
                // the ordered run completed (sequence_count) and the per-step
                // timestamps (sequence_match_events). Sharing the pattern means a
                // counted full-match day always has a full set of step times to
                // show, so the headline and the morning we surface stay aligned.
                let counts = adb.sequence_count(&params)?;
                let events = adb.sequence_match_events(&params)?;
                // window_funnel over the same sequence: how far each morning got
                // down the chain, aggregated into the funnel picture.
                let funnel = adb.funnel(&funnel_params)?;
                Ok((sequence, counts, events, funnel, adb.extension_version()))
            })
            .unwrap_or_else(|| {
                Err(
                    birdnet_behavioral::connection::AnalyticsError::ExtensionLoad(
                        "analytics not available".into(),
                    ),
                )
            })
            .map(Some)
    })
    .await;

    match result {
        Ok(Ok(Some((sequence, counts, events, funnel, version)))) => {
            let mut html = render_dawn_sequence(&sequence, &counts, &events, &funnel, &names);
            let shown = sequence
                .iter()
                .map(|s| names.name_of_stored(s).to_owned())
                .collect::<Vec<_>>()
                .join(" \u{2192} ");
            let query = species_param(&sequence).map(|sp| {
                format!("species={sp}&hour_start={DAWN_HOUR_START}&hour_end={DAWN_HOUR_END}")
            });
            let json: Vec<(&str, String)> = query
                .iter()
                .flat_map(|q| {
                    [
                        ("Runs per morning, as JSON", api("sequence-count", q)),
                        ("When each voice joined, as JSON", api("sequence-match-events", q)),
                        (
                            "How far each morning got, as JSON",
                            api("funnel", &format!("{q}&window={DAWN_FUNNEL_WINDOW_MINUTES}")),
                        ),
                    ]
                })
                .collect();
            html.push_str(&method_panel(
                "sequence_count, sequence_match_events, window_funnel",
                version.as_deref(),
                &[
                    ("Voices, in order", shown),
                    ("Hours of the morning", format!("hour {DAWN_HOUR_START} to hour {DAWN_HOUR_END}, inclusive")),
                    ("Time allowed for the funnel", format!("{DAWN_FUNNEL_WINDOW_MINUTES} minutes")),
                    ("Gap between voices in a run", "none".to_owned()),
                ],
                "The voices are the station's most heard at dawn, in the order they typically open. A run is the voices heard in that order on one morning, each strictly later than the one before; the times are those of the first run of the morning, and the dots are those first-heard times. The funnel counts how far down the order each morning got within the time allowed. The table lists the newest seven mornings.",
                &json,
                &[("Download the table as CSV", "ds-mornings", "morning-running-order")],
            ));
            (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
        }
        Ok(Ok(None)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html")],
            r#"<p class="bh-muted">Not enough dawn activity yet to read a running order — give the mornings a little longer.</p>"#.to_string(),
        ),
        Ok(Err(e)) => analytics_error_html("the dawn sequence", &e),
        // See `error_states::failed_partial` for why this is a 200. `Ok(Err(..))` above is already a 200.
        Err(e) => {
            tracing::warn!(error = %e, "analytics dawn sequence: task failed");
            super::error_states::failed_partial("the order birds joined the dawn chorus")
        }
    }
}

#[cfg(not(feature = "analytics"))]
pub(super) async fn analytics_dawn_sequence_partial(
    State(_): State<AppState>,
) -> impl axum::response::IntoResponse {
    analytics_unavailable_html("Dawn sequence")
}

/// Derive the station's dawn "running order" from its own data: the most
/// prominent dawn-window voices (hours 4–8), ordered by when each typically
/// *starts* — the mean, over the mornings it was heard, of its first dawn
/// detection that morning. Returns up to three species; fewer than two means
/// there isn't enough dawn activity to read an order.
///
/// The card reads "your dawn tends to open …", which is a claim about who
/// starts first. It was ordered by the mean time of *all* of a species' dawn
/// song, so a bird up at 04:30 that sang on past eight was placed after one
/// that sang only from five to twenty past.
#[cfg(feature = "analytics")]
fn derive_dawn_sequence(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    derive_dawn_voices(conn, None, 5, 3)
}

/// The dawn's regular voices, in the order they typically open: the `pool` most heard in the dawn
/// hours (4 to 8) with at least ten songs there, since `since` (a `YYYY-MM-DD`, or all history for
/// `None`), then the first `keep` of those by the mean time of their first song of a morning,
/// earliest first.
///
/// The running order asks for the three earliest of the five most heard; the chorus card for all
/// six of the six most heard, over the last season, so that it follows the voices that are singing
/// now and not the ones of a different time of year.
#[cfg(feature = "analytics")]
fn derive_dawn_voices(
    conn: &rusqlite::Connection,
    since: Option<&str>,
    pool: usize,
    keep: usize,
) -> rusqlite::Result<Vec<String>> {
    const SQL: &str = "WITH dawn_rows AS (
            SELECT Date, Com_Name,
                   CAST(substr(Time, 1, 2) AS REAL) * 3600
                       + CAST(substr(Time, 4, 2) AS REAL) * 60
                       + CAST(substr(Time, 7, 2) AS REAL) AS secs
            FROM detections_analytic
            WHERE length(Time) >= 8
              AND CAST(substr(Time, 1, 2) AS INTEGER) BETWEEN 4 AND 8
              AND (?1 IS NULL OR Date >= ?1)
        ),
        mornings AS (
            SELECT Com_Name, Date, COUNT(*) AS n, MIN(secs) AS first_secs
            FROM dawn_rows
            GROUP BY Com_Name, Date
        ),
        dawn AS (
            SELECT Com_Name, SUM(n) AS c, AVG(first_secs) AS opens_secs
            FROM mornings
            GROUP BY Com_Name
            HAVING c >= 10
        )
        SELECT Com_Name FROM (
            SELECT Com_Name, opens_secs FROM dawn ORDER BY c DESC, Com_Name ASC LIMIT ?2
        ) ORDER BY opens_secs ASC, Com_Name ASC LIMIT ?3";
    let mut stmt = conn.prepare(SQL)?;
    let rows = stmt.query_map(
        rusqlite::params![
            since,
            i64::try_from(pool).unwrap_or(i64::MAX),
            i64::try_from(keep).unwrap_or(i64::MAX)
        ],
        |row| row.get::<_, String>(0),
    )?;
    rows.collect()
}

/// Trim a `DuckDB` timestamp string (`2026-04-12 05:58:23`) to `HH:MM`.
#[cfg(feature = "analytics")]
fn step_time_hhmm(ts: &str) -> &str {
    ts.split([' ', 'T'])
        .nth(1)
        .and_then(|t| t.get(..5))
        .unwrap_or(ts)
}

/// The most recent morning that completed the whole ordered sequence (a full
/// set of step times), to show step-by-step. `events` arrive date-descending,
/// so `find` yields the most recent. Days that reached only a partial in-order
/// prefix aren't a full run, so they're skipped — the card only calls this when
/// `sequence_count` already found a full match, so one always exists.
#[cfg(feature = "analytics")]
fn best_progression(
    full_len: usize,
    events: &[birdnet_behavioral::types::PatternMatchEvents],
) -> Option<&birdnet_behavioral::types::PatternMatchEvents> {
    events.iter().find(|e| e.step_times.len() == full_len)
}

/// Per-step "mornings that reached this step" counts from the per-day funnel
/// results: step `k` (1-based) is the number of days whose `steps_completed`
/// reached at least `k`. The result is non-increasing — the funnel shape.
#[cfg(feature = "analytics")]
fn funnel_step_counts(
    funnel: &[birdnet_behavioral::types::ChorusFunnel],
    total_steps: usize,
) -> Vec<u64> {
    let total = u32::try_from(total_steps).unwrap_or(u32::MAX);
    (1..=total)
        .map(|k| {
            u64::try_from(funnel.iter().filter(|f| f.steps_completed >= k).count()).unwrap_or(0)
        })
        .collect()
}

/// How many of the newest mornings the running order draws a dot and a line for.
#[cfg(feature = "analytics")]
const MORNINGS_DRAWN: usize = 45;

/// The voices and mornings the running-order picture draws.
///
/// `events` are the engine's `sequence_match_events`, newest morning first: for each, the
/// moment each step of the run fired. The typical time of a voice is the median over every
/// morning it was part of a run, however many there are; only the newest [`MORNINGS_DRAWN`]
/// are drawn. A timestamp that does not parse ends that morning's run at the step before it.
#[cfg(feature = "analytics")]
fn morning_inputs(
    sequence: &[String],
    step_counts: &[u64],
    events: &[birdnet_behavioral::types::PatternMatchEvents],
    names: &birdnet_core::i18n::NameLocalizer,
) -> (Vec<super::viz::Voice>, Vec<super::viz::Morning>) {
    let mut per_step: Vec<Vec<u32>> = vec![Vec::new(); sequence.len()];
    let mut mornings = Vec::new();
    for e in events {
        let times: Vec<u32> = e
            .step_times
            .iter()
            .take(sequence.len())
            .map_while(|t| split_timestamp(t).map(|(_, secs)| secs))
            .collect();
        for (k, &t) in times.iter().enumerate() {
            per_step[k].push(t);
        }
        if !times.is_empty() && mornings.len() < MORNINGS_DRAWN {
            mornings.push(super::viz::Morning {
                date: e.date.clone(),
                times,
            });
        }
    }
    let voices = sequence
        .iter()
        .zip(step_counts)
        .zip(per_step.iter_mut())
        .map(|((name, &reached), seen)| super::viz::Voice {
            name: names.name_of_stored(name).to_owned(),
            reached,
            typical_secs: super::viz::funnel::median(seen),
        })
        .collect();
    (voices, mornings)
}

/// Render the dawn-sequence card body from the derived sequence, its per-day
/// occurrence counts (`sequence_count`) and step timings (`sequence_match_events`).
#[cfg(feature = "analytics")]
fn render_dawn_sequence(
    sequence: &[String],
    counts: &[birdnet_behavioral::types::PatternCount],
    events: &[birdnet_behavioral::types::PatternMatchEvents],
    funnel: &[birdnet_behavioral::types::ChorusFunnel],
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let chain = sequence
        .iter()
        .map(|s| escape_html(names.name_of_stored(s)))
        .collect::<Vec<_>>()
        .join(" \u{2192} ");

    let total_occ: u64 = counts.iter().map(|c| c.count).sum();
    let match_days = counts.iter().filter(|c| c.count > 0).count();
    let total_days = counts.len();
    let best = counts.iter().map(|c| c.count).max().unwrap_or(0);

    let mut html =
        format!(r#"<p class="bh-after">Your dawn tends to open <strong>{chain}</strong>.</p>"#);

    // Lead with the picture (the Patterns idiom): the running order, one lane per voice with
    // the mornings as dots on the clock. Omitted — never an empty chart — when nothing reached
    // even the first step.
    let step_counts = funnel_step_counts(funnel, sequence.len());
    if step_counts.first().is_some_and(|c| *c > 0) {
        let (voices, mornings) = morning_inputs(sequence, &step_counts, events, names);
        html.push_str(&super::viz::morning_order(
            &voices,
            &mornings,
            (DAWN_HOUR_START, DAWN_HOUR_END),
        ));
    }

    if total_occ == 0 {
        html.push_str(
            r#"<p class="bh-muted">All heard at dawn, but not yet in that exact order on a single morning.</p>"#,
        );
        return html;
    }

    let _ = write!(
        html,
        r#"<p class="bnb-meta">In order on <strong>{match_days}</strong> of <strong>{total_days}</strong> mornings · <strong>{total_occ}</strong> runs in total · up to <strong>{best}</strong> in one morning.</p>"#
    );

    if let Some(ev) = best_progression(sequence.len(), events) {
        let _ = write!(
            html,
            r#"<p class="bh-after">A recent morning — <strong>{date}</strong>:</p><table class="pt-tbl"><thead><tr><th>Voice</th><th>First heard</th></tr></thead><tbody>"#,
            date = escape_html(&ev.date),
        );
        // step_times[i] pairs with species_sequence[i] for the completed steps;
        // zip stops at the shorter, so partial runs show only what fired.
        for (sp, t) in ev.species_sequence.iter().zip(ev.step_times.iter()) {
            let _ = write!(
                html,
                "<tr><td>{sp}</td><td>{t}</td></tr>",
                sp = escape_html(names.name_of_stored(sp)),
                t = crate::display::clock(step_time_hhmm(t)),
            );
        }
        html.push_str("</tbody></table>");
    }

    // The most recent mornings, tucked under a disclosure (the Patterns
    // "see the numbers" idiom) so the card leads with the headline.
    html.push_str(
        r#"<details class="pt-disc"><summary>Recent mornings</summary><div><table class="pt-tbl" id="ds-mornings"><thead><tr><th>Morning</th><th>In-order runs</th></tr></thead><tbody>"#,
    );
    for c in counts.iter().take(7) {
        let _ = write!(
            html,
            "<tr><td>{date}</td><td>{count}</td></tr>",
            date = escape_html(&c.date),
            count = c.count,
        );
    }
    html.push_str("</tbody></table></div></details>");

    html
}

async fn analytics_config_partial(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let compiled = cfg!(feature = "analytics");
    let configured = state.has_analytics();
    let db_path = escape_html(&state.db_path().display().to_string());
    let version = env!("CARGO_PKG_VERSION");

    // Pull the live extension status from the AnalyticsDb when one is open. The
    // three flags — compiled / active / extension-loaded — measure independent
    // truths, so they're shown as three distinct rows instead of one ambiguous
    // "Connected" pill.
    // On the blocking pool, for the reason `analytics_status` gives: the
    // handle's mutex is shared with every analytics query. A failed task reads
    // as "no store", which the table renders as nothing rather than a guess.
    #[cfg(feature = "analytics")]
    let ext_status: Option<(bool, Option<String>, Option<String>)> = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            state.with_analytics_read(|db| {
                (
                    db.extension_loaded(),
                    db.duckdb_version(),
                    db.extension_version(),
                )
            })
        }
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "analytics config: task failed");
        None
    });
    #[cfg(not(feature = "analytics"))]
    let ext_status: Option<(bool, Option<String>, Option<String>)> = None;

    let mut html = format!(
        r#"<table class="bh-config-table"><tr><td class="bh-key">Version</td><td>{version}</td></tr>
<tr><td class="bh-key">SQLite Database</td><td><code>{db_path}</code></td></tr>
<tr><td class="bh-key">Analytics Compiled</td><td>{compiled}</td></tr>
<tr><td class="bh-key">Analytics Active</td><td>{configured}</td></tr>"#,
    );
    if let Some((loaded, duckdb_v, ext_v)) = ext_status {
        let loaded_str = if loaded { "true" } else { "false" };
        let duckdb_v_str = escape_html(duckdb_v.as_deref().unwrap_or("unknown"));
        let ext_v_str = escape_html(ext_v.as_deref().unwrap_or("\u{2014}"));
        let _ = write!(
            html,
            "<tr><td class=\"bh-key\">DuckDB</td><td><code>{duckdb_v_str}</code></td></tr>\
             <tr><td class=\"bh-key\">Behavioral extension</td><td><code>{ext_v_str}</code> \u{00b7} loaded: <strong>{loaded_str}</strong></td></tr>"
        );
        if !loaded {
            html.push_str(
                // The commands stay — this is a diagnostics table and the
                // person who can act on them reads it — but they no longer
                // lead. The first sentence is for the station's owner, who
                // was previously handed a build-time environment variable and
                // a path inside the source repository.
                r#"<tr><td colspan="2" class="bh-cell-note">The extra behaviour insights (activity sessions, return visits, what sings next) are not available on this station. Everything else keeps recording as normal.<br><span class="bh-muted-sm">For whoever set the station up: run <code>--refresh-extension</code> to fetch the extension from the community registry, or use a release that bundles it (<code>BIRDNET_BUNDLED_EXTENSION_FILE</code> at build time, or a copy vendored under <code>crates/birdnet-behavioral/vendor/</code>).</span></td></tr>"#,
            );
        }
    }
    if compiled && !configured {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">Analytics is on by default — restart the service to open the DuckDB file alongside the SQLite database.</td></tr>"#);
    } else if !compiled {
        html.push_str(r#"<tr><td colspan="2" class="bh-cell-note">This build does not include the extra behaviour insights.<br><span class="bh-muted-sm">For whoever set the station up: rebuild with default features, or <code>--features analytics</code>.</span></td></tr>"#);
    }
    html.push_str("</table>");
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

fn analytics_unavailable_html(
    feature: &str,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    // These render inside the ordinary analytics cards, not the diagnostics
    // table, so they carry no command at all: the reader of a card about the
    // dawn chorus is not holding a terminal.
    let msg = format!(
        r#"<p class="bh-muted">{feature} isn't available on this station — the extra behaviour insights aren't switched on.</p>
<p class="bh-muted-sm">Everything else keeps recording as normal. Whoever set the station up can turn them on.</p>"#
    );
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], msg)
}

/// What to put in a behavioural card that could not be filled.
///
/// This used to open, for every failure, with "The `duckdb-behavioral`
/// extension is required for {func}" followed by the raw
/// `AnalyticsError::to_string()`. The header sentence was simply wrong for
/// three of the four variants: a query error and a lock held by another
/// process both loaded the extension perfectly well. So the card told its
/// reader to install something that was already installed, and then showed
/// them `DuckDB error: Binder Error: ...` underneath.
///
/// `what` completes "We couldn't load {what}", so pass a noun phrase in the
/// reader's words.
#[cfg(feature = "analytics")]
fn analytics_error_html(
    what: &str,
    error: &birdnet_behavioral::connection::AnalyticsError,
) -> (StatusCode, [(header::HeaderName, &'static str); 1], String) {
    use birdnet_behavioral::connection::AnalyticsError;
    tracing::warn!(error = %error, "behavioural card could not be filled: {what}");
    let html = match error {
        // The one case where the old sentence was true. Said without naming a
        // build flag or an environment variable: the reader of this page is
        // not the person who compiles it.
        AnalyticsError::ExtensionLoad(_) => format!(
            r#"<p class="bh-muted">The extra behaviour insights aren't switched on for this station, so {what} can't be worked out here.</p>
<p class="bh-muted-sm">Everything else on the station keeps recording as normal.</p>"#
        ),
        // Transient and self-clearing: say so, because "try again" is the
        // whole of the advice and it actually works.
        AnalyticsError::Locked(_) => format!(
            r#"<p class="bh-muted">The analytics database is busy right now, so {what} isn't ready.</p>
<p class="bh-muted-sm">This usually clears in a moment — reload the page to try again.</p>"#
        ),
        _ => super::error_states::inline(what),
    };
    // Return 200 (not 503) so HTMX swaps this informative fragment into the
    // card; a non-2xx response leaves the "Loading..." placeholder stuck.
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], html)
}

#[cfg(feature = "analytics")]
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

#[cfg(all(test, feature = "analytics"))]
mod tests {
    use super::funnel_step_counts;
    use birdnet_behavioral::types::ChorusFunnel;

    /// The engine-configuration card waits for the analytics handle off the
    /// async runtime (see `status_waits_for_the_handle_off_the_runtime` in
    /// `routes::analytics` for the shape of the check).
    #[tokio::test(flavor = "current_thread")]
    async fn config_card_waits_for_the_handle_off_the_runtime() {
        use axum::response::IntoResponse as _;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new_with_analytics(
            dir.path().join("birds.db"),
            &dir.path().join("analytics.duckdb"),
        )
        .unwrap();
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let holder = {
            let state = state.clone();
            std::thread::spawn(move || {
                // Reads run on the analytics readers, so the check holds all of
                // them (there are two): a request now waits only for a reader.
                state.with_analytics_read(|_| {
                    state.with_analytics_read(|_| {
                        held_tx.send(()).unwrap();
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    });
                });
            })
        };
        held_rx.recv().unwrap();
        let ticked = Arc::new(AtomicBool::new(false));
        let other_task = {
            let ticked = Arc::clone(&ticked);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                ticked.store(true, Ordering::SeqCst);
            })
        };
        let _ = super::analytics_config_partial(axum::extract::State(state))
            .await
            .into_response();
        assert!(
            ticked.load(Ordering::SeqCst),
            "the runtime's only thread was blocked while the card waited for the handle"
        );
        other_task.await.unwrap();
        holder.join().unwrap();
    }

    /// The card names the most recent species and shows who came before and
    /// after it.
    ///
    /// Its trigger lookup ordered a view by `rowid`, which the bundled SQLite
    /// rejects; the error was swallowed and every station's card read "No
    /// detections yet." over any amount of history.
    #[tokio::test]
    async fn the_neighbour_card_renders_from_real_history() {
        use axum::response::IntoResponse as _;
        let dir = tempfile::tempdir().unwrap();
        let state = crate::state::AppState::new_with_analytics(
            dir.path().join("birds.db"),
            &dir.path().join("analytics.duckdb"),
        )
        .unwrap();
        if !state
            .with_analytics(birdnet_behavioral::connection::AnalyticsDb::extension_loaded)
            .unwrap()
        {
            birdnet_behavioral::gating::skip_or_fail(
                "the behavioral extension",
                "it did not load into the test store",
            );
            return;
        }
        // Three mornings of Wren -> Robin -> Blackbird; the latest detection
        // is a Robin, which is what the card asks about.
        state.with_db(|conn| {
            for day in 1..=3 {
                for (time, com, utc) in [
                    ("05:00:00", "Eurasian Wren", 0),
                    ("05:01:00", "European Robin", 60),
                    ("05:02:00", "Eurasian Blackbird", 120),
                ] {
                    conn.execute(
                        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) \
                         VALUES (?1, ?2, 'x', ?3, 0.9, ?4)",
                        rusqlite::params![
                            format!("2024-05-0{day}"),
                            time,
                            com,
                            1_714_539_600 + (day - 1) * 86_400 + utc
                        ],
                    )
                    .unwrap();
                }
            }
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) \
                 VALUES ('2024-05-04', '05:01:00', 'x', 'European Robin', 0.9, 1714799000)",
                [],
            )
            .unwrap();
        });
        state.resync_analytics_full().unwrap().unwrap();

        let response = super::analytics_next_partial(axum::extract::State(state))
            .await
            .into_response();
        let html = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!html.contains("No detections yet"), "{html}");
        assert!(
            html.contains("Before <strong>European Robin</strong>"),
            "{html}"
        );
        assert!(
            html.contains("After <strong>European Robin</strong>"),
            "{html}"
        );
        assert!(html.contains("<td>Eurasian Wren</td>"), "{html}");
        assert!(html.contains("<td>Eurasian Blackbird</td>"), "{html}");
    }

    fn cf(steps_completed: u32) -> ChorusFunnel {
        ChorusFunnel {
            date: "2026-06-21".into(),
            steps_completed,
            total_steps: 3,
            matched_species: Vec::new(),
        }
    }

    #[test]
    fn step_counts_are_non_increasing() {
        // Three mornings reached step >=1, two reached >=2, one reached >=3.
        let funnel = vec![cf(3), cf(2), cf(1)];
        assert_eq!(funnel_step_counts(&funnel, 3), vec![3, 2, 1]);
    }

    #[test]
    fn none_reaching_first_step_is_all_zero() {
        let funnel = vec![cf(0), cf(0)];
        assert_eq!(funnel_step_counts(&funnel, 3), vec![0, 0, 0]);
    }

    #[test]
    fn zero_total_steps_is_empty() {
        assert_eq!(funnel_step_counts(&[], 0), Vec::<u64>::new());
    }

    fn session(n: u32) -> birdnet_behavioral::types::ActivitySession {
        birdnet_behavioral::types::ActivitySession {
            species: "Great Tit".into(),
            session_id: 1,
            detection_count: n,
            start_time: "2026-05-01 05:00:00".into(),
            end_time: "2026-05-01 05:10:00".into(),
            duration_secs: 600,
        }
    }

    /// The footnote counts the bursts, not the singletons filtered out of the
    /// table, and says when the list was cut at the fetch limit.
    #[test]
    fn the_bursts_footnote_counts_bursts() {
        let mut sessions: Vec<_> = (0..25).map(|_| session(3)).collect();
        sessions.extend((0..75).map(|_| session(1)));
        let html = super::render_bursts(
            &sessions,
            30,
            100,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(
            html.contains(
                "Showing the 20 most recent of 25 bursts found among the latest 100 sessions."
            ),
            "{html}"
        );
        // Counterpart: under the limit, no claim about a cut.
        let html = super::render_bursts(
            &sessions[..30],
            30,
            100,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(html.contains("of 25 bursts.</p>"), "{html}");
    }

    /// The empty state names the gap the grouping actually used.
    #[test]
    fn the_empty_bursts_message_names_the_real_gap() {
        let html = super::render_bursts(
            &[session(1), session(1)],
            30,
            100,
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(html.contains("within 30 minutes"), "{html}");
        assert!(!html.contains("20 minutes"), "{html}");
    }

    /// The funnel's window spans the whole dawn filter it runs over.
    ///
    /// The filter is hours 4 to 8 *inclusive* — 04:00 to 08:59, five hours —
    /// and the funnel window was 240 minutes, so a morning whose run spanned
    /// the full dawn counted as "in order" in the headline (`sequence_count`,
    /// no gap limit) and fell short in the funnel drawn above it.
    #[test]
    fn the_funnel_window_covers_the_whole_dawn_filter() {
        assert_eq!(
            super::DAWN_FUNNEL_WINDOW_MINUTES,
            (super::DAWN_HOUR_END - super::DAWN_HOUR_START + 1) * 60
        );
    }

    /// The card says the dawn "opens" with the sequence, so it is ordered by
    /// when each voice *starts* on a typical morning, not by the mean time of
    /// all its song.
    ///
    /// The Robin is first up at 04:30 every morning and keeps singing until
    /// after eight; the Wren sings only from 05:00 to 05:20. By mean time the
    /// Wren came first; by who opens the morning, the Robin does.
    #[test]
    fn the_dawn_sequence_is_ordered_by_who_starts_first() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE detections_analytic (Date TEXT, Time TEXT, Com_Name TEXT);",
        )
        .expect("table");
        for day in 1..=5 {
            let date = format!("2026-05-{day:02}");
            let add = |time: &str, sp: &str| {
                conn.execute(
                    "INSERT INTO detections_analytic VALUES (?1, ?2, ?3)",
                    rusqlite::params![date, time, sp],
                )
                .expect("row");
            };
            add("04:30:00", "European Robin");
            for m in ["00", "10", "20", "30"] {
                add(&format!("08:{m}:00"), "European Robin");
            }
            for m in ["00", "05", "10", "20"] {
                add(&format!("05:{m}:00"), "Eurasian Wren");
            }
        }
        assert_eq!(
            super::derive_dawn_sequence(&conn).expect("read"),
            ["European Robin", "Eurasian Wren"]
        );
    }

    /// A station's dawn: seven regular voices (the seventh with exactly ten songs, the floor), one
    /// with nine, one that sang only in April, and one that sings at noon. `Bird A` to `Bird G`
    /// are heard 40, 35, 30, 25, 20, 15 and 10 times (a few a morning over five mornings in May)
    /// and open at 06:00, 05:00, 05:30, 04:30, 04:10, 06:30 and 05:15. The fifth most heard (Bird E) opens
    /// earliest of all, so it is in the three earliest whenever the pool is wide enough to hold it.
    fn a_dawn_with_seven_voices() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE detections_analytic (Date TEXT, Time TEXT, Com_Name TEXT);",
        )
        .expect("table");
        let sing = |date: &str, name: &str, (h, m): (u32, u32), n: u32| {
            for i in 0..n {
                let (hh, mm) = (h + (m + i) / 60, (m + i) % 60);
                conn.execute(
                    "INSERT INTO detections_analytic VALUES (?1, ?2, ?3)",
                    rusqlite::params![date, format!("{hh:02}:{mm:02}:00"), name],
                )
                .expect("row");
            }
        };
        let voices = [
            ("Bird A", (6, 0), 8),
            ("Bird B", (5, 0), 7),
            ("Bird C", (5, 30), 6),
            ("Bird D", (4, 30), 5),
            ("Bird E", (4, 10), 4),
            ("Bird F", (6, 30), 3),
            ("Bird G", (5, 15), 2),
        ];
        for day in 1..=5 {
            for (name, at, per_day) in voices {
                sing(&format!("2026-05-{day:02}"), name, at, per_day);
            }
        }
        sing("2026-05-01", "Bird H", (5, 10), 9);
        sing("2026-04-01", "Old Bird", (5, 0), 50);
        sing("2026-05-01", "Noon Bird", (12, 0), 60);
        conn
    }

    /// The six regular voices of the season, the most heard, in the order they open: the seventh
    /// (ten songs) is the one left out of a pool of six and in a pool of seven, the voice with nine
    /// songs is never a regular, the April bird is not in May's season and the noon bird is not a
    /// dawn voice at all.
    #[test]
    fn the_regular_voices_are_the_most_heard_in_the_order_they_open() {
        let conn = a_dawn_with_seven_voices();
        let season = Some("2026-05-01");
        assert_eq!(
            super::derive_dawn_voices(&conn, season, 6, 6).expect("read"),
            ["Bird E", "Bird D", "Bird B", "Bird C", "Bird A", "Bird F"],
            "the six most heard, by who opens first"
        );
        assert_eq!(
            super::derive_dawn_voices(&conn, season, 7, 7).expect("read"),
            [
                "Bird E", "Bird D", "Bird B", "Bird G", "Bird C", "Bird A", "Bird F"
            ],
            "ten songs is a regular voice; nine (Bird H) is not"
        );
        // A bigger pool than there are regulars is not padded.
        assert_eq!(
            super::derive_dawn_voices(&conn, season, 20, 20)
                .expect("read")
                .len(),
            7
        );
    }

    /// What the season is: from `since` on, and everything for `None`, where the April bird heard
    /// fifty times is the most heard of all.
    #[test]
    fn the_season_is_from_the_date_given_and_none_is_all_history() {
        let conn = a_dawn_with_seven_voices();
        let all = super::derive_dawn_voices(&conn, None, 6, 6).expect("read");
        assert!(all.contains(&"Old Bird".to_owned()), "{all:?}");
        assert!(
            !all.contains(&"Bird F".to_owned()),
            "pushed out of the six: {all:?}"
        );
        let may = super::derive_dawn_voices(&conn, Some("2026-05-01"), 6, 6).expect("read");
        assert!(!may.contains(&"Old Bird".to_owned()), "{may:?}");
        // The boundary day is in: `since` is inclusive.
        let from_april = super::derive_dawn_voices(&conn, Some("2026-04-01"), 6, 6).expect("read");
        assert!(
            from_april.contains(&"Old Bird".to_owned()),
            "{from_april:?}"
        );
        let after_april = super::derive_dawn_voices(&conn, Some("2026-04-02"), 6, 6).expect("read");
        assert!(
            !after_april.contains(&"Old Bird".to_owned()),
            "{after_april:?}"
        );
        // Never the noon bird, whatever the season.
        assert!(!all.iter().chain(&may).any(|n| n == "Noon Bird"));
    }

    /// The running order's answer did not move when the derivation was shared: the three earliest
    /// of the five most heard, over all history. Ties in opening time go by name.
    #[test]
    fn the_running_order_still_takes_the_three_earliest_of_the_five_most_heard() {
        let conn = a_dawn_with_seven_voices();
        // Most heard over all history: Old Bird 50, A 40, B 35, C 30, D 25. By opening: D 04:30,
        // then B and Old Bird together at 05:00 (B first by name), then C, then A.
        assert_eq!(
            super::derive_dawn_sequence(&conn).expect("read"),
            ["Bird D", "Bird B", "Old Bird"]
        );
    }

    /// Two voices heard equally often: the one first by name makes the pool.
    ///
    /// The query asks for that explicitly (`ORDER BY c DESC, Com_Name ASC`), so it holds by contract
    /// and not by the accident of an order. This test pins the answer; it cannot tell the explicit
    /// tie-break from its absence, because on the bundled SQLite the grouping already yields names in
    /// order (the mutant without it passed, and was judged equivalent, not a gap).
    #[test]
    fn equal_volumes_are_ranked_by_name() {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        conn.execute_batch(
            "CREATE TABLE detections_analytic (Date TEXT, Time TEXT, Com_Name TEXT);",
        )
        .expect("table");
        for name in ["Zebra Finch", "Aardvark Lark"] {
            for i in 0..12 {
                conn.execute(
                    "INSERT INTO detections_analytic VALUES ('2026-05-01', ?1, ?2)",
                    rusqlite::params![format!("05:{i:02}:00"), name],
                )
                .expect("row");
            }
        }
        assert_eq!(
            super::derive_dawn_voices(&conn, None, 1, 1).expect("read"),
            ["Aardvark Lark"]
        );
    }

    /// A read that fails is an error, not an empty order: the card read
    /// "Not enough dawn activity yet" for a station whose detections could
    /// not be read at all. Observed failing with the old
    /// `let Ok(..) else { return Vec::new() }` restored (as `Ok(Vec::new())`).
    #[test]
    fn a_dawn_sequence_that_cannot_be_read_is_an_error() {
        let conn = rusqlite::Connection::open_in_memory().expect("db");
        assert!(super::derive_dawn_sequence(&conn).is_err());
    }
}
