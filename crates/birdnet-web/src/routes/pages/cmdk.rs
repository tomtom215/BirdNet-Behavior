//! Command palette server-side filter (O-19).
//!
//! Endpoint: `GET /pages/cmdk?q=<query>` returns up to ~12 rows grouped by
//! source. The client owns only the open/close/arrow-key behaviour; ranking
//! and filtering live here so they can hit SQLite directly.
//!
//! Adapted from the O-19 command-palette design proposal. The package
//! queried a hypothetical `species_aggregate` table that does not exist in
//! this fork's schema — both species hits and recent detections are now
//! satisfied directly from `detections` using the actual column names
//! (`Com_Name` / `Sci_Name` / `Date` / `Time`).

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::response::Html;
use axum::{Router, routing::get};
use serde::Deserialize;

use super::{escape_html, simple_url_encode};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/pages/cmdk", get(cmdk))
}

#[derive(Deserialize)]
struct CmdkQuery {
    #[serde(default)]
    q: String,
}

async fn cmdk(State(state): State<AppState>, Query(qp): Query<CmdkQuery>) -> Html<String> {
    let q = qp.q.trim();
    let mut out = String::new();

    if q.is_empty() {
        render_group(&mut out, "Jump to", &default_pages());
        render_recent(&mut out, &state).await;
        render_group(&mut out, "Settings", &default_settings());
        return Html(out);
    }

    let qlc = q.to_lowercase();

    let page_hits = filter_entries(&all_pages(), &qlc, 6);
    if !page_hits.is_empty() {
        render_group(&mut out, "Pages", &page_hits);
    }

    if let Some(date_row) = parse_date(q) {
        render_group(&mut out, "Dates", &[date_row]);
    }

    let species_hits = species_hits(&state, &qlc, 8).await;
    if !species_hits.is_empty() {
        render_group(&mut out, "Species", &species_hits);
    }

    let setting_hits = filter_entries(&all_settings(), &qlc, 4);
    if !setting_hits.is_empty() {
        render_group(&mut out, "Settings", &setting_hits);
    }

    if out.is_empty() {
        out.push_str(
            r#"<li class="bnb-cmdk__empty" role="status">
              No matches. Try a species code (NOCA) or a date (yesterday).
            </li>"#,
        );
    }
    Html(out)
}

// ---------------------------------------------------------------------------
// Entry types & rendering
// ---------------------------------------------------------------------------

struct Entry {
    glyph: &'static str,
    label: String,
    sub: String,
    href: String,
    synonyms: &'static [&'static str],
}

fn render_group(out: &mut String, title: &str, rows: &[Entry]) {
    if rows.is_empty() {
        return;
    }
    let _ = write!(
        out,
        r#"<li class="bnb-cmdk__group">{}</li>"#,
        escape_html(title)
    );
    for r in rows {
        let _ = write!(
            out,
            r#"<li role="option" data-href="{href}">
              <span class="glyph">{glyph}</span>
              <span class="label">{label}</span>
              <span class="sub">{sub}</span>
            </li>"#,
            href = escape_html(&r.href),
            glyph = escape_html(r.glyph),
            label = escape_html(&r.label),
            sub = escape_html(&r.sub),
        );
    }
}

fn filter_entries(all: &[Entry], qlc: &str, limit: usize) -> Vec<Entry> {
    let mut hits: Vec<(usize, &Entry)> = all
        .iter()
        .filter_map(|e| {
            let label_lc = e.label.to_lowercase();
            if label_lc.contains(qlc) {
                Some((label_lc.find(qlc).unwrap_or(99), e))
            } else if e.synonyms.iter().any(|s| s.to_lowercase().contains(qlc)) {
                Some((50, e))
            } else if e.href.to_lowercase().contains(qlc) {
                Some((90, e))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter()
        .take(limit)
        .map(|(_, e)| Entry {
            glyph: e.glyph,
            label: e.label.clone(),
            sub: e.sub.clone(),
            href: e.href.clone(),
            synonyms: e.synonyms,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Index — Pages
// ---------------------------------------------------------------------------

fn all_pages() -> Vec<Entry> {
    let make = |glyph, label: &str, sub: &str, href: &str, syn: &'static [&'static str]| Entry {
        glyph,
        label: label.to_string(),
        sub: sub.to_string(),
        href: href.to_string(),
        synonyms: syn,
    };
    vec![
        // The six homes of the v3 spine, in nav order. Old vocabulary stays
        // findable as synonyms so a veteran typing "heatmap" still lands.
        make(
            "⌂",
            "Today",
            "what's happening?",
            "/",
            &["home", "live", "dashboard", "now", "log"],
        ),
        make(
            "⌬",
            "Species",
            "who have I heard?",
            "/species",
            &["birds", "list"],
        ),
        make(
            "▦",
            "Patterns",
            "when & where?",
            "/patterns",
            &["analytics", "heatmap", "activity"],
        ),
        make(
            "♪",
            "Recordings",
            "let me hear them",
            "/recordings",
            &["audio", "wav", "clip", "listen"],
        ),
        make(
            "¶",
            "Reports",
            "the recap",
            "/reports",
            &["weekly", "bulletin", "recap"],
        ),
        make(
            "⌗",
            "Settings",
            "manage my station",
            "/station",
            // "station" stays a keyword deliberately: the section was called
            // that until now, the URL still is, and the docs and prior release
            // notes refer to it. Renaming the label should not make the page
            // unfindable by the name half the material still uses.
            &[
                "station",
                "settings",
                "configure",
                "config",
                "system",
                "admin",
                "pi",
                "cpu",
                "disk",
                "tools",
            ],
        ),
        // The long tail: views inside the homes + utility pages, reachable
        // here and through contextual links (no top-level tab).
        make(
            "▦",
            "When active",
            "hour × day",
            "/patterns",
            &["heatmap", "when", "grid"],
        ),
        make(
            "◐",
            "Dawn chorus",
            "polar plot",
            "/patterns?tab=dawn",
            &["chorus", "polar", "circadian"],
        ),
        make(
            "∿",
            "Migration",
            "phenology",
            "/patterns?tab=migration",
            &["arrivals", "ridgeline", "departures"],
        ),
        make(
            "☰",
            "Who sings together",
            "co-occurrence",
            "/patterns?tab=together",
            &["correlation", "matrix", "pairs"],
        ),
        make(
            "∷",
            "Trends",
            "busier or quieter?",
            "/patterns?tab=trends",
            &["time series", "trend", "compare"],
        ),
        make(
            "⊕",
            "Behavior",
            "the deep tier",
            "/patterns?tab=behavior",
            &["behavioral", "sessions", "funnel", "retention", "sequence"],
        ),
        make(
            "◷",
            "History",
            "browse past days",
            "/reports?tab=history",
            &["browse", "calendar"],
        ),
        make(
            "⊞",
            "Year in review",
            "your year in song",
            "/reports?tab=year",
            &["year", "annual"],
        ),
        make(
            "✦",
            "Life list",
            "journal",
            "/species?view=lifelist",
            &["lifer", "journal"],
        ),
        make(
            "◫",
            "Photos",
            "species gallery",
            "/species?view=photos",
            &["photo", "image", "gallery"],
        ),
        make(
            "⚑",
            "Review",
            "rare-bird queue",
            "/quarantine",
            &["rare", "queue", "review", "quarantine", "confirm"],
        ),
        make(
            "⌕",
            "Search detections",
            "filter the whole log",
            "/search",
            &[
                "search",
                "find",
                "filter",
                "query",
                "date",
                "confidence",
                "hour",
                "source",
                "rejected",
                "unreviewed",
                "bulk",
            ],
        ),
        make("◉", "Kiosk", "wall display", "/kiosk", &["ambient", "wall"]),
        make(
            "≡",
            "Notifications",
            "channels & log",
            "/notifications",
            &["alerts", "channel"],
        ),
        make(
            "♪",
            "Live audio",
            "test your mic",
            "/recordings?view=live",
            &["stream", "mic", "live", "listen"],
        ),
        make(
            "⌥",
            "Changelog",
            "what's new",
            "/system/changelog",
            &["changes", "release", "version"],
        ),
        make(
            "?",
            "Help & methodology",
            "the manual",
            "/help",
            &["docs", "manual", "methodology", "guide"],
        ),
    ]
}

fn default_pages() -> Vec<Entry> {
    all_pages().into_iter().take(8).collect()
}

// ---------------------------------------------------------------------------
// Index — Settings
// ---------------------------------------------------------------------------

fn all_settings() -> Vec<Entry> {
    let make = |label: &str, sub: &str, href: &str, syn: &'static [&'static str]| Entry {
        glyph: "⚙",
        label: label.to_string(),
        sub: sub.to_string(),
        href: href.to_string(),
        synonyms: syn,
    };
    vec![
        make(
            "Detection",
            "settings",
            "/station/capture#detection",
            &["threshold", "sensitivity", "confidence"],
        ),
        make(
            "Audio",
            "sources",
            "/station/capture#audio",
            &["microphone", "mic", "rtsp", "usb", "alsa", "pipewire"],
        ),
        make(
            "Notifications",
            "channels",
            "/station/alerts#notifications",
            &["telegram", "email", "mqtt", "slack", "webhook"],
        ),
        make(
            "Species",
            "filter",
            "/station/capture#species",
            &["exclude", "include", "allow", "list"],
        ),
        make(
            "Rules",
            "alerts",
            "/station/alerts#rules",
            &["alert", "rule"],
        ),
        make(
            "Quality",
            "metrics",
            "/station/data#quality",
            &["false positive", "low confidence"],
        ),
        make(
            "Accounts",
            "sessions",
            "/station/access#accounts",
            &["users", "viewers", "sign out"],
        ),
        make(
            "Backups",
            "recovery",
            "/station/data#backups",
            &["backup", "restore", "snapshot"],
        ),
        make(
            "Migrate",
            "import",
            "/station/data#import",
            &["birdnet-pi", "import"],
        ),
        make(
            "Diagnostics",
            "doctor",
            "/admin/doctor",
            &["health", "self-check"],
        ),
        // These two were in the nav of no page and matched no palette query:
        // the only way to either was to already know its URL. An audit log
        // nobody can find is not an audit log.
        make(
            "Audit log",
            "who changed what",
            "/admin/audit",
            &["audit", "log", "history", "who", "changed"],
        ),
        make(
            "Species images",
            "blacklist & overrides",
            "/admin/images",
            &["image", "photo", "picture", "blacklist", "wikipedia"],
        ),
        make(
            "System status",
            "processes & storage",
            "/admin/system",
            &["status", "cpu", "memory", "disk", "service", "logs"],
        ),
        make(
            "Display",
            "prefs",
            "/station/settings#display-prefs",
            &["theme", "density", "motion", "contrast"],
        ),
    ]
}

fn default_settings() -> Vec<Entry> {
    all_settings().into_iter().take(5).collect()
}

// ---------------------------------------------------------------------------
// Species hits — from this fork's `detections` table (no species_aggregate)
// ---------------------------------------------------------------------------

async fn species_hits(state: &AppState, qlc: &str, limit: usize) -> Vec<Entry> {
    let qlc = qlc.to_string();
    let state2 = state.clone();
    tokio::task::spawn_blocking(move || {
        state2.with_db(|conn| {
            let pattern = format!("%{qlc}%");
            let limit_i64 = i64::try_from(limit).unwrap_or(8);
            let Ok(mut stmt) = conn.prepare(
                "SELECT Com_Name, Sci_Name, COUNT(*) AS n
                   FROM detections_analytic
                  WHERE LOWER(Com_Name) LIKE ?1
                     OR LOWER(Sci_Name) LIKE ?1
                  GROUP BY Com_Name, Sci_Name
                  ORDER BY n DESC
                  LIMIT ?2",
            ) else {
                return Vec::new();
            };
            let rows = stmt.query_map((&pattern, limit_i64), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            });
            let mut out = Vec::new();
            if let Ok(it) = rows {
                for r in it.flatten() {
                    let (com, _sci, n) = r;
                    out.push(Entry {
                        glyph: "♪",
                        label: com.clone(),
                        sub: format!("{n} detections"),
                        href: format!("/species/detail?name={}", simple_url_encode(&com)),
                        synonyms: &[],
                    });
                }
            }
            out
        })
    })
    .await
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Recent detections (default view)
// ---------------------------------------------------------------------------

async fn render_recent(out: &mut String, state: &AppState) {
    let state2 = state.clone();
    let recent: Vec<(String, String)> = tokio::task::spawn_blocking(move || {
        state2.with_db(|conn| {
            let Ok(mut stmt) = conn.prepare(
                "SELECT Com_Name, Date || ' ' || Time AS at
                   FROM detections_analytic
                  ORDER BY Date DESC, Time DESC
                  LIMIT 4",
            ) else {
                return Vec::new();
            };
            let it = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            });
            let mut rows = Vec::new();
            if let Ok(it) = it {
                for r in it.flatten() {
                    rows.push(r);
                }
            }
            rows
        })
    })
    .await
    .unwrap_or_default();

    if recent.is_empty() {
        return;
    }
    let entries: Vec<Entry> = recent
        .into_iter()
        .map(|(com, at)| Entry {
            glyph: "♪",
            label: com.clone(),
            sub: at,
            href: format!("/species/detail?name={}", simple_url_encode(&com)),
            synonyms: &[],
        })
        .collect();
    render_group(out, "Recent", &entries);
}

// ---------------------------------------------------------------------------
// Date parser
// ---------------------------------------------------------------------------

fn parse_date(q: &str) -> Option<Entry> {
    let q = q.trim().to_lowercase();

    if q.len() == 10 && crate::routes::is_valid_date(&q) {
        return Some(Entry {
            glyph: "◷",
            label: q.clone(),
            sub: "history".into(),
            href: format!("/history?date={q}"),
            synonyms: &[],
        });
    }
    if q.len() == 7
        && q.as_bytes().get(4) == Some(&b'-')
        && q[..4].chars().all(|c| c.is_ascii_digit())
        && q[5..].chars().all(|c| c.is_ascii_digit())
    {
        return Some(Entry {
            glyph: "◷",
            label: q.clone(),
            sub: "history (month)".into(),
            href: format!("/history?month={q}"),
            synonyms: &[],
        });
    }

    let today = super::today_date_string();
    let (label, sub, href) = match q.as_str() {
        "today" => ("Today", "live log", "/today".to_string()),
        "yesterday" => (
            "Yesterday",
            "history",
            format!("/history?date={}", shift_date(&today, -1)),
        ),
        "this week" => ("This week", "weekly report", "/weekly".to_string()),
        "last week" => (
            "Last week",
            "weekly report",
            "/weekly?offset=-1".to_string(),
        ),
        "this month" => (
            "This month",
            "trends",
            "/timeseries?period=month".to_string(),
        ),
        "last month" => (
            "Last month",
            "trends",
            "/timeseries?period=month&offset=-1".to_string(),
        ),
        "this year" => ("This year", "year in review", "/year-in-review".to_string()),
        _ => return None,
    };
    Some(Entry {
        glyph: "◷",
        label: label.into(),
        sub: sub.into(),
        href,
        synonyms: &[],
    })
}

fn shift_date(date: &str, days: i32) -> String {
    use super::days_to_date;
    let parts: Vec<u32> = date.split('-').filter_map(|s| s.parse().ok()).collect();
    if parts.len() != 3 {
        return date.to_string();
    }
    let (y, m, d) = (parts[0], parts[1], parts[2]);
    let mut days_se: i64 = ymd_to_days(y, m, d);
    days_se += i64::from(days);
    #[allow(clippy::cast_sign_loss)]
    let (y2, m2, d2) = days_to_date(days_se.max(0) as u64);
    format!("{y2:04}-{m2:02}-{d2:02}")
}

const fn ymd_to_days(y: u32, m: u32, d: u32) -> i64 {
    birdnet_core::civil::days_from_civil(y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_index_has_all_topnav_links() {
        let p = all_pages();
        let labels: Vec<_> = p.iter().map(|e| e.label.as_str()).collect();
        for must in [
            "Today",
            "Species",
            "Patterns",
            "Recordings",
            "Reports",
            "Settings",
        ] {
            assert!(labels.contains(&must), "missing {must} in cmdk pages index");
        }
    }

    /// Every destination the palette offers must actually resolve.
    ///
    /// # Why this needs a gate rather than a reading
    ///
    /// The command palette is not a convenience here — it is the **stated**
    /// fallback for everything the six-home spine does not put in the nav
    /// (`routes::pages::nav`: "the long tail … stays reachable through the
    /// command palette and contextual links"). So a rotted entry is not a
    /// cosmetic miss; it is a destination with no way in.
    ///
    /// Two had rotted, found by walking them against a running station:
    ///
    /// * **Migrate** pointed at `/admin/migration`, which has never existed —
    ///   the route is `/admin/migrate`. It 404'd.
    /// * **Display · prefs** pointed at `/system#display-prefs`. `/system` is a
    ///   pre-spine path that 308s to `/station`, which drops the fragment, and
    ///   `/station` carries no `display-prefs` anchor anyway. Searching
    ///   "theme" took you to the Health tab.
    ///
    /// Nothing could have noticed: the table is a list of strings, and no test
    /// had ever asked the router whether any of them led anywhere.
    #[tokio::test]
    async fn every_palette_destination_resolves() {
        // `/help/*` is a `ServeDir` over `BNB_HELP_DIR`, which the installer
        // and the Docker image both set and a bare `cargo test` does not. Its
        // 404 here is the documented "docs unavailable" path, not a rotted
        // link, so asserting on it would only teach the next reader to ignore
        // this gate.
        const SERVED_FROM_DISK: &[&str] = &["/help"];

        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt as _;

        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        let state =
            crate::state::AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
        let app = crate::server::build_router(state);

        let mut broken = Vec::new();
        for entry in all_pages().into_iter().chain(all_settings()) {
            if SERVED_FROM_DISK.contains(&entry.href.as_str()) {
                continue;
            }
            // The fragment is the browser's business; the router only ever
            // sees the path and query.
            let (path, fragment) = entry
                .href
                .split_once('#')
                .map_or((entry.href.as_str(), None), |(p, f)| (p, Some(f)));

            // Follow redirects the way a browser would, so a legacy path that
            // permanently redirects into the spine still counts as resolving.
            let mut target = path.to_owned();
            let mut status = StatusCode::OK;
            let mut body = String::new();
            for _ in 0..5 {
                let req = Request::builder()
                    .uri(&target)
                    .body(Body::empty())
                    .expect("build request");
                let resp = app.clone().oneshot(req).await.expect("response");
                status = resp.status();
                if status.is_redirection() {
                    let Some(loc) = resp
                        .headers()
                        .get(axum::http::header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .map(ToOwned::to_owned)
                    else {
                        break;
                    };
                    target = loc;
                    continue;
                }
                body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                break;
            }

            if status != StatusCode::OK {
                broken.push(format!("{} → {} → {status}", entry.label, entry.href));
                continue;
            }
            // A fragment that names nothing is a link that lands at the top of
            // the page instead of at the thing the operator searched for —
            // which on an 82 KB merged tab is indistinguishable from broken.
            if let Some(frag) = fragment
                && !body.contains(&format!("id=\"{frag}\""))
            {
                broken.push(format!(
                    "{} → {} → no id=\"{frag}\" on {target}",
                    entry.label, entry.href
                ));
            }
        }
        assert!(
            broken.is_empty(),
            "the command palette is the only way to reach some of these, and \
             they lead nowhere:\n  {}",
            broken.join("\n  ")
        );
    }

    #[test]
    fn settings_index_includes_accounts_after_o15() {
        let s = all_settings();
        assert!(s.iter().any(|e| e.label == "Accounts"));
    }

    #[test]
    fn filter_ranks_label_prefix_higher_than_synonym() {
        let hits = filter_entries(&all_pages(), "spec", 4);
        assert_eq!(hits[0].label, "Species");
    }

    #[test]
    fn dates_today_yesterday() {
        assert_eq!(parse_date("today").map(|e| e.label), Some("Today".into()));
        assert!(parse_date("yesterday").is_some());
        assert!(parse_date("2026-03-12").is_some());
        assert!(parse_date("2026-03").is_some());
    }

    #[test]
    fn dates_bogus_strings_return_none() {
        assert!(parse_date("zonk").is_none());
        assert!(parse_date("13-13-13").is_none());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn shift_date_roundtrip() {
        let d = "2025-05-15";
        let f = shift_date(d, 7);
        let b = shift_date(&f, -7);
        assert_eq!(b, d);
    }

    #[test]
    fn render_group_skips_empty() {
        let mut out = String::new();
        render_group(&mut out, "Pages", &[]);
        assert!(out.is_empty());
    }

    #[test]
    fn render_group_escapes_html() {
        let entries = vec![Entry {
            glyph: ">>",
            label: "<script>".to_string(),
            sub: "subs & co".to_string(),
            href: "/?x=\"y\"".to_string(),
            synonyms: &[],
        }];
        let mut out = String::new();
        render_group(&mut out, "<g>", &entries);
        assert!(out.contains("&lt;script&gt;"));
        assert!(out.contains("subs &amp; co"));
        assert!(out.contains("&lt;g&gt;"));
        assert!(out.contains("/?x=&quot;y&quot;"));
    }

    #[test]
    fn cmdk_covers_every_nav_destination() {
        // Parity guard: the command palette must reach every destination in the
        // nav manifest, so a page reachable from the menus is always reachable
        // from ⌘K too. This locks the third surface to the single source of
        // truth — adding a home to `nav` without a palette entry fails here.
        use crate::routes::pages::nav;
        let pages = all_pages();
        let hrefs: Vec<&str> = pages.iter().map(|e| e.href.as_str()).collect();
        for p in nav::PRIMARY {
            assert!(
                hrefs.contains(&p.path),
                "command palette missing primary destination {}",
                p.path
            );
        }
    }

    #[test]
    fn cmdk_keeps_the_long_tail_reachable() {
        // The v3 spine removed the "More" menu; the palette is now the only
        // global surface for the long tail. Losing one of these entries makes
        // the page unreachable except by typing its URL.
        let pages = all_pages();
        let hrefs: Vec<&str> = pages.iter().map(|e| e.href.as_str()).collect();
        for path in [
            "/quarantine",
            "/kiosk",
            "/system/changelog",
            "/help",
            "/species?view=lifelist",
            "/species?view=photos",
            "/recordings?view=live",
            "/notifications",
        ] {
            assert!(hrefs.contains(&path), "palette lost long-tail page {path}");
        }
    }
}
