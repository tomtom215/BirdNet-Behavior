//! Single source of truth for the site's navigation.
//!
//! Every destination is declared **once** in [`PRIMARY`] (top-level tabs) or
//! [`MORE`] (secondary destinations). The desktop top-nav, the "More" dropdown,
//! the mobile bottom tab bar, the mobile "More" sheet, the breadcrumb trail, and
//! the command-palette page list are all *generated* from these two tables, so
//! the surfaces can no longer drift out of sync the way four separately
//! hand-maintained HTML lists did (which is how `/live` became an orphan, the
//! mobile sheet lost `/kiosk` and `/help`, and `/analytics` vanished from mobile
//! entirely).
//!
//! Active-state is derived from the page's `active_nav` key — the same key each
//! page already passes to `render_page_for_request` — so highlighting can't
//! disagree with the menu it's highlighting.
//!
//! A page becomes reachable simply by being registered here;
//! `manifest_covers_quarantine_badge`/`every_more_key_is_unique` and the
//! router-coverage test guard against regressions.

use std::fmt::Write as _;

/// A top-level destination: always in the desktop top-nav; the ones with a
/// `mobile` glyph/label also occupy a slot in the phone bottom tab bar. The
/// rest (`mobile == None`) surface on phones through the "More" sheet instead.
pub struct Primary {
    pub path: &'static str,
    pub label: &'static str,
    pub key: &'static str,
    /// `(glyph, short label)` when this tab is on the phone bottom bar.
    pub mobile: Option<(&'static str, &'static str)>,
    /// Optional extra markup rendered inside the top-nav link (e.g. the
    /// quarantine pending-count badge). Empty for everything else.
    pub extra: &'static str,
}

/// A secondary destination: appears in the "More" dropdown and the mobile
/// sheet, under `group`, and in the command palette.
pub struct More {
    pub path: &'static str,
    pub label: &'static str,
    pub key: &'static str,
    pub glyph: &'static str,
    pub sub: &'static str,
    pub group: &'static str,
}

/// The quarantine tab carries a live pending-count badge; preserved verbatim.
const QUARANTINE_BADGE: &str = r#" lay-iflex-gap">Quarantine<span hx-get="/pages/quarantine-pending-count" hx-trigger="load, every 60s" hx-swap="innerHTML"></span"#;

pub const PRIMARY: &[Primary] = &[
    Primary {
        path: "/",
        label: "Dashboard",
        key: "dashboard",
        mobile: Some(("⌂", "Now")),
        extra: "",
    },
    Primary {
        path: "/today",
        label: "Today",
        key: "today",
        mobile: Some(("⊙", "Today")),
        extra: "",
    },
    Primary {
        path: "/species",
        label: "Species",
        key: "species",
        mobile: Some(("⌬", "Species")),
        extra: "",
    },
    Primary {
        path: "/heatmap",
        label: "Heatmap",
        key: "heatmap",
        mobile: Some(("▦", "When")),
        extra: "",
    },
    Primary {
        path: "/migration",
        label: "Migration",
        key: "migration",
        mobile: Some(("∿", "Migration")),
        extra: "",
    },
    Primary {
        path: "/analytics",
        label: "Analytics",
        key: "analytics",
        mobile: None,
        extra: "",
    },
    Primary {
        path: "/life-list",
        label: "Life list",
        key: "life-list",
        mobile: None,
        extra: "",
    },
    // Rendered via the badge special-case below.
    Primary {
        path: "/quarantine",
        label: "Quarantine",
        key: "quarantine",
        mobile: None,
        extra: QUARANTINE_BADGE,
    },
    Primary {
        path: "/system",
        label: "System",
        key: "system",
        mobile: None,
        extra: "",
    },
];

pub const MORE: &[More] = &[
    // Reports
    More {
        path: "/history",
        label: "History",
        key: "history",
        glyph: "◷",
        sub: "calendar of past days",
        group: "Reports",
    },
    More {
        path: "/weekly",
        label: "Weekly report",
        key: "weekly",
        glyph: "¶",
        sub: "Sunday recap",
        group: "Reports",
    },
    More {
        path: "/year-in-review",
        label: "Year in review",
        key: "year_in_review",
        glyph: "⊞",
        sub: "annual editorial",
        group: "Reports",
    },
    // Audio (live + recorded playback) — split out from static photos.
    More {
        path: "/listen",
        label: "Live audio",
        key: "listen",
        glyph: "♪",
        sub: "listen now & test your mic",
        group: "Audio",
    },
    More {
        path: "/recordings",
        label: "Recordings",
        key: "recordings",
        glyph: "▶",
        sub: "listen to detection clips",
        group: "Audio",
    },
    // Images
    More {
        path: "/gallery",
        label: "Gallery",
        key: "gallery",
        glyph: "◫",
        sub: "species photos",
        group: "Images",
    },
    // Analytics — deep dives
    More {
        path: "/analytics/dawn-chorus",
        label: "Dawn chorus",
        key: "dawn_chorus",
        glyph: "◐",
        sub: "per-species polar plot",
        group: "Analytics",
    },
    More {
        path: "/correlation",
        label: "Co-occurrence",
        key: "correlation",
        glyph: "☰",
        sub: "who sings with whom",
        group: "Analytics",
    },
    More {
        path: "/timeseries",
        label: "Time series",
        key: "timeseries",
        glyph: "∷",
        sub: "trends & comparisons",
        group: "Analytics",
    },
    // Operations
    More {
        path: "/notifications",
        label: "Notifications",
        key: "notifications",
        glyph: "≡",
        sub: "channels & log",
        group: "Operations",
    },
    More {
        path: "/admin",
        label: "Admin",
        key: "admin",
        glyph: "⌗",
        sub: "settings, audio, backups",
        group: "Operations",
    },
    More {
        path: "/kiosk",
        label: "Kiosk mode",
        key: "kiosk",
        glyph: "◉",
        sub: "wall display",
        group: "Operations",
    },
    // Help
    More {
        path: "/system/changelog",
        label: "Changelog",
        key: "changelog",
        glyph: "⌥",
        sub: "what's new",
        group: "Help",
    },
    More {
        path: "/help",
        label: "Help & methodology",
        key: "help",
        glyph: "?",
        sub: "the manual",
        group: "Help",
    },
];

/// `"active"` when `key` is the page's active section, else `""`.
fn active(key: &str, current: &str) -> &'static str {
    if key == current { "active" } else { "" }
}

/// Desktop top-nav `<a>` links (the `.topnav-links` block).
pub fn topnav_links(current: &str) -> String {
    let mut out = String::with_capacity(1024);
    for p in PRIMARY {
        let a = active(p.key, current);
        if p.extra.is_empty() {
            let _ = write!(
                out,
                r#"<a href="{}" class="topnav-link {a}">{}</a>"#,
                p.path, p.label
            );
        } else {
            // Quarantine: the extra markup re-opens the class attribute and
            // supplies its own label + badge, so close the tag after it.
            let _ = write!(
                out,
                r#"<a href="{}" class="topnav-link {a}{}></a>"#,
                p.path, p.extra
            );
        }
    }
    out
}

/// The grouped body of the "More" dropdown (the `.bnb-more-menu__body` content,
/// minus the trailing Quick-navigator button, which the shell keeps).
pub fn more_groups(current: &str) -> String {
    let mut out = String::with_capacity(4096);
    let mut group: Option<&str> = None;
    for m in MORE {
        if group != Some(m.group) {
            if group.is_some() {
                out.push_str("</ul></section>");
            }
            let _ = write!(
                out,
                r#"<section class="bnb-more-group"><h3 class="bnb-eyebrow">{}</h3><ul>"#,
                m.group
            );
            group = Some(m.group);
        }
        let a = active(m.key, current);
        let _ = write!(
            out,
            r#"<li><a href="{}" class="bnb-more-row {a}"><span class="glyph">{}</span><span class="label">{}</span><span class="sub">{}</span></a></li>"#,
            m.path, m.glyph, m.label, m.sub
        );
    }
    if group.is_some() {
        out.push_str("</ul></section>");
    }
    out
}

/// The phone bottom-tab-bar slots (the primary tabs that fit the bar).
pub fn tabbar_slots(current: &str) -> String {
    let mut out = String::with_capacity(1024);
    for p in PRIMARY {
        let Some((glyph, short)) = p.mobile else {
            continue;
        };
        let a = active(p.key, current);
        let _ = write!(
            out,
            r#"<a href="{}" class="bnb-tabbar__slot {a}" aria-label="{}"><span class="glyph" aria-hidden="true">{glyph}</span><span class="label">{short}</span></a>"#,
            p.path, p.label
        );
    }
    out
}

/// The mobile "More" sheet rows: the top-level sections that don't fit the
/// bottom bar (Analytics, Life list, Quarantine, System — previously missing
/// from mobile), then every secondary destination, grouped like the dropdown so
/// it isn't a flat wall.
pub fn sheet_rows(current: &str) -> String {
    let mut out = String::with_capacity(4096);
    let overflow: Vec<&Primary> = PRIMARY.iter().filter(|p| p.mobile.is_none()).collect();
    if !overflow.is_empty() {
        out.push_str(r#"<li class="bnb-sheet__group">Sections</li>"#);
        for p in overflow {
            let a = active(p.key, current);
            let _ = write!(
                out,
                r#"<li><a href="{}" class="bnb-sheet__row {a}"><span class="glyph">◆</span> {}</a></li>"#,
                p.path, p.label
            );
        }
    }
    let mut group: Option<&str> = None;
    for m in MORE {
        if group != Some(m.group) {
            let _ = write!(out, r#"<li class="bnb-sheet__group">{}</li>"#, m.group);
            group = Some(m.group);
        }
        let a = active(m.key, current);
        let _ = write!(
            out,
            r#"<li><a href="{}" class="bnb-sheet__row {a}"><span class="glyph">{}</span> {}</a></li>"#,
            m.path, m.glyph, m.label
        );
    }
    out
}

/// A breadcrumb trail for a secondary page (`Home › Group › Page`). Empty for a
/// top-level tab and for the dashboard, which need none.
pub fn breadcrumb(current: &str) -> String {
    let Some(m) = MORE.iter().find(|m| m.key == current) else {
        return String::new();
    };
    format!(
        r#"<nav class="bnb-crumbs" aria-label="Breadcrumb"><a href="/">Home</a><span class="sep" aria-hidden="true">›</span><span class="grp">{}</span><span class="sep" aria-hidden="true">›</span><span class="cur" aria-current="page">{}</span></nav>"#,
        m.group, m.label
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn more_keys_are_unique() {
        let mut seen = HashSet::new();
        for m in MORE {
            assert!(seen.insert(m.key), "duplicate More key: {}", m.key);
        }
        for p in PRIMARY {
            assert!(seen.insert(p.key), "duplicate nav key: {}", p.key);
        }
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
        // The quarantine badge survives generation.
        assert!(html.contains("/pages/quarantine-pending-count"));
    }

    #[test]
    fn more_and_sheet_share_the_same_destinations() {
        // The whole point of the manifest: the dropdown and the mobile sheet
        // cannot list different things, because both come from MORE.
        let more = more_groups("");
        let sheet = sheet_rows("");
        for m in MORE {
            assert!(more.contains(m.path), "{} missing from dropdown", m.path);
            assert!(
                sheet.contains(m.path),
                "{} missing from mobile sheet",
                m.path
            );
        }
        // The top-level sections that aren't bottom-bar tabs must appear on
        // mobile via the sheet (this is the /analytics-missing-on-mobile fix).
        assert!(sheet.contains(r#"href="/analytics""#));
        assert!(sheet.contains(r#"href="/life-list""#));
    }

    #[test]
    fn breadcrumb_only_for_secondary_pages() {
        assert!(breadcrumb("dashboard").is_empty());
        assert!(breadcrumb("species").is_empty());
        let crumb = breadcrumb("dawn_chorus");
        assert!(crumb.contains("Home"));
        assert!(crumb.contains("Analytics"));
        assert!(crumb.contains("Dawn chorus"));
    }
}
