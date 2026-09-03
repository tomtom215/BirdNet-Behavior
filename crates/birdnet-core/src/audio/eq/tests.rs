//! Gates for the equaliser chain.

use super::*;

const SR: u32 = 48_000;

/// Measure a chain's actual gain at `hz` by running a tone through it.
///
/// Two seconds; the first is discarded so the reading is of the steady state
/// rather than the filter's transient.
fn measured_db(chain: &EqChain, hz: f32) -> f64 {
    let mut p = chain.build(SR).expect("builds");
    let n = (SR * 2) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        #[allow(clippy::cast_precision_loss)]
        let t = i as f32 / SR as f32;
        out.push(p.process((std::f32::consts::TAU * hz * t).sin()));
    }
    let tail = &out[n / 2..];
    #[allow(clippy::cast_precision_loss)]
    let rms = (tail
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / tail.len() as f64)
        .sqrt();
    20.0 * (rms * std::f64::consts::SQRT_2).log10()
}

// ---------------------------------------------------------------------------
// The stored form
// ---------------------------------------------------------------------------

/// Every chain round-trips through its stored form unchanged.
///
/// The specification is what lives in the database, so a chain that parses
/// into something it does not format back to would drift a station's audio
/// every time the row was rewritten — on a settings save that touched an
/// unrelated field, for instance.
#[test]
fn every_chain_round_trips_through_its_stored_form() {
    let cases = [
        "highpass:120",
        "notch:50:20",
        "peaking:3500:1:4",
        "lowshelf:200:0.9:-6",
        "highpass:80:0.707:0:3",
        "highpass:5; highpass:120",
        "highpass:120:1.2; notch:50:30; notch:150:30; peaking:3500:1:4.5; highshelf:8000:0.7:-3",
    ];
    for spec in cases {
        let chain = EqChain::parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
        let formatted = chain.to_spec();
        let again =
            EqChain::parse(&formatted).unwrap_or_else(|e| panic!("reformatted {formatted:?}: {e}"));
        assert_eq!(
            chain, again,
            "{spec:?} formatted to {formatted:?}, which parsed to something else"
        );
    }
}

/// Passes survive the round trip even for a kind that has no gain.
///
/// The trap: `highpass:80:0.707:3` reads 3 as a *gain*, so a chain meaning
/// "three passes" would come back as one pass of a filter with a meaningless
/// gain. The formatter emits the placeholder; this is what says so.
#[test]
fn passes_survive_a_kind_that_has_no_gain() {
    let chain = EqChain::new(vec![EqStage::new(StageKind::HighPass, 80.0).with_passes(3)]);
    let spec = chain.to_spec();
    assert_eq!(spec, "highpass:80:0.70710677:0:3");
    let back = EqChain::parse(&spec).expect("parses");
    assert_eq!(back.stages()[0].passes, 3, "passes were read as a gain");
    assert_eq!(back, chain);
}

/// Comments and blank lines are for the operator, not the parser.
#[test]
fn comments_and_blank_lines_are_ignored() {
    let chain = EqChain::parse(
        "# the site's own noise\n\
         highpass:120   # wind\n\
         \n\
         notch:50:20    # mains hum\n",
    )
    .expect("parses");
    assert_eq!(chain.stages().len(), 2);
    assert_eq!(chain.stages()[0].kind, StageKind::HighPass);
    assert_eq!(chain.stages()[1].kind, StageKind::Notch);
}

/// A malformed stage is refused by name, never skipped.
///
/// A chain that quietly does less than the operator wrote is the exact failure
/// this feature exists to stop being invisible — the three booleans it
/// replaces were stored, displayed, and reached nothing.
#[test]
fn a_malformed_stage_is_refused_and_named() {
    for (spec, why) in [
        ("nonsense:120", "unknown filter kind"),
        ("highpass", "missing frequency"),
        ("highpass:abc", "frequency is not a number"),
        ("highpass:0", "frequency must be above zero"),
        ("highpass:-40", "frequency must be above zero"),
        ("highpass:120:0", "Q must be above zero"),
        ("highpass:120:x", "Q is not a number"),
        ("peaking:120:1:x", "gain is not a number"),
        ("highpass:120:1:0:0", "passes must be between 1 and 8"),
        ("highpass:120:1:0:9", "passes must be between 1 and 8"),
        ("highpass:120:1:0:1:extra", "too many fields"),
    ] {
        let err = EqChain::parse(spec).unwrap_err();
        assert_eq!(err.reason, why, "for {spec:?}");
        assert!(
            err.to_string().contains(spec.trim()),
            "the message must name the offending stage: {err}"
        );
    }
}

/// An empty specification is an empty chain, not an error.
#[test]
fn an_empty_specification_is_an_empty_chain() {
    for spec in ["", "   ", "\n\n", "# only a comment"] {
        let chain = EqChain::parse(spec).unwrap_or_else(|e| panic!("{spec:?}: {e}"));
        assert!(chain.is_empty());
        assert!(chain.build(SR).expect("builds").is_empty());
    }
}

// ---------------------------------------------------------------------------
// What the chain does to audio
// ---------------------------------------------------------------------------

/// The chain filters, and each stage contributes.
#[test]
fn a_chain_applies_every_stage() {
    let chain = EqChain::parse("highpass:200; notch:1000:20; peaking:4000:1:6").expect("parses");

    assert!(
        measured_db(&chain, 50.0) < -15.0,
        "the high-pass should cut 50 Hz hard: {:.2} dB",
        measured_db(&chain, 50.0)
    );
    assert!(
        measured_db(&chain, 1000.0) < -25.0,
        "the notch should null 1 kHz: {:.2} dB",
        measured_db(&chain, 1000.0)
    );
    assert!(
        (measured_db(&chain, 4000.0) - 6.0).abs() < 0.5,
        "the bell should lift 4 kHz by 6 dB: {:.2} dB",
        measured_db(&chain, 4000.0)
    );
    assert!(
        measured_db(&chain, 500.0).abs() < 0.3,
        "500 Hz sits between every stage and should be untouched: {:.2} dB",
        measured_db(&chain, 500.0)
    );

    // A Q of 1 is a wide bell, and its skirt is real: an octave below the
    // centre it still lifts 1.8 dB, two octaves above 1.6 dB. Pinned rather
    // than avoided, because "left alone" was the first guess here and it was
    // wrong — an operator reading a +6 dB bell as affecting only 4 kHz would
    // make the same mistake.
    assert!(
        (measured_db(&chain, 2000.0) - 1.81).abs() < 0.3,
        "an octave below a Q=1 bell should still be lifted ~1.8 dB, got {:.2}",
        measured_db(&chain, 2000.0)
    );
    assert!(
        (measured_db(&chain, 8000.0) - 1.60).abs() < 0.3,
        "and two octaves above, ~1.6 dB, got {:.2}",
        measured_db(&chain, 8000.0)
    );
}

/// Extra passes make a slope steeper, by the amount they should.
///
/// One pass of a Butterworth high-pass is 12 dB per octave; three is 36. An
/// implementation that stored `passes` and applied the section once — which is
/// the obvious way to get this wrong — gives the same number for both.
#[test]
fn extra_passes_steepen_the_slope() {
    let one = EqChain::parse("highpass:200").expect("parses");
    let three = EqChain::parse("highpass:200:0.70710677:0:3").expect("parses");

    assert_eq!(one.build(SR).expect("builds").section_count(), 1);
    assert_eq!(three.build(SR).expect("builds").section_count(), 3);

    // Two octaves below the corner, so well into the asymptotic slope.
    let a = measured_db(&one, 50.0);
    let b = measured_db(&three, 50.0);
    assert!(
        3.0f64.mul_add(-a, b).abs() < 1.5,
        "three passes should be three times one pass in decibels: one {a:.2} dB, \
         three {b:.2} dB"
    );
}

/// The response curve and the filtered signal agree.
///
/// `magnitude_db_at` draws the curve an operator tunes against. If it were
/// wrong, every editing decision would be made against a lie while the audio
/// did something else — and no test that only ever consults the curve could
/// tell.
#[test]
fn the_response_curve_matches_the_filtered_signal() {
    let chain =
        EqChain::parse("highpass:200; peaking:4000:1:6; highshelf:9000:0.7:-4").expect("parses");
    for hz in [100.0_f32, 400.0, 1000.0, 4000.0, 12_000.0] {
        let curve = chain.magnitude_db_at(hz, SR);
        let real = measured_db(&chain, hz);
        assert!(
            (curve - real).abs() < 0.3,
            "at {hz} Hz the curve says {curve:+.2} dB and the audio measures {real:+.2} dB"
        );
    }
}

/// An empty chain is a wire.
#[test]
fn an_empty_chain_leaves_audio_untouched() {
    let mut p = EqChain::default().build(SR).expect("builds");
    let mut samples: Vec<f32> = (0..1000)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            {
                (i as f32 / 100.0).sin()
            }
        })
        .collect();
    let before = samples.clone();
    p.process_buffer(&mut samples);
    assert_eq!(samples, before);
}

/// `reset` clears the ringing across a capture discontinuity.
#[test]
fn reset_clears_the_state() {
    let chain = EqChain::parse("highpass:200:2").expect("parses");
    let mut p = chain.build(SR).expect("builds");
    for _ in 0..1000 {
        let _ = p.process(1.0);
    }
    let ringing = p.process(0.0).abs();
    p.reset();
    assert!(
        p.process(0.0).abs() < ringing / 10.0,
        "after reset the first silent sample should be near zero, not {ringing:.4}"
    );
}

/// A stage that cannot exist at this rate is an error, not a silent skip.
#[test]
fn a_stage_above_nyquist_fails_the_build() {
    let chain = EqChain::parse("lowpass:30000").expect("parses");
    assert!(
        chain.build(48_000).is_err(),
        "30 kHz is above Nyquist at 48 kHz and must be refused"
    );
    assert!(
        chain.build(96_000).is_ok(),
        "and the same chain must build at 96 kHz"
    );
}

// ---------------------------------------------------------------------------
// The two backends
// ---------------------------------------------------------------------------

/// The ffmpeg fragment names the same filter as the biquad it mirrors.
///
/// Two implementations of one specification is the shape of defect this
/// repository has paid for twice. `width_type=q` is the part that matters:
/// ffmpeg's default width unit differs per filter, so omitting it would mean
/// the two backends implement different filters from the same configuration,
/// silently, and audibly only to a spectrum analyser.
#[test]
fn both_backends_describe_the_same_filter() {
    let cases = [
        (
            "highpass:120",
            "highpass=f=120:width_type=q:width=0.70710677",
        ),
        ("notch:50:20", "bandreject=f=50:width_type=q:width=20"),
        (
            "peaking:3500:1:4",
            "equalizer=f=3500:width_type=q:width=1:g=4",
        ),
        (
            "lowshelf:200:0.9:-6",
            "bass=f=200:width_type=q:width=0.9:g=-6",
        ),
        (
            "highshelf:8000:0.7:3",
            "treble=f=8000:width_type=q:width=0.7:g=3",
        ),
        (
            "lowpass:9000",
            "lowpass=f=9000:width_type=q:width=0.70710677",
        ),
        ("bandpass:2000:2", "bandpass=f=2000:width_type=q:width=2"),
    ];
    for (spec, expected) in cases {
        let chain = EqChain::parse(spec).expect("parses");
        assert_eq!(
            chain.ffmpeg_filters(),
            vec![expected.to_owned()],
            "for {spec:?}"
        );
    }
}

/// Passes become repeated ffmpeg stages, so both backends apply the section
/// the same number of times.
#[test]
fn passes_are_repeated_in_the_ffmpeg_fragment() {
    let chain = EqChain::parse("highpass:80:0.707:0:3").expect("parses");
    let f = &chain.ffmpeg_filters()[0];
    assert_eq!(
        f.matches("highpass=").count(),
        3,
        "three passes must emit three ffmpeg stages, got {f}"
    );
    assert_eq!(
        chain.build(SR).expect("builds").section_count(),
        3,
        "and three biquads in process"
    );
}

/// Where ffmpeg is installed, the two backends agree on real audio.
///
/// The strongest form of the claim above, and the only one that would catch a
/// divergence in what ffmpeg's parameters *mean* rather than what they are
/// called. Skipped where ffmpeg is absent — this container has none — so it
/// runs on a developer's machine and on any station.
#[test]
fn the_two_backends_agree_on_real_audio() {
    if std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("ffmpeg not installed; skipping the cross-backend comparison");
        return;
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let chain = EqChain::parse("highpass:200; peaking:4000:1:6").expect("parses");

    for hz in [100.0_f32, 1000.0, 4000.0] {
        // A two-second tone.
        let n = (SR * 2) as usize;
        let tone: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / SR as f32;
                0.5 * (std::f32::consts::TAU * hz * t).sin()
            })
            .collect();

        let input = dir.path().join("in.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SR,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&input, spec).expect("create");
        for &s in &tone {
            #[allow(clippy::cast_possible_truncation)]
            w.write_sample((s * f32::from(i16::MAX)) as i16)
                .expect("write");
        }
        w.finalize().expect("finalize");

        let output = dir.path().join("out.wav");
        let filters = chain.ffmpeg_filters().join(",");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-i"])
            .arg(&input)
            .args(["-af", &filters])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "ffmpeg failed on filter graph {filters:?}");

        let filtered = crate::audio::decode::decode_file(&output).expect("decode");
        let tail = &filtered.samples[filtered.samples.len() / 2..];
        #[allow(clippy::cast_precision_loss)]
        let rms = (tail
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum::<f64>()
            / tail.len() as f64)
            .sqrt();
        // The input tone has an amplitude of 0.5, so its RMS is 0.5/√2.
        let ffmpeg_db = 20.0 * (rms / (0.5 / std::f64::consts::SQRT_2)).log10();
        let ours = measured_db(&chain, hz);

        assert!(
            (ffmpeg_db - ours).abs() < 0.5,
            "at {hz} Hz ffmpeg gives {ffmpeg_db:+.2} dB and the in-process chain gives \
             {ours:+.2} dB — the two backends have diverged"
        );
    }
}

// ---------------------------------------------------------------------------
// The migration from the three booleans
// ---------------------------------------------------------------------------

/// The default chain reproduces the legacy flags at the same corners.
///
/// An upgrade must not change what a microphone sounds like, and the corners
/// are read from the capture module rather than repeated, so this also pins
/// that they have not drifted apart.
#[test]
fn the_legacy_flags_map_onto_the_same_corners() {
    let both = EqChain::from_pipeline_flags(true, true);
    assert_eq!(both.stages().len(), 2);
    // DC block first, then the wind filter, which is the order the tee applies
    // them in.
    assert!((both.stages()[0].freq_hz - crate::audio::capture::DC_BLOCK_CUTOFF_HZ).abs() < 1e-6);
    assert!((both.stages()[1].freq_hz - crate::audio::capture::HIGH_PASS_CUTOFF_HZ).abs() < 1e-6);
    for s in both.stages() {
        assert_eq!(s.kind, StageKind::HighPass);
    }

    assert_eq!(EqChain::from_pipeline_flags(false, false).stages().len(), 0);
    assert_eq!(EqChain::from_pipeline_flags(true, false).stages().len(), 1);
    assert!(
        (EqChain::from_pipeline_flags(true, false).stages()[0].freq_hz
            - crate::audio::capture::HIGH_PASS_CUTOFF_HZ)
            .abs()
            < 1e-6
    );
    assert!(
        (EqChain::from_pipeline_flags(false, true).stages()[0].freq_hz
            - crate::audio::capture::DC_BLOCK_CUTOFF_HZ)
            .abs()
            < 1e-6
    );
}

/// And the migrated chain actually behaves like a wind filter.
///
/// The counterpart: a mapping that produced two stages at the right
/// frequencies but the wrong *kind* — low-pass instead of high-pass — would
/// satisfy the test above and invert what every existing station hears.
#[test]
fn the_migrated_chain_cuts_rumble_and_passes_song() {
    let chain = EqChain::from_pipeline_flags(true, true);
    assert!(
        measured_db(&chain, 30.0) < -20.0,
        "30 Hz rumble should be cut hard: {:.2} dB",
        measured_db(&chain, 30.0)
    );
    assert!(
        measured_db(&chain, 3000.0).abs() < 0.5,
        "3 kHz song should pass untouched: {:.2} dB",
        measured_db(&chain, 3000.0)
    );
}
