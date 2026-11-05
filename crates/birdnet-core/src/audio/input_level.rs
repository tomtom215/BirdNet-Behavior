//! Reading a microphone's input level from the operating system, read-only,
//! and what to make of it.
//!
//! The station records 16-bit integers, so a level set too high clips for good
//! and no later setting brings the lost samples back. This module turns what
//! `amixer` (ALSA) and `pactl` (PulseAudio, and PipeWire through
//! `pipewire-pulse`) print into one description, and grades it against a cap.
//! It runs nothing and changes nothing: the caller runs the tools, under a
//! deadline, and passes their output here. See
//! `docs/book/field/input-level.md` for what the numbers mean and where each
//! claim comes from.
//!
//! # What is and is not known
//!
//! * `amixer` prints a control's position as a percentage of its **raw range**
//!   by default, not of a gain scale, so 100% is the control's maximum, and the
//!   dB beside it is what that maximum is worth for the device in question
//!   (+23.81 dB on a USB codec whose figure was published, +30.00 dB and an
//!   extra boost on a Raspberry Pi HAT codec's kernel table). VERIFIED in
//!   alsa-utils' manual page and from published output; not measured here, as
//!   this was written on a machine with no sound card.
//! * PulseAudio and PipeWire percentages are cubic (50% is -18.06 dB), and 100%
//!   is 0 dB **of the source's own scale**: unity for a virtual or software
//!   source, the hardware maximum for one with a hardware volume control.
//!   VERIFIED for the percent/dB relation on real servers; the hardware
//!   meaning is read from the PulseAudio and PipeWire sources and was not
//!   observed on hardware.
//! * The cap below ([`CAP_PERCENT`], [`CAP_HARDWARE_DB`]) is a guard rail from
//!   a design judgement, not a published recommendation, and not an acoustic
//!   optimum. The measured signal (peak below full scale, no clipped samples)
//!   is what decides a level; see the manual page.

use serde_json::Value;

/// The highest position, as a percentage, the guide recommends for a control
/// that sits in front of the hardware gain. A guard rail, not an optimum.
pub const CAP_PERCENT: u32 = 75;

/// The highest hardware gain, in dB, the guide recommends.
///
/// The same judgement as [`CAP_PERCENT`]: 75% of the range is about +18 dB on the
/// one USB codec and the one HAT codec whose tables are public, which is where
/// the two figures were chosen to agree.
pub const CAP_HARDWARE_DB: f32 = 18.0;

/// Which sound system a reading came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// ALSA, read with `amixer`.
    Alsa,
    /// PulseAudio, or PipeWire through `pipewire-pulse`, read with `pactl`.
    Pulse,
}

/// One capture-volume control, as the operating system reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct InputLevel {
    /// The sound system.
    pub backend: Backend,
    /// ALSA: the simple control's name (`Mic`, `Capture`). Pulse: the source's name.
    pub control: String,
    /// The position the tool prints. ALSA: percent of the control's raw range.
    /// Pulse: percent of the source's 100% (cubic), so it can exceed 100.
    pub percent: u32,
    /// The gain the tool prints, in dB, where it knows one.
    pub db: Option<f32>,
    /// Pulse only: the gain on top of the hardware's own 0 dB point, that is the
    /// reported gain minus the source's base volume, where both are known.
    pub hardware_db: Option<f32>,
    /// Pulse only: the source's base volume in dB.
    pub base_db: Option<f32>,
    /// Pulse only: whether the source has a hardware volume control.
    pub hardware_controlled: bool,
    /// ALSA only: the control's raw value, minimum and maximum.
    pub raw: Option<(i64, i64, i64)>,
    /// `Some(true)` when capture is switched off, `None` when there is no switch to read.
    pub muted: Option<bool>,
}

/// The verdict on a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    /// Within the cap.
    Ok,
    /// Capture is switched off.
    Muted,
    /// Above the cap, short of the maximum.
    AboveCap,
    /// At the control's maximum.
    AtMaximum,
}

/// Why a tool's output could not be read as a level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The output is not the JSON `pactl --format=json list sources` prints.
    NotJson,
    /// The listing has no source of that name.
    NoSuchSource(String),
    /// The source is listed but carries no volume.
    NoVolume(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson => write!(
                f,
                "pactl did not print JSON (is it older than PulseAudio 16?)"
            ),
            Self::NoSuchSource(name) => write!(f, "no source named {name} in the listing"),
            Self::NoVolume(name) => write!(f, "source {name} reports no volume"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Grade a reading against the cap.
#[must_use]
pub fn assess(level: &InputLevel) -> Advice {
    if level.muted == Some(true) {
        return Advice::Muted;
    }
    match level.backend {
        Backend::Alsa => {
            let at_maximum = level.percent >= 100 && level.raw.is_some_and(|(_, lo, hi)| hi > lo);
            if at_maximum {
                Advice::AtMaximum
            } else if level.percent > CAP_PERCENT || level.db.is_some_and(|d| d > CAP_HARDWARE_DB) {
                Advice::AboveCap
            } else {
                Advice::Ok
            }
        }
        Backend::Pulse if level.hardware_controlled => {
            if level.percent >= 100 {
                Advice::AtMaximum
            } else if level.percent > CAP_PERCENT
                || level.hardware_db.is_some_and(|d| d > CAP_HARDWARE_DB)
            {
                Advice::AboveCap
            } else {
                Advice::Ok
            }
        }
        // A software volume: 100% is unity, and anything above it amplifies the
        // digital signal, which is how a 16-bit capture is pushed into clipping.
        Backend::Pulse => {
            if level.percent > 100 {
                Advice::AboveCap
            } else {
                Advice::Ok
            }
        }
    }
}

/// A command the operator can run to bring the level to the cap, or `None`
/// when no safe one can be written for this reading.
///
/// The station never runs it. A mixer setting changed at run time is lost on a
/// power cut or a replug unless the operator also stores it, which takes root
/// and so cannot be done from the service; the page says so beside the command.
#[must_use]
pub fn command_to_reach_cap(level: &InputLevel, card: &str) -> Option<String> {
    let safe = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '+' | '-'))
    };
    match level.backend {
        Backend::Alsa => {
            let (value, lo, hi) = level.raw?;
            if hi <= lo || level.control.contains('\'') || !safe(card) {
                return None;
            }
            let target = lo + ((hi - lo) * i64::from(CAP_PERCENT) + 50) / 100;
            (target < value).then(|| {
                format!(
                    "amixer -c {card} -- sset '{}' capture {target}",
                    level.control
                )
            })
        }
        Backend::Pulse => {
            if !level.hardware_controlled || !safe(&level.control) {
                return None;
            }
            let mut target = CAP_PERCENT;
            if let Some(base) = level.base_db {
                // The percent whose gain is CAP_HARDWARE_DB above the hardware's
                // own 0 dB: db = 60 * log10(p), so p = 10^(db / 60).
                let allowed = 100.0 * 10_f32.powf((CAP_HARDWARE_DB + base) / 60.0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let allowed = allowed.clamp(1.0, 100.0).floor() as u32;
                target = target.min(allowed);
            }
            (target < level.percent)
                .then(|| format!("pactl set-source-volume {} {target}%", level.control))
        }
    }
}

/// One simple mixer control while `amixer`'s listing is being read.
struct Block {
    name: String,
    capture_volume: bool,
    range: Option<(i64, i64)>,
    channels: Vec<CaptureEntry>,
}

/// `Capture 8 [50%] [11.90dB] [on]` on one channel's line.
struct CaptureEntry {
    raw: i64,
    percent: u32,
    db: Option<f32>,
    on: Option<bool>,
}

impl Block {
    /// The control as a reading, when it is a capture-volume control with at
    /// least one channel read. Its position is the highest of its channels; it
    /// is muted when it has a capture switch and every channel's is off.
    fn into_level(self) -> Option<InputLevel> {
        if !self.capture_volume {
            return None;
        }
        let best = self.channels.iter().max_by_key(|c| c.percent)?;
        let switches: Vec<bool> = self.channels.iter().filter_map(|c| c.on).collect();
        Some(InputLevel {
            backend: Backend::Alsa,
            percent: best.percent,
            db: best.db,
            hardware_db: None,
            base_db: None,
            hardware_controlled: true,
            raw: self.range.map(|(lo, hi)| (best.raw, lo, hi)),
            muted: (!switches.is_empty()).then(|| switches.iter().all(|on| !on)),
            control: self.name,
        })
    }
}

/// Every capture-volume control in `amixer -c <card> scontents`.
///
/// A control counts when its capabilities include `cvolume` (or the joined
/// form).
#[must_use]
pub fn parse_amixer_scontents(text: &str) -> Vec<InputLevel> {
    let mut out = Vec::new();
    let mut block: Option<Block> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Simple mixer control '") {
            out.extend(block.take().and_then(Block::into_level));
            // `'Mic',0`: the name runs to the last `',`, so it may hold a quote.
            let name = rest.rsplit_once("',").map_or(rest, |(n, _)| n);
            block = Some(Block {
                name: name.to_owned(),
                capture_volume: false,
                range: None,
                channels: Vec::new(),
            });
        } else if let Some(b) = block.as_mut() {
            if let Some(caps) = trimmed.strip_prefix("Capabilities:") {
                b.capture_volume = caps
                    .split_whitespace()
                    .any(|c| c == "cvolume" || c == "cvolume-joined");
            } else if let Some(limits) = trimmed.strip_prefix("Limits:") {
                b.range = capture_range(limits);
            } else if let Some(entry) = capture_entry(trimmed) {
                b.channels.push(entry);
            }
        }
    }
    out.extend(block.and_then(Block::into_level));
    out
}

/// `Playback 0 - 127 Capture 0 - 16` -> `(0, 16)`.
fn capture_range(limits: &str) -> Option<(i64, i64)> {
    let tokens: Vec<&str> = limits.split_whitespace().collect();
    let at = tokens.iter().position(|t| *t == "Capture")?;
    let (lo, dash, hi) = (
        tokens.get(at + 1)?,
        tokens.get(at + 2)?,
        tokens.get(at + 3)?,
    );
    (*dash == "-").then_some(())?;
    Some((lo.parse().ok()?, hi.parse().ok()?))
}

/// `Front Left: Capture 8 [50%] [11.90dB] [on]`, or the same after a
/// `Playback ...` part on one line. It needs a `[nn%]`, which the `Limits:`
/// line's `Capture 0 - 16` does not have.
fn capture_entry(line: &str) -> Option<CaptureEntry> {
    let mut tokens = line.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if token != "Capture" {
            continue;
        }
        let Some(raw) = tokens.peek().and_then(|n| n.parse::<i64>().ok()) else {
            continue;
        };
        tokens.next();
        let (mut percent, mut db, mut on) = (None, None, None);
        while let Some(next) = tokens.peek() {
            let Some(inner) = next.strip_prefix('[').and_then(|n| n.strip_suffix(']')) else {
                break;
            };
            if let Some(p) = inner.strip_suffix('%') {
                percent = p.parse::<u32>().ok();
            } else if let Some(d) = inner.strip_suffix("dB") {
                // `-99999.99dB` is ALSA's "minus infinity".
                db = d
                    .parse::<f32>()
                    .ok()
                    .filter(|d| d.is_finite() && *d > -9_999.0);
            } else if inner == "on" || inner == "off" {
                on = Some(inner == "on");
            }
            tokens.next();
        }
        if let Some(percent) = percent {
            return Some(CaptureEntry {
                raw,
                percent,
                db,
                on,
            });
        }
    }
    None
}

/// The source name after `Default Source:` in `pactl info`.
#[must_use]
pub fn default_source(pactl_info: &str) -> Option<String> {
    pactl_info.lines().find_map(|l| {
        l.strip_prefix("Default Source:")
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
    })
}

/// The level of the source called `name` in the output of
/// `pactl --format=json list sources`.
///
/// # Errors
///
/// [`ReadError`] when the output is not that JSON, has no such source, or the
/// source carries no volume.
pub fn parse_pactl_source(json: &str, name: &str) -> Result<InputLevel, ReadError> {
    let Ok(Value::Array(sources)) = serde_json::from_str::<Value>(json) else {
        return Err(ReadError::NotJson);
    };
    let source = sources
        .iter()
        .find(|s| s.get("name").and_then(Value::as_str) == Some(name))
        .ok_or_else(|| ReadError::NoSuchSource(name.to_owned()))?;

    let channels = source
        .get("volume")
        .and_then(Value::as_object)
        .ok_or_else(|| ReadError::NoVolume(name.to_owned()))?;
    let loudest = channels
        .values()
        .filter_map(|c| Some((percent_of(c)?, db_of(c))))
        .max_by_key(|(p, _)| *p)
        .ok_or_else(|| ReadError::NoVolume(name.to_owned()))?;
    let (percent, db) = loudest;
    let base_db = source.get("base_volume").and_then(db_of);
    let hardware_controlled = source
        .get("flags")
        .and_then(Value::as_array)
        .is_some_and(|f| f.iter().any(|f| f.as_str() == Some("HW_VOLUME_CTRL")));
    Ok(InputLevel {
        backend: Backend::Pulse,
        control: name.to_owned(),
        percent,
        db,
        hardware_db: db.zip(base_db).map(|(d, b)| d - b),
        base_db,
        hardware_controlled,
        raw: None,
        muted: source.get("mute").and_then(Value::as_bool),
    })
}

/// `{"value": 65536, "value_percent": "100%", "db": "0.00 dB"}` -> 100.
fn percent_of(channel: &Value) -> Option<u32> {
    let text = channel.get("value_percent")?.as_str()?;
    text.trim().trim_end_matches('%').trim().parse().ok()
}

/// `"-18.06 dB"` -> -18.06; `"-inf dB"` -> `None`.
fn db_of(channel: &Value) -> Option<f32> {
    let text = channel.get("db")?.as_str()?;
    text.trim()
        .trim_end_matches("dB")
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|d| d.is_finite())
}
