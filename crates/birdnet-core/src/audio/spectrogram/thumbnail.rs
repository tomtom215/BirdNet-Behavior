//! Spectrogram **thumbnail** rendering: a small PNG preview of a clip.
//!
//! The Recordings "Clips" grid shows one small spectrogram per saved clip so
//! the eye can scan a morning's recordings at a glance — a loud dawn chorus
//! reads differently from a single distant call. This module turns mono audio
//! samples into an RGB PNG using the same librosa-compatible mel pipeline the
//! live view uses ([`super::mel_spectrogram`] → [`super::MelSpectrogram::to_db`]),
//! then maps the normalized dB grid through a perceptual **magma** colormap and
//! encodes it with the pure-Rust `png` crate (no C dependency).
//!
//! It is deliberately self-contained and synchronous: the web layer calls it
//! from a `spawn_blocking` task to generate-and-cache a thumbnail the first time
//! a clip is viewed, so there is no schema change and historical clips get a
//! preview too. Frequency increases upward (low at the bottom), time runs left
//! to right — the conventional spectrogram orientation.

use std::path::Path;

use crate::audio::decode::{DecodeError, SPECTROGRAM_DECODE_SAMPLE_CAP, decode_file_capped};

use super::{MelConfig, SpectrogramError, mel_spectrogram};

/// Mel parameters tuned for a small visual preview (not model inference).
///
/// A 1024-point FFT with a 256-sample hop gives enough time resolution that a
/// few-second clip yields hundreds of frames to downsample from, while staying
/// cheap on a Raspberry Pi. 128 mel bands match the live view.
const THUMB_MEL: MelConfig = MelConfig {
    n_fft: 1024,
    hop_length: 256,
    n_mels: 128,
    fmin: 0.0,
    fmax: None,
    power: 2.0,
};

/// Dynamic range (dB below peak) the colormap spans, matching the live view.
const TOP_DB: f32 = 80.0;

/// Anchor colours of the **magma** perceptual colormap (matplotlib), evenly
/// spaced over `[0, 1]`. Magma runs near-black → violet → magenta → orange →
/// pale cream, so quiet bins stay dark on the app's dark surface and loud bins
/// glow — a perceptually-uniform ramp that reads honestly without a legend.
const MAGMA: [[u8; 3]; 5] = [
    [0, 0, 4],       // 0.00  near-black
    [81, 18, 124],   // 0.25  deep violet
    [183, 55, 121],  // 0.50  magenta
    [252, 137, 97],  // 0.75  orange
    [252, 253, 191], // 1.00  pale cream
];

/// Errors from thumbnail rendering.
#[derive(Debug)]
pub enum ThumbnailError {
    /// Requested output width or height was zero.
    InvalidDimensions,
    /// The decoded audio had no samples.
    Empty,
    /// The source file could not be decoded.
    Decode(DecodeError),
    /// The mel spectrogram could not be computed (e.g. clip shorter than the
    /// FFT window).
    Spectrogram(SpectrogramError),
    /// PNG encoding failed.
    Encode(String),
}

impl std::fmt::Display for ThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => write!(f, "thumbnail dimensions must be non-zero"),
            Self::Empty => write!(f, "decoded audio is empty"),
            Self::Decode(e) => write!(f, "decode: {e}"),
            Self::Spectrogram(e) => write!(f, "spectrogram: {e}"),
            Self::Encode(e) => write!(f, "png encode: {e}"),
        }
    }
}

impl std::error::Error for ThumbnailError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Spectrogram(e) => Some(e),
            _ => None,
        }
    }
}

/// Map a normalized intensity in `[0, 1]` to an RGB triple via the magma ramp.
///
/// Values outside `[0, 1]` are clamped. Linear interpolation between the five
/// anchors is smooth enough for a small thumbnail.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn magma(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    // Position along the (N-1) segments between anchors.
    let scaled = t * (MAGMA.len() - 1) as f32;
    let lo = (scaled.floor() as usize).min(MAGMA.len() - 2);
    let frac = scaled - lo as f32;
    let a = MAGMA[lo];
    let b = MAGMA[lo + 1];
    [
        lerp_u8(a[0], b[0], frac),
        lerp_u8(a[1], b[1], frac),
        lerp_u8(a[2], b[2], frac),
    ]
}

/// Linear interpolation between two bytes, rounded to nearest.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn lerp_u8(a: u8, b: u8, frac: f32) -> u8 {
    let a = f32::from(a);
    let b = f32::from(b);
    (b - a).mul_add(frac, a).round().clamp(0.0, 255.0) as u8
}

/// Render a spectrogram thumbnail PNG from mono `f32` audio samples.
///
/// The output is a `width`×`height` 8-bit RGB PNG with frequency increasing
/// upward and time left-to-right. The mel grid is normalized to its own
/// `[0, 1]` dynamic range (over finite values) and downsampled by
/// nearest-neighbour to the requested pixel grid, so any clip length produces a
/// crisp, consistently-sized tile.
///
/// # Errors
///
/// Returns [`ThumbnailError::InvalidDimensions`] for a zero dimension,
/// [`ThumbnailError::Spectrogram`] when the clip is shorter than the FFT window,
/// or [`ThumbnailError::Encode`] if PNG encoding fails.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
pub fn render_png(
    samples: &[f32],
    sample_rate: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ThumbnailError> {
    if width == 0 || height == 0 {
        return Err(ThumbnailError::InvalidDimensions);
    }

    let mel =
        mel_spectrogram(samples, sample_rate, &THUMB_MEL).map_err(ThumbnailError::Spectrogram)?;
    let db = mel.to_db(1.0, TOP_DB);

    // Normalize over finite values, mirroring the live view: a single non-finite
    // sample (an overflowing power on an edge frame) must not wipe the tile.
    let (min_val, range) = finite_range(&db.data);

    let n_mels = db.n_mels;
    let n_frames = db.n_frames.max(1);
    let w = width as usize;
    let h = height as usize;
    // Denominators guard the 1-pixel edge so we never divide by zero.
    let denom_x = (w - 1).max(1) as f32;
    let denom_y = (h - 1).max(1) as f32;

    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        // Image row 0 is the top (highest frequency); flip so low freq sits at
        // the bottom — the conventional spectrogram orientation.
        let fy = 1.0 - (y as f32 / denom_y);
        let mel_idx = ((fy * (n_mels.saturating_sub(1)) as f32).round() as usize).min(n_mels - 1);
        for x in 0..w {
            let fx = x as f32 / denom_x;
            let frame = ((fx * (n_frames - 1) as f32).round() as usize).min(n_frames - 1);
            let raw = db.data[mel_idx * db.n_frames + frame];
            let norm = if raw.is_finite() {
                ((raw - min_val) / range).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let [r, g, b] = magma(norm);
            let o = (y * w + x) * 3;
            rgb[o] = r;
            rgb[o + 1] = g;
            rgb[o + 2] = b;
        }
    }

    encode_png(&rgb, width, height)
}

/// Decode an audio file (capped) and render its spectrogram thumbnail.
///
/// A thin convenience over [`render_png`] used by the web layer's
/// generate-and-cache path. The decode is bounded by
/// [`SPECTROGRAM_DECODE_SAMPLE_CAP`] so an over-long recording can't allocate an
/// unbounded buffer on a small Pi.
///
/// # Errors
///
/// Returns [`ThumbnailError::Decode`] when the file cannot be decoded,
/// [`ThumbnailError::Empty`] for a zero-sample file, or any error from
/// [`render_png`].
pub fn render_file_png(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, ThumbnailError> {
    let audio =
        decode_file_capped(path, SPECTROGRAM_DECODE_SAMPLE_CAP).map_err(ThumbnailError::Decode)?;
    if audio.samples.is_empty() {
        return Err(ThumbnailError::Empty);
    }
    render_png(&audio.samples, audio.sample_rate, width, height)
}

/// Compute `(min, range)` over the finite values of a dB grid, with `range`
/// floored at a small epsilon so a flat (silent) clip maps to all-zero rather
/// than dividing by zero.
fn finite_range(data: &[f32]) -> (f32, f32) {
    let min_val = data
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::INFINITY, f32::min);
    let max_val = data
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    // All non-finite (or empty) → neutral range so every pixel maps to 0.
    if !min_val.is_finite() || !max_val.is_finite() {
        return (0.0, 1.0);
    }
    (min_val, (max_val - min_val).max(1e-6))
}

/// Encode an RGB8 buffer as a PNG.
fn encode_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ThumbnailError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ThumbnailError::Encode(e.to_string()))?;
        writer
            .write_image_data(rgb)
            .map_err(|e| ThumbnailError::Encode(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::suboptimal_flops
)]
mod tests {
    use super::*;

    /// A 1 s sine wave at the given frequency, mono.
    fn sine(freq: f32, sample_rate: u32, secs: f32) -> Vec<f32> {
        let n = (secs * sample_rate as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    /// The first eight bytes of every PNG file.
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

    #[test]
    fn renders_a_valid_png_of_the_requested_size() {
        let samples = sine(1000.0, 48_000, 1.0);
        let png = render_png(&samples, 48_000, 64, 32).expect("render");
        assert!(png.starts_with(&PNG_MAGIC), "output is not a PNG");

        // The IHDR width/height are big-endian u32 at bytes 16..24.
        let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((w, h), (64, 32), "IHDR dimensions must match request");
    }

    #[test]
    fn rendering_is_deterministic() {
        let samples = sine(2000.0, 48_000, 1.0);
        let a = render_png(&samples, 48_000, 48, 24).expect("a");
        let b = render_png(&samples, 48_000, 48, 24).expect("b");
        assert_eq!(a, b, "same input must yield byte-identical PNGs");
    }

    #[test]
    fn zero_dimension_is_rejected() {
        let samples = sine(1000.0, 48_000, 1.0);
        assert!(matches!(
            render_png(&samples, 48_000, 0, 10),
            Err(ThumbnailError::InvalidDimensions)
        ));
        assert!(matches!(
            render_png(&samples, 48_000, 10, 0),
            Err(ThumbnailError::InvalidDimensions)
        ));
    }

    #[test]
    fn clip_shorter_than_fft_window_errors() {
        // Fewer samples than n_fft (1024) → spectrogram InputTooShort.
        let samples = vec![0.0_f32; 100];
        assert!(matches!(
            render_png(&samples, 48_000, 32, 16),
            Err(ThumbnailError::Spectrogram(_))
        ));
    }

    #[test]
    fn silence_renders_without_panicking() {
        // A flat (silent) clip has zero dynamic range; the epsilon floor must
        // keep normalization finite rather than dividing by zero.
        let samples = vec![0.0_f32; 48_000];
        let png = render_png(&samples, 48_000, 40, 20).expect("silence renders");
        assert!(png.starts_with(&PNG_MAGIC));
    }

    #[test]
    fn magma_endpoints_and_monotonic_luminance() {
        // Endpoints pinned to the magma anchors.
        assert_eq!(magma(0.0), [0, 0, 4]);
        assert_eq!(magma(1.0), [252, 253, 191]);
        // Out-of-range clamps, doesn't panic.
        assert_eq!(magma(-1.0), [0, 0, 4]);
        assert_eq!(magma(2.0), [252, 253, 191]);

        // Perceived luminance rises monotonically across the ramp — the
        // property that makes louder bins read as brighter.
        let lum = |c: [u8; 3]| {
            0.299 * f32::from(c[0]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[2])
        };
        let mut prev = -1.0;
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let l = lum(magma(t));
            assert!(l >= prev - 0.5, "luminance dipped at t={t}: {l} < {prev}");
            prev = l;
        }
    }

    #[test]
    fn one_pixel_thumbnail_does_not_divide_by_zero() {
        let samples = sine(1000.0, 48_000, 1.0);
        let png = render_png(&samples, 48_000, 1, 1).expect("1x1 renders");
        assert!(png.starts_with(&PNG_MAGIC));
    }

    #[test]
    fn distinct_tones_produce_distinct_thumbnails() {
        // A low tone and a high tone put energy in different mel bands, so the
        // rendered tiles must differ — proof the freq axis actually maps.
        let low = render_png(&sine(300.0, 48_000, 1.0), 48_000, 48, 48).unwrap();
        let high = render_png(&sine(8000.0, 48_000, 1.0), 48_000, 48, 48).unwrap();
        assert_ne!(low, high, "different tones should render differently");
    }
}
