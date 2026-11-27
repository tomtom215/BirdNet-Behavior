//! RES-31 and ACC-12: the classifier's raw output on a fixed input, through the
//! station's own decode, resample and chunking.
//!
//! - **Twice is bit-identical** (RES-31): two fresh loads of one model, on one
//!   machine, give the same bits for every window of the bundled magpie clip.
//! - **The committed golden** (ACC-12): the same record is held to
//!   `tests/data/golden/tiny_v24_magpie.json`, made on x86-64 Linux. On the
//!   platform it was made on it must match bit for bit; on another (aarch64,
//!   the target) the test prints the comparison and holds the differences to
//!   the bounds below, so an aarch64 job running this test is the check the
//!   row asks for.
//!
//! The model is the 11-class test model from `src/testdata`, not BirdNET: it
//! checks the pipeline and the runtime on a platform, not the real model's
//! numbers. A record of the real model is made with
//! `cargo run -p birdnet-core --example model_check -- record ...` (see
//! `docs/book/guides/evaluation.md`); **none has been made on aarch64 by this
//! project.**
//!
//! Regenerate the golden (only when the pipeline is meant to change what the
//! model is fed): `BNB_WRITE_GOLDEN=1 cargo test -p birdnet-core --test
//! model_output_is_reproducible`.

use std::path::{Path, PathBuf};

use birdnet_core::evaluation::golden::{self, GoldenRecord};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

const TINY_V24: &[u8] = include_bytes!("../src/testdata/tiny_v24_test.onnx");

/// Off the platform the golden was made on, the largest difference allowed in
/// a raw output (no window may change its top class).
const OTHER_PLATFORM_MAX_ABS_DIFF: f64 = 1e-5;

struct Fixture {
    _dir: tempfile::TempDir,
    wav: PathBuf,
    model: PathBuf,
    labels: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/testdata/Pica_pica_30s.wav");
    let wav = dir.path().join("2026-05-19-birdnet-06:30:00.wav");
    std::fs::copy(&src, &wav).unwrap();
    let model = dir.path().join("tiny_v24.onnx");
    std::fs::write(&model, TINY_V24).unwrap();
    let labels = dir.path().join("labels.txt");
    let lines: Vec<String> = (0..11)
        .map(|i| format!("Species{i} bird_Bird {i}"))
        .collect();
    std::fs::write(&labels, lines.join("\n")).unwrap();
    Fixture {
        _dir: dir,
        wav,
        model,
        labels,
    }
}

fn record(f: &Fixture) -> GoldenRecord {
    let labels = LabelSet::load(&f.labels).unwrap();
    let mut model = BirdNetModel::load(&f.model, labels, ModelConfig::default()).unwrap();
    let config = golden::pipeline_for(&model);
    golden::record(&f.wav, &config, &mut model, &f.model, &f.labels).expect("records")
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/golden/tiny_v24_magpie.json")
}

#[test]
fn the_same_input_twice_gives_the_same_bits() {
    let f = fixture();
    let a = record(&f);
    let b = record(&f);
    assert_eq!(
        a.windows.len(),
        10,
        "precondition: ten 3 s windows of the 30 s clip"
    );
    assert!(
        a.windows
            .iter()
            .any(|w| w.output_bits.iter().any(|b| *b != 0)),
        "precondition: the model said something"
    );
    let c = golden::compare(&a, &b, 0.5);
    assert!(c.identical, "{c:?}");
}

#[test]
fn the_output_matches_the_committed_golden() {
    let f = fixture();
    let now = record(&f);
    if std::env::var_os("BNB_WRITE_GOLDEN").is_some() {
        std::fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
        std::fs::write(
            golden_path(),
            serde_json::to_string_pretty(&now).unwrap() + "\n",
        )
        .unwrap();
    }
    let text = std::fs::read_to_string(golden_path()).expect("the golden is committed");
    let golden: GoldenRecord = serde_json::from_str(&text).expect("the golden parses");
    let c = golden::compare(&golden, &now, 0.5);
    eprintln!(
        "golden made on {}/{}, run on {}/{}: {c:?}",
        golden.header.arch, golden.header.os, now.header.arch, now.header.os
    );
    assert!(c.same_inputs, "the golden describes other files: {c:?}");
    assert!(
        c.same_window_inputs,
        "the pipeline fed the model different windows: {c:?}"
    );
    if golden.header.arch == now.header.arch && golden.header.os == now.header.os {
        assert!(c.identical, "same platform, different bits: {c:?}");
    } else {
        assert!(c.max_abs_diff <= OTHER_PLATFORM_MAX_ABS_DIFF, "{c:?}");
        assert_eq!(c.finiteness_mismatches, 0, "{c:?}");
        assert_eq!(c.top1_changed, 0, "{c:?}");
    }
}
