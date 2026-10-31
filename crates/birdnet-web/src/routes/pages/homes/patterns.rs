//! The **Patterns** home (`/patterns`) — "when & where?".
//!
//! Folds the six pre-spine analytics surfaces into one home with a
//! plain-English tab per question (v3 spine, `Patterns_home.html`):
//!
//! | Tab       | Question                | Folded surface (old route)        |
//! |-----------|-------------------------|-----------------------------------|
//! | when      | when are they out?      | activity heatmap (`/heatmap`)     |
//! | dawn      | who sings, and when?    | dawn chorus (`/analytics/dawn-chorus`) |
//! | migration | arriving & leaving      | phenology (`/migration`)          |
//! | together  | who co-occurs?          | co-occurrence (`/correlation`)    |
//! | trends    | busier or quieter?      | time series (`/timeseries`)       |
//! | behavior  | how long, how often?    | behavioral analytics (`/analytics`): visits |
//! | sequences | who follows whom?       | the same engine's ordered questions |
//!
//! The old routes 308 here (see `routes::redirects`); the tab bodies are the
//! same renderers and HTMX partials those pages always used.
//!
//! The bar groups the tabs under three labels, so a reader chooses a kind of question before a
//! page: **When** (when active, the dawn chorus, trends), **Who** (who sings together, migration)
//! and **How** (visits, sequences). The groups change what the bar looks like and nothing about
//! an address: every `?tab=` key is the one it always was, and `behavior` keeps meaning the
//! bursts-and-returns page it opened before the engine's ordered questions got a tab of their own.

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{Group, SubTab, grouped_subtabs, resolve_tab};
use crate::state::AppState;

/// The Patterns tab table, in display order. Keys are the `?tab=` values the
/// redirect table targets — changing one is a link-breaking change.
pub const TABS: &[SubTab] = &[
    SubTab {
        key: "when",
        label: "When active",
        question: "when are they out?",
    },
    SubTab {
        key: "dawn",
        label: "Dawn chorus",
        question: "who sings, and when?",
    },
    SubTab {
        key: "trends",
        label: "Trends",
        question: "busier or quieter?",
    },
    SubTab {
        key: "together",
        label: "Who sings together",
        question: "who co-occurs?",
    },
    SubTab {
        key: "migration",
        label: "Migration",
        question: "arriving & leaving",
    },
    SubTab {
        key: "behavior",
        label: "Visits",
        question: "how long, how often?",
    },
    SubTab {
        key: "sequences",
        label: "Sequences",
        question: "who follows whom?",
    },
];

/// The bar's clusters, in the order they are drawn.
///
/// Every key of [`TABS`] is in exactly one (`every_tab_is_in_exactly_one_group` holds it), so a
/// tab added to the table and forgotten here fails a test instead of vanishing from the bar.
pub const GROUPS: &[Group] = &[
    Group {
        label: "When",
        keys: &["when", "dawn", "trends"],
    },
    Group {
        label: "Who",
        keys: &["together", "migration"],
    },
    Group {
        label: "How",
        keys: &["behavior", "sequences"],
    },
];

/// Mount `/patterns`.
pub fn router() -> Router<AppState> {
    Router::new().route("/patterns", get(patterns_page))
}

/// The "When active" tab: its own head and range control, then the soundscape terrain as the
/// lead figure, then the three charts that zoom in. One range sets all four.
fn when_content() -> String {
    crate::routes::pages::heatmap::content(&crate::routes::pages::terrain::content())
}

/// `?tab=` selector for the Patterns home.
#[derive(Deserialize)]
struct TabParam {
    tab: Option<String>,
}

async fn patterns_page(Query(q): Query<TabParam>, headers: HeaderMap) -> Html<String> {
    let tab = resolve_tab(TABS, q.tab.as_deref());
    let body = match tab.key {
        "dawn" => crate::routes::pages::dawn_chorus::content(),
        "migration" => crate::routes::pages::migration::content(),
        "together" => crate::routes::pages::correlation::content(),
        "trends" => crate::routes::pages::timeseries_dash::content(),
        "behavior" => crate::routes::pages::behavioral::content(),
        "sequences" => crate::routes::pages::behavioral::sequences_content(),
        // "when" and the unknown-key clamp.
        _ => when_content(),
    };
    // The provenance note sits above the tabs, not inside one: every Patterns
    // tab is a location- or hour-dependent reading, so a merged history changes
    // all of them. It renders nothing unless a genuinely different site was
    // imported.
    let content = format!(
        "{}{}{body}",
        crate::routes::pages::provenance::slot(),
        grouped_subtabs("/patterns", "tab", TABS, GROUPS, tab.key)
    );
    let title = format!("Patterns · {}", tab.label);
    crate::routes::pages::render_page_for_request(&title, &content, "patterns", &headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_keys_match_the_redirect_targets() {
        // The redirect table in `routes::redirects` points old analytics
        // bookmarks at these exact keys; renaming one silently strands the
        // redirect on the clamped first tab. Adding one is not a link-breaking
        // change, and the order is the table's, which the bar's groups arrange.
        let keys: Vec<&str> = TABS.iter().map(|t| t.key).collect();
        assert_eq!(
            keys,
            vec![
                "when",
                "dawn",
                "trends",
                "together",
                "migration",
                "behavior",
                "sequences"
            ]
        );
    }

    /// The groups are the bar: a tab in none of them would not be drawn, and one in two would be
    /// drawn twice.
    #[test]
    fn every_tab_is_in_exactly_one_group() {
        for t in TABS {
            let n = GROUPS.iter().filter(|g| g.keys.contains(&t.key)).count();
            assert_eq!(n, 1, "{} is in {n} groups", t.key);
        }
        for g in GROUPS {
            for k in g.keys {
                assert!(TABS.iter().any(|t| t.key == *k), "{k} is not a tab");
            }
        }
        let labels: Vec<&str> = GROUPS.iter().map(|g| g.label).collect();
        assert_eq!(labels, ["When", "Who", "How"]);
    }

    /// An old link, an unknown key and a bare `/patterns` all still land somewhere, and the bar
    /// marks the page you are on once, whichever group it is in.
    #[test]
    fn the_grouped_bar_keeps_every_address_and_marks_the_page() {
        let bar = grouped_subtabs("/patterns", "tab", TABS, GROUPS, "sequences");
        assert_eq!(bar.matches("bnb-subtab active").count(), 1, "{bar}");
        assert!(
            bar.contains(r#"href="/patterns""#),
            "the bare address is the first tab"
        );
        for t in &TABS[1..] {
            assert!(
                bar.contains(&format!(r#"href="/patterns?tab={}""#, t.key)),
                "{} keeps its address",
                t.key
            );
        }
        assert_eq!(resolve_tab(TABS, Some("behavior")).key, "behavior");
        assert_eq!(resolve_tab(TABS, Some("sequences")).key, "sequences");
        assert_eq!(resolve_tab(TABS, Some("nonsense")).key, "when");
    }

    /// The terrain is the "When active" tab's lead figure: once, after the head and before the
    /// three charts that zoom in.
    #[test]
    fn the_terrain_leads_the_when_tab_once() {
        let html = when_content();
        assert_eq!(html.matches(r#"id="terrain""#).count(), 1);
        let head = html
            .find("When the yard is <em>alive</em>")
            .expect("the tab's heading");
        let terrain = html.find(r#"id="terrain""#).expect("the terrain card");
        let stream = html
            .find("Activity streamgraph")
            .expect("the streamgraph card");
        let hours = html.find("Detections by hour").expect("the hours card");
        let grid = html.find("Activity grid").expect("the grid card");
        assert!(
            head < terrain && terrain < stream && stream < hours && hours < grid,
            "head, terrain, who, which hours, which weekdays"
        );
    }

    /// One range control, not two: the terrain follows the page's, and has none of its own.
    #[test]
    fn the_when_tab_has_one_range_control() {
        let html = when_content();
        assert_eq!(
            html.matches(r#"role="group" aria-label="Time range"#)
                .count(),
            1
        );
        assert!(
            !html.contains(r#"data-tr="range""#),
            "the terrain's own range control is gone"
        );
        for days in [7, 14, 30, 90, 365] {
            assert!(html.contains(&format!(r#"data-days="{days}""#)), "{days}");
        }
    }
}
