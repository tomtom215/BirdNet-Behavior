//! A detection at a segment boundary must still get a whole clip.
//!
//! `span`'s own tests cover the window arithmetic. This covers the extractor
//! that uses it, end to end and through a real WAV, because that is where the
//! defect was: the extractor clamped the window to the segment before anything
//! else saw it, and the clamp was the bug.
//!
//! The numbers below were measured against the code before the fix, on a
//! 15-second segment at 48 kHz with the default 6-second extraction:
//!
//! ```text
//! detection at  0.0s -> clip 4.500s     <- the whole lead-in, gone
//! detection at  1.0s -> clip 5.500s
//! detection at  1.5s -> clip 6.000s
//! detection at 11.0s -> clip 5.500s
//! detection at 12.0s -> clip 4.500s     <- the whole tail, gone
//! ```
//!
//! At zero overlap a 15-second segment holds five 3-second windows starting at
//! 0, 3, 6, 9 and 12, so two of every five clips were short — silently, with
//! the call itself cut off, in exactly the files a person plays to check an
//! identification and the ones uploaded to BirdWeather.

use birdnet_core::audio::extraction::{AudioFormat, ExtractionConfig, Extractor};
use birdnet_core::detection::types::Detection;

const SR: u32 = 48_000;

/// Write a 15-second segment: silent, except a 0.1 s 1 kHz burst at
/// `burst_at` seconds (negative for none).
fn write_segment(path: &std::path::Path, burst_at: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create");
    for i in 0..(SR * 15) {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / SR as f32;
        let v = if burst_at >= 0.0 && (t - burst_at).abs() < 0.05 {
            (std::f32::consts::TAU * 1000.0 * t).sin()
        } else {
            0.0
        };
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((v * f32::from(i16::MAX)) as i16)
            .expect("write");
    }
    w.finalize().expect("finalize");
}

fn detection(start: f32) -> Detection {
    Detection {
        date: "2026-05-01".into(),
        time: "06:00:15".into(),
        scientific_name: "Strix aluco".into(),
        common_name: "Tawny Owl".into(),
        confidence: 0.9,
        start,
        stop: start + 3.0,
        week: 18,
        file_name_extr: None,
    }
}

fn config(out: std::path::PathBuf, pre_capture_secs: f32) -> ExtractionConfig {
    ExtractionConfig {
        extraction_length: 6.0,
        output_dir: out,
        audio_format: "wav".into(),
        target_format: AudioFormat::Wav,
        recording_length: 15.0,
        freq_shift_hz: 0,
        pre_capture_secs,
    }
}

/// Duration of the written clip, in seconds.
fn clip_secs(path: &std::path::Path) -> f32 {
    let audio = birdnet_core::audio::decode::decode_file(path).expect("decode clip");
    #[allow(clippy::cast_precision_loss)]
    {
        audio.samples.len() as f32 / audio.sample_rate as f32
    }
}

/// Where the loudest 10 ms sits in the clip, or `None` if silent.
fn peak_secs(path: &std::path::Path) -> Option<f32> {
    let audio = birdnet_core::audio::decode::decode_file(path).expect("decode clip");
    let window = (audio.sample_rate / 100) as usize;
    if audio.samples.len() < window {
        return None;
    }
    let mut best = (0_usize, 0.0_f32);
    for start in (0..audio.samples.len() - window).step_by(window / 4) {
        let energy: f32 = audio.samples[start..start + window]
            .iter()
            .map(|s| s * s)
            .sum();
        if energy > best.1 {
            best = (start, energy);
        }
    }
    if best.1 < 1e-6 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(best.0 as f32 / audio.sample_rate as f32)
}

/// Three contiguous segments; the middle one is the source.
fn fixture(bursts: &[(&str, f32)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    for t in ["06:00:00", "06:00:15", "06:00:30"] {
        let burst = bursts
            .iter()
            .find(|(bt, _)| *bt == t)
            .map_or(-1.0, |(_, at)| *at);
        write_segment(
            &dir.path().join(format!("2026-05-01-birdnet-{t}.wav")),
            burst,
        );
    }
    let src = dir.path().join("2026-05-01-birdnet-06:00:15.wav");
    (dir, src)
}

/// Every detection position produces a full-length clip.
///
/// The whole span of the segment, at the five window starts a 15-second
/// segment actually has at zero overlap plus the two half-steps either side of
/// the boundaries.
#[test]
fn every_detection_position_yields_a_full_length_clip() {
    let (dir, src) = fixture(&[]);
    let ex = Extractor::new(config(dir.path().join("out"), 0.0));

    let mut short = Vec::new();
    for start in [0.0_f32, 1.0, 1.5, 3.0, 6.0, 7.5, 9.0, 10.5, 11.0, 12.0] {
        let path = ex
            .extract_detection(&src, &detection(start))
            .expect("extract");
        let secs = clip_secs(&path);
        if (secs - 6.0).abs() > 0.01 {
            short.push(format!("  detection at {start:>4.1}s -> clip {secs:.3}s"));
        }
        std::fs::remove_file(&path).ok();
    }
    assert!(
        short.is_empty(),
        "extraction_length is 6.0, so every clip must be 6.000s:\n{}",
        short.join("\n")
    );
}

/// The lead-in is the predecessor's real audio, in the right place.
///
/// A clip padded with silence would also be 6 seconds long and would also
/// satisfy the test above, while losing exactly what was cut off. The burst
/// lives only in the predecessor, one second before its end, so with a 1.5 s
/// lead-in it belongs half a second into the clip.
#[test]
fn the_lead_in_is_the_previous_segments_audio() {
    let (dir, src) = fixture(&[("06:00:00", 14.0)]);
    let ex = Extractor::new(config(dir.path().join("out"), 0.0));

    let path = ex
        .extract_detection(&src, &detection(0.0))
        .expect("extract");
    let peak = peak_secs(&path).expect("the predecessor's burst must be in the clip");
    assert!(
        (peak - 0.5).abs() < 0.1,
        "expected the burst 0.5s into the clip, found it at {peak:.2}s"
    );
}

/// And the tail is the successor's.
#[test]
fn the_tail_is_the_next_segments_audio() {
    let (dir, src) = fixture(&[("06:00:30", 1.0)]);
    let ex = Extractor::new(config(dir.path().join("out"), 0.0));

    let path = ex
        .extract_detection(&src, &detection(12.0))
        .expect("extract");
    let peak = peak_secs(&path).expect("the successor's burst must be in the clip");
    assert!(
        (peak - 5.5).abs() < 0.1,
        "expected the burst 5.5s into the clip, found it at {peak:.2}s"
    );
}

/// `pre_capture_secs` lengthens the clip at the front only.
///
/// Asserted at a detection in the *middle* of the segment as well as at its
/// start, because those exercise different paths: the middle one never leaves
/// the segment, and a version that only applied the extra lead-in when
/// spanning would pass the boundary case alone.
#[test]
fn pre_capture_lengthens_the_clip_at_the_front() {
    let (dir, src) = fixture(&[]);
    let plain = Extractor::new(config(dir.path().join("out-a"), 0.0));
    let extended = Extractor::new(config(dir.path().join("out-b"), 1.0));

    for start in [0.0_f32, 7.5] {
        let a = plain
            .extract_detection(&src, &detection(start))
            .expect("plain");
        let b = extended
            .extract_detection(&src, &detection(start))
            .expect("extended");
        assert!(
            (clip_secs(&a) - 6.0).abs() < 0.01,
            "at {start}s the plain clip is {:.3}s",
            clip_secs(&a)
        );
        assert!(
            (clip_secs(&b) - 7.0).abs() < 0.01,
            "at {start}s the extended clip is {:.3}s, expected 7.0",
            clip_secs(&b)
        );
    }
}

/// And it really is at the front: the detection sits one second later in the
/// extended clip than in the plain one.
///
/// The counterpart to the lengths above, which a version that appended the
/// extra second to the *end* would satisfy just as well.
#[test]
fn pre_capture_moves_the_detection_later_in_the_clip() {
    let (dir, src) = fixture(&[("06:00:15", 8.0)]);
    let plain = Extractor::new(config(dir.path().join("out-a"), 0.0));
    let extended = Extractor::new(config(dir.path().join("out-b"), 1.0));

    // A detection window of 7.5–10.5 contains the burst at 8.0.
    let a = plain
        .extract_detection(&src, &detection(7.5))
        .expect("plain");
    let b = extended
        .extract_detection(&src, &detection(7.5))
        .expect("extended");

    let pa = peak_secs(&a).expect("burst in plain clip");
    let pb = peak_secs(&b).expect("burst in extended clip");
    assert!(
        (pb - pa - 1.0).abs() < 0.1,
        "the burst is at {pa:.2}s plain and {pb:.2}s extended; one extra second of lead-in \
         should move it exactly one second later"
    );
}

/// With no neighbours at all the clip is short, as it always was.
///
/// The behaviour this change must not alter: a lone segment — the first after
/// a restart, or one whose neighbours the retention purge has taken — has
/// nothing to reach for, and a short clip is the honest result.
#[test]
fn a_lone_segment_still_produces_a_short_clip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("2026-05-01-birdnet-06:00:15.wav");
    write_segment(&src, -1.0);
    let ex = Extractor::new(config(dir.path().join("out"), 0.0));

    let path = ex
        .extract_detection(&src, &detection(0.0))
        .expect("extract");
    assert!(
        (clip_secs(&path) - 4.5).abs() < 0.01,
        "expected the old clamped 4.5s, got {:.3}s",
        clip_secs(&path)
    );
}
