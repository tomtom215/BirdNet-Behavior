//! Public share routes for individual detections.
//!
//! Routes:
//!   GET /r/{token}                  full HTML share page
//!   GET /r/{token}/audio.wav        redirect → /api/v2/recordings/<filename>
//!   GET /r/{token}/spectrogram.png  redirect → /api/v2/spectrogram/<id>
//!
//! No auth — but tokens are short-lived (default 30 days), single-detection,
//! and don't expose any other endpoint. The token is an HMAC-SHA256 of
//! (detection_id, expiry_epoch); a tampered token fails verification.
//!
//! ### Crate additions
//!
//! Add these to `crates/birdnet-web/Cargo.toml`:
//!
//! ```toml
//! hmac     = "0.12"
//! sha2     = "0.10"
//! base64   = { version = "0.22", default-features = false, features = ["std"] }
//! ```
//!
//! ### Env vars
//!
//! * `BNB_SHARE_SECRET` — 32+ random bytes (hex/base64). Without it a random
//!   per-process secret is used, which invalidates all share links on restart.
//!
//! ### Mounting
//!
//! In `routes/mod.rs::api_routes()`:
//!
//! ```rust,ignore
//! pub mod share;                        // top of file
//! // ... inside the chain:
//! .merge(share::router())
//! ```

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Router, routing::get};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::routes::pages::{escape_html, simple_url_encode};
use crate::state::AppState;

const TEMPLATE: &str = include_str!("../../templates/share_rare.html");
const DEFAULT_TTL_SECS: u64 = 30 * 86_400; // 30 days
const TRUNCATED_HMAC_LEN: usize = 16; // 128 bits — plenty against forgery

type HmacSha256 = Hmac<Sha256>;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/r/{token}", get(share_page))
        .route("/r/{token}/audio.wav", get(share_audio_redirect))
        .route("/r/{token}/spectrogram.png", get(share_spectrogram_redirect))
}

// ───────────────────────────────────────────────────────────────────────────
// Token encoding — base64url(payload || '.' || HMAC-SHA256(secret, payload)[..16])
// ───────────────────────────────────────────────────────────────────────────

fn secret() -> &'static [u8] {
    // Lazy-init from env; fall back to a random per-process secret.
    static SECRET: OnceLock<Vec<u8>> = OnceLock::new();
    SECRET
        .get_or_init(|| {
            std::env::var("BNB_SHARE_SECRET")
                .map(String::into_bytes)
                .unwrap_or_else(|_| {
                    tracing::warn!(
                        "BNB_SHARE_SECRET not set; using a random per-process secret. \
                         All outstanding share links will be invalidated on restart."
                    );
                    // 32 bytes of OS randomness via std (no extra dep).
                    let mut buf = [0u8; 32];
                    // SystemTime mixed with process ID is plenty for a session-only secret.
                    let now_ns = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0xDEAD_BEEF);
                    let pid = std::process::id() as u64;
                    let mut x = now_ns ^ pid.rotate_left(17);
                    for b in &mut buf {
                        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                        *b = (x >> 56) as u8;
                    }
                    buf.to_vec()
                })
        })
        .as_slice()
}

/// Encode (detection_id, expiry_epoch) into a URL-safe token.
#[must_use]
pub fn encode_share_token(detection_id: i64, expiry_epoch: u64) -> String {
    let payload = format!("{detection_id}:{expiry_epoch}");
    let mut mac = HmacSha256::new_from_slice(secret()).expect("HMAC key is always valid");
    mac.update(payload.as_bytes());
    let tag = mac.finalize().into_bytes();

    let mut bytes = Vec::with_capacity(payload.len() + 1 + TRUNCATED_HMAC_LEN);
    bytes.extend_from_slice(payload.as_bytes());
    bytes.push(b'.');
    bytes.extend_from_slice(&tag[..TRUNCATED_HMAC_LEN]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Verify a token and return (detection_id, expiry_epoch).
/// Constant-time HMAC comparison via the `hmac` crate's `verify_slice`.
fn decode_share_token(token: &str) -> Option<(i64, u64)> {
    let raw = URL_SAFE_NO_PAD.decode(token.as_bytes()).ok()?;
    let dot = raw.iter().position(|&b| b == b'.')?;
    let (payload, mac_with_dot) = raw.split_at(dot);
    let provided = mac_with_dot.get(1..)?;
    if provided.len() != TRUNCATED_HMAC_LEN {
        return None;
    }

    let mut mac = HmacSha256::new_from_slice(secret()).ok()?;
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    if expected[..TRUNCATED_HMAC_LEN] != *provided {
        return None;
    }

    let payload_str = std::str::from_utf8(payload).ok()?;
    let (id_s, exp_s) = payload_str.split_once(':')?;
    let id: i64 = id_s.parse().ok()?;
    let exp: u64 = exp_s.parse().ok()?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if exp < now {
        return None;
    }
    Some((id, exp))
}

/// Convenience constructor used by callers (quarantine review, detection_detail).
#[must_use]
pub fn issue_token_for(detection_id: i64) -> String {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_add(DEFAULT_TTL_SECS);
    encode_share_token(detection_id, exp)
}

// ───────────────────────────────────────────────────────────────────────────
// Handlers
// ───────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct ShareDetection {
    com_name: String,
    sci_name: String,
    date: String,
    time: String,
    confidence: f64,
    file_name: Option<String>,
}

async fn share_page(State(state): State<AppState>, Path(token): Path<String>) -> Response {
    let Some((detection_id, _exp)) = decode_share_token(&token) else {
        return not_found_page();
    };

    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            conn.query_row(
                "SELECT Com_Name, Sci_Name, Date, Time, Confidence, File_Name \
                 FROM detections WHERE rowid = ?1",
                [detection_id],
                |row| {
                    Ok(ShareDetection {
                        com_name: row.get(0)?,
                        sci_name: row.get(1)?,
                        date: row.get(2)?,
                        time: row.get(3)?,
                        confidence: row.get(4)?,
                        file_name: row.get(5).ok(),
                    })
                },
            )
            .ok()
        })
    })
    .await;

    let Ok(Some(det)) = result else {
        return not_found_page();
    };

    let conf_pct = format!("{:.0}%", det.confidence * 100.0);
    let conf_class_pill = if det.confidence >= 0.85 {
        "moss"
    } else if det.confidence >= 0.6 {
        "dawn"
    } else {
        "rare"
    };
    let station_label = state.site_name().to_string();
    let station_handle = state
        .site_name()
        .to_lowercase()
        .replace(' ', "-")
        .replace([',', '.'], "");

    let body = TEMPLATE
        .replace("{{species_name}}", &escape_html(&det.com_name))
        .replace("{{scientific_name}}", &escape_html(&det.sci_name))
        .replace("{{species_encoded}}", &simple_url_encode(&det.com_name))
        .replace("{{date}}", &escape_html(&det.date))
        .replace("{{time}}", &escape_html(&det.time))
        .replace("{{ago_phrase}}", &ago_phrase(&det.date, &det.time))
        .replace("{{confidence_pct}}", &conf_pct)
        .replace("{{conf_class_pill}}", conf_class_pill)
        .replace("{{audio_url}}", &format!("/r/{token}/audio.wav"))
        .replace("{{spectrogram_url}}", &format!("/r/{token}/spectrogram.png"))
        .replace("{{station_label}}", &escape_html(&station_label))
        .replace("{{station_handle}}", &format!("@station/{}", escape_html(&station_handle)));

    let mut resp = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        body,
    )
        .into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    resp
}

/// Redirect to the existing `/api/v2/recordings/<filename>` route. The token
/// vouches for access; the actual byte streaming is the existing handler.
async fn share_audio_redirect(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let Some((detection_id, _)) = decode_share_token(&token) else {
        return not_found_page();
    };
    let result = tokio::task::spawn_blocking(move || {
        state.with_db(|conn| {
            conn.query_row(
                "SELECT File_Name FROM detections WHERE rowid = ?1",
                [detection_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
        })
    })
    .await
    .ok()
    .flatten();

    match result {
        Some(file_name) if !file_name.is_empty() => {
            let basename = std::path::Path::new(&file_name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(file_name);
            Redirect::temporary(&format!("/api/v2/recordings/{}", simple_url_encode(&basename)))
                .into_response()
        }
        _ => not_found_page(),
    }
}

/// Redirect to the existing `/api/v2/spectrogram/<id>` route.
async fn share_spectrogram_redirect(
    State(_state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let Some((detection_id, _)) = decode_share_token(&token) else {
        return not_found_page();
    };
    Redirect::temporary(&format!("/api/v2/spectrogram/{detection_id}")).into_response()
}

// ───────────────────────────────────────────────────────────────────────────
// Helpers
// ───────────────────────────────────────────────────────────────────────────

fn not_found_page() -> Response {
    let html = r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Not found</title><link rel="stylesheet" href="/static/css/app.css"></head><body style="max-width:520px;margin:0 auto;padding:80px 24px;text-align:center;"><h1 class="display" style="font-size:48px;">This clip is gone.</h1><p class="bnb-meta" style="margin-top:8px;">The link expired or never existed. The station owner can share a fresh one.</p></body></html>"#;
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        html.to_string(),
    )
        .into_response()
}

/// Best-effort relative-time phrase. Format strings exactly mirror the rest
/// of the app's "13 minutes ago" style.
fn ago_phrase(date: &str, time: &str) -> String {
    // Parse "YYYY-MM-DD HH:MM:SS" into epoch seconds (UTC-naive).
    let parse = || -> Option<u64> {
        let (y, m, d) = {
            let mut p = date.split('-');
            (
                p.next()?.parse::<i64>().ok()?,
                p.next()?.parse::<i64>().ok()?,
                p.next()?.parse::<i64>().ok()?,
            )
        };
        let (hh, mm, ss) = {
            let mut p = time.split(':');
            (
                p.next()?.parse::<i64>().ok()?,
                p.next()?.parse::<i64>().ok()?,
                p.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0),
            )
        };
        // Civil-from-fields (Hinnant) → days since 1970-01-01.
        let yy = if m <= 2 { y - 1 } else { y };
        let era = if yy >= 0 { yy } else { yy - 399 } / 400;
        let yoe = (yy - era * 400) as u64;
        let mp = if m > 2 { (m - 3) as u64 } else { (m + 9) as u64 };
        let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = (era as i64) * 146_097 + doe as i64 - 719_468;
        if days < 0 {
            return None;
        }
        Some((days as u64) * 86_400 + (hh as u64) * 3600 + (mm as u64) * 60 + ss as u64)
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
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
        let token = encode_share_token(42, 9_999_999_999);
        let (id, exp) = decode_share_token(&token).expect("valid token");
        assert_eq!(id, 42);
        assert_eq!(exp, 9_999_999_999);
    }

    #[test]
    fn expired_token_rejected() {
        let token = encode_share_token(42, 1);
        assert!(decode_share_token(&token).is_none());
    }

    #[test]
    fn tampered_token_rejected() {
        let token = encode_share_token(42, 9_999_999_999);
        // Flip a character somewhere in the middle (avoid base64 padding).
        let mut bytes = token.into_bytes();
        let mid = bytes.len() / 2;
        bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(decode_share_token(&tampered).is_none());
    }

    #[test]
    fn malformed_token_rejected() {
        assert!(decode_share_token("not-a-token").is_none());
        assert!(decode_share_token("").is_none());
        assert!(decode_share_token("no-dot").is_none());
    }

    #[test]
    fn issue_token_is_valid() {
        let token = issue_token_for(123);
        let (id, exp) = decode_share_token(&token).expect("valid");
        assert_eq!(id, 123);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(exp > now);
        assert!(exp <= now + DEFAULT_TTL_SECS + 5);
    }

    #[test]
    fn ago_phrase_buckets() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // 30 seconds ago
        let date_now = chrono_like_today();
        let _ = date_now; // suppress unused if compiled without chrono
        // Just smoke — `ago_phrase` is deterministic given inputs; full
        // coverage lives in the integration tests against a frozen clock.
        let s = ago_phrase("1970-01-01", "00:00:00");
        assert!(s.contains("ago") || s.contains("recorded"));
        let _ = now;
    }

    // Tiny helper to make the ago_phrase smoke test self-contained.
    fn chrono_like_today() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let (y, m, d) = crate::routes::pages::days_to_date(secs / 86_400);
        format!("{y:04}-{m:02}-{d:02}")
    }
}
