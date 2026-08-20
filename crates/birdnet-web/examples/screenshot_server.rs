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
    clippy::suboptimal_flops,
    clippy::items_after_statements
)]

use birdnet_web::rate_limit::RateLimitConfig;
use birdnet_web::server::build_router_with_rate_limit;
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

/// Days since the Unix epoch to `(year, month, day)`.
///
/// Delegates to `birdnet_core::civil`, which is the single implementation of
/// this arithmetic in the workspace.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let (y, m, d) = birdnet_core::civil::civil_from_days(days);
    (y as i32, m, d)
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
        // Rare vagrant — a vivid, plainly out-of-range bird that gives the
        // rare-bird, quarantine, and notification surfaces a clear example.
        Species {
            sci: "Passerina ciris",
            com: "Painted Bunting",
            weight: 0.05,
            base_conf: 0.72,
            profile: Diurnal,
            rare: true,
            intro: 287,
        },
    ]
}

/// Build a realistic capture-status snapshot for the Station Health screenshot:
/// a healthy local mic, a backing-off RTSP camera, and a stalled one. The
/// shipped binary fills this from the live supervisor; here it's seeded.
fn seed_capture_status() -> birdnet_core::audio::capture::CaptureStatusHandle {
    use birdnet_core::audio::capture::{
        CaptureStatus, SourceState, SourceStatus, UPTIME_SEGMENTS, UptimeSegment,
        new_capture_status, publish_capture_status,
    };

    // A 48-segment strip from a per-index closure (the first two half-hours
    // predate tracking → Out, like a station that booted ~23 h ago).
    fn strip(f: impl Fn(usize) -> UptimeSegment) -> Vec<UptimeSegment> {
        (0..UPTIME_SEGMENTS)
            .map(|i| if i < 2 { UptimeSegment::Out } else { f(i) })
            .collect()
    }

    let handle = new_capture_status();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let sources = vec![
        SourceStatus {
            label: "local".into(),
            state: SourceState::Connected,
            uptime_secs: Some(6 * 3600 + 12 * 60),
            last_audio_age_secs: Some(2),
            restart_attempts: 0,
            next_retry_in_secs: None,
            uptime_24h: strip(|_| UptimeSegment::Up),
        },
        SourceStatus {
            label: "RTSP_1".into(),
            state: SourceState::BackingOff,
            uptime_secs: None,
            last_audio_age_secs: Some(137),
            restart_attempts: 3,
            next_retry_in_secs: Some(12),
            // Solid all day, then a recent outage over the last ~90 minutes.
            uptime_24h: strip(|i| {
                if i >= UPTIME_SEGMENTS - 3 {
                    UptimeSegment::Down
                } else {
                    UptimeSegment::Up
                }
            }),
        },
        SourceStatus {
            label: "RTSP_2".into(),
            state: SourceState::Stalled,
            uptime_secs: None,
            last_audio_age_secs: Some(308),
            restart_attempts: 1,
            next_retry_in_secs: None,
            // Intermittent dropouts through the day, stalled right now.
            uptime_24h: strip(|i| {
                if i == UPTIME_SEGMENTS - 1 || i % 11 == 5 {
                    UptimeSegment::Down
                } else {
                    UptimeSegment::Up
                }
            }),
        },
    ];
    publish_capture_status(
        &handle,
        CaptureStatus {
            sources,
            published_unix: now,
        },
    );
    handle
}

fn seed(conn: &Connection, today_days: i64) {
    let species = species_table();
    let mut rng = Rng::new(0xB17D_5EED_u64.rotate_left(7) ^ 0x9E37_79B9);
    let (lat, lon) = (42.3601, -71.0589); // Boston-ish

    let tx = conn.unchecked_transaction().expect("txn");

    // Rotate detections across three audio sources so the Station Health
    // per-source panel and the Capture audio-source list show a realistic
    // multi-stream field deployment. Weighted toward the local mic.
    const SOURCES: [&str; 4] = ["local", "RTSP_1", "local", "RTSP_2"];
    let mut src_n = 0u64;

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
                let source = SOURCES[(src_n % SOURCES.len() as u64) as usize];
                src_n += 1;
                let file = format!(
                    "{}-{:.0}-{date}-birdnet-{source}-{time}.wav",
                    sp.com.replace(' ', "_"),
                    conf * 100.0
                );
                // Realistic clip lengths (~6–15 s) so the Clips grid's duration
                // column renders varied, plausible values in the fixture.
                let dur = 6.0 + rng.unit() * 9.0;
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO detections
                     (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, Source, Duration_Secs)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![date, time, sp.sci, sp.com, conf, lat, lon, 0.7, week, 1.25, 0.0, file, source, dur],
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
    seed_audio_sources(conn);
}

/// A few configured audio sources so the Station Capture tab's source list and
/// the Health per-source panel show a realistic multi-stream deployment.
fn seed_audio_sources(conn: &Connection) {
    for (id, kind, device, label) in [
        ("local", "usb-alsa", "hw:1,0", "Backyard mic"),
        ("RTSP_1", "rtsp", "rtsp://cam1/audio", "Front-yard RTSP"),
        ("RTSP_2", "rtsp", "rtsp://cam2/audio", "Pond RTSP"),
    ] {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO audio_sources (id, kind, device_id, label) VALUES (?1,?2,?3,?4)",
            params![id, kind, device, label],
        );
    }
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
            "Passerina ciris",
            "Painted Bunting",
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
        ("Painted Bunting", "Passerina ciris", 0.72),
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
        "INSERT OR IGNORE INTO species_thresholds (sci_name, confidence_threshold) VALUES ('Passerina ciris', 0.9)",
        [],
    );
}

/// Write synthetic bird-call WAVs for the given clip filenames into `dir`, so
/// the spectrogram-thumbnail route has real audio to render previews from.
fn seed_demo_audio(dir: &std::path::Path, clips: &[String]) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let sr = 48_000u32;
    for (i, name) in clips.iter().enumerate() {
        let samples = synth_call(sr, i as u32);
        let _ = write_wav16(&dir.join(name), sr, &samples);
    }
    eprintln!(
        "seeded {} demo audio clips for spectrogram previews",
        clips.len()
    );
}

/// Synthesize a short, structured "bird call": a few frequency-swept syllables
/// with two harmonics, varied per index so each thumbnail looks distinct.
fn synth_call(sr: u32, idx: u32) -> Vec<f32> {
    let secs = 2.6f32;
    let n = (secs * sr as f32) as usize;
    let f0 = 1200.0 + (idx % 8) as f32 * 420.0;
    let up = idx.is_multiple_of(2);
    let syllables = 3 + (idx % 3); // 3..=5
    let dt = 1.0 / sr as f32;
    let mut out = vec![0.0f32; n];
    let (mut p1, mut p2, mut p3) = (0.0f32, 0.0f32, 0.0f32);
    let tau = 2.0 * std::f32::consts::PI;
    for (i, s) in out.iter_mut().enumerate() {
        let t = i as f32 * dt;
        let pos = t / secs * syllables as f32;
        let frac = pos.fract();
        // Bell envelope within each syllable, silent in the trailing gap.
        let env = if frac < 0.85 {
            (std::f32::consts::PI * (frac / 0.85)).sin().powi(2)
        } else {
            0.0
        };
        let sweep = if up {
            1.0 + 0.6 * frac
        } else {
            1.6 - 0.6 * frac
        };
        let f = f0 * sweep;
        p1 += tau * f * dt;
        p2 += tau * f * 2.0 * dt;
        p3 += tau * f * 3.0 * dt;
        *s = (p1.sin() * 0.6 + p2.sin() * 0.25 + p3.sin() * 0.12) * env * 0.5;
    }
    out
}

/// Minimal 16-bit mono PCM WAV writer (no extra deps).
fn write_wav16(path: &std::path::Path, sr: u32, samples: &[f32]) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sr.to_le_bytes());
    buf.extend_from_slice(&(sr * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, buf)
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

    // The newest clips, so we can drop real audio in place: the spectrogram
    // thumbnail route renders previews only for clips whose audio is present,
    // so without these the Clips grid shows only the empty-spacer state.
    let demo_clips: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT File_Name FROM detections WHERE File_Name IS NOT NULL \
                 ORDER BY Date DESC, Time DESC LIMIT 20",
            )
            .expect("prepare recent clips");
        stmt.query_map([], |r| r.get::<_, String>(0))
            .expect("query recent clips")
            .filter_map(Result::ok)
            .collect()
    };

    // Run exactly like the shipped binary: analytics on *and active*, not
    // merely compiled in. Reopen the seeded SQLite through the analytics-aware
    // constructor so the DuckDB analytics database is opened and the detections
    // synced — the timeseries (window-function) screens then render live,
    // mirroring the real, zero-config user experience. Slim
    // `--no-default-features` builds fall back to the SQLite-only state.
    #[cfg(feature = "analytics")]
    let state = {
        drop(conn);
        let analytics_path = std::env::temp_dir().join("bnb_screenshots.duckdb");
        let _ = std::fs::remove_file(&analytics_path);
        AppState::new_with_analytics(path, &analytics_path)
            .expect("open analytics database")
            .with_site_name("BirdNet-Behavior".to_string())
    };
    #[cfg(not(feature = "analytics"))]
    let state =
        AppState::from_connection(conn, path).with_site_name("BirdNet-Behavior".to_string());

    // Wire a Wikipedia-backed species image cache, like the shipped binary, so
    // the gallery and species photos actually populate (without it the
    // `/species/image/<sci>/file` endpoint returns "image cache not configured"
    // and the gallery only ever shows the coloured code placeholders). Fetches
    // are cached under a temp dir; if construction fails the server still runs,
    // just with placeholder thumbnails.
    let state = match birdnet_integrations::species_images::ImageCache::with_wikipedia(
        &std::env::temp_dir().join("bnb_screenshots_images"),
    ) {
        Ok(cache) => state.with_image_cache(cache),
        Err(e) => {
            eprintln!("image cache unavailable ({e}); gallery will show code placeholders");
            state
        }
    };

    // Seed live capture-supervisor health so Station Health shows the
    // operator-grade per-source cards (this server runs no real supervisor).
    // Labels match the seeded detection sources so "detections today" merges.
    let state = state.with_capture_status(seed_capture_status());

    // Drop synthetic bird-call WAVs at the newest clips' paths so the
    // Recordings grid shows real spectrogram thumbnails (the route renders from
    // these and caches the PNGs under the data dir's `spectrograms/`).
    seed_demo_audio(&state.recording_dir(), &demo_clips);

    // This fixture exists to be hammered: the visual-QA sweep captures 152
    // pages back to back and the interaction gate drives controls as fast as
    // Chromium will go. The station's 30 req/s limiter is right for a station
    // and wrong here — it throttles the harness rather than the product, and an
    // unlucky burst surfaces as a `429` on a font, reported as a page issue and
    // a red build. Loopback-bound, synthetic data, no reason to throttle.
    let app = build_router_with_rate_limit(
        state,
        RateLimitConfig {
            requests_per_second: 100_000.0,
            burst_capacity: 100_000,
            trust_x_forwarded_for: false,
        },
    );

    let addr = "127.0.0.1:8502";
    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
    eprintln!("screenshot server listening on http://{addr}/");
    axum::serve(listener, app).await.expect("serve");
}
