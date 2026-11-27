//! A station's identity, its deployments, which device each source was, and
//! how far its clock could be trusted (RES-12, RES-22, MSC-13, OOB-17).
//!
//! Every assertion here reads the schema through SQL, so the same file states
//! what the database holds whichever program wrote it: the capture loop, the
//! settings page, an import, or someone at the `sqlite3` prompt.

use birdnet_db::audio_sources::{AudioSourcePatch, AudioSourceStore, NewAudioSource, SourceKind};
use birdnet_db::migration::{MIGRATIONS, migrate};
use rusqlite::Connection;

fn db() -> Connection {
    let conn = Connection::open_in_memory().expect("open");
    migrate(&conn).expect("migrate");
    conn
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .expect("sqlite_master")
        == 1
}

/// `(device_seq, device_id, from_utc, to_utc)` for one source, in the order written.
fn devices(conn: &Connection, source: &str) -> Vec<(i64, String, Option<i64>, Option<i64>)> {
    assert!(
        table_exists(conn, "audio_source_devices"),
        "no record of which device a source used: a source's history is only its row id"
    );
    let mut stmt = conn
        .prepare(
            "SELECT device_seq, device_id, from_utc, to_utc FROM audio_source_devices
              WHERE source_id = ?1 ORDER BY id",
        )
        .expect("prepare");
    stmt.query_map([source], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })
    .expect("query")
    .collect::<Result<_, _>>()
    .expect("rows")
}

#[test]
fn a_station_has_one_identity_and_a_second_migrate_keeps_it() {
    let conn = db();
    assert!(
        table_exists(&conn, "station_identity"),
        "nothing mints or keeps a station id"
    );
    let read = |c: &Connection| -> Vec<String> {
        let mut s = c
            .prepare("SELECT station_uuid FROM station_identity")
            .expect("prepare");
        s.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    };
    let first = read(&conn);
    assert_eq!(first.len(), 1, "exactly one station identity: {first:?}");
    let id = &first[0];
    // RFC 4122 version 4, lower-case: 8-4-4-4-12 with the version nibble 4 and
    // the variant nibble 8, 9, a or b.
    let groups: Vec<&str> = id.split('-').collect();
    assert_eq!(
        groups.iter().map(|g| g.len()).collect::<Vec<_>>(),
        [8, 4, 4, 4, 12],
        "{id}"
    );
    assert!(
        id.chars()
            .all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert!(groups[2].starts_with('4'), "version nibble: {id}");
    assert!(
        matches!(groups[3].as_bytes()[0], b'8' | b'9' | b'a' | b'b'),
        "variant nibble: {id}"
    );
    migrate(&conn).expect("second migrate");
    assert_eq!(
        read(&conn),
        first,
        "a second start must not mint a new station"
    );

    // Counterpart: two stations are two identities, not one constant.
    let other = read(&db());
    assert_ne!(other, first, "two databases were given the same station id");
}

#[test]
fn editing_a_sources_device_starts_a_new_device_record() {
    let conn = db();
    conn.insert(&NewAudioSource::defaults(
        "cam",
        SourceKind::Rtsp,
        "rtsp://pond.local/audio",
    ))
    .expect("insert");
    conn.update(
        "cam",
        &AudioSourcePatch {
            device_id: Some("rtsp://lane.local/audio".to_owned()),
            ..AudioSourcePatch::default()
        },
    )
    .expect("edit the URL");
    let rows = devices(&conn, "cam");
    assert_eq!(rows.len(), 2, "one record per device: {rows:?}");
    assert_eq!(
        (rows[0].0, rows[0].1.as_str()),
        (1, "rtsp://pond.local/audio")
    );
    assert_eq!(
        (rows[1].0, rows[1].1.as_str()),
        (2, "rtsp://lane.local/audio")
    );
    assert!(
        rows[0].2.is_some(),
        "a device added under this schema has a start"
    );
    assert!(
        rows[0].3.is_some(),
        "the first device's record is closed by the edit"
    );
    assert!(rows[1].3.is_none(), "the current device's record is open");
    assert!(
        rows[0].3 <= rows[1].2,
        "the two devices' intervals do not overlap"
    );

    // Counterpart: editing something other than the device writes nothing.
    conn.update(
        "cam",
        &AudioSourcePatch {
            label: Some(Some("Lane".to_owned())),
            ..AudioSourcePatch::default()
        },
    )
    .expect("relabel");
    assert_eq!(devices(&conn, "cam").len(), 2, "a label is not a device");

    // And an edit back to the first camera is the first camera again.
    conn.update(
        "cam",
        &AudioSourcePatch {
            device_id: Some("rtsp://pond.local/audio".to_owned()),
            ..AudioSourcePatch::default()
        },
    )
    .expect("edit back");
    let rows = devices(&conn, "cam");
    assert_eq!(
        rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        [1, 2, 1],
        "{rows:?}"
    );
}

#[test]
fn removing_and_re_adding_the_same_device_keeps_its_device_record() {
    let conn = db();
    let new = NewAudioSource::defaults("mic", SourceKind::UsbAlsa, "plughw:2,0");
    conn.insert(&new).expect("insert");
    conn.soft_delete("mic").expect("remove");
    let removed = devices(&conn, "mic");
    assert_eq!(removed.len(), 1);
    assert!(
        removed[0].3.is_some(),
        "removing a source closes its device record"
    );
    conn.reinstate("mic", &new).expect("re-add");
    let rows = devices(&conn, "mic");
    assert_eq!(
        rows.len(),
        2,
        "a second interval for the time it was back: {rows:?}"
    );
    assert_eq!(
        rows[1].0, 1,
        "the same device keeps its number across a re-add"
    );
    assert!(rows[1].3.is_none());

    // Counterpart: re-adding under the same id with a different device is a
    // new device number, not a merge.
    conn.soft_delete("mic").expect("remove again");
    conn.reinstate(
        "mic",
        &NewAudioSource::defaults("mic", SourceKind::UsbAlsa, "plughw:3,0"),
    )
    .expect("re-add another device");
    let rows = devices(&conn, "mic");
    assert_eq!(rows.last().map(|r| r.0), Some(2), "{rows:?}");
}

/// A source that existed before this record did keeps its id, and its first
/// device record says honestly when it started: at the row's creation when the
/// row was never edited, and unknown when it was, because the device may have
/// been changed in between.
#[test]
fn an_existing_source_keeps_its_id_and_its_start_is_not_invented() {
    let introduced = MIGRATIONS
        .iter()
        .find(|m| {
            m.up_sql
                .contains("CREATE TABLE IF NOT EXISTS audio_source_devices")
        })
        .map(|m| m.version)
        .expect("a migration that records which device a source used");
    let conn = Connection::open_in_memory().expect("open");
    conn.execute_batch(
        "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, description TEXT NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (datetime('now')));",
    )
    .expect("version table");
    for m in MIGRATIONS.iter().take_while(|m| m.version < introduced) {
        conn.execute_batch(m.up_sql)
            .unwrap_or_else(|e| panic!("migration {} failed: {e}", m.version));
        conn.execute(
            "INSERT INTO schema_version (version, description) VALUES (?1, ?2)",
            rusqlite::params![m.version, m.description],
        )
        .expect("stamp");
    }
    conn.execute_batch(
        "INSERT INTO audio_sources (id, kind, device_id, created_at, updated_at)
             VALUES ('kept', 'usb-alsa', 'plughw:1,0', '2025-04-01 06:00:00', '2025-04-01 06:00:00'),
                    ('edited', 'rtsp', 'rtsp://b/x', '2025-04-01 06:00:00', '2025-06-01 06:00:00');
         INSERT INTO audio_sources (id, kind, device_id, created_at, updated_at, disabled_at)
             VALUES ('gone', 'usb-alsa', 'plughw:4,0', '2025-04-01 06:00:00', '2025-05-01 06:00:00',
                     '2025-05-01 06:00:00');",
    )
    .expect("sources from before");
    migrate(&conn).expect("migrate to now");

    let kept = devices(&conn, "kept");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, 1);
    assert_eq!(
        kept[0].2,
        Some(1_743_487_200),
        "never edited: its start is its creation"
    );
    assert_eq!(kept[0].3, None);
    let edited = devices(&conn, "edited");
    assert_eq!(
        edited[0].2, None,
        "edited since creation: when this device began is unknown"
    );
    let gone = devices(&conn, "gone");
    assert_eq!(
        gone[0].3,
        Some(1_746_079_200),
        "a removed source's record ends at its removal"
    );
    assert!(
        conn.get("kept").expect("get").is_some(),
        "every existing source id stays valid"
    );
}

#[test]
fn a_deployment_is_operator_stated_and_closed_by_the_next() {
    let conn = db();
    assert!(
        table_exists(&conn, "deployments"),
        "there is no deployment record"
    );
    conn.insert(&NewAudioSource::defaults(
        "mic",
        SourceKind::UsbAlsa,
        "plughw:2,0",
    ))
    .expect("insert");
    let station: String = conn
        .query_row("SELECT station_uuid FROM station_identity", [], |r| {
            r.get(0)
        })
        .expect("station");
    conn.execute(
        "INSERT INTO deployments (station_uuid, source_id, start_utc, height_m, orientation_deg,
                                  microphone, recorded_at_utc)
         VALUES (?1, 'mic', 1000, 2.5, 180.0, 'Clippy EM272', 1000)",
        [&station],
    )
    .expect("first deployment");
    // Bounds the database itself refuses, whatever wrote the row.
    for bad in [
        "INSERT INTO deployments (station_uuid, source_id, start_utc, height_m, recorded_at_utc)
         VALUES ('s', 'mic', 3000, -1.0, 3000)",
        "INSERT INTO deployments (station_uuid, source_id, start_utc, orientation_deg, recorded_at_utc)
         VALUES ('s', 'mic', 3000, 360.0, 3000)",
        "INSERT INTO deployments (station_uuid, source_id, start_utc, latitude, recorded_at_utc)
         VALUES ('s', 'mic', 3000, 91.0, 3000)",
        "INSERT INTO deployments (station_uuid, source_id, start_utc, end_utc, recorded_at_utc)
         VALUES ('s', 'mic', 3000, 3000, 3000)",
    ] {
        assert!(conn.execute(bad, []).is_err(), "accepted: {bad}");
    }
    // What the operator said is not rewritten afterwards; only an open
    // deployment's end may be set, once.
    assert!(
        conn.execute(
            "UPDATE deployments SET height_m = 9 WHERE source_id = 'mic'",
            []
        )
        .is_err(),
        "a deployment's stated height was rewritten"
    );
    conn.execute(
        "UPDATE deployments SET end_utc = 2000 WHERE source_id = 'mic' AND end_utc IS NULL",
        [],
    )
    .expect("close it");
    assert!(
        conn.execute(
            "UPDATE deployments SET end_utc = 2500 WHERE source_id = 'mic'",
            []
        )
        .is_err(),
        "a closed deployment's end was moved"
    );
}

#[test]
fn the_clock_record_and_the_effort_ledger_keyed_on_utc_exist() {
    let conn = db();
    assert!(
        table_exists(&conn, "clock_observations"),
        "the clock's state is never written down"
    );
    assert!(
        table_exists(&conn, "recording_effort_utc"),
        "effort is keyed only on the local hour, so the repeated autumn hour is one row"
    );
    for bad in [
        "INSERT INTO clock_observations (at_utc, state, authority) VALUES (1, 'probably', 'x')",
        "INSERT INTO recording_effort_utc (hour_start_utc, source) VALUES (1800, 'mic')",
    ] {
        assert!(conn.execute(bad, []).is_err(), "accepted: {bad}");
    }
    // The repeated autumn hour in Berlin, 2026-10-25 02:00 local, is two UTC
    // hours, and two rows.
    for hour in [1_792_886_400_i64, 1_792_890_000] {
        conn.execute(
            "INSERT INTO recording_effort_utc (hour_start_utc, source, seconds_up) VALUES (?1, 'mic', 300)",
            [hour],
        )
        .expect("hour");
    }
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM recording_effort_utc", [], |r| {
            r.get(0)
        })
        .expect("count");
    assert_eq!(n, 2);
}
