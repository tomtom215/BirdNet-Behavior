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
//! It does not alert *on the levels it records*. A noise floor moves for real
//! reasons: weather, season, a road, a lawnmower, leaf-out. A threshold picked
//! here, without a season of real recordings to calibrate against, would fire
//! on all of those and teach an operator to ignore the channel — the exact
//! failure [`super::station_health`] is written to avoid. The measurement comes
//! first; an alert can follow once there is a baseline to draw it from.
//!
//! It *does* alert on the categorically broken, which is a different question.
//! [`birdnet_core::audio::quality::stream_fault`] recognises a source that is
//! alive, punctual, and carrying nothing at all — a muted channel, an unplugged
//! input, a wedged converter, a gain-blown preamp. Those need no baseline: no
//! microphone in any weather produces digitally exact zeros or sits at full
//! scale for a fifth of a segment. The supervisor cannot see it (segments
//! arrive on time, so the source reads `Connected`), and the detection deadman
//! cannot see it either on a multi-source station, because the other
//! microphones keep the station detecting.

use std::path::{Path, PathBuf};
use std::time::Duration;

use birdnet_core::audio::soundlevel::{Calibration, SoundLevelMeter};
use birdnet_db::audio_levels::{LevelSample, record_sample};
use birdnet_db::sound_levels::{BandObservation, BroadbandObservation, record_observation};
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

/// Everything one decoded segment yields.
struct Assessment {
    /// The broadband quality figures that go to `audio_levels`.
    level: LevelSample,
    /// Per-band levels, empty when the segment was too short to close a
    /// one-second window in the meter.
    bands: Vec<BandObservation>,
    /// The broadband A- and Z-weighted figures accompanying `bands`.
    broadband: Option<BroadbandObservation>,
}

/// Decode `path` and assess it, or `None` if it cannot be read.
///
/// One decode feeds both measurements. The quality figures answer "is this
/// source still working"; the band levels answer "what does this site sound
/// like, and if the source is failing, how". Running the meter here rather
/// than on the capture path is the same trade the rest of this module makes:
/// a sample every [`POLL_EVERY`], costing one filter bank pass over audio
/// already in memory, instead of a pass over every segment on a Pi that is
/// already the bottleneck.
///
/// Every failure here is expected in normal operation — the segment can be
/// mid-write, or drained by the retention purge between the listing and the
/// read — so none of them is worth more than a debug line.
fn assess(path: &Path) -> Option<Assessment> {
    let audio = birdnet_core::audio::decode::decode_file_capped(path, MAX_SAMPLES).ok()?;
    if audio.samples.is_empty() {
        return None;
    }
    let score =
        birdnet_core::audio::quality::assess_quality(&audio.samples, audio.sample_rate).ok()?;
    let level = LevelSample {
        noise_floor_dbfs: score.noise_floor_dbfs,
        snr_db: score.snr_db,
        spectral_flatness: score.spectral_flatness,
        rain: score.rain_detected,
    };

    let (bands, broadband) = measure_bands(&audio.samples, audio.sample_rate);
    Some(Assessment {
        level,
        bands,
        broadband,
    })
}

/// Run the third-octave meter over one segment.
///
/// The interval is the whole segment, rounded down to whole seconds, so one
/// observation is one reading rather than several — this is a sample, and
/// storing three readings five minutes apart from the same segment would
/// triple the row count without adding an observation.
///
/// Returns empty when the segment is shorter than a second (the meter needs a
/// full second to close a window) or when no band survives the source's sample
/// rate, both of which are silence rather than failure.
fn measure_bands(
    samples: &[f32],
    sample_rate: u32,
) -> (Vec<BandObservation>, Option<BroadbandObservation>) {
    if sample_rate == 0 {
        return (Vec::new(), None);
    }
    let whole_seconds = u32::try_from(samples.len() / sample_rate as usize).unwrap_or(u32::MAX);
    if whole_seconds == 0 {
        return (Vec::new(), None);
    }
    let Ok(mut meter) = SoundLevelMeter::new(sample_rate, whole_seconds, calibration()) else {
        return (Vec::new(), None);
    };
    let Some(reading) = meter.push(samples) else {
        return (Vec::new(), None);
    };

    let bands = reading
        .bands
        .iter()
        .map(|b| BandObservation {
            band_hz: b.centre_hz,
            mean_db: b.mean_db,
            min_db: b.min_db,
            max_db: b.max_db,
        })
        .collect();
    let broadband = BroadbandObservation {
        a_weighted_db: reading.a_weighted_db,
        z_weighted_db: reading.z_weighted_db,
        calibration_db: calibration().offset_db(),
    };
    (bands, Some(broadband))
}

/// The station's sound-level calibration.
///
/// `BIRDNET_SPL_CALIBRATION_DB` is the sound pressure level, in dB SPL, that a
/// full-scale digital signal corresponds to on this microphone at this gain.
/// Unset means uncalibrated, and the readings are then dBFS and labelled as
/// such — which is the honest default, and is enough for every question about
/// change at one place over time.
fn calibration() -> Calibration {
    match std::env::var("BIRDNET_SPL_CALIBRATION_DB")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
    {
        Some(db) if db.is_finite() => Calibration::SplOffsetDb(db),
        _ => Calibration::FullScale,
    }
}

/// Consecutive faulty observations before a source is reported.
///
/// Three, so at [`POLL_EVERY`] a source must be broken for a quarter of an hour
/// before anyone is told. One odd segment is not a fault — a capture process
/// restarting mid-write can produce a short run of zeros — and an alert that
/// fires on those is one the operator learns to skip past.
const FAULT_TICKS_BEFORE_ALERT: u32 = 3;

/// Something worth telling the operator about one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FaultEvent {
    /// A sustained fault, reported once for the episode.
    Onset {
        /// The source label.
        source: String,
        /// What is wrong with it.
        fault: birdnet_core::audio::quality::stream_fault::StreamFault,
    },
    /// A source that had been reported is producing usable audio again.
    Recovered {
        /// The source label.
        source: String,
    },
}

/// Per-source episode tracking for stream faults.
///
/// Separated from the polling loop, and given the verdict rather than the
/// audio, so the episode rules — how long before reporting, one report per
/// episode, a recovery notice — are testable without decoding a file or
/// waiting a quarter of an hour.
#[derive(Debug, Default)]
pub(super) struct FaultWatch {
    /// Source → (the fault seen, how many consecutive ticks it has held).
    running: std::collections::HashMap<
        String,
        (birdnet_core::audio::quality::stream_fault::StreamFault, u32),
    >,
    /// Sources already reported, so an episode is announced once.
    reported: std::collections::HashSet<String>,
}

impl FaultWatch {
    /// Fold in one observation and report what, if anything, changed.
    ///
    /// `fault` of `None` means the segment was usable *or* unjudgeable; the
    /// caller does not distinguish them, because a source whose segments
    /// cannot be read is the stall detector's problem and reporting it here
    /// too would double every alert.
    pub(super) fn observe(
        &mut self,
        source: &str,
        fault: Option<birdnet_core::audio::quality::stream_fault::StreamFault>,
    ) -> Option<FaultEvent> {
        let Some(fault) = fault else {
            self.running.remove(source);
            // Only announce recovery to someone who was told about the fault.
            return self.reported.remove(source).then(|| FaultEvent::Recovered {
                source: source.to_owned(),
            });
        };

        let entry = self.running.entry(source.to_owned()).or_insert((fault, 0));
        if entry.0 == fault {
            entry.1 = entry.1.saturating_add(1);
        } else {
            // A *different* fault restarts the count: the input changed
            // character, and reporting the new one on the old one's tally
            // would name a fault that has only just appeared as sustained.
            *entry = (fault, 1);
        }
        let held = entry.1;

        if held >= FAULT_TICKS_BEFORE_ALERT && self.reported.insert(source.to_owned()) {
            return Some(FaultEvent::Onset {
                source: source.to_owned(),
                fault,
            });
        }
        None
    }
}

/// Decode `path` and judge whether the input is connected at all.
fn stream_fault(path: &Path) -> Option<birdnet_core::audio::quality::stream_fault::StreamFault> {
    let audio = birdnet_core::audio::decode::decode_file_capped(path, MAX_SAMPLES).ok()?;
    birdnet_core::audio::quality::stream_fault::assess_stream(&audio.samples)
}

/// What one poll produced.
pub(super) struct SampleOutcome {
    /// Observations stored, for the log and for tests.
    pub(super) recorded: usize,
    /// Band-level observations stored.
    pub(super) bands_recorded: usize,
    /// Fault episodes that changed state this tick.
    pub(super) events: Vec<FaultEvent>,
}

/// Take one observation per source.
fn sample_once(state: &AppState, stream_dir: &Path, watch: &mut FaultWatch) -> SampleOutcome {
    let (date, hour) = local_date_hour();
    let mut out = SampleOutcome {
        recorded: 0,
        bands_recorded: 0,
        events: Vec::new(),
    };
    for (source, path) in newest_segment_per_source(stream_dir) {
        // Judged before the quality assessment, and independently of whether
        // it succeeds: a digitally silent segment still produces a perfectly
        // valid `LevelSample` (a floor of -inf clamps to the minimum), so
        // hanging this off `assess` would miss the case it exists for.
        if let Some(event) = watch.observe(&source, stream_fault(&path)) {
            out.events.push(event);
        }
        let Some(assessment) = assess(&path) else {
            tracing::debug!(path = %path.display(), "acoustic sample skipped");
            continue;
        };
        match state.with_db(|conn| record_sample(conn, &date, hour, &source, assessment.level)) {
            Ok(()) => out.recorded += 1,
            Err(e) => tracing::debug!(error = %e, source = %source, "acoustic sample not stored"),
        }
        // Stored separately, and a failure here does not cost the quality
        // observation above: the band levels are the newer and more speculative
        // of the two measurements, and the fault detection that keeps a sealed
        // station alive must not depend on them.
        if let Some(broadband) = assessment.broadband
            && !assessment.bands.is_empty()
        {
            match state.with_db(|conn| {
                record_observation(conn, &date, hour, &source, &assessment.bands, broadband)
            }) {
                Ok(()) => out.bands_recorded += 1,
                Err(e) => {
                    tracing::debug!(error = %e, source = %source, "band levels not stored");
                }
            }
        }
    }
    out
}

/// Log a fault event and, when a notifier is configured, send it.
///
/// One message per episode, with a recovery notice — the same discipline as
/// [`super::deadman`]. A notifier that re-fired every five minutes while the
/// operator slept would train them to ignore it, which costs more than the
/// alert is worth.
async fn report_fault(
    apprise: Option<&crate::integrations::apprise::AppriseHandle>,
    event: FaultEvent,
) {
    let (title, body, kind) = match &event {
        FaultEvent::Onset { source, fault } => {
            tracing::warn!(
                source = %source,
                fault = ?fault,
                "audio source is producing unusable audio — {}",
                fault.label()
            );
            (
                format!("Audio problem on {source}"),
                format!(
                    "The {source} input is running and delivering segments on time, but they                      contain {}. Detections from this source are unreliable until it is fixed.",
                    fault.label()
                ),
                birdnet_integrations::apprise::NotifyType::Warning,
            )
        }
        FaultEvent::Recovered { source } => {
            tracing::info!(source = %source, "audio source is producing usable audio again");
            (
                format!("Audio recovered on {source}"),
                format!("The {source} input is delivering usable audio again."),
                birdnet_integrations::apprise::NotifyType::Info,
            )
        }
    };

    let Some(handle) = apprise else { return };
    if let Err(e) = handle
        .lock()
        .await
        .send_notification(&title, &body, kind)
        .await
    {
        tracing::warn!(error = %e, "stream-fault notification failed to send");
    }
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
///
/// `apprise` carries the stream-fault alerts. `None` leaves the WARN log as the
/// only signal, which is the same bargain [`super::deadman`] makes.
pub fn spawn_acoustic_health(
    state: AppState,
    stream_dir: PathBuf,
    apprise: Option<crate::integrations::apprise::AppriseHandle>,
) {
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
        let mut watch = FaultWatch::default();
        loop {
            tick.tick().await;
            let probe = state.clone();
            let dir = stream_dir.clone();
            let (outcome, returned) = match tokio::task::spawn_blocking(move || {
                let mut w = watch;
                let outcome = sample_once(&probe, &dir, &mut w);
                (outcome, w)
            })
            .await
            {
                Ok(v) => v,
                // The blocking task panicked. Start the watch afresh
                // rather than losing the loop: a lost episode is a
                // duplicate alert at worst, and stopping the sampler
                // would silently end every measurement this task makes.
                Err(e) => {
                    tracing::warn!(error = %e, "acoustic sample task failed");
                    (
                        SampleOutcome {
                            recorded: 0,
                            bands_recorded: 0,
                            events: Vec::new(),
                        },
                        FaultWatch::default(),
                    )
                }
            };
            watch = returned;
            for event in outcome.events {
                report_fault(apprise.as_ref(), event).await;
            }

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

    /// Write a one-second tone segment, for the band-level tests.
    fn write_tone_segment(dir: &Path, name: &str, hz: f32, amplitude: f32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut w = WavWriter::create(dir.join(name), spec).expect("create wav");
        for i in 0..48_000_u32 {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / 48_000.0;
            let v = amplitude * (std::f32::consts::TAU * hz * t).sin();
            #[allow(clippy::cast_possible_truncation)]
            w.write_sample((v * f32::from(i16::MAX)) as i16)
                .expect("write sample");
        }
        w.finalize().expect("finalize");
    }

    /// The band levels come out of the same decode as the quality figures, and
    /// they describe the audio rather than being a constant.
    #[test]
    fn a_segment_yields_band_levels_that_follow_its_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_tone_segment(dir.path(), "2026-08-20-birdnet-06:00:00.wav", 1000.0, 0.5);

        let a = assess(&dir.path().join("2026-08-20-birdnet-06:00:00.wav")).expect("assessed");
        assert!(
            !a.bands.is_empty(),
            "a one-second 48 kHz segment must close one meter interval"
        );
        assert!(a.broadband.is_some());

        let loudest = a
            .bands
            .iter()
            .max_by(|x, y| x.mean_db.total_cmp(&y.mean_db))
            .expect("bands");
        assert!(
            (loudest.band_hz - 1000.0).abs() < 0.01,
            "a 1 kHz tone should peak in the 1 kHz band, not the {} Hz one",
            loudest.band_hz
        );
    }

    /// A segment too short to close a one-second window yields no bands, and
    /// the quality figures still land.
    ///
    /// The counterpart to the test above: without it, "bands is non-empty"
    /// could be satisfied by emitting a band list of floors for anything.
    #[test]
    fn a_sub_second_segment_yields_no_bands_but_still_assesses() {
        let (bands, broadband) = measure_bands(&[0.1_f32; 1000], 48_000);
        assert!(
            bands.is_empty() && broadband.is_none(),
            "1000 samples is 21 ms; the meter cannot close a second from it"
        );
        assert!(
            measure_bands(&[0.1_f32; 1000], 0).0.is_empty(),
            "a zero sample rate must not panic or invent a reading"
        );
    }

    /// End to end: a real segment, through the real sampler, into the real
    /// table, read back by the real query.
    ///
    /// Every piece of this exists separately and is tested separately. What
    /// this asserts is that they are *connected* — the failure mode being a
    /// meter that works, a table that works, and nothing calling one from the
    /// other, which every unit test in both files would still pass.
    #[test]
    fn the_sampler_stores_band_levels_the_query_can_read_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_tone_segment(dir.path(), "2026-08-20-birdnet-06:00:00.wav", 1000.0, 0.5);

        let db = tempfile::tempdir().expect("db dir");
        let state = AppState::new(db.path().join("birds.db")).expect("state");
        let mut watch = FaultWatch::default();
        let outcome = sample_once(&state, dir.path(), &mut watch);

        assert_eq!(outcome.recorded, 1, "the quality observation should land");
        assert_eq!(
            outcome.bands_recorded, 1,
            "the band observation should land alongside it"
        );

        let rows = state
            .with_db(|conn| birdnet_db::sound_levels::latest_hour(conn, "local"))
            .expect("read back");
        assert!(
            rows.len() >= 25,
            "a 48 kHz source measures 30 bands; got {}",
            rows.len()
        );
        let peak = rows
            .iter()
            .max_by(|a, b| a.mean_db.total_cmp(&b.mean_db))
            .expect("rows");
        assert!(
            (peak.band_hz - 1000.0).abs() < 0.01,
            "the stored spectrum peaks at {} Hz, not the 1 kHz of the tone written",
            peak.band_hz
        );

        let broadband = state
            .with_db(|conn| birdnet_db::sound_levels::recent_broadband(conn, 24))
            .expect("read broadband");
        assert_eq!(broadband.len(), 1);
        assert!(
            broadband[0].z_weighted_db > broadband[0].a_weighted_db - 3.0,
            "at 1 kHz the A-weighting is 0 dB, so the two broadband figures should be close: \
             A={:.1} Z={:.1}",
            broadband[0].a_weighted_db,
            broadband[0].z_weighted_db
        );
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
            quiet.level.noise_floor_dbfs < loud.level.noise_floor_dbfs - 10.0,
            "a 30x quieter capsule must read far lower: loud={} quiet={}",
            loud.level.noise_floor_dbfs,
            quiet.level.noise_floor_dbfs
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
        let recorded = sample_once(&state, dir.path(), &mut FaultWatch::default()).recorded;
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

    // ── stream-fault episodes ───────────────────────────────────────────

    use birdnet_core::audio::quality::stream_fault::StreamFault;

    /// Feed `n` consecutive observations of `fault` for one source.
    fn feed(
        w: &mut FaultWatch,
        source: &str,
        fault: Option<StreamFault>,
        n: u32,
    ) -> Vec<FaultEvent> {
        (0..n).filter_map(|_| w.observe(source, fault)).collect()
    }

    #[test]
    fn a_sustained_fault_is_reported_once() {
        // Once per episode, not once per poll. A notifier that re-fired every
        // five minutes overnight is one the operator learns to skip past, and
        // then misses the next real thing.
        let mut w = FaultWatch::default();
        let events = feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 20);
        assert_eq!(
            events,
            vec![FaultEvent::Onset {
                source: "RTSP_1".into(),
                fault: StreamFault::DigitallySilent,
            }],
            "the episode was announced more than once"
        );
    }

    #[test]
    fn a_brief_fault_is_not_reported_at_all() {
        // A capture process restarting mid-write can leave a short run of
        // zeros. Alerting on that is the fastest way to make the alert
        // worthless.
        assert_eq!(FAULT_TICKS_BEFORE_ALERT, 3, "the counts below assume this");
        let mut w = FaultWatch::default();
        assert!(feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 2).is_empty());
        // ...and it clears without a recovery notice, because nobody was told.
        assert!(w.observe("RTSP_1", None).is_none());
    }

    #[test]
    fn the_third_consecutive_observation_reports() {
        // Counterpart to the gate above, with a literal count: a watch that
        // never reported would satisfy it and the feature would do nothing.
        let mut w = FaultWatch::default();
        assert!(w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none());
        assert!(w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none());
        assert_eq!(
            w.observe("RTSP_1", Some(StreamFault::Saturated)),
            Some(FaultEvent::Onset {
                source: "RTSP_1".into(),
                fault: StreamFault::Saturated,
            })
        );
    }

    #[test]
    fn an_intermittent_fault_never_accumulates_into_an_alert() {
        // The count is of *consecutive* observations. Without that, a source
        // that is briefly odd once an hour would eventually be reported as
        // sustained — which it is not.
        let mut w = FaultWatch::default();
        for _ in 0..20 {
            assert!(w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none());
            assert!(w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none());
            assert!(w.observe("RTSP_1", None).is_none());
        }
    }

    #[test]
    fn a_recovery_is_announced_only_to_someone_who_was_told() {
        let mut w = FaultWatch::default();
        feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 3);
        assert_eq!(
            w.observe("RTSP_1", None),
            Some(FaultEvent::Recovered {
                source: "RTSP_1".into()
            })
        );
        // ...and not twice.
        assert!(w.observe("RTSP_1", None).is_none());
        // A source that never faulted gets no recovery notice either.
        assert!(w.observe("local", None).is_none());
    }

    #[test]
    fn the_same_source_can_fault_again_after_recovering() {
        // The episode state has to be cleared by the recovery, or a source
        // that breaks, is fixed, and breaks again is silent the second time.
        let mut w = FaultWatch::default();
        feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 3);
        w.observe("RTSP_1", None);
        assert_eq!(
            feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 3),
            vec![FaultEvent::Onset {
                source: "RTSP_1".into(),
                fault: StreamFault::DigitallySilent,
            }]
        );
    }

    #[test]
    fn a_different_fault_restarts_the_count() {
        // Two ticks of silence then saturation is not three ticks of anything.
        // Reporting the new fault on the old one's tally would announce as
        // sustained something that has only just appeared.
        let mut w = FaultWatch::default();
        assert!(
            w.observe("RTSP_1", Some(StreamFault::DigitallySilent))
                .is_none()
        );
        assert!(
            w.observe("RTSP_1", Some(StreamFault::DigitallySilent))
                .is_none()
        );
        assert!(
            w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none(),
            "a fault reported on a different fault's tally"
        );
        assert!(w.observe("RTSP_1", Some(StreamFault::Saturated)).is_none());
        assert_eq!(
            w.observe("RTSP_1", Some(StreamFault::Saturated)),
            Some(FaultEvent::Onset {
                source: "RTSP_1".into(),
                fault: StreamFault::Saturated,
            })
        );
    }

    #[test]
    fn sources_are_tracked_independently() {
        // A four-camera station with one dead stream is exactly the case the
        // detection deadman cannot see, because the other three keep the
        // station detecting. Sharing a counter here would lose it again.
        let mut w = FaultWatch::default();
        for _ in 0..3 {
            assert!(w.observe("local", None).is_none());
        }
        let events = feed(&mut w, "RTSP_1", Some(StreamFault::DigitallySilent), 3);
        assert_eq!(events.len(), 1, "a healthy source masked a broken one");
        assert!(
            w.observe("local", Some(StreamFault::Saturated)).is_none(),
            "one source's episode leaked into another's count"
        );
    }
}
