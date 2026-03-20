//! Spectrogram generation and encoding.

use super::colormap::viridis;
use super::font::draw_text;
use super::png::write_png_rgba;

/// Optional label to overlay on the spectrogram.
#[derive(Debug)]
pub(crate) struct SpectrogramLabel {
    /// Species name.
    pub species: String,
    /// Confidence percentage (0-100).
    pub confidence_pct: u32,
    /// Detection time.
    pub time: String,
}

/// Generate a PNG-encoded spectrogram from a WAV file (no label).
#[cfg(test)]
pub(super) fn generate_spectrogram_png(path: &std::path::Path) -> Result<Vec<u8>, String> {
    generate_spectrogram_png_with_label(path, None)
}

/// Generate a PNG spectrogram with an optional text overlay.
pub(crate) fn generate_spectrogram_png_with_label(
    path: &std::path::Path,
    label: Option<&SpectrogramLabel>,
) -> Result<Vec<u8>, String> {
    use birdnet_core::audio::decode::decode_file;
    use birdnet_core::audio::spectrogram::{MelConfig, mel_spectrogram};

    // Decode audio file to samples.
    let audio = decode_file(path).map_err(|e| format!("decode error: {e}"))?;

    if audio.samples.is_empty() {
        return Err("empty audio file".to_string());
    }

    // Compute mel spectrogram.
    let config = MelConfig {
        n_fft: 512,
        hop_length: 128,
        n_mels: 128,
        fmin: 0.0,
        fmax: Some(audio.sample_rate as f32 / 2.0),
        power: 2.0,
    };

    let mel = mel_spectrogram(&audio.samples, audio.sample_rate, &config)
        .map_err(|e| format!("spectrogram error: {e}"))?;

    // Convert to dB.
    let mel_db = mel.to_db(1.0, 80.0);

    // Extract into row-major Vec<Vec<f32>>.
    let spec: Vec<Vec<f32>> = (0..mel_db.n_mels)
        .map(|m| (0..mel_db.n_frames).map(|f| mel_db.get(m, f)).collect())
        .collect();

    // Encode to PNG with optional label.
    encode_spectrogram_png_labeled(&spec, label)
}

/// Encode a 2D mel spectrogram as PNG (no label).
#[cfg(test)]
pub(super) fn encode_spectrogram_png(spec: &[Vec<f32>]) -> Result<Vec<u8>, String> {
    encode_spectrogram_png_labeled(spec, None)
}

/// Encode a spectrogram with an optional text overlay.
fn encode_spectrogram_png_labeled(
    spec: &[Vec<f32>],
    label: Option<&SpectrogramLabel>,
) -> Result<Vec<u8>, String> {
    if spec.is_empty() || spec[0].is_empty() {
        return Err("empty spectrogram".to_string());
    }

    let height = spec.len() as u32;
    let width = spec[0].len() as u32;

    // Find global min/max for normalisation.
    let (min_val, max_val) = spec
        .iter()
        .flat_map(|row| row.iter().copied())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), v| {
            (mn.min(v), mx.min(mx).max(v))
        });
    let range = (max_val - min_val).max(1e-6);

    // Build RGBA pixel buffer (viridis-like: dark blue -> green -> yellow).
    let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);

    // Spectrogram rows are low-frequency first; for display flip vertically.
    for row in spec.iter().rev() {
        for &val in row {
            let t = ((val - min_val) / range).clamp(0.0, 1.0);
            let (r, g, b) = viridis(t);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }

    // Overlay text label if provided.
    if let Some(lbl) = label {
        let text = format!("{} ({}%) {}", lbl.species, lbl.confidence_pct, lbl.time);
        draw_text(&mut pixels, width, height, 4, 4, &text);
    }

    // Encode to PNG using a minimal hand-rolled writer to avoid adding a heavy dependency.
    let mut output = Vec::new();
    write_png_rgba(&mut output, width, height, &pixels)
        .map_err(|e| format!("PNG encode error: {e}"))?;
    Ok(output)
}
