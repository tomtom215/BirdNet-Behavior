//! What the station's own microphones sound like, sampled over time.
//!
//! # The failure this is for
//!
//! A microphone that dies outright is caught several ways: the capture
//! supervisor restarts its process, `birdnet_audio_source_up` drops to 0, and
//! the detection deadman fires if the station goes silent. A microphone that
//! merely goes *deaf* — water in the capsule, a spider's web across the port, a
//! connector loosened by a year of thermal cycling, a preamp drifting — is
//! caught by none of them. The process is alive, the gauge reads 1, audio keeps
//! arriving, and the station goes on detecting the loud, close birds while
//! quietly losing everything else.
//!
//! Its only symptom is fewer detections. So is the end of the breeding season.
//! No number this project stored could tell those apart, which for a station
//! sealed into an enclosure for a year is the gap that matters most: by the time
//! anyone notices, the season is over and unrecoverable.
//!
//! The station's own **noise floor** separates them. Ambient background does not
//! stop when the birds do. A quiet season moves the detections and leaves the
//! floor where it was; a deaf microphone takes the floor down with it.
//!
//! # Sampling, not instrumenting
//!
//! Every [`POLL_EVERY`] this task takes the newest segment each source has
//! written to the transient stream directory, decodes it, and folds one
//! observation into `audio_levels`. Deliberately the same shape as
//! [`super::effort`]: a sample rather than an integration.
//!
//! * It touches no part of the audio path, so it cannot disturb capture.
//! * It decodes one ~15-second file per source per interval — milliseconds of
//!   work every five minutes — rather than adding a pass to every segment on a
//!   Pi that is already the bottleneck.
//! * A restart costs at most one interval.
//! * At five minutes it takes ~288 observations a day per source, which is far
//!   more than a trend measured in dB over weeks can use.
//!
//! # What this does *not* do
//!
//! It does not gate inference. `birdnet_core::audio::quality` can also decide
//! whether a chunk is worth analysing, and deliberately does not: that changes
//! which audio reaches the model, and the note in `src/cli.rs` is right that it
//! wants hardware validation behind it before it ships. Observing carries none
//! of that risk — nothing downstream reads these numbers to make a decision —
//! and it is the half a sealed station actually needs.
//!
//! It also does not alert. A noise floor moves for real reasons: weather,
//! season, a road, a lawnmower, leaf-out. A threshold picked here, without a
//! season of real recordings to calibrate against, would fire on all of those
//! and teach an operator to ignore the channel — the exact failure
//! [`super::station_health`] is written to avoid. The measurement comes first;
//! an alert can follow once there is a baseline to draw it from.

use std::path::{Path, PathBuf};
use std::time::Duration;

use birdnet_db::audio_levels::{LevelSample, record_sample};
use birdnet_web::state::AppState;

/// How often each source is sampled.
///
/// Matches the station-health and recording-effort polls, so a station has one
/// cadence rather than three.
const POLL_EVERY: Duration = Duration::from_secs(300);

/// How much history to keep, in days.
///
/// A year, so a full seasonal cycle is available to compare the next one
/// against — which is the comparison the whole table exists to make. At three
/// sources that is ~26 000 rows.
const KEEP_DAYS: u32 = 400;

/// How often the old buckets are pruned.
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// At most this many samples are read from one segment.
///
/// The quality assessment is a whole-buffer statistic, so a longer file buys
/// precision this does not need. Ten seconds at 48 kHz bounds the decode and the
/// arithmetic regardless of how long a segment the operator has configured.
const MAX_SAMPLES: usize = 10 * 48_000;

/// The newest audio file in `dir`, and the source it belongs to.
///
/// "Newest" by modification time rather than by filename: the names carry local
/// wall clock, which repeats for one hour every autumn, and sorting by a
/// timestamp that goes backwards would pick the wrong file on exactly the night
/// the operator would most want the measurement.
fn newest_segment_per_source(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut best: std::collections::BTreeMap<String, (std::time::SystemTime, PathBuf)> =
        std::collections::BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Same extension set the detection pipeline's watcher accepts. Kept
        // as a local list rather than reaching for the crate-private helper:
        // this only ever sees files capture itself wrote.
        let is_audio = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "wav" | "flac" | "mp3"));
        if !is_audio {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let source = crate::daemon::disposition::derive_source_label(&path);
        best.entry(source)
            .and_modify(|slot| {
                if modified > slot.0 {
                    *slot = (modified, path.clone());
                }
            })
            .or_insert((modified, path));
    }
    best.into_iter().map(|(s, (_, p))| (s, p)).collect()
}

/// Decode `path` and assess it, or `None` if it cannot be read.
///
/// Every failure here is expected in normal operation — the segment can be
/// mid-write, or drained by the retention purge between the listing and the
/// read — so none of them is worth more than a debug line.
fn assess(path: &Path) -> Option<LevelSample> {
    let audio = birdnet_core::audio::decode::decode_file_capped(path, MAX_SAMPLES).ok()?;
    if audio.samples.is_empty() {
        return None;
    }
    let score =
        birdnet_core::audio::quality::assess_quality(&audio.samples, audio.sample_rate).ok()?;
    Some(LevelSample {
        noise_floor_dbfs: score.noise_floor_dbfs,
        snr_db: score.snr_db,
        spectral_flatness: score.spectral_flatness,
        rain: score.rain_detected,
    })
}

/// Take one observation per source. Returns how many landed, for the log and
/// for tests.
fn sample_once(state: &AppState, stream_dir: &Path) -> usize {
    let (date, hour) = local_date_hour();
    let mut recorded = 0;
    for (source, path) in newest_segment_per_source(stream_dir) {
        let Some(sample) = assess(&path) else {
            tracing::debug!(path = %path.display(), "acoustic sample skipped");
            continue;
        };
        match state.with_db(|conn| record_sample(conn, &date, hour, &source, sample)) {
            Ok(()) => recorded += 1,
            Err(e) => tracing::debug!(error = %e, source = %source, "acoustic sample not stored"),
        }
    }
    recorded
}

/// The station's local date and hour — the same lens the detections are stamped
/// with, so a bucket lines up with the detections it will be read beside.
fn local_date_hour() -> (String, u8) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
        + birdnet_db::clock::local_utc_offset_secs();
    let c = birdnet_core::civil::civil_from_unix_secs(secs);
    (
        format!("{:04}-{:02}-{:02}", c.year, c.month, c.day),
        u8::try_from(c.hour).unwrap_or(0),
    )
}

/// Spawn the acoustic-health sampler.
///
/// `stream_dir` is the transient capture directory. Skipped in web-only mode by
/// the caller: with no capture running there is nothing to measure, and an
/// empty directory would simply record nothing every five minutes forever.
pub fn spawn_acoustic_health(state: AppState, stream_dir: PathBuf) {
    tokio::spawn(async move {
        tracing::info!(
            poll_secs = POLL_EVERY.as_secs(),
            dir = %stream_dir.display(),
            "acoustic-health sampler started"
        );
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the immediate first tick: at t=0 capture has not written a
        // segment yet, so it would find nothing and only add a wakeup.
        tick.tick().await;
        let mut last_prune = tokio::time::Instant::now();
        loop {
            tick.tick().await;
            let probe = state.clone();
            let dir = stream_dir.clone();
            let _ = tokio::task::spawn_blocking(move || sample_once(&probe, &dir)).await;

            if last_prune.elapsed() >= PRUNE_EVERY {
                last_prune = tokio::time::Instant::now();
                let pruner = state.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    match pruner.with_db(|conn| birdnet_db::audio_levels::prune(conn, KEEP_DAYS)) {
                        Ok(n) if n > 0 => {
                            tracing::info!(removed = n, "pruned old acoustic-health buckets");
                        }
                        Ok(_) => {}
                        Err(e) => tracing::debug!(error = %e, "acoustic-health prune failed"),
                    }
                })
                .await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};

    /// Write a WAV of white noise at `amplitude`, named as capture names its
    /// segments so `derive_source_label` can read the source back out.
    fn write_segment(dir: &Path, name: &str, amplitude: f32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(dir.join(name), spec).expect("create wav");
        // A deterministic pseudo-noise so the test does not depend on an RNG.
        let mut x: u32 = 0x2545_F491;
        for _ in 0..48_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let v = ((x as f32 / u32::MAX as f32) - 0.5) * 2.0 * amplitude;
            #[allow(clippy::cast_possible_truncation)]
            w.write_sample((v * f32::from(i16::MAX)) as i16)
                .expect("write sample");
        }
        w.finalize().expect("finalize");
    }

    #[test]
    fn the_newest_segment_of_each_source_is_the_one_sampled() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_segment(dir.path(), "2026-08-20-birdnet-cam1-06:00:00.wav", 0.2);
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_segment(dir.path(), "2026-08-20-birdnet-cam1-06:00:15.wav", 0.2);
        write_segment(dir.path(), "2026-08-20-birdnet-06:00:15.wav", 0.2);

        let found = newest_segment_per_source(dir.path());
        assert_eq!(found.len(), 2, "one per source: cam1 and the local mic");
        let cam1 = found
            .iter()
            .find(|(s, _)| s == "cam1")
            .expect("cam1 present");
        assert!(
            cam1.1.to_string_lossy().ends_with("06:00:15.wav"),
            "the newer of cam1's two segments must win, got {}",
            cam1.1.display()
        );
        assert!(found.iter().any(|(s, _)| s == "local"));
    }

    /// The measurement has to *move* with the microphone, or the table is a
    /// column of identical numbers dressed up as a diagnostic.
    #[test]
    fn a_quieter_microphone_reads_as_a_lower_noise_floor() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_segment(dir.path(), "2026-08-20-birdnet-loud-06:00:00.wav", 0.30);
        write_segment(dir.path(), "2026-08-20-birdnet-quiet-06:00:00.wav", 0.01);

        let loud = assess(&dir.path().join("2026-08-20-birdnet-loud-06:00:00.wav"))
            .expect("loud assessed");
        let quiet = assess(&dir.path().join("2026-08-20-birdnet-quiet-06:00:00.wav"))
            .expect("quiet assessed");
        assert!(
            quiet.noise_floor_dbfs < loud.noise_floor_dbfs - 10.0,
            "a 30x quieter capsule must read far lower: loud={} quiet={}",
            loud.noise_floor_dbfs,
            quiet.noise_floor_dbfs
        );
    }

    /// End to end: two microphones, one healthy and one at 2 % sensitivity,
    /// through the real assessment and into the real table.
    ///
    /// This is the discrimination the whole feature rests on, and it is not
    /// obvious in advance which measure provides it. Run on the repository's
    /// own 15-second magpie recording, attenuated to 2 %: the **noise floor**
    /// separates the two by ~35 dB (−42.5 vs −77.3 dBFS) while **SNR barely
    /// moves** (2.7 vs 2.9 dB), because attenuation scales signal and
    /// background together. A version of this built on SNR would have looked
    /// perfectly reasonable and detected nothing.
    #[test]
    fn a_failing_microphone_is_visible_in_the_table_a_healthy_one_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_segment(dir.path(), "2026-08-20-birdnet-healthy-06:00:00.wav", 0.30);
        write_segment(dir.path(), "2026-08-20-birdnet-deaf-06:00:00.wav", 0.006);

        let state = crate::integrations::test_support::test_state();
        let recorded = sample_once(&state, dir.path());
        assert_eq!(recorded, 2, "one observation per source");

        let rows = state
            .with_db(|conn| birdnet_db::audio_levels::recent_hours(conn, 10))
            .expect("read back");
        assert_eq!(rows.len(), 2);
        let level = |name: &str| {
            rows.iter()
                .find(|r| r.source == name)
                .map(|r| r.noise_floor_dbfs)
                .expect("source present")
        };
        let (healthy, deaf) = (level("healthy"), level("deaf"));
        assert!(
            deaf < healthy - 20.0,
            "a 50x quieter capsule must be unmistakable in the stored row: \
             healthy={healthy:.1} dBFS deaf={deaf:.1} dBFS"
        );
    }

    #[test]
    fn a_missing_or_unreadable_file_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(assess(&dir.path().join("nope.wav")).is_none());
        std::fs::write(dir.path().join("junk.wav"), b"not a wav").expect("write junk");
        assert!(assess(&dir.path().join("junk.wav")).is_none());
    }

    #[test]
    fn an_empty_stream_directory_records_nothing_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(newest_segment_per_source(dir.path()).is_empty());
        assert!(newest_segment_per_source(Path::new("/nonexistent/bnb")).is_empty());
    }
}
