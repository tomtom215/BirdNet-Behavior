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
pub mod station_tabs;

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

/// A labelled cluster of a home's tabs, named by key.
///
/// The label says what kind of question they answer: the bar says "When", "Who", "How" over the
/// tabs under it, so a reader choosing among seven tabs chooses a question first.
#[derive(Debug)]
pub struct Group {
    /// The cluster's name, one short word.
    pub label: &'static str,
    /// The keys of its tabs, in display order; each is a [`SubTab::key`] of the home.
    pub keys: &'static [&'static str],
}

/// Render one tab link.
fn tab_link(out: &mut String, base: &str, param: &str, i: usize, t: &SubTab, active: &str) {
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

/// Render a home's sub-tab row, its tabs clustered under [`Group`] labels.
///
/// The same links as [`subtabs`] (the first tab in `tabs`, which need not be the first in its
/// group, is the one with the bare canonical URL), in one scrolling strip, each cluster a
/// `role="group"` named by its label.
///
/// A tab no group names is left out, which is why the home's own tests hold that every key is in
/// exactly one group.
pub fn grouped_subtabs(
    base: &str,
    param: &str,
    tabs: &[SubTab],
    groups: &[Group],
    active: &str,
) -> String {
    let mut out = String::with_capacity(1536);
    let _ = write!(
        out,
        r#"<nav class="bnb-subtabs bnb-subtabs--grouped" aria-label="Views" data-screen-label="Sub-tabs">"#
    );
    for g in groups {
        let _ = write!(
            out,
            r#"<div class="bnb-subgroup" role="group" aria-label="{label}"><span class="bnb-subgroup-l" aria-hidden="true">{label}</span><div class="bnb-subgroup-tabs">"#,
            label = g.label
        );
        for key in g.keys {
            if let Some((i, t)) = tabs.iter().enumerate().find(|(_, t)| t.key == *key) {
                tab_link(&mut out, base, param, i, t, active);
            }
        }
        out.push_str("</div></div>");
    }
    out.push_str("</nav>");
    out
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
        tab_link(&mut out, base, param, i, t, active);
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

    const GROUPS: &[Group] = &[
        Group {
            label: "First",
            keys: &["two"],
        },
        Group {
            label: "Second",
            keys: &["one"],
        },
    ];

    /// The same links as the flat bar, under their labels, with the bare URL still on the first
    /// tab of the *table* (so a stale bookmark and the canonical address agree) and the active tab
    /// marked once.
    #[test]
    fn grouped_subtabs_cluster_the_same_links_under_labels() {
        let html = grouped_subtabs("/x", "tab", TABS, GROUPS, "two");
        assert_eq!(html.matches(r#"role="group""#).count(), 2, "{html}");
        assert!(html.contains(r#"aria-label="First""#) && html.contains(r#"aria-label="Second""#));
        assert_eq!(html.matches("bnb-subtab active").count(), 1, "{html}");
        assert!(
            html.contains(r#"href="/x""#),
            "the first tab of the table is the bare URL"
        );
        assert!(html.contains(r#"href="/x?tab=two""#));
        // Display order is the groups' order, not the table's: "two" comes before "one".
        assert!(
            html.find("?tab=two").unwrap_or(usize::MAX) < html.find(r#"href="/x""#).unwrap_or(0),
            "{html}"
        );
        // The label is for the eye; the group carries the name for a screen reader.
        assert_eq!(html.matches(r#"aria-hidden="true">First<"#).count(), 1);
    }

    /// A tab that no group names is not drawn: the bar never shows a link the groups do not mean.
    #[test]
    fn a_tab_no_group_names_is_not_drawn() {
        let html = grouped_subtabs(
            "/x",
            "tab",
            TABS,
            &[Group {
                label: "Only",
                keys: &["two"],
            }],
            "two",
        );
        assert_eq!(html.matches("<a ").count(), 1, "{html}");
    }

    #[test]
    fn resolve_tab_clamps_unknown_to_first() {
        assert_eq!(resolve_tab(TABS, Some("two")).key, "two");
        assert_eq!(resolve_tab(TABS, Some("nope")).key, "one");
        assert_eq!(resolve_tab(TABS, None).key, "one");
    }
}
