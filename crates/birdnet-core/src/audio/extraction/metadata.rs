//! WAV RIFF INFO metadata embedding.
//!
//! After `hound` writes a PCM WAV file, this module appends a standard
//! [RIFF INFO LIST chunk](https://www.daubnet.com/en/file-format-riff) that
//! embeds bird-detection metadata directly in the audio file.  The INFO chunk
//! is readable by any compliant WAV player, audio editor, or metadata tool.
//!
//! # Embedded tags
//!
//! | RIFF ID | Meaning | Content |
//! |---------|---------|---------|
//! | `INAM`  | Title   | Species common name |
//! | `IART`  | Artist  | `"BirdNet-Behavior"` |
//! | `IPRD`  | Product | Species scientific name |
//! | `ICMT`  | Comment | `"Confidence: 87%  [2026-03-23 06:15:00]"` |
//! | `ICRD`  | Created | Detection date (`YYYY-MM-DD`) |
//! | `ISFT`  | Software| `"BirdNet-Behavior"` |
//!
//! # Format compatibility
//!
//! The INFO LIST chunk is written **after** the `data` chunk.  This is
//! valid per the RIFF specification.  Standard WAV decoders skip unknown
//! chunks, so files remain fully playable.  The outer RIFF chunk size is
//! updated in-place to include the new payload.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::path::Path;
//! use birdnet_core::audio::extraction::metadata::{DetectionMeta, embed_wav_metadata};
//!
//! let meta = DetectionMeta {
//!     common_name: "Barn Owl".into(),
//!     scientific_name: "Tyto alba".into(),
//!     confidence: 0.923,
//!     date: "2026-03-23".into(),
//!     time: "21:45:00".into(),
//! };
//!
//! // embed_wav_metadata(Path::new("Barn_Owl-92-2026-03-23.wav"), &meta).unwrap();
//! ```

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors from WAV metadata embedding.
#[derive(Debug)]
pub enum MetaError {
    /// File I/O error.
    Io(io::Error),
    /// File is not a valid RIFF/WAV file.
    NotWav(String),
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "WAV metadata I/O error: {e}"),
            Self::NotWav(msg) => write!(f, "WAV metadata format error: {msg}"),
        }
    }
}

impl std::error::Error for MetaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::NotWav(_) => None,
        }
    }
}

impl From<io::Error> for MetaError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Detection metadata
// ---------------------------------------------------------------------------

/// Metadata to embed in an extracted WAV file.
#[derive(Debug, Clone)]
pub struct DetectionMeta {
    /// Species common name (e.g. `"Barn Owl"`).
    pub common_name: String,
    /// Species scientific name (e.g. `"Tyto alba"`).
    pub scientific_name: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
    /// Detection date (`"YYYY-MM-DD"`).
    pub date: String,
    /// Detection time (`"HH:MM:SS"`).
    pub time: String,
}

// ---------------------------------------------------------------------------
// RIFF INFO chunk builder
// ---------------------------------------------------------------------------

/// Build the raw bytes for a `LIST INFO` chunk containing the given tags.
///
/// Each tag is a 4-byte ASCII identifier followed by a size-prefixed,
/// null-terminated, even-padded string value.
fn build_info_chunk(tags: &[(&[u8; 4], &str)]) -> Vec<u8> {
    // --- Build sub-chunks ---
    let mut sub: Vec<u8> = Vec::new();
    for &(id, value) in tags {
        if value.is_empty() {
            continue;
        }
        // null-terminated, padded to even length
        let mut data: Vec<u8> = value.bytes().take(255).collect();
        data.push(0); // null terminator
        if data.len() % 2 != 0 {
            data.push(0); // padding byte
        }
        // chunk id (4 bytes)
        sub.extend_from_slice(id);
        // chunk size as u32 LE (size of data including null, excluding pad)
        let payload_len = data.len()
            - if data.len() % 2 == 0 && *data.last().unwrap_or(&1) == 0 {
                0
            } else {
                0
            };
        // The chunk size field stores the actual content size (including null terminator)
        // padding byte is written but not counted in the size field
        let content_len = value.len().min(255) + 1; // value + null
        sub.extend_from_slice(&u32::try_from(content_len).unwrap_or(0).to_le_bytes());
        // value bytes + null
        sub.extend_from_slice(&data[..content_len]);
        // padding to even offset
        let _ = payload_len;
        if content_len % 2 != 0 {
            sub.push(0);
        }
    }

    // --- Wrap in LIST INFO ---
    // "LIST" + size (4 + sub.len()) + "INFO" + sub-chunks
    let list_size = 4 + sub.len(); // "INFO" (4) + sub-chunks
    let mut chunk: Vec<u8> = Vec::with_capacity(8 + list_size);
    chunk.extend_from_slice(b"LIST");
    chunk.extend_from_slice(&u32::try_from(list_size).unwrap_or(u32::MAX).to_le_bytes());
    chunk.extend_from_slice(b"INFO");
    chunk.extend_from_slice(&sub);

    chunk
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Embed BirdNet detection metadata into an existing WAV file in-place.
///
/// Appends a RIFF `LIST INFO` chunk to the file and updates the RIFF chunk
/// size field.  The file must be a valid WAV file written by `hound` (or
/// equivalent).
///
/// # Errors
///
/// Returns [`MetaError`] if the file cannot be opened, is not a valid RIFF/WAV
/// file, or a write error occurs.
pub fn embed_wav_metadata(path: &Path, meta: &DetectionMeta) -> Result<(), MetaError> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let conf_pct = (meta.confidence * 100.0).round() as u32;
    let comment = format!(
        "Confidence: {conf_pct}%  [{date} {time}]",
        date = meta.date,
        time = meta.time,
    );

    let tags: [(&[u8; 4], &str); 6] = [
        (b"INAM", &meta.common_name),
        (b"IART", "BirdNet-Behavior"),
        (b"IPRD", &meta.scientific_name),
        (b"ICMT", &comment),
        (b"ICRD", &meta.date),
        (b"ISFT", "BirdNet-Behavior"),
    ];

    let info_chunk = build_info_chunk(&tags);

    // Open for read+write
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;

    // Verify RIFF header
    let mut header = [0u8; 4];
    file.read_exact(&mut header)?;
    if &header != b"RIFF" {
        return Err(MetaError::NotWav(format!(
            "expected RIFF, got {:?}",
            std::str::from_utf8(&header).unwrap_or("???")
        )));
    }

    // Read existing RIFF chunk size
    let mut size_bytes = [0u8; 4];
    file.read_exact(&mut size_bytes)?;
    let riff_size = u32::from_le_bytes(size_bytes);

    // Verify WAVE marker
    let mut wave = [0u8; 4];
    file.read_exact(&mut wave)?;
    if &wave != b"WAVE" {
        return Err(MetaError::NotWav(format!(
            "expected WAVE, got {:?}",
            std::str::from_utf8(&wave).unwrap_or("???")
        )));
    }

    // Seek to end of file and append INFO chunk
    file.seek(SeekFrom::End(0))?;
    file.write_all(&info_chunk)?;

    // Update the RIFF chunk size field (offset 4)
    let new_riff_size =
        riff_size.saturating_add(u32::try_from(info_chunk.len()).unwrap_or(u32::MAX));
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&new_riff_size.to_le_bytes())?;

    tracing::trace!(
        path = %path.display(),
        species = %meta.common_name,
        confidence = meta.confidence,
        info_bytes = info_chunk.len(),
        "embedded WAV metadata"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn make_minimal_wav(path: &Path, sample_rate: u32) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..sample_rate {
            w.write_sample(0_i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn sample_meta() -> DetectionMeta {
        DetectionMeta {
            common_name: "Barn Owl".into(),
            scientific_name: "Tyto alba".into(),
            confidence: 0.923,
            date: "2026-03-23".into(),
            time: "21:45:00".into(),
        }
    }

    #[test]
    fn embed_succeeds_on_valid_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        make_minimal_wav(&path, 48_000);

        let result = embed_wav_metadata(&path, &sample_meta());
        assert!(result.is_ok(), "embed failed: {result:?}");
    }

    #[test]
    fn embedded_file_is_still_readable_by_hound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meta.wav");
        make_minimal_wav(&path, 48_000);
        embed_wav_metadata(&path, &sample_meta()).unwrap();

        // hound must still be able to read it (it skips unknown chunks)
        let reader = hound::WavReader::open(&path).expect("hound should open embedded WAV");
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
    }

    #[test]
    fn file_contains_species_name_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("name.wav");
        make_minimal_wav(&path, 48_000);
        embed_wav_metadata(&path, &sample_meta()).unwrap();

        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();

        let content = String::from_utf8_lossy(&bytes);
        assert!(
            content.contains("Barn Owl"),
            "species name not found in WAV"
        );
        assert!(content.contains("Tyto alba"), "sci name not found in WAV");
        assert!(content.contains("BirdNet-Behavior"), "software tag missing");
    }

    #[test]
    fn riff_size_updated_after_embed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("size.wav");
        make_minimal_wav(&path, 48_000);

        let original_size = std::fs::metadata(&path).unwrap().len();
        embed_wav_metadata(&path, &sample_meta()).unwrap();
        let new_size = std::fs::metadata(&path).unwrap().len();

        assert!(new_size > original_size, "file should grow after embedding");

        // RIFF chunk size (bytes 4-7) should equal file_size - 8
        let mut file = std::fs::File::open(&path).unwrap();
        let mut buf = [0u8; 8];
        file.read_exact(&mut buf).unwrap();
        let riff_size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as u64;
        // RIFF size = total file size - 8 (RIFF header + size field)
        assert_eq!(
            riff_size,
            new_size - 8,
            "RIFF size field not updated correctly"
        );
    }

    #[test]
    fn fails_on_non_wav_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_a_wav.bin");
        std::fs::write(&path, b"This is not a WAV file at all.").unwrap();

        let result = embed_wav_metadata(&path, &sample_meta());
        assert!(result.is_err(), "should fail on non-WAV file");
    }

    #[test]
    fn build_info_chunk_contains_list_marker() {
        let tags: [(&[u8; 4], &str); 1] = [(b"INAM", "Test Bird")];
        let chunk = build_info_chunk(&tags);
        assert_eq!(&chunk[0..4], b"LIST");
        assert_eq!(&chunk[8..12], b"INFO");
        assert!(chunk.contains(&b'T'));
    }

    #[test]
    fn embed_with_special_chars_in_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("special.wav");
        make_minimal_wav(&path, 48_000);

        let meta = DetectionMeta {
            common_name: "Rüppell's Griffon".into(),
            scientific_name: "Gyps rueppelli".into(),
            confidence: 0.75,
            date: "2026-03-23".into(),
            time: "10:00:00".into(),
        };
        // Should not panic, just truncate/encode non-ASCII gracefully
        let result = embed_wav_metadata(&path, &meta);
        assert!(result.is_ok());
    }
}
