//! `/api/v2/streams/hls/*` — live audio as HLS (`G-21`) — against the real
//! router, a real `ffmpeg`, and a capture tap fed the way capture feeds it.
//!
//! What these hold: a start answers with a playlist that a player can fetch,
//! the playlist names segments that are MPEG-TS, a second listener joins the
//! running encoder rather than starting another, the token is the only way in,
//! and the last listener leaving ends the stream and deletes its segments.
//!
//! `ffmpeg` is needed to encode. Without it a start must be the specific
//! refusal `/stream` gives, and the test says that is all it checked; CI sets
//! `BNB_REQUIRE_FFMPEG`, which turns its absence into a failure.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::Connection;
use tower::ServiceExt;

use birdnet_core::audio::capture::{PcmSpec, new_live_audio_hub};
use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const MONO_48K: PcmSpec = PcmSpec {
    sample_rate: 48_000,
    channels: 1,
};

fn ffmpeg_available() -> bool {
    let found = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|d| d.join("ffmpeg").is_file()));
    assert!(
        found || std::env::var_os("BNB_REQUIRE_FFMPEG").is_none(),
        "BNB_REQUIRE_FFMPEG is set and ffmpeg is not on PATH"
    );
    found
}

/// A station with one microphone source `id`, whose tap a thread feeds with
/// a 440 Hz tone at twice real time until the returned flag is cleared.
fn recording_station(id: &str) -> (AppState, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let hub = new_live_audio_hub();
    let tap = hub.tap(id, MONO_48K);
    let producing = Arc::new(AtomicBool::new(true));
    let running = Arc::clone(&producing);
    let feeder = std::thread::spawn(move || {
        let mut n: u64 = 0;
        while running.load(Ordering::Relaxed) {
            // 20 ms of 48 kHz mono S16LE.
            let mut chunk = Vec::with_capacity(1920);
            for _ in 0..960 {
                #[allow(clippy::cast_precision_loss)]
                let t = n as f64 / 48_000.0;
                #[allow(clippy::cast_possible_truncation)]
                let sample = ((t * 440.0 * std::f64::consts::TAU).sin() * 8_000.0) as i16;
                chunk.extend_from_slice(&sample.to_le_bytes());
                n += 1;
            }
            tap.push(&chunk);
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state =
        AppState::from_connection(conn, std::path::PathBuf::from(":memory:")).with_live_audio(hub);
    let new = NewAudioSource::defaults(
        id.to_string(),
        SourceKind::UsbAlsa,
        "plughw:CARD=PRO,DEV=0".to_string(),
    );
    state
        .with_db(|conn| AudioSourceStore::insert(conn, &new))
        .expect("insert audio source");
    (state, producing, feeder)
}

struct Answer {
    status: StatusCode,
    content_type: String,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.text()))
    }
}

async fn call(state: &AppState, method: &str, uri: &str, body: &str) -> Answer {
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    Answer {
        status,
        content_type,
        body,
    }
}

#[tokio::test]
async fn a_listener_is_served_a_playlist_of_mpeg_ts_segments() {
    let (state, producing, feeder) = recording_station("src_hls_play");

    // What a station that was killed mid-stream leaves behind: a stream
    // directory nothing will write again. Starting a stream sweeps it.
    let abandoned = std::env::temp_dir()
        .join("birdnet-hls")
        .join("0123456789abcdef0123456789abcdef");
    std::fs::create_dir_all(&abandoned).expect("abandoned dir");
    std::fs::write(abandoned.join("seg00001.ts"), b"x").expect("segment");
    std::fs::File::open(&abandoned)
        .expect("open dir")
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
        .expect("set mtime");

    let start = call(
        &state,
        "POST",
        "/api/v2/streams/hls/src_hls_play/start",
        r#"{"session_id":"tab-a"}"#,
    )
    .await;
    if !ffmpeg_available() {
        producing.store(false, Ordering::Relaxed);
        feeder.join().expect("feeder");
        assert_eq!(start.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(start.text().contains("ffmpeg"), "{}", start.text());
        eprintln!("SKIPPED the HLS stream itself: ffmpeg is not on PATH");
        return;
    }
    assert_eq!(start.status, StatusCode::OK, "{}", start.text());
    assert!(
        !abandoned.exists(),
        "an abandoned stream's segments were kept"
    );
    let started = start.json();
    assert_eq!(started["status"], "ready", "{started}");
    assert_eq!(started["playlist_ready"], true);
    assert_eq!(started["source"], "src_hls_play");
    assert_eq!(started["active_clients"], 1);
    let token = started["stream_token"].as_str().expect("token").to_owned();
    let playlist_url = started["playlist_url"].as_str().expect("url").to_owned();
    assert_eq!(
        playlist_url,
        format!("/api/v2/streams/hls/t/{token}/playlist.m3u8")
    );

    // The playlist a player fetches.
    let playlist = call(&state, "GET", &playlist_url, "").await;
    assert_eq!(playlist.status, StatusCode::OK);
    assert_eq!(playlist.content_type, "application/vnd.apple.mpegurl");
    let text = playlist.text();
    assert!(text.starts_with("#EXTM3U"), "{text}");
    assert!(
        !text.contains("#EXT-X-ENDLIST"),
        "a live playlist has no end: {text}"
    );
    let segments: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(segments.len() >= 2, "ready means two segments: {text}");
    assert!(
        segments.iter().all(|s| !s.contains('/')),
        "segments are relative to the playlist: {text}"
    );

    // A segment is MPEG-TS: 188-byte packets, each opening with 0x47.
    let segment = call(
        &state,
        "GET",
        &format!("/api/v2/streams/hls/t/{token}/{}", segments[0]),
        "",
    )
    .await;
    assert_eq!(segment.status, StatusCode::OK);
    assert_eq!(segment.content_type, "video/mp2t");
    assert!(
        segment.body.len() >= 188 * 2,
        "{} bytes",
        segment.body.len()
    );
    assert_eq!(segment.body.len() % 188, 0, "not whole TS packets");
    assert!(
        segment.body.chunks(188).all(|p| p[0] == 0x47),
        "every packet opens with the TS sync byte"
    );

    // A second listener joins the same encoder.
    let second = call(
        &state,
        "POST",
        "/api/v2/streams/hls/src_hls_play/start",
        r#"{"session_id":"tab-b"}"#,
    )
    .await
    .json();
    assert_eq!(second["stream_token"], token.as_str(), "a second encoder");
    assert_eq!(second["active_clients"], 2);

    // Status lists it once, without the token.
    let status = call(&state, "GET", "/api/v2/streams/hls/status", "")
        .await
        .json();
    let ours: Vec<&serde_json::Value> = status["streams"]
        .as_array()
        .expect("streams")
        .iter()
        .filter(|s| s["source"] == "src_hls_play")
        .collect();
    assert_eq!(ours.len(), 1, "{status}");
    assert!(
        ours[0].get("stream_token").is_none(),
        "status leaks the token"
    );

    // One listener leaving keeps the stream for the other.
    let beat = call(
        &state,
        "POST",
        "/api/v2/streams/hls/heartbeat?disconnect=true",
        &format!(r#"{{"stream_token":"{token}","session_id":"tab-a"}}"#),
    )
    .await;
    assert_eq!(beat.json()["status"], "ok");
    assert_eq!(
        call(&state, "GET", &playlist_url, "").await.status,
        StatusCode::OK,
        "ended while a listener remained"
    );

    // The last one stopping ends it, and its segments go with it.
    let dir = std::env::temp_dir().join("birdnet-hls").join(&token);
    assert!(dir.is_dir(), "segments are not where they were expected");
    let stopped = call(
        &state,
        "POST",
        "/api/v2/streams/hls/src_hls_play/stop",
        r#"{"session_id":"tab-b"}"#,
    )
    .await;
    assert_eq!(stopped.json()["status"], "stopped");
    assert_eq!(
        call(&state, "GET", &playlist_url, "").await.status,
        StatusCode::NOT_FOUND
    );
    assert!(
        !dir.exists(),
        "the ended stream's segments were left behind"
    );

    producing.store(false, Ordering::Relaxed);
    feeder.join().expect("feeder");
}

#[tokio::test]
async fn a_source_the_station_does_not_have_is_not_found() {
    let (state, producing, feeder) = recording_station("src_hls_known");
    let answer = call(
        &state,
        "POST",
        "/api/v2/streams/hls/no_such_source/start",
        "",
    )
    .await;
    producing.store(false, Ordering::Relaxed);
    feeder.join().expect("feeder");
    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{}", answer.text());
}

#[tokio::test]
async fn only_a_running_streams_token_reaches_it() {
    let (state, producing, feeder) = recording_station("src_hls_token");
    let start = call(&state, "POST", "/api/v2/streams/hls/default/start", "").await;
    producing.store(false, Ordering::Relaxed);
    if !ffmpeg_available() {
        feeder.join().expect("feeder");
        eprintln!("SKIPPED the token checks: ffmpeg is not on PATH");
        return;
    }
    assert_eq!(start.status, StatusCode::OK, "{}", start.text());
    let token = start.json()["stream_token"]
        .as_str()
        .expect("token")
        .to_owned();
    // The counterpart, so the refusals below are about the request.
    assert_eq!(
        call(
            &state,
            "GET",
            &format!("/api/v2/streams/hls/t/{token}/playlist.m3u8"),
            ""
        )
        .await
        .status,
        StatusCode::OK
    );
    for uri in [
        "/api/v2/streams/hls/t/00000000000000000000000000000000/playlist.m3u8".to_owned(),
        format!("/api/v2/streams/hls/t/{token}/..%2F..%2F..%2F..%2F..%2Fetc%2Fpasswd"),
        format!("/api/v2/streams/hls/t/{token}/playlist.m3u8.tmp"),
    ] {
        assert_eq!(
            call(&state, "GET", &uri, "").await.status,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    // A heartbeat without a body is malformed; one for an ended stream is not.
    assert_eq!(
        call(&state, "POST", "/api/v2/streams/hls/heartbeat", "")
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let gone = call(
        &state,
        "POST",
        "/api/v2/streams/hls/heartbeat",
        r#"{"stream_token":"ended-long-ago"}"#,
    )
    .await;
    assert_eq!(gone.status, StatusCode::OK);

    // Ended by a sessionless disconnect, as a client without one sends.
    call(
        &state,
        "POST",
        "/api/v2/streams/hls/heartbeat?status=disconnect",
        &format!(r#"{{"stream_token":"{token}"}}"#),
    )
    .await;
    assert_eq!(
        call(
            &state,
            "GET",
            &format!("/api/v2/streams/hls/t/{token}/playlist.m3u8"),
            ""
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    feeder.join().expect("feeder");
}
