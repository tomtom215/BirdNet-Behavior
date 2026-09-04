//! Server orchestration — everything the binary does once it has decided to
//! run the detection daemon + web server.
//!
//! Split out of `main.rs` so the entry point stays a thin, decision-only
//! shell. The decision (`dispatch_subcommand`) is pure and unit-tested; the
//! orchestration here is inherently I/O-bound — it binds a TCP listener,
//! spawns background tasks, and serves until a shutdown signal — so it can
//! only be exercised by the end-to-end / subprocess layers (ADR-16 layers
//! 3–4), never a unit test. Keeping the two apart is what lets `src/main.rs`
//! carry real unit-test coverage instead of being a 0 %-covered black hole.

use crate::cli::Cli;
use crate::{capture, daemon, helpers, integrations, maintenance, sd_notify, weekly_report};

/// Run the detection daemon and web server until a shutdown signal arrives.
///
/// Called from `main` only for [`crate::Action::RunServer`]; the maintenance
/// and doctor short-circuits are handled before this is reached. Takes
/// ownership of the parsed `cli` and the loaded `config` (already resolved by
/// `main`).
/// Run the detection daemon and web server until a shutdown signal arrives.
///
/// Called from `main` only for [`crate::Action::RunServer`]; the maintenance
/// and doctor short-circuits are handled before this is reached. Takes
/// ownership of the parsed `cli` and the loaded `config` (already resolved by
/// `main`).
pub async fn run(
    cli: Cli,
    config: Option<birdnet_core::config::Config>,
    log_broadcaster: birdnet_web::routes::admin::logs::LogBroadcaster,
) -> Result<(), Box<dyn std::error::Error>> {
    // Race ONLY the startup phases against an early SIGTERM/SIGINT. Without
    // this, a `systemctl restart` mid-startup — e.g. during a cold DuckDB
    // build, SQLite migration, or initial DuckDB sync — would be SIGKILLed at
    // systemd's `TimeoutStartSec` instead of exiting cleanly. The biased
    // select prefers the signal so a signal that arrives at the same tick as
    // a startup result wins, ensuring we always honour the shutdown intent.
    //
    // The race MUST end once `serve` hands off (the `started` arm below):
    // from that point the serve loop's `with_graceful_shutdown` owns signal
    // handling — it wakes live WebSocket/SSE clients, drains connections,
    // and stops the detection daemon so the runtime can wind down. Keeping
    // the outer listener racing past handoff made the biased arm win every
    // post-startup SIGTERM, cancel that choreography, and leave the runtime
    // blocked forever on the detection loop's still-running blocking thread
    // (observed live: "exiting cleanly" logged, process alive minutes later,
    // leaving systemd to SIGKILL at TimeoutStopSec).
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let mut serve_fut = std::pin::pin!(serve(cli, config, log_broadcaster, started_tx));

    tokio::select! {
        biased;
        () = shutdown_signal() => {
            tracing::info!("shutdown signal received during startup; exiting cleanly");
            sd_notify::stopping();
            return Ok(());
        }
        result = &mut serve_fut => return result,
        _ = started_rx => {}
    }

    // Startup is complete and the inner graceful-shutdown hook is already
    // polled (it registered its signal listeners before `serve` first
    // suspended after binding), so no signal can fall between the arms.
    serve_fut.await
}

/// Server orchestration body — runs from config validation through the axum
/// serve loop. Returns when the server stops. Wrapped by [`run`] so an early
/// SIGTERM during startup cancels this future and exits cleanly instead of
/// waiting out systemd's `TimeoutStartSec` and being `SIGKILL`-ed.
#[allow(clippy::too_many_lines)]
async fn serve(
    cli: Cli,
    config: Option<birdnet_core::config::Config>,
    log_broadcaster: birdnet_web::routes::admin::logs::LogBroadcaster,
    started: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fail fast on a misconfigured station: validate the loaded config and
    // refuse to start if any setting is outright invalid (e.g. a latitude
    // outside ±90 or a malformed recording schedule) rather than limping along
    // with a silently-degraded pipeline. Warnings are logged but non-fatal.
    // `--doctor` runs the same checks for an explicit preflight.
    if let Some(ref cfg) = config {
        use birdnet_core::config::validate::{Severity, is_usable, validate};
        let findings = validate(cfg);
        for f in &findings {
            match f.severity {
                Severity::Error => {
                    tracing::error!(key = %f.key, remediation = %f.remediation, "{}", f.message);
                }
                Severity::Warning => {
                    tracing::warn!(key = %f.key, remediation = %f.remediation, "{}", f.message);
                }
            }
        }
        if !is_usable(&findings) {
            let errors = findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count();
            return Err(format!(
                "configuration has {errors} error(s); fix the setting(s) logged above and restart \
                 (run with --doctor to re-check)"
            )
            .into());
        }
    }

    // Startup database resilience check.
    let db_path = helpers::db_path_from_config(config.as_ref());
    // SQLite will not create a missing parent directory — it fails the open
    // with a bare "unable to open database file" — so do it here, before
    // anything touches the path. Matches what every other directory the
    // station owns already does, and makes `--doctor`'s "will be created on
    // first run" true rather than aspirational.
    helpers::ensure_db_dir(&db_path)?;
    let backup_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("backups");

    if db_path.exists() {
        match birdnet_db::resilience::check_and_recover(&db_path, &backup_dir) {
            Ok(result) => {
                if result.action == birdnet_db::resilience::RecoveryAction::Recovered {
                    tracing::warn!(details = %result.details, "database recovered");
                } else {
                    tracing::info!(details = %result.details, "database healthy");
                }
            }
            Err(e) => {
                // Corrupt and unrecoverable (no good backup). Never write to a
                // corrupt database: quarantine it for offline recovery and
                // start fresh, so an unattended station keeps recording rather
                // than refusing to boot. Only refuse to start if we cannot even
                // move the corrupt file aside.
                tracing::error!(
                    error = %e,
                    path = %db_path.display(),
                    "database is corrupt and no good backup exists; quarantining before starting fresh"
                );
                let quarantined = birdnet_db::resilience::quarantine_corrupt_database(&db_path)
                    .map_err(|qe| {
                        format!(
                            "database is corrupt and could not be quarantined ({qe}); refusing to \
                             start to avoid writing to a corrupt database — restore a backup or move \
                             {} aside manually",
                            db_path.display()
                        )
                    })?;
                tracing::error!(
                    quarantined_to = %quarantined.display(),
                    "corrupt database quarantined; starting with a fresh database. Restore a backup \
                     or recover the quarantined file to keep historical detections"
                );
            }
        }
    }

    // Build app state.
    let addr: std::net::SocketAddr = cli.listen.parse()?;

    // Decide the TLS shape now, before any expensive setup: a station asked for
    // HTTPS that cannot have it must say so and stop, not quietly come up on
    // plain HTTP with the operator believing otherwise.
    let tls_plan = helpers::tls::plan(&cli, config.as_ref(), addr, &db_path)?;
    for warning in &tls_plan.warnings {
        tracing::warn!("{warning}");
    }
    let server_config = birdnet_web::server::ServerConfig {
        addr,
        db_path: db_path.clone(),
    };

    #[cfg(feature = "analytics")]
    let state = helpers::build_state_with_analytics(&cli, config.as_ref(), &server_config)?;

    #[cfg(not(feature = "analytics"))]
    let state = {
        if cli.analytics_db.is_some() {
            tracing::warn!(
                "DuckDB analytics requested but not compiled in. Rebuild with --features analytics"
            );
        }
        birdnet_web::state::AppState::new(server_config.db_path.clone())
            .map_err(|e| format!("database error: {e}"))?
    };

    // Thread the active config path so the in-UI diagnostics page can re-read
    // and validate it, and the broadcaster the `tracing` layer installed in
    // `main` is already writing to — without this the state holds the empty
    // one its constructor made, and `GET /admin/system/logs` streams
    // keep-alives for ever.
    let state = state
        .with_config_path(cli.config.clone())
        .with_log_broadcaster(log_broadcaster)
        // Decided once, here, and carried on the state: whether anything would
        // bring this process back if it exited. Both restart handlers read it
        // rather than the environment, so a test binary that inherited
        // `INVOCATION_ID` cannot reach the branch that signals.
        .with_supervised_by_systemd(
            birdnet_web::routes::admin::system_controls::supervised_by_systemd(),
        );

    // O-1: enable the mutating `/api/v2` endpoints when the operator has set a
    // token. Absent one — the default — those routes answer 404 and this
    // station has no write API at all.
    let state = match helpers::build_api_token(config.as_ref()) {
        Some(token) => state.with_api_token(token),
        None => state,
    };

    // O-14 / O-15 wire-flip prep: rotate the seed admin row's password
    // hash to a real argon2id digest of CADDY_PWD on first start (and
    // refresh it whenever the env var changes vs the stored hash). The
    // basic-auth surface stays functional throughout; this only makes
    // the cookie-auth path's user lookup line up.
    helpers::bootstrap_admin_password(&state, config.as_ref());
    // Upgrade path: clear plaintext credential rows a previous build's settings
    // form could write. Must run before the seed/overlay below so the purged
    // keys are gone before anything reads the table.
    helpers::purge_legacy_credential_settings(&state);

    // Seed the admin-UI `settings` table from the installed configuration on
    // first run, so the values supplied at install time — the bare-metal
    // installer's `birdnet.conf` and the Docker image's `BIRDNET_*` env/flags
    // alike — show up in, and are editable from, the web settings form, and a
    // configured station is not bounced back through the onboarding wizard.
    // Insert-only, so it never overwrites a setting the operator later changed
    // in the UI.
    helpers::seed_db_settings_from_config(config.as_ref(), &cli, &state);

    // Overlay the admin-UI settings (SQLite `settings` table) on top of the
    // file config so settings saved in the web UI actually take effect. Without
    // this the settings form is write-only: the daemon and capture subsystems
    // below read only the file config + CLI flags. The database value wins, and
    // changes apply on restart (as the settings page already states). Done here
    // — after the DB-backed state exists, before any subsystem reads config.
    let config = helpers::overlay_db_settings(config, &state);

    // Say once, at start, exactly what offline mode turned off — so the answer
    // to "does this station phone home?" is in the journal rather than in a
    // reading of the source.
    if let Some(notice) = helpers::egress::offline_notice(&cli) {
        tracing::info!("{notice}");
    }

    // Initialize all optional subsystems.
    let state = helpers::init_image_cache(state, &cli, config.as_ref(), &db_path);
    // The flag has no clap default, so `Some` means the operator supplied it;
    // otherwise take the config, which is where a path entered on
    // `/admin/settings` arrives via the overlay above.
    let custom_image_dir = cli.custom_image_dir.clone().or_else(|| {
        config
            .as_ref()
            .and_then(|c| c.get("CUSTOM_IMAGE_DIR"))
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(std::path::PathBuf::from)
    });
    let state = if let Some(dir) = custom_image_dir {
        tracing::info!(path = %dir.display(), "custom species image directory configured");
        state.with_custom_image_dir(dir)
    } else {
        state
    };
    let state = helpers::init_site_name(state, &cli, config.as_ref());
    let state = if cli.info_site == "ebird" {
        state
    } else {
        state.with_info_site(cli.info_site.clone())
    };
    let state = helpers::init_i18n(state, &cli, config.as_ref());

    // The capture supervisor publishes per-source health into this shared
    // handle; the web layer reads it for Station Health. One clone goes into
    // the (about-to-be-shared) AppState, the original into the supervisor
    // thread below.
    let capture_status = birdnet_core::audio::capture::new_capture_status();
    let state = state.with_capture_status(capture_status.clone());

    // Teed capture sources publish their live PCM here, and `/stream` reads it
    // instead of opening the audio device a second time — which an ALSA
    // microphone refuses with `Device or resource busy` while it is being
    // recorded.
    let live_audio = birdnet_core::audio::capture::new_live_audio_hub();
    let state = state.with_live_audio(std::sync::Arc::clone(&live_audio));

    // Captured before `state` is moved into the web server below; used by the
    // per-species recording-cap maintenance task.
    let recordings_dir_for_maintenance = state.recording_dir();
    // Likewise, and for PS-5: the maintenance loop that finds the corruption is
    // the one that has to stop the detection writes, so it holds a clone of the
    // latch the ingest path reads.
    let ingest_halt_for_maintenance = state.ingest_halt_flag();

    let broadcast = state.detection_broadcast();

    // Create integration clients.
    let apprise_client = integrations::create_apprise_client(&cli, config.as_ref());
    let birdweather_client = integrations::create_birdweather_client(&cli, config.as_ref());
    let email_notifier = integrations::create_email_notifier(&state);
    let heartbeat_client = integrations::create_heartbeat_client(&cli, config.as_ref());
    let mqtt_client = integrations::create_mqtt_client(&cli, config.as_ref());
    // Cloned before the detection pipeline takes ownership: the presence
    // session below needs the same broker settings but is spawned after the
    // router is built, and it must run in web-only mode too — a station whose
    // capture is off is still a station whose reachability an operator cares
    // about.
    let mqtt_presence_client = mqtt_client.clone();
    let notification_filter = integrations::create_notification_filter(&cli, config.as_ref());
    let notification_template = integrations::create_notification_template(&cli, config.as_ref());

    // OB-9: give the web layer the *same* notifier the alert loops deliver
    // through, so "Test notifications" exercises the native routes, the
    // `apprise` CLI fallback, the circuit breaker and the rate limiter — the
    // machinery that decides whether a deadman alert leaves the box — instead
    // of a fresh client of its own. The last builder call: `AppState`'s
    // builders abort if the state has already been cloned, and the loops below
    // clone it.
    let state = match apprise_client.clone() {
        Some(handle) => state.with_notifier(birdnet_web::notifier::Notifier::attach(handle).await),
        None => state,
    };

    // Start weekly report scheduler (if Apprise is configured).
    if let Some(ref apprise) = apprise_client {
        let schedule = helpers::resolve::setting_str(
            &cli,
            "weekly_report_schedule",
            &cli.weekly_report_schedule,
            config.as_ref(),
            "WEEKLY_REPORT_SCHEDULE",
        );
        weekly_report::start_weekly_report_scheduler(
            &schedule,
            std::sync::Arc::clone(apprise),
            state.clone(),
        );
    }

    // Store-and-forward drainer: replays BirdWeather uploads parked by the
    // detection path during network outages (outbound_queue, migration 19).
    // Spawned whenever uploads are configured at all — the queue may hold a
    // backlog from before this boot.
    if let Some(ref bw) = birdweather_client {
        integrations::spawn_birdweather_drainer(state.clone(), bw.clone());
    }

    // External liveness ping. Runs regardless of `--web-only`: "is this box
    // still there" is a question a web-only station has too, and it is the only
    // one of the three health signals that an outside observer can answer when
    // the box is gone. See `integrations::heartbeat` for why it is a timer and
    // not, as it was, a line inside the per-detection loop.
    if let Some(hb) = heartbeat_client.clone() {
        integrations::spawn_heartbeat(hb);
    }

    // Detection deadman: end-to-end "is the station actually detecting?"
    // freshness gauge + once-per-episode alert. Resolution: CLI/env, then
    // the DEADMAN_HOURS config key, then the 24 h default; 0 disables the
    // alert but keeps the gauge. Skipped in web-only mode, where a quiet
    // database is expected, not a fault.
    if !cli.web_only {
        let deadman_hours = cli
            .deadman_hours
            .or_else(|| config.as_ref()?.get_parsed::<u32>("DEADMAN_HOURS").ok())
            .unwrap_or(integrations::DEFAULT_DEADMAN_HOURS);
        integrations::spawn_detection_deadman(state.clone(), apprise_client.clone(), deadman_hours);

        // Station health: the operational faults the deadman structurally
        // cannot see, because the station keeps detecting through all of them.
        // Same episode semantics — one alert, one recovery notice, nothing in
        // between. On by default: an unattended station that cannot tell anyone
        // it is degrading is the failure mode this whole subsystem exists for.
        let health_alerts = cli
            .station_health_alerts
            .or_else(|| {
                config
                    .as_ref()?
                    .get_parsed::<bool>("STATION_HEALTH_ALERTS")
                    .ok()
            })
            .unwrap_or(true);
        integrations::spawn_station_health(state.clone(), apprise_client.clone(), health_alerts);

        // Recording effort: how long the station actually listened, per source
        // per day. A detection count divided by nothing is not an abundance,
        // and the denominator moves with the season, with downtime and with a
        // failed microphone — see `integrations::effort`.
        integrations::spawn_effort_recorder(state.clone());

        // Acoustic health: what the microphones themselves sound like. The
        // deadman catches a station that has gone silent; nothing caught one
        // whose microphone had merely gone *deaf*, because that presents as
        // fewer detections and so does the end of the season — see
        // `integrations::acoustic_health`. Reads the same transient stream
        // directory capture writes to, so it needs no coordination with the
        // audio path.
        if let Some(stream_dir) = helpers::stream_dir(&cli, config.as_ref()) {
            integrations::spawn_acoustic_health(state.clone(), stream_dir, apprise_client.clone());
        }
    }

    // Start background subsystems.
    let _disk_manager_threads = helpers::start_disk_manager(&cli, config.as_ref(), &state);
    let _live_spectrogram_thread = helpers::start_live_spectrogram(&cli, config.as_ref(), &state);
    let _capture_handle = capture::start_capture_manager(
        &cli,
        config.as_ref(),
        Some(&state),
        state.metrics(),
        capture_status,
        Some(&live_audio),
    );

    let daemon_handle = if cli.web_only {
        tracing::info!("running in web-only mode (no detection daemon)");
        None
    } else {
        daemon::start_detection_daemon(
            &cli,
            config.as_ref(),
            state.clone(),
            broadcast,
            apprise_client,
            birdweather_client,
            email_notifier,
            mqtt_client,
            notification_filter,
            notification_template,
        )
    };

    // Record whether the detection pipeline actually came up so the health
    // endpoint can surface a station that is silently not detecting (web-only,
    // or a misconfigured model/labels/watch dir) rather than looking healthy.
    state.detection_status_flag().store(
        daemon_handle.is_some(),
        std::sync::atomic::Ordering::Relaxed,
    );

    // Register Avahi mDNS service for zero-config local discovery.
    let site_name = cli.site_name.as_deref().unwrap_or("BirdNet-Behavior");
    helpers::maybe_install_avahi_service(addr.port(), site_name);

    // Start the web server.
    //
    // Warn if the admin UI is exposed off-loopback without a configured
    // password. Keys on `CADDY_PWD` (config or env) — the same knob the
    // cookie middleware's "no admin password → open access" bypass and the
    // `helpers::auth` bootstrap read. (A password set directly via the
    // accounts UI also protects the panel; this loopback warning tracks the
    // env/config knob, matching the pre-cookie-flip behaviour.)
    let admin_password_configured = config
        .as_ref()
        .and_then(|c| c.get("CADDY_PWD").map(str::to_owned))
        .or_else(|| std::env::var("CADDY_PWD").ok())
        .is_some_and(|pwd| !pwd.is_empty());
    if !admin_password_configured && !addr.ip().is_loopback() {
        tracing::warn!(
            addr = %addr,
            "admin web UI is bound to a non-loopback address with NO authentication — anyone on \
             the network can change settings, trigger database backups, and update the software. \
             Set CADDY_PWD in the config to require a password, or bind --listen to 127.0.0.1 and \
             reach it over an SSH tunnel."
        );
    }
    tracing::info!(addr = %addr, "starting web server");
    let metrics_for_watchdog = state.metrics();

    // O-23 weather poll loop. Off by default; opt in with
    // `BNB_WEATHER_ENABLED=1`. Spawns a background task that pulls
    // hourly forecast rows from Open-Meteo (or a self-hosted instance
    // via `BNB_WEATHER_BASE_URL`) every 30 minutes and feeds them to
    // the overlay renderers in `birdnet-web::routes::pages::overlays`.
    // The handle is kept in scope so a future `--no-weather` toggle can
    // abort it; today the loop runs for the lifetime of the process.
    let _weather_poll_handle = integrations::spawn_weather_poll(config.as_ref(), state.clone());

    // Pre-warm the heavy-analytics fragment cache so the first visit to the
    // Heatmap / phenology / co-occurrence / time-series pages is instant, then
    // keep it warm on an interval a little under the cache TTL (10 min). Each
    // pass runs the same aggregate queries a page visit would, on the blocking
    // pool, so it never stalls the runtime; it is best-effort and decoupled from
    // request handling. The eight-minute cadence keeps the recurring background
    // query load gentle on a Raspberry Pi competing with live detection.
    {
        let prewarm_state = state.clone();
        tokio::spawn(async move {
            // Let the initial SQLite→DuckDB sync and the first detections settle
            // before the first (cold) pass.
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(480));
            loop {
                tick.tick().await;
                let s = prewarm_state.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || {
                    birdnet_web::routes::pages::prewarm_analytics(&s);
                })
                .await
                {
                    tracing::debug!(error = %e, "analytics pre-warm pass did not complete");
                }
            }
        });
    }

    // Load or mint the certificate. Deliberately not at the top of `run`: it
    // writes to disk in self-signed mode, and doing that before the database
    // and configuration have been validated would leave material behind from a
    // start that never completed.
    let tls_server_config = match birdnet_web::tls::server_config(&tls_plan.settings) {
        Ok(v) => v,
        Err(e) => {
            return Err(format!(
                "TLS is enabled (--tls-mode {}) but the certificate could not be prepared: {e}",
                tls_plan.settings.mode
            )
            .into());
        }
    };
    if tls_plan.settings.mode == birdnet_web::tls::TlsMode::SelfSigned {
        tracing::info!(
            ca = %birdnet_web::tls::ca_certificate_path(&tls_plan.settings.state_dir).display(),
            "self-signed HTTPS: import this CA file once to stop the browser warning"
        );
    }

    // Keep a handle to the state so the graceful-shutdown hook can wake live
    // WebSocket/SSE handlers (`begin_shutdown`); `build_router` moves `state`.
    let shutdown_state = state.clone();
    let app = birdnet_web::server::build_router(state);

    // Publish Home Assistant MQTT auto-discovery if configured.
    if let Some(ref mqtt) = integrations::get_mqtt_client_ref(&cli, config.as_ref()) {
        integrations::publish_ha_discovery(mqtt, &cli, config.as_ref());
    }

    // Hold one MQTT connection open carrying a last will, so the "Station
    // Status" entity that discovery has always advertised finally has
    // something behind it. Without this the broker has no session to notice
    // dying, and the entity stays `unknown` for the life of the station —
    // which is the one state an offline alert cannot be built on.
    integrations::spawn_mqtt_presence(shutdown_state.clone(), mqtt_presence_client);

    // Spawn daily auto-update check (logs result, does not auto-apply).
    //
    // Gated because this is the station's only outbound connection that nothing
    // asked for: it fired 60 s after start and every 24 h thereafter with no way
    // to stop it, which is a problem on a metered link and unanswerable during
    // an institutional review. `--no-update-check` / `--offline` turn it off.
    if helpers::egress::update_check_allowed(&cli) {
        tokio::spawn(async {
            // Wait 60 seconds after startup before first check.
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let current_version = env!("CARGO_PKG_VERSION");
            loop {
                match tokio::task::spawn_blocking(move || {
                    birdnet_integrations::auto_update::check_for_update(current_version)
                })
                .await
                {
                    Ok(Ok(info)) => {
                        if info.update_available {
                            tracing::info!(
                                current = %info.current_version,
                                latest = %info.latest_version,
                                "new version available — use the admin panel to update"
                            );
                        } else {
                            tracing::debug!("auto-update check: already up to date");
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::debug!(error = %e, "auto-update check failed (non-fatal)");
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "auto-update check task panicked");
                    }
                }
                // Check once every 24 hours.
                tokio::time::sleep(std::time::Duration::from_secs(86_400)).await;
            }
        });
    } else {
        tracing::info!("daily update check disabled; this station will not contact api.github.com");
    }

    // Periodic database maintenance (VACUUM, integrity check, backup rotation),
    // plus the per-species recording cap (MAX_FILES_SPECIES): keep the newest N
    // extracted clips per species on disk, pruned on the daily tick. CLI flag
    // wins over the config key; 0 (default) = unlimited. No-op when the DB does
    // not exist yet.
    let species_cap = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
            .as_ref()
            .and_then(|c| c.get_parsed::<u32>("MAX_FILES_SPECIES").ok())
            .unwrap_or(0)
    };
    // Age-based clip retention (`CLIP_RETENTION_DAYS`). Same precedence as the
    // other disk knobs — explicit flag/env, then the DB-overlaid config, then
    // the default — and 0 (the default) means keep audio forever.
    let clip_retention_days = if cli.clip_retention_days > 0 {
        cli.clip_retention_days
    } else {
        config
            .as_ref()
            .and_then(|c| c.get_parsed::<u32>("CLIP_RETENTION_DAYS").ok())
            .unwrap_or(0)
    };
    // Offsite backups. A broken configuration is reported here and the station
    // carries on: it must keep recording birds either way, and `--doctor` is
    // where the operator is told in full.
    let offsite = match crate::helpers::offsite::plan(&cli, config.as_ref()) {
        crate::helpers::offsite::OffsitePlan::Off => None,
        crate::helpers::offsite::OffsitePlan::On(c) => {
            tracing::info!(
                destination = %c.destination.describe(),
                keep = c.keep,
                "offsite backups enabled"
            );
            Some(std::sync::Arc::new(*c))
        }
        crate::helpers::offsite::OffsitePlan::Broken(problems) => {
            for problem in &problems {
                tracing::error!("offsite backup is configured but cannot run: {problem}");
            }
            tracing::error!(
                count = problems.len(),
                "offsite backups are OFF; backups will stay on this station. \
                 Run `birdnet-behavior --doctor` for the whole list"
            );
            None
        }
    };

    maintenance::spawn_database_maintenance(
        db_path.clone(),
        backup_dir.clone(),
        recordings_dir_for_maintenance,
        species_cap,
        clip_retention_days,
        offsite,
        ingest_halt_for_maintenance,
    );

    // Bind every listener the plan calls for before telling systemd we are up:
    // `Type=notify` treats READY=1 as "the socket is accepting", and a station
    // that reports ready and then fails to bind 8503 is a worse outcome than
    // one that fails to start at all.
    let listener = if tls_plan.wants_plain_listener() {
        Some(tokio::net::TcpListener::bind(addr).await?)
    } else {
        None
    };

    let https = match (tls_plan.https_addr, tls_server_config) {
        (Some(https_addr), Some((config, resolver))) => {
            let listener = tokio::net::TcpListener::bind(https_addr).await?;
            // Only `manual` mode has files somebody else rewrites; the
            // self-signed pair only ever changes when this process changes it.
            if tls_plan.settings.mode == birdnet_web::tls::TlsMode::Manual
                && let (Some(cert), Some(key)) = (
                    tls_plan.settings.cert.clone(),
                    tls_plan.settings.key.clone(),
                )
            {
                birdnet_web::tls::spawn_reloader(resolver, cert, key);
            }
            tracing::info!(
                addr = %https_addr,
                mode = %tls_plan.settings.mode,
                "HTTPS listening"
            );
            Some((listener, config))
        }
        _ => None,
    };

    // The web server is bound — notify systemd that startup is complete and
    // begin pinging the watchdog. If we are not running under systemd these
    // are no-ops.
    sd_notify::ready();
    // Hand shutdown ownership to the graceful-shutdown hook installed below:
    // `run` stops racing its startup-phase signal listener. There is no gap —
    // between here and the serve loop's first suspension there are no awaits,
    // so the hook's signal listeners are registered before this function can
    // yield. A receiver dropped early (run already returning) is fine.
    let _ = started.send(());
    // Gate the watchdog on detection-loop progress: if the pipeline hangs, the
    // heartbeat stops advancing, the pinger withholds WATCHDOG=1, and systemd
    // restarts us instead of leaving a frozen daemon running. In web-only mode
    // there is no detection loop, so the pinger falls back to unconditional pings.
    let detection_heartbeat = daemon_handle
        .as_ref()
        .map(birdnet_core::detection::daemon::DaemonHandle::heartbeat);
    sd_notify::spawn_watchdog_pinger(Some(metrics_for_watchdog), detection_heartbeat);

    // One shutdown signal, fanned out: with TLS configured there are two
    // listeners, and both have to drain. The hook also wakes live WebSocket /
    // SSE handlers (dashboard, spectrogram, admin logs) so they close their
    // sockets and the drain finishes in milliseconds, instead of every restart
    // waiting out SHUTDOWN_GRACE.
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    {
        let tx = shutdown_tx.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            shutdown_state.begin_shutdown();
            let _ = tx.send(());
        });
    }
    let signalled = move |mut rx: tokio::sync::broadcast::Receiver<()>| async move {
        // `recv` errors only if every sender is dropped, which happens on the
        // way out anyway; either way it means "stop".
        let _ = rx.recv().await;
    };

    let https_task = https.map(|(listener, config)| {
        let app = app.clone();
        let shutdown = signalled(shutdown_tx.subscribe());
        tokio::spawn(async move {
            birdnet_web::tls::serve(listener, config, app, shutdown)
                .await
                .map_err(|e| e.to_string())
        })
    });

    let plain_task = listener.map(|listener| {
        // With `--tls-redirect` the plain port stops serving the application
        // and answers only "the same URL, over HTTPS".
        let router = if tls_plan.redirect {
            let port = tls_plan
                .https_addr
                .map_or_else(|| addr.port(), |a| a.port());
            tracing::info!(addr = %addr, https_port = port, "HTTP redirecting to HTTPS");
            birdnet_web::tls::redirect_router(port)
        } else {
            app.clone()
        };
        let shutdown = signalled(shutdown_tx.subscribe());
        tokio::spawn(async move {
            // `into_make_service_with_connect_info` so the per-IP rate limiter
            // can read the client socket address from request extensions.
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(|e| e.to_string())
        })
    });

    let drain = async {
        for task in [https_task, plain_task].into_iter().flatten() {
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::error!(error = %e, "web server stopped with an error"),
                Err(e) => tracing::error!(error = %e, "web server task panicked"),
            }
        }
    };

    // Backstop the graceful drain. The hook above signals live WebSocket /
    // event-stream clients (the dashboard keeps one open) to close, so the
    // drain normally completes at once. SHUTDOWN_GRACE only fires if a
    // connection ignores the signal — without it a stuck client would hang the
    // process until systemd SIGKILLs it at TimeoutStopSec.
    tokio::select! {
        () = drain => {}
        () = shutdown_grace_backstop() => {
            tracing::warn!(
                grace_secs = SHUTDOWN_GRACE.as_secs(),
                "shutdown grace elapsed with connection(s) still open; forcing exit"
            );
        }
    }

    // Stop the detection loop so its event sender drops and the (blocking) event
    // processor returns, letting the runtime wind down cleanly instead of being
    // left to a SIGKILL.
    if let Some(handle) = daemon_handle.as_ref() {
        handle.stop();
    }

    sd_notify::stopping();
    tracing::info!("BirdNet-Behavior stopped");
    Ok(())
}

/// After a shutdown signal, allow in-flight connections a bounded window to
/// drain before we stop waiting — so a long-lived WebSocket can't wedge
/// shutdown until systemd SIGKILLs the process.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

async fn shutdown_grace_backstop() {
    shutdown_signal().await;
    tokio::time::sleep(SHUTDOWN_GRACE).await;
}

/// Wait for a shutdown signal (SIGTERM or SIGINT).
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => tracing::info!("received Ctrl+C"),
        () = terminate => tracing::info!("received SIGTERM"),
    }
}
