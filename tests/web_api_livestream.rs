//! Integration tests for `GET /stream` — the live-audio endpoint — against the
//! real router.
//!
//! # What these exist to protect
//!
//! An ALSA `plughw:` capture device is exclusive. `/stream` used to open the
//! configured device itself, which on a recording station returns `Device or
//! resource busy`, so live audio could not work on a single-microphone build at
//! all. It now subscribes to the PCM capture is already producing, published
//! through the `AppState`'s live-audio hub.
//!
//! The hop these tests guard is the one that is easy to get silently wrong: the
//! tap is keyed by the **`audio_sources` row id**, because that is the label
//! capture registers it under. If the endpoint looked it up under anything else
//! it would find nothing, fall through to opening the device, and reinstate the
//! original bug — while still looking plausible in code review.
//!
//! `ffmpeg` is not assumed to be installed: it is needed only to *encode* the
//! stream as MP3, and every assertion here is about which source `/stream`
//! resolves, which happens first.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::Connection;
use tower::ServiceExt;

use birdnet_core::audio::capture::{LiveAudioHubHandle, LiveTap, PcmSpec, new_live_audio_hub};
use birdnet_db::audio_sources::{AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

const MONO_48K: PcmSpec = PcmSpec {
    sample_rate: 48_000,
    channels: 1,
};

/// A migrated in-memory state, optionally carrying a live-audio hub.
fn state_with(hub: Option<LiveAudioHubHandle>) -> AppState {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    match hub {
        Some(hub) => state.with_live_audio(hub),
        None => state,
    }
}

/// Insert an enabled USB/ALSA source row, as the installer's seeding does.
fn add_mic_source(state: &AppState, id: &str, device: &str) {
    let new = NewAudioSource::defaults(id.to_string(), SourceKind::UsbAlsa, device.to_string());
    state
        .with_db(|conn| AudioSourceStore::insert(conn, &new))
        .expect("insert audio source");
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    // Only read the body for non-2xx: a successful stream body never ends.
    let body = if status.is_success() {
        String::new()
    } else {
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (status, body)
}

#[tokio::test]
async fn no_configured_source_is_service_unavailable() {
    let (status, body) = get(state_with(None), "/stream").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.contains("no audio source configured"),
        "unexpected body: {body}"
    );
}

#[tokio::test]
async fn unknown_source_id_is_not_found() {
    let state = state_with(None);
    add_mic_source(&state, "src_seed_1", "plughw:CARD=PRO,DEV=0");
    let (status, _) = get(state, "/stream?source_id=no_such_source").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A source whose tap exists but is silent is not recording — paused by the
/// schedule or by a quiet window, or down. Saying so beats holding a connection
/// open forever delivering nothing, which is what a naive "just stream the ring"
/// implementation would do.
#[tokio::test]
async fn a_source_that_is_not_recording_says_so() {
    let hub = new_live_audio_hub();
    // Capture registers the tap under the row id and then goes quiet.
    hub.tap("src_seed_1", MONO_48K);

    let state = state_with(Some(hub));
    add_mic_source(&state, "src_seed_1", "plughw:CARD=PRO,DEV=0");

    let (status, body) = get(state, "/stream?source_id=src_seed_1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.contains("not recording"),
        "a silent tap must report that the source is not recording, got: {body}"
    );
}

/// The endpoint must find a tap registered under the `audio_sources` row id.
///
/// With audio flowing, resolution gets past the tap and on to encoding — so the
/// response is either a stream (ffmpeg present) or the specific "needs ffmpeg"
/// refusal (ffmpeg absent, as on this runner). What it must *never* be is the
/// "not recording" refusal above: that would mean the lookup missed and the
/// endpoint was about to open the exclusive capture device instead.
#[tokio::test]
async fn a_recording_source_is_streamed_from_its_tap() {
    let hub = new_live_audio_hub();
    let tap = hub.tap("src_seed_1", MONO_48K);

    // A writer standing in for the capture tee, feeding the tap for the
    // duration of the request.
    let producing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let stop = std::sync::Arc::clone(&producing);
    let feeder = std::thread::spawn(move || {
        while stop.load(std::sync::atomic::Ordering::Relaxed) {
            tap.push(&[0u8; 1920]); // 20 ms of 48 kHz mono S16
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    let state = state_with(Some(hub));
    add_mic_source(&state, "src_seed_1", "plughw:CARD=PRO,DEV=0");
    let (status, body) = get(state, "/stream?source_id=src_seed_1").await;

    producing.store(false, std::sync::atomic::Ordering::Relaxed);
    feeder.join().expect("feeder thread");

    assert!(
        !body.contains("not recording"),
        "the tap is producing audio, so the endpoint must not report the \
         source as idle — it looked the tap up under the wrong key: {body}"
    );
    assert!(
        status == StatusCode::OK || body.contains("ffmpeg"),
        "expected a stream or the ffmpeg-missing refusal, got {status}: {body}"
    );
}

/// The default (no `?source_id=`) path resolves the first enabled row, and must
/// consult that row's tap too — the Listen page uses it on every load.
#[tokio::test]
async fn the_default_source_also_uses_its_tap() {
    let hub = new_live_audio_hub();
    hub.tap("src_seed_1", MONO_48K);

    let state = state_with(Some(hub));
    add_mic_source(&state, "src_seed_1", "plughw:CARD=PRO,DEV=0");

    let (status, body) = get(state, "/stream").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        body.contains("not recording"),
        "the default path must resolve the same tap as ?source_id=: {body}"
    );
}

/// A tap registered under some other label must not be served for this row.
/// Sharing one station's microphone audio under another source's name would be
/// worse than no live audio at all.
#[tokio::test]
async fn a_tap_belonging_to_another_source_is_not_used() {
    let hub = new_live_audio_hub();
    let other = hub.tap("src_seed_2", MONO_48K);
    other.push(&[1u8; 4096]);

    let state = state_with(Some(hub));
    add_mic_source(&state, "src_seed_1", "plughw:CARD=PRO,DEV=0");

    let (_, body) = get(state, "/stream?source_id=src_seed_1").await;
    assert!(
        !body.contains("not recording"),
        "src_seed_1 has no tap at all, so it must fall through to its own \
         device rather than being told it is idle: {body}"
    );
}

/// The tap must carry the format capture recorded at, because ffmpeg is
/// configured from it — a mismatch plays the stream at the wrong pitch.
#[test]
fn a_taps_format_is_the_one_capture_registered() {
    let hub = new_live_audio_hub();
    let spec = PcmSpec {
        sample_rate: 44_100,
        channels: 2,
    };
    hub.tap("src_seed_1", spec);
    let looked_up: std::sync::Arc<LiveTap> = hub.lookup("src_seed_1").expect("tap");
    assert_eq!(looked_up.spec(), spec);
}
