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
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            clippy::cast_possible_wrap,
            clippy::cast_lossless
        )]
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

pub(super) fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn crc32(chunk_type: [u8; 4], data: &[u8]) -> u32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler32_empty() {
        assert_eq!(adler32(&[]), 1);
    }
}
