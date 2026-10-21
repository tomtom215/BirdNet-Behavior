//! Does a recording come apart into the windows BirdNET-Pi cuts, at every overlap?
//!
//! `pipeline.rs` carries a sweep against a *transcription* of BirdNET-Pi's
//! `splitSignal` at six overlaps that are exact in binary (0, 0.5 ... 2.5). The
//! `OVERLAP` setting is a free decimal from 0 to 2.9, and the transcription is not
//! BirdNET-Pi's code. This runs BirdNET-Pi's own `splitSignal` (golden
//! `tools/parity/golden/chunking.json`, made by `tools/parity/chunking_ref.py`)
//! over 59 overlaps (0.00 to 2.90 in steps of 0.05) times 155 recording lengths
//! (1.5 s to 40 s in quarter seconds), and compares the window count with what
//! this project cuts. The count is computed for all 9 145 combinations from the
//! shipping `chunk_step_samples` and a copy of the (private) walk that
//! `process_file` runs; and `process_file` itself, on real WAV files, is asked
//! about the 48 combinations that differ and 48 that do not, so the copy is held to
//! the real thing where it matters.
//!
//! They do not all agree, and the reason is arithmetic, not intent. BirdNET-Pi
//! takes `int((3.0 - overlap) * 48000)` in float64, so for 0.7 it steps
//! 110 399 samples (2.3 is 2.29999999999999982 in binary); this project steps
//! `3 * 48000 - int(overlap_f32 * 48000)` = 110 400. One sample a window is
//! nothing. But when a recording's last window would have exactly the
//! minimum 1.5 s of audio, that sample decides whether it exists, and
//! `KNOWN_DIFFERENCES` lists the 48 of 9 145 combinations where this project
//! cuts one window fewer. Neither the default (`OVERLAP=0`) nor a 15 s segment
//! at any overlap but 2.10 is among them.
//!
//! The list is pinned both ways: a combination that stops differing, or a new one
//! that starts, fails.

use std::collections::BTreeSet;
use std::io::Write as _;

use birdnet_core::detection::pipeline::{PipelineConfig, chunk_step_samples, process_file};

const GOLDEN: &str = "tools/parity/golden/chunking.json";
const RATE: u32 = 48_000;

/// `(overlap hundredths, quarter seconds of audio)` where this project cuts one
/// window fewer than BirdNET-Pi. 48 entries.
const KNOWN_DIFFERENCES: &[(u32, u32)] = &[
    (65, 53),
    (65, 100),
    (65, 147),
    (105, 45),
    (105, 84),
    (105, 123),
    (130, 40),
    (130, 74),
    (130, 108),
    (130, 142),
    (210, 24),
    (210, 42),
    (210, 60),
    (210, 78),
    (210, 96),
    (210, 114),
    (210, 132),
    (210, 150),
    (235, 19),
    (235, 32),
    (235, 45),
    (235, 58),
    (235, 71),
    (235, 84),
    (235, 97),
    (235, 110),
    (235, 123),
    (235, 136),
    (235, 149),
    (260, 14),
    (260, 22),
    (260, 30),
    (260, 38),
    (260, 46),
    (260, 54),
    (260, 62),
    (260, 70),
    (260, 78),
    (260, 86),
    (260, 94),
    (260, 102),
    (260, 110),
    (260, 118),
    (260, 126),
    (260, 134),
    (260, 142),
    (260, 150),
    (260, 158),
];

fn write_wav(path: &std::path::Path, samples: usize) {
    let mut f = std::fs::File::create(path).unwrap();
    let data_len = u32::try_from(samples * 2).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16_u32.to_le_bytes()).unwrap();
    f.write_all(&1_u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&1_u16.to_le_bytes()).unwrap(); // mono
    f.write_all(&RATE.to_le_bytes()).unwrap();
    f.write_all(&(RATE * 2).to_le_bytes()).unwrap();
    f.write_all(&2_u16.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    // A low tone, not silence: the pipeline must be cutting real samples.
    let body: Vec<u8> = (0..samples)
        .flat_map(|i| ((((i % 100) as i16) - 50) * 20).to_le_bytes())
        .collect();
    f.write_all(&body).unwrap();
}

/// The walk `pipeline::process_file` runs over a recording (private there): chunk starts for a `len`-sample recording.
/// `tests` below hold this to `process_file` itself on real files for the cases that matter.
fn our_starts(len: usize, step: usize, min_tail: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < len {
        starts.push(pos);
        pos += step;
        if pos >= len || len - pos < min_tail {
            break;
        }
    }
    starts
}

fn config(overlap: f32) -> PipelineConfig {
    PipelineConfig {
        target_sample_rate: RATE,
        raw_audio_input: true,
        chunk_duration_secs: 3.0,
        chunk_overlap_secs: overlap,
        confidence_threshold: 0.0,
        ..PipelineConfig::default()
    }
}

fn hundredths(overlap: f32) -> u32 {
    u32::try_from((f64::from(overlap) * 100.0).round() as i64).unwrap()
}

#[test]
fn windows_are_cut_where_birdnet_pis_split_signal_cuts_them() {
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let overlaps = golden["overlaps"].as_object().expect("overlaps");
    assert_eq!(overlaps.len(), 59);
    let min_tail = (1.5 * f64::from(RATE)) as usize;
    let mut differing: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut compared = 0;
    for (key, entry) in overlaps {
        let overlap: f32 = key.parse().unwrap();
        let step = chunk_step_samples(&config(overlap));
        for (i, quarter) in (6_u32..=160).enumerate() {
            let len = usize::try_from(quarter * RATE / 4).unwrap();
            let ours = our_starts(len, step, min_tail);
            let pi_count = usize::try_from(entry["counts"][i].as_u64().unwrap()).unwrap();
            compared += 1;
            if ours.len() == pi_count {
                continue;
            }
            assert_eq!(
                ours.len() + 1,
                pi_count,
                "overlap {key}, {} s: ours {} windows, BirdNET-Pi {pi_count}",
                f64::from(quarter) / 4.0,
                ours.len()
            );
            differing.insert((hundredths(overlap), quarter));
        }
    }
    assert_eq!(compared, 59 * 155);
    let want: BTreeSet<(u32, u32)> = KNOWN_DIFFERENCES.iter().copied().collect();
    assert_eq!(
        differing, want,
        "the combinations that differ from BirdNET-Pi changed"
    );
}

/// `process_file` on real files agrees with the walk above on the 48 combinations that differ from BirdNET-Pi
/// and on 48 that do not (every 190th of the rest), so the copy in this file is the real thing's behaviour.
#[test]
fn process_file_cuts_what_the_walk_above_says_it_cuts() {
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(GOLDEN).expect("golden")).expect("json");
    let overlaps = golden["overlaps"].as_object().expect("overlaps");
    let known: BTreeSet<(u32, u32)> = KNOWN_DIFFERENCES.iter().copied().collect();
    let mut cases: Vec<(String, u32)> = Vec::new();
    let mut n = 0;
    for key in overlaps.keys() {
        let h = hundredths(key.parse().unwrap());
        for quarter in 6_u32..=160 {
            n += 1;
            if known.contains(&(h, quarter)) || (n % 190 == 0) {
                cases.push((key.clone(), quarter));
            }
        }
    }
    assert!(cases.len() >= 96, "{} cases", cases.len());
    let min_tail = (1.5 * f64::from(RATE)) as usize;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("2026-06-15-birdnet-10:00:00.wav");
    let mut checked = 0;
    for (key, quarter) in cases {
        let overlap: f32 = key.parse().unwrap();
        let len = usize::try_from(quarter * RATE / 4).unwrap();
        write_wav(&path, len);
        let real = process_file(&path, &config(overlap)).expect("pipeline");
        let walk = our_starts(len, chunk_step_samples(&config(overlap)), min_tail);
        assert_eq!(
            real.len(),
            walk.len(),
            "overlap {key}, {} s",
            f64::from(quarter) / 4.0
        );
        let last = (f64::from(real.last().unwrap().start_secs) * f64::from(RATE)).round() as i64;
        assert!(
            (last - i64::try_from(*walk.last().unwrap()).unwrap()).abs() <= 1,
            "overlap {key}, {} s",
            f64::from(quarter) / 4.0
        );
        checked += 1;
    }
    assert!(checked >= 96);
}

/// The counterpart: the overlaps exact in binary (the ones the six-value sweep in `pipeline.rs` covers) never differ
/// from BirdNET-Pi on any of the 155 lengths, so a blanket loosening of the comparison above cannot pass.
#[test]
fn the_overlaps_exact_in_binary_never_differ() {
    for (h, _) in KNOWN_DIFFERENCES {
        assert!(![0, 50, 100, 150, 200, 250].contains(h), "{h}");
    }
}
