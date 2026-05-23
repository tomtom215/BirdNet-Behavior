//! Throwaway dev server that serves `birdnet-web` against a rich, synthetic
//! dataset so a headless browser can screenshot every screen with realistic
//! content. NOT part of the shipped product — it exists purely for visual QA.
//!
//! Run:  `cargo run -p birdnet-web --example screenshot_server`
//! Open: <http://127.0.0.1:8502/>
//!
//! The fixture spans ~365 days across all 24 hours for ~15 species with a
//! dawn-chorus-weighted hourly distribution, a couple of nocturnal/migratory
//! profiles, varied confidence, plus seeded quarantine, notification and
//! alert-rule rows so the analytics visualisations render fully.

// Throwaway data-seeding example: numeric casts are intentional and bounded,
// and the pedantic style lints below are noise for one-shot fixture code.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::missing_const_for_fn,
    clippy::suboptimal_flops
)]

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use rusqlite::{Connection, params};

/// Deterministic xorshift64* PRNG — reproducible fixtures, no extra deps.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform float in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Inclusive integer in [lo, hi].
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + (self.unit() * f64::from(hi - lo + 1)) as u32
    }
}

/// Hinnant civil-from-days: days since Unix epoch -> (year, month, day).
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Day-of-year (1..=366) for an offset-from-today day number.
fn day_of_year(year: i32, month: u32, day: u32) -> u32 {
    let cum = [0u32, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let mut doy = cum[(month - 1) as usize] + day;
    if leap && month > 2 {
        doy += 1;
    }
    doy
}

#[derive(Clone, Copy)]
enum Profile {
    /// Active through daylight, gentle dawn peak.
    Diurnal,
    /// Heavy dawn-chorus concentration 5–8 AM.
    Dawn,
    /// Active at night, silent midday.
    Nocturnal,
    /// Diurnal but only present in spring + fall migration windows.
    Migratory,
}

struct Species {
    sci: &'static str,
    com: &'static str,
    /// Relative abundance — drives detections per day.
    weight: f64,
    base_conf: f64,
    profile: Profile,
    rare: bool,
    /// First day this species appears (day index from the oldest day, 0..365),
    /// so the life-list / accumulation curve grows realistically over the year.
    intro: u32,
}

/// Hourly sampling weight for a species profile.
fn hour_weight(profile: Profile, hour: u32) -> f64 {
    let h = f64::from(hour);
    match profile {
        Profile::Diurnal => {
            // gentle bell across daylight with a dawn bump
            let day = (-((h - 11.0) / 5.5).powi(2)).exp();
            let dawn = 0.6 * (-((h - 6.0) / 1.3).powi(2)).exp();
            (day + dawn).max(0.01)
        }
        Profile::Dawn => (-((h - 6.0) / 1.4).powi(2)).exp().max(0.01),
        Profile::Nocturnal => {
            // peaks around midnight, wraps across 0
            let d = (h - 0.0).min((24.0 - h).abs());
            (-(d / 3.2).powi(2)).exp().max(0.005)
        }
        Profile::Migratory => {
            let day = (-((h - 8.0) / 3.5).powi(2)).exp();
            day.max(0.01)
        }
    }
}

/// Seasonal multiplier (1.0 baseline) by day-of-year for migratory species.
fn seasonal(profile: Profile, doy: u32) -> f64 {
    match profile {
        Profile::Migratory => {
            let d = f64::from(doy);
            // spring (DOY ~100–135) and fall (~240–290) bumps
            let spring = 1.6 * (-((d - 118.0) / 16.0).powi(2)).exp();
            let fall = 1.4 * (-((d - 265.0) / 20.0).powi(2)).exp();
            (spring + fall).max(0.02)
        }
        _ => 1.0,
    }
}

fn species_table() -> Vec<Species> {
    use Profile::{Dawn, Diurnal, Migratory, Nocturnal};
    vec![
        Species {
            sci: "Cardinalis cardinalis",
            com: "Northern Cardinal",
            weight: 1.0,
            base_conf: 0.88,
            profile: Diurnal,
            rare: false,
            intro: 4,
        },
        Species {
            sci: "Poecile atricapillus",
            com: "Black-capped Chickadee",
            weight: 0.85,
            base_conf: 0.82,
            profile: Diurnal,
            rare: false,
            intro: 7,
        },
        Species {
            sci: "Turdus migratorius",
            com: "American Robin",
            weight: 0.95,
            base_conf: 0.86,
            profile: Dawn,
            rare: false,
            intro: 14,
        },
        Species {
            sci: "Cyanocitta cristata",
            com: "Blue Jay",
            weight: 0.8,
            base_conf: 0.9,
            profile: Diurnal,
            rare: false,
            intro: 22,
        },
        Species {
            sci: "Zenaida macroura",
            com: "Mourning Dove",
            weight: 0.7,
            base_conf: 0.84,
            profile: Dawn,
            rare: false,
            intro: 35,
        },
        Species {
            sci: "Spinus tristis",
            com: "American Goldfinch",
            weight: 0.6,
            base_conf: 0.8,
            profile: Diurnal,
            rare: false,
            intro: 48,
        },
        Species {
            sci: "Sitta carolinensis",
            com: "White-breasted Nuthatch",
            weight: 0.5,
            base_conf: 0.83,
            profile: Diurnal,
            rare: false,
            intro: 62,
        },
        Species {
            sci: "Melospiza melodia",
            com: "Song Sparrow",
            weight: 0.65,
            base_conf: 0.81,
            profile: Dawn,
            rare: false,
            intro: 78,
        },
        Species {
            sci: "Baeolophus bicolor",
            com: "Tufted Titmouse",
            weight: 0.55,
            base_conf: 0.84,
            profile: Diurnal,
            rare: false,
            intro: 96,
        },
        Species {
            sci: "Dryobates pubescens",
            com: "Downy Woodpecker",
            weight: 0.45,
            base_conf: 0.85,
            profile: Diurnal,
            rare: false,
            intro: 112,
        },
        Species {
            sci: "Setophaga coronata",
            com: "Yellow-rumped Warbler",
            weight: 0.7,
            base_conf: 0.78,
            profile: Migratory,
            rare: false,
            intro: 128,
        },
        Species {
            sci: "Agelaius phoeniceus",
            com: "Red-winged Blackbird",
            weight: 0.5,
            base_conf: 0.8,
            profile: Dawn,
            rare: false,
            intro: 142,
        },
        Species {
            sci: "Archilochus colubris",
            com: "Ruby-throated Hummingbird",
            weight: 0.35,
            base_conf: 0.76,
            profile: Migratory,
            rare: false,
            intro: 158,
        },
        Species {
            sci: "Bubo virginianus",
            com: "Great Horned Owl",
            weight: 0.12,
            base_conf: 0.79,
            profile: Nocturnal,
            rare: true,
            intro: 205,
        },
        // Non-Latin common name: exercises the async Noto Sans fallback font.
        Species {
            sci: "Corvus macrorhynchos",
            com: "ハシブトガラス",
            weight: 0.05,
            base_conf: 0.72,
            profile: Diurnal,
            rare: true,
            intro: 287,
        },
    ]
}

fn seed(conn: &Connection, today_days: i64) {
    let species = species_table();
    let mut rng = Rng::new(0xB17D_5EED_u64.rotate_left(7) ^ 0x9E37_79B9);
    let (lat, lon) = (42.3601, -71.0589); // Boston-ish

    let tx = conn.unchecked_transaction().expect("txn");

    for offset in 0..365i64 {
        let day = today_days - (364 - offset); // oldest first, offset 364 == today
        let (y, m, d) = civil_from_days(day);
        let date = format!("{y:04}-{m:02}-{d:02}");
        let doy = day_of_year(y, m, d);
        let week = (doy / 7 + 1).min(52);

        for sp in &species {
            // Species only appear once they've been "discovered" for the year.
            if (offset as u32) < sp.intro {
                continue;
            }
            let rare_factor = if sp.rare { 0.5 } else { 1.0 };
            let lambda = sp.weight * seasonal(sp.profile, doy) * rare_factor * 4.0;
            // Crude Poisson via fractional Bernoulli trials (integer-bounded).
            let trials = (lambda.ceil() as u32).min(30);
            let mut count = 0u32;
            for t in 0..trials {
                let p = (lambda - f64::from(t)).clamp(0.0, 1.0);
                if rng.unit() < p {
                    count += 1;
                }
            }
            for _ in 0..count {
                // sample hour by profile weight (rejection sampling)
                let hour = loop {
                    let h = rng.range(0, 23);
                    if rng.unit() < hour_weight(sp.profile, h) {
                        break h;
                    }
                };
                let minute = rng.range(0, 59);
                let second = rng.range(0, 59);
                let time = format!("{hour:02}:{minute:02}:{second:02}");
                let conf = (sp.base_conf + (rng.unit() - 0.5) * 0.28).clamp(0.51, 0.99);
                let file = format!(
                    "{}-{:.0}-{date}-birdnet-RTSP_1-{time}.wav",
                    sp.com.replace(' ', "_"),
                    conf * 100.0
                );
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    params![date, time, sp.sci, sp.com, conf, lat, lon, 0.7, week, 1.25, 0.0, file],
                );
            }
        }
    }
    tx.commit().expect("commit detections");

    // A handful of locked (pinned) recordings so the lock affordance shows.
    let _ = conn.execute(
        "UPDATE detections SET is_locked = 1 WHERE rowid IN (SELECT rowid FROM detections ORDER BY Confidence DESC LIMIT 6)",
        [],
    );

    seed_quarantine(conn, today_days);
    seed_notifications(conn, today_days);
    seed_alert_rules(conn);
    seed_thresholds(conn);
}

fn seed_quarantine(conn: &Connection, today_days: i64) {
    let rows = [
        (
            "Bubo virginianus",
            "Great Horned Owl",
            0.79,
            0.04,
            "below_sf_thresh",
            0,
        ),
        (
            "Corvus macrorhynchos",
            "ハシブトガラス",
            0.72,
            0.01,
            "below_sf_thresh",
            0,
        ),
        (
            "Buteo jamaicensis",
            "Red-tailed Hawk",
            0.63,
            0.11,
            "low_confidence",
            0,
        ),
        (
            "Megascops asio",
            "Eastern Screech-Owl",
            0.68,
            0.06,
            "below_sf_thresh",
            0,
        ),
        (
            "Spizelloides arborea",
            "American Tree Sparrow",
            0.58,
            0.09,
            "low_confidence",
            0,
        ),
        (
            "Haliaeetus leucocephalus",
            "Bald Eagle",
            0.81,
            0.02,
            "manual",
            0,
        ),
    ];
    for (i, (sci, com, conf, sf, reason, reviewed)) in rows.iter().enumerate() {
        let day = today_days - i as i64;
        let (y, m, d) = civil_from_days(day);
        let date = format!("{y:04}-{m:02}-{d:02}");
        let time = format!("{:02}:{:02}:00", 4 + i, (i * 7) % 60);
        let _ = conn.execute(
            "INSERT OR IGNORE INTO quarantine
             (date, time, sci_name, com_name, confidence, sf_probability, reason, reviewed, approved, file_name, lat, lon, week)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,?9,42.36,-71.06,20)",
            params![date, time, sci, com, conf, sf, reason, reviewed,
                format!("{}-{date}-{time}.wav", com.replace(' ', "_"))],
        );
    }
}

fn seed_notifications(conn: &Connection, today_days: i64) {
    let chans = [
        ("telegram", "sent"),
        ("email", "sent"),
        ("mqtt", "sent"),
        ("webhook", "failed"),
        ("telegram", "sent"),
        ("email", "skipped"),
        ("slack", "sent"),
        ("discord", "sent"),
        ("mqtt", "sent"),
        ("webhook", "sent"),
        ("telegram", "failed"),
        ("email", "sent"),
    ];
    let sp = [
        ("Great Horned Owl", "Bubo virginianus", 0.79),
        ("Northern Cardinal", "Cardinalis cardinalis", 0.94),
        ("ハシブトガラス", "Corvus macrorhynchos", 0.72),
        ("Bald Eagle", "Haliaeetus leucocephalus", 0.81),
    ];
    let (y, m, d) = civil_from_days(today_days);
    let date = format!("{y:04}-{m:02}-{d:02}");
    for (i, (chan, status)) in chans.iter().enumerate() {
        let (com, sci, conf) = sp[i % sp.len()];
        let hh = 23 - (i % 12);
        let sent_at = format!("{date} {hh:02}:{:02}:00", (i * 5) % 60);
        let err = if *status == "failed" {
            Some("connection timed out after 10s")
        } else {
            None
        };
        let _ = conn.execute(
            "INSERT INTO notification_log
             (sent_at, channel, species_com_name, species_sci_name, confidence, detection_date, detection_time, status, message, error)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![sent_at, chan, com, sci, conf, date, format!("{hh:02}:00:00"), status,
                format!("{com} detected at {conf:.2}"), err],
        );
    }
}

fn seed_alert_rules(conn: &Connection) {
    let _ = conn.execute(
        "INSERT INTO alert_rules (name, enabled, species_pattern, confidence_min, confidence_max, hour_start, hour_end, days_of_week, action_type, action_webhook_url)
         VALUES ('Rare bird webhook', 1, '%Owl%', 0.7, 1.0, NULL, NULL, NULL, 'webhook', 'https://example.com/hooks/rare')",
        [],
    );
    let _ = conn.execute(
        "INSERT INTO alert_rules (name, enabled, species_pattern, confidence_min, confidence_max, hour_start, hour_end, days_of_week, action_type)
         VALUES ('Night owl log', 1, 'Bubo virginianus', 0.6, 1.0, 20, 5, NULL, 'log')",
        [],
    );
    let _ = conn.execute(
        "INSERT INTO alert_rules (name, enabled, species_pattern, confidence_min, confidence_max, hour_start, hour_end, days_of_week, action_type)
         VALUES ('Suppress low-confidence', 0, NULL, 0.0, 0.55, NULL, NULL, NULL, 'suppress')",
        [],
    );
}

fn seed_thresholds(conn: &Connection) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO species_thresholds (sci_name, confidence_threshold) VALUES ('Bubo virginianus', 0.85)",
        [],
    );
    let _ = conn.execute(
        "INSERT OR IGNORE INTO species_thresholds (sci_name, confidence_threshold) VALUES ('Corvus macrorhynchos', 0.9)",
        [],
    );
}

#[tokio::main]
async fn main() {
    let path = std::env::temp_dir().join("bnb_screenshots.db");
    let _ = std::fs::remove_file(&path);

    let conn = Connection::open(&path).expect("open sqlite");
    birdnet_db::migration::migrate(&conn).expect("migrate");

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let today_days = (now / 86_400) as i64;

    eprintln!("seeding synthetic dataset…");
    seed(&conn, today_days);
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
        .unwrap_or(0);
    eprintln!("seeded {total} detections");

    let state =
        AppState::from_connection(conn, path).with_site_name("BirdNet-Behavior".to_string());
    let app = build_router(state);

    let addr = "127.0.0.1:8502";
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    eprintln!("screenshot server listening on http://{addr}/");
    axum::serve(listener, app).await.expect("serve");
}
