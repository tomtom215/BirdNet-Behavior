//! BirdNET-Pi schema detector.
//!
//! Implements `SchemaDetector` for the BirdNET-Pi `BirdDB.txt` format.

use std::path::Path;

use crate::error::MigrateError;
use crate::schema::{DetectedSchema, detect_schema};
use crate::traits::SchemaDetector;

/// Detects the BirdNET-Pi `BirdDB.txt` SQLite schema.
///
/// BirdNET-Pi stores detections in a file commonly named `BirdDB.txt` or
/// `birds.db` (it is a plain SQLite file despite the `.txt` extension).
/// All known BirdNET-Pi versions share the same 12-column `detections` table.
#[derive(Debug, Clone, Default)]
pub struct BirdNetPiDetector;

impl SchemaDetector for BirdNetPiDetector {
    fn detect(&self, path: &Path) -> Result<DetectedSchema, MigrateError> {
        let schema = detect_schema(path)?;

        match &schema {
            DetectedSchema::BirdNetPi { .. } => {
                tracing::info!(
                    path = %path.display(),
                    rows = schema.row_count(),
                    "detected BirdNET-Pi schema"
                );
                Ok(schema)
            }
            DetectedSchema::BirdNetBehavior { .. } => {
                // Same columns but already migrated — still accept it so the
                // web UI can show "already a BirdNet-Behavior database".
                tracing::info!(
                    path = %path.display(),
                    "source is already a BirdNet-Behavior database"
                );
                Ok(schema)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn make_pi_db() -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (
                Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
                Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL,
                Week INTEGER, Sens REAL, Overlap REAL, File_Name TEXT);",
        )
        .unwrap();
        drop(conn);
        tmp
    }

    #[test]
    fn detect_birdnet_pi() {
        let tmp = make_pi_db();
        let d = BirdNetPiDetector;
        let schema = d.detect(tmp.path()).unwrap();
        assert!(matches!(schema, DetectedSchema::BirdNetPi { .. }));
    }

    #[test]
    fn detect_not_found() {
        let d = BirdNetPiDetector;
        let err = d.detect(Path::new("/no/such/file.db")).unwrap_err();
        assert!(matches!(err, MigrateError::SourceNotFound(_)));
    }
}
