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

/// A fresh in-memory database with the schema and nothing in it.
///
/// [`temp_db_with_data`] hands back a fixed four-row fixture, which is the
/// right shape for the read tests that grew up around it and the wrong shape
/// for anything asserting on a filter: those need to choose their own rows, and
/// a shared fixture quietly couples every such test to every other one.
pub(super) fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory database");
    crate::migration::migrate(&conn).expect("apply the schema");
    conn
}

/// Insert one detection. Only the columns a filter can select on.
pub(super) fn insert_test_detection(
    conn: &Connection,
    date: &str,
    time: &str,
    com_name: &str,
    sci_name: &str,
    confidence: f64,
) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![date, time, sci_name, com_name, confidence],
    )
    .expect("insert test detection");
}
