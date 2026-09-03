//! Event-processing decision helpers: the disposition gate, the per-event
//! arithmetic (confidence %, latency), and the boolean notification gates.
//!
//! All pure, all extracted from `event_processor` so the boundaries the
//! mutation gate cares about are unit-testable without standing up a
//! database, broadcast channel, or notification stack.

use std::collections::HashMap;

use birdnet_core::detection::dynamic_threshold::DynamicThresholds;

/// Truncating cast of a probability in `[0, 1]` to a 0–100 percentage.
///
/// Matches the historical behaviour of the inline `(confidence * 100.0)
/// as u32` in the notification-context builder. Extracted so the
/// `*` arithmetic mutant has a unit-testable surface.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub(super) fn confidence_pct_trunc(confidence: f32) -> u32 {
    (confidence * 100.0) as u32
}

/// Rounding cast of a probability in `[0, 1]` to a 0–100 percentage.
///
/// Matches the historical behaviour of the inline `(confidence *
/// 100.0).round() as u32` in the MQTT payload builder. Extracted for
/// the same reason as [`confidence_pct_trunc`].
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub(super) fn confidence_pct_round(confidence: f32) -> u32 {
    (confidence * 100.0).round() as u32
}

/// Convert daemon-reported per-event latency (ms) to seconds.
///
/// Extracted so the `/ 1000.0` arithmetic mutant has a unit-testable
/// surface independent of the rest of `event_processor`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub(super) fn latency_ms_to_seconds(latency_ms: u64) -> f64 {
    (latency_ms as f64) / 1000.0
}

/// "Is the row we just inserted the only one for this species today?"
///
/// Used to power the rare-species celebration in the dashboard. Takes the
/// count returned by `detection_count_for_species_date` *after* the
/// current row's insert; `<= 1` covers both "exactly the row we just
/// inserted" and the defensive `0` case where the query failed and the
/// caller fell back to a sentinel. Extracted from `event_processor`
/// because the inline `<= 1` had a `<=` → `>` mutant with no covering
/// test.
#[must_use]
pub const fn is_first_detection_today(count_after_insert: i64) -> bool {
    count_after_insert <= 1
}

/// Combine the "Suppress alert-rule fired?" gate with the trigger-mode
/// filter into a single notification-eligibility verdict.
///
/// Returns `true` only when no Suppress rule matched *and* the trigger
/// filter says notify. Extracted from `event_processor` to make the
/// `!rule_suppressed && filter_says_notify` chain observable in a
/// unit test — the inline form produced `&&` → `||` and `delete !`
/// cargo-mutants that no test could catch without standing up the
/// integration stack.
#[must_use]
pub const fn passes_filter(rule_suppressed: bool, filter_says_notify: bool) -> bool {
    !rule_suppressed && filter_says_notify
}

/// Combine the upstream `passes_filter` verdict with an integration's
/// own send-policy verdict into a final dispatch decision.
///
/// Apprise (and historically other integrations) has its own
/// per-species / confidence-threshold gate that fires *after* the
/// global filter. Pulled into a pure helper for the same reason as
/// [`passes_filter`].
#[must_use]
pub const fn should_dispatch_notification(
    passes_filter: bool,
    integration_says_notify: bool,
) -> bool {
    passes_filter && integration_says_notify
}

/// What to do with a detection after the threshold gates run.
///
/// Extracted from the inline gate logic in `event_processor` so the
/// dispatch decision is unit-testable without spinning up a database,
/// broadcast channel, or notification stack. The actual side effects
/// (DB insert, quarantine row, audio extraction, broadcasts) still live
/// in the processor — this enum just pins the decision boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DispositionDecision {
    /// Detection passes all gates — persist, extract clip, broadcast.
    Accept,
    /// Below an operator-set per-species threshold — quarantine for review.
    Quarantine {
        /// The threshold that gated this detection, for the quarantine row.
        threshold: f64,
    },
    /// Below the global threshold and no per-species override — drop silently.
    DropBelowGlobal,
}

/// Pure helper: decide what to do with a detection based on its confidence
/// and the configured thresholds.
///
/// * A per-species threshold (if present) wins over the global threshold
///   and triggers quarantine (not silent drop) when missed — this preserves
///   the detection for manual review.
/// * Without a per-species override, the global confidence threshold is
///   the gate. Detections below it are dropped silently because the model
///   already applied the same gate; this is a belt-and-braces check.
///
/// `dynamic` may lower — never raise — whichever of those two the detection is
/// being judged against, for a species the station has already confirmed
/// present. Passing `None` is the same as passing a disabled tracker, and is
/// what every caller that does not have one should do rather than constructing
/// an empty one, so "no adjustment configured" and "adjustment configured but
/// this species has none" stay distinguishable at the call site.
///
/// Note that the quarantine boundary moves with the adjustment: a species with
/// an operator-set threshold of 0.9 that the station has confirmed is judged
/// against the lowered figure, so the same detection can be accepted here that
/// would have been quarantined an hour ago. That is the intended behaviour —
/// the operator's number says how sure to be about a species in general, and
/// the adjustment says this one is known to be here — but it is the part a
/// reader is most likely to be surprised by, so it is stated rather than left
/// to be discovered.
///
/// Comparisons are done in `f64` because per-species thresholds come from
/// SQLite REAL columns (f64) and we don't want a single-precision rounding
/// step to flip a `==`-on-boundary case.
pub(super) fn decide_disposition(
    confidence: f32,
    sci_name: &str,
    per_species_thresholds: &HashMap<String, f64>,
    global_confidence: f32,
    dynamic: Option<&DynamicThresholds>,
    now_ms: i64,
) -> DispositionDecision {
    // The adjustment is applied in `f64` and to the *stored* value, not to an
    // `f32` round-trip of it. Taking the threshold through `f32` and back turns
    // a stored 0.8 into 0.800000011920929, which then appears on the quarantine
    // row an operator reads — the same single-precision rounding this
    // function's `f64` comparison exists to avoid.
    let adjustment = dynamic.and_then(|d| d.adjustment(sci_name, now_ms));
    let adjust = |base: f64| -> f64 { adjustment.map_or(base, |a| a.apply(base)) };

    if let Some(&threshold) = per_species_thresholds.get(sci_name) {
        let effective = adjust(threshold);
        if f64::from(confidence) < effective {
            return DispositionDecision::Quarantine {
                threshold: effective,
            };
        }
        return DispositionDecision::Accept;
    }
    if f64::from(confidence) < adjust(f64::from(global_confidence)) {
        return DispositionDecision::DropBelowGlobal;
    }
    DispositionDecision::Accept
}

/// Wall-clock milliseconds since the Unix epoch.
///
/// The dynamic-threshold tracker takes time as a parameter so its rules are
/// testable without a clock; this is the one place the real clock is read, so
/// there is one answer per event rather than a fresh `now` at each comparison.
///
/// A clock before the epoch is not a case worth branching on — it means the
/// RTC has failed, and every date in the database is already wrong — so it
/// saturates at zero, which makes every learned level look lapsed and turns
/// the feature off rather than producing arbitrary leases.
#[must_use]
pub(super) fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Derive a stable audio-source label from a recording's filename.
///
/// Recording filenames follow `YYYY-MM-DD-birdnet[-RTSP_ID]-HH:MM:SS.wav`.
/// The optional `RTSP_ID` segment (`RTSP_1`, `RTSP_2`, …) names the
/// per-stream supervisor that produced the file; its absence means the
/// file came from the local microphone (ALSA / PulseAudio / PipeWire).
/// We collapse all microphone sources to a single `local` label because
/// the supervisor doesn't currently expose finer-grained per-mic IDs.
///
/// Used to populate the `birdnet_audio_source_up{source}` gauge as a
/// best-effort liveness signal. A proper supervisor → metrics path is
/// the right long-term fix, but a per-event freshness gauge is already
/// useful: stations with one source going dark while another stays up
/// show that immediately in Prometheus.
#[must_use]
pub fn derive_source_label(source_file: &std::path::Path) -> String {
    let name = source_file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    birdnet_core::detection::types::RecordingFile::parse(name)
        .and_then(|rf| rf.rtsp_id)
        .unwrap_or_else(|| "local".to_owned())
}

#[cfg(test)]
mod tests {

    /// `epoch_ms` reads the real clock.
    ///
    /// It had no test at all — it is called only from `processor.rs`, and
    /// nothing asserted on its value — so cargo-mutants could replace its whole
    /// body with `0`, `1` or `-1` and the suite stayed green. A constant clock
    /// makes every learned dynamic threshold look either permanently fresh or
    /// permanently lapsed, which is the feature silently not working.
    ///
    /// Bounded rather than pinned, since the value moves: after 2024 and before
    /// 2100. That kills all three constants without inventing a fake clock —
    /// this function exists precisely because everything else takes time as a
    /// parameter, so there is nothing here to inject.
    #[test]
    fn epoch_ms_returns_a_real_clock_reading() {
        const Y2024_MS: i64 = 1_704_067_200_000;
        const Y2100_MS: i64 = 4_102_444_800_000;
        let now = epoch_ms();
        assert!(
            (Y2024_MS..Y2100_MS).contains(&now),
            "epoch_ms must return a plausible wall-clock time in milliseconds, got {now}"
        );
    }

    /// ...and it moves forward. The bound above is wide enough that a clock
    /// frozen at any single plausible instant would pass it; this is the
    /// counterpart that says the reading is taken each call.
    #[test]
    fn epoch_ms_advances() {
        let first = epoch_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = epoch_ms();
        assert!(
            second > first,
            "two readings 5 ms apart must differ: {first} then {second}"
        );
    }
    use super::*;
    use crate::daemon::test_support::thresholds;

    // ── dynamic thresholds ──────────────────────────────────────────────

    /// A tracker that has confirmed `sci_name` once.
    fn confirmed(sci_name: &str) -> DynamicThresholds {
        use birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig;
        let mut d = DynamicThresholds::new(DynamicThresholdConfig {
            enabled: true,
            trigger: 0.80,
            min: 0.10,
            valid_hours: 24,
        });
        d.observe(sci_name, 0.95, 0);
        d
    }

    /// A confirmed species is accepted at a confidence that would otherwise
    /// have been dropped.
    #[test]
    fn a_confirmed_species_is_accepted_below_the_global_threshold() {
        let d = confirmed("Strix aluco");
        // 0.60 is under the 0.70 global but over 0.70 * 0.75 = 0.525.
        let got = decide_disposition(0.60, "Strix aluco", &thresholds(&[]), 0.70, Some(&d), 1000);
        assert_eq!(got, DispositionDecision::Accept);
    }

    /// And an unconfirmed species at the same confidence is still dropped.
    ///
    /// The counterpart, and the one that matters: an adjustment applied to
    /// everything would pass the test above and would be a global threshold
    /// cut wearing a disguise.
    #[test]
    fn an_unconfirmed_species_is_still_dropped_at_the_same_confidence() {
        let d = confirmed("Strix aluco");
        let got = decide_disposition(
            0.60,
            "Turdus merula",
            &thresholds(&[]),
            0.70,
            Some(&d),
            1000,
        );
        assert_eq!(got, DispositionDecision::DropBelowGlobal);
    }

    /// Passing no tracker leaves every decision exactly as it was.
    #[test]
    fn without_a_tracker_the_decision_is_unchanged() {
        assert_eq!(
            decide_disposition(0.60, "Strix aluco", &thresholds(&[]), 0.70, None, 1000),
            DispositionDecision::DropBelowGlobal
        );
    }

    /// The adjustment moves the per-species quarantine boundary too, and the
    /// threshold reported on the quarantine row is the one actually applied.
    ///
    /// A quarantine row recording the configured threshold while the pipeline
    /// judged against a different one would make the review queue unreadable:
    /// the operator sees "below 0.90" on a detection that was in fact measured
    /// against 0.675.
    #[test]
    fn a_quarantine_reports_the_threshold_that_was_actually_applied() {
        let d = confirmed("Strix aluco");
        let t = thresholds(&[("Strix aluco", 0.90)]);

        // 0.70 clears 0.90 * 0.75 = 0.675, so it is now accepted.
        assert_eq!(
            decide_disposition(0.70, "Strix aluco", &t, 0.50, Some(&d), 1000),
            DispositionDecision::Accept
        );

        // 0.60 does not, and the row must say 0.675 rather than 0.90.
        match decide_disposition(0.60, "Strix aluco", &t, 0.50, Some(&d), 1000) {
            DispositionDecision::Quarantine { threshold } => assert!(
                (threshold - 0.675).abs() < 1e-4,
                "quarantine recorded {threshold}, but the detection was judged against 0.675"
            ),
            other => panic!("expected a quarantine, got {other:?}"),
        }
    }

    /// A lapsed confirmation restores the original decision.
    #[test]
    fn a_lapsed_confirmation_restores_the_original_decision() {
        let d = confirmed("Strix aluco");
        let past_lease = 25 * 3_600_000;
        assert_eq!(
            decide_disposition(
                0.60,
                "Strix aluco",
                &thresholds(&[]),
                0.70,
                Some(&d),
                past_lease
            ),
            DispositionDecision::DropBelowGlobal
        );
    }

    #[test]
    fn no_per_species_threshold_accepts_above_global() {
        // global=0.5, detection=0.7 → accept (no per-species).
        let d = decide_disposition(0.7, "Pica pica", &thresholds(&[]), 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn no_per_species_threshold_drops_below_global() {
        // global=0.5, detection=0.4 → drop (no per-species; model already
        // gated, but the double-check fires here too).
        let d = decide_disposition(0.4, "Pica pica", &thresholds(&[]), 0.5, None, 0);
        assert_eq!(d, DispositionDecision::DropBelowGlobal);
    }

    #[test]
    fn per_species_threshold_accepts_when_met() {
        // per-species=0.8, detection=0.85 → accept.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.85, "Pica pica", &t, 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn per_species_threshold_quarantines_when_missed() {
        // per-species=0.8, detection=0.6 → quarantine, not drop.
        // The whole point of the quarantine workflow is to keep these
        // detections around for review rather than silently dropping them.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.6, "Pica pica", &t, 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Quarantine { threshold: 0.8 });
    }

    #[test]
    fn per_species_threshold_overrides_global_when_below_both() {
        // global=0.5, per-species=0.8, detection=0.4 → quarantine (NOT
        // drop). The per-species override wins even when the detection
        // would also have failed the global gate — the operator-configured
        // threshold is the gate that decides quarantine vs. drop.
        let t = thresholds(&[("Pica pica", 0.8)]);
        let d = decide_disposition(0.4, "Pica pica", &t, 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Quarantine { threshold: 0.8 });
    }

    #[test]
    fn per_species_threshold_only_applies_to_named_species() {
        // Threshold for Pica pica; detection for Corvus corax → no override.
        let t = thresholds(&[("Pica pica", 0.95)]);
        let d = decide_disposition(0.6, "Corvus corax", &t, 0.5, None, 0);
        // 0.6 > 0.5 global, no override → accept.
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn boundary_at_threshold_is_accept_for_global() {
        // The check uses `<` so equality passes. Pin the contract.
        let d = decide_disposition(0.5, "Pica pica", &thresholds(&[]), 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn boundary_at_threshold_is_accept_for_per_species() {
        // Use exactly-representable f32 boundary value 0.5 so the
        // `<` → `<=` mutation is observable. The naive choice of 0.8
        // would leave both `<` and `<=` returning false — `0.8_f32`
        // rounds up to ~0.80000001 in f64, while `0.8_f64` is
        // ~0.79999999... so `f64::from(0.8_f32) <= 0.8_f64` is
        // already false. 0.5 is a power of two and round-trips
        // exactly between f32 and f64, so `0.5 < 0.5` is false and
        // `0.5 <= 0.5` is true — the assertion below catches the
        // boundary mutation.
        let t = thresholds(&[("Pica pica", 0.5)]);
        let d = decide_disposition(0.5, "Pica pica", &t, 0.25, None, 0);
        assert_eq!(d, DispositionDecision::Accept);
    }

    #[test]
    fn empty_string_species_is_treated_like_unknown() {
        // Defensive: a malformed detection with no species name should
        // hit the no-override path; whether it accepts or drops depends
        // on confidence.
        let d = decide_disposition(0.9, "", &thresholds(&[]), 0.5, None, 0);
        assert_eq!(d, DispositionDecision::Accept);
        let d2 = decide_disposition(0.1, "", &thresholds(&[]), 0.5, None, 0);
        assert_eq!(d2, DispositionDecision::DropBelowGlobal);
    }

    // ── derive_source_label ────────────────────────────────────────────

    #[test]
    fn source_label_is_local_for_no_rtsp_prefix() {
        let p = std::path::Path::new("/tmp/2026-05-19-birdnet-09:00:00.wav");
        assert_eq!(derive_source_label(p), "local");
    }

    #[test]
    fn source_label_picks_up_rtsp_id() {
        let p = std::path::Path::new("/tmp/2026-05-19-birdnet-RTSP_1-09:00:00.wav");
        assert_eq!(derive_source_label(p), "RTSP_1");
        let p2 = std::path::Path::new("/tmp/2026-05-19-birdnet-RTSP_42-12:34:56.flac");
        assert_eq!(derive_source_label(p2), "RTSP_42");
    }

    #[test]
    fn source_label_reads_the_installer_seeded_source_id() {
        // The shape a bare-metal station actually produces: `seed_sources_from_config`
        // names ALSA rows `src_seed_N`, and capture writes
        // `%Y-%m-%d-birdnet-src_seed_1-%H:%M:%S.wav` through `arecord --use-strftime`.
        // The live spectrogram attributes its frames with this, and the label has
        // to equal the `audio_sources.id` the source picker offers or the picker
        // filters every frame away.
        let p =
            std::path::Path::new("/tmp/birdnet-stream/2026-08-12-birdnet-src_seed_1-11:30:16.wav");
        assert_eq!(derive_source_label(p), "src_seed_1");
    }

    /// The identity round-trip the source picker depends on, closed against the
    /// **real** formatter rather than a hand-typed string.
    ///
    /// Capture registers a source — its health gauge, its live-audio tap, and
    /// the `-birdnet-<id>-` field of every filename it writes — under
    /// `CaptureSource::label()`. The detection side recovers a label from the
    /// filename with `derive_source_label`. If those two ever disagree, the
    /// per-source picker filters every frame away and the station silently
    /// stops attributing detections, which is precisely the failure the live
    /// spectrogram hit in 0.12.0.
    #[test]
    fn source_label_round_trips_through_a_real_capture_filename() {
        use birdnet_core::audio::capture::{AudioFormat, CaptureSource, recording_filename_at};
        use birdnet_core::civil::civil_from_unix_secs;

        // 2026-08-12 11:30:16 local.
        let at = civil_from_unix_secs(1_786_534_216);

        for stream_id in [Some("src_seed_1"), Some("RTSP_1"), None] {
            let source = CaptureSource::Microphone {
                device: "plughw:CARD=PRO,DEV=0".into(),
                sample_rate: 48_000,
                channels: 1,
                channel_pick: None,
                stream_id: stream_id.map(ToOwned::to_owned),
            };
            let name = recording_filename_at(stream_id, AudioFormat::Wav, at);
            let path = std::path::PathBuf::from("/tmp/birdnet-stream").join(&name);
            assert_eq!(
                derive_source_label(&path),
                source.label(),
                "capture writes {name} for a source it labels {}",
                source.label()
            );
        }
    }

    #[test]
    fn source_label_falls_back_to_local_on_unparseable_filename() {
        // Filename that doesn't match the canonical schema.
        let p = std::path::Path::new("/tmp/random-file.wav");
        assert_eq!(derive_source_label(p), "local");
        let p2 = std::path::Path::new("/tmp/");
        assert_eq!(derive_source_label(p2), "local");
    }

    // ── confidence_pct_trunc / confidence_pct_round ────────────────────
    //
    // Both helpers wrap an arithmetic mutant on `*`. The tests below pin
    // a value that catches `*` → `+` (would produce 100.7) and `*` → `/`
    // (would produce ~0.0095).

    #[test]
    fn confidence_pct_trunc_basic() {
        assert_eq!(confidence_pct_trunc(0.0), 0);
        assert_eq!(confidence_pct_trunc(0.5), 50);
        assert_eq!(confidence_pct_trunc(0.954), 95);
        assert_eq!(confidence_pct_trunc(1.0), 100);
    }

    #[test]
    fn confidence_pct_trunc_truncates_not_rounds() {
        // 0.999 * 100 = 99.9 → truncates to 99 (round would give 100).
        // Pinning this catches an accidental swap to round semantics.
        assert_eq!(confidence_pct_trunc(0.999), 99);
    }

    #[test]
    fn confidence_pct_round_basic() {
        assert_eq!(confidence_pct_round(0.0), 0);
        assert_eq!(confidence_pct_round(0.5), 50);
        assert_eq!(confidence_pct_round(0.954), 95);
        assert_eq!(confidence_pct_round(1.0), 100);
    }

    #[test]
    fn confidence_pct_round_rounds_not_truncates() {
        // 0.999 * 100 = 99.9 → rounds to 100. Pins the round semantic.
        assert_eq!(confidence_pct_round(0.999), 100);
        // 0.955 rounds to 96 (vs trunc 95): distinguishes the two helpers
        // and catches a `*` arithmetic mutation that would skew the value.
        assert_eq!(confidence_pct_round(0.955), 96);
    }

    // ── latency_ms_to_seconds ───────────────────────────────────────────

    #[test]
    fn latency_ms_to_seconds_basic() {
        assert!((latency_ms_to_seconds(0) - 0.0).abs() < 1e-9);
        assert!((latency_ms_to_seconds(1_000) - 1.0).abs() < 1e-9);
        assert!((latency_ms_to_seconds(250) - 0.25).abs() < 1e-9);
        assert!((latency_ms_to_seconds(1_500) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn latency_ms_to_seconds_division_distinct_from_modulo() {
        // 1500 ms = 1.5 s. The `/ 1000.0` → `% 1000.0` mutant would
        // produce 500.0; the `/ 1000.0` → `* 1000.0` mutant would
        // produce 1_500_000. Either is caught by the previous test;
        // pin a non-round case here to double-cover.
        let v = latency_ms_to_seconds(2_750);
        assert!((v - 2.75).abs() < 1e-9, "got {v}");
    }

    // ── is_first_detection_today ────────────────────────────────────────

    #[test]
    fn is_first_detection_today_boundary() {
        // The `<=` boundary: 0 and 1 are "first", 2 is not. Pins both sides.
        assert!(is_first_detection_today(0));
        assert!(is_first_detection_today(1));
        assert!(!is_first_detection_today(2));
        assert!(!is_first_detection_today(100));
    }

    #[test]
    fn is_first_detection_today_handles_negative_defensively() {
        // The query returns i64 and could in principle be negative if the
        // upstream sentinel changes. <= 1 still classifies as "first".
        assert!(is_first_detection_today(-1));
    }

    // ── passes_filter ───────────────────────────────────────────────────
    //
    // Four-cell truth table for `!suppressed && filter`. The mutations
    // cargo-mutants generates on the inline form are:
    //   - `&&` → `||`: changes (T,F) and (F,T) results
    //   - `delete !`: changes (T,T) and (F,F)
    //   - replace body with `true`/`false`

    #[test]
    fn passes_filter_truth_table() {
        // (suppressed, filter) → expected
        assert!(passes_filter(false, true)); // green light
        assert!(!passes_filter(false, false)); // filter says no
        assert!(!passes_filter(true, true)); // rule says suppress
        assert!(!passes_filter(true, false)); // both negative
    }

    // ── should_dispatch_notification ────────────────────────────────────

    #[test]
    fn should_dispatch_notification_truth_table() {
        // (dispatch_allowed, integration_says_notify) → expected
        assert!(should_dispatch_notification(true, true));
        assert!(!should_dispatch_notification(true, false));
        assert!(!should_dispatch_notification(false, true));
        assert!(!should_dispatch_notification(false, false));
    }
}
