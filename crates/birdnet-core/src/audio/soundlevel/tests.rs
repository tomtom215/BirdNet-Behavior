//! Gates for the sound level meter.
//!
//! The measurements here are checked against things outside this codebase —
//! the standard's published A-weighting table, the closed-form level of a sine
//! wave, the analytic magnitude response of the filter — rather than against
//! whatever the code happens to produce. A DSP test that only asserts "it ran
//! and gave a number" is the easiest kind to write and the least useful: every
//! wrong implementation passes it.

use super::*;

/// Generate `secs` seconds of a sine at `hz` and `amplitude`.
fn sine(hz: f32, amplitude: f32, sample_rate: u32, secs: u32) -> Vec<f32> {
    let n = (sample_rate * secs) as usize;
    (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / sample_rate as f32;
            amplitude * (std::f32::consts::TAU * hz * t).sin()
        })
        .collect()
}

fn band(reading: &SoundLevelReading, centre_hz: f32) -> BandLevel {
    *reading
        .bands
        .iter()
        .find(|b| (b.centre_hz - centre_hz).abs() < 0.01)
        .unwrap_or_else(|| panic!("no band at {centre_hz} Hz in the reading"))
}

// ---------------------------------------------------------------------------
// The weighting curve, against the standard rather than against ourselves
// ---------------------------------------------------------------------------

/// IEC 61672-1 table 3, the A-weighting at each nominal third-octave band.
const PUBLISHED_A_WEIGHTING_DB: [(f32, f32); 30] = [
    (25.0, -44.7),
    (31.5, -39.4),
    (40.0, -34.6),
    (50.0, -30.2),
    (63.0, -26.2),
    (80.0, -22.5),
    (100.0, -19.1),
    (125.0, -16.1),
    (160.0, -13.4),
    (200.0, -10.9),
    (250.0, -8.6),
    (315.0, -6.6),
    (400.0, -4.8),
    (500.0, -3.2),
    (630.0, -1.9),
    (800.0, -0.8),
    (1000.0, 0.0),
    (1250.0, 0.6),
    (1600.0, 1.0),
    (2000.0, 1.2),
    (2500.0, 1.3),
    (3150.0, 1.2),
    (4000.0, 1.0),
    (5000.0, 0.5),
    (6300.0, -0.1),
    (8000.0, -1.1),
    (10_000.0, -2.5),
    (12_500.0, -4.3),
    (16_000.0, -6.6),
    (20_000.0, -9.3),
];

/// The curve must reproduce the standard's own table — and it must do so at
/// the *exact* centre frequencies, not the rounded labels.
///
/// Both halves are asserted, because the second is the whole reason
/// [`exact_centre_hz`] exists. Evaluating at the labels is off by up to
/// 0.157 dB, which would look like a rounding artefact and is in fact the
/// wrong frequency; evaluating at the exact centres lands inside the table's
/// own 0.05 dB rounding.
#[test]
fn a_weighting_matches_the_published_table() {
    let mut worst_exact: f32 = 0.0;
    let mut worst_label: f32 = 0.0;

    for (index, (label_hz, published)) in PUBLISHED_A_WEIGHTING_DB.into_iter().enumerate() {
        assert!(
            (CENTRE_FREQUENCIES_HZ[index] - label_hz).abs() < 0.01,
            "the published table and CENTRE_FREQUENCIES_HZ disagree at index {index}: \
             {label_hz} vs {}",
            CENTRE_FREQUENCIES_HZ[index]
        );
        worst_exact = worst_exact.max((a_weighting_db(exact_centre_hz(index)) - published).abs());
        worst_label = worst_label.max((a_weighting_db(label_hz) - published).abs());
    }

    assert!(
        worst_exact <= 0.06,
        "A-weighting at the exact centres deviates from IEC 61672-1 table 3 by {worst_exact:.3} dB; \
         the table is rounded to 0.1 dB so anything above ~0.05 means the formula's constants \
         are wrong"
    );
    assert!(
        worst_label > 0.10,
        "evaluating at the rounded labels deviates by only {worst_label:.3} dB, so exact_centre_hz \
         buys nothing and this test no longer discriminates. Either the label table changed or \
         someone made exact_centre_hz return the labels."
    );
}

/// The label table and the base-10 series must name the same bands.
///
/// `exact_centre_hz` is an arithmetic series and `CENTRE_FREQUENCIES_HZ` is a
/// hand-written list; nothing but this connects them, and an off-by-one in
/// `LOWEST_EXPONENT` would silently shift every filter by a third of an octave
/// while every other test still passed.
#[test]
fn every_label_names_its_exact_band() {
    for (index, label) in CENTRE_FREQUENCIES_HZ.into_iter().enumerate() {
        let exact = exact_centre_hz(index);
        let ratio = exact / label;
        assert!(
            (0.97..1.03).contains(&ratio),
            "band {index}: label {label} Hz but exact centre {exact:.3} Hz (ratio {ratio:.4}); \
             the preferred number must be within a couple of percent of the band it names"
        );
    }
}

// ---------------------------------------------------------------------------
// The filter, against its own analytic response
// ---------------------------------------------------------------------------

/// A third-octave bandpass must pass its centre at unity, be 3 dB down at its
/// edges, and reject an octave away.
///
/// Checked from the coefficients rather than by running audio through it, so a
/// failure points at the design and not at the accumulation.
#[test]
fn the_band_filter_is_centred_and_three_db_down_at_its_edges() {
    let fs = 48_000;
    for centre in [63.0_f32, 500.0, 1000.0, 4000.0, 10_000.0] {
        let f = ThirdOctaveBand::new(centre, fs).expect("designs");
        let (lo, hi) = band_edges(centre);

        let peak = f.magnitude_at(centre, fs);
        assert!(
            (peak - 1.0).abs() < 0.02,
            "{centre} Hz: peak gain {peak:.4}, expected 1.0 (0 dB). A peak of about {:.1} would \
             mean the constant-Q rather than the constant-peak-gain bandpass form, which puts \
             every band too loud by a band-dependent amount.",
            third_octave_q(centre)
        );

        for edge in <[f32; 2]>::from((lo, hi)) {
            let g = 20.0 * f.magnitude_at(edge, fs).log10();
            assert!(
                (g + 3.01).abs() < 0.25,
                "{centre} Hz: gain at edge {edge:.1} Hz is {g:.2} dB, expected -3.01. The \
                 un-pre-warped alpha = sin(w0)/2Q form reads -4.35 here at 10 kHz, which is a \
                 band measurably narrower than the one it names."
            );
        }
    }
}

/// The `alpha` form matters, and the gate above must be able to tell.
///
/// Without this, the tolerance on the edge test could be loosened to whatever
/// the simpler formula happens to produce and nobody would notice the bands
/// had narrowed. This asserts the *difference*: the un-pre-warped form is
/// visibly wrong at the top of the range and indistinguishable at the bottom,
/// which is exactly why the error survived in the reference implementation.
#[test]
fn the_pre_warped_alpha_is_what_holds_the_edges_at_high_frequencies() {
    let fs = 48_000;
    let bw = section_bandwidth_octaves(1);
    let q = section_q(1);

    let edge_error = |centre: f32| {
        let warped = Biquad::bandpass(centre, fs, bw).expect("designs");
        #[allow(clippy::cast_possible_truncation)]
        let plain = Biquad::bandpass_q(centre, fs, q as f32).expect("designs");
        let (lo, _) = band_edges(centre);
        (
            20.0_f64
                .mul_add(warped.magnitude_at(lo, fs).log10(), 3.01)
                .abs(),
            20.0_f64
                .mul_add(plain.magnitude_at(lo, fs).log10(), 3.01)
                .abs(),
        )
    };

    let (warped_low, plain_low) = edge_error(125.0);
    assert!(
        warped_low < 0.1 && plain_low < 0.1,
        "at 125 Hz both forms should be right: pre-warped off by {warped_low:.2} dB, plain by \
         {plain_low:.2} dB"
    );

    let (warped_high, plain_high) = edge_error(10_000.0);
    // Not zero, and it cannot be: the sinh form pre-warps the *bandwidth*, but
    // the bilinear transform also warps the edge frequencies themselves, and
    // nothing short of designing at pre-warped edges removes that. 0.15 dB at
    // 10 kHz against 1.34 dB is the whole of the improvement being claimed.
    assert!(
        warped_high < 0.2,
        "at 10 kHz the pre-warped form is off by {warped_high:.2} dB, expected under 0.2"
    );
    assert!(
        plain_high > 1.0,
        "at 10 kHz the plain form is off by only {plain_high:.2} dB, so pre-warping buys nothing \
         and this pair of tests no longer discriminates"
    );
    assert!(
        plain_high > warped_high * 5.0,
        "the pre-warped form ({warped_high:.2} dB) is not clearly better than the plain one \
         ({plain_high:.2} dB)"
    );
}

/// The rejection table in `THIRD_OCTAVE_SECTIONS`' doc comment must be true of
/// the filters that actually ship.
///
/// The table is the argument for using three sections rather than one. A doc
/// table nothing checks is decoration, and this repository has shipped
/// confident, wrong prose before.
#[test]
fn the_cascade_rejection_table_is_accurate() {
    let fs = 48_000;
    let centre = 1000.0_f32;
    let f = ThirdOctaveBand::new(centre, fs).expect("designs");
    let at = |ratio: f32| 20.0 * f.magnitude_at(centre * ratio, fs).log10();

    // Row THIRD_OCTAVE_SECTIONS = 3 of the table, to 0.3 dB.
    let one_band = at(2.0_f32.cbrt());
    let two_bands = at(1.6);
    let one_octave = at(2.0);

    assert!(
        (one_band - -9.4).abs() < 0.3,
        "one band away reads {one_band:.2} dB, table says -9.4"
    );
    assert!(
        (two_bands - -22.5).abs() < 0.3,
        "two bands away reads {two_bands:.2} dB, table says -22.5"
    );
    assert!(
        (one_octave - -32.3).abs() < 0.4,
        "one octave away reads {one_octave:.2} dB, table says -32.3"
    );
}

/// A one-section cascade must be the textbook third-octave filter.
///
/// `section_q` is a derivation, and a derivation that cannot reproduce the
/// known case is not to be trusted with the unknown ones. `Q = 4.318` is the
/// number every acoustics text prints for a third-octave band.
#[test]
fn a_single_section_reproduces_the_textbook_third_octave_q() {
    let derived = section_q(1);
    let textbook = f64::from(third_octave_q(1000.0));
    assert!(
        (derived - textbook).abs() < 1e-6,
        "section_q(1) is {derived:.5} but the third-octave Q is {textbook:.5}"
    );
    assert!(
        (derived - 4.318_47).abs() < 1e-4,
        "and neither of them is the published 4.31847, so both are wrong together"
    );
}

/// Every band must design stably at every sample rate a station can produce.
///
/// A single unstable section would grow without bound and take the whole
/// reading with it, and the failure would appear hours into a run.
#[test]
fn every_band_designs_stably_at_every_supported_rate() {
    for rate in [16_000_u32, 22_050, 32_000, 44_100, 48_000, 96_000, 192_000] {
        let meter = SoundLevelMeter::new(rate, 1, Calibration::FullScale)
            .unwrap_or_else(|e| panic!("meter at {rate} Hz: {e}"));
        assert!(
            meter.band_count() > 0,
            "no bands survive at {rate} Hz, so the meter measures nothing"
        );
        for centre in meter.centre_frequencies() {
            #[allow(clippy::cast_precision_loss)]
            let nyquist = rate as f32 / 2.0;
            assert!(
                centre * THIRD_OCTAVE_EDGE_RATIO < nyquist * NYQUIST_MARGIN,
                "band {centre} Hz was kept at {rate} Hz but its upper edge passes the Nyquist margin"
            );
        }
    }
}

/// The Nyquist exclusion must actually exclude, and only at the top.
///
/// The pair matters: a meter that dropped *no* bands and one that dropped
/// *all* of them would both satisfy "`band_count` > 0".
#[test]
fn a_lower_sample_rate_drops_the_top_bands_and_only_those() {
    let wide = SoundLevelMeter::new(48_000, 1, Calibration::FullScale).unwrap();
    let narrow = SoundLevelMeter::new(22_050, 1, Calibration::FullScale).unwrap();

    assert!(
        narrow.band_count() < wide.band_count(),
        "22.05 kHz kept {} bands and 48 kHz kept {} — a lower rate must measure fewer",
        narrow.band_count(),
        wide.band_count()
    );

    let narrow_bands = narrow.centre_frequencies();
    let wide_bands = wide.centre_frequencies();
    assert_eq!(
        narrow_bands,
        wide_bands[..narrow_bands.len()],
        "the bands dropped at the lower rate must be the highest ones, contiguously"
    );
    assert!(
        !narrow_bands.contains(&20_000.0),
        "the 20 kHz band cannot be measured at a 22.05 kHz sample rate"
    );
    assert!(
        narrow_bands.contains(&1000.0),
        "the 1 kHz band must survive at 22.05 kHz"
    );
}

// ---------------------------------------------------------------------------
// Levels, against closed-form values
// ---------------------------------------------------------------------------

/// A full-scale sine has an RMS of 1/√2, so its level is exactly
/// 10·log₁₀(0.5) = −3.01 dBFS. Measured in its own band, that is what the
/// meter must report.
///
/// This is the gate that catches a scale error anywhere in the chain — a
/// missing square root, a 20·log₁₀ where 10·log₁₀ belongs, a filter with the
/// wrong peak gain. All three produce plausible-looking negative numbers.
#[test]
fn a_full_scale_sine_reads_minus_three_decibels_in_its_own_band() {
    let fs = 48_000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    let reading = meter
        .push(&sine(1000.0, 1.0, fs, 1))
        .expect("one second at a one-second interval yields a reading");

    let b = band(&reading, 1000.0);
    assert!(
        (b.mean_db - (-3.01)).abs() < 0.5,
        "a full-scale 1 kHz sine read {:.2} dBFS in the 1 kHz band, expected -3.01",
        b.mean_db
    );
}

/// Doubling the amplitude must raise the level by 20·log₁₀(2) = 6.02 dB.
///
/// The counterpart to the absolute test above: an implementation with a
/// constant offset passes that one and this one, an implementation with the
/// wrong logarithm base or factor fails this one.
#[test]
fn doubling_the_amplitude_adds_six_decibels() {
    let fs = 48_000;
    let level = |amp: f32| {
        let mut m = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
        band(&m.push(&sine(1000.0, amp, fs, 1)).unwrap(), 1000.0).mean_db
    };
    let quiet = level(0.1);
    let loud = level(0.2);
    assert!(
        ((loud - quiet) - 6.02).abs() < 0.15,
        "0.1 read {quiet:.2} dB and 0.2 read {loud:.2} dB, a difference of {:.2}; expected 6.02",
        loud - quiet
    );
}

/// A tone must land in its own band and be rejected two bands away.
///
/// Without this, a filter bank whose sections were all identical — the easiest
/// way to get the design wrong — would pass every level test above.
#[test]
fn a_tone_lands_in_its_own_band() {
    let fs = 48_000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    let reading = meter.push(&sine(1000.0, 0.5, fs, 1)).unwrap();

    let loudest = reading
        .bands
        .iter()
        .max_by(|a, b| a.mean_db.total_cmp(&b.mean_db))
        .expect("bands");
    assert!(
        (loudest.centre_hz - 1000.0).abs() < 0.01,
        "a 1 kHz tone peaked in the {} Hz band",
        loudest.centre_hz
    );

    let here = band(&reading, 1000.0).mean_db;
    let two_up = band(&reading, 1600.0).mean_db;
    let two_down = band(&reading, 630.0).mean_db;
    assert!(
        here - two_up > 20.0 && here - two_down > 20.0,
        "1 kHz band {here:.1} dB, 630 Hz {two_down:.1} dB, 1600 Hz {two_up:.1} dB — the bank is \
         not separating adjacent bands"
    );
}

/// Digital silence must floor, not produce −∞ or NaN.
#[test]
fn silence_floors_rather_than_diverging() {
    let fs = 48_000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    let reading = meter.push(&vec![0.0_f32; fs as usize]).unwrap();

    for b in &reading.bands {
        assert!(
            b.mean_db.is_finite() && b.min_db.is_finite() && b.max_db.is_finite(),
            "band {} produced a non-finite level on silence",
            b.centre_hz
        );
        assert!(
            (b.mean_db - FLOOR_DBFS).abs() < 0.01,
            "band {} read {:.2} dB on digital silence, expected the {FLOOR_DBFS} floor",
            b.centre_hz,
            b.mean_db
        );
    }
    assert!(reading.a_weighted_db.is_finite());
    assert!(reading.z_weighted_db.is_finite());
}

/// A NaN arriving from a broken decode must not poison the reading.
///
/// One non-finite sample in a recursive filter propagates to every subsequent
/// output forever. The failure is total, permanent, and silent.
#[test]
fn a_non_finite_sample_does_not_poison_the_meter() {
    let fs = 48_000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    let mut samples = sine(1000.0, 0.5, fs, 1);
    samples[100] = f32::NAN;
    samples[200] = f32::INFINITY;

    let reading = meter.push(&samples).expect("still yields a reading");
    assert!(!meter.is_diverged(), "the filter state went non-finite");
    for b in &reading.bands {
        assert!(
            b.mean_db.is_finite(),
            "band {} is non-finite after a NaN sample",
            b.centre_hz
        );
    }
    let here = band(&reading, 1000.0).mean_db;
    assert!(
        here > -20.0,
        "the 1 kHz band read {here:.1} dB; two bad samples out of 48 000 should barely move it"
    );
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// The interval mean is an energy mean, and the difference from an average of
/// the logs is large enough to matter.
///
/// Both numbers are asserted because the point of the choice is the gap
/// between them: a test that only checked the energy mean would be satisfied
/// by any monotone function of the inputs.
#[test]
fn the_interval_mean_is_an_energy_mean() {
    // Thirty seconds at -80 dB and one at -20 dB, computed directly rather
    // than pushed through the meter, so this pins the arithmetic and not the
    // filter bank.
    let quiet_db = -80.0_f32;
    let loud_db = -20.0_f32;
    let n = 31.0_f64;

    let energy_mean =
        power_to_db(30.0_f64.mul_add(db_to_power(quiet_db), db_to_power(loud_db)) / n);
    let log_mean = 30.0_f32.mul_add(quiet_db, loud_db) / 31.0;

    assert!(
        (energy_mean - (-34.91)).abs() < 0.05,
        "energy mean is {energy_mean:.2} dB, expected -34.91"
    );
    assert!(
        (log_mean - (-78.06)).abs() < 0.05,
        "arithmetic mean of the logs is {log_mean:.2} dB, expected -78.06"
    );
    assert!(
        energy_mean - log_mean > 40.0,
        "the two means differ by only {:.1} dB; if that gap has closed, the doc comment claiming \
         it is 43 dB is wrong",
        energy_mean - log_mean
    );
}

/// And the meter must actually use that energy mean.
///
/// The test above pins `power_to_db`/`db_to_power`, which is not the same
/// claim: replacing the meter's accumulation with a plain average of the
/// decibel values left all twenty-three gates green, because nothing drove the
/// meter with a signal where the two differ. This does.
///
/// Two seconds of digital silence and one at full scale. The energy mean of
/// that interval is about −7.8 dB — the loud second, divided across three —
/// and the arithmetic mean of the logs is about −81 dB, because two −120 dB
/// floors drag it down. A 73 dB gap: nothing subtle, but invisible until a
/// test looks.
#[test]
fn the_meter_reports_the_energy_mean_of_its_interval() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 3, Calibration::FullScale).unwrap();
    meter.push(&vec![0.0_f32; fs as usize]);
    meter.push(&vec![0.0_f32; fs as usize]);
    let reading = meter
        .push(&sine(1000.0, 1.0, fs, 1))
        .expect("third second closes the interval");

    let b = band(&reading, 1000.0);
    let energy_expected =
        power_to_db(2.0_f64.mul_add(db_to_power(FLOOR_DBFS), db_to_power(b.max_db)) / 3.0);
    let log_mean = 2.0_f32.mul_add(b.min_db, b.max_db) / 3.0;

    assert!(
        (b.mean_db - energy_expected).abs() < 0.5,
        "the interval mean is {:.1} dB; the energy mean of its three seconds is {:.1} dB and the \
         arithmetic mean of their decibel values is {:.1} dB",
        b.mean_db,
        energy_expected,
        log_mean
    );
    assert!(
        (b.mean_db - log_mean).abs() > 40.0,
        "the two means are only {:.1} dB apart here, so this signal no longer discriminates \
         between them",
        (b.mean_db - log_mean).abs()
    );
}

/// An interval covers the configured number of seconds, and min ≤ mean ≤ max.
#[test]
fn an_interval_reports_its_seconds_and_orders_its_statistics() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 3, Calibration::FullScale).unwrap();

    assert!(
        meter.push(&sine(1000.0, 0.5, fs, 2)).is_none(),
        "two seconds must not complete a three-second interval"
    );
    let reading = meter
        .push(&sine(1000.0, 0.5, fs, 1))
        .expect("the third second completes it");

    assert_eq!(reading.interval_secs, 3);
    for b in &reading.bands {
        assert_eq!(
            b.seconds, 3,
            "band {} counted {} seconds",
            b.centre_hz, b.seconds
        );
        assert!(
            b.min_db <= b.mean_db && b.mean_db <= b.max_db,
            "band {}: min {:.2} mean {:.2} max {:.2} are out of order",
            b.centre_hz,
            b.min_db,
            b.mean_db,
            b.max_db
        );
    }
}

/// A loud second inside a quiet interval must move max but barely move min.
///
/// The discriminator for the statistics: an implementation that reported the
/// same number three times would pass the ordering test above.
#[test]
fn a_transient_moves_the_maximum_and_not_the_minimum() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 3, Calibration::FullScale).unwrap();
    meter.push(&vec![0.0_f32; fs as usize]);
    meter.push(&sine(1000.0, 0.9, fs, 1));
    let reading = meter
        .push(&vec![0.0_f32; fs as usize])
        .expect("third second completes the interval");

    let b = band(&reading, 1000.0);
    assert!(
        b.max_db > -10.0,
        "max is {:.1} dB; the loud second should be near full scale",
        b.max_db
    );
    assert!(
        b.min_db < -80.0,
        "min is {:.1} dB; two of the three seconds were digital silence",
        b.min_db
    );
    assert!(
        b.max_db - b.min_db > 60.0,
        "max and min differ by only {:.1} dB across silence and a full-scale tone",
        b.max_db - b.min_db
    );
}

/// Consecutive intervals must be independent — statistics reset between them.
#[test]
fn statistics_do_not_carry_between_intervals() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();

    // Silence first, then the tone — deliberately that way round. The reverse
    // order tests the same reset but confounds it with the filter's ringdown
    // from the step, which is real signal and lands 65 dB above the floor;
    // see `a_hard_cut_rings_the_bank_and_the_ringing_decays`. Going quiet to
    // loud, a leak would show as interval 2 keeping the floor as its minimum,
    // and no transient can produce that.
    let quiet = meter
        .push(&vec![0.0_f32; fs as usize])
        .expect("first interval");
    let loud = meter
        .push(&sine(1000.0, 0.9, fs, 1))
        .expect("second interval");

    assert!(
        (band(&quiet, 1000.0).max_db - FLOOR_DBFS).abs() < 1.0,
        "the silent interval should sit at the floor, not {:.1} dB",
        band(&quiet, 1000.0).max_db
    );
    assert!(
        band(&loud, 1000.0).min_db > -20.0,
        "the loud interval's minimum is {:.1} dB — the silent interval's floor carried over",
        band(&loud, 1000.0).min_db
    );
    assert_eq!(
        band(&loud, 1000.0).seconds,
        1,
        "the second interval counted seconds from the first"
    );
}

/// Cutting a loud tone dead rings the filter bank, and the ringing is signal.
///
/// Written because it looked like a leak. A full-scale 1 kHz tone stopped
/// abruptly leaves the *next* second of digital silence reading about -35 dBFS,
/// which is 85 dB above the floor and looks exactly like interval statistics
/// carrying over. It is not: a step from -0.64 to 0 is broadband, the bank
/// responds to it, and a filter that did not would be the broken one.
///
/// Pinned rather than tolerated, because the number is a property of the
/// filter's `Q` and would move if the cascade changed — and because the next
/// person to see -35 dB after silence deserves to find this instead of
/// re-deriving it.
#[test]
fn a_hard_cut_rings_the_bank_and_the_ringing_decays() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    meter.push(&sine(1000.0, 0.9, fs, 1));
    let after_cut = meter.push(&vec![0.0_f32; fs as usize]).expect("second");
    let later = meter.push(&vec![0.0_f32; fs as usize]).expect("third");

    let rung = band(&after_cut, 1000.0).max_db;
    assert!(
        (-60.0..-15.0).contains(&rung),
        "the second after a hard cut read {rung:.1} dB; the transient response to a step of this \
         size should land in the -60..-15 dB range"
    );
    assert!(
        (band(&later, 1000.0).max_db - FLOOR_DBFS).abs() < 1.0,
        "the ringing had not decayed by the second interval after the cut: {:.1} dB",
        band(&later, 1000.0).max_db
    );
}

// ---------------------------------------------------------------------------
// Weighting and calibration at the reading level
// ---------------------------------------------------------------------------

/// Low-frequency energy must read far lower A-weighted than unweighted, and
/// energy at 1 kHz must read about the same either way.
///
/// The pair is the point: A-weighting that did nothing would pass the second
/// assertion alone, and A-weighting applied with the wrong sign would pass
/// neither.
#[test]
fn a_weighting_discounts_low_frequencies_and_leaves_one_kilohertz_alone() {
    let fs = 48_000;

    let gap = |hz: f32| {
        let mut m = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
        let r = m.push(&sine(hz, 0.5, fs, 1)).unwrap();
        r.z_weighted_db - r.a_weighted_db
    };

    let low = gap(63.0);
    let mid = gap(1000.0);
    assert!(
        low > 15.0,
        "63 Hz: A-weighted is only {low:.1} dB below unweighted; the published offset there is \
         -26.2 dB, so the weighting is not being applied"
    );
    assert!(
        mid.abs() < 3.0,
        "1 kHz: A-weighted differs from unweighted by {mid:.1} dB, but the weighting is 0 dB there"
    );
}

/// A calibration offset shifts every reported figure by exactly that offset,
/// and nothing else.
#[test]
fn calibration_shifts_every_figure_by_the_offset() {
    let fs = 8000;
    let offset = 94.0_f32;

    let mut raw = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    let mut cal = SoundLevelMeter::new(fs, 1, Calibration::SplOffsetDb(offset)).unwrap();
    let tone = sine(1000.0, 0.5, fs, 1);

    let a = raw.push(&tone).unwrap();
    let b = cal.push(&tone).unwrap();

    assert_eq!(a.bands.len(), b.bands.len());
    for (x, y) in a.bands.iter().zip(b.bands.iter()) {
        assert!((y.mean_db - x.mean_db - offset).abs() < 0.01);
        assert!((y.min_db - x.min_db - offset).abs() < 0.01);
        assert!((y.max_db - x.max_db - offset).abs() < 0.01);
    }
    assert!((b.a_weighted_db - a.a_weighted_db - offset).abs() < 0.01);
    assert!((b.z_weighted_db - a.z_weighted_db - offset).abs() < 0.01);

    assert_eq!(Calibration::FullScale.unit(), "dBFS");
    assert_eq!(Calibration::SplOffsetDb(offset).unit(), "dB SPL");
}

/// `reset` must clear the ringing so a source restart does not bleed into the
/// first second of the new signal.
#[test]
fn reset_clears_the_filter_ringing() {
    let fs = 8000;
    let mut meter = SoundLevelMeter::new(fs, 1, Calibration::FullScale).unwrap();
    // Half a second of a loud tone, then reset, then a full second of silence.
    meter.push(&sine(1000.0, 0.9, fs, 1));
    meter.reset();
    let reading = meter
        .push(&vec![0.0_f32; fs as usize])
        .expect("a full second after the reset");

    assert!(
        (band(&reading, 1000.0).mean_db - FLOOR_DBFS).abs() < 1.0,
        "after reset, silence read {:.1} dB — the filter is still ringing from before",
        band(&reading, 1000.0).mean_db
    );
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

#[test]
fn band_labels_are_the_ones_an_acoustics_tool_prints() {
    assert_eq!(label_for(25.0), "25");
    assert_eq!(label_for(31.5), "31.5");
    assert_eq!(label_for(1000.0), "1k");
    assert_eq!(
        label_for(1250.0),
        "1.25k",
        "one decimal place gives 1.2k here, because Rust rounds 1.25 to even — and 1.2k is a \
         band that does not exist"
    );
    assert_eq!(label_for(3150.0), "3.15k");
    assert_eq!(label_for(1600.0), "1.6k");
    assert_eq!(label_for(12_500.0), "12.5k");
    assert_eq!(label_for(20_000.0), "20k");
}

/// Every band a meter measures must have a distinct label, or two series
/// collapse into one and the chart silently loses a band.
#[test]
fn every_band_has_a_distinct_label() {
    let meter = SoundLevelMeter::new(48_000, 1, Calibration::FullScale).unwrap();
    let mut labels: Vec<String> = meter
        .centre_frequencies()
        .iter()
        .copied()
        .map(label_for)
        .collect();
    let before = labels.len();
    labels.sort();
    labels.dedup();
    assert_eq!(before, labels.len(), "two bands share a label: {labels:?}");
}
