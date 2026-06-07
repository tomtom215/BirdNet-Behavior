//! Shared test fixtures for the detection-query submodules.

use rusqlite::{Connection, params};

use crate::sqlite::connection::open_or_create;

/// Open a fresh temp database seeded with four detections across two dates
/// (three on 2026-03-11, one on 2026-03-10). Shared by the read, write and
/// lock query tests.
pub(super) fn temp_db_with_data() -> (tempfile::NamedTempFile, Connection) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let conn = open_or_create(tmp.path()).unwrap();
    for (date, time, sci, com, conf) in [
        (
            "2026-03-11",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
        ),
        (
            "2026-03-11",
            "06:45:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
        ),
        (
            "2026-03-11",
            "07:00:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.75,
        ),
        ("2026-03-10", "18:00:00", "Parus major", "Great Tit", 0.80),
    ] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) VALUES (?1,?2,?3,?4,?5)",
            params![date, time, sci, com, conf],
        ).unwrap();
    }
    (tmp, conn)
}
