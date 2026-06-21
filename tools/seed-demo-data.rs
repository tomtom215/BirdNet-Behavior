//! Seed a fresh `birds.db` with a deterministic, clearly-tagged demo
//! dataset. Used by the screenshot pipeline to produce data-bearing
//! shots of every dashboard / analytics screen without a real
//! detection daemon.
//!
//! ## Usage
//!
//! ```bash
//! cargo run --bin seed-demo-data -- --db /tmp/demo/birds.db
//! ```
//!
//! ## What lands
//!
//! Exactly **1 500 detections** over the trailing 30 days, distributed
//! across 16 northeastern-US-garden species (15 common + 1 rare-bird
//! sentinel: Painted Bunting). Diurnal peak at dawn 05:00–07:00 with
//! a smaller dusk hump at 18:00–20:00; very few overnight detections.
//! Confidence distribution: centred at 0.78, σ≈0.10, clamped to
//! [0.40, 0.99]; ~5 % of detections fall below the typical 0.7 quarantine
//! threshold so the quality / quarantine pages show realistic data.
//!
//! ## Determinism
//!
//! A fixed SplitMix64 seed makes the same call produce the same dataset
//! every time, so screenshots taken before and after a code change
//! show only chrome differences, not data noise.
//!
//! ## Demo-tagging contract
//!
//! Every row is tagged `correlation_id = "demo-seed-{n}"` so an operator
//! can `DELETE FROM detections WHERE correlation_id LIKE 'demo-seed-%'`
//! to remove the seed cleanly without affecting real captures. The site
//! name "BirdNet-Behavior Demo Garden" is set via
//! `settings.station_name`.

// Demo seeder uses ad-hoc numeric casts (calendar math, distribution
// sampling) and short math identifiers. Production code keeps these
// tight; a one-shot tool does not need the same surface.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::unnecessary_cast,
    clippy::doc_markdown,
    clippy::missing_const_for_fn
)]

use std::path::PathBuf;
use std::process::ExitCode;

use birdnet_db::sqlite::{DetectionRecord, insert_detection, open_or_create};
use rusqlite::params;

/// Fixed seed so repeated invocations produce identical data.
const SEED: u64 = 0x1779_5BAD_CAFE_F00D;
const ROWS_TARGET: usize = 1_500;
const DAYS: i64 = 30;
const STATION_NAME: &str = "BirdNet-Behavior Demo Garden";

/// 16 northeastern-US-garden species. The last row (Painted Bunting) is
/// the deliberate rare-bird sentinel — vanishingly small probability,
/// chosen so the rare-bird share / quarantine surfaces have content to
/// show even on a fresh capture.
const SPECIES: &[(&str, &str, f32, u8)] = &[
    // (Sci_Name, Com_Name, relative-frequency weight, peak-hour bias)
    ("Turdus migratorius", "American Robin", 24.0, 5),
    ("Cardinalis cardinalis", "Northern Cardinal", 18.0, 6),
    ("Melospiza melodia", "Song Sparrow", 14.0, 6),
    ("Poecile atricapillus", "Black-capped Chickadee", 12.0, 7),
    ("Zonotrichia albicollis", "White-throated Sparrow", 10.0, 6),
    ("Passer domesticus", "House Sparrow", 9.0, 9),
    ("Corvus brachyrhynchos", "American Crow", 8.0, 8),
    ("Baeolophus bicolor", "Tufted Titmouse", 7.0, 7),
    ("Sayornis phoebe", "Eastern Phoebe", 6.0, 6),
    ("Thryothorus ludovicianus", "Carolina Wren", 5.5, 6),
    ("Sitta carolinensis", "White-breasted Nuthatch", 5.0, 8),
    ("Zenaida macroura", "Mourning Dove", 4.5, 7),
    ("Dryobates pubescens", "Downy Woodpecker", 4.0, 9),
    ("Troglodytes aedon", "House Wren", 3.5, 6),
    ("Sialia sialis", "Eastern Bluebird", 3.0, 7),
    ("Passerina ciris", "Painted Bunting", 0.3, 10), // rare sentinel
];

#[derive(Debug)]
struct Args {
    db: PathBuf,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("seed-demo-data: {e}");
            eprintln!("usage: seed-demo-data --db <PATH>");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(n) => {
            println!(
                "seed-demo-data: inserted {n} detections into {}",
                args.db.display()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("seed-demo-data: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut db: Option<PathBuf> = None;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--db" | "-d" => {
                db = it.next().map(PathBuf::from);
            }
            "--help" | "-h" => {
                println!("seed-demo-data — deterministic demo data for screenshot capture");
                println!();
                println!("  --db, -d PATH    target SQLite database (created if missing)");
                println!("  --help, -h       this help");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        db: db.ok_or_else(|| "missing required --db PATH".to_string())?,
    })
}

fn run(args: &Args) -> Result<usize, String> {
    if let Some(parent) = args.db.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let conn = open_or_create(&args.db).map_err(|e| format!("open_or_create: {e}"))?;

    // Snapshot the species-frequency totals so we can later assert the
    // distribution looks right.
    let weights_total: f32 = SPECIES.iter().map(|s| s.2).sum();
    let mut rng = SplitMix64 { state: SEED };

    // Set the demo-tagged station name so the dashboard hero / share
    // headers don't read "BirdNet-Behavior" generically. Idempotent
    // INSERT/REPLACE.
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("ensure settings table: {e}"))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('station_name', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![STATION_NAME],
    )
    .map_err(|e| format!("write station_name: {e}"))?;

    let mut inserted = 0_usize;
    for i in 0..ROWS_TARGET {
        let (sci, com, _, peak_hour) = pick_species(&mut rng, weights_total);
        // Days ago: skewed towards the recent past so the "today" /
        // "yesterday" pages have content. Triangular distribution.
        let days_ago_f = triangular(&mut rng, 0.0, DAYS as f32 - 1.0, 2.0);
        let days_ago = days_ago_f as i64;
        let (year, month, day) = ymd_from_days_ago(days_ago);

        // Hour-of-day: bimodal peak at `peak_hour` (dawn) + secondary
        // smaller bump at 19:00 (dusk).
        let dawn_h = gauss(&mut rng, f32::from(peak_hour), 1.4);
        let dusk_h = gauss(&mut rng, 19.0, 1.0);
        let hour_f = if next_u32(&mut rng) % 100 < 65 {
            dawn_h
        } else {
            dusk_h
        }
        .clamp(0.0, 23.999);
        let hour = hour_f as u32;
        let minute = ((hour_f.fract() * 60.0) as u32).min(59);
        let second = (next_u32(&mut rng) % 60) as u32;

        // Confidence: gaussian centred 0.78 σ=0.10, clamped to
        // [0.40, 0.99]. ~5 % land below 0.7 so the quarantine page
        // has rows.
        let confidence = gauss(&mut rng, 0.78, 0.10).clamp(0.40, 0.99);

        let date_str = format!("{year:04}-{month:02}-{day:02}");
        let time_str = format!("{hour:02}:{minute:02}:{second:02}");
        let file_name = format!(
            "By_Date/{year:04}-{month:02}-{day:02}/{com_safe}/demo-{i:04}.wav",
            com_safe = com.replace(' ', "_"),
        );
        let correlation_id = format!("demo-seed-{i:04}");
        let week = iso_week(year, month, day);

        let record = DetectionRecord {
            date: &date_str,
            time: &time_str,
            sci_name: sci,
            com_name: com,
            confidence: f64::from(confidence),
            lat: Some(42.3601),
            lon: Some(-71.0589),
            cutoff: Some(0.70),
            week: Some(i64::from(week)),
            sensitivity: Some(1.0),
            overlap: Some(0.5),
            file_name: &file_name,
            chunk_offset_secs: Some(0.0),
            correlation_id: Some(&correlation_id),
            source: Some("local"),
            // Demo clips are a realistic ~9 s (a 3 s detection plus padding).
            duration_secs: Some(9.0),
        };
        // Each row carries a unique correlation_id and a unique file_name,
        // so the schema's UNIQUE key never trips. Failures here are real.
        insert_detection(&conn, &record).map_err(|e| format!("insert row {i}: {e}"))?;
        inserted += 1;
    }

    Ok(inserted)
}

// ---------------------------------------------------------------------------
// Deterministic randomness (no rand crate; project pattern uses
// SplitMix64 over a fixed seed)
// ---------------------------------------------------------------------------

struct SplitMix64 {
    state: u64,
}

fn next_u64(rng: &mut SplitMix64) -> u64 {
    rng.state = rng.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = rng.state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn next_u32(rng: &mut SplitMix64) -> u32 {
    (next_u64(rng) >> 32) as u32
}

fn uniform_01(rng: &mut SplitMix64) -> f32 {
    // 24 bits → [0, 1) with no bias.
    (next_u32(rng) >> 8) as f32 / (1_u32 << 24) as f32
}

/// Standard-normal sample via Box–Muller.
fn gauss(rng: &mut SplitMix64, mean: f32, sigma: f32) -> f32 {
    let u1 = uniform_01(rng).max(f32::MIN_POSITIVE);
    let u2 = uniform_01(rng);
    let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
    mean + sigma * z
}

/// Triangular distribution on `[lo, hi]` peaking at `lo` and falling
/// linearly toward `hi`. Useful for "more recent days = more dense".
fn triangular(rng: &mut SplitMix64, lo: f32, hi: f32, _shape: f32) -> f32 {
    // Pure descending-triangular CDF: x = lo + (hi-lo)*(1 - sqrt(u))
    let u = uniform_01(rng);
    lo + (hi - lo) * (1.0 - u.sqrt())
}

fn pick_species<'a>(rng: &mut SplitMix64, weights_total: f32) -> (&'a str, &'a str, f32, u8) {
    let r = uniform_01(rng) * weights_total;
    let mut acc = 0.0_f32;
    for s in SPECIES {
        acc += s.2;
        if acc >= r {
            return *s;
        }
    }
    *SPECIES.last().expect("non-empty species table")
}

// ---------------------------------------------------------------------------
// Calendar helpers (no chrono — hand-roll over UNIX_EPOCH to avoid the
// no-new-deps rule)
// ---------------------------------------------------------------------------

fn ymd_from_days_ago(days_ago: i64) -> (i32, u32, u32) {
    let secs_now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let target_secs = secs_now.saturating_sub(days_ago * 86_400);
    epoch_to_ymd_utc(target_secs)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]
fn epoch_to_ymd_utc(secs: i64) -> (i32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = (y + i64::from(m <= 2)) as i32;
    (year, m as u32, d as u32)
}

/// ISO 8601 week number. Faithful for the date range this seeder uses;
/// the algorithm matches Hinnant's "Algorithm 6.4".
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn iso_week(year: i32, month: u32, day: u32) -> u32 {
    let ord = ordinal_day(year, month, day);
    let dow = day_of_week(year, month, day); // 1=Mon..7=Sun
    let week = (10 + ord as i32 - dow as i32) / 7;
    if week < 1 {
        let prev = year - 1;
        let prev_ord = ordinal_day(prev, 12, 31);
        let prev_dow = day_of_week(prev, 12, 31);
        ((10 + prev_ord as i32 - prev_dow as i32) / 7) as u32
    } else if week > 52 {
        // Week 53 only when Jan 1 is Thu (or Wed in a leap year).
        let jan1_dow = day_of_week(year + 1, 1, 1);
        if jan1_dow == 5 || (jan1_dow == 6 && is_leap(year)) {
            53
        } else {
            1
        }
    } else {
        week as u32
    }
}

fn ordinal_day(year: i32, month: u32, day: u32) -> u32 {
    let mdays = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut total = day;
    for m in 0..(month as usize - 1).min(11) {
        total += mdays[m];
    }
    if month > 2 && is_leap(year) {
        total += 1;
    }
    total
}

const fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Day of week: 1=Mon..7=Sun. Zeller's congruence.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
fn day_of_week(year: i32, month: u32, day: u32) -> u32 {
    let (m, y) = if month < 3 {
        (month + 12, year - 1)
    } else {
        (month, year)
    };
    let k = (y as i64).rem_euclid(100);
    let j = (y as i64).div_euclid(100);
    let h = ((day as i64) + (13 * (m as i64 + 1)) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7);
    // h: 0=Saturday, 1=Sunday, 2=Monday, ... → convert to 1=Mon..7=Sun
    ((h + 5).rem_euclid(7) + 1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_db() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("birds.db");
        (dir, path)
    }

    #[test]
    fn epoch_to_ymd_known_anchors() {
        // 1970-01-01 00:00:00 UTC = epoch 0
        assert_eq!(epoch_to_ymd_utc(0), (1970, 1, 1));
        // 2000-01-01 00:00:00 UTC = 946 684 800
        assert_eq!(epoch_to_ymd_utc(946_684_800), (2000, 1, 1));
        // 2024-02-29 00:00:00 UTC (leap day) = 1 709 164 800
        assert_eq!(epoch_to_ymd_utc(1_709_164_800), (2024, 2, 29));
    }

    #[test]
    fn iso_week_for_first_monday_of_year_is_1() {
        // 2024-01-01 was a Monday → ISO week 1.
        assert_eq!(iso_week(2024, 1, 1), 1);
    }

    #[test]
    fn iso_week_for_mid_year_is_in_range() {
        let w = iso_week(2026, 5, 28);
        assert!((20..=22).contains(&w), "got week {w}");
    }

    #[test]
    fn deterministic_seed_produces_repeatable_first_species() {
        // Two RNGs initialised with the same seed must agree.
        let mut a = SplitMix64 { state: SEED };
        let mut b = SplitMix64 { state: SEED };
        let weights_total: f32 = SPECIES.iter().map(|s| s.2).sum();
        let pa = pick_species(&mut a, weights_total);
        let pb = pick_species(&mut b, weights_total);
        assert_eq!(pa.0, pb.0); // sci_name
        assert_eq!(pa.1, pb.1); // com_name
    }

    #[test]
    fn seed_inserts_the_target_row_count() {
        let (_d, path) = fresh_db();
        let n = run(&Args { db: path.clone() }).unwrap();
        assert_eq!(n, ROWS_TARGET);

        // All rows carry the demo-seed correlation_id prefix so an
        // operator can clean them out without touching real data.
        let conn = open_or_create(&path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE correlation_id LIKE 'demo-seed-%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, ROWS_TARGET as i64);
    }

    #[test]
    fn seed_writes_demo_station_name() {
        let (_d, path) = fresh_db();
        let _ = run(&Args { db: path.clone() }).unwrap();
        let conn = open_or_create(&path).unwrap();
        let name: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'station_name'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, STATION_NAME);
    }

    #[test]
    fn seed_covers_at_least_eight_species() {
        // With a 1500-row sample over a 16-species table, almost every
        // species should appear at least once (Painted Bunting at
        // weight 0.3 has ~0.2 % expected share = ~3 rows on average).
        let (_d, path) = fresh_db();
        let _ = run(&Args { db: path.clone() }).unwrap();
        let conn = open_or_create(&path).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(DISTINCT Sci_Name) FROM detections", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(n >= 8, "expected ≥8 species, got {n}");
    }

    #[test]
    fn seed_confidence_distribution_lands_in_realistic_range() {
        let (_d, path) = fresh_db();
        let _ = run(&Args { db: path.clone() }).unwrap();
        let conn = open_or_create(&path).unwrap();
        let avg: f64 = conn
            .query_row("SELECT AVG(Confidence) FROM detections", [], |r| r.get(0))
            .unwrap();
        // The seeder draws confidences centred on 0.78 σ=0.10 then
        // clamps to [0.40, 0.99]; with N=1500 the mean should land
        // within 0.05 of the target (Chebyshev: σ/√N ≪ 0.05).
        assert!((avg - 0.78).abs() < 0.05, "avg confidence drifted: {avg}");
    }
}
