//! Minimal PNG writer (no external image crate required).

/// Write a minimal RGBA PNG to `output`.
pub fn write_png_rgba(
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
    write_png_chunk(output, *b"IHDR", &ihdr)?;

    // IDAT chunk: filter + zlib compress scanlines
    let row_bytes = (width as usize) * 4;
    let mut raw: Vec<u8> = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0); // None filter
        raw.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
    }
    let compressed = zlib_compress(&raw);
    write_png_chunk(output, *b"IDAT", &compressed)?;

    // IEND chunk
    write_png_chunk(output, *b"IEND", &[])?;
    Ok(())
}

fn write_png_chunk(
    output: &mut Vec<u8>,
    chunk_type: [u8; 4],
    data: &[u8],
) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::cast_possible_wrap,
        clippy::cast_lossless
    )]
    let len = data.len() as u32;
    output.write_all(&len.to_be_bytes())?;
    output.write_all(&chunk_type)?;
    output.write_all(data)?;
    // CRC: CRC32 of chunk_type + data
    let crc = crc32(chunk_type, data);
    output.write_all(&crc.to_be_bytes())?;
    Ok(())
}

/// DEFLATE level for the IDAT stream.
///
/// Level 6 is zlib's default and the knee of the curve here: measured against
/// a real 975 × 128 spectrogram's scanlines, level 6 gives 67 268 bytes and
/// level 9 gives 66 686 — 0.9 % more compression for materially more CPU, on a
/// Raspberry Pi, inside a request. Take the 6.
const DEFLATE_LEVEL: u32 = 6;

/// zlib-compress the filtered scanlines.
///
/// This used to emit **stored** (type-0) DEFLATE blocks, with the comment
/// "not great compression but no dependency and correct output". The output was
/// indeed correct, and the cost was 7.5×: a served `/api/v2/spectrogram/…`
/// response measured 499 431 bytes whose IDAT re-deflated to 66 686, and
/// `/recordings` renders twenty thumbnails, so one page load moved 3.28 MB of
/// PNG where 0.53 MB would do. The 32 MiB render cache held ~200 thumbnails
/// instead of ~1 200.
///
/// The "no dependency" half stopped being true some time ago: `flate2` and
/// `miniz_oxide` are already resolved in `Cargo.lock` as build-dependencies of
/// `libduckdb-sys`, so this takes a version the lockfile already vets, in a
/// pure-Rust backend with no C and no build step.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;

    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(DEFLATE_LEVEL));
    // Writing to a `Vec` is infallible, and so is the encoder's own buffering;
    // `expect` here documents that rather than propagating an error no caller
    // could act on.
    encoder
        .write_all(data)
        .expect("writing to a Vec cannot fail");
    encoder.finish().expect("writing to a Vec cannot fail")
}

/// CRC-32 table (polynomial `0xEDB88320`), built once per process.
///
/// It used to be rebuilt inside `crc32`, which is called once per PNG chunk —
/// 2 048 iterations of table construction per image, for a table that is a
/// constant.
static CRC32_TABLE: std::sync::LazyLock<[u32; 256]> = std::sync::LazyLock::new(|| {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    table
});

fn crc32(chunk_type: [u8; 4], data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc = CRC32_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spectrogram-shaped raster: mostly dark, with a few bright bands.
    ///
    /// Real spectrograms compress to roughly an eighth — measured on a served
    /// `/api/v2/spectrogram/…` response: 499 328 raw scanline bytes down to
    /// 66 686. This fixture is deliberately in that family rather than random
    /// noise, because a compression assertion against random data would be
    /// asserting the impossible.
    fn spectrogram_like(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let band = u8::from(y % 17 == 0 && x % 3 != 0);
                let v = if band == 1 { 200 } else { 12 };
                px.extend_from_slice(&[v, v / 2, v / 3, 255]);
            }
        }
        px
    }

    /// Extract the concatenated IDAT payload from an encoded PNG.
    fn idat_of(png: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 8; // past the signature
        while i + 8 <= png.len() {
            let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
            let ty = &png[i + 4..i + 8];
            if ty == b"IDAT" {
                out.extend_from_slice(&png[i + 8..i + 8 + len]);
            }
            i += 12 + len;
        }
        out
    }

    /// The encoder must actually compress.
    ///
    /// It did not: `zlib_compress` emitted type-0 (stored) DEFLATE blocks, so
    /// every spectrogram shipped at its raw size — 499 431 bytes for one
    /// 975 × 128 image, and 3.28 MB for the twenty thumbnails on
    /// `/recordings`. The bound below is deliberately loose (half) rather than
    /// tuned to the observed eighth: the point is to catch a regression to
    /// *stored*, not to pin a ratio that a filter change could legitimately
    /// move.
    #[test]
    fn idat_is_compressed_not_stored() {
        let (w, h) = (320_u32, 128_u32);
        let mut png = Vec::new();
        write_png_rgba(&mut png, w, h, &spectrogram_like(w, h)).expect("encode");

        let raw_len = ((w as usize) * 4 + 1) * h as usize;
        let idat = idat_of(&png);
        assert!(
            idat.len() * 2 < raw_len,
            "IDAT is {} bytes for {raw_len} bytes of scanlines — that is stored, \
             not deflated",
            idat.len()
        );
    }

    /// …and the compressed stream must still be the exact bytes that went in.
    ///
    /// The counterpart to the assertion above: a truncating encoder would
    /// satisfy "smaller" perfectly.
    #[test]
    fn idat_inflates_back_to_the_filtered_scanlines() {
        use std::io::Read as _;

        let (w, h) = (64_u32, 24_u32);
        let pixels = spectrogram_like(w, h);
        let mut png = Vec::new();
        write_png_rgba(&mut png, w, h, &pixels).expect("encode");

        let row_bytes = (w as usize) * 4;
        let mut expected = Vec::with_capacity((row_bytes + 1) * h as usize);
        for row in 0..h as usize {
            expected.push(0); // None filter, as the encoder writes
            expected.extend_from_slice(&pixels[row * row_bytes..(row + 1) * row_bytes]);
        }

        let mut got = Vec::new();
        flate2::read::ZlibDecoder::new(&idat_of(&png)[..])
            .read_to_end(&mut got)
            .expect("IDAT is a valid zlib stream");
        assert_eq!(
            got, expected,
            "round-trip through the encoder changed bytes"
        );
    }

    /// The CRC of a chunk has to be right, or every decoder rejects the file.
    /// Pinned against a known vector so a table refactor cannot quietly break
    /// it.
    #[test]
    fn iend_chunk_crc_matches_the_png_specs_constant() {
        // The IEND chunk is fixed by the spec: length 0, type "IEND",
        // CRC 0xAE426082.
        let mut png = Vec::new();
        write_png_rgba(&mut png, 1, 1, &[0, 0, 0, 255]).expect("encode");
        assert!(
            png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]),
            "IEND chunk or its CRC is wrong"
        );
    }
}
