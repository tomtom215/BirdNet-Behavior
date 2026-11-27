//! CHT-24, RES-16, UIR-22, N06-02: whether the co-occurrence analysis says
//! "nothing" when nothing is there.
//!
//! * **Independence**: six species, each its own process (its own hours, its
//!   own rate, its own day-to-day persistence), 90 days. Two of them share
//!   the dawn hours, so a chance level that ignored time of day would call
//!   them a pair. No pair may stand out.
//! * **Counterpart**: the same station with one planted pair (one species
//!   heard in most of another's blocks). That pair, and only that pair, must
//!   stand out, and the independent pairs beside it must not.
//! * **Sites**: two microphones in different places, a robin at one and a wren
//!   at the other a minute apart every day, were never together.
//! * **Too little data**: a window too short for any pair to reach the
//!   adjusted threshold says so, rather than "nothing stands out".

use birdnet_behavioral::null_model::SplitMix64;
use birdnet_db::sqlite::queries::correlation::{
    CooccurrenceAnalysis, CooccurrenceQuery, Verdict, cooccurrence_analysis,
};
use rusqlite::Connection;

fn station() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    conn
}

/// One species' behaviour: active between `from` and `to` (five-minute slots),
/// on a day it is present heard in each active slot with `rate`; presence from
/// day to day a Markov chain with `stay` probability of keeping its state.
struct Process {
    sci: &'static str,
    com: &'static str,
    from: usize,
    to: usize,
    rate: f64,
    stay: f64,
}

const SPECIES: [Process; 6] = [
    Process {
        sci: "Turdus merula",
        com: "Blackbird",
        from: 60,
        to: 96,
        rate: 0.25,
        stay: 0.8,
    },
    Process {
        sci: "Erithacus rubecula",
        com: "Robin",
        from: 62,
        to: 100,
        rate: 0.2,
        stay: 0.7,
    },
    Process {
        sci: "Parus major",
        com: "Great Tit",
        from: 90,
        to: 150,
        rate: 0.15,
        stay: 0.6,
    },
    Process {
        sci: "Strix aluco",
        com: "Tawny Owl",
        from: 230,
        to: 270,
        rate: 0.2,
        stay: 0.85,
    },
    Process {
        sci: "Sitta europaea",
        com: "Nuthatch",
        from: 100,
        to: 200,
        rate: 0.1,
        stay: 0.5,
    },
    Process {
        sci: "Columba palumbus",
        com: "Woodpigeon",
        from: 70,
        to: 180,
        rate: 0.12,
        stay: 0.9,
    },
];

fn time(slot: usize) -> String {
    format!("{:02}:{:02}:00", slot / 12, (slot % 12) * 5)
}

/// Rows of the independence fixture, and of a planted follower of the
/// Blackbird when `plant` is set.
fn fill(conn: &Connection, seed: u64, plant: bool) -> usize {
    let mut rng = SplitMix64::new(seed);
    let mut present = [true; 6];
    let mut rows = Vec::new();
    for back in 1..=90 {
        let d = format!("DATE('now', 'localtime', '-{back} days')");
        for (i, p) in SPECIES.iter().enumerate() {
            if rng.next_f64() >= p.stay {
                present[i] = !present[i];
            }
            if !present[i] {
                continue;
            }
            for slot in p.from..p.to {
                if rng.next_f64() < p.rate {
                    rows.push(format!(
                        "({d},'{}','{}','{}',0.9,'local')",
                        time(slot),
                        p.sci,
                        p.com
                    ));
                    if plant && i == 0 && rng.next_f64() < 0.8 {
                        rows.push(format!(
                            "({d},'{}','Certhia familiaris','Treecreeper',0.9,'local')",
                            time(slot)
                        ));
                    }
                }
            }
        }
    }
    for chunk in rows.chunks(500) {
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Source) VALUES {};",
            chunk.join(",")
        ))
        .unwrap();
    }
    rows.len()
}

fn query() -> CooccurrenceQuery {
    CooccurrenceQuery::new(90, 100, 3)
}

#[test]
fn independent_species_never_stand_out() {
    let conn = station();
    assert!(
        fill(&conn, 0xC0FF_EE01, false) > 1000,
        "the fixture wrote its rows"
    );
    let a = cooccurrence_analysis(&conn, &query()).unwrap();
    // The fixture did something: the dawn species share blocks.
    let dawn = a
        .pairs
        .iter()
        .find(|p| p.species_a == "Blackbird" && p.species_b == "Robin")
        .expect("the two dawn species share blocks");
    assert!(dawn.shared_blocks >= 20, "{dawn:?}");
    assert!(!a.too_little_data, "90 days are enough to tell: {a:?}");
    let loud: Vec<_> = a
        .pairs
        .iter()
        .filter(|p| p.verdict == Verdict::StandsOut)
        .map(|p| {
            (
                &p.species_a,
                &p.species_b,
                p.shared_blocks,
                p.expected_shared,
                p.q_value,
            )
        })
        .collect();
    assert!(loud.is_empty(), "independent species stood out: {loud:?}");
    assert_eq!(a.stands_out, 0);
    // The chance level is where the observed share is, give or take.
    let lift = dawn.lift.expect("chance gives some");
    assert!((0.6..1.6).contains(&lift), "observed over chance {lift}");
    assert!(dawn.chance_overlap > 0.0);
}

#[test]
fn a_planted_pair_stands_out_and_nothing_else_does() {
    let conn = station();
    fill(&conn, 0xC0FF_EE01, true);
    let a = cooccurrence_analysis(&conn, &query()).unwrap();
    let loud: Vec<(&str, &str)> = a
        .pairs
        .iter()
        .filter(|p| p.verdict == Verdict::StandsOut)
        .map(|p| (p.species_a.as_str(), p.species_b.as_str()))
        .collect();
    assert_eq!(loud, [("Blackbird", "Treecreeper")], "{:?}", a.pairs);
    let planted = &a.pairs[0];
    assert!(planted.lift.unwrap_or(0.0) > 2.0, "{planted:?}");
}

#[test]
fn two_sites_are_never_together() {
    let conn = station();
    let mut rows = Vec::new();
    for back in 1..=10 {
        let d = format!("DATE('now', 'localtime', '-{back} days')");
        rows.push(format!(
            "({d},'07:00:00','Erithacus rubecula','Robin',0.9,'garden')"
        ));
        rows.push(format!(
            "({d},'07:01:00','Troglodytes troglodytes','Wren',0.9,'cam1')"
        ));
    }
    conn.execute_batch(&format!(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Source) VALUES {};",
        rows.join(",")
    ))
    .unwrap();
    let a = cooccurrence_analysis(&conn, &CooccurrenceQuery::new(30, 10, 1)).unwrap();
    assert!(
        a.pairs.is_empty(),
        "a robin at one microphone and a wren at another were drawn as a pair: {:?}",
        a.pairs
    );
    // Counterpart: told the two are one site, they are a pair.
    let mut q = CooccurrenceQuery::new(30, 10, 1);
    q.sites = birdnet_db::sqlite::queries::events::SiteMap::identity()
        .with("garden", "yard")
        .with("cam1", "yard");
    let a = cooccurrence_analysis(&conn, &q).unwrap();
    assert_eq!(a.pairs.len(), 1, "{:?}", a.pairs);
    assert_eq!(a.pairs[0].site.as_deref(), Some("yard"));
    assert_eq!(a.pairs[0].shared_blocks, 10);
}

#[test]
fn a_short_window_says_it_cannot_tell() {
    let conn = station();
    let mut rows = Vec::new();
    for back in 1..=10 {
        let d = format!("DATE('now', 'localtime', '-{back} days')");
        for s in ["07:00:00", "07:20:00", "07:40:00"] {
            rows.push(format!(
                "({d},'{s}','Sitta europaea','Nuthatch',0.9,'local')"
            ));
            rows.push(format!(
                "({d},'{s}','Certhia familiaris','Treecreeper',0.9,'local')"
            ));
        }
        rows.push(format!(
            "({d},'12:00:00','Parus major','Great Tit',0.9,'local')"
        ));
    }
    conn.execute_batch(&format!(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Source) VALUES {};",
        rows.join(",")
    ))
    .unwrap();
    let a = cooccurrence_analysis(&conn, &CooccurrenceQuery::new(30, 10, 3)).unwrap();
    assert!(!a.pairs.is_empty());
    assert!(a.too_little_data, "{a:?}");
    assert!(a.pairs.iter().all(|p| p.verdict == Verdict::TooLittleData));
}

#[test]
fn the_seed_reproduces_the_answer() {
    let conn = station();
    fill(&conn, 0xC0FF_EE02, true);
    let one = cooccurrence_analysis(&conn, &query()).unwrap();
    let two = cooccurrence_analysis(&conn, &query()).unwrap();
    let ps = |a: &CooccurrenceAnalysis| {
        a.pairs
            .iter()
            .map(|p| (p.p_value, p.expected_shared))
            .collect::<Vec<_>>()
    };
    assert_eq!(ps(&one), ps(&two));
    assert_eq!(one.seed, birdnet_behavioral::null_model::DEFAULT_SEED);
}
