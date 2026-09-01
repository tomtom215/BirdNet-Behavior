//! Audio sources — first-class CRUD (O-13).
//!
//! Replaces the single-string `state.audio_source()` model with a real
//! entity that the admin UI manipulates through CRUD endpoints. Each row
//! is one microphone or one RTSP stream.
//!
//! The capture pipeline reads from this table when non-empty
//! (`src/capture.rs::resolve_sources_from_db`), with the legacy CLI/config
//! path as a fallback for stations that haven't migrated yet. The
//! remaining O-13 follow-up is on `probe(id)` (still synthetic) — see
//! `birdnet-web/src/routes/admin/audio.rs` for the per-source daemon-
//! metrics handle that the real `probe(id)` will read.
//!
//! Synchronous SQLite-backed stores per the project rule. Hand-rolled
//! error types; no `anyhow`/`thiserror`. The trait `AudioSourceStore` is
//! implemented for `rusqlite::Connection` so the web server bridges it
//! through `AppState::with_db`.

use std::fmt;

use rusqlite::{Connection, OptionalExtension, params};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from the audio-sources store.
#[derive(Debug)]
pub enum AudioSourceError {
    /// `SQLite` error (constraint, IO, locking).
    Sqlite(rusqlite::Error),
    /// Lookup returned no row.
    NotFound(String),
    /// Constraint violation (duplicate id).
    Conflict(String),
    /// Invalid input (unknown kind, channels, transport, malformed time).
    Invalid(String),
}

impl fmt::Display for AudioSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::NotFound(msg) => write!(f, "not found: {msg}"),
            Self::Conflict(msg) => write!(f, "conflict: {msg}"),
            Self::Invalid(msg) => write!(f, "invalid: {msg}"),
        }
    }
}

impl std::error::Error for AudioSourceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for AudioSourceError {
    fn from(e: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(ref code, _) = e
            && code.code == rusqlite::ErrorCode::ConstraintViolation
        {
            return Self::Conflict("constraint violation".to_string());
        }
        Self::Sqlite(e)
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// What kind of input a row represents. SQL-on-disk: 'usb-alsa', 'pipewire',
/// 'rtsp'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// USB microphone captured via the ALSA `plughw:` / `hw:` device path.
    UsbAlsa,
    /// Microphone exposed through PipeWire (Raspberry Pi OS Bookworm default).
    PipeWire,
    /// Remote audio stream delivered over RTSP.
    Rtsp,
}

impl SourceKind {
    /// Returns the canonical SQL-on-disk string for this variant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UsbAlsa => "usb-alsa",
            Self::PipeWire => "pipewire",
            Self::Rtsp => "rtsp",
        }
    }
}

impl std::str::FromStr for SourceKind {
    type Err = AudioSourceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "usb-alsa" => Ok(Self::UsbAlsa),
            "pipewire" => Ok(Self::PipeWire),
            "rtsp" => Ok(Self::Rtsp),
            other => Err(AudioSourceError::Invalid(format!("unknown kind: {other}"))),
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Channel layout. 'mono' / 'left' / 'right' / 'stereo'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    /// Single-channel capture (downmix or hardware mono).
    Mono,
    /// Use only the left channel of a stereo input.
    Left,
    /// Use only the right channel of a stereo input.
    Right,
    /// Preserve both channels; inference runs on the stereo mix.
    Stereo,
}

impl Channels {
    /// Returns the canonical SQL-on-disk string for this variant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Left => "left",
            Self::Right => "right",
            Self::Stereo => "stereo",
        }
    }
}

impl std::str::FromStr for Channels {
    type Err = AudioSourceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mono" => Ok(Self::Mono),
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "stereo" => Ok(Self::Stereo),
            other => Err(AudioSourceError::Invalid(format!(
                "unknown channels: {other}"
            ))),
        }
    }
}

impl fmt::Display for Channels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RTSP transport preference. 'auto' / 'tcp' / 'udp'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtspTransport {
    /// Let the capture daemon negotiate the best available transport.
    Auto,
    /// Force RTSP over TCP (reliable, higher latency).
    Tcp,
    /// Force RTSP over UDP (lower latency, packet-loss risk on Wi-Fi).
    Udp,
}

impl RtspTransport {
    /// Returns the canonical SQL-on-disk string for this variant.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

impl std::str::FromStr for RtspTransport {
    type Err = AudioSourceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(Self::Auto),
            "tcp" => Ok(Self::Tcp),
            "udp" => Ok(Self::Udp),
            other => Err(AudioSourceError::Invalid(format!(
                "unknown rtsp transport: {other}"
            ))),
        }
    }
}

impl fmt::Display for RtspTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Audio-pipeline toggles, as stored.
///
/// Four independent per-source flags. Each is intrinsically boolean (a filter
/// is either applied or not), so the pedantic "more than 3 bools" suggestion to
/// refactor into enums is not useful here.
///
/// This is the **storage** shape. `birdnet-core` must not depend on this crate,
/// so the capture path consumes `birdnet_core::audio::capture::AudioPipeline`
/// and `birdnet-behavior`'s `capture::sources::map_pipeline` is the one seam
/// that converts between them. Consult that function before assuming a field
/// here means what its name suggests — `rtsp_keepalive` in particular does not:
/// ffmpeg sends RTSP keepalives on its own and offers no switch for them, so it
/// maps to `AudioPipeline::rtsp_stall_timeout`, which bounds socket reads so a
/// stalled stream is noticed and restarted.
///
/// This comment previously claimed the daemon "honours" all four. It did not
/// read any of them: they were written, round-tripped, and dropped at the
/// resolver. The claim is worth remembering as a caution about this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct PipelineFlags {
    /// Apply a high-pass filter to reduce low-frequency rumble before inference. Default `true`.
    pub high_pass: bool,
    /// Remove DC offset from the captured signal. Default `true`.
    pub dc_removal: bool,
    /// Enable automatic gain control to normalise recording levels. Default `false`.
    pub agc: bool,
    /// Send periodic RTSP OPTIONS keepalive requests to prevent stream timeout. Default `true`.
    pub rtsp_keepalive: bool,
}

impl Default for PipelineFlags {
    fn default() -> Self {
        Self {
            high_pass: true,
            dc_removal: true,
            agc: false,
            rtsp_keepalive: true,
        }
    }
}

/// One row from the `audio_sources` table.
#[derive(Debug, Clone)]
pub struct AudioSource {
    /// Stable identifier for this source (alphanumeric, `-`, `_`, `.`; max 64 chars).
    pub id: String,
    /// Physical or protocol kind of this source.
    pub kind: SourceKind,
    /// Device path or URL that identifies the hardware or stream (e.g. `plughw:1,0`
    /// or `rtsp://camera.local/audio`).
    pub device_id: String,
    /// Optional human-readable label shown in the admin UI (e.g. `Backyard feeder`).
    pub label: Option<String>,
    /// Capture sample rate in Hz (e.g. `48000`).
    pub sample_rate: u32,
    /// Channel layout to use from the captured signal.
    pub channels: Channels,
    /// Sample bit depth (e.g. `16`, `24`).
    pub bit_depth: u8,
    /// Pre-amplification gain in dB applied before inference. `0.0` means unity gain.
    pub gain_db: f32,
    /// Preferred RTSP transport; ignored for non-RTSP sources.
    pub rtsp_transport: RtspTransport,
    /// `(start, end)` quiet schedule in HH:MM form, when set.
    pub schedule_quiet: Option<(String, String)>,
    /// Audio pipeline feature flags for this source.
    pub pipeline: PipelineFlags,
    /// ISO-8601 timestamp at which this source was soft-deleted, if any.
    pub disabled_at: Option<String>,
    /// ISO-8601 timestamp when this row was first inserted.
    pub created_at: String,
    /// ISO-8601 timestamp of the most recent update to this row.
    pub updated_at: String,
}

/// Input for [`AudioSourceStore::insert`].
#[derive(Debug, Clone)]
pub struct NewAudioSource {
    /// Stable identifier chosen by the caller; see [`AudioSource::id`] for constraints.
    pub id: String,
    /// Physical or protocol kind of this source.
    pub kind: SourceKind,
    /// Device path or URL; see [`AudioSource::device_id`].
    pub device_id: String,
    /// Optional human-readable label for the admin UI.
    pub label: Option<String>,
    /// Capture sample rate in Hz.
    pub sample_rate: u32,
    /// Channel layout.
    pub channels: Channels,
    /// Sample bit depth.
    pub bit_depth: u8,
    /// Pre-amplification gain in dB (`0.0` = unity).
    pub gain_db: f32,
    /// Preferred RTSP transport; ignored for non-RTSP sources.
    pub rtsp_transport: RtspTransport,
    /// Optional `(start, end)` quiet-hours window in `HH:MM` form.
    pub schedule_quiet: Option<(String, String)>,
    /// Audio pipeline feature flags.
    pub pipeline: PipelineFlags,
}

impl NewAudioSource {
    /// Minimal constructor used by the admin form — every operator-facing
    /// field is documented as optional on the form and falls back to a
    /// sensible default.
    #[must_use]
    pub fn defaults(id: impl Into<String>, kind: SourceKind, device_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            device_id: device_id.into(),
            label: None,
            sample_rate: 48_000,
            channels: Channels::Mono,
            bit_depth: 24,
            gain_db: 0.0,
            rtsp_transport: RtspTransport::Auto,
            schedule_quiet: None,
            pipeline: PipelineFlags::default(),
        }
    }
}

/// Optional updates for [`AudioSourceStore::update`]. `None` leaves the
/// column unchanged.
#[derive(Debug, Clone, Default)]
pub struct AudioSourcePatch {
    /// New label; `Some(None)` clears the existing label.
    pub label: Option<Option<String>>,
    /// Replacement device path or URL.
    pub device_id: Option<String>,
    /// Replacement sample rate in Hz.
    pub sample_rate: Option<u32>,
    /// Replacement channel layout.
    pub channels: Option<Channels>,
    /// Replacement sample bit depth.
    pub bit_depth: Option<u8>,
    /// Replacement gain in dB.
    pub gain_db: Option<f32>,
    /// Replacement RTSP transport preference.
    pub rtsp_transport: Option<RtspTransport>,
    /// Replacement quiet schedule; `Some(None)` clears the existing schedule.
    pub schedule_quiet: Option<Option<(String, String)>>,
    /// Replacement pipeline flags (all four are replaced atomically).
    pub pipeline: Option<PipelineFlags>,
}

fn validate_id(id: &str) -> Result<(), AudioSourceError> {
    if id.is_empty() {
        return Err(AudioSourceError::Invalid("source id is empty".to_string()));
    }
    if id.len() > 64 {
        return Err(AudioSourceError::Invalid(
            "source id longer than 64 characters".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(AudioSourceError::Invalid(
            "source id has unsupported characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_hhmm(s: &str) -> Result<(), AudioSourceError> {
    let bytes = s.as_bytes();
    let ok = s.len() == 5
        && bytes[2] == b':'
        && bytes[..2].iter().all(u8::is_ascii_digit)
        && bytes[3..].iter().all(u8::is_ascii_digit);
    if !ok {
        return Err(AudioSourceError::Invalid(format!(
            "expected HH:MM, got '{s}'"
        )));
    }
    let hh: u8 = s[..2]
        .parse()
        .map_err(|_| AudioSourceError::Invalid("invalid hour".to_string()))?;
    let mm: u8 = s[3..]
        .parse()
        .map_err(|_| AudioSourceError::Invalid("invalid minute".to_string()))?;
    if hh > 23 || mm > 59 {
        return Err(AudioSourceError::Invalid(format!(
            "out-of-range time '{s}'"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Store trait
// ---------------------------------------------------------------------------

/// CRUD operations on `audio_sources`.
pub trait AudioSourceStore {
    /// Return every non-disabled source, in `created_at ASC` order.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSourceError::Sqlite`] on database failure.
    fn list(&self) -> Result<Vec<AudioSource>, AudioSourceError>;

    /// Fetch one source by id (returns `None` for unknown id; soft-deleted
    /// rows do return).
    ///
    /// # Errors
    ///
    /// Returns [`AudioSourceError::Sqlite`] on database failure.
    fn get(&self, id: &str) -> Result<Option<AudioSource>, AudioSourceError>;

    /// Insert a new row. The caller picks the id (the admin form generates
    /// `src_{kind}_{epoch}`); duplicates surface as [`AudioSourceError::Conflict`].
    ///
    /// # Errors
    ///
    /// Returns [`AudioSourceError::Invalid`] when the id fails validation,
    /// [`AudioSourceError::Conflict`] when the id is taken, and
    /// [`AudioSourceError::Sqlite`] for underlying database failure.
    fn insert(&self, new: &NewAudioSource) -> Result<AudioSource, AudioSourceError>;

    /// Apply the patch to the row with `id`. Fields whose patch value is
    /// `None` are left unchanged. The `updated_at` column is bumped.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSourceError::NotFound`] if the row is absent,
    /// [`AudioSourceError::Invalid`] on a malformed value, and
    /// [`AudioSourceError::Sqlite`] for underlying database failure.
    fn update(&self, id: &str, patch: &AudioSourcePatch) -> Result<AudioSource, AudioSourceError>;

    /// Mark the row soft-deleted (sets `disabled_at = datetime('now')`).
    /// Subsequent `list()` calls exclude it.
    ///
    /// # Errors
    ///
    /// Returns [`AudioSourceError::NotFound`] if no such row exists, and
    /// [`AudioSourceError::Sqlite`] for underlying database failure.
    fn soft_delete(&self, id: &str) -> Result<(), AudioSourceError>;
}

impl AudioSourceStore for Connection {
    fn list(&self) -> Result<Vec<AudioSource>, AudioSourceError> {
        let mut stmt = self.prepare(
            "SELECT id, kind, device_id, label, sample_rate, channels, bit_depth,
                    gain_db, rtsp_transport, schedule_quiet_start, schedule_quiet_end,
                    pipeline_high_pass, pipeline_dc_removal, pipeline_agc,
                    pipeline_rtsp_keepalive, disabled_at, created_at, updated_at
             FROM audio_sources
             WHERE disabled_at IS NULL
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], row_to_source)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get(&self, id: &str) -> Result<Option<AudioSource>, AudioSourceError> {
        self.query_row(
            "SELECT id, kind, device_id, label, sample_rate, channels, bit_depth,
                    gain_db, rtsp_transport, schedule_quiet_start, schedule_quiet_end,
                    pipeline_high_pass, pipeline_dc_removal, pipeline_agc,
                    pipeline_rtsp_keepalive, disabled_at, created_at, updated_at
             FROM audio_sources WHERE id = ?1",
            params![id],
            row_to_source,
        )
        .optional()
        .map_err(Into::into)
    }

    fn insert(&self, new: &NewAudioSource) -> Result<AudioSource, AudioSourceError> {
        validate_id(&new.id)?;
        if let Some((s, e)) = new.schedule_quiet.as_ref() {
            validate_hhmm(s)?;
            validate_hhmm(e)?;
        }
        let (q_start, q_end) = match new.schedule_quiet.as_ref() {
            Some((s, e)) => (Some(s.as_str()), Some(e.as_str())),
            None => (None, None),
        };
        self.execute(
            "INSERT INTO audio_sources (
                id, kind, device_id, label, sample_rate, channels, bit_depth,
                gain_db, rtsp_transport, schedule_quiet_start, schedule_quiet_end,
                pipeline_high_pass, pipeline_dc_removal, pipeline_agc,
                pipeline_rtsp_keepalive
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                new.id,
                new.kind.as_str(),
                new.device_id,
                new.label,
                new.sample_rate,
                new.channels.as_str(),
                new.bit_depth,
                f64::from(new.gain_db),
                new.rtsp_transport.as_str(),
                q_start,
                q_end,
                i32::from(new.pipeline.high_pass),
                i32::from(new.pipeline.dc_removal),
                i32::from(new.pipeline.agc),
                i32::from(new.pipeline.rtsp_keepalive),
            ],
        )?;
        self.get(&new.id)?
            .ok_or_else(|| AudioSourceError::NotFound(new.id.clone()))
    }

    fn update(&self, id: &str, patch: &AudioSourcePatch) -> Result<AudioSource, AudioSourceError> {
        if let Some(Some((s, e))) = patch.schedule_quiet.as_ref() {
            validate_hhmm(s)?;
            validate_hhmm(e)?;
        }
        let _existing = self
            .get(id)?
            .ok_or_else(|| AudioSourceError::NotFound(format!("source id={id}")))?;

        // Build a single UPDATE that only writes the columns the caller
        // touched. SQLite supports parameterised SET lists so each clause
        // is appended with its own placeholder.
        let mut sets = Vec::<&str>::new();
        let mut bindings: Vec<rusqlite::types::Value> = Vec::new();
        macro_rules! push {
            ($col:literal, $val:expr) => {{
                sets.push(concat!($col, " = ?"));
                bindings.push($val);
            }};
        }
        if let Some(label) = patch.label.as_ref() {
            push!(
                "label",
                label.as_ref().map_or(rusqlite::types::Value::Null, |s| {
                    rusqlite::types::Value::Text(s.clone())
                })
            );
        }
        if let Some(device_id) = patch.device_id.as_ref() {
            push!("device_id", rusqlite::types::Value::Text(device_id.clone()));
        }
        if let Some(rate) = patch.sample_rate {
            push!(
                "sample_rate",
                rusqlite::types::Value::Integer(i64::from(rate))
            );
        }
        if let Some(channels) = patch.channels {
            push!(
                "channels",
                rusqlite::types::Value::Text(channels.as_str().to_string())
            );
        }
        if let Some(bd) = patch.bit_depth {
            push!("bit_depth", rusqlite::types::Value::Integer(i64::from(bd)));
        }
        if let Some(gain) = patch.gain_db {
            push!("gain_db", rusqlite::types::Value::Real(f64::from(gain)));
        }
        if let Some(transport) = patch.rtsp_transport {
            push!(
                "rtsp_transport",
                rusqlite::types::Value::Text(transport.as_str().to_string())
            );
        }
        if let Some(sched) = patch.schedule_quiet.as_ref() {
            let (s, e) = match sched.as_ref() {
                Some((s, e)) => (
                    rusqlite::types::Value::Text(s.clone()),
                    rusqlite::types::Value::Text(e.clone()),
                ),
                None => (rusqlite::types::Value::Null, rusqlite::types::Value::Null),
            };
            push!("schedule_quiet_start", s);
            push!("schedule_quiet_end", e);
        }
        if let Some(p) = patch.pipeline {
            push!(
                "pipeline_high_pass",
                rusqlite::types::Value::Integer(i64::from(p.high_pass))
            );
            push!(
                "pipeline_dc_removal",
                rusqlite::types::Value::Integer(i64::from(p.dc_removal))
            );
            push!(
                "pipeline_agc",
                rusqlite::types::Value::Integer(i64::from(p.agc))
            );
            push!(
                "pipeline_rtsp_keepalive",
                rusqlite::types::Value::Integer(i64::from(p.rtsp_keepalive))
            );
        }

        if sets.is_empty() {
            // No-op patch.
            return self
                .get(id)?
                .ok_or_else(|| AudioSourceError::NotFound(format!("source id={id}")));
        }
        sets.push("updated_at = datetime('now')");
        let sql = format!(
            "UPDATE audio_sources SET {set_list} WHERE id = ?",
            set_list = sets.join(", ")
        );
        bindings.push(rusqlite::types::Value::Text(id.to_string()));
        let params_dyn: Vec<&dyn rusqlite::ToSql> =
            bindings.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        self.execute(&sql, params_dyn.as_slice())?;
        self.get(id)?
            .ok_or_else(|| AudioSourceError::NotFound(format!("source id={id}")))
    }

    fn soft_delete(&self, id: &str) -> Result<(), AudioSourceError> {
        let n = self.execute(
            "UPDATE audio_sources
             SET disabled_at = datetime('now'), updated_at = datetime('now')
             WHERE id = ?1 AND disabled_at IS NULL",
            params![id],
        )?;
        if n == 0 {
            return Err(AudioSourceError::NotFound(format!("source id={id}")));
        }
        Ok(())
    }
}

fn row_to_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudioSource> {
    let kind_str: String = row.get(1)?;
    let kind = kind_str.parse::<SourceKind>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let channels_str: String = row.get(5)?;
    let channels = channels_str.parse::<Channels>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let transport_str: String = row.get(8)?;
    let rtsp_transport = transport_str.parse::<RtspTransport>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let q_start: Option<String> = row.get(9)?;
    let q_end: Option<String> = row.get(10)?;
    // Clippy 1.96 prefers `Option::zip` over the manual and_then(|s| map(|e| (s, e))) form.
    let schedule_quiet = q_start.zip(q_end);
    let gain: f64 = row.get(7)?;
    Ok(AudioSource {
        id: row.get(0)?,
        kind,
        device_id: row.get(2)?,
        label: row.get(3)?,
        sample_rate: row.get(4)?,
        channels,
        bit_depth: row.get(6)?,
        #[allow(clippy::cast_possible_truncation)]
        gain_db: gain as f32,
        rtsp_transport,
        schedule_quiet,
        pipeline: PipelineFlags {
            high_pass: row.get::<_, i64>(11)? != 0,
            dc_removal: row.get::<_, i64>(12)? != 0,
            agc: row.get::<_, i64>(13)? != 0,
            rtsp_keepalive: row.get::<_, i64>(14)? != 0,
        },
        disabled_at: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration;

    fn open_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory");
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        migration::migrate(&conn).expect("migrate");
        conn
    }

    fn sample_new(id: &str, kind: SourceKind, device_id: &str) -> NewAudioSource {
        NewAudioSource::defaults(id, kind, device_id)
    }

    #[test]
    fn fresh_table_starts_empty() {
        let conn = open_db();
        assert!(conn.list().unwrap().is_empty());
    }

    #[test]
    fn insert_and_list_round_trip() {
        let conn = open_db();
        let new = sample_new("src_usb_1", SourceKind::UsbAlsa, "plughw:1,0");
        let inserted = conn.insert(&new).unwrap();
        assert_eq!(inserted.id, "src_usb_1");
        assert_eq!(inserted.kind, SourceKind::UsbAlsa);
        assert_eq!(inserted.device_id, "plughw:1,0");
        assert_eq!(inserted.sample_rate, 48_000);
        assert_eq!(inserted.channels, Channels::Mono);
        assert_eq!(inserted.bit_depth, 24);
        assert!((inserted.gain_db - 0.0).abs() < 1e-6);
        assert_eq!(inserted.rtsp_transport, RtspTransport::Auto);
        assert_eq!(inserted.pipeline, PipelineFlags::default());

        let all = conn.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].device_id, "plughw:1,0");
    }

    #[test]
    fn duplicate_id_returns_conflict() {
        let conn = open_db();
        let new = sample_new("dup", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        let err = conn.insert(&new).expect_err("duplicate must fail");
        assert!(matches!(err, AudioSourceError::Conflict(_)));
    }

    #[test]
    fn invalid_id_rejected() {
        let conn = open_db();
        let bad = sample_new("a b", SourceKind::UsbAlsa, "hw:1,0");
        let err = conn.insert(&bad).expect_err("space rejected");
        assert!(matches!(err, AudioSourceError::Invalid(_)));
    }

    #[test]
    fn update_patches_only_named_columns() {
        let conn = open_db();
        let new = sample_new("src_u", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        let patch = AudioSourcePatch {
            label: Some(Some("Backyard feeder".to_string())),
            gain_db: Some(12.5),
            ..AudioSourcePatch::default()
        };
        let updated = conn.update("src_u", &patch).unwrap();
        assert_eq!(updated.label.as_deref(), Some("Backyard feeder"));
        assert!((updated.gain_db - 12.5).abs() < 1e-4);
        // Untouched columns retained.
        assert_eq!(updated.device_id, "hw:1,0");
        assert_eq!(updated.sample_rate, 48_000);
    }

    #[test]
    fn update_with_empty_patch_is_noop() {
        let conn = open_db();
        let new = sample_new("src_u", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        let patch = AudioSourcePatch::default();
        let result = conn.update("src_u", &patch).unwrap();
        assert_eq!(result.id, "src_u");
    }

    #[test]
    fn update_invalid_schedule_rejected() {
        let conn = open_db();
        let new = sample_new("src_u", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        let patch = AudioSourcePatch {
            schedule_quiet: Some(Some(("25:00".to_string(), "06:00".to_string()))),
            ..AudioSourcePatch::default()
        };
        assert!(matches!(
            conn.update("src_u", &patch),
            Err(AudioSourceError::Invalid(_))
        ));
    }

    #[test]
    fn update_missing_id_returns_not_found() {
        let conn = open_db();
        let patch = AudioSourcePatch {
            label: Some(Some("x".to_string())),
            ..AudioSourcePatch::default()
        };
        assert!(matches!(
            conn.update("missing", &patch),
            Err(AudioSourceError::NotFound(_))
        ));
    }

    #[test]
    fn soft_delete_hides_from_list_but_get_returns() {
        let conn = open_db();
        let new = sample_new("src_u", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        conn.soft_delete("src_u").unwrap();
        assert!(conn.list().unwrap().is_empty());
        let row = conn.get("src_u").unwrap().expect("row still present");
        assert!(row.disabled_at.is_some());
    }

    #[test]
    fn soft_delete_twice_returns_not_found_the_second_time() {
        let conn = open_db();
        let new = sample_new("src_u", SourceKind::UsbAlsa, "hw:1,0");
        conn.insert(&new).unwrap();
        conn.soft_delete("src_u").unwrap();
        assert!(matches!(
            conn.soft_delete("src_u"),
            Err(AudioSourceError::NotFound(_))
        ));
    }

    #[test]
    fn kinds_round_trip_through_strings() {
        for k in [SourceKind::UsbAlsa, SourceKind::PipeWire, SourceKind::Rtsp] {
            assert_eq!(k.as_str().parse::<SourceKind>().unwrap(), k);
        }
        assert!("nope".parse::<SourceKind>().is_err());
    }

    #[test]
    fn channels_round_trip() {
        for c in [
            Channels::Mono,
            Channels::Left,
            Channels::Right,
            Channels::Stereo,
        ] {
            assert_eq!(c.as_str().parse::<Channels>().unwrap(), c);
        }
    }

    #[test]
    fn rtsp_transport_round_trip() {
        for t in [RtspTransport::Auto, RtspTransport::Tcp, RtspTransport::Udp] {
            assert_eq!(t.as_str().parse::<RtspTransport>().unwrap(), t);
        }
    }

    #[test]
    fn hhmm_validation() {
        assert!(validate_hhmm("00:00").is_ok());
        assert!(validate_hhmm("23:59").is_ok());
        assert!(validate_hhmm("24:00").is_err());
        assert!(validate_hhmm("12:60").is_err());
        assert!(validate_hhmm("9:00").is_err());
        assert!(validate_hhmm("").is_err());
    }
}
