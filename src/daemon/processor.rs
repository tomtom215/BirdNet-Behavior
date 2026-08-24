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
use super::webhook::dispatch_webhook;
use birdnet_db::notifications::NotifStatus;

use crate::integrations::{AppriseHandle, EmailHandle, HeartbeatHandle, MqttHandle};

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
    heartbeat: Option<HeartbeatHandle>,
    mqtt: Option<MqttHandle>,
    notification_filter: NotificationFilter,
    notification_template: NotificationTemplate,
    rt_handle: tokio::runtime::Handle,
    species_thresholds: std::collections::HashMap<String, f64>,
    global_confidence: f32,
    extractor: Extractor,
) {
    tracing::debug!("event processor started");
    let mut species_thresholds = ThresholdCache::new(species_thresholds);

    loop {
        let Ok(event) = event_rx.recv() else {
            tracing::info!("event channel closed, stopping event processor");
            break;
        };
        let species_thresholds = species_thresholds.current(&state);

        let detection = &event.detection;
        let correlation_id = event.correlation_id.as_str();

        // Apply per-species confidence threshold.
        // Detections that pass the global threshold but fail a stricter
        // per-species threshold are quarantined for manual review rather
        // than silently dropped.
        match decide_disposition(
            detection.confidence,
            &detection.scientific_name,
            species_thresholds,
            global_confidence,
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
                if let Err(e) =
                    state.with_db(|conn| birdnet_db::sqlite::insert_quarantine(conn, &q_record))
                {
                    tracing::warn!(
                        correlation_id,
                        error = %e,
                        species = %detection.scientific_name,
                        "failed to quarantine detection"
                    );
                }
                continue;
            }
            DispositionDecision::DropBelowGlobal => continue,
            DispositionDecision::Accept => {}
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
        let insert_result =
            state.with_db(|conn| birdnet_db::sqlite::insert_detection(conn, &record));
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
                        let rule_name = rule.name.clone();
                        rt_handle.spawn(async move {
                            match dispatch_webhook(&url, &method, body.as_deref()).await {
                                Ok(status) => tracing::debug!(
                                    rule = %rule_name,
                                    url,
                                    status,
                                    "webhook dispatched"
                                ),
                                Err(e) => tracing::warn!(
                                    rule = %rule_name,
                                    url,
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
                    queue_state.with_db(|conn| {
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

        // Heartbeat ping after processing.
        if let Some(ref hb) = heartbeat {
            let hb = Arc::clone(hb);
            rt_handle.spawn(async move {
                if let Err(e) = hb.ping().await {
                    tracing::debug!(error = %e, "heartbeat ping failed");
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
                None,
                filter,
                template,
                rt_handle,
                HashMap::new(),
                0.25,
                extractor,
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
    async fn run_processor(
        state: &birdnet_web::state::AppState,
        events: Vec<birdnet_core::detection::daemon::DetectionEvent>,
        species_thresholds: HashMap<String, f64>,
        global_confidence: f32,
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
                None,
                filter,
                template,
                rt_handle,
                species_thresholds,
                global_confidence,
                extractor,
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
}
