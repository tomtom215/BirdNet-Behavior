# ML Inference

> BirdNET model inference via ONNX Runtime.

## Table of Contents

- [Inference Pipeline](#inference-pipeline)
- [Model Variants](#model-variants)
- [Runtime: ort (ONNX Runtime)](#runtime-ort-onnx-runtime)
- [Inference Code Pattern](#inference-code-pattern)
- [Validation](#validation)
- [Performance](#performance)
- [Hot Reload](#hot-reload)

---

Inference is implemented in `crates/birdnet-core/src/inference/` and wired
into the detection daemon via `src/daemon.rs`.

## Inference Pipeline

```
Audio chunk (f32, model sample rate)
        │
        ▼
    ┌───────────────┐
    │ ONNX Runtime  │   ort crate, C++ core statically linked
    └───────────────┘
        │
        ▼
    Raw logits (f32 vector, thousands of entries)
        │
    sigmoid(sensitivity * logits)
        │
    Top-N species with confidence scores
        │
    Filter by confidence threshold
        │
    Filter by species-occurrence metadata model (optional)
        │
    Detection results → daemon event processor
```

## Model Variants

| Model | Species | Input | Metadata | Notes |
|-------|---------|-------|----------|-------|
| BirdNET+ V3.0 | ~11 000 | 3 s audio @ 48 kHz | Optional | Default (shipped by the installer from Zenodo) |
| BirdNET V2.4 FP16 | 6 362 | 3 s audio @ 48 kHz | Separate metadata model | Legacy compatibility |
| BirdNET V1 | 6 000+ | 3 s audio @ 48 kHz | Lat/lon/week tensor | Legacy |

BirdNET models are supplied as ONNX. Users can drop in either the
upstream TFLite conversion or any ONNX export with the correct input
shape by setting `--model` / `BIRDNET_MODEL` at startup.

## Runtime: `ort` (ONNX Runtime)

The `ort` crate (v2.0.0-rc) wraps Microsoft's ONNX Runtime. Configured
features in the workspace manifest:

- `std` — standard library support
- `ndarray` — tensor inputs and outputs via the `ndarray` crate
- `download-binaries` — auto-fetch platform-specific ONNX Runtime binaries
- `tls-rustls` — rustls-based TLS for the binary download (no OpenSSL)
- `copy-dylibs` — copy platform dylibs when they exist (release images
  still statically link the runtime)

ONNX Runtime on ARM64 provides NEON acceleration and thread-pool tuning
suitable for constrained Raspberry Pi hardware.

## Inference Code Pattern

```rust
use ort::session::Session;
use ort::value::Tensor;

pub struct BirdNetModel {
    session: Session,
    labels: LabelSet,
    sensitivity: f32,
    confidence_threshold: f32,
    top_n: usize,
}

impl BirdNetModel {
    pub fn load(config: ModelConfig, model_path: &Path, labels: LabelSet) -> Result<Self, InferenceError> {
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(config.num_threads)?
            .commit_from_file(model_path)?;
        Ok(Self { session, labels, /* … */ })
    }

    pub fn predict(&self, audio: &[f32]) -> Result<Vec<Detection>, InferenceError> {
        let input = Tensor::from_array(([1, audio.len()], audio.to_vec()))?;
        let outputs = self.session.run(ort::inputs![input])?;
        let (_, logits) = outputs[0].try_extract_tensor::<f32>()?;
        Ok(self.post_process(logits))
    }
}
```

The model is held inside an `Arc<Model>` in the detection daemon so it
can be shared across concurrent detection chunks without cloning the
underlying session.

## Validation

- Unit tests in `crates/birdnet-core/src/inference/` cover label parsing,
  species filter metadata lookup, sigmoid post-processing, and shape
  extraction.
- End-to-end tests in `tests/inference_e2e.rs` run a real WAV fixture
  (`tests/testdata/Pica_pica_30s.wav`) through the full pipeline and
  assert a Eurasian Magpie detection at expected confidence.

## Performance

| Metric | Python (TFLite) | Rust (ort) |
|--------|----------------|------------|
| Model load time | 2–5 s | < 1 s |
| Inference (3 s clip, Pi 5) | 1–2 s | 0.3–0.8 s |
| Inference (3 s clip, Pi 4) | 2–4 s | 0.8–1.5 s |
| Memory (model loaded) | ~200 MB | ~50 MB |

## Hot Reload

Model updates without a service restart:

- Watch the model file for changes via `notify`
- Load the new session on a background task
- Swap atomically via `Arc` replacement
- Validate that the new session produces reasonable output before
  committing the swap

---

[← Audio Pipeline](05-audio-pipeline.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Database →](07-database.md)
