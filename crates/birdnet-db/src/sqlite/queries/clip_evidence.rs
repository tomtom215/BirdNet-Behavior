//! What a detection's clip was when it was written, and whether the file on
//! disk is still that clip (RES-23), named by the detection's permanent
//! identifier (RES-25).
//!
//! # What is recorded
//!
//! When the station writes a detection with a clip it records the clip's
//! SHA-256 and length in bytes in the same transaction as the row
//! (`Clip_Sha256`, `Clip_Bytes`, migration 69). Together with what the row
//! already holds — the clip's name, its duration, where the detection sits in
//! it (`clip_offset_secs`, `detection_secs`) and the detection's identifier —
//! that is what a reader needs to say "this file is the audio this row
//! describes". The checksum is of the file as written, tags included, so it
//! is checked against the file exactly as served.
//!
//! A row stored before migration 69, or whose clip could not be read when it
//! was written, has neither value. That is *unknown*, and stays so: nothing
//! here computes a checksum afterwards from whatever file has that name now,
//! because a checksum taken later attests only to the later file.
//!
//! # What verification can say
//!
//! [`verify`] reads the file and compares; [`ClipState`] is every answer it
//! can give, and none of them is a guess. It takes a [`ClipRecord`] read
//! beforehand, so a caller can read the row under the database lock and hash
//! the file outside it.

use std::io::Read as _;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

use crate::sqlite::connection::DbError;

/// A clip's SHA-256 (lowercase hex) and length in bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClipDigest {
    /// SHA-256 of the file's bytes, lowercase hex, 64 characters.
    pub sha256: String,
    /// The file's length in bytes.
    pub bytes: u64,
}

/// Hash the file at `path`.
///
/// # Errors
///
/// The I/O error that stopped the read.
pub fn digest_file(path: &Path) -> std::io::Result<ClipDigest> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes += n as u64;
    }
    Ok(ClipDigest {
        sha256: hex(&hasher.finalize()),
        bytes,
    })
}

/// [`digest_file`] for a write path, where a clip that cannot be read costs
/// its checksum and not the detection: `None`, logged, is stored as NULL,
/// which reads as unknown.
#[must_use]
pub fn digest_clip(path: &Path) -> Option<ClipDigest> {
    match digest_file(path) {
        Ok(d) => Some(d),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "the clip just written could not be read back to record its checksum; \
                 its checksum is stored as unknown"
            );
            None
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Record `digest` as the clip of the detection at `rowid`.
///
/// Called in the transaction that wrote the row. Returns `false` when the
/// row already had a checksum (it is never replaced; migration 69's trigger
/// refuses a change) or no row has that rowid.
///
/// # Errors
///
/// Returns `DbError` on update failure.
pub fn record_clip_digest(
    conn: &Connection,
    rowid: i64,
    digest: &ClipDigest,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE detections SET Clip_Sha256 = ?2, Clip_Bytes = ?3
          WHERE rowid = ?1 AND Clip_Sha256 IS NULL",
        params![rowid, digest.sha256, bytes_i64(digest.bytes)],
    )?;
    Ok(changed > 0)
}

/// Record `digest` as the clip a quarantine row named `file_name` was cut
/// with; approval carries it to the detection.
///
/// A cut clip's name is unique in the recordings directory, so the name
/// selects the one row. Returns `false` when no row names it, or the row
/// already has one.
///
/// # Errors
///
/// Returns `DbError` on update failure.
pub fn record_quarantine_clip_digest(
    conn: &Connection,
    file_name: &str,
    digest: &ClipDigest,
) -> Result<bool, DbError> {
    let changed = conn.execute(
        "UPDATE quarantine SET clip_sha256 = ?2, clip_bytes = ?3
          WHERE file_name = ?1 AND clip_sha256 IS NULL",
        params![file_name, digest.sha256, bytes_i64(digest.bytes)],
    )?;
    Ok(changed > 0)
}

fn bytes_i64(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

/// The permanent identifier of the detection at `rowid`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn detection_uuid_of(conn: &Connection, rowid: i64) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row(
            "SELECT detection_uuid FROM detections WHERE rowid = ?1",
            params![rowid],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// What the row says about its clip.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClipRecord {
    /// The detection's permanent identifier.
    pub detection_uuid: String,
    /// `File_Name`, when the row has one.
    pub file_name: Option<String>,
    /// The checksum recorded when the clip was written; `None` is unknown.
    pub recorded: Option<ClipDigest>,
    /// When retention removed the clip's audio, Unix seconds.
    pub pruned_at: Option<i64>,
}

/// The [`ClipRecord`] of the detection `detection_uuid` names, or `None` when
/// no detection has that identifier.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn clip_record(conn: &Connection, detection_uuid: &str) -> Result<Option<ClipRecord>, DbError> {
    Ok(conn
        .query_row(
            "SELECT detection_uuid, File_Name, Clip_Sha256, Clip_Bytes, Clip_Pruned_At
               FROM detections WHERE detection_uuid = ?1",
            params![detection_uuid],
            map_record,
        )
        .optional()?)
}

fn map_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRecord> {
    let sha: Option<String> = r.get(2)?;
    let bytes: Option<i64> = r.get(3)?;
    Ok(ClipRecord {
        detection_uuid: r.get(0)?,
        file_name: r
            .get::<_, Option<String>>(1)?
            .filter(|n| !n.trim().is_empty()),
        recorded: match (sha, bytes) {
            (Some(sha256), Some(bytes)) => Some(ClipDigest {
                sha256,
                bytes: u64::try_from(bytes).unwrap_or(0),
            }),
            _ => None,
        },
        pruned_at: r.get(4)?,
    })
}

/// What [`verify`] found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClipState {
    /// The file's SHA-256 and length are those recorded when it was written.
    Verified,
    /// The file is there and is not the clip that was written.
    Altered {
        /// What the file hashes to now.
        found: ClipDigest,
    },
    /// The row names a clip the recordings directory does not have, and
    /// retention did not record removing it.
    Missing,
    /// Retention removed the clip's audio; the row survives.
    Pruned,
    /// The row has no recorded checksum (it predates them, or the clip could
    /// not be read when written), so there is nothing to compare: whether
    /// the file is the original is unknown.
    Unrecorded,
    /// The detection has no clip.
    NoClip,
    /// The file could not be read; the error is given.
    Unreadable {
        /// The I/O error.
        error: String,
    },
}

/// Compare the clip `record` names in `recordings_dir` with what was recorded.
///
/// Reads the whole file; call it off an async runtime.
#[must_use]
pub fn verify(record: &ClipRecord, recordings_dir: &Path) -> ClipState {
    let Some(name) = record.file_name.as_deref() else {
        return ClipState::NoClip;
    };
    if record.pruned_at.is_some() {
        return ClipState::Pruned;
    }
    // The base name only, as the recordings route serves it: a row whose clip
    // could not be cut names its source segment by path.
    let Some(base) = Path::new(name).file_name() else {
        return ClipState::NoClip;
    };
    let Some(recorded) = &record.recorded else {
        return ClipState::Unrecorded;
    };
    match digest_file(&recordings_dir.join(base)) {
        Ok(found) if &found == recorded => ClipState::Verified,
        Ok(found) => ClipState::Altered { found },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ClipState::Missing,
        Err(e) => ClipState::Unreadable {
            error: e.to_string(),
        },
    }
}

/// Call `each` for every detection that names a clip, in rowid order.
///
/// What a backup's clip manifest is written from. Rows that cannot be read
/// fail the call rather than being left out of the manifest (N25-07).
///
/// # Errors
///
/// Returns `DbError` on query failure, or whatever `each` returns.
pub fn for_each_clip_record<F>(conn: &Connection, mut each: F) -> Result<(), DbError>
where
    F: FnMut(ClipRecord) -> Result<(), DbError>,
{
    let mut stmt = conn.prepare(
        "SELECT detection_uuid, File_Name, Clip_Sha256, Clip_Bytes, Clip_Pruned_At
           FROM detections
          WHERE File_Name IS NOT NULL AND TRIM(File_Name) <> ''
            AND detection_uuid IS NOT NULL
          ORDER BY rowid",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        each(map_record(row)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::migrate(&conn).unwrap();
        conn
    }

    fn insert(conn: &Connection, file: &str) -> i64 {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '06:00:00', 'Strix aluco', 'Tawny Owl', 0.9, ?1)",
            [file],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// The known answer: SHA-256 of "abc".
    #[test]
    fn a_digest_is_the_files_sha256_and_length() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.wav");
        std::fs::write(&p, b"abc").unwrap();
        let d = digest_file(&p).unwrap();
        assert_eq!(
            d.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(d.bytes, 3);
    }

    /// Every answer `verify` can give, each from the state that produces it:
    /// the same bytes verify, one changed byte does not, a removed file is
    /// missing, a pruned row is pruned, and a row with no checksum is unknown
    /// even when a file of that name is there.
    #[test]
    fn verification_tells_the_written_clip_from_any_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db();
        std::fs::write(dir.path().join("a.wav"), b"RIFF-the-clip").unwrap();
        let rowid = insert(&conn, "a.wav");
        let digest = digest_file(&dir.path().join("a.wav")).unwrap();
        assert!(record_clip_digest(&conn, rowid, &digest).unwrap());
        assert!(
            !record_clip_digest(
                &conn,
                rowid,
                &ClipDigest {
                    sha256: "0".repeat(64),
                    bytes: 1
                }
            )
            .unwrap(),
            "a recorded checksum is never replaced"
        );
        let uuid = detection_uuid_of(&conn, rowid).unwrap().unwrap();
        let record = clip_record(&conn, &uuid).unwrap().unwrap();
        assert_eq!(record.recorded.as_ref(), Some(&digest));
        assert_eq!(verify(&record, dir.path()), ClipState::Verified);

        std::fs::write(dir.path().join("a.wav"), b"RIFF-the-clip!").unwrap();
        assert!(matches!(
            verify(&record, dir.path()),
            ClipState::Altered { found } if found.bytes == 14
        ));

        std::fs::remove_file(dir.path().join("a.wav")).unwrap();
        assert_eq!(verify(&record, dir.path()), ClipState::Missing);

        let pruned = ClipRecord {
            pruned_at: Some(1),
            ..record
        };
        assert_eq!(verify(&pruned, dir.path()), ClipState::Pruned);

        std::fs::write(dir.path().join("b.wav"), b"anything").unwrap();
        let legacy = insert(&conn, "b.wav");
        let legacy_uuid = detection_uuid_of(&conn, legacy).unwrap().unwrap();
        let legacy_record = clip_record(&conn, &legacy_uuid).unwrap().unwrap();
        assert_eq!(legacy_record.recorded, None);
        assert_eq!(
            verify(&legacy_record, dir.path()),
            ClipState::Unrecorded,
            "no checksum was recorded, so whether the file is the original is unknown"
        );

        assert_eq!(clip_record(&conn, "not-an-id").unwrap(), None);
    }

    #[test]
    fn every_clip_row_reaches_the_manifest_walk() {
        let conn = db();
        insert(&conn, "a.wav");
        insert(&conn, "b.wav");
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '06:00:01', 'Strix aluco', 'Tawny Owl', 0.9, NULL)",
            [],
        )
        .unwrap();
        let mut seen = Vec::new();
        for_each_clip_record(&conn, |r| {
            seen.push(r.file_name.unwrap());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, ["a.wav", "b.wav"]);
    }
}
