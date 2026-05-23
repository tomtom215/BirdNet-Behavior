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
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use serde::Deserialize;

use crate::routes::pages::simple_url_encode;
use crate::state::AppState;

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
// Rare RSS — last N "first today" or quarantine-confirmed detections.
// ---------------------------------------------------------------------------

async fn rare_rss(State(state): State<AppState>, Query(q): Query<FeedQuery>) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let base = q.base.unwrap_or_else(default_base_url);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
                 FROM detections d \
                 WHERE d.Confidence > 0.85 \
                   AND (SELECT MIN(Date) FROM detections d2 WHERE d2.Com_Name = d.Com_Name) = d.Date \
                 ORDER BY d.Date DESC, d.Time DESC \
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

    let rows = result.ok().and_then(Result::ok).unwrap_or_default();
    let body = build_rss(
        &rows,
        &base,
        "Rare birds",
        "/feeds/rare.rss",
        "First-ever detections at this station (confidence ≥ 0.85).",
    );

    rss_response(body)
}

async fn today_rss(State(state): State<AppState>, Query(q): Query<FeedQuery>) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let base = q.base.unwrap_or_else(default_base_url);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT Com_Name, Sci_Name, Date, Time, Confidence \
                 FROM detections \
                 WHERE Date = date('now') \
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

    let rows = result.ok().and_then(Result::ok).unwrap_or_default();
    let body = build_rss(
        &rows,
        &base,
        "Today's detections",
        "/feeds/today.rss",
        "Every detection from this station today.",
    );
    rss_response(body)
}

fn rss_response(body: String) -> Response {
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/rss+xml; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    resp
}

// ---------------------------------------------------------------------------
// iCal feed — rare detections as point-in-time events.
// ---------------------------------------------------------------------------

async fn rare_ics(State(state): State<AppState>, Query(q): Query<FeedQuery>) -> Response {
    let limit = q.limit.unwrap_or(200).clamp(1, 1000);
    let base = q.base.unwrap_or_else(default_base_url);

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            let mut stmt = conn.prepare(
                "SELECT d.Com_Name, d.Sci_Name, d.Date, d.Time, d.Confidence \
                 FROM detections d \
                 WHERE d.Confidence > 0.85 \
                   AND (SELECT MIN(Date) FROM detections d2 WHERE d2.Com_Name = d.Com_Name) = d.Date \
                 ORDER BY d.Date DESC, d.Time DESC \
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

    let rows = result.ok().and_then(Result::ok).unwrap_or_default();
    let body = build_ics(&rows, &base);

    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/calendar; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
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

    for d in rows {
        let pub_date = rfc822(&d.date, &d.time);
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
            title = escape_xml(&d.com),
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

fn build_ics(rows: &[DetRow], base: &str) -> String {
    let mut s = String::with_capacity(4096);
    s.push_str(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//BirdNet-Behavior//RareBirds 1.0//EN\r\nCALSCALE:GREGORIAN\r\nMETHOD:PUBLISH\r\nX-WR-CALNAME:BirdNet rare detections\r\nX-WR-CALDESC:First-of-station detections from your listening station.\r\n",
    );
    for d in rows {
        let dt = ics_datetime(&d.date, &d.time);
        let uid = ics_uid(&d.date, &d.time, &d.sci);
        let url = detail_url(base, &d.date, &d.time, &d.com);
        let _ = write!(
            s,
            "BEGIN:VEVENT\r\nUID:bnb-{uid}@birdnet-behavior\r\nDTSTAMP:{dt}\r\nDTSTART:{dt}\r\nDURATION:PT3M\r\nSUMMARY:{name} (rare)\r\nDESCRIPTION:{name} — {sci} — confidence {conf}%.\\nListen: {url}\r\nURL:{url}\r\nCATEGORIES:rare-bird\r\nEND:VEVENT\r\n",
            uid = uid,
            dt = dt,
            url = url,
            name = escape_ics(&d.com),
            sci = escape_ics(&d.sci),
            conf = (d.conf * 100.0).round() as i32,
        );
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
    std::env::var("BNB_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string())
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn escape_ics(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// Convert "2026-03-12" + "06:14:32" to RFC 822 — Mon, 12 Mar 2026 06:14:32 +0000.
/// Treats input as UTC; fine for the use case (per-device feed, single TZ).
fn rfc822(date: &str, time: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return format!("{date} {time}");
    }
    let y: i32 = parts[0].parse().unwrap_or(1970);
    let m: u32 = parts[1].parse().unwrap_or(1);
    let d: u32 = parts[2].parse().unwrap_or(1);
    let dow = day_of_week(y, m, d);
    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];
    let dow_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow as usize];
    let t = time.replace('Z', "");
    format!("{dow_name}, {d:02} {month_name} {y} {t} +0000")
}

fn ics_datetime(date: &str, time: &str) -> String {
    // 20260312T061432Z
    let d = date.replace('-', "");
    let t = time.replace([':', 'Z'], "");
    format!("{d}T{t}Z")
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

    #[test]
    fn rfc822_known_date() {
        // 2024-01-15 was a Monday.
        let s = rfc822("2024-01-15", "06:14:32");
        assert!(s.starts_with("Mon, 15 Jan 2024 06:14:32"));
    }

    #[test]
    fn ics_datetime_format() {
        assert_eq!(ics_datetime("2026-03-12", "06:14:32"), "20260312T061432Z");
    }

    #[test]
    fn escape_ics_special_chars() {
        assert_eq!(escape_ics("a,b;c\nd\\e"), "a\\,b\\;c\\nd\\\\e");
    }

    #[test]
    fn build_rss_empty_is_valid() {
        let body = build_rss(&[], "http://x", "Test", "/feeds/test.rss", "test feed");
        assert!(body.contains("<rss version=\"2.0\""));
        assert!(body.contains("</channel></rss>"));
    }
}
