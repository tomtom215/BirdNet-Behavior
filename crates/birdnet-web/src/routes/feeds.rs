//! RSS + iCal feed endpoints for rare detections.
//!
//! Mounts:
//!   GET /feeds/rare.rss    — Atom-style RSS 2.0
//!   GET /feeds/rare.ics    — iCalendar VEVENT per confirmed rare detection
//!   GET /feeds/today.rss   — every detection today (very chatty)
//!
//! Pure read; no auth; respects ?token=… for non-default streams in case
//! you want to gate feeds in future. Cache-Control: 5-minute public for RSS,
//! 1-hour for iCal (calendar clients are slow to repoll).

// Adapted feed-rendering module: int<->float casts and short date-math
// identifiers (Zeller's congruence) are intrinsic here.
#![allow(clippy::pedantic, clippy::nursery)]

use std::fmt::Write as _;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::routes::pages::simple_url_encode;
use crate::state::AppState;

/// Mount the RSS and iCal feed routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/feeds/rare.rss", get(rare_rss))
        .route("/feeds/rare.ics", get(rare_ics))
        .route("/feeds/today.rss", get(today_rss))
}

#[derive(Deserialize, Default)]
struct FeedQuery {
    /// Override default limit (50 for RSS, 200 for iCal).
    limit: Option<i64>,
    /// Optional base URL — used when the device is behind a reverse proxy.
    base: Option<String>,
}

// ---------------------------------------------------------------------------
// Admission
// ---------------------------------------------------------------------------

// Feeds are public and each scans the detection history: the rare-bird query
// took 5.2 s on a 300 000-detection station, measured against the real binary.
// They used to take the single writer lock (`with_db`), which the detection
// processor and every other writer also take, so sixteen concurrent requests
// held every one of them for 81 s and a plain `/api/v2/detections?limit=1`
// waited 38.9 s. They read through the pooled readers now, inside the shared
// heavy-read gate ([`AppState::try_heavy_read_permit`]), so a flood cannot take
// every reader. Past the gate a request is turned away with a `503` and
// `Retry-After` rather than queued: a feed reader retries, and a queue is what
// held the threads.

/// Every slot is taken: `503`, uncached, with a short `Retry-After`.
fn feed_busy() -> Response {
    let mut resp = (
        StatusCode::SERVICE_UNAVAILABLE,
        "feed busy; try again shortly",
    )
        .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("15"));
    resp
}

// ---------------------------------------------------------------------------
// Rare RSS — last N "first today" or quarantine-confirmed detections.
// ---------------------------------------------------------------------------

/// SQL selecting the "rare" detections, given a gap threshold in days.
///
/// Two definitions of rare, and the threshold picks between them:
///
/// * `0` — the original behaviour: a species' very first sighting at this
///   station. Correct but permanently one-shot; once a bird has been recorded
///   once it can never be rare again, so one absent for three years came back
///   without comment.
/// * `n > 0` — BirdNET-Pi's `RARE_SPECIES_THRESHOLD`: a detection counts as
///   rare when the species has not been heard for `n` days. A first-ever
///   sighting still qualifies, because there is no previous detection at all.
///
/// The gap is measured against the previous detection of the same species
/// strictly *before* this one, which is what makes a returning bird rare on
/// the day it returns and ordinary for the rest of its stay.
///
/// The boundary is `<=`, not `<`. "Not heard for one day" has to exclude a
/// bird heard yesterday, and yesterday is exactly `1.0` julian days ago — with
/// `<` a resident recorded every single day came back rare on every one of
/// them, which `a_resident_is_never_rare` caught.
fn rare_detections_sql(gap_days: u32) -> String {
    if gap_days == 0 {
        return "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
                FROM detections_analytic d \
                WHERE d.Confidence > 0.85 \
                  AND (SELECT MIN(Date) FROM detections_analytic d2 \
                       WHERE d2.Com_Name = d.Com_Name) = d.Date \
                ORDER BY d.Date DESC, d.Time DESC \
                LIMIT ?1"
            .to_owned();
    }
    // The window is written out twice: as a range on `Date` (which the
    // `(Com_Name, Date)` index can seek to) and as the day count the rule is
    // stated in. For a valid date they say the same thing; the day count stays
    // so a malformed `Date` is excluded exactly as it always was. Without the
    // range the planner walked each species from its first day.
    format!(
        "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
         FROM detections_analytic d \
         WHERE d.Confidence > 0.85 \
           AND NOT EXISTS ( \
                 SELECT 1 FROM detections_analytic p \
                 WHERE p.Com_Name = d.Com_Name \
                   AND p.Date >= date(d.Date, '-{gap_days} days') \
                   AND p.Date < d.Date \
                   AND julianday(d.Date) - julianday(p.Date) <= {gap_days} \
               ) \
         ORDER BY d.Date DESC, d.Time DESC \
         LIMIT ?1"
    )
}

/// Read the configured rare-species gap, in days.
fn rare_gap_days(state: &AppState) -> u32 {
    state
        .with_read_db(|conn| {
            birdnet_db::settings::get_or(conn, "rare_species_days", "30")
                .unwrap_or_else(|_| "30".to_owned())
        })
        .trim()
        .parse::<u32>()
        .unwrap_or(30)
        .min(3650)
}

async fn rare_rss(
    State(state): State<AppState>,
    Query(q): Query<FeedQuery>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let base = resolve_base(q.base, request_authority(&uri, &headers).as_deref());
    let names = state.names_handle();
    let Some(permit) = state.try_heavy_read_permit() else {
        return feed_busy();
    };

    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        let sql = rare_detections_sql(rare_gap_days(&state));
        state.with_read_db(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([&limit], |r| {
                Ok(DetRow {
                    com: r.get(0)?,
                    sci: r.get(1)?,
                    date: r.get(2)?,
                    time: r.get(3)?,
                    conf: r.get(4)?,
                })
            })?;
            Ok::<_, rusqlite::Error>(rows.flatten().collect::<Vec<_>>())
        })
    })
    .await;

    let Some(rows) = result.ok().and_then(Result::ok) else {
        return feed_unavailable();
    };
    let body = build_rss(
        &rows,
        &base,
        "Rare birds",
        "/feeds/rare.rss",
        "First-ever detections at this station (confidence above 0.85).",
        &names,
    );

    rss_response(body)
}

async fn today_rss(
    State(state): State<AppState>,
    Query(q): Query<FeedQuery>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let base = resolve_base(q.base, request_authority(&uri, &headers).as_deref());
    let names = state.names_handle();
    let Some(permit) = state.try_heavy_read_permit() else {
        return feed_busy();
    };

    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        state.with_read_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT Com_Name, Sci_Name, Date, Time, Confidence \
                 FROM detections_analytic \
                 WHERE Date = date('now','localtime') \
                 ORDER BY Time DESC \
                 LIMIT ?1",
            )?;
            let rows = stmt.query_map([&limit], |r| {
                Ok(DetRow {
                    com: r.get(0)?,
                    sci: r.get(1)?,
                    date: r.get(2)?,
                    time: r.get(3)?,
                    conf: r.get(4)?,
                })
            })?;
            Ok::<_, rusqlite::Error>(rows.flatten().collect::<Vec<_>>())
        })
    })
    .await;

    let Some(rows) = result.ok().and_then(Result::ok) else {
        return feed_unavailable();
    };
    let body = build_rss(
        &rows,
        &base,
        "Today's detections",
        "/feeds/today.rss",
        "Every detection from this station today.",
        &names,
    );
    rss_response(body)
}

/// A feed whose read failed: `503`, uncached.
///
/// A failed read became a valid, empty feed with a five-minute cache, and a
/// feed reader told "nothing" replaces what it had with nothing. A `503` makes
/// the reader keep its last good copy and try again.
fn feed_unavailable() -> Response {
    tracing::error!("feed read failed");
    let mut resp = (
        StatusCode::SERVICE_UNAVAILABLE,
        "feed temporarily unavailable",
    )
        .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("300"));
    resp
}

fn rss_response(body: String) -> Response {
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/rss+xml; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        // `private`, like the recordings: on a private station a feed is served
        // only to a signed-in session, and a shared cache must not hand it on.
        HeaderValue::from_static("private, max-age=300"),
    );
    resp
}

// ---------------------------------------------------------------------------
// iCal feed — rare detections as point-in-time events.
// ---------------------------------------------------------------------------

async fn rare_ics(
    State(state): State<AppState>,
    Query(q): Query<FeedQuery>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let base = resolve_base(q.base, request_authority(&uri, &headers).as_deref());
    let names = state.names_handle();
    let Some(permit) = state.try_heavy_read_permit() else {
        return feed_busy();
    };

    let result = tokio::task::spawn_blocking(move || {
        let _slot = permit;
        let sql = rare_detections_sql(rare_gap_days(&state));
        state.with_read_db(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([&limit], |r| {
                Ok(DetRow {
                    com: r.get(0)?,
                    sci: r.get(1)?,
                    date: r.get(2)?,
                    time: r.get(3)?,
                    conf: r.get(4)?,
                })
            })?;
            Ok::<_, rusqlite::Error>(rows.flatten().collect::<Vec<_>>())
        })
    })
    .await;

    let Some(rows) = result.ok().and_then(Result::ok) else {
        return feed_unavailable();
    };
    let body = build_ics(&rows, &base, &names);

    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/calendar; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    resp
}

// ---------------------------------------------------------------------------
// Shape + render
// ---------------------------------------------------------------------------

struct DetRow {
    com: String,
    sci: String,
    date: String,
    time: String,
    conf: f64,
}

fn build_rss(
    rows: &[DetRow],
    base: &str,
    title: &str,
    self_path: &str,
    description: &str,
    names: &birdnet_core::i18n::NameLocalizer,
) -> String {
    let mut s = String::with_capacity(4096);
    let _ = write!(
        s,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
<channel>
<title>BirdNet-Behavior — {}</title>
<link>{base}/</link>
<atom:link href="{base}{self_path}" rel="self" type="application/rss+xml"/>
<description>{}</description>
<language>en</language>
<ttl>5</ttl>
"#,
        escape_xml(title),
        escape_xml(description),
    );

    // The station's own offset, so `pubDate` names the instant the detection
    // happened rather than relabelling local digits as Greenwich.
    let offset_secs = birdnet_db::clock::local_utc_offset_secs();
    for d in rows {
        let pub_date = rfc822(&d.date, &d.time, offset_secs);
        let conf_pct = (d.conf * 100.0).round() as i32;
        let link = escape_xml(&detail_url(base, &d.date, &d.time, &d.com));
        let _ = write!(
            s,
            r#"<item>
<title>{title}</title>
<link>{link}</link>
<guid isPermaLink="true">{link}</guid>
<pubDate>{pub}</pubDate>
<description><![CDATA[<p><strong>{title}</strong> — <em>{sci}</em></p><p>Heard at {date} {time} · confidence {conf}%.</p>]]></description>
<category>{cat}</category>
</item>
"#,
            title = escape_xml(names.name(&d.sci, &d.com)),
            sci = escape_xml(&d.sci),
            date = escape_xml(&d.date),
            time = escape_xml(&d.time),
            pub = pub_date,
            conf = conf_pct,
            cat = if conf_pct >= 90 { "high-confidence" } else { "rare" },
        );
    }

    s.push_str("</channel></rss>\n");
    s
}

fn build_ics(rows: &[DetRow], base: &str, names: &birdnet_core::i18n::NameLocalizer) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//BirdNet-Behavior//RareBirds 1.0//EN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nX-WR-CALNAME:BirdNet rare detections\r\nX-WR-CALDESC:First-of-station detections from your listening station.\r\n",
    );
    let stamp = ics_dtstamp_now();
    for d in rows {
        let dt = ics_local_datetime(&d.date, &d.time);
        let uid = ics_uid(&d.date, &d.time, &d.sci);
        let url = detail_url(base, &d.date, &d.time, &d.com);
        let name = escape_ics(names.name(&d.sci, &d.com));
        let sci = escape_ics(&d.sci);
        #[allow(clippy::cast_possible_truncation)]
        let conf = (d.conf * 100.0).round() as i32;
        for line in [
            "BEGIN:VEVENT".to_owned(),
            format!("UID:bnb-{uid}@birdnet-behavior"),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{dt}"),
            "DURATION:PT3M".to_owned(),
            format!("SUMMARY:{name} (rare)"),
            format!("DESCRIPTION:{name} — {sci} — confidence {conf}%.\\nListen: {url}"),
            format!("URL:{url}"),
            "CATEGORIES:rare-bird".to_owned(),
            "END:VEVENT".to_owned(),
        ] {
            fold_ics_line(&mut s, &line);
        }
    }
    s.push_str("END:VCALENDAR\r\n");
    s
}

// ---------------------------------------------------------------------------
// Format helpers
// ---------------------------------------------------------------------------

/// Public detail-page URL for a detection. Keyed on the app's real
/// `(Date, Time, Com_Name)` identity — there is no integer detection id.
fn detail_url(base: &str, date: &str, time: &str, com: &str) -> String {
    format!(
        "{base}/detections/detail?date={}&time={}&name={}",
        simple_url_encode(date),
        simple_url_encode(time),
        simple_url_encode(com),
    )
}

/// Stable iCal UID derived from the detection identity.
fn ics_uid(date: &str, time: &str, sci: &str) -> String {
    let slug: String = sci
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!(
        "{}-{}-{}",
        date.replace('-', ""),
        time.replace(':', ""),
        slug
    )
}

fn default_base_url() -> String {
    // The host header is more reliable; the implementer can swap in
    // `axum::extract::Host` if needed. For a Pi reachable at http://birdnet.local
    // the env var works fine.
    std::env::var("BNB_BASE_URL").unwrap_or_else(|_| "http://localhost:8502".to_string())
}

/// The `host[:port]` this request was addressed to: the `Host` header, or the
/// URI's authority for an HTTP/2 request, which carries none.
fn request_authority(uri: &Uri, headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| uri.authority().map(|a| a.as_str().to_owned()))
}

/// The `host[:port]` of each address the operator has configured for this
/// station: `BNB_PUBLIC_URL` and `BNB_BASE_URL`.
fn configured_authorities() -> Vec<String> {
    ["BNB_PUBLIC_URL", "BNB_BASE_URL"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .filter_map(|u| url_authority(&u).map(str::to_owned))
        .collect()
}

/// Resolve the feed base URL, honouring a client-supplied `?base=` only for an
/// address this station is already known by.
///
/// `base` is interpolated into RSS/Atom XML elements and attributes and into
/// iCal lines, and it decides where every item link in the feed leads. Two
/// checks, for two different harms:
///
/// * **Shape.** An unvalidated value (containing `"`, `<`, `>`, `&`, or CR/LF)
///   is a feed/XML-injection vector, so only an `http(s)://` origin built from
///   URL-unreserved characters is accepted — which excludes every XML/iCal
///   metacharacter.
/// * **Host.** A well-formed value is still not the caller's to choose.
///   `/feeds/rare.rss?base=https://evil.example` is a feed on this station's own
///   domain whose every link leads to someone else's page, and a feed address is
///   what people paste into a reader, a chat or an email. The host must be the
///   one the request was addressed to, or one the operator configured
///   (`BNB_PUBLIC_URL`, `BNB_BASE_URL`); what the override still adds is the
///   scheme and a path prefix, which is what a reverse proxy needs.
///
/// Anything else falls back to the server default rather than failing, so a
/// subscribed reader keeps working.
fn resolve_base(base: Option<String>, request_host: Option<&str>) -> String {
    resolve_base_with(base, request_host, &configured_authorities())
}

/// [`resolve_base`] with the configured authorities given explicitly.
fn resolve_base_with(
    base: Option<String>,
    request_host: Option<&str>,
    configured: &[String],
) -> String {
    match base {
        Some(b) if is_safe_base_url(&b) && host_is_known(&b, request_host, configured) => b,
        _ => default_base_url(),
    }
}

/// The authority (`host[:port]`) of an `http(s)://` URL, without userinfo
/// handling: [`is_safe_base_url`] has already refused an `@`.
fn url_authority(url: &str) -> Option<&str> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then_some(authority)
}

/// Whether `base`'s authority is the request's own host or a configured one.
/// Compared whole and case-insensitively: `station.local.evil.example` is not
/// `station.local`.
fn host_is_known(base: &str, request_host: Option<&str>, configured: &[String]) -> bool {
    let Some(authority) = url_authority(base) else {
        return false;
    };
    request_host
        .into_iter()
        .chain(configured.iter().map(String::as_str))
        .any(|known| known.trim().eq_ignore_ascii_case(authority))
}

/// Whether `b` is an `http(s)://host[:port][/path]` made only of URL-unreserved
/// characters (so it carries no XML/iCal-significant bytes).
fn is_safe_base_url(b: &str) -> bool {
    if b.is_empty() || b.len() > 256 {
        return false;
    }
    let Some(rest) = b
        .strip_prefix("https://")
        .or_else(|| b.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '~' | ':' | '/'))
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Escape a value for an iCalendar TEXT property (RFC 5545 §3.3.11).
///
/// Every line break becomes the two characters `\n`, whichever way it was
/// written: CRLF, LF, or a lone CR. The lone CR is the one that mattered — it
/// was passed through raw, and a CR is a line end to enough calendar parsers
/// that a species name containing one could start its own property. (Names come
/// from the model's label file and from imported databases, neither of which
/// promises a name without a control character.)
fn escape_ics(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// Convert a detection's local `Date` + `Time` to an RFC 822 `pubDate`.
///
/// `offset_secs` is the **station's** UTC offset, east-positive. It is not
/// cosmetic: a detection's `Date`/`Time` is local wall clock, and this used to
/// append a flat `+0000`, which asserts the station stands in Greenwich. Every
/// reader that localises a `pubDate` then shifted every item by the station's
/// whole offset — a 20:46 detection read as 16:46 on a UTC−4 station and as
/// 04:46 the next morning on UTC+8.
///
/// RSS has no floating-time form the way iCalendar does, so the offset has to
/// be stated. It is the station's *current* offset, which is exact for the
/// half of the year that offset holds and an hour out for the other half —
/// the schema stores no per-row offset to do better with. An hour is the
/// residual; the whole offset was the bug.
fn rfc822(date: &str, time: &str, offset_secs: i64) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return format!("{date} {time}");
    }
    let y: i32 = parts[0].parse().unwrap_or(1970);
    let m: u32 = parts[1].parse().unwrap_or(1);
    let d: u32 = parts[2].parse().unwrap_or(1);
    // Guard the month index: `month_name[m - 1]` panics on month 0 (subtraction
    // underflow) or > 12 (out of bounds). `Date` strings are not always
    // calendar-valid — rows imported from a BirdNET-Pi database aren't
    // validated — and this runs in the public, unauthenticated RSS/iCal feeds,
    // where with `panic = "abort"` a single bad row would crash the whole
    // process. Degrade to a plain string instead.
    if !(1..=12).contains(&m) {
        return format!("{date} {time}");
    }
    let dow = day_of_week(y, m, d);
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];
    let dow_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow as usize];
    let t = time.replace('Z', "");
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let (oh, om) = (offset_secs.abs() / 3600, (offset_secs.abs() % 3600) / 60);
    format!("{dow_name}, {d:02} {month_name} {y} {t} {sign}{oh:02}{om:02}")
}

/// A detection's local `Date` + `Time` as an iCalendar **floating** date-time.
///
/// No `Z`, and no `TZID`: RFC 5545 §3.3.5 form 1, which means "this wall-clock
/// time", not "this instant in Greenwich". That is what the station actually
/// knows — the row carries local digits and no offset — and it renders as the
/// time the operator saw on the dashboard.
///
/// It used to append `Z`, asserting UTC. On any station not on UTC that moved
/// every event in every subscriber's calendar by the station's whole offset.
/// The alternative fix — subtract today's offset and emit a true UTC instant —
/// was rejected because it is an hour wrong for every detection on the far side
/// of a daylight-saving boundary, and a value that is confidently wrong is
/// worse here than one that is honestly imprecise.
fn ics_local_datetime(date: &str, time: &str) -> String {
    // 20260312T061432
    let d = date.replace('-', "");
    let t = time.replace([':', 'Z'], "");
    format!("{d}T{t}")
}

/// Now, as an iCalendar UTC date-time.
///
/// `DTSTAMP` is when the calendar object was created, and RFC 5545 §3.8.7.2
/// says it MUST be UTC. The previous code reused the *detection's* local time
/// for it, which was the wrong property and the wrong clock at once.
fn ics_dtstamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// Fold one iCalendar content line to RFC 5545 §3.1's 75-octet limit.
///
/// Continuation lines begin with a single space, which a parser strips before
/// re-joining. The split is on **octets**, not chars, but never inside a UTF-8
/// sequence — species names carry em dashes and accented letters, and cutting
/// one in half would corrupt the value rather than merely lengthen the line.
fn fold_ics_line(out: &mut String, line: &str) {
    const LIMIT: usize = 75;
    let bytes = line.as_bytes();
    if bytes.len() <= LIMIT {
        out.push_str(line);
        out.push_str("\r\n");
        return;
    }
    let mut start = 0;
    let mut first = true;
    while start < bytes.len() {
        // A continuation line spends one octet on its leading space.
        let budget = if first { LIMIT } else { LIMIT - 1 };
        let mut end = (start + budget).min(bytes.len());
        while end > start && !line.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            // A single character wider than the budget cannot be split; emit
            // it whole rather than looping forever.
            end = line[start..]
                .char_indices()
                .nth(1)
                .map_or(bytes.len(), |(i, _)| start + i);
        }
        if !first {
            out.push(' ');
        }
        out.push_str(&line[start..end]);
        out.push_str("\r\n");
        start = end;
        first = false;
    }
}

/// Zeller's congruence; 0 = Sunday … 6 = Saturday.
fn day_of_week(y: i32, m: u32, d: u32) -> u32 {
    let (y, m) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let k = y.rem_euclid(100);
    let j = y.div_euclid(100);
    let h = (d as i32 + (13 * (m as i32 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    // Zeller: 0 = Saturday. Shift to 0 = Sunday.
    ((h + 6) % 7) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A detection's `Date`/`Time` is the station's **local** wall clock. The
    /// feeds stamped it `+0000` / `Z` — asserting it was UTC — so every
    /// calendar and every reader that localises a timestamp shifted every
    /// detection by the station's whole offset: a 20:46 detection on a UTC−4
    /// station showed as 16:46, and as 04:46 the next day on UTC+8.
    ///
    /// The wall-clock digits must survive unchanged; only the zone designator
    /// is allowed to move.
    #[test]
    fn rfc822_carries_the_stations_offset_not_a_claim_of_utc() {
        // UTC−4 (EDT).
        let s = rfc822("2026-07-28", "20:46:32", -4 * 3600);
        assert!(
            s.ends_with("-0400"),
            "the offset must be the station's, not +0000: {s}"
        );
        assert!(
            s.contains("20:46:32"),
            "the wall clock the station recorded must be preserved: {s}"
        );
        // UTC+5:30 — India Standard Time, a half-hour zone, which a naive
        // hours-only formatter renders as +0500.
        let s = rfc822("2026-07-28", "20:46:32", 5 * 3600 + 1800);
        assert!(s.ends_with("+0530"), "half-hour zones must survive: {s}");
        // A station genuinely on UTC still says +0000.
        assert!(rfc822("2026-07-28", "20:46:32", 0).ends_with("+0000"));
    }

    /// `DTSTART` is the *event*: the bird sang at 20:46 where the station
    /// stands. Emitting `20:46Z` claims that was 20:46 in Greenwich.
    ///
    /// The fix is a **floating** value (RFC 5545 §3.3.5 form 1) rather than an
    /// offset-corrected UTC one, because the station stores no offset with the
    /// row: applying today's offset to a detection from the other side of a
    /// daylight-saving boundary would be an hour wrong, and inventing a
    /// precision the data does not have is worse than reading the wall clock
    /// back exactly as it was written.
    #[test]
    fn ics_event_times_are_floating_local_not_a_claim_of_utc() {
        let dt = ics_local_datetime("2026-03-12", "06:14:32");
        assert_eq!(dt, "20260312T061432");
        assert!(
            !dt.ends_with('Z'),
            "a Z suffix asserts UTC, which this value is not"
        );
    }

    /// `DTSTAMP` is not the event time — RFC 5545 §3.8.7.2 says it is when the
    /// calendar object was created, and that it MUST be UTC. Reusing the
    /// detection's local time for it was both wrong properties at once.
    #[test]
    fn ics_dtstamp_is_a_real_utc_instant_not_the_detection_time() {
        let rows = [DetRow {
            com: "House Wren".to_owned(),
            sci: "Troglodytes aedon".to_owned(),
            date: "2026-07-28".to_owned(),
            time: "20:46:32".to_owned(),
            conf: 0.87,
        }];
        let ics = build_ics(
            &rows,
            "http://x",
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        let dtstamp = ics
            .lines()
            .find_map(|l| l.trim_end().strip_prefix("DTSTAMP:"))
            .expect("every VEVENT carries a DTSTAMP");
        assert!(
            dtstamp.ends_with('Z') && dtstamp.len() == 16,
            "DTSTAMP must be a UTC date-time: {dtstamp}"
        );
        assert_ne!(
            dtstamp, "20260728T204632Z",
            "DTSTAMP must not be the detection's local time relabelled as UTC"
        );
        assert!(
            ics.contains("DTSTART:20260728T204632\r\n"),
            "DTSTART must be the floating local wall clock: {ics}"
        );
    }

    /// RFC 5545 §3.1: content lines are folded at 75 octets. The description
    /// line carries a species name and a full URL and routinely runs to ~180.
    #[test]
    fn ics_content_lines_are_folded_to_the_spec_limit() {
        let rows = [DetRow {
            com: "Black-throated Blue Warbler".to_owned(),
            sci: "Setophaga caerulescens".to_owned(),
            date: "2026-07-28".to_owned(),
            time: "20:46:32".to_owned(),
            conf: 0.91,
        }];
        let ics = build_ics(
            &rows,
            "https://a-fairly-long-station-hostname.example.org",
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        for line in ics.split("\r\n") {
            assert!(
                line.len() <= 75,
                "unfolded {}-octet line: {line}",
                line.len()
            );
        }
        // Folding is only correct if it can be undone: every continuation line
        // must begin with a single space, which a parser strips.
        assert!(
            ics.contains("\r\n "),
            "a description this long must actually have been folded"
        );
    }

    #[test]
    fn rfc822_known_date() {
        // 2024-01-15 was a Monday.
        let s = rfc822("2024-01-15", "06:14:32", 0);
        assert!(s.starts_with("Mon, 15 Jan 2024 06:14:32"));
    }

    /// Replaces `ics_datetime_format`, which asserted
    /// `"20260312T061432Z"` — the defect itself, pinned as the contract. A
    /// detection's `Date`/`Time` is local wall clock; the `Z` claimed it was
    /// Greenwich. See `ics_event_times_are_floating_local_not_a_claim_of_utc`
    /// above for the replacement, and this for the shape of the value.
    #[test]
    fn ics_local_datetime_format() {
        assert_eq!(
            ics_local_datetime("2026-03-12", "06:14:32"),
            "20260312T061432"
        );
    }

    #[test]
    fn escape_ics_special_chars() {
        assert_eq!(escape_ics("a,b;c\nd\\e"), "a\\,b\\;c\\nd\\\\e");
    }

    /// A lone CR was passed through raw; CRLF and LF were already handled
    /// (CRLF as a stray CR plus an escaped LF). All three are now the same
    /// two-character escape, and nothing raw is left.
    #[test]
    fn escape_ics_leaves_no_raw_line_break_of_any_kind() {
        assert_eq!(escape_ics("a\rb"), "a\\nb");
        assert_eq!(escape_ics("a\r\nb"), "a\\nb");
        assert_eq!(escape_ics("a\nb"), "a\\nb");
        assert_eq!(escape_ics("a\r\rb\n\r"), "a\\n\\nb\\n\\n");
        for s in ["a\rb", "a\r\nb", "\r", "x\r\n\r\ny"] {
            let e = escape_ics(s);
            assert!(!e.contains('\r') && !e.contains('\n'), "{s:?} -> {e:?}");
        }
        // The backslash is still escaped first, so an escaped newline in the
        // input is not turned into a real one.
        assert_eq!(escape_ics("a\\nb"), "a\\\\nb");
    }

    // ── the ?base= override ─────────────────────────────────────────────

    fn base(candidate: &str, host: Option<&str>, configured: &[&str]) -> String {
        let configured: Vec<String> = configured.iter().map(|s| (*s).to_owned()).collect();
        resolve_base_with(Some(candidate.to_owned()), host, &configured)
    }

    /// The override is honoured for the host the request was addressed to, and
    /// for a host the operator configured — case-insensitively, whole-authority.
    #[test]
    fn a_base_naming_a_known_host_is_honoured() {
        assert_eq!(
            base("https://station.local/bn", Some("station.local"), &[]),
            "https://station.local/bn"
        );
        assert_eq!(
            base("http://Station.Local:8502", Some("station.local:8502"), &[]),
            "http://Station.Local:8502"
        );
        // A reverse proxy that does not forward Host: the operator named it.
        assert_eq!(
            base(
                "https://birds.example.com",
                Some("192.168.1.5:8502"),
                &["birds.example.com"]
            ),
            "https://birds.example.com"
        );
    }

    /// Everything else falls back to the server default.
    #[test]
    fn a_base_naming_any_other_host_falls_back() {
        let default = default_base_url();
        for (candidate, host) in [
            ("https://evil.example", Some("station.local")),
            ("https://evil.example/birdnet", Some("station.local")),
            // Same prefix, different host.
            ("https://station.local.evil.example", Some("station.local")),
            // A different port is a different authority.
            ("https://station.local:8443", Some("station.local")),
            // Userinfo smuggling is refused by the shape check.
            ("http://station.local@evil.example", Some("station.local")),
            // No Host and nothing configured: nothing to match.
            ("https://station.local", None),
        ] {
            assert_eq!(
                base(candidate, host, &[]),
                default,
                "{candidate} for {host:?}"
            );
        }
        // A configured host does not open the door to a different one.
        assert_eq!(
            base("https://evil.example", None, &["birds.example.com"]),
            default
        );
    }

    /// The shape rule still stands in front of the host rule: a known host with
    /// an XML metacharacter after it is not a base.
    #[test]
    fn a_known_host_does_not_excuse_a_malformed_base() {
        let default = default_base_url();
        for candidate in [
            "https://station.local\"onload=x",
            "https://station.local/<script>",
            "https://station.local/a&b",
            "https://station.local/\r\nX: y",
        ] {
            assert_eq!(
                base(candidate, Some("station.local"), &[]),
                default,
                "{candidate:?}"
            );
        }
    }

    #[test]
    fn build_rss_empty_is_valid() {
        let body = build_rss(
            &[],
            "http://x",
            "Test",
            "/feeds/test.rss",
            "test feed",
            &birdnet_core::i18n::NameLocalizer::english(),
        );
        assert!(body.contains("<rss version=\"2.0\""));
        assert!(body.contains("</channel></rss>"));
    }
}

// ── rare is configurable, and the two definitions really differ ─────────
#[cfg(test)]
mod rare_threshold_tests {
    use super::*;
    use rusqlite::Connection;

    /// Detections for one species on the given dates, plus a common bird that
    /// is heard constantly and must never read as rare under a gap rule.
    fn seed(conn: &Connection, returning: &[&str]) {
        conn.execute_batch(
            "CREATE TABLE detections_analytic (
                 Com_Name TEXT, Sci_Name TEXT, Date TEXT, Time TEXT, Confidence REAL);",
        )
        .unwrap();
        for d in returning {
            conn.execute(
                "INSERT INTO detections_analytic VALUES ('Wryneck','Jynx torquilla',?1,'06:00',0.95)",
                [d],
            )
            .unwrap();
        }
        // A resident, present every day across the same span.
        for day in 1..=28 {
            conn.execute(
                "INSERT INTO detections_analytic VALUES ('Robin','Erithacus rubecula',?1,'06:00',0.95)",
                [format!("2026-04-{day:02}")],
            )
            .unwrap();
        }
    }

    fn rare_dates(conn: &Connection, gap_days: u32) -> Vec<(String, String)> {
        let sql = rare_detections_sql(gap_days);
        let mut stmt = conn.prepare(&sql).expect("the SQL must be valid");
        let rows = stmt
            .query_map([500], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(2)?))
            })
            .unwrap();
        let mut v: Vec<(String, String)> = rows.flatten().collect();
        v.sort();
        v
    }

    /// `0` keeps the original behaviour: only the very first sighting ever.
    #[test]
    fn zero_means_first_ever_only() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn, &["2026-01-10", "2026-04-20", "2026-04-21"]);
        let rare = rare_dates(&conn, 0);
        assert_eq!(
            rare.iter().filter(|(n, _)| n == "Wryneck").count(),
            1,
            "exactly the first sighting: {rare:?}"
        );
        assert!(
            rare.iter().any(|(_, d)| d == "2026-01-10"),
            "and it is the earliest date: {rare:?}"
        );
    }

    /// The gap rule is the point: a bird absent for a hundred days is rare
    /// again when it returns. This is what `0` can never report, so the two
    /// answers are genuinely different rather than a relabelling.
    #[test]
    fn a_gap_makes_a_returning_bird_rare_again() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn, &["2026-01-10", "2026-04-20", "2026-04-21"]);

        let rare: Vec<String> = rare_dates(&conn, 30)
            .into_iter()
            .filter(|(n, _)| n == "Wryneck")
            .map(|(_, d)| d)
            .collect();
        assert_eq!(
            rare,
            vec!["2026-01-10".to_string(), "2026-04-20".to_string()],
            "the first sighting and the return, but not the day after the return"
        );
    }

    /// The counterpart that stops the gap rule being an "everything is rare"
    /// alarm: a bird heard every day is never rare, at any threshold.
    #[test]
    fn a_resident_is_never_rare() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn, &["2026-01-10"]);
        for gap in [1, 7, 30, 365] {
            let robins = rare_dates(&conn, gap)
                .into_iter()
                .filter(|(n, _)| n == "Robin")
                .count();
            assert_eq!(
                robins, 1,
                "at gap={gap} a daily resident may only appear for its first day"
            );
        }
    }

    /// The query as it was before the window was written into it: the
    /// definition the current form must agree with.
    fn reference_sql(gap_days: u32) -> String {
        if gap_days == 0 {
            return "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
                    FROM detections_analytic d \
                    WHERE d.Confidence > 0.85 \
                      AND (SELECT MIN(Date) FROM detections_analytic d2 \
                           WHERE d2.Com_Name = d.Com_Name) = d.Date \
                    ORDER BY d.Date DESC, d.Time DESC \
                    LIMIT ?1"
                .to_owned();
        }
        format!(
            "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
             FROM detections_analytic d \
             WHERE d.Confidence > 0.85 \
               AND NOT EXISTS ( \
                     SELECT 1 FROM detections_analytic p \
                     WHERE p.Com_Name = d.Com_Name \
                       AND p.Date < d.Date \
                       AND julianday(d.Date) - julianday(p.Date) <= {gap_days} \
                   ) \
             ORDER BY d.Date DESC, d.Time DESC \
             LIMIT ?1"
        )
    }

    /// Same rows, over data built to sit on the rule's edges: gaps of exactly
    /// the threshold, one day under and over, sightings on the same day, month
    /// and year ends, a leap day, species of every rhythm from daily to once.
    #[test]
    fn the_windowed_query_returns_what_the_unwindowed_one_did() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE detections_analytic (
                 Com_Name TEXT, Sci_Name TEXT, Date TEXT, Time TEXT, Confidence REAL);
             CREATE INDEX idx_species_date ON detections_analytic(Com_Name, Date);",
        )
        .unwrap();
        // Days since 2023-12-01, spanning a leap day and two year ends.
        let day = |n: u32| -> String {
            let (y, m, d) = birdnet_core::civil::civil_from_days(
                birdnet_core::civil::days_from_civil(2023, 12, 1) + i64::from(n),
            );
            format!("{y:04}-{m:02}-{d:02}")
        };
        let mut x: u64 = 0x5EED;
        let mut next = move |m: u64| {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (x >> 33) % m
        };
        for sp in 0..30_u32 {
            // Rhythm: sighted on a day with probability 1/period.
            let period = 1 + u64::from(sp % 10) * 4;
            for n in 0..800_u32 {
                if next(period) == 0 {
                    // Sometimes twice in a day, sometimes below the confidence cut.
                    for _ in 0..=next(2) {
                        let conf = 0.80 + (next(25) as f64) / 100.0;
                        conn.execute(
                            "INSERT INTO detections_analytic VALUES (?1, ?2, ?3, '06:00', ?4)",
                            rusqlite::params![
                                format!("Bird {sp:02}"),
                                format!("Genus {sp:02}"),
                                day(n),
                                conf
                            ],
                        )
                        .unwrap();
                    }
                }
            }
        }
        let rows = |sql: &str| -> Vec<(String, String, String, String, i64)> {
            let mut stmt = conn.prepare(sql).unwrap();
            let mut v: Vec<_> = stmt
                .query_map([100_000], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        (r.get::<_, f64>(4)? * 1000.0).round() as i64,
                    ))
                })
                .unwrap()
                .flatten()
                .collect();
            v.sort();
            v
        };
        let mut compared = 0;
        for gap in [0, 1, 2, 3, 7, 14, 30, 31, 90, 365, 366] {
            let (now, then) = (rows(&rare_detections_sql(gap)), rows(&reference_sql(gap)));
            assert_eq!(
                now, then,
                "gap={gap}: the windowed query disagrees with the reference"
            );
            compared += now.len();
        }
        assert!(
            compared > 500,
            "only {compared} rare rows compared; the data is too tame to test the edges"
        );
    }

    /// The point of the window: the probe for an earlier sighting is an index
    /// seek on the species and the days it covers, not a walk over every earlier
    /// row of the species. Measured on 300 000 rows: 3.25 s with only the old
    /// indexes, 17 s with the new index and the old predicate (the planner then
    /// walks the species from its first day), 0.94 s with both. The `gap = 0`
    /// form (`MIN(Date)` per row) took 300 s before and 0.96 s after.
    #[test]
    fn the_rare_query_seeks_by_species_and_date_window() {
        let conn = Connection::open_in_memory().unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let plan = |gap: u32| -> String {
            let mut stmt = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {}", rare_detections_sql(gap)))
                .unwrap();
            stmt.query_map([1], |r| r.get::<_, String>(3))
                .unwrap()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n")
        };
        let windowed = plan(30);
        assert!(
            windowed.contains("idx_detections_species_date (Com_Name=? AND Date>? AND Date<?)"),
            "the earlier-sighting probe is not a species-and-window seek:\n{windowed}"
        );
        let first = plan(0);
        assert!(
            first.contains("idx_detections_species_date"),
            "the first-ever probe does not use the species/date index:\n{first}"
        );
    }

    /// Both SQL forms must actually parse — a `format!`-built query that
    /// SQLite rejects would make the feed silently empty rather than error.
    #[test]
    fn both_query_forms_are_valid_sql() {
        let conn = Connection::open_in_memory().unwrap();
        seed(&conn, &["2026-01-10"]);
        for gap in [0, 1, 30, 3650] {
            conn.prepare(&rare_detections_sql(gap))
                .unwrap_or_else(|e| panic!("gap={gap} produced invalid SQL: {e}"));
        }
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    fn station() -> AppState {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        birdnet_db::migration::migrate(&conn).expect("migrate");
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
    }

    async fn status(app: &Router, path: &str) -> (StatusCode, Option<String>) {
        let res = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).expect("request"))
            .await
            .expect("response");
        let retry = res
            .headers()
            .get(header::RETRY_AFTER)
            .map(|v| v.to_str().unwrap_or("").to_owned());
        (res.status(), retry)
    }

    /// Past the slots a request is turned away at once, not queued behind the
    /// others: a queue is what held the threads. With the slots free again the
    /// same requests answer, so "turned away" cannot be a broken route.
    #[tokio::test]
    async fn a_request_past_the_slots_is_turned_away_not_queued() {
        let state = station();
        let app = router().with_state(state.clone());
        let paths = ["/feeds/rare.rss", "/feeds/rare.ics", "/feeds/today.rss"];

        // Take every slot there is.
        let mut held = Vec::new();
        while let Some(permit) = state.try_heavy_read_permit() {
            held.push(permit);
        }
        assert!(!held.is_empty(), "precondition: the gate has a slot");
        for path in paths {
            let (code, retry) = status(&app, path).await;
            assert_eq!(code, StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(retry.as_deref(), Some("15"), "{path}");
        }
        drop(held);

        for path in paths {
            assert_eq!(
                status(&app, path).await.0,
                StatusCode::OK,
                "{path} with the slots free"
            );
        }
    }

    /// The slot is released when the query ends, not leaked: more requests than
    /// slots, one after another, all answer.
    #[tokio::test]
    async fn slots_are_given_back() {
        let state = station();
        let app = router().with_state(state.clone());
        for _ in 0..4 {
            assert_eq!(status(&app, "/feeds/rare.rss").await.0, StatusCode::OK);
        }
        assert!(state.heavy_read_slots_free() >= 1);
    }
}
