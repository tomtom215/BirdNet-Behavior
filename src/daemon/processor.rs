//! The detection-event processor: drains the daemon's event channel and
//! performs the per-event side effects (DB insert / quarantine, audio clip
//! extraction, alert-rule evaluation, WebSocket broadcast, and the external
//! notification fan-out). The pure decisions it makes live in `disposition`
//! and `webhook`; this module is the I/O orchestration around them.

use std::sync::{Arc, mpsc};

use birdnet_core::audio::extraction::Extractor;
use birdnet_integrations::notification::{
    NotificationContext, NotificationFilter, NotificationTemplate,
};

use super::disposition::{
    DispositionDecision, confidence_pct_round, confidence_pct_trunc, decide_disposition,
    derive_source_label, is_first_detection_today, latency_ms_to_seconds, passes_filter,
    should_dispatch_notification,
};
use birdnet_db::notifications::NotifStatus;
use birdnet_integrations::webhook::dispatch_webhook;

use crate::integrations::{AppriseHandle, EmailHandle, MqttHandle};

/// The learned per-species thresholds, and their persistence.
///
/// Wraps the pure tracker with the two pieces of I/O it needs: loading what the
/// station knew before this process started, and writing back when something
/// changes.
///
/// Writes happen on a level change and not on every confirmation. A confirmation
/// that only extends a lease is worth nothing on disk — it will be re-learned
/// within the hour — and a dawn chorus produces hundreds of them. Level changes
/// are bounded by the tracker's own cooldown to at most one per species per
/// fifteen minutes, so this is a handful of writes an hour on a busy morning.
struct DynamicThresholdState {
    tracker: birdnet_core::detection::dynamic_threshold::DynamicThresholds,
    enabled: bool,
}

impl DynamicThresholdState {
    /// Load persisted levels into `tracker`.
    ///
    /// A load failure is logged and the station starts with nothing learned,
    /// which is the safe direction: every threshold is then the operator's own
    /// until the site re-confirms its species.
    fn new(
        mut tracker: birdnet_core::detection::dynamic_threshold::DynamicThresholds,
        state: &birdnet_web::state::AppState,
    ) -> Self {
        let enabled = tracker.config().enabled;
        if !enabled {
            return Self { tracker, enabled };
        }
        let now_ms = crate::daemon::disposition::epoch_ms();
        match state.with_db(birdnet_db::dynamic_thresholds::load_all) {
            Ok(rows) => {
                let count = rows.len();
                tracker.restore(
                    rows.into_iter().map(|r| {
                        (
                            r.sci_name,
                            birdnet_core::detection::dynamic_threshold::SpeciesLevel {
                                level: r.level,
                                confirmations: r.confirmations,
                                expires_at_ms: r.expires_at_ms,
                                first_learned_ms: r.first_learned_ms,
                                last_confirmed_ms: r.last_confirmed_ms,
                            },
                        )
                    }),
                    now_ms,
                );
                tracing::info!(
                    stored = count,
                    live = tracker.live_count(now_ms),
                    "dynamic thresholds enabled"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "could not load learned thresholds; starting with none"
                );
            }
        }
        Self { tracker, enabled }
    }

    /// The tracker to judge against, or `None` when the feature is off.
    fn tracker(&self) -> Option<&birdnet_core::detection::dynamic_threshold::DynamicThresholds> {
        self.enabled.then_some(&self.tracker)
    }

    /// Record a detection that survived every gate, persisting on a change.
    ///
    /// Called at the point of insert and nowhere earlier. That placement is the
    /// safety property: a species the occurrence filter excluded, a chunk the
    /// noise filter dropped, a record quarantined for an implausible hour, a
    /// detection suppressed as a duplicate — none of them has confirmed
    /// anything, and letting one lower its own species' threshold would turn a
    /// single false positive into a stream of them.
    fn confirm(
        &mut self,
        sci_name: &str,
        confidence: f32,
        now_ms: i64,
        state: &birdnet_web::state::AppState,
    ) {
        if !self.enabled || !self.tracker.observe(sci_name, confidence, now_ms) {
            return;
        }
        let rows: Vec<birdnet_db::dynamic_thresholds::LearnedThreshold> = self
            .tracker
            .snapshot(now_ms)
            .into_iter()
            .map(
                |(sci_name, s)| birdnet_db::dynamic_thresholds::LearnedThreshold {
                    sci_name,
                    level: s.level,
                    confirmations: s.confirmations,
                    expires_at_ms: s.expires_at_ms,
                    first_learned_ms: s.first_learned_ms,
                    last_confirmed_ms: s.last_confirmed_ms,
                },
            )
            .collect();
        if let Err(e) =
            state.with_db(|conn| birdnet_db::dynamic_thresholds::replace_all(conn, &rows))
        {
            tracing::warn!(error = %e, "could not persist learned thresholds");
        }
    }
}

/// Per-species confidence thresholds, re-read from the database on a short TTL.
///
/// The thresholds decide whether a detection is accepted or quarantined for
/// review, and they are operator-driven: `/admin/species` writes them at
/// runtime. Capturing them once when the daemon starts meant a threshold set in
/// the UI did nothing at all until the service was restarted — the row appeared
/// in the table, the page confirmed the save, and detections kept being judged
/// by the old value, with nothing anywhere saying why. That is the same trap
/// the startup snapshot of locked clips set, and it bites hardest during the
/// one workflow this feature exists for: tuning a species up or down and
/// watching what happens next.
///
/// A TTL rather than a query per detection: the table is small and indexed, but
/// a dawn-chorus burst can push hundreds of events through this loop in a
/// minute, and none of them need a threshold fresher than this.
struct ThresholdCache {
    map: std::collections::HashMap<String, f64>,
    refreshed: std::time::Instant,
}

impl ThresholdCache {
    /// How stale a threshold may get before the next event re-reads it.
    const TTL: std::time::Duration = std::time::Duration::from_secs(30);

    fn new(initial: std::collections::HashMap<String, f64>) -> Self {
        Self {
            map: initial,
            refreshed: std::time::Instant::now(),
        }
    }

    /// The thresholds to judge the next detection by, refreshed if stale.
    ///
    /// A failed read keeps the previous map rather than falling back to an
    /// empty one: losing every threshold on a transient `SQLITE_BUSY` would
    /// silently accept the detections the operator asked to scrutinise.
    fn current(
        &mut self,
        state: &birdnet_web::state::AppState,
    ) -> &std::collections::HashMap<String, f64> {
        if self.refreshed.elapsed() >= Self::TTL {
            match state.with_db(birdnet_db::sqlite::get_species_threshold_map) {
                Ok(fresh) => self.map = fresh,
                Err(e) => {
                    tracing::warn!(error = %e, "could not refresh per-species thresholds; keeping the previous set");
                }
            }
            self.refreshed = std::time::Instant::now();
        }
        &self.map
    }
}

/// What a notification-log row says about the detection it concerns.
///
/// Built once per detection and cloned into each channel's task, so a channel
/// cannot record a different species than the one it sent.
#[derive(Clone)]
struct NotificationSubject {
    com_name: String,
    sci_name: String,
    confidence: f32,
    date: String,
    time: String,
}

/// Record one channel's outcome in `notification_log`.
///
/// # Why this exists
///
/// `notification_log` had **no writer in production**. `log_notification` was
/// implemented, unit-tested, and called by nothing — its only caller anywhere in
/// the tree was `examples/screenshot_server.rs`, the fixture that seeds the
/// documentation screenshots. Three surfaces read the table: the Notification
/// Center page, `/admin/notifications`, and the Station home's recent-activity
/// tab. All three were empty on every real station, permanently, while the
/// shipped `docs/book/images/notifications.png` showed them populated.
///
/// The feature is worth having rather than deleting, and specifically worth
/// having on the deployment this project is for: "did my BirdWeather uploads
/// actually land while I was away for a fortnight?" is not a question the
/// journal answers well, and it is the question an unattended station raises.
///
/// Best-effort by construction. A logging failure must never affect the
/// notification it is describing, and it must never propagate into the detection
/// pipeline, so this swallows its error into a `debug` line.
fn record_notification(
    state: &birdnet_web::state::AppState,
    channel: &str,
    subject: &NotificationSubject,
    status: NotifStatus,
    message: Option<&str>,
    error: Option<&str>,
) {
    let record = birdnet_db::notifications::NotifRecord {
        channel,
        species_com_name: Some(&subject.com_name),
        species_sci_name: Some(&subject.sci_name),
        confidence: Some(f64::from(subject.confidence)),
        detection_date: Some(&subject.date),
        detection_time: Some(&subject.time),
        status,
        message,
        error,
    };
    if let Err(e) = state.with_db(|conn| birdnet_db::notifications::log_notification(conn, &record))
    {
        tracing::debug!(error = %e, channel, "could not record notification outcome");
    }
}

/// Bridge detection events from the daemon to database inserts and WebSocket broadcasts.
#[allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(super) fn event_processor(
    event_rx: mpsc::Receiver<birdnet_core::detection::daemon::DetectionEvent>,
    state: birdnet_web::state::AppState,
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    apprise: Option<AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<EmailHandle>,
    mqtt: Option<MqttHandle>,
    notification_filter: NotificationFilter,
    notification_template: NotificationTemplate,
    rt_handle: tokio::runtime::Handle,
    species_thresholds: std::collections::HashMap<String, f64>,
    global_confidence: f32,
    extractor: Extractor,
    duplicate_interval_secs: i64,
    daylight: crate::daemon::daylight::DaylightFilter,
    dynamic: birdnet_core::detection::dynamic_threshold::DynamicThresholds,
) {
    tracing::debug!("event processor started");
    let mut species_thresholds = ThresholdCache::new(species_thresholds);
    let mut dynamic = DynamicThresholdState::new(dynamic, &state);
    let mut duplicates = crate::daemon::duplicate::DuplicateGate::new(duplicate_interval_secs);
    if daylight.is_enabled() {
        tracing::info!("taxon-aware daylight filter enabled");
    }
    if duplicates.is_enabled() {
        tracing::info!(
            interval_secs = duplicate_interval_secs,
            "duplicate-prediction interval enabled"
        );
    }

    loop {
        let Ok(event) = event_rx.recv() else {
            tracing::info!("event channel closed, stopping event processor");
            break;
        };
        let species_thresholds = species_thresholds.current(&state);
        let now_ms = crate::daemon::disposition::epoch_ms();

        let detection = &event.detection;
        let correlation_id = event.correlation_id.as_str();

        // Does this recording name a day the station could have recorded on?
        //
        // First of every gate, because a row with an impossible date is worse
        // than a wrong species: it cannot be corrected later and it cannot be
        // deleted by anything the operator would think to run. Before NTP
        // lands, an RTC-less Raspberry Pi reads the epoch; the capture tee
        // stamps that into the segment filename, and `Date` and `Time` are
        // parsed straight back out of it. Nothing checked. Such a row is filed
        // under 1970-01-01 hour 00 for ever, makes `MIN(Date)` report every
        // species touched in that window as "first seen 1970", stretches the
        // history calendar across 56 years, and sorts before everything the
        // station has ever heard. Retention then reclaims its audio for being
        // older than any cutoff, so the evidence goes and the row stays.
        //
        // Quarantined rather than dropped: something was genuinely heard, and
        // the operator should be able to see that their station spent a
        // fortnight recording without knowing what day it was.
        // `tests/clock_steps_backwards.rs` already pins that a naive "drop
        // implausible dates" filter is the wrong answer.
        if !birdnet_core::civil::date_looks_plausible(&detection.date) {
            tracing::warn!(
                correlation_id,
                species = %detection.scientific_name,
                date = %detection.date,
                time = %detection.time,
                "quarantining a detection whose recording date is not a real date: \
                 the system clock was not set when this was captured"
            );
            let week_str = detection.week.to_string();
            let file_str = event.source_file.to_string_lossy();
            let q_record = birdnet_db::sqlite::QuarantineRecord {
                date: &detection.date,
                time: &detection.time,
                sci_name: &detection.scientific_name,
                com_name: &detection.common_name,
                confidence: f64::from(detection.confidence),
                sf_probability: None,
                reason: birdnet_db::sqlite::QuarantineReason::ImplausibleClock,
                file_name: if file_str.is_empty() {
                    None
                } else {
                    Some(file_str.as_ref())
                },
                lat: None,
                lon: None,
                week: week_str.parse::<i32>().ok(),
            };
            if let Some(Err(e)) =
                state.with_ingest_db(|conn| birdnet_db::sqlite::insert_quarantine(conn, &q_record))
            {
                tracing::warn!(
                    correlation_id,
                    error = %e,
                    species = %detection.scientific_name,
                    "failed to quarantine a detection with an unset clock"
                );
            }
            state.metrics().inc_detection_dropped("implausible_clock");
            continue;
        }

        // Is this bird plausible at this hour? Runs before the threshold
        // gates so a night-time songbird is quarantined with the reason that
        // actually explains it, rather than being recorded as a threshold
        // miss — the operator reviewing the queue is deciding whether to
        // believe it, and "2 a.m." is the fact that decides.
        if crate::daemon::daylight::DaylightVerdict::Quarantine
            == daylight.verdict(&detection.scientific_name, &detection.date, &detection.time)
        {
            tracing::debug!(
                correlation_id,
                species = %detection.scientific_name,
                time = %detection.time,
                "quarantining a day bird heard in the middle of the night"
            );
            let week_str = detection.week.to_string();
            let file_str = event.source_file.to_string_lossy();
            let q_record = birdnet_db::sqlite::QuarantineRecord {
                date: &detection.date,
                time: &detection.time,
                sci_name: &detection.scientific_name,
                com_name: &detection.common_name,
                confidence: f64::from(detection.confidence),
                sf_probability: None,
                reason: birdnet_db::sqlite::QuarantineReason::ImplausibleHour,
                file_name: if file_str.is_empty() {
                    None
                } else {
                    Some(file_str.as_ref())
                },
                lat: None,
                lon: None,
                week: week_str.parse::<i32>().ok(),
            };
            if let Some(Err(e)) =
                state.with_ingest_db(|conn| birdnet_db::sqlite::insert_quarantine(conn, &q_record))
            {
                tracing::warn!(
                    correlation_id,
                    error = %e,
                    species = %detection.scientific_name,
                    "failed to quarantine a night-time detection"
                );
            }
            state.metrics().inc_detection_dropped("implausible_hour");
            continue;
        }

        // Apply per-species confidence threshold.
        // Detections that pass the global threshold but fail a stricter
        // per-species threshold are quarantined for manual review rather
        // than silently dropped.
        match decide_disposition(
            detection.confidence,
            &detection.scientific_name,
            species_thresholds,
            global_confidence,
            dynamic.tracker(),
            now_ms,
        ) {
            DispositionDecision::Quarantine { threshold } => {
                tracing::debug!(
                    correlation_id,
                    species = %detection.scientific_name,
                    confidence = detection.confidence,
                    threshold,
                    "detection below per-species threshold — quarantining for review"
                );
                let week_str = detection.week.to_string();
                let file_str = event.source_file.to_string_lossy();
                let q_record = birdnet_db::sqlite::QuarantineRecord {
                    date: &detection.date,
                    time: &detection.time,
                    sci_name: &detection.scientific_name,
                    com_name: &detection.common_name,
                    confidence: f64::from(detection.confidence),
                    sf_probability: None,
                    reason: birdnet_db::sqlite::QuarantineReason::LowConfidence,
                    file_name: if file_str.is_empty() {
                        None
                    } else {
                        Some(file_str.as_ref())
                    },
                    lat: None,
                    lon: None,
                    week: week_str.parse::<i32>().ok(),
                };
                if let Some(Err(e)) = state
                    .with_ingest_db(|conn| birdnet_db::sqlite::insert_quarantine(conn, &q_record))
                {
                    tracing::warn!(
                        correlation_id,
                        error = %e,
                        species = %detection.scientific_name,
                        "failed to quarantine detection"
                    );
                }
                state.metrics().inc_detection_dropped("quarantine");
                continue;
            }
            DispositionDecision::DropBelowGlobal => {
                // Counted, not silent. "The station is detecting nothing" and
                // "the station is discarding everything" look identical from
                // outside, and the reason label is what tells them apart.
                state.metrics().inc_detection_dropped("confidence");
                continue;
            }
            DispositionDecision::Accept => {}
        }

        // One continuous song is one detection. Applied *after* the threshold
        // gates so a suppressed duplicate cannot consume the interval on
        // behalf of a detection that would itself have been quarantined —
        // otherwise a run of low-confidence chunks would shadow the confident
        // one in the middle of them.
        //
        // A detection whose own timestamp cannot be read is admitted rather
        // than gated: `Date`/`Time` are free-form text, and the alternative is
        // dropping a real detection because its filename was odd.
        if let Some(at_secs) =
            crate::daemon::duplicate::detection_secs(&detection.date, &detection.time)
            && let crate::daemon::duplicate::DuplicateVerdict::Suppress { since_last_secs } =
                duplicates.admit(&detection.scientific_name, at_secs)
        {
            tracing::debug!(
                correlation_id,
                species = %detection.scientific_name,
                since_last_secs,
                "suppressing a repeat within the duplicate-prediction interval"
            );
            state.metrics().inc_detection_dropped("duplicate");
            continue;
        }

        // Insert into SQLite. Numeric columns receive Option<f64> /
        // Option<i64> so missing values become SQLite NULLs (the schema
        // declares Lat/Lon/Cutoff/Sens/Overlap as REAL and Week as
        // INTEGER). Previously the daemon passed empty strings here,
        // which SQLite silently stored as TEXT and every subsequent
        // typed read returned "Invalid column type Text at index N".
        // Per-detection source/stream label parsed from the source filename: the
        // RTSP stream id (e.g. `cam1`) or `local` for the on-board mic. Tags the
        // row (so multi-stream detections are attributable) and feeds the
        // per-source liveness gauge below.
        let source_label = derive_source_label(&event.source_file);

        // Extract the clip FIRST, so the DB row can reference the SAVED CLIP's
        // filename — the flat name the web serves recordings by (`File_Name`) —
        // rather than the transient source segment, which lives on the RAM tmpfs
        // and is drained after processing. On failure, fall back to the source
        // name so the detection is still recorded (just without a playable clip).
        let extracted = extractor.extract_detection(&event.source_file, detection);
        match &extracted {
            Ok(path) => tracing::debug!(
                species = %detection.common_name,
                path = %path.display(),
                "audio clip extracted"
            ),
            Err(e) => tracing::warn!(
                error = %e,
                species = %detection.common_name,
                "audio clip extraction failed"
            ),
        }
        // `File_Name` is the clip's base name (what `/api/v2/recordings/{name}`
        // serves), and the persisted duration is the clip's — not the ~15 s
        // source segment's. Read cheaply from the file header; `None` leaves the
        // column NULL (never faked).
        let (file_str, source_duration_secs) = match &extracted {
            Ok(clip_path) => (
                clip_path.file_name().map_or_else(
                    || event.source_file.to_string_lossy().into_owned(),
                    |n| n.to_string_lossy().into_owned(),
                ),
                birdnet_core::audio::decode::probe_duration_secs(clip_path),
            ),
            Err(_) => (
                event.source_file.to_string_lossy().into_owned(),
                birdnet_core::audio::decode::probe_duration_secs(&event.source_file),
            ),
        };
        let record = birdnet_db::sqlite::DetectionRecord {
            date: &detection.date,
            time: &detection.time,
            sci_name: &detection.scientific_name,
            com_name: &detection.common_name,
            confidence: f64::from(detection.confidence),
            lat: None,
            lon: None,
            cutoff: None,
            week: Some(i64::from(detection.week)),
            sensitivity: None,
            overlap: None,
            file_name: &file_str,
            // Without this, every chunk of one recording shares the same
            // UNIQUE key (Date, Time, Sci_Name, File_Name) and only the
            // first chunk's detection is kept. See migration 11.
            chunk_offset_secs: Some(f64::from(detection.start)),
            // The correlation_id is the per-file id stamped on every
            // event; persisting it closes the log→row round trip so an
            // admin clicking a suspicious detection in the web UI can
            // grep the journal for the exact decode/infer/notify
            // slice that produced it. Empty string means "no id"
            // (forward-compat with non-daemon write paths) — store as
            // NULL so the DB doesn't try to index empty strings.
            correlation_id: if correlation_id.is_empty() {
                None
            } else {
                Some(correlation_id)
            },
            source: Some(&source_label),
            duration_secs: source_duration_secs,
            // The instant, not just the wall clock. `Date`/`Time` are local and
            // carry no offset, so they are not a point in time — the local hour
            // daylight saving repeats each autumn happens twice under one
            // label. This path is the only one that can resolve that, because
            // it is running *now* and the offset in force is known: the first
            // pass is stamped while the offset is still +2 and the second while
            // it is +1, so the two land an hour apart, which is what happened.
            //
            // Everything else — imports, the backfill — falls back to migration
            // 32's trigger and a tz-database lookup, which is right for the date
            // and cannot tell those two apart.
            detected_at_utc: birdnet_core::civil::unix_secs_from_local(
                &detection.date,
                &detection.time,
                birdnet_db::clock::local_utc_offset_secs(),
            ),
        };

        let metrics = state.metrics();
        // Liveness signal: a detection here proves the source produced a
        // file the watcher picked up. Stamp the audio-source label
        // accordingly. We parse the source label from the filename
        // because the source supervisor doesn't currently feed liveness
        // updates upstream; the filename's RTSP prefix is the only
        // per-event tag the daemon sees (derived once above for the record).
        metrics.set_source_up(&source_label, true);

        let db_start = std::time::Instant::now();
        // `with_ingest_db`, not `with_db`: PS-5. A database that has failed its
        // daily integrity check stops taking detection rows, and this is one of
        // the three writes that stop.
        let Some(insert_result) =
            state.with_ingest_db(|conn| birdnet_db::sqlite::insert_detection(conn, &record))
        else {
            // Not a database failure — a refusal. Counted through the same
            // metric anyway, because that metric's contract is "a classified
            // detection was lost after the model agreed it was real", and this
            // is exactly that. An operator already alerting on it should hear
            // about this too.
            metrics.inc_detection_write_failed();
            tracing::warn!(
                correlation_id,
                species = %detection.scientific_name,
                "detection classified but not recorded: this station's database \
                 failed its integrity check and detection writes are halted"
            );
            continue;
        };
        metrics.observe_db_write_seconds(db_start.elapsed().as_secs_f64());
        if let Err(e) = insert_result {
            // Counted, not just logged. This is the only place a classified
            // detection can be lost after the model has agreed it is real, and
            // an unattended station that only writes it to the journal has not
            // told anybody. The known route here is the local hour that
            // daylight-saving repeats each autumn — `(Date, Time, Sci_Name,
            // File_Name, chunk_offset_secs)` is this schema's identity and all
            // of it is local wall-clock, so the second pass of that hour can
            // collide with the first — but a full disk or a locked database
            // arrives the same way.
            metrics.inc_detection_write_failed();
            tracing::warn!(
                correlation_id,
                error = %e,
                species = %detection.scientific_name,
                date = %detection.date,
                time = %detection.time,
                "detection classified but refused by the database — it is lost"
            );
        } else {
            metrics.inc_detection(&detection.scientific_name, detection.start);
            // The one place a species can confirm itself, and it is after the
            // row exists on purpose. Every gate has run by here — the chunk
            // filters and the occurrence filter in the daemon, the plausible-
            // hour filter, the threshold gate, the duplicate interval — and a
            // detection that failed any of them took an early `continue`. A
            // confirmation from a detection the database then refused would be
            // learning from something the station did not record.
            dynamic.confirm(
                &detection.scientific_name,
                detection.confidence,
                now_ms,
                &state,
            );
        }
        // event.latency_ms covers decode + inference; surface as a histogram
        // so the dashboard can flag rising p95s before they catch the eye.
        metrics.observe_inference_seconds(latency_ms_to_seconds(event.latency_ms));

        // (The clip was extracted above, before the insert, so File_Name records
        // the saved clip rather than the transient source segment.)

        // Also insert into DuckDB analytics (if enabled).
        //
        // The same twelve columns the bulk sync copies, from the same `record`
        // that went to SQLite. This used to write six and leave Lat, Lon,
        // Cutoff, Week, Sens and Overlap NULL, so a row written live and the
        // same row rebuilt by a resync were different — including after the
        // startup drift rebuild, which would silently fill them in.
        #[cfg(feature = "analytics")]
        if state.has_analytics() {
            let live = birdnet_behavioral::connection::LiveDetection {
                date: record.date,
                time: record.time,
                sci_name: record.sci_name,
                com_name: record.com_name,
                confidence: record.confidence,
                lat: record.lat,
                lon: record.lon,
                cutoff: record.cutoff,
                week: record.week.and_then(|w| i32::try_from(w).ok()),
                sens: record.sensitivity,
                overlap: record.overlap,
                file_name: &file_str,
                // The same instant the SQLite row carries, so the two copies
                // agree. Recomputing it here would let the two drift across a
                // daylight-saving boundary that fell between the writes.
                detected_at_utc: record.detected_at_utc,
            };
            let insert_result = state.with_analytics(|adb| adb.insert_detection(&live));
            if let Some(Err(e)) = insert_result {
                tracing::warn!(error = %e, "failed to insert detection into DuckDB");
            }
        }

        // Evaluate alert rules (loaded fresh for each detection to reflect UI changes).
        let rule_suppressed = state.with_db(|conn| {
            let rules = birdnet_db::alert_rules::list_rules(conn).unwrap_or_default();
            let matched = birdnet_db::alert_rules::evaluate_rules(
                &rules,
                &detection.common_name,
                f64::from(detection.confidence),
                &detection.time,
            );

            let mut suppressed = false;
            for rule in matched {
                match &rule.action {
                    birdnet_db::alert_rules::AlertAction::Log => {
                        tracing::info!(
                            rule = %rule.name,
                            species = %detection.common_name,
                            sci_name = %detection.scientific_name,
                            confidence = detection.confidence,
                            date = %detection.date,
                            time = %detection.time,
                            "alert rule matched (log action)"
                        );
                    }
                    birdnet_db::alert_rules::AlertAction::Suppress => {
                        tracing::debug!(
                            rule = %rule.name,
                            species = %detection.common_name,
                            "alert rule suppressing notifications"
                        );
                        suppressed = true;
                    }
                    birdnet_db::alert_rules::AlertAction::Webhook {
                        url,
                        method,
                        body_template,
                        auth,
                    } => {
                        let body = body_template.as_deref().map(|tmpl| {
                            birdnet_db::alert_rules::render_webhook_body(
                                tmpl,
                                &detection.common_name,
                                &detection.scientific_name,
                                f64::from(detection.confidence),
                                &detection.date,
                                &detection.time,
                            )
                        });
                        let url = url.clone();
                        let method = method.clone();
                        let auth = auth.clone();
                        let rule_name = rule.name.clone();
                        // The path and query of a webhook URL are where its
                        // secret lives, so the log names the host only.
                        let target = birdnet_integrations::webhook::redact_url(&url);
                        rt_handle.spawn(async move {
                            match dispatch_webhook(&url, &method, body.as_deref(), auth.as_ref())
                                .await
                            {
                                Ok(status) => tracing::debug!(
                                    rule = %rule_name,
                                    target,
                                    status,
                                    "webhook dispatched"
                                ),
                                Err(e) => tracing::warn!(
                                    rule = %rule_name,
                                    target,
                                    error = %e,
                                    "webhook dispatch failed"
                                ),
                            }
                        });
                    }
                }
            }
            suppressed
        });

        // Check if this is the first detection of this species today
        // (to power the rare-species celebration in the dashboard).
        let is_new_today = state.with_db(|conn| {
            let today_count = birdnet_db::sqlite::detection_count_for_species_date(
                conn,
                &detection.date,
                &detection.scientific_name,
            )
            .unwrap_or(1);
            is_first_detection_today(today_count)
        });

        // Broadcast to WebSocket clients.
        let ws_event = birdnet_web::routes::websocket::WsDetectionEvent {
            event: "detection",
            common_name: detection.common_name.clone(),
            scientific_name: detection.scientific_name.clone(),
            confidence: detection.confidence,
            date: detection.date.clone(),
            time: detection.time.clone(),
            start: detection.start,
            stop: detection.stop,
            is_new_today,
        };
        broadcast.send(&ws_event);

        // Build notification context for template rendering.
        let notify_ctx = NotificationContext {
            sci_name: detection.scientific_name.clone(),
            com_name: detection.common_name.clone(),
            confidence: detection.confidence,
            confidence_pct: confidence_pct_trunc(detection.confidence),
            date: detection.date.clone(),
            time: detection.time.clone(),
            week: detection.week,
            latitude: 0.0,
            longitude: 0.0,
            reason: String::new(),
            listen_url: None,
            image_url: None,
            station_url: None,
        };

        // Check notification filter (trigger mode + species filter).
        // Also respect Suppress alert rules.
        let filter_says_notify =
            notification_filter.should_notify(&detection.scientific_name, None);
        let dispatch_allowed = passes_filter(rule_suppressed, filter_says_notify);

        // What every channel below records about this detection. Built once so a
        // channel cannot log a different species than it sent.
        let subject = NotificationSubject {
            com_name: detection.common_name.clone(),
            sci_name: detection.scientific_name.clone(),
            confidence: detection.confidence,
            date: detection.date.clone(),
            time: detection.time.clone(),
        };

        // Apprise push notification (with filter and template).
        if let Some(ref apprise) = apprise {
            let apprise_says_notify = apprise
                .blocking_lock()
                .should_notify(&detection.common_name, detection.confidence);
            let should_send = should_dispatch_notification(dispatch_allowed, apprise_says_notify);

            if should_send {
                let (title, body) = notification_template.render(&notify_ctx);
                let client = Arc::clone(apprise);
                let log_state = state.clone();
                let log_subject = subject.clone();

                rt_handle.spawn(async move {
                    let result = client
                        .lock()
                        .await
                        .send_notification(
                            &title,
                            &body,
                            birdnet_integrations::apprise::NotifyType::Info,
                        )
                        .await;
                    match result {
                        Ok(()) => record_notification(
                            &log_state,
                            "apprise",
                            &log_subject,
                            NotifStatus::Sent,
                            Some(&title),
                            None,
                        ),
                        Err(e) => {
                            log_state
                                .metrics()
                                .inc_notification_dropped(e.drop_reason());
                            // A notification no destination was even tried for
                            // — every one skipped by the rate limiter or an
                            // open circuit, or none configured — is counted
                            // and not logged as a row. During a dawn chorus
                            // the limiter refuses thousands of these, and a
                            // row each would bury the sends that matter, for
                            // the same reason there is deliberately no
                            // `skipped` row below.
                            if !e.nothing_was_attempted() {
                                tracing::warn!(error = %e, "Apprise notification failed");
                                record_notification(
                                    &log_state,
                                    "apprise",
                                    &log_subject,
                                    NotifStatus::Failed,
                                    Some(&title),
                                    Some(&e.to_string()),
                                );
                            }
                        }
                    }
                });
            }
            // Deliberately no `skipped` row. On a station where Apprise is set
            // to "new species only", every one of the day's several thousand
            // detections would write one — a row per detection per channel,
            // burying the sends that matter under the ones that were never going
            // to happen, and putting four extra writes on the detection path for
            // it. The log records delivery *attempts*.
        }

        // BirdWeather upload.
        if let Some(ref bw) = birdweather {
            let post = birdnet_integrations::birdweather::DetectionPost {
                timestamp: format!("{}T{}Z", detection.date, detection.time),
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: detection.confidence,
                lat: bw.coordinates().0,
                lon: bw.coordinates().1,
            };
            let client = bw.clone();
            let queue_state = state.clone();
            let log_state = state.clone();
            let log_subject = subject.clone();
            rt_handle.spawn(async move {
                let Err(e) = client.post_detection(&post).await else {
                    record_notification(
                        &log_state,
                        "birdweather",
                        &log_subject,
                        NotifStatus::Sent,
                        None,
                        None,
                    );
                    return;
                };
                // Recorded as `queued`, not `failed`: the payload is parked for
                // the store-and-forward drainer, so "this did not reach
                // BirdWeather yet" is a different fact from "this was lost", and
                // the Notification Center is where an operator on a flaky uplink
                // needs to be able to tell them apart.
                record_notification(
                    &log_state,
                    "birdweather",
                    &log_subject,
                    NotifStatus::Queued,
                    None,
                    Some(&e.to_string()),
                );
                // Park the payload for the store-and-forward drainer instead
                // of dropping it: BirdWeather is an append-only record that
                // accepts late posts, so an upload lost to a Wi-Fi/LTE outage
                // is real data loss with no second chance otherwise.
                tracing::warn!(
                    error = %e,
                    species = %post.common_name,
                    "BirdWeather post failed; queueing for replay"
                );
                let payload = match serde_json::to_string(&post) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::error!(error = %e, "BirdWeather payload unserialisable; dropped");
                        return;
                    }
                };
                let _ = tokio::task::spawn_blocking(move || {
                    let now = crate::integrations::unix_now_secs();
                    queue_state.with_ingest_db(|conn| {
                        if let Err(e) = birdnet_db::outbound_queue::enqueue(
                            conn,
                            birdnet_integrations::birdweather::QUEUE_KIND,
                            &payload,
                            now,
                        ) {
                            tracing::warn!(error = %e, "failed to queue BirdWeather payload");
                        }
                    });
                })
                .await;
            });
        }

        // Email alert.
        if let Some(ref notifier) = email {
            let notifier = std::sync::Arc::clone(notifier);
            let alert = birdnet_integrations::email::DetectionEmail {
                common_name: detection.common_name.clone(),
                scientific_name: detection.scientific_name.clone(),
                confidence: f64::from(detection.confidence),
                date: detection.date.clone(),
                time: detection.time.clone(),
                station_name: None,
                detection_url: None,
            };
            let log_state = state.clone();
            let log_subject = subject.clone();
            rt_handle.spawn(async move {
                match notifier.notify(&alert).await {
                    Ok(true) => {
                        tracing::debug!(species = %alert.common_name, "email alert sent");
                        record_notification(
                            &log_state,
                            "email",
                            &log_subject,
                            NotifStatus::Sent,
                            None,
                            None,
                        );
                    }
                    // The notifier's own filter declined; not an attempt.
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, species = %alert.common_name, "email alert failed");
                        record_notification(
                            &log_state,
                            "email",
                            &log_subject,
                            NotifStatus::Failed,
                            None,
                            Some(&e.to_string()),
                        );
                    }
                }
            });
        }

        // MQTT publish. Fire-and-forget on the blocking pool — the same
        // discipline as the BirdWeather/Apprise/email/heartbeat dispatches
        // above. `publish_detection` opens a fresh TCP connection bounded by
        // `connect_timeout`, but several seconds × every detection is still
        // far too long to run inline: this `event_processor` is a SINGLE
        // blocking thread draining the detection-event channel, so a blocking
        // publish to an offline broker would serialize detection handling
        // behind a dead network path and back the channel up. Spawning it
        // detaches that latency from the detection pipeline entirely.
        if let Some(ref mqtt_client) = mqtt {
            let payload = birdnet_integrations::mqtt::DetectionPayload {
                timestamp: format!("{}T{}", detection.date, detection.time),
                scientific_name: detection.scientific_name.clone(),
                common_name: detection.common_name.clone(),
                confidence: detection.confidence,
                confidence_pct: confidence_pct_round(detection.confidence),
                file_name: detection.file_name_extr.clone(),
                rtsp_id: event
                    .source_file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| {
                        birdnet_core::detection::types::RecordingFile::parse(n)
                            .and_then(|rf| rf.rtsp_id)
                    }),
            };
            let client = Arc::clone(mqtt_client);
            let species = detection.common_name.clone();
            let log_state = state.clone();
            let log_subject = subject.clone();
            rt_handle.spawn_blocking(move || match client.publish_detection(&payload) {
                Ok(()) => record_notification(
                    &log_state,
                    "mqtt",
                    &log_subject,
                    NotifStatus::Sent,
                    None,
                    None,
                ),
                Err(e) => {
                    tracing::debug!(
                        error = %e,
                        species = %species,
                        "MQTT publish failed (broker may be offline)"
                    );
                    record_notification(
                        &log_state,
                        "mqtt",
                        &log_subject,
                        NotifStatus::Failed,
                        None,
                        Some(&e.to_string()),
                    );
                }
            });
        }

        tracing::debug!(
            correlation_id,
            species = %detection.common_name,
            confidence = format!("{:.0}%", detection.confidence * 100.0),
            latency_ms = event.latency_ms,
            ws_clients = broadcast.client_count(),
            "event processed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::test_support::thresholds;
    use birdnet_core::audio::extraction::ExtractionConfig;
    use std::collections::HashMap;
    use std::path::PathBuf;

    // ── event_processor: one-shot in-process test ───────────────────────
    //
    // Sends a single `DetectionEvent` through the channel, then drops the
    // sender so `event_processor` exits its loop after consuming the one
    // event. Asserts a row was inserted into the DB. This catches
    // `replace event_processor with ()` because the mutant produces no
    // DB write at all (the row count stays 0).
    //
    // Lives inside the daemon module so the private `event_processor`
    // function is directly callable.

    // ── DynamicThresholdState ──────────────────────────────────────────
    //
    // Neither `tracker()` nor `confirm()` had a test. Both are the seam
    // between the dynamic-threshold feature and the rest of the daemon, and
    // cargo-mutants could rewrite either without the suite noticing.

    use birdnet_core::detection::dynamic_threshold::{DynamicThresholdConfig, DynamicThresholds};

    fn dyn_state(enabled: bool) -> (tempfile::TempDir, birdnet_web::state::AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let _ = enabled;
        (tmp, state)
    }

    fn tracker_with(enabled: bool) -> DynamicThresholds {
        DynamicThresholds::new(DynamicThresholdConfig {
            enabled,
            trigger: 0.9,
            min: 0.3,
            valid_hours: 24,
        })
    }

    /// The feature's off-switch. `tracker()` hands the judging path a tracker
    /// only when the operator turned the feature on — return `Some` regardless
    /// and every station gets adjusted thresholds it never asked for; return
    /// `None` regardless and the feature is dead while still appearing
    /// configured. Both halves are asserted, so this discriminates rather than
    /// alarms.
    #[test]
    fn the_tracker_is_offered_only_when_the_feature_is_enabled() {
        let (_tmp, state) = dyn_state(true);
        let on = DynamicThresholdState::new(tracker_with(true), &state);
        assert!(
            on.tracker().is_some(),
            "an enabled tracker must be offered to the judging path"
        );

        let (_tmp2, state2) = dyn_state(false);
        let off = DynamicThresholdState::new(tracker_with(false), &state2);
        assert!(
            off.tracker().is_none(),
            "a disabled tracker must not be offered — the feature is off"
        );
    }

    /// `confirm` learns only when the feature is on, and only from a detection
    /// confident enough to trigger.
    ///
    /// The guard is `if !self.enabled || !self.tracker.observe(..) { return }`,
    /// which is three mutable pieces: the `||`, and each `!`. Every one of them
    /// either stops the feature learning at all or lets it learn from
    /// detections it should not, so each gets an assertion here.
    #[test]
    fn confirming_learns_only_when_enabled_and_confident() {
        let (_tmp, state) = dyn_state(true);
        let mut on = DynamicThresholdState::new(tracker_with(true), &state);

        // Below the 0.9 trigger: nothing is learned, so no row is written.
        on.confirm("Turdus merula", 0.5, 1_000, &state);
        let rows = state
            .with_db(birdnet_db::dynamic_thresholds::load_all)
            .expect("load");
        assert!(
            rows.is_empty(),
            "a detection below the trigger must not confirm anything, got {rows:?}"
        );

        // At the trigger: the species is learned and persisted.
        on.confirm("Turdus merula", 0.95, 1_000, &state);
        let rows = state
            .with_db(birdnet_db::dynamic_thresholds::load_all)
            .expect("load");
        assert_eq!(
            rows.len(),
            1,
            "a confident detection must be learned and written: {rows:?}"
        );
        assert_eq!(rows[0].sci_name, "Turdus merula");

        // With the feature off, even a confident detection learns nothing.
        let (_tmp2, state2) = dyn_state(false);
        let mut off = DynamicThresholdState::new(tracker_with(false), &state2);
        off.confirm("Turdus merula", 0.99, 1_000, &state2);
        let rows = state2
            .with_db(birdnet_db::dynamic_thresholds::load_all)
            .expect("load");
        assert!(
            rows.is_empty(),
            "a disabled feature must not learn: {rows:?}"
        );
    }

    // ── ThresholdCache ─────────────────────────────────────────────────

    #[test]
    fn threshold_cache_serves_the_initial_map_until_the_ttl_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let mut cache = ThresholdCache::new(thresholds(&[("Pica pica", 0.9)]));

        // A threshold written after construction is not visible yet — the
        // cache is fresh, so this pass must not pay for a database read.
        state.with_db(|c| {
            birdnet_db::sqlite::set_species_threshold(c, "Turdus merula", 0.5).unwrap();
        });
        let now = cache.current(&state);
        assert!(now.contains_key("Pica pica"));
        assert!(!now.contains_key("Turdus merula"));
    }

    #[test]
    fn threshold_cache_picks_up_a_new_threshold_once_stale() {
        // The regression: thresholds were captured when the daemon started, so
        // setting one in /admin/species did nothing until a restart.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let mut cache = ThresholdCache::new(HashMap::new());

        state.with_db(|c| {
            birdnet_db::sqlite::set_species_threshold(c, "Turdus merula", 0.5).unwrap();
        });
        // Age the cache past its TTL rather than sleeping for it.
        cache.refreshed = std::time::Instant::now()
            .checked_sub(ThresholdCache::TTL)
            .expect("clock is well past the TTL");

        let fresh = cache.current(&state);
        assert_eq!(
            fresh.get("Turdus merula"),
            Some(&0.5),
            "a threshold set at runtime must apply without a restart"
        );
    }

    #[test]
    fn threshold_cache_keeps_the_previous_map_when_a_refresh_fails() {
        // Falling back to an empty map on a transient read failure would
        // silently accept the detections the operator asked to scrutinise.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let mut cache = ThresholdCache::new(thresholds(&[("Pica pica", 0.9)]));

        // Break the table out from under the cache, then force a refresh.
        state.with_db(|c| {
            c.execute("DROP TABLE species_thresholds", []).unwrap();
        });
        cache.refreshed = std::time::Instant::now()
            .checked_sub(ThresholdCache::TTL)
            .expect("clock is well past the TTL");

        assert_eq!(
            cache.current(&state).get("Pica pica"),
            Some(&0.9),
            "a failed refresh must not drop the thresholds already in force"
        );
    }

    // ── record_notification ────────────────────────────────────────────

    /// One notification outcome must actually reach `notification_log`.
    ///
    /// `tests/notification_log_is_written.rs` guards the defect this function
    /// was written for — production code calling the writer at all — by reading
    /// source text, and it says a behavioural test "cannot catch it" because
    /// such a test would seed the row itself. That is true of *that* defect and
    /// not of this one: a source scan cannot tell whether the body still does
    /// anything, so `replace record_notification with ()` survived it. Mutation
    /// testing found exactly that, on `src/daemon/processor.rs:118`.
    ///
    /// This test seeds nothing. It calls the function and asks the table.
    #[test]
    fn record_notification_writes_a_row() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let before = state
            .with_db(|c| birdnet_db::notifications::recent_notifications(c, 10, 0))
            .expect("read the empty log");
        assert!(before.is_empty(), "precondition: nothing logged yet");

        let subject = NotificationSubject {
            com_name: "Eurasian Blackbird".to_owned(),
            sci_name: "Turdus merula".to_owned(),
            confidence: 0.91,
            date: "2026-03-16".to_owned(),
            time: "06:30:00".to_owned(),
        };
        record_notification(
            &state,
            "apprise",
            &subject,
            NotifStatus::Sent,
            Some("delivered"),
            None,
        );

        let after = state
            .with_db(|c| birdnet_db::notifications::recent_notifications(c, 10, 0))
            .expect("read the log");
        assert_eq!(
            after.len(),
            1,
            "the notification outcome never reached notification_log — the three \
             surfaces that read this table would stay empty on a real station"
        );
    }

    /// The counterpart: a failure outcome is recorded too, and carries its
    /// error. A writer that only logged successes would leave an operator
    /// asking "did my uploads land?" with a log that answers yes and nothing
    /// else — which is the question this table exists for.
    #[test]
    fn record_notification_records_a_failure_with_its_error() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let subject = NotificationSubject {
            com_name: "Great Tit".to_owned(),
            sci_name: "Parus major".to_owned(),
            confidence: 0.77,
            date: "2026-03-16".to_owned(),
            time: "07:00:00".to_owned(),
        };
        record_notification(
            &state,
            "birdweather",
            &subject,
            NotifStatus::Failed,
            None,
            Some("connection refused"),
        );

        let rows = state
            .with_db(|c| birdnet_db::notifications::recent_notifications(c, 10, 0))
            .expect("read the log");
        assert_eq!(rows.len(), 1, "a failed delivery must still be recorded");
        let row = &rows[0];
        assert_eq!(row.channel, "birdweather");
        assert_eq!(
            row.error.as_deref(),
            Some("connection refused"),
            "the reason is the whole value of the row: {row:?}"
        );
    }

    /// Build an Apprise handle whose sends either fail for real or are never
    /// attempted at all.
    ///
    /// `destination` = `Some(addr)` gives one native `json://` route, so a send
    /// is genuinely tried and fails; `None` gives a client with nowhere to
    /// send, which returns `NoDestinations` without touching the network.
    fn apprise_that(destination: Option<&str>) -> crate::integrations::AppriseHandle {
        let routes = destination.map_or_else(Vec::new, |addr| {
            let target = birdnet_integrations::dispatch::parse(&format!("json://{addr}/hook"))
                .expect("a json:// route parses");
            vec![birdnet_integrations::dispatch::Route {
                target,
                label: "json".to_owned(),
            }]
        });
        let client = birdnet_integrations::apprise::Client::new_cli_only(
            std::path::PathBuf::from("/nonexistent"),
            birdnet_integrations::apprise::NotifyConfig::default(),
        )
        .expect("client")
        .with_native_routes(routes, false);
        std::sync::Arc::new(tokio::sync::Mutex::new(client))
    }

    /// Rows on the `apprise` channel, waited for rather than sampled.
    ///
    /// The notification is dispatched from a task `event_processor` spawns and
    /// detaches, so it can still be in flight when `event_processor` returns.
    /// Polling to a deadline is what makes this test about the decision under
    /// test rather than about scheduling.
    async fn apprise_rows_within(
        state: &birdnet_web::state::AppState,
        deadline: std::time::Duration,
    ) -> usize {
        let start = std::time::Instant::now();
        loop {
            let rows = state
                .with_db(|conn| birdnet_db::notifications::recent_notifications(conn, 100, 0))
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.channel == "apprise")
                .count();
            if rows > 0 || start.elapsed() >= deadline {
                return rows;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Run one accepted detection through `event_processor` with `apprise`
    /// attached, and return how many `apprise` rows it logged.
    async fn apprise_rows_for_one_detection(apprise: crate::integrations::AppriseHandle) -> usize {
        use birdnet_core::audio::extraction::ExtractionConfig;
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;

        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let broadcast = state.detection_broadcast();

        let (event_tx, event_rx) = mpsc::channel::<DetectionEvent>();
        event_tx
            .send(DetectionEvent {
                detection: Detection {
                    date: "2026-05-19".into(),
                    time: "09:00:00".into(),
                    scientific_name: "Pica pica".into(),
                    common_name: "Eurasian Magpie".into(),
                    // Above NotifyConfig::default()'s 0.8 floor, so the
                    // notification is genuinely dispatched.
                    confidence: 0.95,
                    start: 0.0,
                    stop: 3.0,
                    week: 20,
                    file_name_extr: None,
                },
                source_file: tmp.path().join("nonexistent.wav"),
                latency_ms: 100,
                correlation_id: "test-corr-notify".into(),
            })
            .unwrap();
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let rt_handle = tokio::runtime::Handle::current();
        let state_for_processor = state.clone();
        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                Some(apprise),
                None,
                None,
                None,
                filter,
                birdnet_integrations::notification::NotificationTemplate::default(),
                rt_handle,
                HashMap::new(),
                0.25,
                Extractor::new(ExtractionConfig::default()),
                0,
                crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
                birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                    birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
                ),
            );
        })
        .await
        .unwrap();

        apprise_rows_within(&state, std::time::Duration::from_secs(20)).await
    }

    #[tokio::test]
    async fn a_notification_that_failed_on_the_wire_is_logged() {
        // Nothing was listening on port 1, so a destination was tried and the
        // send genuinely failed. That is a delivery attempt, and the
        // Notification Center exists to show it.
        let rows = apprise_rows_for_one_detection(apprise_that(Some("127.0.0.1:1"))).await;
        assert_eq!(rows, 1, "a real failed attempt must leave one row");
    }

    #[tokio::test]
    async fn a_notification_no_destination_was_tried_for_is_not_logged() {
        // The counterpart, and the discrimination `cargo-mutants` found
        // untested: deleting the `!` from `if !e.nothing_was_attempted()`
        // inverts these two outcomes, and until this pair existed nothing
        // noticed. On a station whose Apprise routes are all rate-limited or
        // circuit-open, that inversion writes a `Failed` row per detection —
        // thousands a day, burying the sends that actually happened — while
        // the genuine failures stop being recorded at all.
        let rows = apprise_rows_for_one_detection(apprise_that(None)).await;
        assert_eq!(
            rows, 0,
            "a notification no destination was tried for is counted in \
             birdnet_notifications_dropped_total, not logged as an attempt"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_inserts_row_for_accepted_event() {
        use birdnet_core::audio::extraction::ExtractionConfig;
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("birds.db");
        let state = birdnet_web::state::AppState::new(db_path).unwrap();
        let broadcast = state.detection_broadcast();

        // Send exactly one event, then close the channel.
        let (event_tx, event_rx) = mpsc::channel::<DetectionEvent>();
        let detection = Detection {
            date: "2026-05-19".into(),
            time: "09:00:00".into(),
            scientific_name: "Pica pica".into(),
            common_name: "Eurasian Magpie".into(),
            confidence: 0.95,
            start: 0.0,
            stop: 3.0,
            week: 20,
            file_name_extr: None,
        };
        event_tx
            .send(DetectionEvent {
                detection,
                source_file: tmp.path().join("nonexistent.wav"),
                latency_ms: 100,
                correlation_id: "test-corr-abc".into(),
            })
            .unwrap();
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let template = birdnet_integrations::notification::NotificationTemplate::default();
        let extractor = Extractor::new(ExtractionConfig::default());
        let rt_handle = tokio::runtime::Handle::current();

        let state_for_processor = state.clone();
        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                None,
                None,
                None,
                None,
                filter,
                template,
                rt_handle,
                HashMap::new(),
                0.25,
                extractor,
                0,
                crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
                birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                    birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
                ),
            );
        })
        .await
        .unwrap();

        let count = state.with_db(|conn| birdnet_db::sqlite::detection_count(conn).unwrap_or(0));
        assert_eq!(
            count, 1,
            "event_processor must persist accepted events; got {count} rows"
        );

        // And the correlation id round-trips.
        let recent = state
            .with_db(|conn| birdnet_db::sqlite::recent_detections(conn, 1).unwrap_or_default());
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].correlation_id.as_deref(), Some("test-corr-abc"));
    }

    /// Drive one detection through the real `event_processor` and report what
    /// landed where.
    ///
    /// Returns `(rows in detections, rows in quarantine, quarantine reason)`.
    async fn run_one_dated(date: &str) -> (i64, i64, Option<String>) {
        use birdnet_core::audio::extraction::ExtractionConfig;
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;

        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let broadcast = state.detection_broadcast();

        let (event_tx, event_rx) = mpsc::channel::<DetectionEvent>();
        event_tx
            .send(DetectionEvent {
                detection: Detection {
                    date: date.into(),
                    time: "09:00:00".into(),
                    scientific_name: "Pica pica".into(),
                    common_name: "Eurasian Magpie".into(),
                    confidence: 0.95,
                    start: 0.0,
                    stop: 3.0,
                    week: 19,
                    file_name_extr: None,
                },
                source_file: tmp.path().join("nonexistent.wav"),
                latency_ms: 100,
                correlation_id: "clock-gate".into(),
            })
            .unwrap();
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let state_for_processor = state.clone();
        let rt_handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                None,
                None,
                None,
                None,
                filter,
                birdnet_integrations::notification::NotificationTemplate::default(),
                rt_handle,
                HashMap::new(),
                0.25,
                Extractor::new(ExtractionConfig::default()),
                0,
                crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
                birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                    birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
                ),
            );
        })
        .await
        .unwrap();

        state.with_db(|conn| {
            let detections: i64 = conn
                .query_row("SELECT COUNT(*) FROM detections", [], |r| r.get(0))
                .unwrap_or(0);
            let quarantined: i64 = conn
                .query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0))
                .unwrap_or(0);
            let reason: Option<String> = conn
                .query_row("SELECT reason FROM quarantine LIMIT 1", [], |r| r.get(0))
                .ok();
            (detections, quarantined, reason)
        })
    }

    /// Drive one detection through the real `event_processor` with the ingest
    /// latch in the given state, and return the row count it left behind.
    ///
    /// Today's date, because the clock-plausibility gate (`NT-1`) must not be
    /// what decides this — `run_one_dated` uses fixed dates for the opposite
    /// reason.
    async fn detections_after_one_event(halted: bool) -> i64 {
        use birdnet_core::audio::extraction::ExtractionConfig;
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;

        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        if halted {
            state
                .ingest_halt_flag()
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        let broadcast = state.detection_broadcast();

        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();
        let civil = birdnet_core::civil::civil_from_unix_secs(
            now + birdnet_db::clock::local_utc_offset_secs(),
        );
        let date = format!("{:04}-{:02}-{:02}", civil.year, civil.month, civil.day);

        let (event_tx, event_rx) = mpsc::channel::<DetectionEvent>();
        event_tx
            .send(DetectionEvent {
                detection: Detection {
                    date,
                    time: "09:00:00".into(),
                    scientific_name: "Pica pica".into(),
                    common_name: "Eurasian Magpie".into(),
                    confidence: 0.95,
                    start: 0.0,
                    stop: 3.0,
                    week: 19,
                    file_name_extr: None,
                },
                source_file: tmp.path().join("nonexistent.wav"),
                latency_ms: 100,
                correlation_id: "ps5-gate".into(),
            })
            .unwrap();
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let state_for_processor = state.clone();
        let rt_handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                None,
                None,
                None,
                None,
                filter,
                birdnet_integrations::notification::NotificationTemplate::default(),
                rt_handle,
                HashMap::new(),
                0.25,
                Extractor::new(ExtractionConfig::default()),
                0,
                crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
                birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                    birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
                ),
            );
        })
        .await
        .unwrap();

        state.with_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM detections", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(-1)
        })
    }

    /// The control for the gate below. Without it, "no row was written" proves
    /// nothing about the halt.
    #[tokio::test]
    async fn a_healthy_station_records_the_detection() {
        assert_eq!(
            detections_after_one_event(false).await,
            1,
            "the fixture must record a detection when nothing is halted, or the \
             gate below is measuring a broken pipeline rather than the halt"
        );
    }

    /// `PS-5`. A station whose database failed its daily integrity check must
    /// record nothing further into it.
    ///
    /// The shipped behaviour was to log one `error!` and keep inserting, for
    /// months, while `backup_database` — which refuses a corrupt source —
    /// quietly stopped producing restore points. See
    /// `crates/birdnet-web/tests/a_corrupt_database_stops_taking_detections.rs`
    /// for the other half: that an *administrative* write still succeeds, which
    /// is what makes this narrower than a read-only connection.
    #[tokio::test]
    async fn a_halted_station_records_nothing() {
        assert_eq!(
            detections_after_one_event(true).await,
            0,
            "the detection was written to a database already known to be \
             corrupt — which is PS-5: the daily check found it, said so once, \
             and changed nothing"
        );
    }

    /// A detection recorded before the clock was set must never reach the
    /// history.
    ///
    /// A Raspberry Pi has no battery-backed RTC. Before NTP lands it reads the
    /// epoch, the capture tee stamps that into the segment filename, and the
    /// detection's `Date` and `Time` are parsed straight back out of it.
    /// Nothing checked, so such a row was stored as `1970-01-01` — permanently.
    /// `species_summary` files it under hour 00 for ever; `MIN(Date)` makes
    /// every species touched in that window "first seen 1970"; the history
    /// calendar acquires a 56-year span. Retention then reclaims its audio for
    /// being older than any cutoff, so the evidence goes and the row stays.
    ///
    /// Observed failing before the gate existed: `1970-01-01` produced
    /// `detections = 1, quarantine = 0`.
    #[tokio::test]
    async fn a_detection_from_before_the_clock_was_set_is_quarantined_not_filed() {
        let (detections, quarantined, reason) = run_one_dated("1970-01-01").await;
        assert_eq!(
            detections, 0,
            "a 1970-dated detection must not reach the history"
        );
        assert_eq!(
            quarantined, 1,
            "it must be quarantined rather than dropped: something was heard, \
             and the operator should see the station was recording blind"
        );
        assert_eq!(
            reason.as_deref(),
            Some("implausible_clock"),
            "the reason is the whole value of the row — an operator reviewing \
             the queue needs to know it was the clock and not the bird"
        );
    }

    /// The discrimination. A gate that quarantined everything would satisfy the
    /// test above and stop the station recording anything at all.
    #[tokio::test]
    async fn an_ordinary_detection_still_reaches_the_history() {
        let (detections, quarantined, _) = run_one_dated("2026-05-19").await;
        assert_eq!(detections, 1, "a real date must still be filed");
        assert_eq!(quarantined, 0, "and must not be quarantined");
    }

    #[tokio::test]
    async fn event_processor_persists_saved_clip_duration() {
        // The processor probes the SAVED CLIP's length and persists it
        // (migration 20) so the Recordings grid shows a real duration. Here the
        // 2-second source clamps the extracted clip to ~2.0 s, so it must
        // round-trip to ~2.0 s — this also kills a mutant that drops the probe
        // (the duration would then read back NULL).
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("clip.wav");
        {
            use hound::{SampleFormat, WavSpec, WavWriter};
            let spec = WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut w = WavWriter::create(&wav, spec).unwrap();
            for _ in 0..96_000 {
                w.write_sample(0_i16).unwrap(); // 96_000 / 48_000 = 2.0 s
            }
            w.finalize().unwrap();
        }

        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let ev = make_event("Pica pica", "Eurasian Magpie", 0.95, wav, "test-dur");
        run_processor(&state, vec![ev], HashMap::new(), 0.25).await;

        let recent = state
            .with_db(|conn| birdnet_db::sqlite::recent_detections(conn, 1).unwrap_or_default());
        assert_eq!(recent.len(), 1);
        let dur = recent[0]
            .duration_secs
            .expect("the saved clip's duration is persisted");
        assert!((dur - 2.0).abs() < 1e-3, "expected ~2.0 s, got {dur}");
    }

    // ── event_processor: disposition + alert-rule branches ──────────────
    //
    // The accepted-event test above walks the happy path. These extend the
    // same in-process harness to the branches that the carryover (item A3)
    // flagged as un-exercised: the quarantine path, the silent-drop path,
    // and the three alert-rule actions (Log / Suppress / Webhook). All
    // external integrations stay disabled (None) — these tests target the
    // DB / disposition / alert-rule logic, not the network sinks.

    /// Build a `DetectionEvent` with the given species and confidence.
    fn make_event(
        sci: &str,
        com: &str,
        confidence: f32,
        source_file: PathBuf,
        correlation_id: &str,
    ) -> birdnet_core::detection::daemon::DetectionEvent {
        use birdnet_core::detection::daemon::DetectionEvent;
        use birdnet_core::detection::types::Detection;
        DetectionEvent {
            detection: Detection {
                date: "2026-05-19".into(),
                time: "09:00:00".into(),
                scientific_name: sci.into(),
                common_name: com.into(),
                confidence,
                start: 0.0,
                stop: 3.0,
                week: 20,
                file_name_extr: None,
            },
            source_file,
            latency_ms: 100,
            correlation_id: correlation_id.into(),
        }
    }

    /// Drive `event_processor` to completion over `events` against the
    /// caller-supplied `state` (so a test can pre-seed alert rules), with
    /// every external integration disabled. Returns once the channel is
    /// drained and the processor's loop exits, so DB assertions are
    /// observed after all synchronous work has run.
    /// Drive the processor with the duplicate interval and daylight filter off.
    async fn run_processor(
        state: &birdnet_web::state::AppState,
        events: Vec<birdnet_core::detection::daemon::DetectionEvent>,
        species_thresholds: HashMap<String, f64>,
        global_confidence: f32,
    ) {
        run_processor_full(
            state,
            events,
            species_thresholds,
            global_confidence,
            0,
            crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
        )
        .await;
    }

    /// Drive the processor, suppressing repeats within `duplicate_interval_secs`.
    async fn run_processor_with_interval(
        state: &birdnet_web::state::AppState,
        events: Vec<birdnet_core::detection::daemon::DetectionEvent>,
        species_thresholds: HashMap<String, f64>,
        global_confidence: f32,
        duplicate_interval_secs: i64,
    ) {
        run_processor_full(
            state,
            events,
            species_thresholds,
            global_confidence,
            duplicate_interval_secs,
            crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
        )
        .await;
    }

    /// Drive the processor with every gate under the test's control.
    async fn run_processor_full(
        state: &birdnet_web::state::AppState,
        events: Vec<birdnet_core::detection::daemon::DetectionEvent>,
        species_thresholds: HashMap<String, f64>,
        global_confidence: f32,
        duplicate_interval_secs: i64,
        daylight: crate::daemon::daylight::DaylightFilter,
    ) {
        run_processor_dynamic(
            state,
            events,
            species_thresholds,
            global_confidence,
            duplicate_interval_secs,
            daylight,
            birdnet_core::detection::dynamic_threshold::DynamicThresholds::new(
                birdnet_core::detection::dynamic_threshold::DynamicThresholdConfig::default(),
            ),
        )
        .await;
    }

    /// As [`run_processor_full`], with the learned-threshold tracker supplied.
    #[allow(clippy::too_many_arguments)]
    async fn run_processor_dynamic(
        state: &birdnet_web::state::AppState,
        events: Vec<birdnet_core::detection::daemon::DetectionEvent>,
        species_thresholds: HashMap<String, f64>,
        global_confidence: f32,
        duplicate_interval_secs: i64,
        daylight: crate::daemon::daylight::DaylightFilter,
        dynamic: birdnet_core::detection::dynamic_threshold::DynamicThresholds,
    ) {
        let broadcast = state.detection_broadcast();
        let (event_tx, event_rx) = mpsc::channel();
        for ev in events {
            event_tx.send(ev).unwrap();
        }
        drop(event_tx);

        let filter = birdnet_integrations::notification::NotificationFilter {
            trigger: birdnet_integrations::notification::TriggerMode::EachDetection,
            species_filter: birdnet_integrations::notification::SpeciesFilter::new(None, None),
        };
        let template = birdnet_integrations::notification::NotificationTemplate::default();
        let extractor = Extractor::new(ExtractionConfig::default());
        let rt_handle = tokio::runtime::Handle::current();
        let state_for_processor = state.clone();

        tokio::task::spawn_blocking(move || {
            super::event_processor(
                event_rx,
                state_for_processor,
                broadcast,
                None,
                None,
                None,
                None,
                filter,
                template,
                rt_handle,
                species_thresholds,
                global_confidence,
                extractor,
                duplicate_interval_secs,
                daylight,
                dynamic,
            );
        })
        .await
        .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_quarantines_below_per_species_threshold() {
        // global=0.25, per-species "Pica pica"=0.80, detection=0.60: passes
        // the global gate but fails the stricter per-species threshold, so
        // the event is quarantined for review rather than stored or dropped.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let ev = make_event(
            "Pica pica",
            "Eurasian Magpie",
            0.60,
            tmp.path().join("rec.wav"),
            "corr-quarantine",
        );

        run_processor(&state, vec![ev], thresholds(&[("Pica pica", 0.80)]), 0.25).await;

        let detections = state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1));
        assert_eq!(
            detections, 0,
            "a detection below its per-species threshold must not be stored as a detection"
        );
        let pending =
            state.with_db(|c| birdnet_db::sqlite::quarantine_pending_count(c).unwrap_or(-1));
        assert_eq!(
            pending, 1,
            "a detection below its per-species threshold must be quarantined for review"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_drops_below_global_threshold() {
        // global=0.25, no per-species override, detection=0.10: fails the
        // global gate and is dropped silently — neither stored nor
        // quarantined (quarantine is reserved for the per-species case).
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let ev = make_event(
            "Pica pica",
            "Eurasian Magpie",
            0.10,
            tmp.path().join("rec.wav"),
            "corr-drop",
        );

        run_processor(&state, vec![ev], HashMap::new(), 0.25).await;

        let detections = state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1));
        assert_eq!(detections, 0, "below-global detections must not be stored");
        let pending =
            state.with_db(|c| birdnet_db::sqlite::quarantine_pending_count(c).unwrap_or(-1));
        assert_eq!(
            pending, 0,
            "below-global with no per-species override must not quarantine"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_runs_every_alert_rule_action() {
        // Seed one rule per action type — Log, Suppress, Webhook — each
        // matching any detection, then send one accepted event. This walks
        // all three arms of the alert-rule match loop. The accepted event is
        // still stored: a Suppress rule gates *notifications*, not storage.
        use birdnet_db::alert_rules::{AlertAction, NewAlertRule, insert_rule};

        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let match_all = |name: &str, action: AlertAction| NewAlertRule {
            name: name.to_owned(),
            enabled: true,
            species_pattern: None,
            confidence_min: 0.0,
            confidence_max: 1.0,
            hour_start: None,
            hour_end: None,
            days_of_week: None,
            action,
        };

        state.with_db(|c| {
            insert_rule(c, &match_all("log-all", AlertAction::Log)).unwrap();
            insert_rule(c, &match_all("suppress-all", AlertAction::Suppress)).unwrap();
            insert_rule(
                c,
                &match_all(
                    "hook-all",
                    AlertAction::Webhook {
                        // RFC 5737 TEST-NET-2: the spawned dispatch can never
                        // connect, but the test does not await it — the
                        // runtime is dropped at end-of-test, aborting the task.
                        url: "http://198.51.100.1:1/".to_owned(),
                        method: "POST".to_owned(),
                        body_template: Some("{\"species\":\"{{species}}\"}".to_owned()),
                        auth: None,
                    },
                ),
            )
            .unwrap();
        });

        let ev = make_event(
            "Pica pica",
            "Eurasian Magpie",
            0.95,
            tmp.path().join("rec.wav"),
            "corr-rules",
        );
        run_processor(&state, vec![ev], HashMap::new(), 0.25).await;

        let detections = state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1));
        assert_eq!(
            detections, 1,
            "an accepted detection is stored even when a Suppress alert rule fires"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn event_processor_processes_multiple_events_in_one_run() {
        // Two accepted events in one channel drain: confirms the loop
        // re-enters and both rows land. Catches a mutant that would `break`
        // after the first event instead of looping.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let events = vec![
            make_event(
                "Pica pica",
                "Eurasian Magpie",
                0.95,
                tmp.path().join("a.wav"),
                "corr-a",
            ),
            make_event(
                "Corvus corax",
                "Northern Raven",
                0.90,
                tmp.path().join("b.wav"),
                "corr-b",
            ),
        ];

        run_processor(&state, events, HashMap::new(), 0.25).await;

        let detections = state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1));
        assert_eq!(detections, 2, "both accepted events must be stored");
    }

    // ── the duplicate-prediction interval, through the processor ────────

    /// An event for `sci` at `hms` on a fixed date.
    fn event_at(
        sci: &str,
        hms: &str,
        dir: &std::path::Path,
    ) -> birdnet_core::detection::daemon::DetectionEvent {
        let mut ev = make_event(sci, sci, 0.95, dir.join("rec.wav"), "corr-dup");
        ev.detection.time = hms.to_owned();
        ev
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_duplicate_interval_collapses_one_song_into_one_row() {
        // Through the real processor, not the gate in isolation: this is what
        // says the gate is wired in at all, and wired in on the accept path
        // rather than somewhere the detection had already been dropped.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let events = ["05:00:00", "05:00:03", "05:00:06", "05:00:09", "05:00:12"]
            .iter()
            .map(|hms| event_at("Turdus merula", hms, tmp.path()))
            .collect();
        run_processor_with_interval(&state, events, HashMap::new(), 0.25, 30).await;

        let stored = state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1));
        assert_eq!(
            stored, 1,
            "five chunks of one song were recorded as {stored} detections"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn without_the_interval_every_chunk_is_still_recorded() {
        // Counterpart, and the guarantee that matters on upgrade: the feature
        // is off by default and a station that has not asked for it must keep
        // recording exactly what it recorded before.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let events = ["05:00:00", "05:00:03", "05:00:06", "05:00:09", "05:00:12"]
            .iter()
            .map(|hms| event_at("Turdus merula", hms, tmp.path()))
            .collect();
        run_processor_with_interval(&state, events, HashMap::new(), 0.25, 0).await;

        assert_eq!(
            state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1)),
            5
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_interval_does_not_shadow_a_different_species() {
        // A dawn chorus is many species at once; a gate keyed on anything
        // coarser than the species would record one bird and drop the chorus.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        let events = vec![
            event_at("Turdus merula", "05:00:00", tmp.path()),
            event_at("Parus major", "05:00:01", tmp.path()),
            event_at("Erithacus rubecula", "05:00:02", tmp.path()),
            event_at("Turdus merula", "05:00:03", tmp.path()),
        ];
        run_processor_with_interval(&state, events, HashMap::new(), 0.25, 30).await;

        assert_eq!(
            state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1)),
            3,
            "the chorus was collapsed to the first bird heard"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_quarantined_detection_does_not_consume_the_interval() {
        // The gate runs after the threshold gates for this reason: a run of
        // low-confidence chunks must not shadow the confident one among them.
        // With the order reversed the first chunk claims the interval, is then
        // quarantined, and the good detection in the middle of the song is
        // dropped as its duplicate — leaving no row at all.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();
        let thresholds = HashMap::from([("Turdus merula".to_owned(), 0.90_f64)]);

        let mut weak = event_at("Turdus merula", "05:00:00", tmp.path());
        weak.detection.confidence = 0.50;
        let strong = event_at("Turdus merula", "05:00:03", tmp.path());

        run_processor_with_interval(&state, vec![weak, strong], thresholds, 0.25, 30).await;

        assert_eq!(
            state.with_db(|c| birdnet_db::sqlite::detection_count(c).unwrap_or(-1)),
            1,
            "the quarantined chunk consumed the interval and the real \
             detection was dropped as its duplicate"
        );
    }

    // ── the taxon-aware daylight filter, through the processor ──────────

    /// A daylight filter at Greenwich, UTC, with an hour of margin.
    fn greenwich_night() -> crate::daemon::daylight::DaylightFilter {
        crate::daemon::daylight::DaylightFilter::new(
            Some(birdnet_scheduler::solar::Location::new_unchecked(
                51.48, 0.0,
            )),
            60,
            0,
            Vec::new(),
        )
    }

    /// An event for `sci` at `hms` on 15 January 2026.
    fn winter_event(
        sci: &str,
        hms: &str,
        dir: &std::path::Path,
    ) -> birdnet_core::detection::daemon::DetectionEvent {
        let mut ev = make_event(sci, sci, 0.95, dir.join("rec.wav"), "corr-night");
        ev.detection.date = "2026-01-15".into();
        ev.detection.time = hms.to_owned();
        ev
    }

    /// How many rows are in `detections` and `quarantine`.
    fn counts(state: &birdnet_web::state::AppState) -> (i64, i64) {
        state.with_db(|c| {
            (
                birdnet_db::sqlite::detection_count(c).unwrap_or(-1),
                c.query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0))
                    .unwrap_or(-1),
            )
        })
    }

    /// An enabled tracker.
    fn dynamic_on() -> birdnet_core::detection::dynamic_threshold::DynamicThresholds {
        use birdnet_core::detection::dynamic_threshold::{
            DynamicThresholdConfig, DynamicThresholds,
        };
        DynamicThresholds::new(DynamicThresholdConfig {
            enabled: true,
            trigger: 0.80,
            min: 0.10,
            valid_hours: 24,
        })
    }

    /// The learned levels the station has persisted.
    fn learned(state: &birdnet_web::state::AppState) -> Vec<(String, u8)> {
        state
            .with_db(birdnet_db::dynamic_thresholds::load_all)
            .unwrap_or_default()
            .into_iter()
            .map(|r| (r.sci_name, r.level))
            .collect()
    }

    /// A recorded detection confirms its species, and the confirmation is
    /// persisted.
    ///
    /// This is the wiring gate: the tracker works and the table works, both
    /// tested where they live. The failure this catches is that nothing calls
    /// one from the other — every unit test in both files passes in that
    /// state.
    #[tokio::test(flavor = "current_thread")]
    async fn a_recorded_detection_confirms_its_species() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_dynamic(
            &state,
            vec![make_event(
                "Strix aluco",
                "Tawny Owl",
                0.95,
                tmp.path().join("rec.wav"),
                "corr-dyn",
            )],
            HashMap::new(),
            0.25,
            0,
            crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
            dynamic_on(),
        )
        .await;

        assert_eq!(
            learned(&state),
            vec![("Strix aluco".to_string(), 1)],
            "an accepted detection at 0.95 should have confirmed its species once"
        );
    }

    /// A quarantined detection confirms nothing.
    ///
    /// The safety property the whole feature rests on, and the reason
    /// `confirm` is called after the insert rather than after the model. A
    /// detection the station declined to record has not established that its
    /// species is present, and letting it lower its own threshold would turn
    /// one false positive into a stream of them.
    #[tokio::test(flavor = "current_thread")]
    async fn a_quarantined_detection_confirms_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        // A per-species threshold of 0.99 quarantines a 0.95 detection, which
        // is still well above the tracker's 0.80 trigger — so a version that
        // confirmed before the gate would learn from it.
        let mut thresholds = HashMap::new();
        thresholds.insert("Strix aluco".to_string(), 0.99);

        run_processor_dynamic(
            &state,
            vec![make_event(
                "Strix aluco",
                "Tawny Owl",
                0.95,
                tmp.path().join("rec.wav"),
                "corr-dyn-q",
            )],
            thresholds,
            0.25,
            0,
            crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
            dynamic_on(),
        )
        .await;

        let (detections, quarantined) = counts(&state);
        assert_eq!(detections, 0, "the detection should have been quarantined");
        assert_eq!(quarantined, 1);
        assert!(
            learned(&state).is_empty(),
            "a quarantined detection must not confirm its species: {:?}",
            learned(&state)
        );
    }

    /// A day bird quarantined for the hour confirms nothing either.
    ///
    /// The counterpart with a different rejection reason, because the
    /// plausible-hour filter takes its own `continue` well before the
    /// threshold gate — a `confirm` placed anywhere in between would pass the
    /// test above and fail here.
    #[tokio::test(flavor = "current_thread")]
    async fn a_detection_rejected_for_its_hour_confirms_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_dynamic(
            &state,
            vec![winter_event("Cyanistes caeruleus", "02:30:00", tmp.path())],
            HashMap::new(),
            0.25,
            0,
            greenwich_night(),
            dynamic_on(),
        )
        .await;

        let (detections, quarantined) = counts(&state);
        assert_eq!(detections, 0);
        assert_eq!(
            quarantined, 1,
            "the blue tit at 02:30 should be quarantined"
        );
        assert!(
            learned(&state).is_empty(),
            "a detection quarantined for its hour must not confirm its species"
        );
    }

    /// With the tracker disabled nothing is learned and nothing is written.
    #[tokio::test(flavor = "current_thread")]
    async fn a_disabled_tracker_learns_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor(
            &state,
            vec![make_event(
                "Strix aluco",
                "Tawny Owl",
                0.95,
                tmp.path().join("rec.wav"),
                "corr-dyn-off",
            )],
            HashMap::new(),
            0.25,
        )
        .await;

        assert_eq!(
            counts(&state).0,
            1,
            "the detection should still be recorded"
        );
        assert!(learned(&state).is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_day_bird_at_two_in_the_morning_is_quarantined_not_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_full(
            &state,
            vec![winter_event("Cyanistes caeruleus", "02:30:00", tmp.path())],
            HashMap::new(),
            0.25,
            0,
            greenwich_night(),
        )
        .await;

        let (detections, quarantined) = counts(&state);
        assert_eq!(detections, 0, "a blue tit at 02:30 was recorded as fact");
        assert_eq!(quarantined, 1, "and it was not preserved for review either");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_owl_at_two_in_the_morning_is_recorded_normally() {
        // The taxon half, end to end. Without it this is a blanket night
        // filter, and it quarantines the detections an operator most wants.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_full(
            &state,
            vec![winter_event("Strix aluco", "02:30:00", tmp.path())],
            HashMap::new(),
            0.25,
            0,
            greenwich_night(),
        )
        .await;

        assert_eq!(
            counts(&state),
            (1, 0),
            "a tawny owl was quarantined for hooting at night"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_same_day_bird_in_the_afternoon_is_recorded_normally() {
        // Counterpart: a filter that quarantined regardless of hour passes the
        // first gate and silently empties the station.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_full(
            &state,
            vec![winter_event("Cyanistes caeruleus", "13:00:00", tmp.path())],
            HashMap::new(),
            0.25,
            0,
            greenwich_night(),
        )
        .await;

        assert_eq!(counts(&state), (1, 0));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_night_quarantine_says_it_was_the_hour_not_the_confidence() {
        // The reason is what the operator reads in the review queue, and the
        // hour is the fact that decides whether to believe the detection. A
        // 0.95 detection filed as "below species threshold" would be actively
        // misleading.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_full(
            &state,
            vec![winter_event("Cyanistes caeruleus", "02:30:00", tmp.path())],
            // A per-species threshold this detection also misses, so the two
            // gates would both fire and the order decides which reason is
            // recorded.
            HashMap::from([("Cyanistes caeruleus".to_owned(), 0.99_f64)]),
            0.25,
            0,
            greenwich_night(),
        )
        .await;

        let reason: String = state.with_db(|c| {
            c.query_row("SELECT reason FROM quarantine", [], |r| r.get(0))
                .unwrap_or_default()
        });
        assert_eq!(reason, "implausible_hour", "filed under the wrong reason");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_the_filter_off_a_day_bird_at_night_is_still_recorded() {
        // The upgrade guarantee: off by default, and a station that has not
        // asked for this keeps recording exactly what it recorded before.
        let tmp = tempfile::tempdir().unwrap();
        let state = birdnet_web::state::AppState::new(tmp.path().join("birds.db")).unwrap();

        run_processor_full(
            &state,
            vec![winter_event("Cyanistes caeruleus", "02:30:00", tmp.path())],
            HashMap::new(),
            0.25,
            0,
            crate::daemon::daylight::DaylightFilter::new(None, 60, 0, Vec::new()),
        )
        .await;

        assert_eq!(counts(&state), (1, 0));
    }
}
