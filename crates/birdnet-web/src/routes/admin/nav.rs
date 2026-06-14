//! Single source of truth for the admin panel's sub-navigation.
//!
//! Every admin destination is declared **once** in [`ADMIN_NAV`]. The admin
//! shell's nav row ([`nav_links`]) and the breadcrumb trail ([`breadcrumb`])
//! are both generated from it, so they can't drift the way the previously
//! hand-maintained per-page navs did — e.g. the standalone Settings / System /
//! Migration pages each carried their own four-link `<nav>` (Dashboard /
//! Settings / Migration / System) that disagreed with the shell's ten-link
//! nav, and Migration was missing from the shell entirely.
//!
//! Active-state is derived from the page's key — the same key each page passes
//! to `super::admin_shell` — so the highlight can't disagree with the menu it
//! highlights. `admin_router_serves_every_nav_destination` parity-tests that
//! every entry here resolves to a real admin route.

use std::fmt::Write as _;

/// One admin destination: a tab in the shell's nav row, a breadcrumb target,
/// and a route the parity test guards.
#[derive(Debug)]
pub struct AdminNav {
    /// Stable key matching the page's `active` argument to `super::admin_shell`.
    pub key: &'static str,
    /// The route this tab links to (a real `GET` route under `/admin`).
    pub path: &'static str,
    /// The human label shown in the nav and breadcrumb.
    pub label: &'static str,
    /// The v3-spine Station task group this destination belongs to.
    ///
    /// One of Health · Capture · Alerts · Data · Settings · Access; the shell
    /// nav renders one labelled cluster per group, matching the Station tabs.
    pub group: &'static str,
}

/// The admin sub-navigation, ordered by Station task group.
///
/// The shell renders one labelled cluster per group. Adding a page is a
/// one-line edit here; the parity test then requires a matching route, and the
/// grouping test requires each group's entries to stay contiguous.
pub const ADMIN_NAV: &[AdminNav] = &[
    // ── Health · is it working? ──
    AdminNav {
        key: "overview",
        path: "/admin/overview",
        label: "Overview",
        group: "Health",
    },
    AdminNav {
        key: "system",
        path: "/admin/system",
        label: "System",
        group: "Health",
    },
    AdminNav {
        key: "doctor",
        path: "/admin/doctor",
        label: "Diagnostics",
        group: "Health",
    },
    // ── Capture · what am I recording? ──
    AdminNav {
        key: "audio",
        path: "/admin/audio",
        label: "Audio",
        group: "Capture",
    },
    // Managing which species are detected/excluded is a core station function —
    // a first-class tab so a non-technical operator finds it in the menu rather
    // than only via a quick-link. Its Filter-test page is a sub-page beneath it.
    AdminNav {
        key: "species",
        path: "/admin/species",
        label: "Species",
        group: "Capture",
    },
    // ── Alerts · tell me when… ──
    AdminNav {
        key: "rules",
        path: "/admin/rules",
        label: "Rules",
        group: "Alerts",
    },
    AdminNav {
        key: "notifications",
        path: "/admin/notifications",
        label: "Notifications",
        group: "Alerts",
    },
    // ── Data · keep it safe / bring it in ──
    AdminNav {
        key: "backups",
        path: "/admin/backups",
        label: "Backups",
        group: "Data",
    },
    // Migration was reachable from the standalone pages' bespoke navs but was
    // absent from the shared shell nav — folding the pages in surfaces it here.
    AdminNav {
        key: "migrate",
        path: "/admin/migrate",
        label: "Migration",
        group: "Data",
    },
    AdminNav {
        key: "quality",
        path: "/admin/quality",
        label: "Quality",
        group: "Data",
    },
    // ── Settings · my preferences ──
    AdminNav {
        key: "settings",
        path: "/admin/settings",
        label: "Settings",
        group: "Settings",
    },
    // ── Access · who can get in ──
    AdminNav {
        key: "accounts",
        path: "/admin/accounts",
        label: "Accounts",
        group: "Access",
    },
];

/// The Station task groups, in display order. The shell nav renders one
/// labelled cluster per group from [`ADMIN_NAV`]; this is the canonical order.
pub const GROUPS: &[&str] = &["Health", "Capture", "Alerts", "Data", "Settings", "Access"];

/// Render the admin nav, grouped into labelled Station task clusters.
///
/// The tab whose key equals `active` is marked; entries are emitted in
/// [`ADMIN_NAV`] order with a `.am-nav-group` label whenever the group changes.
#[must_use]
pub fn nav_links(active: &str) -> String {
    let mut out = String::with_capacity(768);
    let mut current_group = "";
    for n in ADMIN_NAV {
        if n.group != current_group {
            current_group = n.group;
            let _ = write!(out, r#"<span class="am-nav-group">{}</span>"#, n.group);
        }
        let attr = if n.key == active {
            " class=\"am-nav-active\""
        } else {
            ""
        };
        let _ = write!(out, "<a href=\"{}\"{attr}>{}</a>", n.path, n.label);
    }
    out
}

/// A breadcrumb trail for an admin page (`Home › Admin › <Page>`).
///
/// Overview is the admin landing, so it gets the shorter `Home › Admin` with no
/// trailing page. An unknown key yields an empty string (no breadcrumb). Uses
/// the same `.bnb-crumbs` markup as the main-nav breadcrumbs so styling matches.
#[must_use]
pub fn breadcrumb(active: &str) -> String {
    let Some(n) = ADMIN_NAV.iter().find(|n| n.key == active) else {
        return String::new();
    };
    if n.key == "overview" {
        return r#"<nav class="bnb-crumbs" aria-label="Breadcrumb"><a href="/">Home</a><span class="sep" aria-hidden="true">›</span><span class="cur" aria-current="page">Admin</span></nav>"#.to_string();
    }
    format!(
        r#"<nav class="bnb-crumbs" aria-label="Breadcrumb"><a href="/">Home</a><span class="sep" aria-hidden="true">›</span><a href="/admin/overview">Admin</a><span class="sep" aria-hidden="true">›</span><span class="cur" aria-current="page">{}</span></nav>"#,
        n.label
    )
}

/// A breadcrumb for a **sub-page** that lives beneath nav tab `parent_key`.
///
/// Renders `Home › Admin › <Parent> › <subpage>`, with Home/Admin/Parent all
/// links so an operator always has a one-click way back up. The parent tab is
/// what [`nav_links`] highlights for these pages, giving a clear sense of place
/// even though the sub-page has no tab of its own. Empty for an unknown parent.
#[must_use]
pub fn breadcrumb_subpage(parent_key: &str, subpage: &str) -> String {
    let Some(parent) = ADMIN_NAV.iter().find(|n| n.key == parent_key) else {
        return String::new();
    };
    format!(
        r#"<nav class="bnb-crumbs" aria-label="Breadcrumb"><a href="/">Home</a><span class="sep" aria-hidden="true">›</span><a href="/admin/overview">Admin</a><span class="sep" aria-hidden="true">›</span><a href="{}">{}</a><span class="sep" aria-hidden="true">›</span><span class="cur" aria-current="page">{subpage}</span></nav>"#,
        parent.path, parent.label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::collections::HashSet;
    use tower::ServiceExt as _; // for `oneshot`

    fn test_state() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate schema");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    #[test]
    fn admin_nav_keys_and_paths_are_unique() {
        let mut keys = HashSet::new();
        let mut paths = HashSet::new();
        for n in ADMIN_NAV {
            assert!(keys.insert(n.key), "duplicate admin nav key: {}", n.key);
            assert!(paths.insert(n.path), "duplicate admin nav path: {}", n.path);
        }
    }

    #[test]
    fn admin_nav_is_grouped_into_the_station_task_groups() {
        // Every entry belongs to one of the six canonical groups, and entries
        // stay contiguous by group so the shell renders one cluster each.
        let mut order: Vec<&str> = Vec::new();
        for n in ADMIN_NAV {
            assert!(GROUPS.contains(&n.group), "unknown group {}", n.group);
            if order.last() != Some(&n.group) {
                assert!(
                    !order.contains(&n.group),
                    "group {} is not contiguous",
                    n.group
                );
                order.push(n.group);
            }
        }
        // The distinct groups appear in the canonical order…
        assert_eq!(order, GROUPS);
        // …and the rendered nav carries each group's label exactly once.
        let html = nav_links("overview");
        for g in GROUPS {
            assert_eq!(
                html.matches(&format!(r#"<span class="am-nav-group">{g}</span>"#))
                    .count(),
                1,
                "group label {g} should render exactly once"
            );
        }
    }

    #[test]
    fn nav_links_mark_exactly_the_active_tab() {
        let html = nav_links("system");
        assert_eq!(
            html.matches("am-nav-active").count(),
            1,
            "exactly one admin tab is active"
        );
        assert!(html.contains(r#"href="/admin/system" class="am-nav-active""#));
        // Every destination is present.
        for n in ADMIN_NAV {
            assert!(html.contains(n.path), "{} missing from admin nav", n.path);
        }
    }

    #[test]
    fn breadcrumb_shapes() {
        assert!(breadcrumb("unknown-key").is_empty());
        let overview = breadcrumb("overview");
        assert!(overview.contains("Admin"));
        assert!(!overview.contains("/admin/overview")); // overview is the leaf
        let system = breadcrumb("system");
        assert!(system.contains(r#"href="/admin/overview""#));
        assert!(system.contains("System"));
    }

    #[tokio::test]
    async fn admin_router_serves_every_nav_destination() {
        // Parity guard, mirroring `cmdk_covers_every_nav_destination`: every
        // admin nav tab must resolve to a real route, so a tab can never link to
        // a 404. (We assert on route existence, not the handler's status — a bare
        // test state may make a handler 500, but it must never be NOT_FOUND.)
        for n in ADMIN_NAV {
            let app = crate::routes::admin::router().with_state(test_state());
            let res = app
                .oneshot(
                    Request::builder()
                        .uri(n.path)
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("router responds");
            assert_ne!(
                res.status(),
                StatusCode::NOT_FOUND,
                "admin nav destination {} ({}) has no registered route",
                n.path,
                n.key
            );
        }
    }

    /// GET `path` against the real admin router + a migrated state, returning
    /// the status and the rendered body.
    async fn get_admin(path: &str) -> (StatusCode, String) {
        let app = crate::routes::admin::router().with_state(test_state());
        let res = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router responds");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("read body");
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn health_detail_pages_render_through_the_shared_shell() {
        // The Health-cluster detail pages (Overview · System · Diagnostics) and
        // the full-form Settings fallback stay gated admin-shell pages — they are
        // not folded into a Station tab, so they still render the shared admin nav
        // (right tab active), a breadcrumb, and their own content.
        for (path, marker) in [
            ("/admin/settings", "Admin Settings"),
            ("/admin/system", "System Status"),
            ("/admin/overview", "Admin Overview"),
        ] {
            let (status, body) = get_admin(path).await;
            assert!(status.is_success(), "{path} returned {status}");
            assert!(
                body.contains("admin-nav"),
                "{path} missing shared admin nav"
            );
            assert!(body.contains("bnb-crumbs"), "{path} missing breadcrumb");
            assert!(
                body.contains(&format!(r#"href="{path}" class="am-nav-active""#)),
                "{path} tab is not marked active in the shell nav"
            );
            assert!(
                body.contains(marker),
                "{path} missing its content: {marker}"
            );
        }
    }

    #[tokio::test]
    async fn folded_pages_redirect_to_their_station_tab() {
        // The v3-spine 301 cut: the eight management pages whose content moved
        // into the gated Station tabs permanently redirect their old `/admin/*`
        // GET to the new home — never a 404 for a veteran's bookmark. The admin
        // POST/partial/action endpoints keep their `/admin/...` paths (unchanged).
        for (path, target) in [
            ("/admin", "/station"),
            ("/admin/audio", "/station/capture"),
            ("/admin/species", "/station/capture"),
            ("/admin/rules", "/station/alerts"),
            ("/admin/notifications", "/station/alerts"),
            ("/admin/backups", "/station/data"),
            ("/admin/migrate", "/station/data"),
            ("/admin/quality", "/station/data"),
            ("/admin/accounts", "/station/access"),
        ] {
            let app = crate::routes::admin::router().with_state(test_state());
            let res = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("build request"),
                )
                .await
                .expect("router responds");
            assert_eq!(
                res.status(),
                StatusCode::PERMANENT_REDIRECT,
                "{path} should permanently redirect"
            );
            let loc = res
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            assert_eq!(loc, target, "{path} should redirect to {target}");
        }
    }

    #[tokio::test]
    async fn subpages_render_under_their_parent_tab() {
        // Admin sub-pages have no tab of their own, so they highlight their
        // PARENT tab (sense of place) and carry a breadcrumb down to the leaf
        // (Home › Admin › <Parent> › <leaf>) — the standard, intuitive pattern.
        // (path, parent tab path, breadcrumb leaf, page content marker)
        for (path, parent, leaf, marker) in [
            (
                "/admin/species/test",
                "/admin/species",
                "Filter test",
                "Species Filter Preview",
            ),
            (
                "/admin/notifications/test",
                "/admin/notifications",
                "Test",
                "Test Notification Channels",
            ),
            (
                "/admin/images",
                "/admin/species",
                "Images",
                "Species Image Blacklist",
            ),
        ] {
            let (status, body) = get_admin(path).await;
            assert!(status.is_success(), "{path} returned {status}");
            // The PARENT tab is lit, not a tab for the sub-page itself.
            assert!(
                body.contains(&format!(r#"href="{parent}" class="am-nav-active""#)),
                "{path} should highlight its parent tab {parent}"
            );
            // The breadcrumb's current (leaf) crumb names the sub-page.
            assert!(
                body.contains(&format!(r#"aria-current="page">{leaf}</span>"#)),
                "{path} breadcrumb should end at {leaf}"
            );
            assert!(body.contains(marker), "{path} missing content: {marker}");
        }
    }
}
