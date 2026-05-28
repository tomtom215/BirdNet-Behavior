//! HTMX page and partial routes.
//!
//! Split into focused sub-modules by concern:
//!
//! | Module                | Responsibility                                  |
//! |-----------------------|-------------------------------------------------|
//! | `dashboard`           | Main dashboard page and stats/detection partials|
//! | `charts`              | SVG chart rendering helpers                     |
//! | `health`              | Health badge and disk status partials           |
//! | `species_pages`       | Species list, detail page, species partials     |
//! | `behavioral`          | Behavioral analytics HTMX partials              |
//! | `timeseries_dash`     | Time-series analytics page and partials         |
//! | `heatmap`             | 24h × 7-day activity heatmap page               |
//! | `correlation`         | Species co-occurrence correlation page          |
//! | `quarantine`          | Rare-bird quarantine review page and actions    |
//! | `life_list`           | Life list / birding journal page                |
//! | `system_dashboard`    | System health monitoring dashboard              |
//! | `notification_center` | Notification history and channel status          |

pub mod atoms;
pub mod audio_player;
pub mod behavioral;
pub mod charts;
pub(crate) mod cmdk;
pub(crate) mod confirm;
pub mod correlation;
pub mod dashboard;
pub mod dawn_chorus;
pub mod detection_detail;
pub mod detection_reviews;
pub mod empty_states;
pub mod gallery;
pub mod health;
pub mod heatmap;
pub mod history;
pub mod life_list;
pub mod livestream;
pub mod migration;
pub mod notification_center;
pub mod onboarding;
pub mod quarantine;
pub mod recordings;
pub(crate) mod skeletons;
pub mod species_pages;
pub mod system_dashboard;
pub mod timeseries_dash;
pub mod today;
pub(crate) mod today_phrase;
pub(crate) mod toast;
pub mod viz;
pub mod weekly_report;
pub mod year_in_review;

use axum::Router;
use axum::response::Html;
use axum::routing::get;

use crate::state::AppState;

// Embedded HTML templates (compiled into the binary).
pub(crate) const LAYOUT_HTML: &str = include_str!("../../../templates/layout.html");
pub(crate) const DASHBOARD_HTML: &str = include_str!("../../../templates/dashboard.html");
pub(crate) const SPECIES_PAGE_HTML: &str = include_str!("../../../templates/species.html");
pub(crate) const ANALYTICS_PAGE_HTML: &str = include_str!("../../../templates/analytics.html");
pub(crate) const SPECIES_DETAIL_HTML: &str = include_str!("../../../templates/species_detail.html");
pub(crate) const TIMESERIES_PAGE_HTML: &str = include_str!("../../../templates/timeseries.html");
pub(crate) const TODAY_PAGE_HTML: &str = include_str!("../../../templates/today.html");
pub(crate) const RECORDINGS_PAGE_HTML: &str = include_str!("../../../templates/recordings.html");
/// Themed confirmation modal (O-17), injected into every full-page shell.
pub(crate) const CONFIRM_MODAL_HTML: &str =
    include_str!("../../../templates/_partial_confirm_modal.html");
/// Toast / snackbar live region (O-18), injected into every full-page shell.
pub(crate) const TOAST_REGION_HTML: &str =
    include_str!("../../../templates/_partial_toast_region.html");
/// Topnav "More" overflow menu (O-26) — grouped secondary navigation.
pub(crate) const TOPNAV_MORE_HTML: &str =
    include_str!("../../../templates/_partial_topnav_more.html");
/// Real footer (O-26) — site-meta only, destinations live in the topnav + More.
pub(crate) const FOOTER_HTML: &str =
    include_str!("../../../templates/_partial_footer.html");
/// Command palette overlay (O-19), injected into every full-page shell.
pub(crate) const CMDK_HTML: &str = include_str!("../../../templates/_partial_cmdk.html");

/// Build all page and partial routes.
pub fn router() -> Router<AppState> {
    dashboard::router()
        .merge(audio_player::router())
        .merge(health::router())
        .merge(detection_detail::router())
        .merge(detection_reviews::router())
        .merge(species_pages::router())
        .merge(behavioral::router())
        .merge(timeseries_dash::router())
        .merge(heatmap::router())
        .merge(correlation::router())
        .merge(quarantine::router())
        .merge(today::router())
        .merge(recordings::router())
        .merge(livestream::router())
        .merge(weekly_report::router())
        .merge(history::router())
        .merge(life_list::router())
        .merge(gallery::router())
        .merge(system_dashboard::router())
        .merge(notification_center::router())
        .merge(year_in_review::router())
        .merge(onboarding::router())
        .merge(migration::router())
        .merge(dawn_chorus::router())
        .merge(cmdk::router())
        .route(
            "/pages/today-phrase",
            get(today_phrase::today_phrase_partial),
        )
}

/// Render a full page by substituting content into the layout template.
pub(crate) fn render_page(title: &str, content: &str, active_nav: &str) -> Html<String> {
    let version = env!("CARGO_PKG_VERSION");
    let nav = |key| {
        if active_nav == key { "active" } else { "" }
    };
    // Insert the layout partials FIRST so their own `{{nav_*}}` / `{{version}}`
    // / `{{uptime_short}}` placeholders are resolved by the subsequent passes.
    // (O-26's topnav-more + footer both reference those slots.)
    let html = LAYOUT_HTML
        .replace("{{title}}", title)
        .replace("{{content}}", content)
        .replace("{{topnav_more}}", TOPNAV_MORE_HTML)
        .replace("{{footer}}", FOOTER_HTML)
        // O-14 — populated by the cookie auth wire once it's flipped. Empty
        // for now so the slot is harmless on unauthenticated requests; the
        // CSS handles the missing element gracefully (no layout shift).
        // TODO(O-14-followup): substitute the rendered "Sign out" form when
        // the request carries a valid `bnb-session` cookie.
        .replace("{{sign_out_link}}", "")
        .replace("{{version}}", version)
        // Live uptime is not wired here yet — empty value triggers the
        // `[data-empty-hide=""]` rule in the O-26 CSS so the pill stays hidden.
        .replace("{{uptime_short}}", "")
        .replace("{{nav_dashboard}}", nav("dashboard"))
        .replace("{{nav_today}}", nav("today"))
        .replace("{{nav_species}}", nav("species"))
        .replace("{{nav_recordings}}", nav("recordings"))
        .replace("{{nav_analytics}}", nav("analytics"))
        .replace("{{nav_timeseries}}", nav("timeseries"))
        .replace("{{nav_history}}", nav("history"))
        .replace("{{nav_weekly}}", nav("weekly"))
        .replace("{{nav_quarantine}}", nav("quarantine"))
        .replace("{{nav_life_list}}", nav("life-list"))
        .replace("{{nav_heatmap}}", nav("heatmap"))
        .replace("{{nav_migration}}", nav("migration"))
        .replace("{{nav_system}}", nav("system"))
        .replace("{{nav_notifications}}", nav("notifications"))
        // O-26 — slots referenced by the topnav-more partial.
        .replace("{{nav_year_in_review}}", nav("year_in_review"))
        .replace("{{nav_gallery}}", nav("gallery"))
        .replace("{{nav_dawn_chorus}}", nav("dawn_chorus"))
        .replace("{{nav_correlation}}", nav("correlation"))
        .replace("{{nav_admin}}", nav("admin"))
        .replace("{{nav_kiosk}}", nav("kiosk"))
        .replace("{{nav_changelog}}", nav("changelog"))
        .replace("{{nav_help}}", nav("help"))
        .replace("{{confirm_modal}}", CONFIRM_MODAL_HTML)
        .replace("{{toast_region}}", TOAST_REGION_HTML)
        .replace("{{cmdk_partial}}", CMDK_HTML);
    Html(html)
}

/// Friendly `404` page rendered in the full app layout. Wired as the router
/// fallback so a mistyped URL gets the branded shell and a way back, rather
/// than an empty body.
pub(crate) async fn not_found() -> impl axum::response::IntoResponse {
    let body = render_page(
        "Page not found",
        r#"<section class="bnb-card" style="max-width:560px;margin:48px auto;text-align:center;padding:40px 28px;">
  <div class="display" style="font-size:48px;line-height:1;margin-bottom:8px;">404</div>
  <h1 style="margin:0 0 10px;font-size:20px;">That page flew off</h1>
  <p style="opacity:.8;margin:0 0 20px;">The link may be stale, or the page may have moved. Check the address, or head back to the dashboard.</p>
  <a class="bnb-btn" href="/">Back to the dashboard</a>
</section>"#,
        "",
    );
    (axum::http::StatusCode::NOT_FOUND, body)
}

// ---------------------------------------------------------------------------
// Shared utilities (used across multiple sub-modules)
// ---------------------------------------------------------------------------

/// Minimal HTML escaping for XSS prevention.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Minimal percent-encoding for URL path segments and query values.
pub(crate) fn simple_url_encode(s: &str) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// Get today's date as YYYY-MM-DD string (no external crate needed).
pub(crate) fn today_date_string() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, d) = days_to_date(secs / 86400);
    format!("{y}-{m:02}-{d:02}")
}

/// Convert days since Unix epoch to (year, month, day) using the Hinnant algorithm.
#[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
pub(crate) const fn days_to_date(days_since_epoch: u64) -> (u32, u32, u32) {
    let z = days_since_epoch as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    #[allow(clippy::cast_sign_loss)]
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    #[allow(clippy::cast_sign_loss, clippy::cast_lossless)]
    let y = (yoe as i64 + era * 400) as u32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Count detections for today's date in `SQLite`.
pub(crate) fn today_count(conn: &rusqlite::Connection) -> i64 {
    let today = today_date_string();
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date = ?1",
        [&today],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Format an integer with thousands separators (e.g. 9914 → "9,914").
pub(crate) fn group_thousands(n: i64) -> String {
    let neg = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if neg { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_html_basic() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("\"hello\""), "&quot;hello&quot;");
    }

    #[test]
    fn days_to_date_epoch() {
        assert_eq!(days_to_date(0), (1970, 1, 1));
    }

    #[test]
    fn days_to_date_known() {
        // 2026-03-12 = 20524 days since epoch
        assert_eq!(days_to_date(20524), (2026, 3, 12));
    }

    #[test]
    fn today_date_string_format() {
        let date = today_date_string();
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }

    #[test]
    fn simple_url_encode_spaces() {
        assert_eq!(simple_url_encode("Pica pica"), "Pica%20pica");
    }

    #[test]
    fn simple_url_encode_preserves_unreserved() {
        assert_eq!(simple_url_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn group_thousands_formats() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(42), "42");
        assert_eq!(group_thousands(9914), "9,914");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(group_thousands(-12_345), "-12,345");
    }

    #[test]
    fn render_page_nav_active() {
        let html = render_page("Test", "<p>hi</p>", "dashboard");
        // The active section link carries the `active` modifier alongside the
        // base `topnav-link` class, and the content is substituted in.
        assert!(html.0.contains("topnav-link active"));
        assert!(html.0.contains("<p>hi</p>"));
        // Inactive sections must not be marked active.
        assert!(!html.0.contains("{{nav_dashboard}}"));
    }
}
