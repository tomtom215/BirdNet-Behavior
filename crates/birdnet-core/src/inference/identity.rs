//! What a detection was made *with*: the identity of the classifier files a
//! run analysed with, as a checksum of their bytes.
//!
//! A detection row says where it was heard, when, at what threshold, with what
//! sensitivity and overlap — and, before this module, nothing about the model
//! that produced it. Two classifiers with different label sets and different
//! calibrations then share one table indistinguishably the day the operator
//! swaps the model file, and a season spanning the swap cannot be split by
//! which model heard what. The station's own record of a model is the bytes
//! on disk, so that is what is hashed: not the filename (renamed freely, and
//! two releases of one architecture are the same size), not a version string
//! nothing verifies.
//!
//! Hashing streams the file in fixed-size reads; the FP32 model is 541 MB and
//! must never be read into memory whole on a 1 GiB budget.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Read granularity for streaming a file through the hasher.
const HASH_READ_BYTES: usize = 1 << 20;

/// The SHA-256 of a file, as 64 lowercase hex characters, and its length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDigest {
    /// Lowercase hex SHA-256 of every byte of the file.
    pub sha256: String,
    /// The file's length in bytes, as hashed.
    pub bytes: u64,
}

/// Stream `path` through SHA-256.
///
/// # Errors
///
/// Any I/O error opening or reading the file. A truncated model reads
/// cleanly and hashes to something the pinned release checksum will not match,
/// which is the point: the digest is a fact about the bytes, not a verdict.
pub fn file_digest(path: &Path) -> std::io::Result<FileDigest> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; HASH_READ_BYTES];
    let mut bytes: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes += n as u64;
    }
    let digest = hasher.finalize();
    let mut sha256 = String::with_capacity(64);
    for byte in &digest {
        // Infallible: writing to a String never errors.
        let _ = write!(sha256, "{byte:02x}");
    }
    Ok(FileDigest { sha256, bytes })
}

/// The classifier a run analysed with, as the checksums of its files.
///
/// Built once at daemon start by [`ModelIdentity::resolve`] and written to the
/// station's `analysis_runs` table, so every detection row of that run can be
/// keyed to exactly these bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelIdentity {
    /// The model file's stem — `BirdNET+_V3.0-preview3_Global_11K_FP32` for the
    /// shipped release — as the operator-facing name of the model. Declared,
    /// not verified: the checksum is the identity, this is its label.
    pub model_name: String,
    /// Where the model was read from.
    pub model_path: PathBuf,
    /// Digest of the model file.
    pub model: FileDigest,
    /// Where the labels were read from.
    pub labels_path: PathBuf,
    /// Digest of the labels file.
    pub labels: FileDigest,
    /// Digest of the occurrence-filter (geo) model, when the run has one. It
    /// is part of the identity because it decides which species are candidates
    /// at all; two runs with one classifier and different geomodels admit
    /// different species from the same audio.
    pub geomodel: Option<FileDigest>,
}

impl ModelIdentity {
    /// Hash the model, labels and (when configured) geomodel at `paths`.
    ///
    /// # Errors
    ///
    /// The first file that cannot be read, with its path in the error so the
    /// log line names the file and not just "hash failed".
    pub fn resolve(
        model_path: &Path,
        labels_path: &Path,
        geomodel_path: Option<&Path>,
    ) -> Result<Self, IdentityError> {
        let digest_of = |path: &Path| {
            file_digest(path).map_err(|source| IdentityError {
                path: path.to_path_buf(),
                source,
            })
        };
        let model = digest_of(model_path)?;
        let labels = digest_of(labels_path)?;
        let geomodel = geomodel_path.map(digest_of).transpose()?;
        Ok(Self {
            model_name: model_name_of(model_path),
            model_path: model_path.to_path_buf(),
            model,
            labels_path: labels_path.to_path_buf(),
            labels,
            geomodel,
        })
    }
}

/// The operator-facing name of a model file: its stem, or the whole file name
/// when it has no extension, or the path when it has no file name at all.
#[must_use]
pub fn model_name_of(model_path: &Path) -> String {
    model_path
        .file_stem()
        .or_else(|| model_path.file_name())
        .map_or_else(
            || model_path.display().to_string(),
            |s| s.to_string_lossy().into_owned(),
        )
}

/// A classifier file could not be hashed.
#[derive(Debug)]
pub struct IdentityError {
    /// The file that could not be read.
    pub path: PathBuf,
    /// Why.
    pub source: std::io::Error,
}

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot hash {}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `printf 'abc' | sha256sum`.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn the_digest_is_the_sha256_of_the_bytes_and_their_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.bin");
        std::fs::write(&path, b"abc").unwrap();
        let d = file_digest(&path).unwrap();
        assert_eq!(d.sha256, ABC_SHA256);
        assert_eq!(d.bytes, 3);
    }

    #[test]
    fn a_file_longer_than_one_read_hashes_the_same_as_one_shot() {
        // Three reads' worth plus a tail, so the streaming loop's boundary
        // handling is exercised rather than assumed.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        let bytes: Vec<u8> = (0..(HASH_READ_BYTES * 3 + 7))
            .map(|i| u8::try_from(i % 251).unwrap_or(0))
            .collect();
        std::fs::write(&path, &bytes).unwrap();
        let streamed = file_digest(&path).unwrap();
        let one_shot = Sha256::digest(&bytes);
        let mut expected = String::new();
        for b in &one_shot {
            let _ = write!(expected, "{b:02x}");
        }
        assert_eq!(streamed.sha256, expected);
        assert_eq!(streamed.bytes, bytes.len() as u64);
    }

    #[test]
    fn two_models_of_equal_size_have_different_identities() {
        // The case a filename or a size cannot separate: two releases of one
        // architecture are the same length and may share a name.
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("model.onnx");
        let b = dir.path().join("also.onnx");
        std::fs::write(&a, vec![1u8; 4096]).unwrap();
        std::fs::write(&b, vec![2u8; 4096]).unwrap();
        let labels = dir.path().join("labels.txt");
        std::fs::write(&labels, "Turdus merula_Eurasian Blackbird\n").unwrap();
        let ia = ModelIdentity::resolve(&a, &labels, None).unwrap();
        let ib = ModelIdentity::resolve(&b, &labels, None).unwrap();
        assert_eq!(ia.model.bytes, ib.model.bytes);
        assert_ne!(ia.model.sha256, ib.model.sha256);
        assert_eq!(ia.labels, ib.labels);
        assert_eq!(ia.model_name, "model");
        assert!(ia.geomodel.is_none());
    }

    #[test]
    fn a_missing_file_names_itself_in_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.onnx");
        std::fs::write(&model, b"m").unwrap();
        let missing = dir.path().join("nowhere.csv");
        let err = ModelIdentity::resolve(&model, &missing, None).unwrap_err();
        assert_eq!(err.path, missing);
        assert!(err.to_string().contains("nowhere.csv"), "{err}");
    }

    #[test]
    fn the_geomodel_is_part_of_the_identity_when_configured() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir
            .path()
            .join("BirdNET+_V3.0-preview3_Global_11K_FP32.onnx");
        let labels = dir.path().join("labels.csv");
        let geo = dir.path().join("geo.onnx");
        std::fs::write(&model, b"m").unwrap();
        std::fs::write(&labels, b"l").unwrap();
        std::fs::write(&geo, b"g").unwrap();
        let id = ModelIdentity::resolve(&model, &labels, Some(&geo)).unwrap();
        assert_eq!(id.model_name, "BirdNET+_V3.0-preview3_Global_11K_FP32");
        assert_eq!(id.geomodel.as_ref().map(|d| d.bytes), Some(1));
    }

    #[test]
    fn model_name_falls_back_when_there_is_no_stem() {
        assert_eq!(model_name_of(Path::new("model")), "model");
        assert_eq!(model_name_of(Path::new("/a/b/model.onnx")), "model");
    }
}
