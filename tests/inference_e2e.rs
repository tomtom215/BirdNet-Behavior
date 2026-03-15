//! End-to-end inference test: decode → resample → model → detect Pica pica.
//!
//! Requires the BirdNET+ V3.0 model and labels to be present at the paths
//! set by the environment variables:
//!
//!   BIRDNET_TEST_MODEL  — path to the .onnx file
//!   BIRDNET_TEST_LABELS — path to the labels .csv file
//!
//! The test is automatically skipped when those variables are unset, so the
//! regular CI suite is not affected.
//!
//! Run manually with:
//!   BIRDNET_TEST_MODEL=/tmp/birdnet_models/BirdNET_V3_FP32.onnx \
//!   BIRDNET_TEST_LABELS=/tmp/birdnet_labels.csv \
//!   cargo test --test inference_e2e -- --nocapture

use std::path::Path;

use birdnet_core::audio::decode;
use birdnet_core::audio::resample;
use birdnet_core::inference::labels::LabelSet;
use birdnet_core::inference::model::{BirdNetModel, ModelConfig};

const PICA_PICA_WAV: &str = "tests/testdata/Pica_pica_30s.wav";

/// Load model + labels from env vars; return None to skip if not set.
fn load_model() -> Option<BirdNetModel> {
    let model_path = std::env::var("BIRDNET_TEST_MODEL").ok()?;
    let labels_path = std::env::var("BIRDNET_TEST_LABELS").ok()?;

    let model_path = Path::new(&model_path);
    let labels_path = Path::new(&labels_path);

    if !model_path.exists() {
        eprintln!("SKIP: model not found at {}", model_path.display());
        return None;
    }
    if !labels_path.exists() {
        eprintln!("SKIP: labels not found at {}", labels_path.display());
        return None;
    }

    let labels = LabelSet::load(labels_path).expect("failed to load labels");
    eprintln!("Labels loaded: {} species", labels.len());

    let config = ModelConfig {
        confidence_threshold: 0.1, // low threshold to catch Pica pica reliably
        ..ModelConfig::default()
    };

    let model = BirdNetModel::load(model_path, labels, config).expect("failed to load model");
    eprintln!(
        "Model loaded: input_shape={:?}, sample_rate={}Hz, raw_audio={}",
        model.input_shape(),
        model.infer_sample_rate(),
        model.expects_raw_audio()
    );

    Some(model)
}

#[test]
fn e2e_labels_load_pica_pica_present() {
    let labels_path = match std::env::var("BIRDNET_TEST_LABELS") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("SKIP: BIRDNET_TEST_LABELS not set");
            return;
        }
    };

    let labels = LabelSet::load(Path::new(&labels_path)).expect("failed to load labels");
    assert!(labels.len() > 1000, "expected >1000 species, got {}", labels.len());

    let pica = labels.find_by_scientific_name("Pica pica");
    assert!(
        pica.is_some(),
        "Pica pica (Eurasian Magpie) not found in labels — check label file"
    );
    let pica = pica.unwrap();
    eprintln!(
        "Pica pica found at index {}: '{}'",
        pica.index, pica.common_name
    );
    assert_eq!(pica.common_name, "Eurasian Magpie");
}

#[test]
fn e2e_model_detects_pica_pica() {
    let Some(mut model) = load_model() else { return };

    let wav_path = Path::new(PICA_PICA_WAV);
    assert!(wav_path.exists(), "test WAV not found at {PICA_PICA_WAV}");

    // Decode the 30-second recording.
    let audio = decode::decode_file(wav_path).expect("failed to decode WAV");
    assert_eq!(audio.sample_rate, 48_000);
    eprintln!(
        "Decoded: {} samples ({:.1}s at {}Hz)",
        audio.samples.len(),
        audio.samples.len() as f64 / audio.sample_rate as f64,
        audio.sample_rate
    );

    // Resample to the rate the model expects (auto-detected from input shape).
    let target_rate = model.infer_sample_rate();
    let samples = if audio.sample_rate == target_rate {
        audio.samples.clone()
    } else {
        resample::resample(&audio.samples, audio.sample_rate, target_rate)
            .expect("resampling failed")
    };
    eprintln!(
        "Resampled: {} samples at {}Hz",
        samples.len(),
        target_rate
    );

    // Split into 3-second chunks and run inference on each.
    let chunk_samples = (3.0 * target_rate as f64) as usize;
    let mut all_detections = Vec::new();

    let start = std::time::Instant::now();

    for (i, chunk_start) in (0..samples.len()).step_by(chunk_samples).enumerate() {
        let chunk_end = (chunk_start + chunk_samples).min(samples.len());
        let mut chunk = samples[chunk_start..chunk_end].to_vec();
        if chunk.len() < chunk_samples {
            chunk.resize(chunk_samples, 0.0); // zero-pad last chunk
        }

        let detections = model
            .predict(&chunk, "2026-03-15", "06:30:00", chunk_start as f32 / target_rate as f32, chunk_end as f32 / target_rate as f32, 11)
            .expect("inference failed");

        for d in &detections {
            eprintln!(
                "  chunk {:2}: {:.1}s-{:.1}s  {:.1}%  {} ({})",
                i,
                chunk_start as f32 / target_rate as f32,
                chunk_end as f32 / target_rate as f32,
                d.confidence * 100.0,
                d.common_name,
                d.scientific_name
            );
        }
        all_detections.extend(detections);
    }

    let elapsed = start.elapsed();
    eprintln!(
        "\nInference complete: {} total detections in {:.1}ms ({:.1}ms/chunk)",
        all_detections.len(),
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1000.0 / (samples.len() / chunk_samples) as f64
    );

    // The recording is a Eurasian Magpie (Pica pica) — it must be detected.
    let pica_detections: Vec<_> = all_detections
        .iter()
        .filter(|d| d.scientific_name == "Pica pica")
        .collect();

    assert!(
        !pica_detections.is_empty(),
        "Expected at least one Pica pica detection in 30s of Magpie audio, got none.\n\
         Top detections:\n{}",
        all_detections
            .iter()
            .take(10)
            .map(|d| format!("  {:.1}%  {} ({})", d.confidence * 100.0, d.common_name, d.scientific_name))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let best = pica_detections
        .iter()
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .unwrap();

    eprintln!(
        "\nPica pica detected {} time(s), best confidence: {:.1}%",
        pica_detections.len(),
        best.confidence * 100.0
    );

    assert!(
        best.confidence > 0.5,
        "Pica pica best confidence {:.1}% is too low (expected >50%)",
        best.confidence * 100.0
    );
}
