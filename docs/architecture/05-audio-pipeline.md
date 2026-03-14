# Audio Pipeline

> Pure Rust, zero C dependencies. Decode, resample, and generate mel spectrograms.

## Table of Contents

- [Pipeline Overview](#pipeline-overview)
- [Audio Decoding (symphonia)](#audio-decoding-symphonia)
- [Resampling (rubato)](#resampling-rubato)
- [Mel Spectrogram](#mel-spectrogram)
- [Audio Capture](#audio-capture)
- [Detection Pipeline](#detection-pipeline)
- [Model-Specific Parameters](#model-specific-parameters)
- [Performance Targets](#performance-targets)

---

## Pipeline Overview

```
Microphone/RTSP → arecord/ffmpeg → WAV files in StreamData/
                                        │
                    notify watches ──────┘
                                        │
                    symphonia decode → f32 samples (mono)   ✅ IMPLEMENTED
                                        │
                    rubato resample → target sample rate     ✅ IMPLEMENTED
                                        │
                    mel spectrogram → f32 matrix             ⚠️ STUBBED
                                        │
                    ort/tract inference → species + confidence scores  ❌ TODO
                                        │
                    SQLite INSERT + BirdWeather POST + email/Apprise   ✅ IMPLEMENTED
```

The entire audio pipeline (symphonia + rubato + mel spectrogram) cross-compiles
trivially to aarch64 with **zero system dependencies**.

## Audio Decoding (symphonia)

**Status: ✅ Implemented** — `crates/birdnet-core/src/audio/decode.rs`

- Pure Rust decoder supporting WAV, FLAC, MP3
- Automatic mono downmix (multi-channel → mono via averaging)
- Returns `AudioData { samples: Vec<f32>, sample_rate: u32 }`
- No system dependencies (no libsndfile, no ffmpeg bindings)

## Resampling (rubato)

**Status: ✅ Implemented** — `crates/birdnet-core/src/audio/resample.rs`

- Async polynomial resampler for high-quality rate conversion
- Handles chunk-based processing with zero-padded remainder
- Smart passthrough when input rate matches target rate
- Primary use: 48kHz microphone input → 16kHz (BirdNET) or 32kHz (Perch)

## Mel Spectrogram

**Status: ⚠️ Stubbed** — `crates/birdnet-core/src/audio/spectrogram.rs`

This is the most critical component for model accuracy. The mel spectrogram
must produce output numerically equivalent to librosa's `melspectrogram()`,
since BirdNET models were trained on librosa-generated features.

### Approach: Pure Rust Implementation

Rather than depending on the `mel_spec` crate, implement the mel spectrogram
pipeline directly using standard Rust with minimal dependencies:

1. **Windowing**: Hann window function (trivial math, ~5 lines)
2. **STFT**: Short-time Fourier transform using `realfft` crate (pure Rust FFT)
3. **Mel filterbank**: Construct mel-scale triangular filters (pure math)
4. **Power spectrum**: Magnitude-squared of STFT output
5. **Log scaling**: `10 * log10(power + 1e-10)` for dB conversion

### Parameters (matching librosa defaults for BirdNET)

```
Sample rate:  48000 Hz (or model-specific)
n_fft:        2048
hop_length:   512
n_mels:       128
fmin:         0 Hz
fmax:         sample_rate / 2
window:       hann
power:        2.0 (power spectrogram)
```

### Validation

- Feed identical WAV files through Python (librosa) and Rust pipelines
- Compare mel spectrogram matrices element-wise
- Must be within 1e-4 tolerance for model accuracy
- Benchmark: expect 5–10x speedup over librosa on equivalent hardware

### Key Implementation Notes

- **Mel scale conversion**: `mel = 2595 * log10(1 + hz / 700)` (HTK formula)
- **Hann window**: `w[n] = 0.5 * (1 - cos(2π * n / (N-1)))`
- **Overlap-add**: Standard STFT with `hop_length` stride
- **Normalization**: Match librosa's `norm="slaney"` mel filter normalization

## Audio Capture

**Status: ⚠️ Stubbed** — `crates/birdnet-core/src/audio/capture.rs`

Audio capture uses subprocess management rather than direct ALSA bindings:

- **Microphone**: `arecord` subprocess with configured format/rate
- **RTSP streams**: `ffmpeg` subprocess with reconnection logic
- **Gap detection**: Monitor for missing files, alert on prolonged silence
- **Disk management**: Rotate old recordings, enforce space limits

This avoids the `cpal` crate dependency and leverages battle-tested system tools.

## Detection Pipeline

**Status: ✅ Implemented** — `crates/birdnet-core/src/detection/pipeline.rs`

The file watcher and event-to-detection pipeline is implemented:

```rust
pub trait DetectionHandler: Send + 'static {
    fn handle(&self, detection: Detection, file: &Path) -> Result<(), HandlerError>;
}
```

- `notify`-based file watcher watches the `StreamData/` directory
- New WAV files trigger the decode → resample → spectrogram → infer → report chain
- `DetectionHandler` trait allows swapping backends (SQLite, test doubles)
- Daemon integration in `src/daemon.rs` wires the pipeline to SQLite + integrations

### Event Processor

The `event_processor` in `src/daemon.rs` handles each detection:

1. Insert into SQLite via `birdnet-db`
2. Post to BirdWeather (if configured)
3. Send email alert (if configured + confidence threshold met)
4. Send Apprise notification (if configured)
5. Broadcast to SSE stream for live dashboard updates

## Model-Specific Parameters

| Model | Sample Rate | Chunk Duration | Input Shape |
|-------|------------|----------------|-------------|
| BirdNET V2.4 FP16 | 48000 Hz | 3 seconds | Audio float32 |
| BirdNET V1 | 48000 Hz | 3 seconds | Audio float32 + metadata |
| Perch V2 | 32000 Hz | 5 seconds | Audio float32 |

## Performance Targets

| Metric | Python (librosa) | Rust (target) |
|--------|-----------------|---------------|
| Decode + resample (3s clip) | ~100 ms | ~10 ms |
| Mel spectrogram (3s clip) | ~50 ms | ~5 ms |
| Total audio pipeline | ~150 ms | ~15 ms |
| Memory per clip | ~50 MB (numpy arrays) | ~1 MB (stack + heap) |

---

*Last updated: 2026-03-14*

[← Dependencies](04-dependencies.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: ML Inference →](06-ml-inference.md)
