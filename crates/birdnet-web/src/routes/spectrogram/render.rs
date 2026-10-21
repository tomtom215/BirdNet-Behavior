//! Spectrogram generation and encoding.

use super::colormap::Colormap;
use super::font::draw_text;
use super::look::Look;
use super::png::write_png_rgba;

/// Optional label to overlay on the spectrogram.
#[derive(Debug)]
pub struct SpectrogramLabel {
    /// Species name.
    pub species: String,
    /// Confidence percentage (0-100).
    pub confidence_pct: u32,
    /// Detection time.
    pub time: String,
}

/// Generate a PNG-encoded spectrogram from a WAV file (no label, full size).
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn generate_spectrogram_png(path: &std::path::Path) -> Result<Vec<u8>, String> {
    generate_spectrogram_png_with_label(path, None, None, Look::default())
}

/// Generate a PNG spectrogram with an optional text overlay.
///
/// When `thumb_width` is `Some(w)`, the spectrogram's time axis is max-pooled
/// down to `w` columns (preserving call structure) and any label is dropped —
/// the Recordings grid asks for these small previews so it doesn't ship a
/// multi-thousand-pixel image per row. `None` renders at native resolution (one
/// column per mel frame), unchanged.
///
/// `look` is the colour map and the dynamic range (see [`Look`]); the default
/// is what this function always drew.
pub fn generate_spectrogram_png_with_label(
    path: &std::path::Path,
    label: Option<&SpectrogramLabel>,
    thumb_width: Option<u32>,
    look: Look,
) -> Result<Vec<u8>, String> {
    use birdnet_core::audio::decode::{SPECTROGRAM_DECODE_SAMPLE_CAP, decode_file_capped};
    use birdnet_core::audio::spectrogram::{MelConfig, mel_spectrogram};

    // Decode audio to samples. The spectrogram is a visual, so an over-long
    // recording is decoded only up to the cap (the endpoint is public and
    // decodes on demand — an unbounded buffer would OOM a Pi).
    let audio = decode_file_capped(path, SPECTROGRAM_DECODE_SAMPLE_CAP)
        .map_err(|e| format!("decode error: {e}"))?;

    if audio.samples.is_empty() {
        return Err("empty audio file".to_string());
    }

    // Compute mel spectrogram.
    let config = MelConfig {
        n_fft: 512,
        hop_length: 128,
        n_mels: 128,
        fmin: 0.0,
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
        fmax: Some(audio.sample_rate as f32 / 2.0),
        power: 2.0,
    };

    let mel = mel_spectrogram(&audio.samples, audio.sample_rate, &config)
        .map_err(|e| format!("spectrogram error: {e}"))?;

    // Convert to dB, flooring everything more than the chosen range below the
    // loudest cell.
    let mel_db = mel.to_db(1.0, f32::from(look.range_db));

    // Extract into row-major Vec<Vec<f32>>.
    let spec: Vec<Vec<f32>> = (0..mel_db.n_mels)
        .map(|m| (0..mel_db.n_frames).map(|f| mel_db.get(m, f)).collect())
        .collect();

    // For a grid thumbnail, collapse the (often thousands of) time columns to a
    // fixed width with max-pooling so transient calls survive the shrink, and
    // drop any overlay — a tiny tile has no room for text. Full renders unchanged.
    if let Some(w) = thumb_width {
        let thumb = downsample_width(&spec, w as usize);
        return encode_spectrogram_png_labeled(&thumb, None, look.colormap);
    }

    // Encode to PNG with optional label.
    encode_spectrogram_png_labeled(&spec, label, look.colormap)
}

/// Max-pool a spectrogram's time axis down to `target_w` columns.
///
/// Each output column takes the maximum over the source columns that map to it,
/// so a brief loud call still lights up its column instead of being averaged
/// away. Frequency rows are untouched. A spectrogram already at or below
/// `target_w` columns is returned unchanged (no upsampling).
fn downsample_width(spec: &[Vec<f32>], target_w: usize) -> Vec<Vec<f32>> {
    if target_w == 0 || spec.is_empty() {
        return spec.to_vec();
    }
    let src_w = spec[0].len();
    if src_w <= target_w {
        return spec.to_vec();
    }
    spec.iter()
        .map(|row| {
            (0..target_w)
                .map(|x| {
                    let x0 = x * src_w / target_w;
                    let x1 = ((x + 1) * src_w / target_w).max(x0 + 1).min(src_w);
                    row[x0..x1]
                        .iter()
                        .copied()
                        .fold(f32::NEG_INFINITY, f32::max)
                })
                .collect()
        })
        .collect()
}

/// Encode a 2D mel spectrogram as PNG (no label).
#[cfg(test)]
#[allow(dead_code)]
pub(super) fn encode_spectrogram_png(spec: &[Vec<f32>]) -> Result<Vec<u8>, String> {
    encode_spectrogram_png_labeled(spec, None, Colormap::default())
}

/// Encode a spectrogram with an optional text overlay.
fn encode_spectrogram_png_labeled(
    spec: &[Vec<f32>],
    label: Option<&SpectrogramLabel>,
    colormap: Colormap,
) -> Result<Vec<u8>, String> {
    let (width, height, pixels) = render_pixels(spec, label, colormap)?;
    // Encode to PNG using a minimal hand-rolled writer to avoid adding a heavy dependency.
    let mut output = Vec::new();
    write_png_rgba(&mut output, width, height, &pixels)
        .map_err(|e| format!("PNG encode error: {e}"))?;
    Ok(output)
}

/// The RGBA pixels of a spectrogram, as `(width, height, pixels)`: the
/// normalisation, the vertical flip, the colour map and the text overlay.
///
/// Split from the PNG encoding so that what the reader *sees* can be pinned
/// byte for byte (`the_pixels_are_what_the_etags_revision_names`) without
/// depending on the deflate implementation.
fn render_pixels(
    spec: &[Vec<f32>],
    label: Option<&SpectrogramLabel>,
    colormap: Colormap,
) -> Result<(u32, u32, Vec<u8>), String> {
    if spec.is_empty() || spec[0].is_empty() {
        return Err("empty spectrogram".to_string());
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let height = spec.len() as u32;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let width = spec[0].len() as u32;

    // Find global min/max for normalisation.
    let (min_val, max_val) = spec
        .iter()
        .flat_map(|row| row.iter().copied())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(mn, mx), v| {
            (mn.min(v), mx.max(v))
        });
    let range = (max_val - min_val).max(1e-6);

    // Build the RGBA pixel buffer through the chosen colour map.
    let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);

    // Spectrogram rows are low-frequency first; for display flip vertically.
    for row in spec.iter().rev() {
        for &val in row {
            let t = ((val - min_val) / range).clamp(0.0, 1.0);
            let (r, g, b) = colormap.rgb(t);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }

    // Overlay text label if provided.
    if let Some(lbl) = label {
        let text = format!("{} ({}%) {}", lbl.species, lbl.confidence_pct, lbl.time);
        draw_text(&mut pixels, width, height, 4, 4, &text);
    }
    Ok((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::{Colormap, SpectrogramLabel, downsample_width, render_pixels};

    /// FNV-1a over the pixel bytes.
    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// The part of a render that is integer-exact and platform-independent: a
    /// fixed synthetic spectrogram, a label, and every colour map.
    fn pixel_hashes() -> Vec<(&'static str, u64)> {
        let spec: Vec<Vec<f32>> = (0..32)
            .map(|r| (0..96).map(|c| ((r * 7 + c * 13) % 97) as f32).collect())
            .collect();
        let label = SpectrogramLabel {
            species: "Parus major".to_owned(),
            confidence_pct: 91,
            time: "06:30:12".to_owned(),
        };
        Colormap::ALL
            .iter()
            .map(|m| {
                let (_, _, px) = render_pixels(&spec, Some(&label), *m).unwrap();
                (m.as_str(), fnv(&px))
            })
            .collect()
    }

    /// The `ETag` names the inputs of a render (`mtime`, label, look) and a
    /// *revision of the renderer*, because the same inputs under a build that
    /// draws differently must not be answered `304` from a browser's copy of the
    /// old picture. A change to the colour tables, the font, the flip or the
    /// normalisation changes these hashes; when this test fails, that is the
    /// signal to raise `RENDER_REVISION` in `spectrogram/mod.rs` and then update
    /// the numbers here, in the same commit.
    ///
    /// What it does not see: the audio-to-dB stage (mel filters, windowing) ahead
    /// of these pixels is floating point and not pinned here, so a change to
    /// `birdnet-core`'s spectrogram needs `RENDER_REVISION` raised by hand.
    #[test]
    fn the_pixels_are_what_the_etags_revision_names() {
        let expected: [(&str, u64); 5] = [
            ("viridis", 317209647005015307),
            ("magma", 4364708519087716287),
            ("inferno", 5989941986992592300),
            ("grey", 16976510620029363965),
            ("grey_inverted", 15880343174023650506),
        ];
        let got = pixel_hashes();
        assert_eq!(
            got, expected,
            "the picture a reader sees changed: raise RENDER_REVISION (spectrogram/mod.rs) \
             and update these hashes"
        );
    }

    #[test]
    fn downsample_width_max_pools_time_and_keeps_rows() {
        // One row, 6 columns → 3 columns: each output is the max of a pair.
        let spec = vec![vec![0.0, 9.0, 1.0, 2.0, 3.0, 8.0]];
        let out = downsample_width(&spec, 3);
        assert_eq!(out.len(), 1, "row count is preserved");
        assert_eq!(out[0], vec![9.0, 2.0, 8.0], "each column is the bin max");
    }

    #[test]
    fn downsample_width_noop_when_already_small() {
        // At or below the target width the spectrogram is returned unchanged —
        // a short clip is never upsampled.
        let spec = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        assert_eq!(downsample_width(&spec, 5), spec);
        assert_eq!(downsample_width(&spec, 2), spec);
    }

    #[test]
    fn downsample_width_preserves_all_rows() {
        let spec = vec![vec![0.0; 100], vec![1.0; 100], vec![2.0; 100]];
        let out = downsample_width(&spec, 10);
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|r| r.len() == 10));
    }
}
