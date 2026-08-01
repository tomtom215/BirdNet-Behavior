//! OS-level integration: the disk-manager background thread and Avahi mDNS
//! service registration.

use std::path::PathBuf;

use crate::cli::Cli;

/// Default retention for raw capture segments in the transient stream dir, in
/// seconds. Segments are deleted this long after capture — far longer than the
/// detection pipeline needs to read and extract them — so the RAM-backed stream
/// dir (`--watch-dir`, typically `/tmp/birdnet-stream`) self-drains instead of
/// filling to 100 %. Override with `STREAM_RETENTION_SECS` in the config.
const DEFAULT_STREAM_RETENTION_SECS: u64 = 600;

/// Default hard ceiling on the transient stream dir, in mebibytes — a backstop
/// for many-stream / backed-up runs; oldest segments drop first. Override with
/// `STREAM_MAX_MB` in the config.
const DEFAULT_STREAM_MAX_MB: u64 = 512;

/// Start the disk managers as background threads — one per directory that can
/// fill up.
///
/// A station has **two** growing directories, on two different filesystems,
/// with opposite retention rules, and both have to be bounded:
///
///   * the **transient stream dir** (`--watch-dir`, typically a RAM-backed
///     tmpfs) holds raw capture segments the pipeline reads and never deletes.
///     It is drained by age and size — nothing in it is worth keeping.
///   * the **persistent recordings dir** ([`AppState::recording_dir`]) holds
///     the extracted detection clips. Those are the operator's data, so they
///     are *never* age-purged; only the disk-full backstop touches them, oldest
///     first, skipping anything locked.
///
/// Supervising only one of them was a real gap: the bare-metal installer always
/// passes `--watch-dir`, so the manager attached to the tmpfs and the data disk
/// — where clips actually accumulate, alongside `birds.db` — had no bound at
/// all. `DISK_PURGE_THRESHOLD` appeared to protect the recordings (as
/// `docs/FIELD_DEPLOYMENT.md` describes) while in fact only ever measuring the
/// tmpfs. Left alone, a 24/7 station fills its SD card until SQLite writes
/// start failing.
///
/// Returns one join handle per supervised directory (empty when nothing is
/// configured).
pub fn start_disk_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Vec<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::capture::{DiskManagerConfig, FullDiskAction, LockedFilesProvider};

    let max_files_per_species = if cli.max_files_per_species > 0 {
        cli.max_files_per_species
    } else {
        config
            .and_then(|c| c.get_parsed::<u32>("MAX_FILES_SPECIES").ok())
            .unwrap_or(0)
    };

    let purge_threshold = config
        .and_then(|c| c.get_parsed::<u8>("DISK_PURGE_THRESHOLD").ok())
        .unwrap_or(95);

    // Re-read once per purge cycle rather than snapshotted here: `/admin/recordings`
    // → "lock" writes `is_locked` at runtime, so a set captured at startup
    // protects only what was already locked at the last reboot — and silently
    // lets the purge delete everything locked since.
    let locked_provider: LockedFilesProvider = {
        let state = state.clone();
        std::sync::Arc::new(move || {
            state.with_db(|conn| birdnet_db::sqlite::locked_file_names(conn).unwrap_or_default())
        })
    };

    let mut handles = Vec::new();

    // ── Transient stream dir: drain by age and size ────────────────────────
    if let Some(stream_dir) = cli.watch_dir.clone() {
        let retention = config
            .and_then(|c| c.get_parsed::<u64>("STREAM_RETENTION_SECS").ok())
            .unwrap_or(DEFAULT_STREAM_RETENTION_SECS);
        let max_mb = config
            .and_then(|c| c.get_parsed::<u64>("STREAM_MAX_MB").ok())
            .unwrap_or(DEFAULT_STREAM_MAX_MB);
        handles.push(spawn_manager(
            DiskManagerConfig {
                monitored_dir: stream_dir,
                purge_threshold,
                full_disk_action: FullDiskAction::Purge,
                // The per-species cap walks `By_Date/<date>/<species>/`, a
                // layout the flat stream dir never has. It is enforced from the
                // database instead (see `crate::maintenance`), against the
                // recordings dir where the clips really live.
                max_files_per_species: 0,
                check_interval_secs: 60,
                exclude_paths: cli.disk_exclude.clone(),
                locked_file_names: Vec::new(),
                locked_provider: Some(std::sync::Arc::clone(&locked_provider)),
                stream_retention_secs: retention,
                stream_max_bytes: max_mb.saturating_mul(1024 * 1024),
            },
            "stream",
        ));
    }

    // ── Persistent recordings dir: disk-full backstop only ─────────────────
    //
    // `AppState::recording_dir` is deliberately the source of truth here: it is
    // the exact directory the detection daemon extracts clips into and the web
    // serves them from. Keying off the config's `RECS_DIR` instead would
    // monitor a directory nothing necessarily writes to.
    let recordings_dir = state.recording_dir();
    let already_monitored = cli.watch_dir.as_ref().is_some_and(|w| *w == recordings_dir);
    if !already_monitored {
        // `disk_usage` shells out to `df`, which fails on a path that does not
        // exist — and the recordings dir is only created lazily by the first
        // extraction. Create it now so the backstop reports real numbers from
        // the first cycle instead of logging an error every minute until the
        // station's first detection.
        if let Err(e) = std::fs::create_dir_all(&recordings_dir) {
            tracing::warn!(
                dir = %recordings_dir.display(),
                error = %e,
                "could not create the recordings directory; disk monitoring for it is disabled"
            );
        } else {
            handles.push(spawn_manager(
                DiskManagerConfig {
                    monitored_dir: recordings_dir,
                    purge_threshold,
                    full_disk_action: FullDiskAction::Purge,
                    max_files_per_species,
                    check_interval_secs: 60,
                    exclude_paths: cli.disk_exclude.clone(),
                    locked_file_names: Vec::new(),
                    locked_provider: Some(locked_provider),
                    // Never age- or size-drain the operator's clips: they are
                    // only ever removed by the disk-full backstop above, and
                    // then oldest-first and never if locked.
                    stream_retention_secs: 0,
                    stream_max_bytes: 0,
                },
                "recordings",
            ));
        }
    }

    handles
}

/// Spawn one disk-manager thread for `config`, logging what it will do.
fn spawn_manager(
    config: birdnet_core::audio::capture::DiskManagerConfig,
    role: &'static str,
) -> std::thread::JoinHandle<()> {
    use birdnet_core::audio::capture::DiskManager;

    tracing::info!(
        role,
        dir = %config.monitored_dir.display(),
        max_files_per_species = config.max_files_per_species,
        purge_threshold = config.purge_threshold,
        stream_retention_secs = config.stream_retention_secs,
        stream_max_bytes = config.stream_max_bytes,
        excluded_paths = config.exclude_paths.len(),
        "disk manager configured"
    );

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let manager = DiskManager::new(config);
    let handle = std::thread::spawn(move || {
        manager.run(&stop_rx);
    });
    // The manager should run for the lifetime of the process; leaking the
    // sender keeps `recv_timeout` from seeing a disconnect and stopping.
    std::mem::forget(stop_tx);
    handle
}

/// Start the live-spectrogram producer as a background thread.
///
/// Watches the capture pipeline's recording directory; for each new
/// audio file, computes a mel spectrogram and broadcasts the frame to
/// every `/api/v2/ws/spectrogram` client via the `AppState`'s
/// `SpectrogramBroadcast`. Returns `None` when no watch directory is
/// configured (matches the basic-station "audio only via REST upload"
/// path, which also has no detection daemon to feed the producer).
pub fn start_live_spectrogram(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Option<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::spectrogram::live::{LiveSpectrogramConfig, SpectrogramFrame, run};
    use birdnet_web::routes::spectrogram_ws::WsSpectrogramEvent;

    let watch_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))?;

    let broadcast = state.spectrogram_broadcast();
    let cfg = LiveSpectrogramConfig {
        watch_dir: watch_dir.clone(),
        ..LiveSpectrogramConfig::default()
    };

    tracing::info!(
        dir = %watch_dir.display(),
        n_mels = cfg.mel_config.n_mels,
        max_frames = cfg.max_frames,
        "live spectrogram producer configured"
    );

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();

    let handle = std::thread::spawn(move || {
        let on_frame = move |frame: SpectrogramFrame| {
            // Translate the core `SpectrogramFrame` into the
            // wire-shaped `WsSpectrogramEvent` the broadcast carries.
            let event = WsSpectrogramEvent {
                event: "spectrogram",
                filename: frame.filename,
                n_mels: frame.n_mels,
                n_frames: frame.n_frames,
                data: frame.data,
                sample_rate: frame.sample_rate,
            };
            broadcast.send(&event);
        };
        if let Err(e) = run(&cfg, on_frame, &stop_rx) {
            tracing::warn!(error = %e, "live spectrogram daemon exited with error");
        }
    });

    // The receiver loop in `run` only stops when stop_tx is dropped /
    // sends. We want the daemon to run for the lifetime of the process,
    // so leak the sender (matches the disk-manager pattern above).
    std::mem::forget(stop_tx);
    Some(handle)
}

/// Generate an Avahi mDNS service file for local network discovery.
pub fn maybe_install_avahi_service(port: u16, site_name: &str) {
    let avahi_dir = std::path::Path::new("/etc/avahi/services");
    if !avahi_dir.exists() {
        return;
    }

    let service_file = avahi_dir.join("birdnet-behavior.service");
    if service_file.exists() {
        return;
    }

    let name = if site_name.is_empty() || site_name == "BirdNet-Behavior" {
        "BirdNet-Behavior".to_string()
    } else {
        site_name.to_string()
    };

    let xml = format!(
        r#"<?xml version="1.0" standalone='no'?>
<!DOCTYPE service-group SYSTEM "avahi-service.dtd">
<service-group>
  <name replace-wildcards="yes">{name} on %h</name>
  <service>
    <type>_http._tcp</type>
    <port>{port}</port>
    <txt-record>path=/</txt-record>
    <txt-record>software=BirdNet-Behavior</txt-record>
  </service>
</service-group>
"#
    );

    match std::fs::write(&service_file, xml) {
        Ok(()) => tracing::info!(
            path = %service_file.display(),
            "Avahi mDNS service file written — station discoverable as birdnet.local"
        ),
        Err(e) => tracing::debug!(
            error = %e,
            "Could not write Avahi service file (non-fatal, run as root to enable mDNS)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{maybe_install_avahi_service, start_disk_manager};
    use crate::helpers::test_support::{default_cli, test_state_in};

    #[test]
    fn avahi_is_noop_when_target_dir_absent() {
        // /etc/avahi/services rarely exists inside CI containers and
        // this function returns early without trying to write — the
        // test pins that "did not panic, did not write" contract.
        maybe_install_avahi_service(8502, "TestStation");
        // No assertion needed beyond "did not panic"; the early-return
        // path is the entire surface for an unprivileged caller.
    }

    #[test]
    fn recordings_dir_is_supervised_even_without_a_watch_dir() {
        // The persistent recordings directory always gets a manager: it holds
        // the operator's clips next to birds.db, so it is the directory whose
        // filling up takes the station down. With no --watch-dir there is no
        // transient stream dir, so exactly one manager runs.
        let tmp = tempfile::tempdir().unwrap();
        let cli = default_cli();
        let state = test_state_in(tmp.path());
        let handles = start_disk_manager(&cli, None, &state);
        assert_eq!(handles.len(), 1, "the recordings dir must be supervised");
        assert!(
            state.recording_dir().is_dir(),
            "the recordings dir is created so `df` can measure it from cycle one"
        );
    }

    #[test]
    fn both_dirs_are_supervised_when_a_watch_dir_is_set() {
        // The regression this pins: the bare-metal installer always passes
        // --watch-dir, and the old helper monitored *only* that (the tmpfs),
        // leaving the data disk where clips accumulate completely unbounded.
        let tmp = tempfile::tempdir().unwrap();
        let stream = tmp.path().join("stream");
        std::fs::create_dir_all(&stream).unwrap();
        let mut cli = default_cli();
        cli.watch_dir = Some(stream);
        let state = test_state_in(tmp.path());
        let handles = start_disk_manager(&cli, None, &state);
        assert_eq!(
            handles.len(),
            2,
            "the stream dir AND the recordings dir must both be supervised"
        );
    }

    #[test]
    fn one_manager_when_watch_dir_is_the_recordings_dir() {
        // Counter-test for the de-duplication: pointing --watch-dir at the
        // recordings dir must not start two managers racing on one directory.
        let tmp = tempfile::tempdir().unwrap();
        let state = test_state_in(tmp.path());
        let mut cli = default_cli();
        cli.watch_dir = Some(state.recording_dir());
        let handles = start_disk_manager(&cli, None, &state);
        assert_eq!(handles.len(), 1, "the same directory is supervised once");
    }
}
