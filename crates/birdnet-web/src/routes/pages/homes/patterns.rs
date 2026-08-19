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
//! | behavior  | the deep tier           | behavioral analytics (`/analytics`) |
//!
//! The old routes 308 here (see `routes::redirects`); the tab bodies are the
//! same renderers and HTMX partials those pages always used.

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{SubTab, resolve_tab, subtabs};
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
        key: "migration",
        label: "Migration",
        question: "arriving & leaving",
    },
    SubTab {
        key: "together",
        label: "Who sings together",
        question: "who co-occurs?",
    },
    SubTab {
        key: "trends",
        label: "Trends",
        question: "busier or quieter?",
    },
    SubTab {
        key: "behavior",
        label: "Behavior",
        question: "the deep tier",
    },
];

/// Mount `/patterns`.
pub fn router() -> Router<AppState> {
    Router::new().route("/patterns", get(patterns_page))
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
        // "when" and the unknown-key clamp.
        _ => crate::routes::pages::heatmap::content(),
    };
    // The provenance note sits above the tabs, not inside one: every Patterns
    // tab is a location- or hour-dependent reading, so a merged history changes
    // all of them. It renders nothing unless a genuinely different site was
    // imported.
    let content = format!(
        "{}{}{body}",
        crate::routes::pages::provenance::slot(),
        subtabs("/patterns", "tab", TABS, tab.key)
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
        // redirect on the clamped first tab.
        let keys: Vec<&str> = TABS.iter().map(|t| t.key).collect();
        assert_eq!(
            keys,
            vec![
                "when",
                "dawn",
                "migration",
                "together",
                "trends",
                "behavior"
            ]
        );
    }
}
