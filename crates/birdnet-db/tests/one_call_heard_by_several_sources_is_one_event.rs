//! The read-time event layer (decision 1 of 2026-11-25; MSC-02, MSC-07,
//! RES-18): the rule in `sqlite::queries::events`, held by examples and by
//! properties over every ordering of small sets of rows.

use std::collections::BTreeSet;

use birdnet_db::sqlite::queries::events::{
    DEFAULT_TOLERANCE_SECS, EVENT_POLICY_VERSION, Event, EventRow, SiteMap, day_species_counts,
    event_id, events_between, group_events, site_counts, utc_date,
};
use proptest::prelude::*;

fn row(id: &str, t: i64, source: Option<&str>, sci: &str) -> EventRow {
    EventRow {
        row_id: id.to_owned(),
        instant: t,
        sci_name: sci.to_owned(),
        com_name: format!("{sci} (common)"),
        source: source.map(str::to_owned),
        confidence: 0.9,
    }
}

/// Every event as its sorted member ids, sorted: a comparison that ignores
/// output order and field details.
fn shape(events: &[Event]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            let mut ids: Vec<String> = e.members.iter().map(|m| m.row_id.clone()).collect();
            ids.sort();
            ids
        })
        .collect();
    out.sort();
    out
}

fn ids(v: &[&[&str]]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = v
        .iter()
        .map(|e| {
            let mut ids: Vec<String> = e.iter().map(|s| (*s).to_owned()).collect();
            ids.sort();
            ids
        })
        .collect();
    out.sort();
    out
}

/// All three sources at one site.
fn one_site() -> SiteMap {
    SiteMap::identity()
        .with("A", "yard")
        .with("B", "yard")
        .with("C", "yard")
}

#[test]
fn a_chorus_is_never_chained_into_one_event() {
    // A at 0, B at 3, C at 6, tolerance 4: A–B and B–C are each within it,
    // A–C is not. One event would claim A and C are one call six seconds apart.
    let rows = [
        row("a0", 0, Some("A"), "x"),
        row("b3", 3, Some("B"), "x"),
        row("c6", 6, Some("C"), "x"),
    ];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a0", "b3"], &["c6"]]));
    for e in &events {
        assert!(
            e.last - e.first <= 4,
            "span {} > tolerance",
            e.last - e.first
        );
    }
}

#[test]
fn the_nearest_row_wins_over_the_earlier_one() {
    // B at 3 is 3 s from the A at 0 and 0 s from the A at 3.
    let rows = [
        row("a0", 0, Some("A"), "x"),
        row("a3", 3, Some("A"), "x"),
        row("b3", 3, Some("B"), "x"),
    ];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a0"], &["a3", "b3"]]));
}

#[test]
fn two_candidates_at_one_distance_go_to_the_earlier_row() {
    // A at 0 and A at 2 are both one second from B at 1: the documented rule
    // gives B to the earlier A, and the later A is an event of its own.
    let rows = [
        row("a2", 2, Some("A"), "x"),
        row("b1", 1, Some("B"), "x"),
        row("a0", 0, Some("A"), "x"),
    ];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a0", "b1"], &["a2"]]));
}

#[test]
fn a_tie_in_time_is_broken_by_the_stable_row_id() {
    // Two A rows at the same instant, one B: the A with the smaller id pairs.
    let rows = [
        row("a-2", 5, Some("A"), "x"),
        row("a-1", 5, Some("A"), "x"),
        row("b", 5, Some("B"), "x"),
    ];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a-1", "b"], &["a-2"]]));
}

#[test]
fn by_default_every_source_is_its_own_site() {
    let rows = [row("a", 0, Some("A"), "x"), row("b", 0, Some("B"), "x")];
    let events = group_events(&rows, 4.0, &SiteMap::identity());
    assert_eq!(shape(&events), ids(&[&["a"], &["b"]]));
    // Counterpart: the same rows put at one site are one event.
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a", "b"]]));
}

#[test]
fn different_sites_never_merge() {
    let sites = SiteMap::identity().with("A", "north").with("B", "south");
    let rows = [row("a", 0, Some("A"), "x"), row("b", 0, Some("B"), "x")];
    assert_eq!(
        shape(&group_events(&rows, 4.0, &sites)),
        ids(&[&["a"], &["b"]])
    );
}

#[test]
fn different_species_never_merge() {
    let rows = [row("a", 0, Some("A"), "x"), row("b", 0, Some("B"), "y")];
    assert_eq!(
        shape(&group_events(&rows, 4.0, &one_site())),
        ids(&[&["a"], &["b"]])
    );
}

#[test]
fn one_source_twice_is_two_events() {
    let rows = [row("a0", 0, Some("A"), "x"), row("a1", 1, Some("A"), "x")];
    assert_eq!(
        shape(&group_events(&rows, 4.0, &one_site())),
        ids(&[&["a0"], &["a1"]])
    );
}

#[test]
fn a_row_with_no_source_is_an_event_of_its_own() {
    let rows = [
        row("n1", 0, None, "x"),
        row("n2", 0, None, "x"),
        row("a", 0, Some("A"), "x"),
    ];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(shape(&events), ids(&[&["a"], &["n1"], &["n2"]]));
    assert_eq!(events.iter().filter(|e| e.site.is_none()).count(), 2);
}

#[test]
fn the_default_tolerance_is_the_v24_window_and_a_second() {
    assert!((DEFAULT_TOLERANCE_SECS - 4.0).abs() < f64::EPSILON);
    // The boundary, both sides: 4 s apart pairs, 5 s does not.
    let at = |dt| {
        let rows = [row("a", 0, Some("A"), "x"), row("b", dt, Some("B"), "x")];
        group_events(&rows, DEFAULT_TOLERANCE_SECS, &one_site()).len()
    };
    assert_eq!(at(4), 1);
    assert_eq!(at(5), 2);
}

#[test]
fn an_event_id_is_its_members_and_the_policy() {
    let a = event_id(&["r1", "r2"], EVENT_POLICY_VERSION);
    assert_eq!(a, event_id(&["r2", "r1"], EVENT_POLICY_VERSION));
    assert_ne!(a, event_id(&["r1", "r2"], "events-0"));
    assert_ne!(a, event_id(&["r1"], EVENT_POLICY_VERSION));
    // Length-prefixed: ["ab", "c"] is not ["a", "bc"].
    assert_ne!(
        event_id(&["ab", "c"], EVENT_POLICY_VERSION),
        event_id(&["a", "bc"], EVENT_POLICY_VERSION)
    );
}

#[test]
fn an_event_across_utc_midnight_is_one_event_on_the_first_date() {
    // 2026-03-28T23:59:58Z and 2026-03-29T00:00:01Z.
    let t = 1_774_742_398;
    assert_eq!(utc_date(t), "2026-03-28");
    assert_eq!(utc_date(t + 3), "2026-03-29");
    let rows = [row("a", t, Some("A"), "x"), row("b", t + 3, Some("B"), "x")];
    let events = group_events(&rows, 4.0, &one_site());
    assert_eq!(events.len(), 1);
    let days = day_species_counts(&events);
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].date, "2026-03-28");
    assert_eq!((days[0].events, days[0].rows), (1, 2));
}

#[test]
fn utc_dates_hold_on_leap_days_and_year_ends() {
    assert_eq!(utc_date(0), "1970-01-01");
    assert_eq!(utc_date(951_782_400), "2000-02-29");
    assert_eq!(utc_date(1_709_164_800), "2024-02-29");
    assert_eq!(utc_date(1_798_761_599), "2026-12-31");
    assert_eq!(utc_date(1_798_761_600), "2027-01-01");
    assert_eq!(utc_date(-1), "1969-12-31");
}

#[test]
fn per_site_counts_count_events_and_rows() {
    let sites = SiteMap::identity()
        .with("A", "yard")
        .with("B", "yard")
        .with("C", "field");
    let rows = [
        row("a", 0, Some("A"), "x"),
        row("b", 1, Some("B"), "x"),
        row("c", 1, Some("C"), "x"),
    ];
    let counts = site_counts(&group_events(&rows, 4.0, &sites));
    let got: Vec<(Option<String>, u64, u64, usize)> = counts
        .iter()
        .map(|c| (c.site.clone(), c.events, c.rows, c.sources.len()))
        .collect();
    assert_eq!(
        got,
        vec![
            (Some("field".into()), 1, 1, 1),
            (Some("yard".into()), 1, 2, 2)
        ]
    );
}

/// Heap's algorithm: every ordering of `v`.
fn permutations<T: Clone>(v: &[T]) -> Vec<Vec<T>> {
    fn go<T: Clone>(k: usize, a: &mut Vec<T>, out: &mut Vec<Vec<T>>) {
        if k <= 1 {
            out.push(a.clone());
            return;
        }
        go(k - 1, a, out);
        for i in 0..k - 1 {
            if k.is_multiple_of(2) {
                a.swap(i, k - 1);
            } else {
                a.swap(0, k - 1);
            }
            go(k - 1, a, out);
        }
    }
    let mut a = v.to_vec();
    let mut out = Vec::new();
    go(a.len(), &mut a, &mut out);
    out
}

/// Small sets of rows over four sources (three at one site, one at its own),
/// rows with no source, two species, and
/// instants close enough together that pairing decisions interact.
fn small_rows() -> impl Strategy<Value = Vec<EventRow>> {
    prop::collection::vec(
        (
            0i64..12,
            prop::sample::select(vec!["A", "B", "C", "D", "E"]),
            0usize..2,
        ),
        1..=6,
    )
    .prop_map(|v| {
        v.into_iter()
            .enumerate()
            .map(|(i, (t, s, sp))| {
                let source = if s == "D" { None } else { Some(s) };
                row(&format!("r{i}"), t, source, ["x", "y"][sp])
            })
            .collect()
    })
}

fn sites_ab_c() -> SiteMap {
    one_site()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// The same rows in every order give the same events, with the same ids.
    #[test]
    fn the_events_do_not_depend_on_the_order_of_the_rows(rows in small_rows(), tol in 0u8..6) {
        let tol = f64::from(tol);
        let sites = sites_ab_c();
        let want = group_events(&rows, tol, &sites);
        for perm in permutations(&rows) {
            let got = group_events(&perm, tol, &sites);
            prop_assert_eq!(&got, &want);
        }
    }

    /// Every row in exactly one event; every event one site, one species, at
    /// most one row per source and a span within the tolerance; and no two
    /// events could be joined without breaking one of those.
    #[test]
    fn every_event_obeys_the_rule_and_none_could_be_joined(rows in small_rows(), tol in 0u8..6) {
        let tol_s = i64::from(tol);
        let sites = sites_ab_c();
        let events = group_events(&rows, f64::from(tol), &sites);
        let mut seen: Vec<String> = events
            .iter()
            .flat_map(|e| e.members.iter().map(|m| m.row_id.clone()))
            .collect();
        seen.sort();
        let mut all: Vec<String> = rows.iter().map(|r| r.row_id.clone()).collect();
        all.sort();
        prop_assert_eq!(seen, all);
        let joinable = |e: &Event| {
            let srcs: Vec<&str> = e.members.iter().filter_map(|m| m.source.as_deref()).collect();
            let distinct: BTreeSet<&str> = srcs.iter().copied().collect();
            e.site.is_some()
                && srcs.len() == e.members.len()
                && distinct.len() == srcs.len()
                && e.members.iter().all(|m| m.sci_name == e.sci_name)
                && e.members.iter().map(|m| m.instant).max().unwrap()
                    - e.members.iter().map(|m| m.instant).min().unwrap()
                    <= tol_s
                && e.members.iter().all(|m| {
                    m.source.as_deref().map(|s| sites.site_of(s)) == e.site.as_deref()
                })
        };
        for e in &events {
            if e.site.is_some() {
                prop_assert!(joinable(e), "event breaks the rule: {:?}", e);
            } else {
                prop_assert_eq!(e.members.len(), 1);
            }
        }
        for (i, x) in events.iter().enumerate() {
            for y in &events[i + 1..] {
                if x.site.is_none() || x.site != y.site || x.sci_name != y.sci_name {
                    continue;
                }
                let mut members = x.members.clone();
                members.extend(y.members.iter().cloned());
                let first = members.iter().map(|m| m.instant).min().unwrap();
                let last = members.iter().map(|m| m.instant).max().unwrap();
                let union = Event { members, first, last, ..x.clone() };
                prop_assert!(!joinable(&union), "{:?} and {:?} could be one event", x, y);
            }
        }
    }
}

// ---------------------------------------------------------------- the query

fn conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("schema");
    conn
}

fn insert(
    conn: &rusqlite::Connection,
    date: &str,
    time: &str,
    file: &str,
    source: &str,
    utc: Option<i64>,
) {
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Source, detected_at_utc)
         VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9, ?3, ?4, ?5)",
        rusqlite::params![date, time, file, source, utc],
    )
    .expect("insert");
}

/// 2026-10-25 00:00:00Z.
const DAY: i64 = 1_792_886_400;

#[test]
fn the_query_reads_instants_not_local_times_and_counts_rows_without_one() {
    let conn = conn();
    // The fall-back fold: two rows at the same local 02:30 an hour apart, from
    // two sources at one site. By local time they would be one event.
    insert(
        &conn,
        "2026-10-25",
        "02:30:00",
        "a1.wav",
        "A",
        Some(DAY + 30 * 60),
    );
    insert(
        &conn,
        "2026-10-25",
        "02:30:00",
        "b1.wav",
        "B",
        Some(DAY + 90 * 60),
    );
    // Two sources four seconds apart: one event.
    insert(
        &conn,
        "2026-10-25",
        "08:00:00",
        "a2.wav",
        "A",
        Some(DAY + 8 * 3600),
    );
    insert(
        &conn,
        "2026-10-25",
        "08:00:04",
        "b2.wav",
        "B",
        Some(DAY + 8 * 3600 + 4),
    );
    // A row whose local time named no instant: excluded and counted.
    // The insert trigger stamps a NULL instant from Date and Time when they
    // round-trip on this host's zone, so clear it as a skipped hour would.
    insert(&conn, "2026-10-25", "02:30:00", "c.wav", "A", None);
    conn.execute(
        "UPDATE detections SET detected_at_utc = NULL WHERE File_Name = 'c.wav'",
        [],
    )
    .expect("clear");
    // Precondition: the trigger did not stamp the NULL row behind our back.
    let nulls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM detections WHERE detected_at_utc IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(nulls, 1, "the fixture needs one row without an instant");

    let yard = SiteMap::identity().with("A", "yard").with("B", "yard");
    let read =
        events_between(&conn, DAY, DAY + 86_400, DEFAULT_TOLERANCE_SECS, &yard).expect("read");
    assert_eq!(read.rows_without_instant, 1);
    assert!(read.edges_settled);
    let sizes: Vec<usize> = read.events.iter().map(|e| e.members.len()).collect();
    assert_eq!(sizes, vec![1, 1, 2], "{:?}", read.events);
    // The row id is the detection's permanent identifier, not the rowid.
    let uuid: String = conn
        .query_row(
            "SELECT detection_uuid FROM detections WHERE File_Name = 'a2.wav'",
            [],
            |r| r.get(0),
        )
        .expect("uuid");
    assert_eq!(read.events[2].members[0].row_id, uuid);

    // Counterpart: with the default site map nothing merges.
    let read = events_between(
        &conn,
        DAY,
        DAY + 86_400,
        DEFAULT_TOLERANCE_SECS,
        &SiteMap::identity(),
    )
    .expect("read");
    assert_eq!(read.events.len(), 4);
}

/// RES-25 in the event layer: relabelling a detection keeps the identity of
/// the event it is in; the composite key the layer used to name a row by
/// included the species, so a relabel made it another event.
#[test]
fn a_relabelled_detection_keeps_its_event_s_identity() {
    let conn = conn();
    insert(
        &conn,
        "2026-10-25",
        "08:00:00",
        "a.wav",
        "A",
        Some(DAY + 8 * 3600),
    );
    let id = || {
        events_between(
            &conn,
            DAY,
            DAY + 86_400,
            DEFAULT_TOLERANCE_SECS,
            &SiteMap::identity(),
        )
        .expect("read")
        .events[0]
            .event_id
            .clone()
    };
    let before = id();
    conn.execute(
        "UPDATE detections SET Sci_Name = 'Turdus philomelos', Com_Name = 'Song Thrush'",
        [],
    )
    .expect("relabel");
    assert_eq!(id(), before, "a relabel made the detection another event");
}

#[test]
fn an_event_at_the_edge_of_the_range_pairs_as_it_would_in_a_wider_read() {
    let conn = conn();
    // A at -6, B at -3, A at +1, tolerance 4. B is 3 s from the first A and
    // 4 s from the second, so it pairs with the first, before the range, and
    // the A at +1 is an event of its own in the range. A read of only a
    // tolerance's margin, [-4, ..), misses the A at -6, pairs B with the A at
    // +1, and reports nothing in the range at all.
    insert(
        &conn,
        "2026-10-24",
        "23:59:54",
        "a1.wav",
        "A",
        Some(DAY - 6),
    );
    insert(&conn, "2026-10-24", "23:59:57", "b.wav", "B", Some(DAY - 3));
    insert(
        &conn,
        "2026-10-25",
        "00:00:01",
        "a2.wav",
        "A",
        Some(DAY + 1),
    );
    let sites = SiteMap::identity().with("A", "yard").with("B", "yard");
    let narrow = events_between(&conn, DAY, DAY + 3600, 4.0, &sites).expect("read");
    let wide = events_between(&conn, DAY - 3600, DAY + 3600, 4.0, &sites).expect("read");
    let in_narrow: Vec<&Event> = wide.events.iter().filter(|e| e.first >= DAY).collect();
    assert_eq!(narrow.events.iter().collect::<Vec<_>>(), in_narrow);
    assert_eq!(narrow.events.len(), 1, "{:?}", narrow.events);
    assert_eq!(narrow.events[0].members.len(), 1);
    assert!(
        narrow.read_from <= DAY - 6,
        "read from {}",
        narrow.read_from - DAY
    );
}
