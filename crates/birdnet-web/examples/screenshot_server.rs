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
    let leap = birdnet_core::civil::is_leap_year(u32::try_from(year.max(0)).unwrap_or(0));
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

/// Local sunrise hour (UTC, for the fixture's Greenwich station) on day-of-year
/// `doy`: about 04:30 at midsummer and 08:15 at midwinter. Dawn singers follow it,
/// so the chorus drifts through the year the way a real one does.
fn sunrise_hour(doy: u32) -> f64 {
    6.4 - 1.85 * (std::f64::consts::TAU * (f64::from(doy) - 172.0) / 365.0).cos()
}

/// Hourly sampling weight for a species profile, at the middle of `hour`.
fn hour_weight(profile: Profile, hour: u32, sunrise: f64) -> f64 {
    let h = f64::from(hour) + 0.5;
    let dawn = sunrise + 0.6;
    match profile {
        Profile::Diurnal => {
            // gentle bell across daylight with a dawn bump
            let day = (-((h - 12.0) / 5.5).powi(2)).exp();
            let bump = 0.6 * (-((h - dawn) / 1.3).powi(2)).exp();
            (day + bump).max(0.01)
        }
        Profile::Dawn => (-((h - dawn) / 1.4).powi(2)).exp().max(0.01),
        Profile::Nocturnal => {
            // peaks around midnight, wraps across 0
            let d = h.min(24.0 - h);
            (-(d / 3.2).powi(2)).exp().max(0.005)
        }
        Profile::Migratory => {
            let day = (-((h - (dawn + 1.5)) / 3.5).powi(2)).exp();
            day.max(0.01)
        }
    }
}

/// A seasonal multiplier by day-of-year, per species: residents sing up in spring and
/// visit feeders in winter, a robin is scarce in January, an owl calls in the winter,
/// a warbler passes through twice a year. (Greenwich-ish; the point is that a year of
/// the fixture is not a flat line, which hid every defect a trend chart can have.)
fn season_factor(sp: &Species, doy: u32) -> f64 {
    let d = f64::from(doy);
    // A bump on a circular year, 1.0 at `peak`.
    let bump = |peak: f64, width: f64| {
        let off = (d - peak + 182.5).rem_euclid(365.0) - 182.5;
        (-(off / width).powi(2)).exp()
    };
    match (sp.profile, sp.sci) {
        (Profile::Migratory, _) => {
            // spring (DOY ~100–135) and fall (~240–290) bumps
            let spring = 1.6 * (-((d - 118.0) / 16.0).powi(2)).exp();
            let fall = 1.4 * (-((d - 265.0) / 20.0).powi(2)).exp();
            (spring + fall).max(0.02)
        }
        (_, "Turdus migratorius") => 0.12 + 1.0 * bump(150.0, 55.0),
        (_, "Melospiza melodia") => 0.15 + 1.1 * bump(135.0, 45.0),
        (_, "Agelaius phoeniceus") => 0.10 + 1.3 * bump(120.0, 40.0),
        (_, "Zenaida macroura") => 0.40 + 0.8 * bump(170.0, 80.0),
        (_, "Spinus tristis") => 0.50 + 0.9 * bump(220.0, 60.0),
        (_, "Cyanocitta cristata") => 0.80 + 0.5 * bump(285.0, 40.0),
        (Profile::Nocturnal, _) => 0.20 + 2.2 * bump(30.0, 30.0),
        (Profile::Diurnal, _) => 0.55 + 0.45 * bump(135.0, 70.0) + 0.15 * bump(20.0, 40.0),
        _ => 1.0,
    }
}

/// A well-mixed 64-bit hash, so "the weather on day N" is a pure function of N.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A uniform in `[0, 1)` from `(day, stream)`.
fn hashed_unit(day: i64, stream: u64) -> f64 {
    (mix((day as u64)
        .wrapping_mul(0x0100_0000_01B3)
        .wrapping_add(stream))
        >> 11) as f64
        / (1u64 << 53) as f64
}

/// What the day was like, deterministically: it drives both the weather table and how
/// much the birds sang (rain keeps them quiet).
struct DayWeather {
    temp_c: f64,
    precip_mm: f64,
    wind_kt: f64,
    /// Multiplier on every species' rate that day.
    activity: f64,
}

fn day_weather(days_ago: i64, doy: u32) -> DayWeather {
    let seasonal = 11.0 - 9.0 * (std::f64::consts::TAU * (f64::from(doy) - 20.0) / 365.0).cos();
    // Warm and cold spells run for days, not hours.
    let spell = 3.5 * (days_ago as f64 / 9.0).sin();
    let temp_c = seasonal + spell + (hashed_unit(days_ago, 1) - 0.5) * 3.0;
    // Wet spells too: a bit under a third of days have measurable rain, in runs.
    let wet = (days_ago as f64 / 5.3).sin() + (hashed_unit(days_ago, 2) - 0.5) * 1.2;
    let precip_mm = if wet > 0.55 { (wet - 0.55) * 14.0 } else { 0.0 };
    let wind_kt = 3.0 + hashed_unit(days_ago, 3) * 10.0 + precip_mm;
    let base = if precip_mm > 6.0 {
        0.3
    } else if precip_mm > 1.0 {
        0.55
    } else if wind_kt > 14.0 {
        0.8
    } else if temp_c < 0.0 {
        0.7
    } else {
        1.0
    };
    DayWeather {
        temp_c,
        precip_mm,
        wind_kt,
        activity: base * (0.9 + 0.2 * hashed_unit(days_ago, 4)),
    }
}

/// Days (counted back from today) on which the station was switched off: no
/// recording, so no detections and, importantly, no *effort*: "not listening" is not
/// "quiet" and a chart has to be able to say which.
const OFF_DAYS: [i64; 8] = [12, 13, 41, 88, 143, 200, 201, 250];
/// Days it listened for only part of the day, and how much of it.
const PARTIAL_DAYS: [(i64, f64); 4] = [(6, 0.4), (22, 0.6), (70, 0.25), (160, 0.5)];
/// A day it listened all day and heard almost nothing (a fog, a gale): quiet, not off.
const SILENT_DAY: i64 = 30;
/// Flocks and fall-outs: `(days ago, scientific name, multiplier)`.
const BURSTS: [(i64, &str, f64); 6] = [
    (3, "Spinus tristis", 4.0),
    (4, "Spinus tristis", 3.0),
    (9, "Cyanocitta cristata", 5.0),
    (40, "Zenaida macroura", 3.5),
    (120, "Agelaius phoeniceus", 4.0),
    (230, "Turdus migratorius", 3.0),
];

/// Fraction of the day the station listened.
fn listening_fraction(days_ago: i64) -> f64 {
    if OFF_DAYS.contains(&days_ago) {
        0.0
    } else {
        PARTIAL_DAYS
            .iter()
            .find(|(d, _)| *d == days_ago)
            .map_or(1.0, |(_, f)| *f)
    }
}

/// A standard normal from two uniforms (Box–Muller).
fn gauss(rng: &mut Rng) -> f64 {
    let (u, v) = (rng.unit().max(1e-12), rng.unit());
    (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
}

/// A Poisson draw: exact below 25, normal approximation above it.
fn poisson(rng: &mut Rng, lambda: f64) -> u32 {
    if lambda <= 0.0 {
        return 0;
    }
    if lambda < 25.0 {
        let limit = (-lambda).exp();
        let (mut k, mut p) = (0_u32, 1.0_f64);
        loop {
            p *= rng.unit();
            if p <= limit {
                return k;
            }
            k += 1;
        }
    }
    (lambda + lambda.sqrt() * gauss(rng)).round().max(0.0) as u32
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
/// A made-up range-filter reading for the fixture's station (see the call
/// site): the names are real birds, the probabilities are not the model's.
fn seed_occurrence() -> birdnet_web::occurrence::OccurrenceReport {
    use birdnet_core::inference::species_filter::OccurrenceScore;
    let score = |sci: &str, com: &str, probability: f32, admitted: bool| OccurrenceScore {
        scientific_name: sci.to_owned(),
        common_name: com.to_owned(),
        probability,
        admitted,
    };
    // The birds the fixture's station hears are all admitted, at plausible odds.
    let mut species = vec![
        score("Turdus migratorius", "American Robin", 0.91, true),
        score("Cyanocitta cristata", "Blue Jay", 0.84, true),
        score("Poecile atricapillus", "Black-capped Chickadee", 0.8, true),
        score("Spinus tristis", "American Goldfinch", 0.62, true),
        score("Dryobates pubescens", "Downy Woodpecker", 0.55, true),
        score("Cardinalis cardinalis", "Northern Cardinal", 0.88, true),
        score("Zenaida macroura", "Mourning Dove", 0.7, true),
        score("Melospiza melodia", "Song Sparrow", 0.66, true),
        score("Bubo virginianus", "Great Horned Owl", 0.4, true),
        // Heard at the fixture's station, and held back this week: a species
        // page for either shows the link to why.
        score("Passerina ciris", "Painted Bunting", 0.008, false),
        score(
            "Archilochus colubris",
            "Ruby-throated Hummingbird",
            0.021,
            false,
        ),
        // Over the threshold and kept out by the operator's exclude list.
        score("Sturnus vulgaris", "European Starling", 0.5, false),
        // Under it, and let in by the Always allowed list.
        score("Bubo scandiacus", "Snowy Owl", 0.011, true),
        // Under it: held back by the range model.
        score("Pica pica", "Eurasian Magpie", 0.004, false),
        score("Spiza americana", "Dickcissel", 0.0004, false),
        score("Erithacus rubecula", "European Robin", 0.00003, false),
    ];
    // Enough more admitted birds to need a second page of the list, so the
    // page's paging link and its long-list layout are exercised.
    for n in 1..=130_u16 {
        let probability = 0.5 - f32::from(n) * 0.0035;
        species.push(score(
            &format!("Fixturus avis{n:03}"),
            &format!("Fixture warbler {n:03}"),
            probability,
            true,
        ));
    }
    species.sort_by(|a, b| b.probability.total_cmp(&a.probability));
    birdnet_web::occurrence::OccurrenceReport {
        latitude: 51.48,
        longitude: -0.13,
        week: 1,
        threshold: 0.03,
        species,
    }
}

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
            restarts_last_hour: 0,
            flapping: false,
            next_retry_in_secs: None,
            uptime_24h: strip(|_| UptimeSegment::Up),
        },
        SourceStatus {
            label: "RTSP_1".into(),
            state: SourceState::BackingOff,
            uptime_secs: None,
            last_audio_age_secs: Some(137),
            restart_attempts: 3,
            restarts_last_hour: 0,
            flapping: false,
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
            restarts_last_hour: 0,
            flapping: false,
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

/// How few detections "today" may hold, whatever the time of day. The Today page pages its
/// feed (`Load more`), and `interactions.mjs` needs more than one page of it; a fixture
/// that seeds only what has happened by the clock holds six rows at 05:00 UTC and turned
/// that gate red at certain hours of the day.
const MIN_TODAY_ROWS: usize = 24;

/// The shortest stretch of the day the top-up below may spread `MIN_TODAY_ROWS` over, so a run
/// just after midnight still has an hour chart with several bars in it. It is the one place
/// the fixture draws detections the clock has not reached yet (by under three hours, and only
/// when fewer than `MIN_TODAY_ROWS` have happened).
const MIN_TODAY_WINDOW_SECS: u32 = 10_800;

/// The fixture's idea of the current time in unix seconds. `BNB_FIXTURE_NOW=<seconds>`
/// replaces the clock the SEEDING reads, so a gate can be run as if it were 02:00 or 23:00
/// without waiting for it. The server itself still reads the real clock, so choose a time
/// on the real UTC date; on another date the page's "today" is not the fixture's.
fn fixture_now_secs() -> u64 {
    if let Some(v) = std::env::var("BNB_FIXTURE_NOW")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        return v;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn seed(conn: &Connection, today_days: i64) {
    let species = species_table();
    let mut rng = Rng::new(0xB17D_5EED_u64.rotate_left(7) ^ 0x9E37_79B9);
    // The station is at Greenwich, not Boston: the fixture's clock is UTC and its
    // detection times are naive "local" strings, so a station whose local time is UTC
    // is the only one whose sunrise lands where its dawn chorus does. The sun lines on
    // Today and the night bands on Patterns are drawn from these two settings.
    let (lat, lon) = (51.4778, -0.0015);
    seed_location(conn);

    // Today stops at the clock: a station cannot have heard what has not happened yet,
    // and the Today chart has a "now" marker that would otherwise sit left of its own
    // bars. Early in the day that leaves too few rows to page through, so when fewer than
    // `MIN_TODAY_ROWS` have happened the rest of the day's schedule is compressed into the
    // hours that have (see the top-up after the loop) rather than dropped.
    let cutoff_today = (fixture_now_secs() % 86_400) as u32;
    // Detections of today that fell after the clock, kept for that top-up:
    // (seconds into the day, species index).
    let mut late_today: Vec<(u32, usize)> = Vec::new();
    let mut kept_today = 0usize;

    let tx = conn.unchecked_transaction().expect("txn");

    // Rotate detections across three audio sources so the Station Health
    // per-source panel and the Capture audio-source list show a realistic
    // multi-stream field deployment. Weighted toward the local mic.
    const SOURCES: [&str; 4] = ["local", "RTSP_1", "local", "RTSP_2"];
    let mut src_n = 0u64;

    for offset in 0..365i64 {
        let days_ago = 364 - offset;
        let day = today_days - days_ago; // oldest first, offset 364 == today
        let (y, m, d) = civil_from_days(day);
        let date = format!("{y:04}-{m:02}-{d:02}");
        let doy = day_of_year(y, m, d);
        let week = (doy / 7 + 1).min(52);
        let sunrise = sunrise_hour(doy);
        let weather = day_weather(days_ago, doy);
        let listening = listening_fraction(days_ago);
        let day_factor = if days_ago == SILENT_DAY {
            0.03
        } else {
            weather.activity
        };

        for (sp_idx, sp) in species.iter().enumerate() {
            // Species only appear once they've been "discovered" for the year.
            if (offset as u32) < sp.intro {
                continue;
            }
            let rare_factor = if sp.rare { 0.5 } else { 1.0 };
            let burst = BURSTS
                .iter()
                .find(|(ago, sci, _)| *ago == days_ago && *sci == sp.sci)
                .map_or(1.0, |b| b.2);
            // A species has its own good and bad days on top of the weather's.
            let own = (0.35 * gauss(&mut rng)).exp();
            let lambda = sp.weight
                * season_factor(sp, doy)
                * rare_factor
                * 11.0
                * day_factor
                * listening
                * burst
                * own;
            let count = poisson(&mut rng, lambda).min(400);
            for _ in 0..count {
                // sample hour by profile weight (rejection sampling)
                let hour = loop {
                    let h = rng.range(0, 23);
                    if rng.unit() < hour_weight(sp.profile, h, sunrise) {
                        break h;
                    }
                };
                let minute = rng.range(0, 59);
                let second = rng.range(0, 59);
                if days_ago == 0 {
                    let at = hour * 3600 + minute * 60 + second;
                    if at > cutoff_today {
                        late_today.push((at, sp_idx));
                        continue; // has not happened yet
                    }
                    kept_today += 1;
                }
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

        // What the station listened to that day. A day with no row is a day it was
        // off; a day with a row and no detections was a quiet one.
        if listening > 0.0 {
            let secs = 86_400.0 * listening;
            let _ = tx.execute(
                "INSERT OR REPLACE INTO recording_effort (date, source, seconds) VALUES (?1, 'local', ?2)",
                params![date, secs],
            );
            if days_ago < 120 {
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO recording_effort (date, source, seconds) VALUES (?1, 'RTSP_2', ?2)",
                    params![date, secs * 0.9],
                );
            }
        }

        // Hourly weather for the last two months, today up to the current hour.
        if days_ago < 60 {
            let last_hour = if days_ago == 0 {
                (cutoff_today / 3600) as usize
            } else {
                23
            };
            for hour in 0..=last_hour {
                let diurnal = 4.5 * (std::f64::consts::TAU * (hour as f64 - 9.0) / 24.0).sin();
                let _ = tx.execute(
                    "INSERT OR REPLACE INTO weather (at, temp_c, precip_mm, wind_kt, wind_dir_deg, pressure_hpa, cloud_pct, code)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        format!("{date}T{hour:02}:00:00Z"),
                        weather.temp_c + diurnal,
                        weather.precip_mm / 24.0,
                        weather.wind_kt,
                        (days_ago * 37) % 360,
                        1013.0 - weather.precip_mm,
                        if weather.precip_mm > 1.0 { 90 } else { 30 },
                        if weather.precip_mm > 1.0 { 61 } else { 1 },
                    ],
                );
            }
        }
    }
    // Top-up: if fewer than `MIN_TODAY_ROWS` of today's detections had happened by the
    // clock, bring the first of the rest forward and spread them evenly over the hours that
    // have (at least `MIN_TODAY_WINDOW_SECS` of them), so today always has a page and a bit
    // to scroll and the hour chart has bars at any time of day. At the usual hour nothing
    // here runs.
    if kept_today < MIN_TODAY_ROWS {
        late_today.sort_unstable();
        let window = cutoff_today.max(MIN_TODAY_WINDOW_SECS);
        let (y, m, d) = civil_from_days(today_days);
        let date = format!("{y:04}-{m:02}-{d:02}");
        let week = (day_of_year(y, m, d) / 7 + 1).min(52);
        let take = (MIN_TODAY_ROWS - kept_today).min(late_today.len());
        for (i, &(at, sp_idx)) in late_today.iter().take(take).enumerate() {
            let sp = &species[sp_idx];
            // Evenly, so no two share a second (the table is keyed on the time, and a dropped
            // insert would leave the count short) and every hour of the window has some.
            let secs = (((i as f64 + 0.5) / take as f64) * f64::from(window)) as u32;
            let time = format!(
                "{:02}:{:02}:{:02}",
                secs / 3600,
                secs % 3600 / 60,
                secs % 60
            );
            let source = SOURCES[(src_n % SOURCES.len() as u64) as usize];
            src_n += 1;
            let conf = sp.base_conf.clamp(0.51, 0.99);
            let file = format!(
                "{}-{:.0}-{date}-birdnet-{source}-{time}.wav",
                sp.com.replace(' ', "_"),
                conf * 100.0
            );
            let _ = tx.execute(
                "INSERT OR IGNORE INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, Source, Duration_Secs)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![date, time, sp.sci, sp.com, conf, lat, lon, 0.7, week, 1.25, 0.0, file, source, 6.0 + f64::from(at % 9)],
            );
        }
    }
    tx.commit().expect("commit detections");
    {
        let (y, m, d) = civil_from_days(today_days);
        let today: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM detections WHERE Date = ?1",
                params![format!("{y:04}-{m:02}-{d:02}")],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert!(
            today >= MIN_TODAY_ROWS as i64,
            "the fixture must hold at least {MIN_TODAY_ROWS} detections for today, found {today}"
        );
        eprintln!("today holds {today} detections (clock cutoff {cutoff_today} s into the day)");
    }

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

/// The station's location, so the sun lines and the night bands draw (see `seed`).
fn seed_location(conn: &Connection) {
    for (k, v) in [("latitude", "51.4778"), ("longitude", "-0.0015")] {
        let _ =
            birdnet_db::settings::set(conn, k, v, birdnet_db::settings::SettingsCategory::Location);
    }
}

/// Profile `empty`: a configured station that has heard nothing yet.
fn seed_profile_empty(conn: &Connection) {
    seed_location(conn);
    seed_audio_sources(conn);
}

/// Profile `one`: exactly one detection, this morning, and a ledger that says the station
/// has been listening for a day. Every chart must draw one point honestly.
fn seed_profile_one(conn: &Connection, today_days: i64) {
    seed_location(conn);
    let (y, m, d) = civil_from_days(today_days);
    let date = format!("{y:04}-{m:02}-{d:02}");
    let _ = conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, Source, Duration_Secs)
         VALUES (?1,'06:30:00','Cyanocitta cristata','Blue Jay',0.93,51.4778,-0.0015,0.7,42,1.25,0.0,?2,'local',8.0)",
        params![date, format!("Blue_Jay-93-{date}-birdnet-local-06:30:00.wav")],
    );
    let _ = conn.execute(
        "INSERT OR REPLACE INTO recording_effort (date, source, seconds) VALUES (?1, 'local', 86400)",
        params![date],
    );
    seed_audio_sources(conn);
}

/// Profile `stress`: the shapes that break a chart's layout.
///
/// * sixty species, so the streamgraph folds fifty-two of them into one band and the
///   legend has to wrap;
/// * names from four letters to sixty-odd characters, so a direct label, a tooltip row
///   and a legend entry each have to cope with a name that does not fit;
/// * a full year of days, so the 90-day and 365-day ranges draw 90 and 365 columns;
/// * an hour yesterday with 1.6 million detections in it, so an axis label, a tooltip
///   number and a peak annotation are all asked to hold a seven-digit count.
fn seed_profile_stress(conn: &Connection, today_days: i64) {
    seed_location(conn);
    let mut rng = Rng::new(0x57_E55_u64.rotate_left(9) ^ 0x1234_5678);
    let long_names = [
        "Extraordinarily Long-named Warbler of the Far Northern Highland Forests",
        "Black-and-white Striped Tropical Mockingbird Imitator",
        "Eurasian",
        "Tit",
    ];
    let words = [
        "Northern", "Southern", "Western", "Eastern", "Common", "Lesser", "Greater", "Spotted",
        "Crested", "Rufous",
    ];
    let kinds = [
        "Warbler",
        "Sparrow",
        "Finch",
        "Thrush",
        "Wren",
        "Swift",
        "Tanager",
        "Flycatcher",
        "Bunting",
        "Vireo",
    ];
    let mut species: Vec<(String, String)> = Vec::new();
    for (i, n) in long_names.iter().enumerate() {
        species.push(((*n).to_string(), format!("Stressus longus{i}")));
    }
    for i in 0..56 {
        species.push((
            format!(
                "{} {} {}",
                words[i % 10],
                kinds[(i / 10 + i) % 10],
                i / 20 + 1
            )
            .replace(" 1", ""),
            format!("Stressus bird{i}"),
        ));
    }
    let tx = conn.unchecked_transaction().expect("txn");
    for days_ago in 0..365_i64 {
        let (y, m, d) = civil_from_days(today_days - days_ago);
        let date = format!("{y:04}-{m:02}-{d:02}");
        let doy = day_of_year(y, m, d);
        let season = 0.6
            + 0.4
                * (std::f64::consts::TAU * (f64::from(doy) - 110.0) / 365.0)
                    .cos()
                    .abs();
        for (k, (com, sci)) in species.iter().enumerate() {
            // A long tail: the first few species are common, the rest are heard now and then.
            let lambda = 6.0 / (1.0 + k as f64 * 0.35) * season;
            for _ in 0..poisson(&mut rng, lambda) {
                let hour = rng.range(4, 21);
                let (minute, second) = (rng.range(0, 59), rng.range(0, 59));
                let time = format!("{hour:02}:{minute:02}:{second:02}");
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, Source, Duration_Secs)
                     VALUES (?1,?2,?3,?4,0.85,51.4778,-0.0015,0.7,?5,1.25,0.0,?6,'local',8.0)",
                    params![date, time, sci, com, (doy / 7 + 1).min(52), format!("s{k}-{date}-{time}.wav")],
                );
            }
        }
        let _ = tx.execute(
            "INSERT OR REPLACE INTO recording_effort (date, source, seconds) VALUES (?1, 'local', 86400)",
            params![date],
        );
    }
    // 1.6 million detections inside one hour yesterday.
    let (y, m, d) = civil_from_days(today_days - 1);
    let _ = tx.execute(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1600000)
         INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, Source, Duration_Secs)
         SELECT ?1, printf('06:%02d:%02d', (i / 60) % 60, i % 60), ?3, ?2, 0.9, 51.4778, -0.0015, 0.7, 42, 1.25, 0.0, printf('huge-%d.wav', i), 'local', 8.0 FROM n",
        params![format!("{y:04}-{m:02}-{d:02}"), species[4].0, species[4].1],
    );
    tx.commit().expect("commit stress detections");
    seed_audio_sources(conn);
}

/// A few configured audio sources so the Station Capture tab's source list and
/// the Health per-source panel show a realistic multi-stream deployment.
/// Write one snapshot into the backup ring beside the database, named the way
/// `backup_database` names them (`birds.db.backup.<unix_secs>`), an hour old.
fn seed_backup_snapshot(db_path: &std::path::Path) {
    let dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let db_name = db_path.file_name().map_or_else(
        || "birds.db".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .saturating_sub(3600);
    let path = dir.join(format!("{db_name}.backup.{stamp}"));
    // A recognisable size in the listing; the content is not read.
    let _ = std::fs::write(&path, vec![0_u8; 6 * 1024 * 1024]);
}

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

/// A tiny but valid JPEG, so a picture in the fixture's cache is a picture and not a broken image.
const TINY_JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xdb, 0x00, 0x43, 0x00, 0x10, 0x0b, 0x0c, 0x0e, 0x0c, 0x0a, 0x10,
    0x0e, 0x0d, 0x0e, 0x12, 0x11, 0x10, 0x13, 0x18, 0x28, 0x1a, 0x18, 0x16, 0x16, 0x18, 0x31, 0x23,
    0x25, 0x1d, 0x28, 0x3a, 0x33, 0x3d, 0x3c, 0x39, 0x33, 0x38, 0x37, 0x40, 0x48, 0x5c, 0x4e, 0x40,
    0x44, 0x57, 0x45, 0x37, 0x38, 0x50, 0x6d, 0x51, 0x57, 0x5f, 0x62, 0x67, 0x68, 0x67, 0x3e, 0x4d,
    0x71, 0x79, 0x70, 0x64, 0x78, 0x5c, 0x65, 0x67, 0x63, 0xff, 0xc0, 0x00, 0x0b, 0x08, 0x00, 0x02,
    0x00, 0x02, 0x01, 0x01, 0x11, 0x00, 0xff, 0xc4, 0x00, 0x1f, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0xff, 0xc4, 0x00, 0xb5, 0x10, 0x00, 0x02, 0x01, 0x03,
    0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00,
    0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
    0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2,
    0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9,
    0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6,
    0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xff, 0xda,
    0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3f, 0x00, 0x2b, 0xff, 0xd9,
];

/// Where the fixture serves the one picture every species is given.
const FIXTURE_PICTURE_PATH: &str = "/__fixture/picture.jpg";

/// A species-picture source with no network: every species the fixture knows has a credited
/// picture and a one-line description, delivered from this server itself (`FIXTURE_PICTURE_PATH`),
/// through the same provider, download, disk and credit path the shipped binary takes.
///
/// The fixture used to call Wikipedia, like the binary. Then a page's pictures were whatever the
/// network answered: a run on a machine behind a proxy or a rate limit rendered initials where a
/// run in CI rendered photographs, so the strict visual sweep failed on one and passed on the
/// other with the same code, and a change to a Wikipedia article could change CI's layout. A
/// source that answers the same everywhere makes the gates grade the same pages everywhere.
///
/// `Zenaida macroura` is the deliberate exception: its picture is seeded on disk without a credit
/// (the state of a picture cached before credits were kept), and this answers it with a network
/// failure, which keeps that picture and keeps the state in the sweep. Any other name, such as
/// the sixty species of the `stress` profile, is "no picture", so the placeholder path is graded.
struct FixturePictures {
    /// The address this fixture listens on, for the picture's URL.
    addr: String,
    /// `(scientific name, common name)` of every species the fixture seeds.
    known: Vec<(&'static str, &'static str)>,
}

impl birdnet_integrations::species_images::ImageProvider for FixturePictures {
    fn fetch<'life0, 'life1, 'async_trait>(
        &'life0 self,
        scientific_name: &'life1 str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        birdnet_integrations::species_images::SpeciesImage,
                        birdnet_integrations::species_images::ImageError,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        use birdnet_integrations::species_images::{ImageCredit, ImageError, SpeciesImage};
        let name = scientific_name.to_owned();
        let found = self
            .known
            .iter()
            .find(|(sci, _)| sci.eq_ignore_ascii_case(scientific_name))
            .copied();
        let url = format!("http://{}{FIXTURE_PICTURE_PATH}", self.addr);
        Box::pin(async move {
            let Some((sci, com)) = found else {
                return Err(ImageError::NotFound(name));
            };
            if sci == "Zenaida macroura" {
                return Err(ImageError::Http(
                    "fixture: this one is always offline".into(),
                ));
            }
            Ok(SpeciesImage {
                url,
                cached_path: None,
                width: birdnet_integrations::species_images::wikipedia::DEFAULT_THUMB_WIDTH,
                description: Some(format!(
                    "The {com} ({sci}) is a bird of the fixture station, here so that a species \
                     page has a line of prose from its source to set beside the picture."
                )),
                wiki_url: Some(format!(
                    "https://en.wikipedia.org/wiki/{}",
                    sci.replace(' ', "_")
                )),
                credit: Some(ImageCredit {
                    title: format!("{com}, a fixture picture"),
                    author: "Fixture Photographer".to_owned(),
                    source: "Wikimedia Commons".to_owned(),
                    source_url: "https://commons.wikimedia.org/wiki/File:Fixture.jpg".to_owned(),
                    licence: "CC BY-SA 4.0".to_owned(),
                    licence_url: "https://creativecommons.org/licenses/by-sa/4.0/".to_owned(),
                }),
            })
        })
    }
}

/// Fills the fixture's picture cache with what the Picture credits page lists: three credited
/// photographs (one with a long author's name, one with none), one picture cached before credits
/// were kept (no `.credit.json`, counted but not listed), and one credited public-domain plate.
/// Files that are already there are left alone, so a restart does not rewrite them.
fn seed_credited_pictures(images: &std::path::Path) {
    use birdnet_integrations::species_images::ImageCredit;
    let credited = |title: &str, author: &str, licence: &str, licence_url: &str| ImageCredit {
        title: title.to_owned(),
        author: author.to_owned(),
        source: "Wikimedia Commons".to_owned(),
        source_url: "https://commons.wikimedia.org/wiki/File:Fixture.jpg".to_owned(),
        licence: licence.to_owned(),
        licence_url: licence_url.to_owned(),
    };
    let photos = [
        (
            "cardinalis_cardinalis",
            Some(credited(
                "A cardinal at a feeder",
                "A. Fixture",
                "CC BY-SA 4.0",
                "https://creativecommons.org/licenses/by-sa/4.0/",
            )),
        ),
        (
            "cyanocitta_cristata",
            Some(credited(
                "Blue Jay, Cyanocitta cristata, on a fence post in the early morning light",
                "Alexandra Maria-Louise Featherstone-Haugh-Montgomery",
                "CC BY 2.0",
                "https://creativecommons.org/licenses/by/2.0/",
            )),
        ),
        (
            "poecile_atricapillus",
            Some(credited("Chickadee", "", "CC0", "")),
        ),
        ("zenaida_macroura", None),
    ];
    let plate = (
        "turdus_migratorius",
        credited(
            "Turdus migratorius, plate from The Birds of Europe (1832–1837)",
            "John Gould",
            "Public domain",
            "",
        ),
    );
    let write = |dir: &std::path::Path, key: &str, credit: Option<&ImageCredit>| {
        let _ = std::fs::create_dir_all(dir);
        let picture = dir.join(format!("{key}.jpg"));
        if !picture.exists() {
            let _ = std::fs::write(&picture, TINY_JPEG);
        }
        let sidecar = dir.join(format!("{key}.credit.json"));
        if let (Some(credit), false) = (credit, sidecar.exists())
            && let Ok(json) = serde_json::to_vec(credit)
        {
            let _ = std::fs::write(&sidecar, json);
        }
    };
    for (key, credit) in &photos {
        write(images, key, credit.as_ref());
    }
    write(&images.join("plates"), plate.0, Some(&plate.1));
}

#[tokio::main]
async fn main() {
    // Where the fixture keeps its files, and where it listens. Both default to
    // what they always were; set them to run a second instance beside the
    // first — the base-path gate serves one under a prefix (`BIRDNET_BASE_PATH`).
    // The default is deliberately deep. The Data tab prints the database's full
    // path, and a path as long as a macOS home folder's pushed that page 84 px past
    // a phone's edge while the fixture, which lived directly in `/tmp`, never
    // showed it. A fixture whose paths are short cannot grade a page that prints
    // them.
    let dir = std::env::var_os("BNB_FIXTURE_DIR").map_or_else(
        || {
            std::env::temp_dir().join(
                "birdnet-behavior-fixture-whose-data-directory-path-is-as-long-as-a-macos-home-one",
            )
        },
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let addr = std::env::var("BNB_FIXTURE_ADDR").unwrap_or_else(|_| "127.0.0.1:8502".to_owned());
    let path = dir.join("bnb_screenshots.db");
    let _ = std::fs::remove_file(&path);

    let conn = Connection::open(&path).expect("open sqlite");
    birdnet_db::migration::migrate(&conn).expect("migrate");

    let now = fixture_now_secs();
    let today_days = (now / 86_400) as i64;

    eprintln!("seeding synthetic dataset…");
    // The charts are graded on the awkward stations too, not only the lived-in one:
    // `BNB_FIXTURE_PROFILE` = `empty` (nothing heard), `one` (one detection), `stress`
    // (sixty species with very long names, a year of them, and an hour with 1.6 million
    // detections in it). Anything else is the usual fixture.
    match std::env::var("BNB_FIXTURE_PROFILE")
        .unwrap_or_default()
        .as_str()
    {
        "empty" => seed_profile_empty(&conn),
        "one" => seed_profile_one(&conn, today_days),
        "stress" => seed_profile_stress(&conn, today_days),
        _ => seed(&conn, today_days),
    }
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
    // Every other row names a clip this fixture never writes. A real station
    // has those files until retention reclaims them, and then marks the row
    // `Clip_Pruned_At` — which every play surface reads through
    // `CLIP_AVAILABLE`. Without the mark, a species page picked a clip with no
    // file as its best detection and the sweep graded a broken player that no
    // real station shows.
    {
        // Numbered explicitly: a bare `?` takes the next index after the
        // highest one written before it, so `?21` ahead of a list of `?`
        // numbered that list 22 to 41.
        let keep = (2..=demo_clips.len() + 1)
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE detections SET Clip_Pruned_At = ?1 \
             WHERE File_Name IS NOT NULL AND File_Name NOT IN ({keep})"
        );
        let reclaimed_at = now as i64;
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&reclaimed_at];
        params.extend(demo_clips.iter().map(|c| c as &dyn rusqlite::ToSql));
        let pruned = conn
            .execute(&sql, params.as_slice())
            .expect("mark unwritten clips as reclaimed");
        eprintln!("marked {pruned} clips the fixture does not write as reclaimed");
    }

    // Run exactly like the shipped binary: analytics on *and active*, not
    // merely compiled in. Reopen the seeded SQLite through the analytics-aware
    // constructor so the DuckDB analytics database is opened and the detections
    // synced — the timeseries (window-function) screens then render live,
    // mirroring the real, zero-config user experience. Slim
    // `--no-default-features` builds fall back to the SQLite-only state.
    #[cfg(feature = "analytics")]
    let state = {
        drop(conn);
        let analytics_path = dir.join("bnb_screenshots.duckdb");
        let _ = std::fs::remove_file(&analytics_path);
        AppState::new_with_analytics(path, &analytics_path)
            .expect("open analytics database")
            .with_site_name("BirdNet-Behavior".to_string())
    };
    #[cfg(not(feature = "analytics"))]
    let state =
        AppState::from_connection(conn, path).with_site_name("BirdNet-Behavior".to_string());

    // Wire a species image cache, as the shipped binary does, so the gallery and species photos
    // actually populate (without it the `/species/image/<sci>/file` endpoint returns "image
    // cache not configured" and the gallery only ever shows the coloured code placeholders). Its
    // source is the fixture's own (`FixturePictures`), never the network; fetches are cached under
    // a temp dir; if construction fails the server still runs, just with placeholder thumbnails.
    seed_credited_pictures(&dir.join("bnb_screenshots_images"));
    let pictures = FixturePictures {
        addr: addr.clone(),
        known: species_table().iter().map(|s| (s.sci, s.com)).collect(),
    };
    let state = match birdnet_integrations::species_images::ImageCache::new(
        &dir.join("bnb_screenshots_images"),
        std::sync::Arc::new(pictures),
        birdnet_integrations::species_images::wikipedia::DEFAULT_THUMB_WIDTH,
    ) {
        Ok(cache) => state.with_image_cache(cache),
        Err(e) => {
            eprintln!("image cache unavailable ({e}); gallery will show code placeholders");
            state
        }
    };
    // The public-domain plate cache, as the shipped binary builds it, so the kiosk and the credits
    // page have a plates section to show.
    let state = match birdnet_integrations::species_images::ImageCache::new(
        &dir.join("bnb_screenshots_images").join("plates"),
        std::sync::Arc::new(birdnet_integrations::species_images::PlateProvider::embedded()),
        birdnet_integrations::species_images::PLATE_WIDTH,
    ) {
        Ok(cache) => state.with_plate_cache(cache),
        Err(e) => {
            eprintln!("plate cache unavailable ({e})");
            state
        }
    };

    // Seed live capture-supervisor health so Station Health shows the
    // operator-grade per-source cards (this server runs no real supervisor).
    // Labels match the seeded detection sources so "detections today" merges.
    let state = state.with_capture_status(seed_capture_status());

    // The range filter, so the "Why isn't a bird heard here?" page has data to
    // show. The fixture runs no detection daemon and loads no range model; this
    // answers from a table. The numbers are made up, the shape is the real one:
    // every cell of (over or under the threshold) x (admitted or not), a long
    // admitted list to page through, and a name the lookup can be asked about.
    state.set_occurrence_source(std::sync::Arc::new(
        birdnet_web::occurrence::FixedOccurrence::answering(seed_occurrence()),
    ));

    // Opt-in (`BNB_FIXTURE_PARITY=1`): a running tracker with learned levels,
    // per-species intervals and a TLS setting to draw, for the pictures of the
    // Learned thresholds, Minimum interval and Secure connection cards. Off by
    // default so the shared fixture's other pages are unchanged.
    if std::env::var("BNB_FIXTURE_PARITY").is_ok() {
        use birdnet_core::detection::dynamic_threshold::{
            DynamicThresholdConfig, DynamicThresholdHandle,
        };
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis()),
        )
        .unwrap_or(0);
        let hour = 3_600_000_i64;
        let learned = |name: &str, level: u8, conf: u32, expires: i64, first: i64, last: i64| {
            birdnet_db::dynamic_thresholds::LearnedThreshold {
                sci_name: name.to_owned(),
                level,
                confirmations: conf,
                expires_at_ms: now + expires * hour,
                first_learned_ms: now - first * hour,
                last_confirmed_ms: now - last * hour,
            }
        };
        let _ = state.with_db(|c| {
            birdnet_db::dynamic_thresholds::replace_all(
                c,
                &[
                    learned("Strix aluco", 3, 9, 14, 30, 2),
                    learned("Turdus merula", 2, 5, 20, 10, 4),
                    learned("Erithacus rubecula", 1, 2, 2, 20, 22),
                    learned("Parus major", 1, 1, -3, 30, 27),
                ],
            )?;
            for (sci, secs) in [
                ("Strix aluco", 60),
                ("Turdus merula", 5),
                ("Pica pica", 3600),
            ] {
                birdnet_db::sqlite::set_species_interval(c, sci, secs)?;
            }
            Ok::<(), birdnet_db::sqlite::DbError>(())
        });
        state.set_dynamic_threshold_handle(std::sync::Arc::new(DynamicThresholdHandle::new(
            DynamicThresholdConfig {
                enabled: true,
                trigger: 0.8,
                min: 0.25,
                valid_hours: 24,
            },
        )));
        let mode = std::env::var("BNB_FIXTURE_TLS").unwrap_or_else(|_| "self-signed".into());
        let dir = std::env::temp_dir().join(format!("bnb-fixture-tls-{}", std::process::id()));
        let mode = match mode.as_str() {
            "off" => birdnet_web::tls::TlsMode::Off,
            "manual" => birdnet_web::tls::TlsMode::Manual,
            _ => {
                let _ = birdnet_web::tls::ensure_self_signed(&dir, &["localhost".to_owned()], 397);
                birdnet_web::tls::TlsMode::SelfSigned
            }
        };
        state.set_https_status(birdnet_web::routes::admin::https::HttpsStatus {
            mode,
            https_addr: mode.enabled().then(|| "0.0.0.0:8503".to_owned()),
            state_dir: dir,
            waiting_for_clock: false,
        });
    }

    // The per-source `audio_source_up` gauge is what the dashboard pill, the
    // header badge and the Today rail grade the station on, and the shipped
    // binary publishes it from the supervisor. Without it the fixture read as
    // "Not recording" on every page, and the documentation screenshots taken
    // from it showed a warning pill over a healthy-looking feed. The values
    // match the three seeded cards: the local mic is up, both cameras are not.
    for (source, up) in [("local", true), ("RTSP_1", false), ("RTSP_2", false)] {
        state.metrics().set_source_up(source, up);
    }
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);

    // Drop synthetic bird-call WAVs at the newest clips' paths so the
    // Recordings grid shows real spectrogram thumbnails (the route renders from
    // these and caches the PNGs under the data dir's `spectrograms/`).
    seed_demo_audio(&state.recording_dir(), &demo_clips);

    // One snapshot in the backup ring, so /station/data renders a snapshot
    // row. The seeded database had none, and the row's grid overflowed the
    // 390 px viewport only once one existed (DD-30) — a state the overflow
    // gate could not see until the fixture had it.
    seed_backup_snapshot(state.db_path());

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
            trusted_proxies: birdnet_web::client_ip::TrustedProxies::default(),
        },
    );

    // The one picture every species is given (see `FixturePictures`), served by this server itself.
    let app = app.route(
        FIXTURE_PICTURE_PATH,
        axum::routing::get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "image/jpeg")],
                TINY_JPEG,
            )
        }),
    );

    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    eprintln!("screenshot server listening on http://{addr}/");
    axum::serve(listener, app).await.expect("serve");
}
