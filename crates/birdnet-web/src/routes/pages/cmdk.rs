//! Command palette server-side filter (O-19).
//!
//! Endpoint: `GET /pages/cmdk?q=<query>` returns up to ~12 rows grouped by
//! source. The client owns only the open/close/arrow-key behaviour; ranking
//! and filtering live here so they can hit SQLite directly.
//!
//! Adapted from `docs/proposed_changes/O-19_cmdk/src/cmdk.rs`. The package
//! queried a hypothetical `species_aggregate` table that does not exist in
//! this fork's schema — both species hits and recent detections are now
//! satisfied directly from `detections` using the actual column names
//! (`Com_Name` / `Sci_Name` / `Date` / `Time`).
//!
//! See O-19 DIFF.md.

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
        make("⌂", "Dashboard", "live feed", "/", &["home", "live"]),
        make("⊙", "Today", "detection log", "/today", &["log", "now"]),
        make("⌬", "Species", "all voices", "/species", &["birds", "list"]),
        make("▦", "Heatmap", "hour × day", "/heatmap", &["activity", "when"]),
        make(
            "◐",
            "Dawn chorus",
            "polar plot",
            "/analytics/dawn-chorus",
            &["chorus", "polar", "circadian"],
        ),
        make(
            "∿",
            "Migration",
            "phenology",
            "/migration",
            &["arrivals", "ridgeline", "departures"],
        ),
        make(
            "☰",
            "Correlation",
            "co-occurrence",
            "/correlation",
            &["matrix", "pairs"],
        ),
        make("∷", "Time series", "trends", "/timeseries", &["trend", "compare"]),
        make("◷", "History", "calendar", "/history", &["browse"]),
        make("✦", "Life list", "journal", "/life-list", &["lifer", "journal"]),
        make("◫", "Gallery", "photos", "/gallery", &["photo", "image"]),
        make(
            "▶",
            "Recordings",
            "clips",
            "/recordings",
            &["audio", "wav", "clip"],
        ),
        make(
            "⚠",
            "Quarantine",
            "review queue",
            "/quarantine",
            &["rare", "queue", "review"],
        ),
        make(
            "¶",
            "Weekly report",
            "Sunday recap",
            "/weekly",
            &["bulletin", "sunday"],
        ),
        make(
            "⊞",
            "Year in review",
            "annual recap",
            "/year-in-review",
            &["year", "annual"],
        ),
        make("◉", "Kiosk", "wall display", "/kiosk", &["ambient", "wall"]),
        make(
            "≡",
            "Notifications",
            "channels & log",
            "/notifications",
            &["alerts", "channel"],
        ),
        make("⚙", "System", "health", "/system", &["pi", "cpu", "disk"]),
        make("⌗", "Admin", "settings", "/admin", &["config"]),
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
            "/admin/settings",
            &["threshold", "sensitivity", "confidence"],
        ),
        make(
            "Audio",
            "sources",
            "/admin/audio",
            &["microphone", "mic", "rtsp", "usb", "alsa", "pipewire"],
        ),
        make(
            "Notifications",
            "channels",
            "/admin/notifications",
            &["telegram", "email", "mqtt", "slack", "webhook"],
        ),
        make(
            "Species",
            "filter",
            "/admin/species",
            &["exclude", "include", "allow", "list"],
        ),
        make("Rules", "alerts", "/admin/rules", &["alert", "rule"]),
        make(
            "Quality",
            "metrics",
            "/admin/quality",
            &["false positive", "low confidence"],
        ),
        make(
            "Accounts",
            "sessions",
            "/admin/accounts",
            &["users", "viewers", "sign out"],
        ),
        make(
            "Backups",
            "recovery",
            "/admin/backups",
            &["backup", "restore", "snapshot"],
        ),
        make(
            "Migrate",
            "import",
            "/admin/migration",
            &["birdnet-pi", "import"],
        ),
        make(
            "Diagnostics",
            "doctor",
            "/admin/doctor",
            &["health", "self-check"],
        ),
        make(
            "Display",
            "prefs",
            "/system#display-prefs",
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
                   FROM detections
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
                   FROM detections
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
        "last week" => ("Last week", "weekly report", "/weekly?offset=-1".to_string()),
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
        "this year" => (
            "This year",
            "year in review",
            "/year-in-review".to_string(),
        ),
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

#[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
fn ymd_to_days(y: u32, m: u32, d: u32) -> i64 {
    let y = i64::from(if m <= 2 { y - 1 } else { y });
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = u64::from(m);
    let d = u64::from(d);
    let doy = (153 * if m > 2 { m - 3 } else { m + 9 } + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_index_has_all_topnav_links() {
        let p = all_pages();
        let labels: Vec<_> = p.iter().map(|e| e.label.as_str()).collect();
        for must in [
            "Dashboard",
            "Today",
            "Species",
            "Heatmap",
            "Migration",
            "Life list",
            "Quarantine",
            "System",
        ] {
            assert!(
                labels.contains(&must),
                "missing {must} in cmdk pages index"
            );
        }
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
}
