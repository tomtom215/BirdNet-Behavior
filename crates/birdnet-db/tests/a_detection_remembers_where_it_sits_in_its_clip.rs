//! The extractor's lead-in and the window's length reach the row and come
//! back out of every read (FR-1, migration 47).
//!
//! `chunk_offset_secs` is the start in the *source segment*, which is drained
//! minutes later; a selection table needs the start in the *clip*, and only
//! the extractor knows it. Written through `insert_detection`, read through
//! `detections_for_clip` (the per-clip export) and `analytic_detections`
//! (the combined one), and absent — not zero — on a row that never had a
//! clip.

use birdnet_db::sqlite::{
    DetectionRecord, analytic_detections, detections_for_clip, insert_detection,
};

const fn record<'a>(
    time: &'a str,
    clip: &'a str,
    offset: Option<f64>,
    len: Option<f64>,
) -> DetectionRecord<'a> {
    DetectionRecord {
        date: "2026-09-08",
        time,
        sci_name: "Turdus merula",
        com_name: "Eurasian Blackbird",
        confidence: 0.9,
        lat: None,
        lon: None,
        cutoff: None,
        week: None,
        sensitivity: None,
        overlap: None,
        file_name: clip,
        chunk_offset_secs: Some(0.0),
        correlation_id: None,
        source: None,
        duration_secs: Some(6.0),
        detected_at_utc: None,
        run_id: None,
        clip_offset_secs: offset,
        detection_secs: len,
    }
}

#[test]
fn the_clip_offset_and_length_round_trip_and_are_absent_when_never_known() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    insert_detection(&conn, &record("06:10:00", "a.wav", Some(1.5), Some(3.0))).unwrap();
    insert_detection(&conn, &record("06:10:03", "a.wav", Some(4.5), Some(1.5))).unwrap();
    insert_detection(&conn, &record("07:00:00", "old.wav", None, None)).unwrap();

    let a = detections_for_clip(&conn, "a.wav").unwrap();
    assert_eq!(
        a.iter()
            .map(|r| (r.clip_offset_secs, r.detection_secs))
            .collect::<Vec<_>>(),
        vec![(Some(1.5), Some(3.0)), (Some(4.5), Some(1.5))],
        "in clip order, with what the extractor wrote"
    );
    let old = detections_for_clip(&conn, "old.wav").unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(
        (old[0].clip_offset_secs, old[0].detection_secs),
        (None, None)
    );

    let (all, _) = analytic_detections(&conn, None, None, 10).unwrap();
    let placed = all.iter().filter(|r| r.clip_offset_secs.is_some()).count();
    assert_eq!(placed, 2, "the combined read carries the columns too");
}
