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
//! | `station_health`      | Station Health surface (public `/station` tab)  |
//! | `notification_center` | Notification history and channel status          |

pub mod atoms;
pub mod audio_player;
pub mod behavioral;
pub(crate) mod changelog;
pub mod charts;
pub(crate) mod cmdk;
pub(crate) mod confirm;
pub mod correlation;
pub mod dashboard;
pub mod dawn_chorus;
pub mod detection_detail;
pub mod detection_reviews;
pub mod empty_states;
pub mod health;
pub mod heatmap;
pub(crate) mod help;
pub mod history;
pub mod homes;
pub mod life_list;
pub(crate) mod listen;
pub mod migration;
pub(crate) mod nav;
pub mod notification_center;
pub mod onboarding;
pub(crate) mod overlays;
pub mod provenance;
pub mod quarantine;
pub mod recordings;
pub(crate) mod skeletons;
pub mod species_pages;
pub mod station_health;
pub mod timeseries_dash;
pub(crate) mod toast;
pub mod today;
pub(crate) mod today_phrase;
pub mod viz;
pub mod weekly_report;
pub mod year_in_review;

use axum::Router;
use axum::response::Html;
use axum::routing::get;

use crate::state::AppState;

// Embedded HTML templates (compiled into the binary).
pub(crate) const LAYOUT_HTML: &str = include_str!("../../../templates/layout.html");
pub(crate) const ANALYTICS_PAGE_HTML: &str = include_str!("../../../templates/analytics.html");
pub(crate) const SPECIES_DETAIL_HTML: &str = include_str!("../../../templates/species_detail.html");
pub(crate) const TIMESERIES_PAGE_HTML: &str = include_str!("../../../templates/timeseries.html");
pub(crate) const TODAY_PAGE_HTML: &str = include_str!("../../../templates/today.html");
/// Themed confirmation modal (O-17), injected into every full-page shell.
pub(crate) const CONFIRM_MODAL_HTML: &str =
    include_str!("../../../templates/_partial_confirm_modal.html");
/// Toast / snackbar live region (O-18), injected into every full-page shell.
pub(crate) const TOAST_REGION_HTML: &str =
    include_str!("../../../templates/_partial_toast_region.html");
/// Real footer (O-26) — site-meta only, destinations live in the topnav.
pub(crate) const FOOTER_HTML: &str = include_str!("../../../templates/_partial_footer.html");
/// Command palette overlay (O-19), injected into every full-page shell.
pub(crate) const CMDK_HTML: &str = include_str!("../../../templates/_partial_cmdk.html");
/// Help-drawer dialog (O-20), injected into every full-page shell.
pub(crate) const HELP_DRAWER_HTML: &str =
    include_str!("../../../templates/_partial_help_drawer.html");
/// Post-upgrade banner mount (O-21), injected at the top of `<main>`.
pub(crate) const UPDATE_BANNER_HTML: &str =
    include_str!("../../../templates/_partial_update_banner.html");
/// Phone-only bottom tab bar (O-24), injected before `</body>`.
pub(crate) const TABBAR_HTML: &str = include_str!("../../../templates/_partial_tabbar.html");

/// Pre-compute and cache the heaviest analytics fragments at their default
/// parameters.
///
/// Called once shortly after startup and then periodically by a background task
/// so the multi-second aggregate queries behind the Heatmap / phenology /
/// co-occurrence / time-series pages are already warm when an operator first
/// opens them — the "pre-warmed queries" that keep page-to-page navigation snappy
/// on a Raspberry Pi. Each page contributes its own `prewarm`; failures inside a
/// page are swallowed there (best-effort), so one slow query never blocks the
/// rest.
pub fn prewarm_analytics(state: &AppState) {
    heatmap::prewarm(state);
    migration::prewarm(state);
    correlation::prewarm(state);
    timeseries_dash::prewarm(state);
}

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
        .merge(provenance::router())
        .merge(quarantine::router())
        .merge(today::router())
        .merge(recordings::router())
        .merge(weekly_report::router())
        .merge(history::router())
        .merge(life_list::router())
        .merge(notification_center::router())
        .merge(year_in_review::router())
        .merge(onboarding::router())
        .merge(migration::router())
        .merge(dawn_chorus::router())
        .merge(homes::router())
        .merge(cmdk::router())
        .merge(help::router())
        .merge(changelog::router())
        .route(
            "/pages/today-phrase",
            get(today_phrase::today_phrase_partial),
        )
}

/// Sign-out form fragment rendered into the topnav's `{{sign_out_link}}`
/// slot when the request carries a valid `bnb-session` cookie. Posts to
/// `/logout` which revokes the bound session row and clears the cookie.
/// Matches the form shipped inside the admin shell for visual parity.
pub(crate) const SIGN_OUT_LINK_HTML: &str = r#"<form action="/logout" method="post" class="topnav-signout pm-signout-form">
  <button type="submit" class="bnb-btn ghost pm-signout-btn">Sign out</button>
</form>"#;

/// Render a full page, populating the `{{sign_out_link}}` slot when the
/// request carries a valid `bnb-session` cookie.
///
/// The check is HMAC-only (see [`crate::session::looks_signed_in`]) — no
/// DB round-trip, no extension lookup. A revoked-but-still-in-browser
/// cookie may surface the sign-out link; the subsequent `POST /logout`
/// is idempotent and clears the dead cookie, so no harm done.
pub(crate) fn render_page_for_request(
    title: &str,
    content: &str,
    active_nav: &str,
    headers: &axum::http::HeaderMap,
) -> Html<String> {
    render_page_inner(
        title,
        content,
        active_nav,
        crate::session::looks_signed_in(headers),
    )
}

fn render_page_inner(
    title: &str,
    content: &str,
    active_nav: &str,
    signed_in: bool,
) -> Html<String> {
    let version = env!("CARGO_PKG_VERSION");
    let sign_out_link = if signed_in { SIGN_OUT_LINK_HTML } else { "" };
    // Live process uptime for the topnav pill; empty when unavailable (non-Linux
    // or `/proc` unreadable) so the O-26 `[data-empty-hide=""]` rule hides it.
    let uptime_short = crate::system_info::process_uptime_secs()
        .map(crate::system_info::format_uptime)
        .unwrap_or_default();
    // Both navigation surfaces (desktop top-nav, phone bottom bar) are
    // generated from the single `nav` manifest — the v3 spine's six homes — so
    // they can't drift apart. Active-state is derived from the page's
    // `active_nav` key.
    let topnav_links = nav::topnav_links(active_nav);
    let tabbar_slots = nav::tabbar_slots(active_nav);
    // The partial *shells* (the `<dialog>`/`<nav>` chrome + their scripts) are
    // inlined first; the tab bar's `{{tabbar_slots}}` slot is filled by the
    // manifest-generated list in the same pass.
    let html = LAYOUT_HTML
        .replace("{{title}}", title)
        .replace("{{active_nav}}", active_nav)
        .replace("{{content}}", content)
        .replace("{{topnav_links}}", &topnav_links)
        .replace("{{footer}}", FOOTER_HTML)
        .replace("{{tabbar}}", TABBAR_HTML)
        .replace("{{tabbar_slots}}", &tabbar_slots)
        .replace("{{sign_out_link}}", sign_out_link)
        .replace("{{version}}", version)
        .replace("{{uptime_short}}", &uptime_short)
        // Inline the update banner partial BEFORE the final `{{version}}`
        // pass below, so the banner's `data-current-version="{{version}}"`
        // resolves. Previously this happened after the version pass, which
        // left the banner with the literal placeholder + the dismissal-
        // localStorage key disabled by an empty `currentVersion`.
        .replace("{{update_banner}}", UPDATE_BANNER_HTML)
        .replace("{{confirm_modal}}", CONFIRM_MODAL_HTML)
        .replace("{{toast_region}}", TOAST_REGION_HTML)
        .replace("{{cmdk_partial}}", CMDK_HTML)
        .replace("{{help_drawer}}", HELP_DRAWER_HTML)
        // Second `{{version}}` substitution pass — picks up any `{{version}}`
        // tokens that landed via the partials inlined above (currently the
        // update banner's `data-current-version`).
        .replace("{{version}}", version);
    Html(html)
}

/// Friendly `404` page rendered in the full app layout. Wired as the router
/// fallback so a mistyped URL gets the branded shell and a way back, rather
/// than an empty body.
///
/// Unmatched paths under `/api/` get a machine-readable JSON 404 instead:
/// API consumers are scripts and dashboards, and feeding them a full HTML
/// page hides the actual failure (and bloats every typo'd poll).
pub(crate) async fn not_found(
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    if uri.path().starts_with("/api/") {
        return (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": "not found",
                "path": uri.path(),
            })),
        )
            .into_response();
    }

    let body = render_page_for_request(
        "Page not found",
        r#"<section class="bnb-card pm-404-card">
  <div class="display pm-404-code">404</div>
  <h1 class="pm-404-title">That page flew off</h1>
  <p class="pm-404-text">The link may be stale, or the page may have moved. Check the address, or head back to the dashboard.</p>
  <a class="bnb-btn" href="/">Back to the dashboard</a>
</section>"#,
        "",
        &headers,
    );
    (axum::http::StatusCode::NOT_FOUND, body).into_response()
}

// ---------------------------------------------------------------------------
// Shared utilities (used across multiple sub-modules)
// ---------------------------------------------------------------------------

/// Minimal HTML escaping for XSS prevention.
/// # The only HTML escaper in this crate
///
/// There were three, and they were not the same. This one escaped
/// `& < > " '`; the copies in `admin/migration/render.rs` and
/// `admin/backup_recovery.rs` escaped `& < > "` and omitted the apostrophe.
/// Neither of those two happened to interpolate into a single-quoted attribute,
/// so the difference was latent rather than exploitable — but "latent" is a
/// property of today's call sites, not of the function, and nothing kept the
/// three in step. Escaping is not a place to have three answers.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Minimal percent-encoding for URL path segments and query values.
pub(crate) fn simple_url_encode(s: &str) -> String {
    crate::urls::encode_segment(s)
}

/// Seconds since the Unix epoch, saturating to 0 before it.
pub(crate) fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The system's UTC offset in seconds, east-positive (CEST → `7200`).
///
/// Detections are timestamped in **local** time — capture names its segment
/// files with local `%H:%M:%S` — so every hour-of-day and date this module
/// derives has to be local too, or the two disagree by the offset. They did:
/// the day strip plotted local-hour bars against a UTC "now" line and UTC
/// sunrise/sunset markers, which on a CEST station drew "now" two hours behind
/// the detections beside it.
///
/// Delegates to [`birdnet_db::clock::local_utc_offset_secs`], which is now the
/// single home of that rule: capture needs the very same offset to *write* the
/// filenames this module reads back, and a second copy would let the writer and
/// the reader drift apart.
pub(crate) fn local_utc_offset_secs() -> i64 {
    birdnet_db::clock::local_utc_offset_secs()
}

/// Today's sunrise/sunset as fractional hours in **local** time, from the
/// configured station location. `None` when no location is set or the sun never
/// rises/sets at this latitude today.
///
/// # Why local, and why this is shared
///
/// Every hour-of-day axis in this app is local: the day strip's bars come from
/// `hourly_activity`, which buckets the local `Time` column, and the "now"
/// marker is [`now_hour_local`]. Returning the solver's raw UTC minutes drew
/// sunrise two hours early on a CEST station and mislabelled the pills with it.
///
/// The Today page was fixed; the dawn-chorus polar chart kept a private copy
/// that was wrong three ways at once, which is why this now lives here instead
/// of there:
///
/// * it read the coordinates from `BNB_STATION_LAT`/`BNB_STATION_LON` only,
///   falling back to a hard-coded (40.0 N, -74.0 W) — so a station that set its
///   location in the setup wizard got sun markers for the New Jersey coast;
/// * its day-of-year was `((unix_secs / 86_400) % 365) + 1`, which drifts about
///   a day a year (14 days out by 2026, moving sunrise 18 min at Boston, 27 min
///   at London, 40 min at Oslo) and wraps to January in late December;
/// * it returned UTC hours while the chorus ribbons it was drawn over are
///   bucketed from the local `Time` column.
///
/// [`birdnet_scheduler::SolarDay`] already had a correct, leap-aware
/// implementation. One caller, one clock, one day-of-year.
pub(crate) fn solar_times_local(conn: &rusqlite::Connection) -> Option<(f64, f64)> {
    let lat: f64 = birdnet_db::settings::get_or(conn, "latitude", "")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let lon: f64 = birdnet_db::settings::get_or(conn, "longitude", "")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let location = birdnet_scheduler::Location::new(lat, lon).ok()?;
    let date = today_date_string();
    let year: u32 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    let solar = birdnet_scheduler::SolarDay::for_date(location, year, month, day).ok()?;
    #[allow(clippy::cast_precision_loss)]
    let offset_h = local_utc_offset_secs() as f64 / 3600.0;
    let sunrise = wrap_hour(f64::from(solar.sunrise_utc_min?) / 60.0 + offset_h);
    let sunset = wrap_hour(f64::from(solar.sunset_utc_min?) / 60.0 + offset_h);
    Some((sunrise, sunset))
}

/// Fold an hour-of-day into `[0, 24)` after a UTC→local shift.
///
/// A station far enough east or west pushes sunrise past midnight; without this
/// the value leaves the axis the strip draws and the marker vanishes off one
/// end rather than wrapping to the other.
pub(crate) fn wrap_hour(h: f64) -> f64 {
    h.rem_euclid(24.0)
}

/// Current hour-of-day in **local** time as a fraction (09:43 → `9.72`).
///
/// This is the axis the day strip's bars live on, because they are bucketed
/// from the local `Time` column.
pub(crate) fn now_hour_local() -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let h = (unix_secs() + local_utc_offset_secs()).rem_euclid(86_400) as f64 / 3600.0;
    h
}

/// Get today's **local** date as a YYYY-MM-DD string (no external crate needed).
///
/// Local, not UTC: detections are stored with a local `Date`, so a UTC "today"
/// selected the wrong day for however many hours the station sits ahead of UTC
/// (on CEST, every detection between 00:00 and 02:00 local landed on a date the
/// page never asked for).
pub(crate) fn today_date_string() -> String {
    let secs = (unix_secs() + local_utc_offset_secs()).max(0);
    #[allow(clippy::cast_sign_loss)]
    let (y, m, d) = days_to_date(secs as u64 / 86400);
    format!("{y}-{m:02}-{d:02}")
}

/// Convert days since Unix epoch to (year, month, day).
///
/// Delegates to [`birdnet_core::civil::civil_from_days`]. This was a verbatim
/// copy of Hinnant's algorithm — one of nine in the workspace, all of which
/// agreed when checked against each other over 200 years. The consolidation
/// fixes nothing that was broken; it removes the tenth copy's chance to be the
/// one that isn't.
#[allow(clippy::cast_possible_wrap)]
pub(crate) const fn days_to_date(days_since_epoch: u64) -> (u32, u32, u32) {
    birdnet_core::civil::civil_from_days(days_since_epoch as i64)
}

/// Convert a `YYYY-MM-DD` date to days since the Unix epoch (rata die) — the
/// inverse of [`days_to_date`]. Shared by the weekly-report, history and
/// year-in-review pages.
///
/// Reads the year/month/day with char-boundary-safe [`str::get`] rather than
/// `date[a..b]` indexing: a date that is long enough to clear the length check
/// but carries a multibyte UTF-8 byte at a slice boundary (a corrupt or
/// imported row) would make a byte-index slice panic, and with `panic = "abort"`
/// that crashes the whole process. Unparseable parts fall back to the
/// epoch-date defaults (1970-01-01), so a malformed date degrades to `0`
/// instead of taking the station down.
pub(crate) fn date_to_epoch_days(date: &str) -> u64 {
    if date.len() < 10 {
        return 0;
    }
    let part = |range: std::ops::Range<usize>, default: u64| {
        date.get(range)
            .and_then(|s| s.parse().ok())
            .unwrap_or(default)
    };
    let y = part(0..4, 1970);
    let m = part(5..7, 1);
    let d = part(8..10, 1);

    // Contract: Gregorian dates from the Unix epoch onward. A pre-1970 or
    // out-of-range date (e.g. "0000-..." or "...-00") returns 0 rather than
    // reaching the rata-die arithmetic below, whose `y - 1` / `d - 1` /
    // `… - 719_468` would underflow — wrapping to garbage in release and
    // panicking in a debug/test build. Real dates (the only callers) are
    // unaffected; only out-of-range input collapses to the epoch sentinel.
    if y < 1970 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return 0;
    }

    // Rata Die day number. The guard above keeps this in range; the shared
    // primitive clamps a pre-year-0 date to 0 for the same reason.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let days = birdnet_core::civil::days_from_civil(y as u32, m as u32, d as u32);
    #[allow(clippy::cast_sign_loss)]
    let out = days.max(0) as u64;
    out
}

/// Count detections for today's date in `SQLite`.
pub(crate) fn today_count(conn: &rusqlite::Connection) -> i64 {
    // `detections_analytic`, not `detections`: this is a number shown to an
    // operator, and a detection they rejected is one they have said was not
    // there. Its neighbours on the dashboard tile row ("Species", "Last hour",
    // the sparkline) have always excluded rejections, so counting every row
    // here made adjacent tiles disagree about the same day.
    let today = today_date_string();
    birdnet_db::sqlite::analytic_detection_count_for_date(conn, &today).unwrap_or(0)
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

    /// The canonical escaper must cover the apostrophe.
    ///
    /// Two of the three implementations this replaced did not, which is safe
    /// only for as long as nobody writes `attr='{value}'`. The gate is on the
    /// escaper rather than on the call sites, because the call sites are what
    /// change.
    #[test]
    fn escape_html_covers_every_character_that_can_break_out() {
        assert_eq!(
            escape_html(r#"&<>"'"#),
            "&amp;&lt;&gt;&quot;&#x27;",
            "all five must be escaped, in one pass, ampersand first"
        );
        // Ampersand first, or the entities themselves get double-escaped.
        assert_eq!(escape_html("&amp;"), "&amp;amp;");
        // A single-quoted attribute must not be escapable.
        assert!(!escape_html("' onerror='alert(1)").contains('\''));
    }

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
    fn date_to_epoch_days_round_trips_days_to_date() {
        assert_eq!(date_to_epoch_days("1970-01-01"), 0);
        assert_eq!(date_to_epoch_days("2026-03-12"), 20524);
        // Inverse of days_to_date over a span of dates.
        for days in [0_u64, 1, 365, 20_000, 20_524, 50_000] {
            let (y, m, d) = days_to_date(days);
            assert_eq!(date_to_epoch_days(&format!("{y}-{m:02}-{d:02}")), days);
        }
    }

    #[test]
    fn date_to_epoch_days_tolerates_malformed_input() {
        // Too short → 0 (no panic).
        assert_eq!(date_to_epoch_days("2026"), 0);
        assert_eq!(date_to_epoch_days(""), 0);
        // A 10-byte string whose bytes don't parse falls back to the epoch.
        assert_eq!(date_to_epoch_days("not-a-date!"), 0);
    }

    #[test]
    fn date_to_epoch_days_clamps_out_of_range_without_underflow() {
        // Regression: these would underflow the rata-die `y - 1` / `d - 1` /
        // `… - 719_468` — panicking in a debug build and wrapping to a garbage
        // value in release. They must now collapse to the epoch sentinel.
        assert_eq!(date_to_epoch_days("0000-01-01"), 0); // y - 1 underflow
        assert_eq!(date_to_epoch_days("2026-00-15"), 0); // month 0
        assert_eq!(date_to_epoch_days("2026-13-15"), 0); // month 13
        assert_eq!(date_to_epoch_days("2026-03-00"), 0); // day 0 (d - 1 underflow)
        assert_eq!(date_to_epoch_days("1969-12-31"), 0); // pre-epoch
    }

    #[test]
    fn date_to_epoch_days_does_not_panic_on_multibyte_date() {
        // Regression: `date[5..7]` / `date[8..10]` byte-slicing panics when a
        // multibyte UTF-8 char straddles a slice boundary. With `panic =
        // "abort"` that would crash the process from one corrupt/imported row.
        // "2026-1é-9" is 10 bytes (é is 2) with the boundary mid-char.
        let multibyte = "2026-1\u{e9}-9";
        assert_eq!(multibyte.len(), 10);
        // Must return a value, not panic. The exact number is unimportant; the
        // month part fails to parse and falls back, so it stays finite.
        let _ = date_to_epoch_days(multibyte);
        // A trailing multibyte char at the very end also must not panic.
        let _ = date_to_epoch_days("2026-03-1\u{e9}");
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
        use axum::http::HeaderMap;
        let html = render_page_for_request("Test", "<p>hi</p>", "today", &HeaderMap::new());
        // The active section link carries the `active` modifier alongside the
        // base `topnav-link` class, and the content is substituted in.
        assert!(html.0.contains("topnav-link active"));
        assert!(html.0.contains("<p>hi</p>"));
        // The page's nav key reaches the shell for per-home styling.
        assert!(html.0.contains(r#"data-home="today""#));
        // The uptime pill slot is always substituted (wired, never left literal).
        assert!(!html.0.contains("{{uptime_short}}"));
    }

    #[test]
    fn render_page_for_request_shows_sign_out_with_valid_cookie() {
        use axum::http::{HeaderMap, HeaderValue, header};
        let sid = crate::session::generate_session_id();
        let token = crate::session::issue_token(&sid, 60_000);
        let mut headers = HeaderMap::new();
        let cookie = format!("{}={}", crate::session::COOKIE_NAME, token);
        headers.insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        let html = render_page_for_request("Test", "<p>hi</p>", "dashboard", &headers);
        assert!(html.0.contains("/logout"));
        assert!(html.0.contains("Sign out"));
    }

    #[test]
    fn render_page_for_request_omits_sign_out_without_cookie() {
        use axum::http::HeaderMap;
        let headers = HeaderMap::new();
        let html = render_page_for_request("Test", "<p>hi</p>", "dashboard", &headers);
        assert!(!html.0.contains("/logout"));
    }
}
