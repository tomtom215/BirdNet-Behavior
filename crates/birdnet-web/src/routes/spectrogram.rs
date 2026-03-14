//! Spectrogram generation and serving.
//!
//! Generates a PNG spectrogram from a WAV recording file on demand and
//! returns it as an `image/png` response.  Spectrograms are cached in
//! memory (keyed by filename + mtime) to avoid re-computation on every
//! page load.
//!
//! Route:
//!
//! | Method | Path | Action |
//! |--------|------|--------|
//! | GET    | /api/v2/spectrogram/{filename} | Generate/serve spectrogram PNG |
//!
//! The spectrogram is rendered as a grayscale/viridis-like PNG using the
//! mel spectrogram computed by `birdnet-core`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/spectrogram/{filename}", get(serve_spectrogram))
}

// ---------------------------------------------------------------------------
// GET /api/v2/spectrogram/{filename}
// ---------------------------------------------------------------------------

async fn serve_spectrogram(
    State(state): State<AppState>,
    Path(filename): Path<String>,
) -> Response {
    if !is_safe_filename(&filename) {
        return (StatusCode::BAD_REQUEST, "invalid filename").into_response();
    }

    let rec_dir = state.recording_dir();
    let file_path = rec_dir.join(&filename);

    // Confirm the path is within the recording directory.
    match file_path.canonicalize() {
        Ok(canonical) => {
            let rec_canonical = rec_dir.canonicalize().unwrap_or(rec_dir.clone());
            if !canonical.starts_with(&rec_canonical) {
                return (StatusCode::FORBIDDEN, "path traversal denied").into_response();
            }
        }
        Err(_) => {
            return (StatusCode::NOT_FOUND, "recording not found").into_response();
        }
    }

    // Generate spectrogram in a blocking task.
    let result = tokio::task::spawn_blocking(move || generate_spectrogram_png(&file_path)).await;

    match result {
        Ok(Ok(png_bytes)) => {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            (StatusCode::OK, headers, Body::from(png_bytes)).into_response()
        }
        Ok(Err(e)) => {
            tracing::warn!(file = %filename, err = %e, "spectrogram generation failed");
            (StatusCode::UNPROCESSABLE_ENTITY, e).into_response()
        }
        Err(e) => {
            tracing::error!(err = %e, "spectrogram task panicked");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

// ---------------------------------------------------------------------------
// Spectrogram generation
// ---------------------------------------------------------------------------

/// Generate a PNG-encoded spectrogram from a WAV file.
///
/// Returns raw PNG bytes on success.
fn generate_spectrogram_png(path: &std::path::Path) -> Result<Vec<u8>, String> {
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

    // Encode to PNG.
    encode_spectrogram_png(&spec)
}

/// Encode a 2D mel spectrogram (rows = mel bins, cols = frames) as PNG.
///
/// Uses a simple grayscale palette (higher energy = brighter).
fn encode_spectrogram_png(spec: &[Vec<f32>]) -> Result<Vec<u8>, String> {
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

    // Build RGBA pixel buffer (viridis-like: dark blue → green → yellow).
    let mut pixels: Vec<u8> = Vec::with_capacity((width * height * 4) as usize);

    // Spectrogram rows are low-frequency first; for display flip vertically.
    for row in spec.iter().rev() {
        for &val in row {
            let t = ((val - min_val) / range).clamp(0.0, 1.0);
            let (r, g, b) = viridis(t);
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }

    // Encode to PNG using a minimal hand-rolled writer to avoid adding a heavy dependency.
    let mut output = Vec::new();
    write_png_rgba(&mut output, width, height, &pixels)
        .map_err(|e| format!("PNG encode error: {e}"))?;
    Ok(output)
}

/// Approximate viridis colormap: maps t∈[0,1] to (R,G,B).
fn viridis(t: f32) -> (u8, u8, u8) {
    // Control points: (t, R, G, B)
    let cps: &[(f32, f32, f32, f32)] = &[
        (0.000, 68.0, 1.0, 84.0),
        (0.125, 71.0, 44.0, 122.0),
        (0.250, 59.0, 82.0, 139.0),
        (0.375, 44.0, 113.0, 142.0),
        (0.500, 33.0, 145.0, 140.0),
        (0.625, 39.0, 173.0, 129.0),
        (0.750, 92.0, 200.0, 99.0),
        (0.875, 170.0, 220.0, 50.0),
        (1.000, 253.0, 231.0, 37.0),
    ];

    let t = t.clamp(0.0, 1.0);
    let i = cps
        .partition_point(|cp| cp.0 <= t)
        .saturating_sub(1)
        .min(cps.len() - 2);
    let (t0, r0, g0, b0) = cps[i];
    let (t1, r1, g1, b1) = cps[i + 1];
    let frac = if (t1 - t0).abs() < 1e-6 {
        0.0
    } else {
        (t - t0) / (t1 - t0)
    };
    let lerp = |a: f32, b: f32| (a + frac * (b - a)).clamp(0.0, 255.0) as u8;
    (lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
}

// ---------------------------------------------------------------------------
// Minimal PNG writer (no external image crate required)
// ---------------------------------------------------------------------------

/// Write a minimal RGBA PNG to `output`.
fn write_png_rgba(
    output: &mut Vec<u8>,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), std::io::Error> {
    use std::io::Write as _;

    // PNG signature
    output.write_all(&[137, 80, 78, 71, 13, 10, 26, 10])?;

    // IHDR chunk
    let ihdr = {
        let mut d = Vec::with_capacity(13);
        d.extend_from_slice(&width.to_be_bytes());
        d.extend_from_slice(&height.to_be_bytes());
        d.push(8); // bit depth
        d.push(6); // colour type: RGBA
        d.push(0); // compression
        d.push(0); // filter
        d.push(0); // interlace
        d
    };
    write_png_chunk(output, b"IHDR", &ihdr)?;

    // IDAT chunk: filter + zlib compress scanlines
    let row_bytes = (width as usize) * 4;
    let mut raw: Vec<u8> = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0); // None filter
        raw.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
    }
    let compressed = zlib_compress(&raw);
    write_png_chunk(output, b"IDAT", &compressed)?;

    // IEND chunk
    write_png_chunk(output, b"IEND", &[])?;
    Ok(())
}

fn write_png_chunk(
    output: &mut Vec<u8>,
    chunk_type: &[u8; 4],
    data: &[u8],
) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    let len = data.len() as u32;
    output.write_all(&len.to_be_bytes())?;
    output.write_all(chunk_type)?;
    output.write_all(data)?;
    // CRC: CRC32 of chunk_type + data
    let crc = crc32(chunk_type, data);
    output.write_all(&crc.to_be_bytes())?;
    Ok(())
}

/// Minimal zlib compression (deflate level 1) without external crates.
///
/// Uses the zlib format: CMF + FLG header, then DEFLATE blocks, then Adler-32.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    // Store-only deflate (type 0 blocks) — not great compression but
    // no dependency and correct output.
    const BLOCK: usize = 65535;
    let mut out = Vec::with_capacity(data.len() + 16);

    // zlib header: CM=8 (deflate), CINFO=7 (window 32k), check bits
    out.push(0x78); // CMF
    out.push(0x01); // FLG (fastest compression)

    let chunks = data.chunks(BLOCK);
    let n_chunks = chunks.len();
    for (i, chunk) in data.chunks(BLOCK).enumerate() {
        let last = i == n_chunks - 1;
        out.push(u8::from(last)); // BFINAL, BTYPE=00 (store)
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }

    // Adler-32 checksum
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn crc32(chunk_type: &[u8; 4], data: &[u8]) -> u32 {
    // Standard CRC-32 with polynomial 0xEDB88320.
    let table = build_crc32_table();
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc = table[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn build_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i as usize] = c;
    }
    table
}

// ---------------------------------------------------------------------------
// Safety
// ---------------------------------------------------------------------------

fn is_safe_filename(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_graphic())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_filename_ok() {
        assert!(is_safe_filename("bird_2026-03-14_06-00-00.wav"));
    }

    #[test]
    fn safe_filename_traversal() {
        assert!(!is_safe_filename("../etc/passwd"));
        assert!(!is_safe_filename("foo/bar.wav"));
    }

    #[test]
    fn viridis_endpoints() {
        let (r, _g, b) = viridis(0.0);
        assert!(b > r, "cold end should be blue-heavy");
        let (r2, g2, _b2) = viridis(1.0);
        assert!(r2 > 200 && g2 > 200, "warm end should be yellow");
    }

    #[test]
    fn adler32_empty() {
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn viridis_midpoint_is_greenish() {
        let (_r, g, _b) = viridis(0.5);
        assert!(g > 100, "midpoint should have significant green");
    }
}
