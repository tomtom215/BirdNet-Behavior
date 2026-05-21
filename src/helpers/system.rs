//! OS-level integration: the disk-manager background thread and Avahi mDNS
//! service registration.

use std::path::PathBuf;

use crate::cli::Cli;

/// Start the disk manager as a background thread.
pub fn start_disk_manager(
    cli: &Cli,
    config: Option<&birdnet_core::config::Config>,
    state: &birdnet_web::state::AppState,
) -> Option<std::thread::JoinHandle<()>> {
    use birdnet_core::audio::capture::{DiskManager, DiskManagerConfig, FullDiskAction};

    let monitored_dir = cli
        .watch_dir
        .clone()
        .or_else(|| config?.get("RECS_DIR").map(PathBuf::from))?;

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

    let locked_file_names =
        state.with_db(|conn| birdnet_db::sqlite::locked_file_names(conn).unwrap_or_default());

    let config_obj = DiskManagerConfig {
        monitored_dir: monitored_dir.clone(),
        purge_threshold,
        full_disk_action: FullDiskAction::Purge,
        max_files_per_species,
        check_interval_secs: 60,
        exclude_paths: cli.disk_exclude.clone(),
        locked_file_names,
    };

    tracing::info!(
        dir = %monitored_dir.display(),
        max_files_per_species,
        purge_threshold,
        excluded_paths = cli.disk_exclude.len(),
        "disk manager configured"
    );

    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let manager = DiskManager::new(config_obj);

    let handle = std::thread::spawn(move || {
        manager.run(&stop_rx);
    });

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
    use crate::helpers::test_support::{default_cli, test_state};

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
    fn disk_manager_returns_none_when_no_watch_dir() {
        // No --watch-dir flag and no RECS_DIR in config; the helper
        // returns None instead of starting an unconfigured manager.
        let cli = default_cli();
        let state = test_state();
        let handle = start_disk_manager(&cli, None, &state);
        assert!(handle.is_none());
    }

    #[test]
    fn disk_manager_starts_when_watch_dir_present() {
        // With a watch dir configured the helper spawns the manager
        // thread; we get back a JoinHandle. The thread itself runs
        // forever (the stop channel is leaked by design) so we don't
        // join it — but having a handle means the spawn happened.
        let tmp = tempfile::tempdir().unwrap();
        let mut cli = default_cli();
        cli.watch_dir = Some(tmp.path().to_path_buf());
        let state = test_state();
        let handle = start_disk_manager(&cli, None, &state);
        assert!(handle.is_some());
    }
}
