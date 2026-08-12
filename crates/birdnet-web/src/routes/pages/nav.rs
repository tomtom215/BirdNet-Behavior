//! Single source of truth for the site's navigation — the **v3 spine**.
//!
//! Every destination is declared **once** in [`PRIMARY`]. The desktop top-nav,
//! the mobile bottom tab bar, and the command-palette page list are all
//! *generated* from this table, so the surfaces can no longer drift out of
//! sync the way separately hand-maintained HTML lists did.
//!
//! The v3 spine (see `docs/design/handover/v3_spine/HANDOFF_v3.html`) collapses
//! the previous 9 primary tabs + 14 "More" destinations into **six homes**:
//!
//! | Home       | Question it answers      |
//! |------------|--------------------------|
//! | Today      | "what's happening?"      |
//! | Species    | "who have I heard?"      |
//! | Patterns   | "when & where?"          |
//! | Recordings | "let me hear them"       |
//! | Reports    | "the recap"              |
//! | Station    | "manage my station"      |
//!
//! Every pre-spine destination folds into one of the homes (legacy paths 301
//! via `routes::redirects`); the long tail (Review, Kiosk, Changelog, Help,
//! detail pages) stays reachable through the command palette and contextual
//! links. The "More" dropdown and the mobile More-sheet are gone: all six
//! homes fit the phone bottom bar.
//!
//! Active-state is derived from the page's `active_nav` key — the same key each
//! page already passes to `render_page_for_request` — so highlighting can't
//! disagree with the menu it's highlighting.

use std::fmt::Write as _;

/// A top-level destination: a desktop top-nav tab and a phone bottom-bar slot.
pub struct Primary {
    /// Route the tab links to.
    pub path: &'static str,
    /// Human label (identical on desktop and mobile — the v3 spine retired the
    /// `Now`/`When` short-label divergence).
    pub label: &'static str,
    /// Stable key matching the page's `active_nav` argument.
    pub key: &'static str,
    /// Glyph for the phone bottom-bar slot.
    pub glyph: &'static str,
}

/// The six homes of the v3 spine, in display order.
pub const PRIMARY: &[Primary] = &[
    Primary {
        path: "/",
        label: "Today",
        key: "today",
        glyph: "⌂",
    },
    Primary {
        path: "/species",
        label: "Species",
        key: "species",
        glyph: "⌬",
    },
    Primary {
        path: "/patterns",
        label: "Patterns",
        key: "patterns",
        glyph: "▦",
    },
    Primary {
        path: "/recordings",
        label: "Recordings",
        key: "recordings",
        glyph: "♪",
    },
    Primary {
        path: "/reports",
        label: "Reports",
        key: "reports",
        glyph: "¶",
    },
    Primary {
        // The path stays `/station` — it is bookmarked, linked from a couple of
        // dozen places, and renaming a URL buys nothing. Only the label moves.
        // "Station" described the *thing being configured*; people looking to
        // configure it went hunting for "Settings", which was the name of a tab
        // one level down and invisible from here.
        path: "/station",
        label: "Settings",
        key: "station",
        glyph: "⌗",
    },
];

/// `"active"` when `key` is the page's active section, else `""`.
fn active(key: &str, current: &str) -> &'static str {
    if key == current { "active" } else { "" }
}

/// Desktop top-nav `<a>` links (the `.topnav-links` block).
pub fn topnav_links(current: &str) -> String {
    let mut out = String::with_capacity(512);
    for p in PRIMARY {
        let a = active(p.key, current);
        let _ = write!(
            out,
            r#"<a href="{}" class="topnav-link {a}">{}</a>"#,
            p.path, p.label
        );
    }
    out
}

/// The phone bottom-tab-bar slots — all six homes, no "More" overflow.
pub fn tabbar_slots(current: &str) -> String {
    let mut out = String::with_capacity(1024);
    for p in PRIMARY {
        let a = active(p.key, current);
        let _ = write!(
            out,
            r#"<a href="{}" class="bnb-tabbar__slot {a}" aria-label="{}"><span class="glyph" aria-hidden="true">{}</span><span class="label">{}</span></a>"#,
            p.path, p.label, p.glyph, p.label
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn nav_keys_and_paths_are_unique() {
        let mut keys = HashSet::new();
        let mut paths = HashSet::new();
        for p in PRIMARY {
            assert!(keys.insert(p.key), "duplicate nav key: {}", p.key);
            assert!(paths.insert(p.path), "duplicate nav path: {}", p.path);
        }
    }

    #[test]
    fn spine_is_the_six_homes_in_order() {
        // The v3 spine contract: exactly these six, in this order. A seventh
        // tab (or a rename) is an IA change and must be a deliberate edit here.
        let spine: Vec<(&str, &str)> = PRIMARY.iter().map(|p| (p.path, p.label)).collect();
        assert_eq!(
            spine,
            vec![
                ("/", "Today"),
                ("/species", "Species"),
                ("/patterns", "Patterns"),
                ("/recordings", "Recordings"),
                ("/reports", "Reports"),
                ("/station", "Settings"),
            ]
        );
    }

    #[test]
    fn topnav_marks_exactly_the_active_tab() {
        let html = topnav_links("species");
        assert_eq!(
            html.matches("topnav-link active").count(),
            1,
            "exactly one tab active"
        );
        assert!(html.contains(r#"href="/species" class="topnav-link active""#));
    }

    #[test]
    fn tabbar_carries_every_home() {
        // All six homes fit the phone bottom bar — there is no More sheet, so a
        // home missing here would be unreachable on mobile.
        let html = tabbar_slots("reports");
        for p in PRIMARY {
            assert!(
                html.contains(&format!(r#"href="{}""#, p.path)),
                "{} missing from the tab bar",
                p.path
            );
        }
        assert_eq!(html.matches("bnb-tabbar__slot active").count(), 1);
    }

    #[test]
    fn desktop_and_mobile_share_one_vocabulary() {
        // The pre-spine bar used divergent short labels ("Now", "When"); the
        // spine's labels are canonical on both surfaces.
        let top = topnav_links("");
        let bar = tabbar_slots("");
        for p in PRIMARY {
            assert!(top.contains(p.label));
            assert!(bar.contains(&format!("<span class=\"label\">{}</span>", p.label)));
        }
    }
}
