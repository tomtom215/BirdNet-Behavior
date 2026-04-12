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
Microphone / RTSP → arecord / ffmpeg → WAV files in StreamData/
                                            │
                    notify watches ─────────┘
                                            │
                    symphonia decode → f32 samples (mono)
                                            │
                    rubato resample → target sample rate
                                            │
                    mel spectrogram (pure Rust realfft)
                                            │
                    ort inference → species + confidence scores
                                            │
                    SQLite INSERT + BirdWeather POST + notifications
```

The entire audio pipeline (symphonia + rubato + realfft) cross-compiles
to aarch64 with **zero system dependencies**.

## Audio Decoding (symphonia)

Implemented in `crates/birdnet-core/src/audio/decode.rs`.

- Pure Rust decoder supporting WAV, FLAC, and MP3
- Automatic mono downmix (multi-channel → mono via averaging)
- Returns `AudioData { samples: Vec<f32>, sample_rate: u32 }`
- No system dependencies — no libsndfile, no ffmpeg bindings

## Resampling (rubato)

Implemented in `crates/birdnet-core/src/audio/resample.rs`.

- Asynchronous polynomial resampler for high-quality rate conversion
- Chunk-based processing with zero-padded remainder
- Smart passthrough when input rate already matches the target
- Primary use: 48 kHz microphone input → 48 kHz (BirdNET+) model input

## Mel Spectrogram

Implemented in `crates/birdnet-core/src/audio/spectrogram/`.

The mel spectrogram must produce output numerically equivalent to
`librosa.melspectrogram()`, since BirdNET models were trained on
librosa-generated features.

### Implementation

The mel spectrogram is built directly from primitives rather than adding
a `mel_spec` dependency:

1. **Windowing** — Hann window function
2. **STFT** — Short-time Fourier transform using the `realfft` crate
3. **Mel filterbank** — Mel-scale triangular filters (HTK formula)
4. **Power spectrum** — magnitude-squared of STFT output
5. **Log scaling** — `10 * log10(power + 1e-10)` for dB conversion

### Parameters (librosa defaults for BirdNET)

```
Sample rate:  48 000 Hz (or model-specific)
n_fft:        2048
hop_length:   512
n_mels:       128
fmin:         0 Hz
fmax:         sample_rate / 2
window:       hann
power:        2.0 (power spectrogram)
```

### Validation

The mel implementation is covered by unit tests that compare against
reference spectrograms generated from librosa on identical WAV inputs.
Matching tolerance is `1e-4`.

### Key implementation notes

- **Mel scale conversion**: `mel = 2595 * log10(1 + hz / 700)` (HTK formula)
- **Hann window**: `w[n] = 0.5 * (1 - cos(2π * n / (N-1)))`
- **Overlap-add**: standard STFT with `hop_length` stride
- **Normalisation**: matches librosa's `norm="slaney"` mel filter normalisation

### Live spectrogram

`crates/birdnet-core/src/audio/spectrogram/live.rs` runs a file-watcher
that produces `SpectrogramFrame` updates pushed over a broadcast channel
to the WebSocket endpoint at `/api/v2/ws/spectrogram` for the live
dashboard.

## Audio Capture

Implemented in `crates/birdnet-core/src/audio/capture/`.

Audio capture uses subprocess management rather than direct ALSA bindings:

- **Microphone** — `arecord` subprocess with configured format / rate
- **PulseAudio / PipeWire** — `parec` subprocess
- **RTSP streams** — `ffmpeg` subprocess with reconnection logic
- **Gap detection** — monitor for missing files and alert on prolonged silence
- **Disk management** — rotate old recordings, enforce space limits
- **tmpfs support** — mount transient audio on tmpfs to reduce SD card wear

This avoids a `cpal` dependency and leverages battle-tested system tools
that are already present on every supported platform.

## Detection Pipeline

Implemented in `crates/birdnet-core/src/detection/pipeline.rs` and
`crates/birdnet-core/src/detection/daemon.rs`.

```rust
pub trait DetectionHandler: Send + 'static {
    fn handle(&self, detection: Detection, file: &Path) -> Result<(), HandlerError>;
}
```

- `notify`-based file watcher watches the `StreamData/` directory
- New WAV files trigger the decode → resample → spectrogram → infer → report chain
- `DetectionHandler` trait allows swapping backends (SQLite, test doubles)
- Daemon integration in `src/daemon.rs` wires the pipeline to SQLite and integrations

### Event processor

The `event_processor` in `src/daemon.rs` handles each detection:

1. Insert into SQLite via `birdnet-db`
2. Post to BirdWeather (if configured)
3. Send email alert (if configured and confidence threshold met)
4. Send Apprise notification (if configured)
5. Publish to MQTT (if configured)
6. Broadcast to WebSocket subscribers for live dashboard updates

## Model-Specific Parameters

| Model | Sample Rate | Chunk Duration | Input Shape |
|-------|------------|----------------|-------------|
| BirdNET+ V3.0 | 48 000 Hz | 3 seconds | Audio float32 |
| BirdNET V2.4 FP16 | 48 000 Hz | 3 seconds | Audio float32 |
| BirdNET V1 | 48 000 Hz | 3 seconds | Audio float32 + metadata |

## Performance Targets

| Metric | Python (librosa) | Rust (target) |
|--------|-----------------|---------------|
| Decode + resample (3 s clip) | ~100 ms | ~10 ms |
| Mel spectrogram (3 s clip) | ~50 ms | ~5 ms |
| Total audio pipeline | ~150 ms | ~15 ms |
| Memory per clip | ~50 MB (numpy arrays) | ~1 MB |

---

[← Dependencies](04-dependencies.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: ML Inference →](06-ml-inference.md)
