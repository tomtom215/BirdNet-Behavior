//! The live-audio policy's storage (OOB-13): which stations are seeded open
//! and which closed, that the seed never overwrites a choice, that a missing or
//! misspelt value reads closed, and which paths the policy covers.
//!
//! Integration tests rather than unit tests so they run from the library this
//! crate's other route tests already link.

use birdnet_web::live_audio::{LiveAudioPolicy, is_live_audio_path, policy, seed};
use rusqlite::Connection;

fn db() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn
}

#[test]
fn the_covered_paths_are_the_audio_and_not_the_token_content() {
    for p in [
        "/stream",
        "/api/v2/streams/hls/default/start",
        "/api/v2/streams/hls/src_a/stop",
        "/api/v2/streams/hls/heartbeat",
        "/api/v2/streams/hls/status",
    ] {
        assert!(is_live_audio_path(p), "{p} must be covered");
    }
    for p in [
        "/api/v2/streams/hls/t/abc/playlist.m3u8",
        "/api/v2/ws/spectrogram",
        "/streamx",
        "/api/v2/recordings/x.wav",
        "/recordings",
    ] {
        assert!(!is_live_audio_path(p), "{p} must not be covered");
    }
}

#[test]
fn a_station_with_detections_is_seeded_open_and_a_new_one_closed() {
    let new = db();
    assert_eq!(seed(&new).unwrap(), Some(LiveAudioPolicy::SignedIn));
    assert_eq!(policy(&new), LiveAudioPolicy::SignedIn);

    let existing = db();
    existing
        .execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) \
             VALUES ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, 'x.wav')",
            [],
        )
        .unwrap();
    assert_eq!(seed(&existing).unwrap(), Some(LiveAudioPolicy::Anyone));
    assert_eq!(policy(&existing), LiveAudioPolicy::Anyone);
}

#[test]
fn the_seed_never_overwrites_a_choice() {
    let conn = db();
    assert_eq!(seed(&conn).unwrap(), Some(LiveAudioPolicy::SignedIn));
    // Detections arrive; the next start must not reopen the feed.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) \
         VALUES ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, 'x.wav')",
        [],
    )
    .unwrap();
    assert_eq!(seed(&conn).unwrap(), None);
    assert_eq!(policy(&conn), LiveAudioPolicy::SignedIn);
}

#[test]
fn a_missing_or_misspelt_value_reads_closed() {
    let conn = db();
    assert_eq!(policy(&conn), LiveAudioPolicy::SignedIn);
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('live_audio', 'anyon')",
        [],
    )
    .unwrap();
    assert_eq!(policy(&conn), LiveAudioPolicy::SignedIn);
    conn.execute(
        "UPDATE settings SET value = 'anyone' WHERE key = 'live_audio'",
        [],
    )
    .unwrap();
    assert_eq!(policy(&conn), LiveAudioPolicy::Anyone);
}

#[test]
fn the_spellings_round_trip() {
    for p in [LiveAudioPolicy::SignedIn, LiveAudioPolicy::Anyone] {
        assert_eq!(LiveAudioPolicy::parse(p.as_str()), Some(p));
    }
    assert_eq!(LiveAudioPolicy::parse(""), None);
}
