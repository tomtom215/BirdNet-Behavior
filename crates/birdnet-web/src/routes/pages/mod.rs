//! HTMX page and partial routes.
//!
//! Split into focused sub-modules by concern:
//!
//! | Module           | Responsibility                                  |
//! |------------------|-------------------------------------------------|
//! | `dashboard`      | Main dashboard page and stats/detection partials|
//! | `charts`         | SVG chart rendering helpers                     |
//! | `health`         | Health badge and disk status partials           |
//! | `species_pages`  | Species list, detail page, species partials     |
//! | `behavioral`     | Behavioral analytics HTMX partials              |
//! | `timeseries_dash`| Time-series analytics page and partials         |
//! | `heatmap`        | 24h × 7-day activity heatmap page               |
//! | `correlation`    | Species co-occurrence correlation page          |

pub mod behavioral;
pub mod charts;
pub mod correlation;
pub mod dashboard;
pub mod detection_detail;
pub mod health;
pub mod heatmap;
pub mod livestream;
pub mod recordings;
pub mod species_pages;
pub mod timeseries_dash;
pub mod today;

use axum::Router;
use axum::response::Html;

use crate::state::AppState;

// Embedded HTML templates (compiled into the binary).
pub(crate) const LAYOUT_HTML: &str = include_str!("../../../templates/layout.html");
pub(crate) const DASHBOARD_HTML: &str = include_str!("../../../templates/dashboard.html");
pub(crate) const SPECIES_PAGE_HTML: &str = include_str!("../../../templates/species.html");
pub(crate) const ANALYTICS_PAGE_HTML: &str = include_str!("../../../templates/analytics.html");
pub(crate) const SPECIES_DETAIL_HTML: &str =
    include_str!("../../../templates/species_detail.html");
pub(crate) const TIMESERIES_PAGE_HTML: &str =
    include_str!("../../../templates/timeseries.html");
pub(crate) const TODAY_PAGE_HTML: &str = include_str!("../../../templates/today.html");
pub(crate) const RECORDINGS_PAGE_HTML: &str = include_str!("../../../templates/recordings.html");

/// Build all page and partial routes.
pub fn router() -> Router<AppState> {
    dashboard::router()
        .merge(health::router())
        .merge(detection_detail::router())
        .merge(species_pages::router())
        .merge(behavioral::router())
        .merge(timeseries_dash::router())
        .merge(heatmap::router())
        .merge(correlation::router())
        .merge(today::router())
        .merge(recordings::router())
        .merge(livestream::router())
}

/// Render a full page by substituting content into the layout template.
pub(crate) fn render_page(title: &str, content: &str, active_nav: &str) -> Html<String> {
    let version = env!("CARGO_PKG_VERSION");
    let nav = |key| {
        if active_nav == key {
            "active"
        } else {
            ""
        }
    };
    let html = LAYOUT_HTML
        .replace("{{title}}", title)
        .replace("{{content}}", content)
        .replace("{{version}}", version)
        .replace("{{nav_dashboard}}", nav("dashboard"))
        .replace("{{nav_today}}", nav("today"))
        .replace("{{nav_species}}", nav("species"))
        .replace("{{nav_recordings}}", nav("recordings"))
        .replace("{{nav_analytics}}", nav("analytics"))
        .replace("{{nav_timeseries}}", nav("timeseries"));
    Html(html)
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

/// Count detections for today's date in SQLite.
pub(crate) fn today_count(conn: &rusqlite::Connection) -> i64 {
    let today = today_date_string();
    conn.query_row(
        "SELECT COUNT(*) FROM detections WHERE Date = ?1",
        [&today],
        |row| row.get(0),
    )
    .unwrap_or(0)
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
    fn render_page_nav_active() {
        let html = render_page("Test", "<p>hi</p>", "dashboard");
        assert!(html.0.contains("class=\"active\""));
    }
}
