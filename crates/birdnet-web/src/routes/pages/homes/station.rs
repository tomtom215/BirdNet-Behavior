//! The **Station** home (`/station`) — "manage my station".
//!
//! Health-first (v3 spine, `Station_home.html`): the public Health tab folds
//! the old read-only `/system` page so "is it working?" stays checkable from
//! the field without a login, exactly as `/system` always was. The five
//! management groups (Capture · Alerts · Data · Settings · Access) regroup
//! the twelve flat `/admin/*` pages by task; they are gated behind the same
//! admin auth as ever and currently link to their `/admin` homes (the Wave B2
//! regroup re-hosts them under `/station/...`).

use axum::http::HeaderMap;
use axum::response::Html;
use axum::{Router, routing::get};

use super::{SubTab, subtabs};
use crate::state::AppState;

/// The six Station task groups, in display order. Health is the only public
/// tab; the rest link into the gated admin area.
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
        key: "settings",
        label: "Settings",
        question: "my preferences",
    },
    SubTab {
        key: "access",
        label: "Access",
        question: "who can get in",
    },
];

/// Where each gated task group currently lives. Wave B2 replaces these with
/// `/station/<key>` pages; until then the tabs deep-link into the existing
/// admin shell so the IA is already navigable.
const GATED_HOMES: &[(&str, &str)] = &[
    ("capture", "/admin/audio"),
    ("alerts", "/admin/rules"),
    ("data", "/admin/backups"),
    ("settings", "/admin/settings"),
    ("access", "/admin/accounts"),
];

/// Mount `/station`.
pub fn router() -> Router<AppState> {
    Router::new().route("/station", get(station_page))
}

/// Render the Station tab row with Health active and the gated groups linking
/// to their current admin homes.
fn station_tabs() -> String {
    let mut html = subtabs("/station", "tab", TABS, "health");
    for (key, admin_path) in GATED_HOMES {
        html = html.replace(
            &format!("href=\"/station?tab={key}\""),
            &format!("href=\"{admin_path}\""),
        );
    }
    html
}

async fn station_page(headers: HeaderMap) -> Html<String> {
    let content = format!(
        "{}{}",
        station_tabs(),
        crate::routes::pages::system_dashboard::content()
    );
    crate::routes::pages::render_page_for_request("Station", &content, "station", &headers)
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
    fn gated_tabs_link_into_the_admin_area() {
        let html = station_tabs();
        // Health is the active, canonical tab…
        assert!(html.contains(r#"href="/station""#));
        assert_eq!(html.matches("bnb-subtab active").count(), 1);
        // …and every management group resolves to a real admin page, never a
        // dangling /station?tab= placeholder.
        for (_, admin_path) in GATED_HOMES {
            assert!(html.contains(admin_path), "{admin_path} missing");
        }
        assert!(!html.contains("/station?tab="));
    }
}
