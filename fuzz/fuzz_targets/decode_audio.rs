//! Fuzz the audio decode path end to end.
//!
//! `decode_file_capped` is the first thing that touches a file dropped into
//! the watch directory, so it parses fully attacker-controlled bytes via
//! symphonia's WAV/FLAC/MP3 demuxers. Any panic, OOM, or hang here is a
//! denial-of-service against the detection daemon.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((selector, body)) = data.split_first() else {
        return;
    };

    // The demuxer is chosen by file extension, so route the same corpus
    // through all three container parsers.
    let ext = match selector % 3 {
        0 => "wav",
        1 => "flac",
        _ => "mp3",
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(format!("fuzz.{ext}"));
    std::fs::write(&path, body).expect("write fuzz input");

    // Cap mirrors the daemon's own bound; success and typed errors are both
    // fine — the only failure mode of interest is a crash.
    let _ = birdnet_core::audio::decode::decode_file_capped(&path, 1_000_000);
});
