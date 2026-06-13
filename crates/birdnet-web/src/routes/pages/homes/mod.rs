//! The v3-spine **homes**: tabbed shells that fold the pre-spine standalone
//! pages into six top-level destinations (see
//! `docs/design/handover/v3_spine/HANDOFF_v3.html`).
//!
//! | Module     | Home        | Folds                                                  |
//! |------------|-------------|--------------------------------------------------------|
//! | `patterns` | /patterns   | heatmap · dawn chorus · migration · co-occurrence · time series · behavioral |
//! | `reports`  | /reports    | weekly report · year in review · history               |
//! | `station`  | /station    | system health (public) + the admin task groups         |
//!
//! Today (`/`), Species (`/species`) and Recordings (`/recordings`) are homes
//! too, but they live in their existing modules (`dashboard`/`today`,
//! `species_pages`, `recordings`) rather than here, because they re-compose
//! those modules' own partials instead of embedding other pages.
//!
//! Each home is a row of sub-tabs (`subtabs`) over server-rendered tab bodies
//! selected by a `?tab=`/`?view=` query parameter — query parameters rather
//! than the mockups' `#fragments`, because fragments never reach the server
//! and the tab bodies are real server-rendered pages (a Pi shouldn't render
//! six analytics surfaces to serve one).

pub mod patterns;
pub mod reports;
pub mod station;

use std::fmt::Write as _;

use axum::Router;

use crate::state::AppState;

/// One sub-tab in a home's tab row: a label over a small italic "question"
/// line, exactly the treatment the v3 mockups use (`.pt-tab`/`.st-tab` …,
/// consolidated here as `.bnb-subtab`).
#[derive(Debug)]
pub struct SubTab {
    /// Value carried in the query parameter; also the stable test key.
    pub key: &'static str,
    /// Tab label.
    pub label: &'static str,
    /// The plain-English question the tab answers (small second line).
    pub question: &'static str,
}

/// Render a home's sub-tab row. The active tab is marked both visually and
/// with `aria-current`; the first tab links to the bare `base` path (its
/// canonical URL), the rest carry `?{param}={key}`.
///
/// These are real links (server-rendered tabs), so the row is a `<nav>`, not
/// a JS `tablist` like the static mockups.
pub fn subtabs(base: &str, param: &str, tabs: &[SubTab], active: &str) -> String {
    let mut out = String::with_capacity(1024);
    let _ = write!(
        out,
        r#"<nav class="bnb-subtabs" aria-label="Views" data-screen-label="Sub-tabs">"#
    );
    for (i, t) in tabs.iter().enumerate() {
        let href = if i == 0 {
            base.to_string()
        } else {
            format!("{base}?{param}={}", t.key)
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

/// Resolve the requested tab key against a home's tab table, falling back to
/// the first tab for a missing or unknown value (never a 404 — a stale
/// bookmark should land somewhere sensible).
pub fn resolve_tab<'t>(tabs: &'t [SubTab], requested: Option<&str>) -> &'t SubTab {
    requested
        .and_then(|want| tabs.iter().find(|t| t.key == want))
        .unwrap_or(&tabs[0])
}

/// Mount the home pages built in this module.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(patterns::router())
        .merge(reports::router())
        .merge(station::router())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABS: &[SubTab] = &[
        SubTab {
            key: "one",
            label: "One",
            question: "first?",
        },
        SubTab {
            key: "two",
            label: "Two",
            question: "second?",
        },
    ];

    #[test]
    fn subtabs_mark_exactly_the_active_tab() {
        let html = subtabs("/x", "tab", TABS, "two");
        assert_eq!(html.matches("bnb-subtab active").count(), 1);
        assert!(html.contains(r#"aria-current="page""#));
        // First tab links to the canonical bare path; later tabs carry the param.
        assert!(html.contains(r#"href="/x""#));
        assert!(html.contains(r#"href="/x?tab=two""#));
    }

    #[test]
    fn resolve_tab_clamps_unknown_to_first() {
        assert_eq!(resolve_tab(TABS, Some("two")).key, "two");
        assert_eq!(resolve_tab(TABS, Some("nope")).key, "one");
        assert_eq!(resolve_tab(TABS, None).key, "one");
    }
}
