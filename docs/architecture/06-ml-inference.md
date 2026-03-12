# ML Inference

> Running BirdNET model inference in Rust.

## Inference Pipeline

```
Mel spectrogram (f32 matrix)
        │
        ▼
    ┌───────────────┐
    │ ONNX Runtime  │  ← Primary (ort crate, C++ backend)
    │ or Tract      │  ← Long-term goal (pure Rust)
    └───────────────┘
        │
        ▼
    Raw logits (f32 vector, 6K+ entries)
        │
    sigmoid(sensitivity * logits)
        │
    Top-N species with confidence scores
        │
    Filter by confidence threshold
        │
    Detection results
```

## Model Variants

BirdNET-Pi supports 4 model variants. All must be supported:

| Model | Species | Input | Metadata | Notes |
|-------|---------|-------|----------|-------|
| BirdNET V2.4 FP16 | 6,362 | 3s audio @ 48kHz | Separate metadata model | Primary model |
| BirdNET V1 | 6,000+ | 3s audio @ 48kHz | Lat/lon/week tensor | Legacy |
| BirdNET-Go v20250916 | 6,362+ | 3s audio @ 48kHz | Extends V2.4 | Community model |
| Perch V2 | Varies | 5s audio @ 32kHz | None | Google research model |

## Model Conversion (TFLite → ONNX)

```bash
pip install tf2onnx onnxruntime
python -m tf2onnx.convert --tflite BirdNET_model.tflite --output BirdNET_model.onnx --opset 18
```

Validate conversion:
```python
# Compare outputs between TFLite and ONNX on same input
import numpy as np
assert np.allclose(tflite_output, onnx_output, atol=1e-4)
```

## Preferred Runtime: tract (Pure Rust)

`tract-onnx` v0.22 by Sonos is a pure Rust ONNX inference engine:

- **Zero C/C++ dependencies** -- trivial cross-compilation to any target
- CPU-only (no GPU), but BirdNET models are small enough for CPU inference
- Passes ~85% of ONNX backend tests; common operators (conv, dense, sigmoid) well supported
- Smaller binary than ort (no ONNX Runtime shared library)
- Works on aarch64, x86_64, and even WebAssembly

**Bridge option:** The `ort-tract` crate provides the `ort` API surface with
tract as the backend, allowing future backend swaps without API changes.

## Fallback Runtime: ort (ONNX Runtime)

The `ort` crate v2.0.0-rc wraps Microsoft's ONNX Runtime:

- ARM64 with NEON acceleration (XNNPACK backend)
- FP16 tensor support via `half` feature flag
- Quantized model support (INT8) for further Pi optimization
- Thread pool configuration for constrained environments
- For aarch64 Linux: supply ONNX Runtime binaries via `ORT_LIB_PATH` (requires glibc >= 2.35)
- Model loading from file or embedded bytes

### Inference Code Pattern

```rust
use ort::{Session, Value};

pub struct BirdNetModel {
    session: Session,
    labels: Vec<String>,
    sensitivity: f32,
}

impl BirdNetModel {
    pub fn load(model_path: &Path, sensitivity: f32) -> Result<Self, InferenceError> {
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_intra_threads(2)?  // Limit threads on Pi
            .commit_from_file(model_path)?;
        // Load labels...
        Ok(Self { session, labels, sensitivity })
    }

    pub fn predict(&self, audio: &[f32]) -> Result<Vec<Detection>, InferenceError> {
        let input = Value::from_array(([1, audio.len()], audio))?;
        let outputs = self.session.run(ort::inputs![input]?)?;
        let logits = outputs[0].try_extract_tensor::<f32>()?;
        // Apply sigmoid with sensitivity, filter, return top-N
    }
}
```

## Validation Plan

Before committing to a runtime, validate both tract and ort:

1. Convert BirdNET TFLite model to ONNX (opset 18)
2. Load model in both tract and ort
3. Run same test WAV files through the Rust audio pipeline
4. Compare inference outputs against Python TFLite (must match within 1e-4)
5. Benchmark latency on Pi 4/5 (target: <1s per 3-second chunk)
6. If tract matches accuracy, prefer it (pure Rust, simpler deployment)
7. If tract has accuracy issues, fall back to ort

## Inference Chain (Matching Python)

```
1. Load audio file → f32 samples (symphonia)
2. Resample to model sample rate (rubato)
3. Split into chunks (3s for BirdNET, 5s for Perch) with overlap
4. Pad short chunks with zeros
5. Generate mel spectrogram for each chunk
6. Run inference → raw logits
7. sigmoid(sensitivity * logits) → confidence scores
8. Top-10 species per chunk
9. Filter by confidence threshold
10. Optional: metadata model filters rare species by location
```

## Performance Targets

| Metric | Python (TFLite) | Rust (target) |
|--------|----------------|---------------|
| Model load time | 2-5 seconds | <1 second |
| Inference (3s clip, Pi 5) | 1-2 seconds | 0.3-0.8 seconds |
| Inference (3s clip, Pi 4) | 2-4 seconds | 0.8-1.5 seconds |
| Memory (model loaded) | ~200 MB | ~50 MB |

## Hot Reload

Support model updates without restarting the service:
- Watch model file for changes
- Load new model in background
- Swap atomically (Arc + RwLock pattern)
- Validate new model produces reasonable output before committing

---

[← Audio Pipeline](05-audio-pipeline.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Database →](07-database.md)
