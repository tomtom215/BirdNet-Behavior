//! The **Station** home (`/station`) — "manage my station".
//!
//! Health-first (v3 spine, `Station_home.html`): the public Health tab is the
//! operator-grade "is it working?" surface ([`super::super::station_health`]) —
//! the heir to the old read-only `/system` page, still checkable from the field
//! without a login. The five management groups (Capture · Alerts · Data ·
//! Settings · Access) regroup the twelve flat `/admin/*` pages by task; they are
//! gated behind the same admin auth as ever (their handlers live in
//! [`super::station_tabs`], mounted inside the admin router) and are hosted at
//! `/station/<key>` sub-routes.

use std::fmt::Write as _;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};

use super::SubTab;
use crate::state::AppState;

/// The six Station task groups, in display order. Health is the only public
/// tab; the rest are gated (see [`super::station_tabs`]).
pub const TABS: &[SubTab] = &[
    SubTab {
        key: "health",
        label: "Health",
        question: "is it working?",
    },
    SubTab {
        key: "capture",
        label: "Capture",
        question: "what am I recording?",
    },
    SubTab {
        key: "alerts",
        label: "Alerts",
        question: "tell me when…",
    },
    SubTab {
        key: "data",
        label: "Data",
        question: "keep it safe",
    },
    SubTab {
        // "General", not "Settings": the section itself is now called Settings,
        // and a Settings → Settings breadcrumb tells the reader nothing. The
        // route key stays `settings` so `/station/settings` keeps working.
        key: "settings",
        label: "General",
        question: "my preferences",
    },
    SubTab {
        key: "access",
        label: "Access",
        question: "who can get in",
    },
];

/// Mount `/station` (the public Health tab). The gated `/station/<key>` tabs are
/// mounted inside the admin router (see [`super::station_tabs::router`]).
pub fn router() -> Router<AppState> {
    Router::new().route("/station", get(station_page))
}

/// Render the Station sub-tab row with `active` marked.
///
/// Health links the canonical bare `/station`; the five management groups link
/// their `/station/<key>` sub-routes (real server-rendered pages, so this is a
/// `<nav>` of links, not a JS tablist). Shared by the public Health page and the
/// gated tab handlers so the row is identical across the home.
pub(crate) fn station_subtabs(active: &str) -> String {
    let mut out = String::with_capacity(1024);
    // Every Station tab composes its content from a sub-tab strip plus a
    // fragment, and none of those fragments carries a page heading, so the six
    // Station screens were the only ones in the app served with no `<h1>` at
    // all — their first heading was an `<h2 class="st-h3">`. A screen reader
    // announcing the page had nothing to announce it as, and the heading order
    // started at level 2. Emitted here rather than per tab so it cannot be
    // forgotten by the next one added.
    let label = TABS
        .iter()
        .find(|t| t.key == active)
        .map_or("Station", |t| t.label);
    let _ = write!(out, r#"<h1 class="sr-only">Station — {label}</h1>"#);
    out.push_str(r#"<nav class="bnb-subtabs" aria-label="Views" data-screen-label="Sub-tabs">"#);
    for t in TABS {
        let href = if t.key == "health" {
            "/station".to_string()
        } else {
            format!("/station/{}", t.key)
        };
        let (cls, cur) = if t.key == active {
            (" active", r#" aria-current="page""#)
        } else {
            ("", "")
        };
        let _ = write!(
            out,
            r#"<a class="bnb-subtab{cls}" href="{href}"{cur}><span class="l">{}</span><span class="q">{}</span></a>"#,
            t.label, t.question
        );
    }
    out.push_str("</nav>");
    out
}

async fn station_page(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let content = format!(
        "{}{}",
        station_subtabs("health"),
        crate::routes::pages::station_health::content(&state).await
    );
    crate::routes::pages::render_page_for_request("Settings", &content, "station", &headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_task_groups_in_order() {
        let keys: Vec<&str> = TABS.iter().map(|t| t.key).collect();
        assert_eq!(
            keys,
            vec!["health", "capture", "alerts", "data", "settings", "access"]
        );
    }

    #[test]
    fn subtabs_mark_exactly_the_active_tab_and_link_sub_routes() {
        let html = station_subtabs("capture");
        // Exactly one tab is active, and it carries aria-current.
        assert_eq!(html.matches("bnb-subtab active").count(), 1);
        assert!(html.contains(r#"aria-current="page""#));
        // Health is the canonical bare path; the gated groups are sub-routes.
        assert!(html.contains(r#"href="/station""#));
        assert!(html.contains(r#"href="/station/capture""#));
        assert!(html.contains(r#"href="/station/alerts""#));
        assert!(html.contains(r#"href="/station/data""#));
        assert!(html.contains(r#"href="/station/settings""#));
        assert!(html.contains(r#"href="/station/access""#));
        // Never the legacy `?tab=` placeholder.
        assert!(!html.contains("/station?tab="));
    }

    #[test]
    fn health_is_the_default_active_tab() {
        let html = station_subtabs("health");
        assert!(html.contains(r#"href="/station" aria-current="page""#));
    }
}
