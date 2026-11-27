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

/// What to do when `check_and_recover` could not leave a healthy database.
///
/// This used to be one branch. Every error out of recovery was read as "corrupt
/// and no good backup exists", quarantined, and followed by a fresh database —
/// which is right when there is genuinely nothing to lose, and is how an
/// unattended station keeps recording instead of sitting dead.
///
/// It is exactly wrong when a good backup was found and could not be put in
/// place. Reproduced on a full card: a 2.7 MB backup, a truncated `birds.db`,
/// and 1.3 MB free. The restore failed with `DiskFull`, and the station was
/// about to announce that no good backup existed and start fresh — with the
/// backup sitting intact beside it, and the weekly ring due to rotate it away
/// within five weeks. Starting fresh also buys nothing in that state, because a
/// station with no room cannot record either.
#[derive(Debug, PartialEq, Eq)]
enum RecoveryFallback {
    /// Nothing recoverable exists. Move the corrupt file aside and start fresh.
    QuarantineAndStartFresh,
    /// The history is still recoverable and must not be discarded.
    RefuseToStart,
}

/// Pick the fallback [`RecoveryFallback`] documents.
const fn recovery_fallback(e: &birdnet_db::resilience::ResilienceError) -> RecoveryFallback {
    match e {
        birdnet_db::resilience::ResilienceError::RestoreFailed { .. } => {
            RecoveryFallback::RefuseToStart
        }
        _ => RecoveryFallback::QuarantineAndStartFresh,
    }
}

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
///
/// The start-up, in the order it happens, one named step each (MDL-19): the
/// configuration this start runs on, the database, the HTTPS plan, the
/// application state and the settings laid over the file, the integrations,
/// the alerting and background tasks, the detector, the periodic tasks, the
/// listeners, and the serve loop until a shutdown signal. Each step returns
/// what the later ones need as a typed value, so what one step decides is
/// visible in its signature rather than in a local twenty screens up.
async fn serve(
    cli: Cli,
    config: Option<birdnet_core::config::Config>,
    log_broadcaster: birdnet_web::routes::admin::logs::LogBroadcaster,
    started: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Chosen {
        cli,
        config,
        decision: config_decision,
        secret_files,
    } = choose_configuration(cli, config);
    let db = prepare_database(config.as_ref())?;

    // Build app state.
    let addr: std::net::SocketAddr = cli.listen.parse()?;
    let tls_plan = plan_https(&cli, config.as_ref(), addr, &db.path)?;
    let state = build_state(&cli, config.as_ref(), &db.path, log_broadcaster)?;
    let (state, config) =
        settle_settings(state, &cli, config, &config_decision, &secret_files, &db).await;
    let (state, capture) = attach_capture(state);

    // Captured before `state` is moved into the web server below; used by the
    // maintenance task. PS-5: the maintenance loop that finds the corruption
    // is the one that has to stop the detection writes, so it holds a clone
    // of the latch the ingest path reads.
    let maintenance_inputs = MaintenanceInputs {
        recordings_dir: state.recording_dir(),
        ingest_halt: state.ingest_halt_flag(),
        metrics: state.metrics(),
    };
    let broadcast = state.detection_broadcast();

    let integrations =
        build_integrations(&cli, config.as_ref(), &state, addr, &tls_plan, &db.path).await;
    let state = attach_notifier(state, integrations.apprise.as_ref()).await;
    spawn_alerting(&cli, config.as_ref(), &state, &integrations);
    let background = start_background(&cli, config.as_ref(), &state, capture);
    keep_last_good(&cli, &config_decision);
    let mut integrations = integrations;
    let mqtt_presence = integrations.mqtt_presence.take();

    let daemon_handle = start_detector(
        &cli,
        config.as_ref(),
        &state,
        &config_decision,
        DetectorInputs {
            broadcast,
            integrations,
            in_flight: background.in_flight.clone(),
        },
    );
    announce_access(&cli, &state, addr);
    let metrics_for_watchdog = state.metrics();
    spawn_periodic(config.as_ref(), &state);
    let tls = prepare_certificate(&tls_plan, &state, addr)?;

    // Keep a handle to the state so the graceful-shutdown hook can wake live
    // WebSocket/SSE handlers (`begin_shutdown`); `build_router` moves `state`.
    let shutdown_state = state.clone();
    let app = birdnet_web::server::build_router(state);
    spawn_mqtt(&cli, config.as_ref(), &shutdown_state, mqtt_presence);
    spawn_update_check(&cli);
    spawn_maintenance(&cli, config.as_ref(), &db, maintenance_inputs);

    let listeners = bind_listeners(&tls_plan, tls, addr).await?;

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
    // The station fetches a missing or damaged model file again itself, and
    // restarts to load one it went without (MDL-10). After the detector, so
    // what it went without is already published, and after the hand-off
    // above, so a restart it asks for is the serve loop's graceful one and
    // never the start-up race's early exit.
    daemon::repair::spawn(&cli, config.as_ref(), shutdown_state.clone());
    // Gate the watchdog on detection-loop progress: if the pipeline hangs, the
    // heartbeat stops advancing, the pinger withholds WATCHDOG=1, and systemd
    // restarts us instead of leaving a frozen daemon running. In web-only mode
    // there is no detection loop, so the pinger falls back to unconditional pings.
    let detection_heartbeat = daemon_handle
        .as_ref()
        .map(birdnet_core::detection::daemon::DaemonHandle::heartbeat);
    sd_notify::spawn_watchdog_pinger(Some(metrics_for_watchdog), detection_heartbeat);

    serve_until_shutdown(listeners, app, shutdown_state, &tls_plan, addr).await;
    stop_detector(daemon_handle.as_ref()).await;

    sd_notify::stopping();
    tracing::info!("BirdNet-Behavior stopped");
    // Held to here: the data directory stays this process's until it returns
    // (DD-25), and the background threads' handles with it.
    drop((background, db));
    Ok(())
}

/// The configuration this start runs on, and how it was chosen.
struct Chosen {
    /// The command line, with `--web-only` forced when the configuration
    /// leaves nothing safe to detect with.
    cli: Cli,
    /// The configuration, with secrets mounted as files resolved.
    config: Option<birdnet_core::config::Config>,
    /// Whether the file loaded, was reverted, or was rejected.
    decision: helpers::startup_config::ConfigDecision,
    /// The keys whose values came from `BIRDNET_<KEY>_FILE`.
    secret_files: birdnet_core::config::secret_file::SecretFiles,
}

/// Step 1: validate the configuration and decide which one this start runs on.
///
/// A misconfigured station used to refuse to start here — validation ran
/// inside the new process, after systemd had killed the old one, so a typo
/// made over SSH became a restart loop with no web UI and no way back (LC-6).
/// Now: warnings are logged; errors are logged and the station runs on the
/// last configuration a start succeeded on when there is one, else web-only
/// on the file as it is, and the boot journal says which.
fn choose_configuration(cli: Cli, config: Option<birdnet_core::config::Config>) -> Chosen {
    let mut cli = cli;
    if let Some(ref cfg) = config {
        use birdnet_core::config::validate::{Severity, validate};
        for f in validate(cfg) {
            match f.severity {
                Severity::Error => {
                    tracing::error!(key = %f.key, remediation = %f.remediation, "{}", f.message);
                }
                Severity::Warning => {
                    tracing::warn!(key = %f.key, remediation = %f.remediation, "{}", f.message);
                }
            }
        }
    }
    let (config, decision) = helpers::startup_config::choose(config, &cli.config);

    // Credentials mounted as files (`BIRDNET_<KEY>_FILE`), the way Docker and
    // Kubernetes hand a secret to a process without putting it in the
    // environment. Resolved here — after the config is chosen, before anything
    // reads a credential out of it — and the keys that came from a file are
    // carried to the settings seed, which must not copy them into the
    // database.
    let (config, secret_files) = helpers::resolve_secret_files(config);
    // Keys in an existing config file that now do what they say; a station that
    // upgraded with one set is doing something different from last week.
    for notice in helpers::startup_notices::notices(config.as_ref()) {
        tracing::info!("{notice}");
    }
    match &decision {
        helpers::startup_config::ConfigDecision::Loaded => {}
        helpers::startup_config::ConfigDecision::Reverted { errors, last_good } => {
            tracing::error!(
                errors = errors.len(),
                last_good = %last_good.display(),
                "configuration has errors; running on the last good configuration instead"
            );
        }
        helpers::startup_config::ConfigDecision::Rejected { errors } => {
            tracing::error!(
                errors = errors.len(),
                "configuration has errors and there is no last good copy; running web-only so \
                 the diagnostics are reachable"
            );
        }
    }
    if decision.forces_web_only() {
        cli.web_only = true;
    }
    Chosen {
        cli,
        config,
        decision,
        secret_files,
    }
}

/// The database this start opened, and the lock that makes it this
/// process's.
struct DatabaseStart {
    /// The SQLite file.
    path: std::path::PathBuf,
    /// Where its backups are kept.
    backup_dir: std::path::PathBuf,
    /// Whether the file existed before this start (UP-3).
    present_at_start: bool,
    /// One process per data directory (DD-25), held until `serve` returns.
    _lock: helpers::instance_lock::InstanceLock,
}

/// Step 2: make the database's directory, take the data directory's lock,
/// recover a damaged database or refuse to start, and convert it to
/// incremental auto-vacuum once.
///
/// # Errors
///
/// The directory cannot be made, another instance holds the directory, or
/// the database is damaged and its good backup could not be put in place.
fn prepare_database(
    config: Option<&birdnet_core::config::Config>,
) -> Result<DatabaseStart, Box<dyn std::error::Error>> {
    let db_path = helpers::db_path_from_config(config);
    // SQLite will not create a missing parent directory — it fails the open
    // with a bare "unable to open database file" — so do it here, before
    // anything touches the path. Matches what every other directory the
    // station owns already does, and makes `--doctor`'s "will be created on
    // first run" true rather than aspirational.
    helpers::ensure_db_dir(&db_path)?;
    // For the boot journal (UP-3): whether the database existed before this
    // start opened it. `open_or_create` makes the path exist, so it has to be
    // read here.
    let present_at_start = db_path.exists();
    // One process per data directory (DD-25), before anything opens a file
    // there: a second instance used to quarantine the first one's live
    // analytics store and only then die on the bind. Held until `serve`
    // returns.
    let data_dir = db_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    let lock =
        helpers::instance_lock::acquire(&data_dir, helpers::instance_lock::grace_from_env())?;
    tracing::debug!(lock = %lock.path.display(), "data directory locked for this process");
    let backup_dir = data_dir.join("backups");
    if db_path.exists() {
        recover_database(&db_path, &backup_dir)?;
        convert_to_incremental_vacuum(&db_path);
    }
    Ok(DatabaseStart {
        path: db_path,
        backup_dir,
        present_at_start,
        _lock: lock,
    })
}

/// Check the database and recover it from a backup, quarantine it and start
/// fresh, or refuse to start — the choice [`RecoveryFallback`] documents.
///
/// # Errors
///
/// A good backup exists and could not be put in place, or a corrupt database
/// could not be moved aside.
fn recover_database(
    db_path: &std::path::Path,
    backup_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let e = match birdnet_db::resilience::check_and_recover(db_path, backup_dir) {
        Ok(result) => {
            if result.action == birdnet_db::resilience::RecoveryAction::Recovered {
                tracing::warn!(details = %result.details, "database recovered");
            } else {
                tracing::info!(details = %result.details, "database healthy");
            }
            return Ok(());
        }
        Err(e) => e,
    };
    match recovery_fallback(&e) {
        RecoveryFallback::RefuseToStart => Err(format!(
            "{e} — refusing to start. Starting fresh here would discard the one copy \
             of the history that is still good, and the weekly backup ring would \
             rotate it away within five weeks."
        )
        .into()),
        RecoveryFallback::QuarantineAndStartFresh => {
            // Never write to a corrupt database: quarantine it for offline
            // recovery and start fresh, so an unattended station keeps
            // recording rather than refusing to boot. Only refuse to start if
            // we cannot even move the corrupt file aside.
            tracing::error!(
                error = %e,
                path = %db_path.display(),
                "database is corrupt and no good backup exists; quarantining before starting fresh"
            );
            let quarantined = birdnet_db::resilience::quarantine_corrupt_database(db_path)
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
            Ok(())
        }
    }
}

/// PS-3: a database from before incremental auto-vacuum is rewritten into it
/// once, here, before anything writes — so the weekly reclaim never has to.
/// The copy is staged beside the file, not in the unit's tmpfs.
fn convert_to_incremental_vacuum(db_path: &std::path::Path) {
    match birdnet_db::resilience::ensure_incremental_vacuum(db_path) {
        Ok(birdnet_db::resilience::VacuumMode::AlreadyIncremental) => {}
        Ok(birdnet_db::resilience::VacuumMode::Converted {
            bytes_before,
            bytes_after,
        }) => tracing::info!(
            bytes_before,
            bytes_after,
            "database converted to incremental auto-vacuum (a one-time rewrite; the \
             weekly reclaim now moves only the free pages)"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "database could not be converted to incremental auto-vacuum; the weekly space \
             reclaim will fail until it is (free space beside the database is what it needs)"
        ),
    }
}

/// Step 3: decide the TLS shape now, before any expensive setup, and keep the
/// station's session-signing secret beside the database.
///
/// A station asked for HTTPS that cannot have it must say so and stop, not
/// quietly come up on plain HTTP with the operator believing otherwise.
///
/// # Errors
///
/// The TLS settings ask for something this station cannot do.
fn plan_https(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    addr: std::net::SocketAddr,
    db_path: &std::path::Path,
) -> Result<helpers::tls::TlsPlan, Box<dyn std::error::Error>> {
    let tls_plan = helpers::tls::plan(cli, config, addr, db_path)?;
    for warning in &tls_plan.warnings {
        tracing::warn!("{warning}");
    }
    // The station's own session-signing secret, beside the database (DD-15):
    // without it every bare-metal install ran on a per-process secret and
    // every login died on restart. A data directory that cannot take the
    // file degrades to the old derivation, and says so.
    if let Some(data_dir) = db_path.parent()
        && let Err(e) = birdnet_web::session::install_persisted_secret(data_dir)
    {
        tracing::warn!(
            error = %e,
            dir = %data_dir.display(),
            "could not persist a session secret; login sessions will not survive a restart"
        );
    }
    Ok(tls_plan)
}

/// Step 4: the application state over the database, with the configuration
/// path, the log stream, the supervision facts and the diagnostics hooks.
///
/// # Errors
///
/// The database (or the analytics store) cannot be opened.
fn build_state(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    db_path: &std::path::Path,
    log_broadcaster: birdnet_web::routes::admin::logs::LogBroadcaster,
) -> Result<birdnet_web::state::AppState, Box<dyn std::error::Error>> {
    let server_config = birdnet_web::server::ServerConfig {
        addr: cli.listen.parse()?,
        db_path: db_path.to_path_buf(),
    };

    #[cfg(feature = "analytics")]
    let state = helpers::build_state_with_analytics(cli, config, &server_config)?;

    #[cfg(not(feature = "analytics"))]
    let state = {
        if cli.analytics_db.is_some() {
            tracing::warn!(
                "DuckDB analytics requested but not compiled in. Rebuild with --features analytics"
            );
        }
        birdnet_web::state::AppState::new(server_config.db_path)
            .map_err(|e| format!("database error: {e}"))?
    };

    // Commit durability, from DB_SYNCHRONOUS, on the one connection that
    // writes detections (review F17).
    helpers::apply_db_durability(&state, config);

    // Thread the active config path so the in-UI diagnostics page can re-read
    // and validate it, and the broadcaster the `tracing` layer installed in
    // `main` is already writing to — without this the state holds the empty
    // one its constructor made, and `GET /admin/system/logs` streams
    // keep-alives for ever.
    // The bundle hook wants the same ring the state streams, so the support
    // bundle an operator downloads carries the process's recent log.
    let logs_for_bundle = log_broadcaster.clone();
    Ok(state
        .with_config_path(cli.config.clone())
        .with_log_broadcaster(log_broadcaster)
        // Decided once, here, and carried on the state: whether anything would
        // bring this process back if it exited. Both restart handlers read it
        // rather than the environment, so a test binary that inherited
        // `INVOCATION_ID` cannot reach the branch that signals.
        .with_supervised_by_systemd(
            birdnet_web::routes::admin::system_controls::supervised_by_systemd(),
        )
        // Where nothing supervises the process (the binary run from a terminal,
        // a container without a restart policy) a restart request makes it shut
        // down and run itself again, so the browser can finish what it started:
        // the setup wizard ends on a restart. `main` does the exec.
        .with_self_restart(cfg!(unix))
        // OP-1: `--doctor` and `--support-bundle` from the browser. The hooks
        // are read-only clones of the command line and the configuration, so
        // a GET runs the checks and never the repairs.
        .with_diagnostics(helpers::diagnostics::hooks(cli, config, logs_for_bundle)))
}

/// Step 5: the boot journal, the admin account and the settings table laid
/// over the configuration file, then everything the web layer derives from
/// the configuration that results. Returns the state and that configuration,
/// which is what every later step reads.
async fn settle_settings(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<birdnet_core::config::Config>,
    config_decision: &helpers::startup_config::ConfigDecision,
    secret_files: &birdnet_core::config::secret_file::SecretFiles,
    db: &DatabaseStart,
) -> (
    birdnet_web::state::AppState,
    Option<birdnet_core::config::Config>,
) {
    // The boot journal (UP-3): this start against the last one, kept outside
    // the database so a volume that did not mount cannot take the memory of
    // it with it. Read by the health verdict and the station-health notifier.
    let mut boot_anomalies =
        helpers::boot_journal::record_boot(&cli.config, &db.path, db.present_at_start, &state);
    boot_anomalies.extend(config_decision.anomaly());
    state.set_boot_anomalies(boot_anomalies);

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
    helpers::seed_db_settings_from_config(config.as_ref(), cli, &state, &secret_files.resolved());
    // OOB-13: who may hear the live microphone, written once — `anyone` for a
    // station that already holds detections, `signed-in` for a new one.
    state
        .run_blocking(birdnet_web::live_audio::seed_at_start)
        .await;

    // Overlay the admin-UI settings (SQLite `settings` table) on top of the
    // file config so settings saved in the web UI actually take effect. Without
    // this the settings form is write-only: the daemon and capture subsystems
    // below read only the file config + CLI flags. The database value wins, and
    // changes apply on restart (as the settings page already states). Done here
    // — after the DB-backed state exists, before any subsystem reads config.
    let config = helpers::overlay_db_settings(config, &state);
    let state = derive_from_settings(state, cli, config.as_ref(), &db.path).await;
    (state, config)
}

/// What the web layer takes from the settled configuration: who may frame
/// the station, the offline notice, and the species pages' images, names,
/// codes, taxonomy and language.
async fn derive_from_settings(
    state: birdnet_web::state::AppState,
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    db_path: &std::path::Path,
) -> birdnet_web::state::AppState {
    // Which sites may frame the station (off unless EMBED_ORIGINS lists some).
    helpers::embedding::apply_embed_origins(&state, config);

    // Say once, at start, exactly what offline mode turned off — so the answer
    // to "does this station phone home?" is in the journal rather than in a
    // reading of the source.
    if let Some(notice) = helpers::egress::offline_notice(cli) {
        tracing::info!("{notice}");
    }

    // Initialize all optional subsystems.
    let state = helpers::init_image_cache(state, cli, config, db_path);
    // The flag has no clap default, so `Some` means the operator supplied it;
    // otherwise take the config, which is where a path entered on
    // `/admin/settings` arrives via the overlay above.
    let custom_image_dir = cli.custom_image_dir.clone().or_else(|| {
        config
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
    let state = helpers::init_site_name(state, cli, config);
    let info_site = helpers::info_site(cli, config);
    let state = if info_site == "ebird" {
        state
    } else {
        state.with_info_site(info_site)
    };
    let state = helpers::init_species_codes(state, cli, config);
    let state = helpers::init_taxonomy(state, cli, config);
    let observed = state.run_blocking(helpers::observed_names).await;
    let state = helpers::init_i18n(state, cli, config, &observed);
    helpers::init_private_mode(state, cli, config)
}

/// The capture supervisor's shared handles, created before the state is
/// shared and handed to the supervisor in [`start_background`].
struct CaptureHandles {
    status: birdnet_core::audio::capture::CaptureStatusHandle,
    control: birdnet_core::audio::capture::CaptureControlHandle,
    live_audio: birdnet_core::audio::capture::LiveAudioHubHandle,
}

/// Step 6: the handles the capture supervisor and the web layer share.
fn attach_capture(
    state: birdnet_web::state::AppState,
) -> (birdnet_web::state::AppState, CaptureHandles) {
    // The capture supervisor publishes per-source health into this shared
    // handle; the web layer reads it for Station Health. One clone goes into
    // the (about-to-be-shared) AppState, the original into the supervisor
    // thread.
    let status = birdnet_core::audio::capture::new_capture_status();
    let state = state.with_capture_status(status.clone());

    // The reverse direction of the same seam: the web layer records a
    // restart request for one source here, and the supervisor drains it on its
    // next tick. Without this an operator's only remedy for one wedged RTSP
    // camera is restarting the whole service, which drops every other source
    // and the audio in flight with them.
    let control = birdnet_core::audio::capture::new_capture_control();
    let state = state.with_capture_control(control.clone());

    // Teed capture sources publish their live PCM here, and `/stream` reads it
    // instead of opening the audio device a second time — which an ALSA
    // microphone refuses with `Device or resource busy` while it is being
    // recorded.
    let live_audio = birdnet_core::audio::capture::new_live_audio_hub();
    let state = state.with_live_audio(std::sync::Arc::clone(&live_audio));
    (
        state,
        CaptureHandles {
            status,
            control,
            live_audio,
        },
    )
}

/// What the maintenance task needs from the state before the state moves
/// into the router.
struct MaintenanceInputs {
    recordings_dir: std::path::PathBuf,
    ingest_halt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    metrics: birdnet_web::metrics::SharedMetrics,
}

/// The integration clients this start built, for the alert loops and the
/// detector.
struct Integrations {
    apprise: Option<integrations::AppriseHandle>,
    birdweather: Option<birdnet_integrations::birdweather::Client>,
    email: Option<integrations::EmailHandle>,
    heartbeat: Option<std::sync::Arc<birdnet_integrations::heartbeat::HeartbeatClient>>,
    mqtt: Option<integrations::MqttHandle>,
    /// The same broker settings, for the presence session that runs in
    /// web-only mode too.
    mqtt_presence: Option<integrations::MqttHandle>,
    notification_filter: birdnet_integrations::notification::NotificationFilter,
    notification_template: birdnet_integrations::notification::NotificationTemplate,
}

/// Step 7: the notification and upload clients, and the probes the web
/// layer's "Test" buttons run.
async fn build_integrations(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    addr: std::net::SocketAddr,
    tls_plan: &helpers::tls::TlsPlan,
    db_path: &std::path::Path,
) -> Integrations {
    let apprise = integrations::create_apprise_client(cli, config);
    // The blur offset is kept beside the database so it is the same after a
    // restart (see `fuzz_location` for what a fixed offset does and does not hide).
    let birdweather = match integrations::create_birdweather_client(cli, config) {
        Some(client) => {
            let data_dir = db_path.parent().map(std::path::Path::to_path_buf);
            // A file read (and, once, a write): off the async workers. If the
            // task dies the client is gone with it, which is the fail-closed
            // answer for a client that was about to post a blurred position.
            match tokio::task::spawn_blocking(move || {
                integrations::with_persisted_blur_seed(client, data_dir.as_deref())
            })
            .await
            {
                Ok(client) => Some(client),
                Err(e) => {
                    tracing::error!(error = %e, "BirdWeather uploads are OFF: the blur-seed task failed");
                    None
                }
            }
        }
        None => None,
    };
    let email = integrations::create_email_notifier(state);
    // What "Test all channels" can reach (DD-24): the integrations this
    // binary built, wrapped as probes the web layer runs on demand.
    let mut probes = integrations::notification_probes(
        integrations::get_mqtt_client_ref(cli, config),
        email.clone(),
    );
    // eBird and weather are not channels, so "Test all" leaves them out; they
    // have a button each (P-15).
    (probes.ebird, probes.weather) = integrations::data_source_probes(config);
    state.set_notification_probes(probes);
    let heartbeat = integrations::create_heartbeat_client(cli, config);
    let mqtt = integrations::create_mqtt_client(cli, config);
    // Cloned before the detection pipeline takes ownership: the presence
    // session needs the same broker settings but is spawned after the router
    // is built, and it must run in web-only mode too — a station whose
    // capture is off is still a station whose reachability an operator cares
    // about.
    let mqtt_presence = mqtt.clone();
    let notification_filter = integrations::create_notification_filter(cli, config);
    // How a phone reaches this station, for the links a notification carries.
    let notification_template = integrations::create_notification_template(
        cli,
        config,
        integrations::station_url(addr, tls_plan),
    );
    Integrations {
        apprise,
        birdweather,
        email,
        heartbeat,
        mqtt,
        mqtt_presence,
        notification_filter,
        notification_template,
    }
}

/// OB-9: give the web layer the *same* notifier the alert loops deliver
/// through, so "Test notifications" exercises the native routes, the
/// `apprise` CLI fallback, the circuit breaker and the rate limiter — the
/// machinery that decides whether a deadman alert leaves the box — instead of
/// a fresh client of its own. The last builder call: `AppState`'s builders
/// abort if the state has already been cloned, and the loops after it clone
/// it.
async fn attach_notifier(
    state: birdnet_web::state::AppState,
    apprise: Option<&integrations::AppriseHandle>,
) -> birdnet_web::state::AppState {
    match apprise {
        Some(handle) => {
            state.with_notifier(birdnet_web::notifier::Notifier::attach(handle.clone()).await)
        }
        None => state,
    }
}

/// Step 8: the loops that tell somebody when something is wrong: the weekly
/// report, the upload drainer, the heartbeat, and (when detecting) the
/// deadman, station health, recording effort and acoustic health.
fn spawn_alerting(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    integrations: &Integrations,
) {
    // Start weekly report scheduler (if Apprise is configured).
    if let Some(ref apprise) = integrations.apprise {
        let schedule = helpers::resolve::setting_str(
            cli,
            "weekly_report_schedule",
            &cli.weekly_report_schedule,
            config,
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
    if let Some(ref bw) = integrations.birdweather {
        integrations::spawn_birdweather_drainer(state.clone(), bw.clone());
    }

    // External liveness ping. Runs regardless of `--web-only`: "is this box
    // still there" is a question a web-only station has too, and it is the only
    // one of the three health signals that an outside observer can answer when
    // the box is gone. See `integrations::heartbeat` for why it is a timer and
    // not, as it was, a line inside the per-detection loop.
    if let Some(hb) = integrations.heartbeat.clone() {
        integrations::spawn_heartbeat(hb);
    }

    // Skipped in web-only mode, where a quiet database is expected, not a
    // fault.
    if !cli.web_only {
        spawn_detection_watch(cli, config, state, integrations.apprise.as_ref());
    }
}

/// The alerting that only means something while the station detects.
fn spawn_detection_watch(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    apprise: Option<&integrations::AppriseHandle>,
) {
    // Detection deadman: end-to-end "is the station actually detecting?"
    // freshness gauge + once-per-episode alert. Resolution: CLI/env, then
    // the DEADMAN_HOURS config key, then the 24 h default; 0 disables the
    // alert but keeps the gauge.
    let deadman_hours = cli
        .deadman_hours
        .or_else(|| config?.get_parsed::<u32>("DEADMAN_HOURS").ok())
        .unwrap_or(integrations::DEFAULT_DEADMAN_HOURS);
    integrations::spawn_detection_deadman(state.clone(), apprise.cloned(), deadman_hours);

    // Station health: the operational faults the deadman structurally
    // cannot see, because the station keeps detecting through all of them.
    // Same episode semantics — one alert, one recovery notice, nothing in
    // between. On by default: an unattended station that cannot tell anyone
    // it is degrading is the failure mode this whole subsystem exists for.
    let health_alerts = cli
        .station_health_alerts
        .or_else(|| config?.get_parsed::<bool>("STATION_HEALTH_ALERTS").ok())
        .unwrap_or(true);
    integrations::spawn_station_health(state.clone(), apprise.cloned(), health_alerts);

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
    if let Some(stream_dir) = helpers::stream_dir(cli, config) {
        integrations::spawn_acoustic_health(state.clone(), stream_dir, apprise.cloned());
    }
}

/// The background threads this start runs, held until `serve` returns.
struct Background {
    /// Shared between the stream directory's purge and the detection daemon
    /// (PR-1 / S-3): the segments the pipeline is reading are the ones the
    /// purge must not take.
    in_flight: birdnet_core::detection::daemon::InFlight,
    _data_volume_watch: std::thread::JoinHandle<()>,
    _disk_managers: Vec<std::thread::JoinHandle<()>>,
    _live_spectrogram: Option<std::thread::JoinHandle<()>>,
    _capture: Option<capture::CaptureHandle>,
}

/// Step 9: the data volume's watch, the disk managers, the live spectrogram
/// and the capture supervisor.
fn start_background(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    capture: CaptureHandles,
) -> Background {
    // The data volume's own watch (PS-9, DD-19, DD-20): writable, still
    // mounted, how full — measured now and every minute, read by the health
    // verdict. Nothing else in the process finds out about a read-only remount.
    let data_volume_watch = birdnet_web::data_volume::spawn_watch(state.clone());
    let in_flight = birdnet_core::detection::daemon::InFlight::new();
    let disk_managers = helpers::start_disk_manager(cli, config, state, &in_flight);
    let live_spectrogram = helpers::start_live_spectrogram(cli, config, state);
    let capture = capture::start_capture_manager(
        cli,
        config,
        Some(state),
        state.metrics(),
        capture.status,
        capture.control,
        Some(&capture.live_audio),
    );
    Background {
        in_flight,
        _data_volume_watch: data_volume_watch,
        _disk_managers: disk_managers,
        _live_spectrogram: live_spectrogram,
        _capture: capture,
    }
}

/// The file this start ran on becomes the one a bad edit falls back to. Only
/// a start that validated writes it: a reverted or rejected start must not
/// make its file the "last good" one.
fn keep_last_good(cli: &Cli, decision: &helpers::startup_config::ConfigDecision) {
    if *decision == helpers::startup_config::ConfigDecision::Loaded
        && cli.config.exists()
        && let Err(e) = helpers::startup_config::record_last_good(&cli.config)
    {
        tracing::warn!(
            error = %e,
            path = %helpers::startup_config::last_good_path(&cli.config).display(),
            "could not keep a last-good copy of the configuration; a bad edit will not have one to fall back to"
        );
    }
}

/// What the detector takes from the start-up: where it broadcasts, the
/// clients it delivers through, and the claims the stream purge honours.
struct DetectorInputs {
    broadcast: birdnet_web::routes::websocket::DetectionBroadcast,
    integrations: Integrations,
    in_flight: birdnet_core::detection::daemon::InFlight,
}

/// Step 10: the detector, or the reason there is none, published where the
/// web layer reads it: the health flag the strict health endpoint reads, the
/// classifier panel, and the station's model state (MDL-01 to MDL-16).
fn start_detector(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    config_decision: &helpers::startup_config::ConfigDecision,
    inputs: DetectorInputs,
) -> Option<birdnet_core::detection::daemon::DaemonHandle> {
    let daemon_handle = if cli.web_only {
        tracing::info!("running in web-only mode (no detection daemon)");
        state.metrics().set_detector_state(
            birdnet_core::inference::model_state::DetectorState::WebOnly {
                why: web_only_why(config_decision),
            },
        );
        None
    } else {
        let i = inputs.integrations;
        daemon::start_detection_daemon(
            cli,
            config,
            state.clone(),
            daemon::Delivery {
                broadcast: inputs.broadcast,
                apprise: i.apprise,
                birdweather: i.birdweather,
                email: i.email,
                mqtt: i.mqtt,
                notification_filter: i.notification_filter,
                notification_template: i.notification_template,
            },
            inputs.in_flight,
        )
    };

    // Record whether the detection pipeline actually came up so the health
    // endpoint can surface a station that is silently not detecting (web-only,
    // or a misconfigured model/labels/watch dir) rather than looking healthy.
    state.detection_status_flag().store(
        daemon_handle.is_some(),
        std::sync::atomic::Ordering::Relaxed,
    );
    // What the browser's classifier panel reports: where an install would land,
    // what this start asked the detection engine to run, and whether it came up.
    state
        .classifier_setup()
        .configure(birdnet_web::classifier_setup::Layout {
            model_dir: helpers::models::model_dir(config),
            model: cli
                .model
                .clone()
                .or_else(|| config?.get("MODEL_PATH").map(std::path::PathBuf::from)),
            range_filter: cli
                .metadata_model
                .clone()
                .or_else(|| {
                    config?
                        .get("METADATA_MODEL_PATH")
                        .map(std::path::PathBuf::from)
                })
                .filter(|m| !birdnet_core::inference::model_state::occurrence_filter_turned_off(m)),
            started: daemon_handle.is_some(),
            web_only: cli.web_only,
        });
    // The settings pages tell a change the running station has not picked up
    // from one it has by comparing each row's last write with this moment, which
    // is after the start-up seeding of the table and before anyone can save.
    if let Some(now) = state.with_db(|conn| {
        conn.query_row("SELECT datetime('now')", [], |r| r.get::<_, String>(0))
            .ok()
    }) {
        state.pending_restart().mark_booted(now);
    }
    // ...and keep it true only while the loop thread is alive (PR-5 / OP-2):
    // stored once, the flag reported a daemon that died after boot as running
    // to `?strict=1` for the life of the process.
    if let Some(handle) = daemon_handle.as_ref() {
        daemon::mirror_liveness(
            handle.running_flag(),
            state.detection_status_flag(),
            state.metrics(),
            std::time::Duration::from_secs(5),
        );
    }
    daemon_handle
}

/// Step 11: advertise the station on the local network, and say where its
/// admin pages are open to anyone.
fn announce_access(cli: &Cli, state: &birdnet_web::state::AppState, addr: std::net::SocketAddr) {
    // Register Avahi mDNS service for zero-config local discovery.
    let site_name = cli.site_name.as_deref().unwrap_or("BirdNet-Behavior");
    helpers::maybe_install_avahi_service(addr.port(), site_name);

    // Warn if the admin UI is exposed off-loopback without a password. The
    // predicate is the gate's own, so a password set through the setup wizard
    // or the accounts page counts, as it does for every request: reading only
    // `CADDY_PWD` here announced a lockout on a wizard-protected private
    // station that did not have one. The bootstrap has already put a
    // configured `CADDY_PWD` on the seed admin row.
    let admin_password_configured = birdnet_web::auth_middleware::admin_password_configured(state);
    // O-4: a private station with no password fails closed — every request
    // outside the sign-in and the probe gets a 503 — so say so where the
    // operator will look first. `--doctor` reports the same.
    if !admin_password_configured && state.private_mode() {
        tracing::error!(
            "private mode is on but NO admin password is set: the station will answer 503 to \
             everything except the sign-in and the health probe, and nobody can sign in. Set \
             CADDY_PWD in the config or the environment, or unset BIRDNET_PRIVATE_MODE."
        );
    }
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
}

/// Step 12: the polls and the cache warmer that run for the life of the
/// process.
fn spawn_periodic(
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) {
    // O-23 weather poll loop. Off by default; opt in with
    // `BNB_WEATHER_ENABLED=1`. Spawns a background task that pulls
    // hourly forecast rows from Open-Meteo (or a self-hosted instance
    // via `BNB_WEATHER_BASE_URL`) every 30 minutes and feeds them to
    // the overlay renderers in `birdnet-web::routes::pages::overlays`.
    // The loop runs for the lifetime of the process; dropping its handle
    // detaches it rather than stopping it.
    let _weather_poll_handle = integrations::spawn_weather_poll(config, state.clone());

    // G-27 eBird corroboration. Off unless `EBIRD_API_KEY` is configured —
    // eBird needs a key for every endpoint, so the key is the opt-in and there
    // is no second switch. Publishes what other people reported near the
    // station into `AppState`, where the suspect-species report and the
    // detection detail page consult it.
    let _ebird_poll_handle = integrations::spawn_ebird_poll(config, state.clone());

    // Pre-warm the heavy-analytics fragment cache so the first visit to the
    // Heatmap / phenology / co-occurrence / time-series pages is instant, then
    // keep it warm on an interval a little under the cache TTL (10 min). Each
    // pass runs the same aggregate queries a page visit would, on the blocking
    // pool, so it never stalls the runtime; it is best-effort and decoupled from
    // request handling. The eight-minute cadence keeps the recurring background
    // query load gentle on a Raspberry Pi competing with live detection.
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

/// The certificate this start serves HTTPS with, if any.
struct Certificate {
    /// The server configuration and its resolver; `None` with TLS off or
    /// deferred.
    server: Option<birdnet_web::tls::ConfiguredTls>,
    /// HTTPS is deferred until the clock is set (NT-2).
    deferred_for_clock: bool,
}

/// Step 13: load or mint the certificate, and tell the Access tab.
///
/// Deliberately not at the top of `run`: it writes to disk in self-signed
/// mode, and doing that before the database and configuration have been
/// validated would leave material behind from a start that never completed.
/// A clock that has not been set yet (NT-2) is not a reason to refuse to
/// start, and not a reason to mint a 1970 certificate either: serve plain
/// HTTP for this run, and restart once the clock is set so the next run mints
/// a real one. `Restart=always` and `restart: unless-stopped` bring the
/// process back after a clean exit.
///
/// # Errors
///
/// TLS is enabled and the certificate could not be prepared for any reason
/// but the clock.
fn prepare_certificate(
    tls_plan: &helpers::tls::TlsPlan,
    state: &birdnet_web::state::AppState,
    addr: std::net::SocketAddr,
) -> Result<Certificate, Box<dyn std::error::Error>> {
    let mut deferred_for_clock = false;
    let server = match birdnet_web::tls::server_config(&tls_plan.settings) {
        Ok(v) => v,
        Err(e @ birdnet_web::tls::TlsError::ClockNotSet(_)) => {
            tracing::error!(
                error = %e,
                "HTTPS deferred: serving plain HTTP on {addr} until the clock is set"
            );
            deferred_for_clock = true;
            None
        }
        Err(e) => {
            return Err(format!(
                "TLS is enabled (--tls-mode {}) but the certificate could not be prepared: {e}",
                tls_plan.settings.mode
            )
            .into());
        }
    };
    // What the Access tab's "Secure connection" card says (P-22).
    state.set_https_status(birdnet_web::routes::admin::https::HttpsStatus {
        mode: tls_plan.settings.mode,
        https_addr: tls_plan.https_addr.map(|a| a.to_string()),
        state_dir: tls_plan.settings.state_dir.clone(),
        waiting_for_clock: deferred_for_clock,
    });
    if tls_plan.settings.mode == birdnet_web::tls::TlsMode::SelfSigned {
        tracing::info!(
            ca = %birdnet_web::tls::ca_certificate_path(&tls_plan.settings.state_dir).display(),
            "self-signed HTTPS: import this CA file once to stop the browser warning"
        );
    }
    Ok(Certificate {
        server,
        deferred_for_clock,
    })
}

/// Step 14: Home Assistant discovery, and the MQTT session that carries the
/// station's last will.
fn spawn_mqtt(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
    presence: Option<integrations::MqttHandle>,
) {
    // Publish Home Assistant MQTT auto-discovery if configured.
    if let Some(ref mqtt) = integrations::get_mqtt_client_ref(cli, config) {
        integrations::publish_ha_discovery(mqtt, cli, config);
    }

    // Hold one MQTT connection open carrying a last will, so the "Station
    // Status" entity that discovery has always advertised finally has
    // something behind it. Without this the broker has no session to notice
    // dying, and the entity stays `unknown` for the life of the station —
    // which is the one state an offline alert cannot be built on.
    integrations::spawn_mqtt_presence(state.clone(), presence);
}

/// Step 15: the daily update check (logs result, does not auto-apply).
///
/// Gated because this is the station's only outbound connection that nothing
/// asked for: it fired 60 s after start and every 24 h thereafter with no way
/// to stop it, which is a problem on a metered link and unanswerable during
/// an institutional review. `--no-update-check` / `--offline` turn it off.
fn spawn_update_check(cli: &Cli) {
    if !helpers::egress::update_check_allowed(cli) {
        tracing::info!("daily update check disabled; this station will not contact api.github.com");
        return;
    }
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
}

/// Step 16: periodic database maintenance (VACUUM, integrity check, backup
/// rotation, offsite copies), plus the per-species recording cap and the
/// clip retention, pruned on the daily tick.
fn spawn_maintenance(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    db: &DatabaseStart,
    inputs: MaintenanceInputs,
) {
    // Keep the newest N extracted clips per species on disk. CLI flag wins
    // over the config key; 0 (default) = unlimited. No-op when the DB does not
    // exist yet.
    let species_cap = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
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
            .and_then(|c| c.get_parsed::<u32>("CLIP_RETENTION_DAYS").ok())
            .unwrap_or(0)
    };
    maintenance::spawn_database_maintenance(
        maintenance::MaintenancePlan {
            db_path: db.path.clone(),
            backup_dir: db.backup_dir.clone(),
            recordings_dir: inputs.recordings_dir,
            species_cap,
            clip_retention_days,
            retain_reviewed_audio: cli.retain_reviewed_audio,
            offsite: offsite_plan(cli, config),
            backup_schedule: backup_schedule(config),
        },
        inputs.ingest_halt,
        Some(inputs.metrics),
    );
}

/// Offsite backups. A broken configuration is reported here and the station
/// carries on: it must keep recording birds either way, and `--doctor` is
/// where the operator is told in full.
fn offsite_plan(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
) -> Option<std::sync::Arc<birdnet_integrations::offsite::OffsiteConfig>> {
    match crate::helpers::offsite::plan(cli, config) {
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
    }
}

/// The one cadence an operator chooses. A name nobody implements is reported
/// and the default kept, rather than stopping the station: a typo in a
/// schedule must not cost it its backups entirely.
fn backup_schedule(
    config: Option<&birdnet_core::config::Config>,
) -> birdnet_db::sqlite::BackupSchedule {
    config.and_then(|c| c.get("BACKUP_SCHEDULE")).map_or_else(
        birdnet_db::sqlite::BackupSchedule::default,
        |raw| match birdnet_db::sqlite::BackupSchedule::parse(raw) {
            Ok(s) => s,
            Err(why) => {
                tracing::warn!(%why, "keeping the default weekly backup schedule");
                birdnet_db::sqlite::BackupSchedule::default()
            }
        },
    )
}

/// The sockets this start serves on.
struct Listeners {
    /// Plain HTTP, unless HTTPS shares the socket.
    plain: Option<tokio::net::TcpListener>,
    /// HTTPS and its server configuration.
    https: Option<(tokio::net::TcpListener, birdnet_web::tls::ConfiguredTls)>,
    /// HTTPS is deferred until the clock is set (NT-2).
    deferred_for_clock: bool,
}

/// Step 17: bind every listener the plan calls for before telling systemd
/// we are up.
///
/// `Type=notify` treats READY=1 as "the socket is accepting", and a station
/// that reports ready and then fails to bind 8503 is a worse outcome than one
/// that fails to start at all. With HTTPS deferred, the plain listener is the
/// only way in, even on a plan that would have served HTTPS alone on this
/// port.
///
/// # Errors
///
/// A socket could not be bound.
async fn bind_listeners(
    tls_plan: &helpers::tls::TlsPlan,
    certificate: Certificate,
    addr: std::net::SocketAddr,
) -> Result<Listeners, Box<dyn std::error::Error>> {
    let plain = if tls_plan.wants_plain_listener() || certificate.deferred_for_clock {
        Some(tokio::net::TcpListener::bind(addr).await?)
    } else {
        None
    };
    let https = match (tls_plan.https_addr, certificate.server) {
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
                birdnet_web::tls::spawn_reloader(std::sync::Arc::clone(&resolver), cert, key);
            } else if tls_plan.settings.mode == birdnet_web::tls::TlsMode::SelfSigned {
                // The leaf used to be renewed only at process start (NT-3):
                // a station up past day 397 served an expired certificate.
                birdnet_web::tls::spawn_renewer(
                    std::sync::Arc::clone(&resolver),
                    tls_plan.settings.clone(),
                );
            }
            tracing::info!(
                addr = %https_addr,
                mode = %tls_plan.settings.mode,
                "HTTPS listening"
            );
            Some((listener, (config, resolver)))
        }
        _ => None,
    };
    Ok(Listeners {
        plain,
        https,
        deferred_for_clock: certificate.deferred_for_clock,
    })
}

/// Step 18: serve until a shutdown signal, then drain within the grace.
///
/// One shutdown signal, fanned out: with TLS configured there are two
/// listeners, and both have to drain. The hook also wakes live WebSocket /
/// SSE handlers (dashboard, spectrogram, admin logs) so they close their
/// sockets and the drain finishes in milliseconds, instead of every restart
/// waiting out `SHUTDOWN_GRACE`.
async fn serve_until_shutdown(
    listeners: Listeners,
    app: axum::Router,
    shutdown_state: birdnet_web::state::AppState,
    tls_plan: &helpers::tls::TlsPlan,
    addr: std::net::SocketAddr,
) {
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    if listeners.deferred_for_clock {
        restart_once_the_clock_is_set(shutdown_tx.clone(), shutdown_state.clone());
    }
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

    let https_task = listeners.https.map(|(listener, (config, _resolver))| {
        let app = app.clone();
        let shutdown = signalled(shutdown_tx.subscribe());
        tokio::spawn(async move {
            birdnet_web::tls::serve(listener, config, app, shutdown)
                .await
                .map_err(|e| e.to_string())
        })
    });

    let plain_task = listeners.plain.map(|listener| {
        // With `--tls-redirect` the plain port stops serving the application
        // and answers only "the same URL, over HTTPS". A redirect to a port
        // that is not listening is a dead station: with HTTPS deferred, the
        // plain port serves the application.
        let router = if tls_plan.redirect && !listeners.deferred_for_clock {
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
}

/// Wait for the clock (NTP), then leave cleanly so the supervisor's restart
/// mints the certificate that was deferred. Checked once a minute; the clock
/// moving from 1970 to today is not a thing that half-happens.
fn restart_once_the_clock_is_set(
    tx: tokio::sync::broadcast::Sender<()>,
    state: birdnet_web::state::AppState,
) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if birdnet_core::civil::clock_looks_plausible(now) {
                tracing::warn!(
                    "the clock has been set since boot; restarting to mint the HTTPS \
                     certificate that was deferred"
                );
                state.begin_shutdown();
                let _ = tx.send(());
                return;
            }
        }
    });
}

/// Step 19: stop the detection loop so its event sender drops and the
/// (blocking) event processor returns, letting the runtime wind down cleanly
/// instead of being left to a SIGKILL.
async fn stop_detector(handle: Option<&birdnet_core::detection::daemon::DaemonHandle>) {
    let Some(handle) = handle else {
        return;
    };
    handle.stop();
    // `stop` only signals. Returning at once ended the process under a file
    // mid-analysis, and before the loop could say what it was leaving
    // unanalysed (PIPE8b). Give it a bounded window: SHUTDOWN_GRACE plus this
    // stays inside the unit's TimeoutStopSec=30.
    let deadline = tokio::time::Instant::now() + DAEMON_STOP_GRACE;
    while handle.is_running() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if handle.is_running() {
        tracing::warn!(
            grace_secs = DAEMON_STOP_GRACE.as_secs(),
            "detection loop still analysing at shutdown; exiting without waiting for it"
        );
    }
}

/// Why a web-only station is web-only: the configuration's errors forced it,
/// or it was asked for.
fn web_only_why(decision: &helpers::startup_config::ConfigDecision) -> String {
    if decision.forces_web_only() {
        "the configuration file has errors and there is no last good copy to fall back to; \
         run `birdnet-behavior --doctor` for what to correct"
            .to_owned()
    } else {
        "the station was started with --web-only".to_owned()
    }
}

/// How long shutdown waits for the detection loop to finish the segment it is
/// analysing and exit.
const DAEMON_STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

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

#[cfg(test)]
mod tests {
    use super::{RecoveryFallback, recovery_fallback};
    use birdnet_db::resilience::ResilienceError;

    /// The reproduction, at the decision that acts on it. A good backup that
    /// could not be written must not be treated as no backup at all.
    #[test]
    fn a_restore_that_could_not_be_written_does_not_start_fresh() {
        let e = ResilienceError::RestoreFailed {
            backup: std::path::PathBuf::from("/data/backups/birds.db.backup.20260101-000000"),
            detail: "database or disk is full".to_string(),
        };
        assert_eq!(
            recovery_fallback(&e),
            RecoveryFallback::RefuseToStart,
            "the history is still sitting in that backup; starting fresh discards \
             the one chance to get it back, and on a full card it does not let \
             the station record either"
        );
        assert!(
            e.to_string()
                .contains("/data/backups/birds.db.backup.20260101-000000"),
            "and the operator must be told which file is intact: {e}"
        );
    }

    /// The counterpart, and the reason the fix cannot be "always refuse".
    /// An unattended station with genuinely nothing to restore has to keep
    /// recording rather than sit dead until someone drives out to it.
    #[test]
    fn no_good_backup_still_quarantines_and_starts_fresh() {
        for e in [
            ResilienceError::NoBackup,
            ResilienceError::Unrecoverable("all 5 backup(s) failed verification".to_string()),
            ResilienceError::Io(std::io::Error::other("some other failure")),
        ] {
            assert_eq!(
                recovery_fallback(&e),
                RecoveryFallback::QuarantineAndStartFresh,
                "nothing recoverable exists for {e:?}, so the station must keep \
                 recording rather than refuse to boot"
            );
        }
    }

    /// The steps `serve` is made of can be called and checked on their own
    /// (MDL-19). These pin what the first ones decide: a configuration with
    /// errors and no last good copy forces web-only and says why, a clean one
    /// does not; a database directory that does not exist is made, and the
    /// start knows the database was not there before it.
    mod steps {
        use super::super::{
            backup_schedule, choose_configuration, keep_last_good, prepare_database, web_only_why,
        };
        use crate::cli::Cli;
        use clap::Parser as _;

        fn cli_for(config: &std::path::Path) -> Cli {
            let mut cli = Cli::parse_from(["birdnet-behavior"]);
            cli.config = config.to_path_buf();
            cli
        }

        #[test]
        fn a_configuration_with_errors_and_no_last_good_copy_runs_web_only() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("birdnet.conf");
            let bad = birdnet_core::config::Config::parse("LATITUDE=abc\nLONGITUDE=1\n").unwrap();
            let chosen = choose_configuration(cli_for(&path), Some(bad));
            assert!(chosen.cli.web_only, "{:?}", chosen.decision);
            assert!(
                web_only_why(&chosen.decision).contains("errors"),
                "{}",
                web_only_why(&chosen.decision)
            );

            // Counterpart: a clean configuration detects, and an operator who
            // asked for web-only is told that is why.
            let good = birdnet_core::config::Config::parse("LATITUDE=51.5\nLONGITUDE=0\n").unwrap();
            let chosen = choose_configuration(cli_for(&path), Some(good));
            assert!(!chosen.cli.web_only);
            assert!(web_only_why(&chosen.decision).contains("--web-only"));
        }

        #[test]
        fn only_a_start_that_validated_keeps_a_last_good_copy() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("birdnet.conf");
            std::fs::write(&path, "LATITUDE=abc\n").unwrap();
            let last_good = crate::helpers::startup_config::last_good_path(&path);

            let bad = birdnet_core::config::Config::parse("LATITUDE=abc\n").unwrap();
            let chosen = choose_configuration(cli_for(&path), Some(bad));
            keep_last_good(&chosen.cli, &chosen.decision);
            assert!(
                !last_good.exists(),
                "a rejected file must not become the last good"
            );

            std::fs::write(&path, "LATITUDE=51.5\n").unwrap();
            let good = birdnet_core::config::Config::parse("LATITUDE=51.5\n").unwrap();
            let chosen = choose_configuration(cli_for(&path), Some(good));
            keep_last_good(&chosen.cli, &chosen.decision);
            assert!(last_good.exists());
        }

        #[test]
        fn a_database_directory_that_is_not_there_is_made_and_the_start_knows() {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("deep/data/birds.db");
            let config =
                birdnet_core::config::Config::parse(&format!("DB_PATH={}\n", db.display()))
                    .unwrap();
            let start = prepare_database(Some(&config)).expect("prepared");
            assert_eq!(start.path, db);
            assert!(db.parent().unwrap().is_dir());
            assert!(!start.present_at_start);
            assert_eq!(start.backup_dir, dir.path().join("deep/data/backups"));
        }

        #[test]
        fn a_backup_schedule_nobody_implements_keeps_the_default() {
            let typo =
                birdnet_core::config::Config::parse("BACKUP_SCHEDULE=fortnightly-ish\n").unwrap();
            assert_eq!(
                backup_schedule(Some(&typo)),
                birdnet_db::sqlite::BackupSchedule::default()
            );
            assert_eq!(
                backup_schedule(None),
                birdnet_db::sqlite::BackupSchedule::default()
            );
            // Counterpart: a schedule that exists is kept.
            let daily = birdnet_core::config::Config::parse("BACKUP_SCHEDULE=daily\n").unwrap();
            assert_eq!(
                backup_schedule(Some(&daily)),
                birdnet_db::sqlite::BackupSchedule::Daily
            );
        }
    }
}
