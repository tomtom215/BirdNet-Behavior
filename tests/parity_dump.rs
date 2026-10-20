//! Producer of the "ours" side of the BirdNET-Pi parity diff (`tools/parity/`).
//!
//! Runs our real decode -> resample -> chunk -> ONNX path over a list of WAVs
//! and writes, per WAV, every chunk's start/end and the model's top 10 as raw
//! logits (sensitivity 1.0, threshold 0, so `confidence = sigmoid(raw)` and the
//! logit is recovered exactly), the rank of the best human class among all the
//! model's classes (`human_rank`, what BirdNET-Pi's privacy rule reads), plus
//! the first three chunks' samples as
//! little-endian `f32` for comparing against BirdNET-Pi's own resampled input.
//! `tools/parity/compare_rust.py` reads the JSON.
//!
//! Not an assertion: it skips unless every variable is set, so it costs nothing
//! in an ordinary `cargo test`.
//!
//! ```text
//! PARITY_MODEL=BirdNET_v2.4_fp32_dfttrunc.onnx PARITY_LABELS=labels.txt \
//! PARITY_WAVS=a.wav:b.wav PARITY_OUT=out/ \
//!   cargo test --features analytics --test parity_dump -- --nocapture
//! python3 tools/parity/compare_rust.py out/ours.json tools/parity/golden/matrix/a1_station.json
//! ```
//! `PARITY_OVERLAP` (seconds, default 0) sets the chunk overlap.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use birdnet_core::detection::pipeline::{PipelineConfig, process_file};
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

#[test]
fn parity_dump() {
    let (Ok(model_p), Ok(labels_p), Ok(wavs), Ok(out_dir)) = (
        std::env::var("PARITY_MODEL"),
        std::env::var("PARITY_LABELS"),
        std::env::var("PARITY_WAVS"),
        std::env::var("PARITY_OUT"),
    ) else {
        eprintln!("SKIP: PARITY_* unset");
        return;
    };
    let overlap: f32 = std::env::var("PARITY_OVERLAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let stage = out_dir.join("stage");
    std::fs::create_dir_all(&stage).unwrap();

    let labels = LabelSet::load(Path::new(&labels_p)).unwrap();
    let mut model = BirdNetModel::load(
        Path::new(&model_p),
        labels,
        ModelConfig {
            sensitivity: 1.0,
            confidence_threshold: 0.0,
            top_n: 10,
            num_threads: 2,
        },
    )
    .unwrap();
    let cfg = PipelineConfig {
        target_sample_rate: model.infer_sample_rate(),
        raw_audio_input: model.expects_raw_audio(),
        chunk_duration_secs: model.recommended_chunk_secs(),
        chunk_overlap_secs: overlap,
        confidence_threshold: 0.0,
        ..PipelineConfig::default()
    };
    eprintln!(
        "rate={} raw={} chunk_secs={} overlap={overlap}",
        cfg.target_sample_rate, cfg.raw_audio_input, cfg.chunk_duration_secs
    );

    let mut json = String::from("{\n");
    for (n, wav) in wavs.split(':').enumerate() {
        let stem = Path::new(wav)
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let link = stage.join(format!("2026-06-15-birdnet-10:{:02}:00.wav", n % 60));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(wav, &link).unwrap();
        let chunks = process_file(&link, &cfg).unwrap();
        if n > 0 {
            json.push_str(",\n");
        }
        write!(json, "\"{stem}\": [").unwrap();
        for (i, c) in chunks.iter().enumerate() {
            let pred = model
                .predict_chunk(
                    &c.spectrogram.data,
                    "2026-06-15",
                    "10:00:00",
                    c.start_secs,
                    c.end_secs,
                    23,
                )
                .unwrap();
            if i > 0 {
                json.push(',');
            }
            write!(
                json,
                "\n {{\"start\":{},\"end\":{},\"top\":[",
                c.start_secs, c.end_secs
            )
            .unwrap();
            for (k, d) in pred.detections.iter().enumerate() {
                let p = f64::from(d.confidence);
                let logit = (p / (1.0 - p)).ln();
                if k > 0 {
                    json.push(',');
                }
                write!(json, "[\"{}\",{:.6},{:.6}]", d.scientific_name, logit, p).unwrap();
            }
            match pred.human_rank {
                Some(rank) => write!(json, "],\"human_rank\":{rank}}}").unwrap(),
                None => json.push_str("],\"human_rank\":null}"),
            }
            if i < 3 {
                let bytes: Vec<u8> = c
                    .spectrogram
                    .data
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect();
                std::fs::write(out_dir.join(format!("{stem}.chunk{i}.f32")), bytes).unwrap();
            }
        }
        json.push_str("\n]");
    }
    json.push_str("\n}\n");
    std::fs::write(out_dir.join("ours.json"), json).unwrap();
}
