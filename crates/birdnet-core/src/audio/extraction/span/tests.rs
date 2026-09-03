//! Gates for segment-spanning clip windows.
//!
//! The claim is not "the clip is the right length" — padding with silence
//! would satisfy that, and would be worse than a short clip because it looks
//! like the bird stopped calling. The claim is that the extra audio is the
//! *real* audio from the neighbouring segment, in the right place. So the
//! fixtures put a marker burst in one segment only and the assertions locate
//! it inside the clip.

use super::*;
use crate::audio::decode::AudioData;

const SR: u32 = 48_000;

/// A segment of `secs` seconds, silent except for a 0.1-second 1 kHz burst
/// centred at `burst_at` (negative = no burst).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn segment(secs: f32, burst_at: f32) -> AudioData {
    let n = (secs * SR as f32) as usize;
    let samples = (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / SR as f32;
            if burst_at >= 0.0 && (t - burst_at).abs() < 0.05 {
                (std::f32::consts::TAU * 1000.0 * t).sin()
            } else {
                0.0
            }
        })
        .collect();
    AudioData {
        samples,
        sample_rate: SR,
    }
}

/// Write `audio` as a 16-bit WAV.
fn write(path: &std::path::Path, audio: &AudioData) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in &audio.samples {
        #[allow(clippy::cast_possible_truncation)]
        w.write_sample((s * f32::from(i16::MAX)) as i16)
            .expect("write");
    }
    w.finalize().expect("finalize");
}

/// Seconds into `samples` where the loudest 10 ms window sits, or `None` when
/// the whole buffer is silent.
fn peak_at_secs(samples: &[f32]) -> Option<f32> {
    let window = (SR / 100) as usize;
    if samples.len() < window {
        return None;
    }
    let mut best = (0_usize, 0.0_f32);
    for start in (0..samples.len() - window).step_by(window / 4) {
        let energy: f32 = samples[start..start + window].iter().map(|s| s * s).sum();
        if energy > best.1 {
            best = (start, energy);
        }
    }
    if best.1 < 1e-6 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some(best.0 as f32 / SR as f32)
}

#[allow(clippy::cast_precision_loss)]
fn secs(samples: &[f32]) -> f32 {
    samples.len() as f32 / SR as f32
}

/// A directory of contiguous 15-second segments at 06:00:00, :15 and :30.
///
/// The middle one is the source. Only the named segment carries a burst, so
/// wherever the burst turns up in a clip identifies which file it came from.
struct Fixture {
    /// Held so the directory outlives the fixture; never read.
    _dir: tempfile::TempDir,
    source: std::path::PathBuf,
    audio: AudioData,
}

impl Fixture {
    /// `bursts` are `(filename time, burst offset within that segment)`.
    fn new(bursts: &[(&str, f32)]) -> Self {
        Self::with_times(
            &["06:00:00", "06:00:15", "06:00:30"],
            "06:00:15",
            bursts,
            15.0,
        )
    }

    fn with_times(
        times: &[&str],
        source_time: &str,
        bursts: &[(&str, f32)],
        secs_each: f32,
    ) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut source = std::path::PathBuf::new();
        let mut audio = None;
        for t in times {
            let burst = bursts
                .iter()
                .find(|(bt, _)| bt == t)
                .map_or(-1.0, |(_, at)| *at);
            let seg = segment(secs_each, burst);
            let path = dir.path().join(format!("2026-05-01-birdnet-{t}.wav"));
            write(&path, &seg);
            if *t == source_time {
                source = path;
                audio = Some(seg);
            }
        }
        Self {
            _dir: dir,
            source,
            audio: audio.expect("source segment is in `times`"),
        }
    }

    fn read(&self, start: f32, stop: f32) -> SpannedWindow {
        read_window(&self.source, &self.audio, start, stop).expect("read window")
    }
}

// ---------------------------------------------------------------------------
// The window that does not span
// ---------------------------------------------------------------------------

/// A window wholly inside the segment is that slice, and reads no neighbour.
#[test]
fn a_window_inside_the_segment_is_just_a_slice() {
    let f = Fixture::new(&[("06:00:15", 7.0)]);
    let w = f.read(4.5, 10.5);
    assert!((secs(&w.samples) - 6.0).abs() < 1e-3);
    assert!((w.lead_in_secs).abs() < 1e-6);
    assert!((w.tail_secs).abs() < 1e-6);
    assert!(
        peak_at_secs(&w.samples).is_some_and(|p| (p - 2.5).abs() < 0.1),
        "the burst at 7.0s in the source should land 2.5s into a clip starting at 4.5s"
    );
}

// ---------------------------------------------------------------------------
// Spanning, and what the extra audio actually is
// ---------------------------------------------------------------------------

/// A detection at the very start of a segment gets its lead-in from the
/// predecessor, and the lead-in is that segment's real audio.
///
/// The marker lives only in the predecessor, one second before its end, so a
/// window reaching 1.5 seconds back must show it half a second into the clip.
/// Padding with silence, or with a repeat of the source's own start, both give
/// a 6-second clip and neither shows the burst.
#[test]
fn a_detection_at_the_start_takes_real_audio_from_the_predecessor() {
    let f = Fixture::new(&[("06:00:00", 14.0)]);
    let w = f.read(-1.5, 4.5);

    assert!(
        (secs(&w.samples) - 6.0).abs() < 1e-3,
        "clip is {:.3}s",
        secs(&w.samples)
    );
    assert!((w.lead_in_secs - 1.5).abs() < 1e-3);
    assert!((w.tail_secs).abs() < 1e-6);

    let peak = peak_at_secs(&w.samples).expect("the predecessor's burst must be in the clip");
    assert!(
        (peak - 0.5).abs() < 0.1,
        "the burst is at 14.0s in a 15.0s predecessor, so with a 1.5s lead-in it belongs \
         0.5s into the clip; found it at {peak:.2}s"
    );
}

/// And a detection at the end takes its tail from the successor.
#[test]
fn a_detection_at_the_end_takes_real_audio_from_the_successor() {
    let f = Fixture::new(&[("06:00:30", 1.0)]);
    let w = f.read(10.5, 16.5);

    assert!((secs(&w.samples) - 6.0).abs() < 1e-3);
    assert!((w.tail_secs - 1.5).abs() < 1e-3);

    let peak = peak_at_secs(&w.samples).expect("the successor's burst must be in the clip");
    assert!(
        (peak - 5.5).abs() < 0.1,
        "the burst is 1.0s into the successor, which begins 4.5s into this clip, so it \
         belongs at 5.5s; found it at {peak:.2}s"
    );
}

/// A window reaching past both ends takes from both neighbours.
#[test]
fn a_window_can_span_both_ways_at_once() {
    let f = Fixture::with_times(
        &["06:00:00", "06:00:03", "06:00:06"],
        "06:00:03",
        &[("06:00:00", 2.5), ("06:00:06", 0.5)],
        3.0,
    );
    let w = f.read(-1.0, 4.0);
    assert!((secs(&w.samples) - 5.0).abs() < 1e-3);
    assert!((w.lead_in_secs - 1.0).abs() < 1e-3);
    assert!((w.tail_secs - 1.0).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// The guard: never splice across a gap
// ---------------------------------------------------------------------------

/// A neighbour that does not abut is refused, and the clip is short.
///
/// **The most important test here.** The predecessor is 15 seconds long but
/// starts 60 seconds before this one, so 45 seconds of recording are missing
/// between them — the station restarted, or the source dropped. Splicing them
/// would produce a clip containing audio from two different times presented as
/// one continuous recording. That is not a shorter clip, it is a fabricated
/// one, and nothing downstream could ever detect it.
///
/// A short clip is the correct outcome, and it is what this asserts.
#[test]
fn a_neighbour_across_a_recording_gap_is_refused() {
    let f = Fixture::with_times(
        &["05:59:15", "06:00:15"],
        "06:00:15",
        &[("05:59:15", 14.0)],
        15.0,
    );
    let w = f.read(-1.5, 4.5);

    assert!((w.lead_in_secs).abs() < 1e-6, "no lead-in may be taken");
    assert!(
        (secs(&w.samples) - 4.5).abs() < 1e-3,
        "the clip must be short ({:.3}s), not spliced",
        secs(&w.samples)
    );
    assert!(
        peak_at_secs(&w.samples).is_none(),
        "audio from 45 seconds earlier appeared in the clip"
    );
}

/// The tolerance absorbs a real segmenter's jitter but not a gap.
///
/// `arecord` and `ffmpeg` cut on a period boundary, so a nominally 15-second
/// segment is routinely 14.98 or 15.02 seconds. Refusing those would disable
/// the feature on every real station; accepting a whole missing segment would
/// be the fabrication above. Both edges are asserted.
#[test]
fn the_contiguity_tolerance_absorbs_jitter_and_not_a_gap() {
    assert!(
        Neighbour::abuts(15.0, 15.0 - 0.02),
        "a segment 20ms short must still count as contiguous"
    );
    assert!(Neighbour::abuts(15.0, 15.0 + 0.02), "and one 20ms long");
    assert!(
        !Neighbour::abuts(15.0, 15.0 - CONTIGUITY_TOLERANCE_SECS - 0.01),
        "just past the tolerance must be refused"
    );
    assert!(
        !Neighbour::abuts(60.0, 15.0),
        "a 45-second hole must be refused"
    );
    assert!(
        !Neighbour::abuts(-15.0, 15.0),
        "a negative gap is not a predecessor at all"
    );
}

/// Another source's segments are never read, however contiguous they look.
///
/// Three microphones write into one directory. Reaching across to another
/// one's audio would put a different place's soundscape into this clip, which
/// is the same fabrication as splicing across a gap and rather more likely to
/// go unnoticed.
#[test]
fn another_capture_source_is_never_used() {
    let dir = tempfile::tempdir().expect("tempdir");
    // This source has no predecessor of its own...
    let source = dir.path().join("2026-05-01-birdnet-RTSP_1-06:00:15.wav");
    let audio = segment(15.0, -1.0);
    write(&source, &audio);
    // ...but another source has a perfectly contiguous one.
    write(
        &dir.path().join("2026-05-01-birdnet-RTSP_2-06:00:00.wav"),
        &segment(15.0, 14.0),
    );
    write(
        &dir.path().join("2026-05-01-birdnet-06:00:00.wav"),
        &segment(15.0, 14.0),
    );

    let w = read_window(&source, &audio, -1.5, 4.5).expect("read");
    assert!(
        (w.lead_in_secs).abs() < 1e-6,
        "audio from another microphone was used as this one's lead-in"
    );
    assert!(peak_at_secs(&w.samples).is_none());
}

/// A neighbour recorded at a different sample rate is refused.
///
/// A rate change means the device changed or was reconfigured. Concatenating
/// without resampling would play part of the clip at the wrong speed;
/// resampling would hide a configuration change the operator needs to know
/// about. Refusing does neither.
///
/// The neighbour here is a genuine 15 seconds at 44.1 kHz, so it *abuts*
/// perfectly and the contiguity guard has nothing to say about it — only the
/// rate check can refuse it. The first version of this test wrote 48 kHz worth
/// of samples and relabelled the header, which made the file 16.33 seconds
/// long; the contiguity guard rejected it and the test passed without the rate
/// check ever running. A mutant deleting that check left it green.
#[test]
fn a_neighbour_at_a_different_sample_rate_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("2026-05-01-birdnet-06:00:15.wav");
    let audio = segment(15.0, -1.0);
    write(&source, &audio);

    let other = AudioData {
        samples: vec![0.5_f32; 15 * 44_100],
        sample_rate: 44_100,
    };
    write(&dir.path().join("2026-05-01-birdnet-06:00:00.wav"), &other);

    let w = read_window(&source, &audio, -1.5, 4.5).expect("read");
    assert!(
        (w.lead_in_secs).abs() < 1e-6,
        "audio at another sample rate was spliced in and would play at the wrong speed"
    );
    assert!((secs(&w.samples) - 4.5).abs() < 1e-3);
}

/// ...and the counterpart, so the test above is not satisfied by refusing
/// every neighbour: the *same* fixture at the matching rate is accepted.
#[test]
fn a_matching_sample_rate_neighbour_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("2026-05-01-birdnet-06:00:15.wav");
    let audio = segment(15.0, -1.0);
    write(&source, &audio);

    let other = AudioData {
        samples: vec![0.5_f32; 15 * SR as usize],
        sample_rate: SR,
    };
    write(&dir.path().join("2026-05-01-birdnet-06:00:00.wav"), &other);

    let w = read_window(&source, &audio, -1.5, 4.5).expect("read");
    assert!((w.lead_in_secs - 1.5).abs() < 1e-3);
    assert!((secs(&w.samples) - 6.0).abs() < 1e-3);
}

/// With no neighbour at all, the window clamps — which is what the extractor
/// did everywhere before this existed.
#[test]
fn a_lone_segment_still_produces_a_clamped_clip() {
    let f = Fixture::with_times(&["06:00:15"], "06:00:15", &[], 15.0);
    let w = f.read(-1.5, 4.5);
    assert!((w.lead_in_secs).abs() < 1e-6);
    assert!((secs(&w.samples) - 4.5).abs() < 1e-3);
}

// ---------------------------------------------------------------------------
// Time arithmetic
// ---------------------------------------------------------------------------

/// Segment times must survive being compared, at real dates.
///
/// The first version of `segment_epoch_secs` returned `f32`. Epoch seconds in
/// 2026 are about 1.78e9 and `f32` has a 24-bit mantissa, so its resolution up
/// there is **256 seconds**: three 15-second segments all mapped to the same
/// value, no segment was ever earlier or later than any other, no neighbour
/// was ever found, and the whole feature silently did nothing while every
/// clip-length assertion still passed. Nothing but this catches that.
#[test]
fn a_segment_time_survives_being_compared() {
    let at = |time: &str| {
        segment_epoch_secs(
            &crate::detection::types::RecordingFile::parse(&format!(
                "/x/2026-05-01-birdnet-{time}.wav"
            ))
            .expect("parse"),
        )
        .expect("epoch")
    };
    let a = at("06:00:00");
    let b = at("06:00:15");
    let c = at("06:00:30");

    assert!(a < b && b < c, "15-second segments must order: {a} {b} {c}");
    assert_eq!(
        b - a,
        15,
        "and the gap must be exactly 15 seconds, not 0 or 256"
    );
    assert_eq!(c - b, 15);
}

/// Midnight is not a discontinuity.
///
/// A nocturnal station crosses it every night, and a date-blind comparison
/// would make 00:00:00 appear 86 385 seconds *before* the previous day's
/// 23:59:45 rather than 15 seconds after it — so the last segment of the
/// night would never find its predecessor, and the first of the morning would
/// pick a wrong one.
#[test]
fn the_predecessor_of_midnight_is_the_previous_night() {
    let at = |date: &str, time: &str| {
        segment_epoch_secs(
            &crate::detection::types::RecordingFile::parse(&format!(
                "/x/{date}-birdnet-{time}.wav"
            ))
            .expect("parse"),
        )
        .expect("epoch")
    };
    assert_eq!(
        at("2026-05-02", "00:00:00") - at("2026-05-01", "23:59:45"),
        15
    );

    // ...and end to end, through the real lookup.
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("2026-05-02-birdnet-00:00:00.wav");
    let audio = segment(15.0, -1.0);
    write(&source, &audio);
    write(
        &dir.path().join("2026-05-01-birdnet-23:59:45.wav"),
        &segment(15.0, 14.0),
    );

    let w = read_window(&source, &audio, -1.5, 4.5).expect("read");
    assert!(
        (w.lead_in_secs - 1.5).abs() < 1e-3,
        "the segment before midnight is this one's predecessor"
    );
    assert!(peak_at_secs(&w.samples).is_some_and(|p| (p - 0.5).abs() < 0.1));
}

/// A malformed or impossible timestamp yields no time rather than a plausible
/// one, so such a file is never treated as a neighbour.
#[test]
fn an_impossible_timestamp_is_refused() {
    let at = |name: &str| {
        crate::detection::types::RecordingFile::parse(name).and_then(|f| segment_epoch_secs(&f))
    };
    assert!(at("/x/2026-05-01-birdnet-06:00:00.wav").is_some());
    assert_eq!(at("/x/2026-13-01-birdnet-06:00:00.wav"), None, "month 13");
    assert_eq!(at("/x/2026-05-32-birdnet-06:00:00.wav"), None, "day 32");
    assert_eq!(at("/x/2026-05-01-birdnet-25:00:00.wav"), None, "hour 25");
    assert_eq!(at("/x/2026-05-01-birdnet-06:60:00.wav"), None, "minute 60");
    // A leap second is legal in a wall-clock timestamp and must be accepted.
    assert!(at("/x/2026-05-01-birdnet-06:00:60.wav").is_some());
}

/// The directory listing survives files it cannot parse.
#[test]
fn unparseable_files_in_the_directory_are_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("2026-05-01-birdnet-06:00:15.wav");
    let audio = segment(15.0, -1.0);
    write(&source, &audio);
    write(
        &dir.path().join("2026-05-01-birdnet-06:00:00.wav"),
        &segment(15.0, 14.0),
    );
    std::fs::write(dir.path().join("notes.txt"), b"hello").expect("write");
    std::fs::create_dir(dir.path().join("subdir")).expect("mkdir");

    let w = read_window(&source, &audio, -1.5, 4.5).expect("read");
    assert!((w.lead_in_secs - 1.5).abs() < 1e-3);
}
