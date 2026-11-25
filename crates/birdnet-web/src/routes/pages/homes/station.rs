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
use crate::routes::pages::help::Topic;
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

/// The opening of every Station tab: the sub-tab row with `active` marked, then the shared page
/// head ([`crate::routes::pages::frame::page_head`]) with the tab's own lede.
///
/// Health links the canonical bare `/station`; the five management groups link
/// their `/station/<key>` sub-routes (real server-rendered pages, so this is a
/// `<nav>` of links, not a JS tablist). Shared by the public Health page and the
/// gated tab handlers so the row is identical across the home.
///
/// The head is emitted here rather than per tab so it cannot be forgotten by the next one
/// added: every Station tab composes its content from this plus a fragment, and none of the
/// fragments carries a page heading. The six screens were once served with no `<h1>` at all,
/// then with a screen-reader-only one, while every other home opened with the shared head
/// (UIR-27). `lead` is text and `lede` authored markup, as [`Head`](crate::routes::pages::frame::Head)
/// takes them; both may be empty.
pub(crate) fn station_head(active: &str, lead: &str, lede: &str) -> String {
    // Named as the navigation names the section ("Settings"), read from the nav table so a
    // rename cannot leave this heading behind: it said "Station — Health" for a section whose
    // nav entry, page title and breadcrumbs all said "Settings".
    let section = crate::routes::pages::nav::PRIMARY
        .iter()
        .find(|p| p.key == "station")
        .map_or("Settings", |p| p.label);
    let title = TABS
        .iter()
        .find(|t| t.key == active)
        .map_or(section, |t| t.label);
    let topic = match active {
        "capture" => Topic::AdminAudio,
        "alerts" => Topic::AdminNotifications,
        "data" => Topic::AdminBackups,
        "settings" => Topic::AdminSettings,
        "access" => Topic::AdminRemoteAccess,
        _ => Topic::AdminSystem,
    };
    let head = crate::routes::pages::frame::page_head(&crate::routes::pages::frame::Head {
        eyebrow: section,
        help: &crate::routes::pages::help::help_link(topic),
        title: &crate::routes::pages::escape_html(title),
        lead,
        lede,
        aside: "",
    });
    format!("{}{head}", station_subtabs(active))
}

/// The Station sub-tab row alone, with `active` marked.
pub(crate) fn station_subtabs(active: &str) -> String {
    let mut out = String::with_capacity(1024);
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
        station_head(
            "health",
            "Everything your station needs to keep listening",
            "— the streams, the hardware, and the pipeline behind them. This is the screen to check from the field.",
        ),
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
