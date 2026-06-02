//! Public, signed share links for individual detections.
//!
//! Routes:
//!   GET /r/{token}                  full HTML share page
//!   GET /r/{token}/audio.wav        302 -> `/api/v2/recordings/<filename>`
//!   GET /r/{token}/spectrogram.png  302 -> `/api/v2/spectrogram/<filename>`
//!
//! A detection has no integer id in this schema — it is identified by the
//! `(Date, Time, Com_Name)` triple (the `UNIQUE(Date, Time, Sci_Name)` index
//! from migration 5; `Com_Name` is 1:1 with `Sci_Name` in practice and is what
//! the rest of the UI keys on). The token is
//! `base64url(payload || HMAC-SHA256(secret, payload)[..16])` where
//! `payload = date \x1f time \x1f com_name \x1f expiry`. The MAC is the trailing
//! 16 bytes, so the payload may contain any character except the field
//! separator. A tampered or expired token fails verification and renders the
//! "gone" page rather than leaking anything.
//!
//! Set `BNB_SHARE_SECRET` (32+ random bytes) in the environment so links
//! survive restarts; without it a random per-process secret is used
//! (fail-secure: every outstanding link invalidates on restart).

// Crypto + HTTP rendering: short identifiers and doc acronyms (HMAC-SHA256,
// base64url) are intrinsic; allow the pedantic/nursery style noise.
#![allow(clippy::pedantic, clippy::nursery)]

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Router, routing::get};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::routes::pages::{escape_html, simple_url_encode};
use crate::state::AppState;

const TEMPLATE: &str = include_str!("../../templates/share_rare.html");
const DEFAULT_TTL_SECS: u64 = 30 * 86_400; // 30 days
const TRUNCATED_HMAC_LEN: usize = 16; // 128 bits — ample against forgery
const FIELD_SEP: char = '\u{1f}'; // ASCII unit separator — never appears in our fields

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/r/{token}", get(share_page))
        .route("/r/{token}/audio.wav", get(share_audio_redirect))
        .route(
            "/r/{token}/spectrogram.png",
            get(share_spectrogram_redirect),
        )
}

// ───────────────────────────────────────────────────────────────────────────
// Token: base64url(payload || HMAC-SHA256(secret, payload)[..16])
// ───────────────────────────────────────────────────────────────────────────

fn secret() -> &'static [u8] {
    static SECRET: OnceLock<Vec<u8>> = OnceLock::new();
    SECRET
        .get_or_init(|| {
            std::env::var("BNB_SHARE_SECRET")
                .map(String::into_bytes)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "BNB_SHARE_SECRET not set; using a random per-process secret. \
                         Outstanding share links invalidate on restart."
                    );
                    random_secret().to_vec()
                })
        })
        .as_slice()
}

/// 32 bytes of best-effort per-process entropy (std-only). Only used as the
/// fail-secure fallback when `BNB_SHARE_SECRET` is unset — production should
/// always set the env var so links survive restarts.
#[allow(clippy::cast_possible_truncation)]
fn random_secret() -> [u8; 32] {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0xDEAD_BEEF_u64, |d| {
            u64::from(d.subsec_nanos()) ^ d.as_secs().rotate_left(21)
        })
        ^ u64::from(std::process::id());
    let mut x = seed;
    let mut buf = [0u8; 32];
    for b in &mut buf {
        // SplitMix64-style scramble; good enough for a session-only secret.
        x = x
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *b = (x >> 56) as u8;
    }
    buf
}

fn truncated_mac(payload: &[u8]) -> [u8; TRUNCATED_HMAC_LEN] {
    let mut mac = HmacSha256::new_from_slice(secret()).expect("HMAC accepts any key length");
    mac.update(payload);
    let tag = mac.finalize().into_bytes();
    let mut out = [0u8; TRUNCATED_HMAC_LEN];
    out.copy_from_slice(&tag[..TRUNCATED_HMAC_LEN]);
    out
}

/// Encode `(date, time, com_name, expiry)` into a URL-safe token.
#[must_use]
pub fn encode_share_token(date: &str, time: &str, com: &str, expiry_epoch: u64) -> String {
    let payload = format!("{date}{FIELD_SEP}{time}{FIELD_SEP}{com}{FIELD_SEP}{expiry_epoch}");
    let tag = truncated_mac(payload.as_bytes());
    let mut bytes = payload.into_bytes();
    bytes.extend_from_slice(&tag);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Verify a token and return `(date, time, com_name)` if it is well-formed,
/// authentic, and unexpired. Uses a constant-time tag comparison.
fn decode_share_token(token: &str) -> Option<(String, String, String)> {
    let raw = URL_SAFE_NO_PAD.decode(token.as_bytes()).ok()?;
    if raw.len() <= TRUNCATED_HMAC_LEN {
        return None;
    }
    let (payload, provided) = raw.split_at(raw.len() - TRUNCATED_HMAC_LEN);

    // Constant-time verification of the truncated tag.
    let mut mac = HmacSha256::new_from_slice(secret()).ok()?;
    mac.update(payload);
    mac.verify_truncated_left(provided).ok()?;

    let payload_str = std::str::from_utf8(payload).ok()?;
    let mut parts = payload_str.split(FIELD_SEP);
    let date = parts.next()?.to_string();
    let time = parts.next()?.to_string();
    let com = parts.next()?.to_string();
    let exp: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // payload must be exactly four fields
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if exp < now {
        return None;
    }
    Some((date, time, com))
}

/// Issue a fresh 30-day token for the detection identified by
/// `(date, time, com_name)`. Called by the detection-detail "Share" button.
#[must_use]
pub fn issue_token_for(date: &str, time: &str, com: &str) -> String {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .saturating_add(DEFAULT_TTL_SECS);
    encode_share_token(date, time, com, exp)
}

// ───────────────────────────────────────────────────────────────────────────
// Handlers
// ───────────────────────────────────────────────────────────────────────────

struct ShareDetection {
    com_name: String,
    sci_name: String,
    date: String,
    time: String,
    confidence: f64,
}

fn lookup(
    conn: &rusqlite::Connection,
    date: &str,
    time: &str,
    com: &str,
) -> Option<ShareDetection> {
    let query = |sql: &str| {
        conn.query_row(sql, rusqlite::params![date, time, com], |row| {
            Ok(ShareDetection {
                com_name: row.get(0)?,
                sci_name: row.get(1)?,
                date: row.get(2)?,
                time: row.get(3)?,
                confidence: row.get(4)?,
            })
        })
        .ok()
    };
    query(
        "SELECT Com_Name, Sci_Name, Date, Time, Confidence \
         FROM detections WHERE Date = ?1 AND Time = ?2 AND Com_Name = ?3 LIMIT 1",
    )
    // O-07: rare birds shared from the quarantine queue are not in `detections`
    // until approved, so fall back to the quarantine table.
    .or_else(|| {
        query(
            "SELECT com_name, sci_name, date, time, confidence \
             FROM quarantine WHERE date = ?1 AND time = ?2 AND com_name = ?3 LIMIT 1",
        )
    })
}

async fn share_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let Some((date, time, com)) = decode_share_token(&token) else {
        return gone_page();
    };

    let station_label = state.site_name().to_string();
    let db = state.clone();
    let result =
        tokio::task::spawn_blocking(move || db.with_db(|conn| lookup(conn, &date, &time, &com)))
            .await;

    let Ok(Some(det)) = result else {
        return gone_page();
    };

    let conf_pct = format!("{:.0}%", det.confidence * 100.0);
    let conf_class_pill = if det.confidence >= 0.85 {
        "moss"
    } else if det.confidence >= 0.60 {
        "dawn"
    } else {
        "rare"
    };
    let station_handle = station_label
        .to_lowercase()
        .replace(' ', "-")
        .replace([',', '.'], "");

    let body = TEMPLATE
        // O-18: toast live region for the share page's "Copy permalink" UX.
        .replace(
            "{{toast_region}}",
            crate::routes::pages::TOAST_REGION_HTML,
        )
        .replace("{{species_name}}", &escape_html(&det.com_name))
        .replace("{{scientific_name}}", &escape_html(&det.sci_name))
        .replace("{{species_encoded}}", &simple_url_encode(&det.com_name))
        .replace("{{date}}", &escape_html(&det.date))
        .replace("{{time}}", &escape_html(&det.time))
        .replace(
            "{{ago_phrase}}",
            &escape_html(&ago_phrase(&det.date, &det.time)),
        )
        .replace("{{confidence_pct}}", &conf_pct)
        .replace("{{conf_class_pill}}", conf_class_pill)
        .replace("{{audio_url}}", &format!("/r/{token}/audio.wav"))
        .replace(
            "{{spectrogram_url}}",
            &format!("/r/{token}/spectrogram.png"),
        )
        .replace("{{station_label}}", &escape_html(&station_label))
        .replace(
            "{{station_handle}}",
            &escape_html(&format!("@station/{station_handle}")),
        );

    html_ok(body)
}

/// 302 to the existing recordings route. The token vouches for access; the
/// actual byte streaming is the existing handler. Uses the looked-up filename
/// (there is no integer id to key the media route on).
async fn share_audio_redirect(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let Some((date, time, com)) = decode_share_token(&token) else {
        return gone_page();
    };
    match lookup_basename(state, &date, &time, &com).await {
        Some(name) => {
            Redirect::temporary(&format!("/api/v2/recordings/{}", simple_url_encode(&name)))
                .into_response()
        }
        None => gone_page(),
    }
}

/// 302 to the existing spectrogram route, which is keyed by **filename** (not
/// an id) — this is the fix for the original bundle, which redirected to
/// `/api/v2/spectrogram/<id>` and always 404'd.
async fn share_spectrogram_redirect(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let Some((date, time, com)) = decode_share_token(&token) else {
        return gone_page();
    };
    match lookup_basename(state, &date, &time, &com).await {
        Some(name) => {
            Redirect::temporary(&format!("/api/v2/spectrogram/{}", simple_url_encode(&name)))
                .into_response()
        }
        None => gone_page(),
    }
}

/// Resolve the recording's bare filename for a detection identity.
async fn lookup_basename(state: AppState, date: &str, time: &str, com: &str) -> Option<String> {
    let (d, t, c) = (date.to_string(), time.to_string(), com.to_string());
    tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            conn.query_row(
                "SELECT File_Name FROM detections \
                 WHERE Date = ?1 AND Time = ?2 AND Com_Name = ?3 LIMIT 1",
                rusqlite::params![d, t, c],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            // O-07: fall back to the quarantine row's recording.
            .or_else(|| {
                conn.query_row(
                    "SELECT file_name FROM quarantine \
                     WHERE date = ?1 AND time = ?2 AND com_name = ?3 LIMIT 1",
                    rusqlite::params![d, t, c],
                    |row| row.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
            })
        })
    })
    .await
    .ok()
    .flatten()
    .filter(|f| !f.is_empty())
    .map(|f| {
        std::path::Path::new(&f)
            .file_name()
            .map_or(f.clone(), |n| n.to_string_lossy().into_owned())
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Helpers
// ───────────────────────────────────────────────────────────────────────────

fn html_ok(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            ),
        ],
        body,
    )
        .into_response()
}

fn gone_page() -> Response {
    let html = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Not found</title><meta name="robots" content="noindex"><link rel="stylesheet" href="/static/css/app.css"></head><body class="sh-gone"><h1 class="display sh-gone-title">This clip is gone.</h1><p class="bnb-meta sh-gone-text">The link expired or never existed. The station owner can share a fresh one.</p></body></html>"#;
    (
        StatusCode::NOT_FOUND,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        html.to_string(),
    )
        .into_response()
}

/// Best-effort relative-time phrase ("13 minutes ago"). Treats the stored
/// civil time as UTC, which is what the rest of the app assumes for a
/// single-station feed.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn ago_phrase(date: &str, time: &str) -> String {
    let parse = || -> Option<u64> {
        let mut dp = date.split('-');
        let y = dp.next()?.parse::<i64>().ok()?;
        let m = dp.next()?.parse::<i64>().ok()?;
        let d = dp.next()?.parse::<i64>().ok()?;
        let mut tp = time.split(':');
        let hh = tp.next()?.parse::<i64>().ok()?;
        let mm = tp.next()?.parse::<i64>().ok()?;
        let ss = tp.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        // Civil-from-fields (Howard Hinnant) -> days since 1970-01-01.
        let yy = if m <= 2 { y - 1 } else { y };
        let era = if yy >= 0 { yy } else { yy - 399 } / 400;
        let yoe = (yy - era * 400) as u64;
        let mp = if m > 2 {
            (m - 3) as u64
        } else {
            (m + 9) as u64
        };
        let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe as i64 - 719_468;
        if days < 0 {
            return None;
        }
        Some((days as u64) * 86_400 + (hh as u64) * 3600 + (mm as u64) * 60 + ss as u64)
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let then = parse().unwrap_or(now);
    let elapsed = now.saturating_sub(then);
    match elapsed {
        0..=59 => "just now".to_string(),
        s if s < 3600 => format!("{} minutes ago", s / 60),
        s if s < 86_400 => format!("{} hours ago", s / 3600),
        s if s < 30 * 86_400 => format!("{} days ago", s / 86_400),
        _ => format!("recorded {date} · {time}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip() {
        let token = encode_share_token("2026-05-19", "06:14:32", "Eurasian Magpie", 9_999_999_999);
        let (d, t, c) = decode_share_token(&token).expect("valid token");
        assert_eq!(d, "2026-05-19");
        assert_eq!(t, "06:14:32");
        assert_eq!(c, "Eurasian Magpie");
    }

    #[test]
    fn roundtrips_names_with_punctuation() {
        // Apostrophes, spaces, and periods are common in common names; the
        // \x1f separator keeps them unambiguous.
        let token = encode_share_token("2026-05-19", "06:14:32", "Bell's Vireo", 9_999_999_999);
        let (_, _, c) = decode_share_token(&token).expect("valid");
        assert_eq!(c, "Bell's Vireo");
    }

    #[test]
    fn expired_token_rejected() {
        let token = encode_share_token("2026-05-19", "06:14:32", "American Robin", 1);
        assert!(decode_share_token(&token).is_none());
    }

    #[test]
    fn tampered_token_rejected() {
        let token = encode_share_token("2026-05-19", "06:14:32", "American Robin", 9_999_999_999);
        let mut bytes = token.into_bytes();
        let mid = bytes.len() / 2;
        // Swap to a different but still-valid base64url character.
        bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(decode_share_token(&tampered).is_none());
    }

    #[test]
    fn malformed_token_rejected() {
        assert!(decode_share_token("").is_none());
        assert!(decode_share_token("not-a-real-token").is_none());
    }

    #[test]
    fn issue_token_is_valid_and_unexpired() {
        let token = issue_token_for("2026-05-19", "06:14:32", "American Robin");
        let (d, t, c) = decode_share_token(&token).expect("valid");
        assert_eq!(
            (d.as_str(), t.as_str(), c.as_str()),
            ("2026-05-19", "06:14:32", "American Robin")
        );
    }
}
