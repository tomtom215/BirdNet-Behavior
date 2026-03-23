//! Criterion benchmarks for the `birdnet-db` `SQLite` query layer.
//!
//! Measures throughput of the most performance-sensitive operations:
//! - Single detection insert
//! - Batch detection insert (100 rows)
//! - Top-species query (aggregation over a populated table)
//! - Recent-detections query (indexed fetch with LIMIT)
//! - Weekly heatmap query (GROUP BY date+hour)
//!
//! Run with:
//! ```bash
//! cargo bench -p birdnet-db
//! # HTML report: target/criterion/report/index.html
//! ```

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rusqlite::Connection;
use std::hint::black_box;

// ---------------------------------------------------------------------------
// Test database helpers
// ---------------------------------------------------------------------------

/// Open an in-memory `SQLite` database with the production schema applied.
fn open_db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory SQLite");
    conn.execute_batch(SCHEMA_SQL).expect("apply schema");
    conn
}

/// Insert `n` synthetic detection rows for benchmarking queries.
fn populate(conn: &Connection, n: usize) {
    let tx = conn.unchecked_transaction().expect("begin tx");
    let mut stmt = conn
        .prepare(
            "INSERT INTO detections
                (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff,
                 Week, Sens, Overlap, File_Name)
             VALUES (?1, ?2, ?3, ?4, ?5, 51.5, -0.1, 0.1, ?6, 1.25, 0.0, ?7)",
        )
        .expect("prepare insert");

    for i in 0..n {
        let species_idx = i % SPECIES.len();
        let (sci, com) = SPECIES[species_idx];
        let date = format!("2026-01-{:02}", (i % 28) + 1);
        let hour = i % 24;
        let time = format!("{hour:02}:00:00");
        #[allow(clippy::cast_precision_loss)]
        let confidence = ((i % 50) as f64).mul_add(0.01, 0.5);
        #[allow(clippy::cast_possible_truncation)]
        let week = (i % 52) as u32 + 1;
        let filename = format!("{date}-birdnet-{time}.wav");

        stmt.execute(rusqlite::params![
            date, time, sci, com, confidence, week, filename
        ])
        .expect("insert row");
    }
    tx.commit().expect("commit");
}

const SPECIES: &[(&str, &str)] = &[
    ("Turdus merula", "Eurasian Blackbird"),
    ("Erithacus rubecula", "European Robin"),
    ("Parus major", "Great Tit"),
    ("Fringilla coelebs", "Chaffinch"),
    ("Sylvia atricapilla", "Eurasian Blackcap"),
    ("Columba palumbus", "Common Wood Pigeon"),
    ("Troglodytes troglodytes", "Eurasian Wren"),
    ("Sturnus vulgaris", "Common Starling"),
    ("Hirundo rustica", "Barn Swallow"),
    ("Apus apus", "Common Swift"),
];

// Minimal production schema subset needed for benchmarks
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS detections (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    Date        TEXT NOT NULL,
    Time        TEXT NOT NULL,
    Sci_Name    TEXT NOT NULL,
    Com_Name    TEXT NOT NULL,
    Confidence  REAL NOT NULL DEFAULT 0.0,
    Lat         REAL,
    Lon         REAL,
    Cutoff      REAL NOT NULL DEFAULT 0.1,
    Week        INTEGER NOT NULL DEFAULT 0,
    Sens        REAL NOT NULL DEFAULT 1.25,
    Overlap     REAL NOT NULL DEFAULT 0.0,
    File_Name   TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_detections_date ON detections(Date);
CREATE INDEX IF NOT EXISTS idx_detections_com_name ON detections(Com_Name);
CREATE INDEX IF NOT EXISTS idx_detections_date_hour
    ON detections(Date, substr(Time, 1, 2));
";

// ---------------------------------------------------------------------------
// Insert benchmarks
// ---------------------------------------------------------------------------

fn bench_single_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/insert");

    group.bench_function("single_row", |b| {
        let conn = open_db();
        let mut n = 0_u64;
        b.iter(|| {
            conn.execute(
                "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
                 VALUES ('2026-01-01', '08:00:00', 'Turdus merula', 'Eurasian Blackbird',
                         0.87, 51.5, -0.1, 0.1, 10, 1.25, 0.0, '2026-01-01-birdnet-08:00:00.wav')",
                [],
            )
            .unwrap();
            n += 1;
            black_box(n)
        });
    });

    group.finish();
}

fn bench_batch_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/insert");

    for batch_size in [10_usize, 100, 1_000] {
        group.bench_with_input(
            BenchmarkId::new("batch_transaction", batch_size),
            &batch_size,
            |b, &n| {
                let conn = open_db();
                b.iter(|| {
                    let tx = conn.unchecked_transaction().unwrap();
                    for i in 0..n {
                        let (sci, com) = SPECIES[i % SPECIES.len()];
                        conn.execute(
                            "INSERT INTO detections
                             (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon,
                              Cutoff, Week, Sens, Overlap, File_Name)
                             VALUES (?1, ?2, ?3, ?4, ?5, 51.5, -0.1, 0.1, 10, 1.25, 0.0, ?6)",
                            rusqlite::params![
                                "2026-01-01",
                                "08:00:00",
                                sci,
                                com,
                                0.87,
                                "2026-01-01-birdnet-08:00:00.wav"
                            ],
                        )
                        .unwrap();
                    }
                    tx.commit().unwrap();
                });
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Query benchmarks (pre-populated table)
// ---------------------------------------------------------------------------

fn bench_top_species(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/query");

    let conn = open_db();
    populate(&conn, 10_000);

    group.bench_function("top_species_10000_rows", |b| {
        b.iter(|| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT Com_Name, COUNT(*) as count, AVG(Confidence) as avg_conf
                     FROM detections
                     GROUP BY Com_Name
                     ORDER BY count DESC
                     LIMIT 10",
                )
                .unwrap();
            let rows: Vec<(String, i64, f64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .filter_map(std::result::Result::ok)
                .collect();
            black_box(rows)
        });
    });

    group.finish();
}

fn bench_recent_detections(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/query");

    let conn = open_db();
    populate(&conn, 10_000);

    group.bench_function("recent_detections_limit50", |b| {
        b.iter(|| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT Date, Time, Com_Name, Confidence, File_Name
                     FROM detections
                     ORDER BY Date DESC, Time DESC
                     LIMIT 50",
                )
                .unwrap();
            let rows: Vec<(String, String, String, f64, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .unwrap()
                .filter_map(std::result::Result::ok)
                .collect();
            black_box(rows)
        });
    });

    group.finish();
}

fn bench_weekly_heatmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/query");

    let conn = open_db();
    populate(&conn, 10_000);

    group.bench_function("weekly_heatmap_10000_rows", |b| {
        b.iter(|| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT
                         strftime('%w', Date) as day_of_week,
                         CAST(substr(Time, 1, 2) AS INTEGER) as hour,
                         COUNT(*) as count
                     FROM detections
                     WHERE Date >= date('now', '-7 days')
                     GROUP BY day_of_week, hour
                     ORDER BY day_of_week, hour",
                )
                .unwrap();
            let rows: Vec<(String, i64, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .filter_map(std::result::Result::ok)
                .collect();
            black_box(rows)
        });
    });

    group.finish();
}

fn bench_species_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("sqlite/query");

    let conn = open_db();
    populate(&conn, 10_000);

    group.bench_function("species_search_like", |b| {
        b.iter(|| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT DISTINCT Com_Name, Sci_Name
                     FROM detections
                     WHERE Com_Name LIKE '%Robin%' OR Sci_Name LIKE '%robin%'
                     LIMIT 20",
                )
                .unwrap();
            let rows: Vec<(String, String)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .filter_map(std::result::Result::ok)
                .collect();
            black_box(rows)
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion entry points
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_single_insert,
    bench_batch_insert,
    bench_top_species,
    bench_recent_detections,
    bench_weekly_heatmap,
    bench_species_search,
);
criterion_main!(benches);
