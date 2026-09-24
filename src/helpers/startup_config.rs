//! What the station starts on when its configuration file is wrong, and how a
//! new configuration is applied without a hand on the console (LC-6).
//!
//! The daemon validates its file at start and used to refuse to run on an
//! invalid setting. Validation ran inside the new process, after systemd had
//! killed the old one, so a typo in `LATITUDE` made by someone over SSH took
//! the station down: `Restart=always` with no start limit retried every ten
//! seconds for ever, with no web UI to say why and no way back short of
//! another SSH session. The dry-run validator (`--doctor`) existed and nothing
//! pointed at it.
//!
//! Two things fix that. Every successful start copies the file it ran on to
//! `<config>.last-good`; a start whose file has errors runs on that copy
//! instead, and says so through the boot journal (`config_reverted`). A start
//! with errors and no last-good copy runs web-only on the file as it is
//! (`config_rejected`), so the diagnostics page is reachable and shows the
//! errors. And `--apply-config <file>` is the one-step way to change the
//! file: it validates the candidate, backs up the current file, installs the
//! candidate atomically and restarts the service, so a bad edit is refused
//! before it is in place rather than discovered by the restart loop.

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;
use birdnet_core::config::validate::{Finding, Severity, is_usable, validate};
use birdnet_web::boot_journal::Anomaly;

/// The suffix under which the last configuration a start succeeded on is kept.
pub const LAST_GOOD_SUFFIX: &str = "last-good";

/// `<config_path>.last-good`.
#[must_use]
pub fn last_good_path(config_path: &Path) -> PathBuf {
    let mut name = config_path.as_os_str().to_os_string();
    name.push(".");
    name.push(LAST_GOOD_SUFFIX);
    PathBuf::from(name)
}

/// What a start decided about its configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigDecision {
    /// The file validated (or there is no file); the station runs on it.
    Loaded,
    /// The file has errors and the last-good copy validated; the station
    /// runs on the copy.
    Reverted {
        /// The errors, as `key: message`.
        errors: Vec<String>,
        /// The copy the station is running on.
        last_good: PathBuf,
    },
    /// The file has errors and there is no usable last-good copy; the
    /// station runs web-only on the file as it is, so the diagnostics are
    /// reachable.
    Rejected {
        /// The errors, as `key: message`.
        errors: Vec<String>,
    },
}

impl ConfigDecision {
    /// The boot-journal anomaly this decision is, if it is one.
    #[must_use]
    pub fn anomaly(&self) -> Option<Anomaly> {
        match self {
            Self::Loaded => None,
            Self::Reverted { errors, last_good } => Some(Anomaly::ConfigReverted {
                errors: errors.clone(),
                last_good: last_good.display().to_string(),
            }),
            Self::Rejected { errors } => Some(Anomaly::ConfigRejected {
                errors: errors.clone(),
            }),
        }
    }

    /// Whether the station must run without the detection daemon.
    #[must_use]
    pub const fn forces_web_only(&self) -> bool {
        matches!(self, Self::Rejected { .. })
    }
}

/// The errors among `findings`, as `key: message`.
fn errors_of(findings: &[Finding]) -> Vec<String> {
    findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .map(|f| format!("{}: {}", f.key, f.message))
        .collect()
}

/// Decide what to run on: `loaded` when it validates, the last-good copy
/// when it does not and the copy does, the file as it is (web-only) when
/// neither. Pure apart from reading the last-good file.
#[must_use]
pub fn choose(loaded: Option<Config>, config_path: &Path) -> (Option<Config>, ConfigDecision) {
    let Some(cfg) = loaded else {
        return (None, ConfigDecision::Loaded);
    };
    let findings = validate(&cfg);
    if is_usable(&findings) {
        return (Some(cfg), ConfigDecision::Loaded);
    }
    let errors = errors_of(&findings);
    let last_good = last_good_path(config_path);
    if let Ok(copy) = Config::load_from(&last_good)
        && is_usable(&validate(&copy))
    {
        let copy = keep_access_as_tight_as(copy, &cfg);
        return (Some(copy), ConfigDecision::Reverted { errors, last_good });
    }
    (Some(cfg), ConfigDecision::Rejected { errors })
}

/// The last-good `copy`, with the file on disk's access controls wherever
/// they are the tighter of the two.
///
/// A revert exists to keep a typo from stopping the station, not to undo a
/// lock-down made in the same edit: the last-good file may predate the
/// operator turning private mode on or setting a password. So:
///
/// - private mode on in either file is on. When only the file on disk turns
///   it on, its carve-out list (`PUBLIC_ACCESS`) applies — none, if it names
///   none; when both do, only the carve-outs both name;
/// - a password the file on disk sets is the password.
///
/// Nothing here can make the station more open than the copy alone would.
fn keep_access_as_tight_as(mut copy: Config, on_disk: &Config) -> Config {
    let on = |c: &Config| {
        c.get("PRIVATE_MODE").is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
    };
    if on(on_disk) && !on(&copy) {
        copy.set(
            "PRIVATE_MODE",
            on_disk.get("PRIVATE_MODE").unwrap_or("true"),
        );
        copy.set("PUBLIC_ACCESS", on_disk.get("PUBLIC_ACCESS").unwrap_or(""));
    } else if on(on_disk) {
        // Both private: only the carve-outs both files open.
        let names = |c: &Config| -> Vec<String> {
            c.get("PUBLIC_ACCESS")
                .unwrap_or("")
                .split(',')
                .map(|n| n.trim().to_ascii_lowercase().replace('-', "_"))
                .filter(|n| !n.is_empty())
                .collect()
        };
        let disk = names(on_disk);
        let both: Vec<String> = names(&copy)
            .into_iter()
            .filter(|n| disk.contains(n))
            .collect();
        copy.set("PUBLIC_ACCESS", both.join(","));
    }
    if let Some(pwd) = on_disk.get("CADDY_PWD").filter(|p| !p.trim().is_empty()) {
        copy.set("CADDY_PWD", pwd);
    }
    copy
}

/// What a start with errors in its file would do, for the doctor to say
/// before the start: `Some(path)` when a usable last-good copy exists,
/// `None` when the station would run web-only.
#[must_use]
pub fn recovery_for(config_path: &Path) -> Option<PathBuf> {
    let last_good = last_good_path(config_path);
    Config::load_from(&last_good)
        .ok()
        .filter(|copy| is_usable(&validate(copy)))
        .map(|_| last_good)
}

/// Keep the file the station just started on as the last-good copy, whole
/// or not at all.
///
/// # Errors
///
/// The I/O error when the copy cannot be written; the caller logs it, the
/// station keeps running.
pub fn record_last_good(config_path: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(config_path)?;
    let dest = last_good_path(config_path);
    let part = last_good_path(config_path).with_extension("last-good.part");
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, &dest)
}

/// Replace `target` with `bytes` atomically, keeping its mode **and its
/// owner and group**.
///
/// `--apply-config` is documented to run under `sudo`, so the new file is
/// created by root. Copying only the mode turned the installer's
/// `root:<service user> 0640` into `root:root 0640`: the service could no
/// longer read its own configuration, `--doctor` in `ExecStartPre` failed
/// with the wrong reason, and the unit never started — with no web UI to
/// say why. If the ownership cannot be carried over, nothing is installed.
fn install_config(bytes: &[u8], target: &Path) -> std::io::Result<()> {
    let part = PathBuf::from(format!("{}.part", target.display()));
    let result = std::fs::write(&part, bytes).and_then(|()| {
        if let Ok(meta) = std::fs::metadata(target) {
            std::fs::set_permissions(&part, meta.permissions())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt as _;
                std::os::unix::fs::chown(&part, Some(meta.uid()), Some(meta.gid()))?;
            }
        }
        std::fs::rename(&part, target)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

/// `--apply-config <candidate>`: validate, back up, install, restart.
/// Returns the process exit code: `0` applied, `2` refused.
pub fn run_apply_config(candidate: &Path, target: &Path) -> i32 {
    let text = match std::fs::read_to_string(candidate) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {}: {e}", candidate.display());
            return 2;
        }
    };
    let cfg = match Config::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} is not a configuration file: {e}", candidate.display());
            return 2;
        }
    };
    let findings = validate(&cfg);
    for f in &findings {
        println!(
            "{}: {}: {} ({})",
            f.severity, f.key, f.message, f.remediation
        );
    }
    if !is_usable(&findings) {
        eprintln!(
            "not applied: {} has {} error(s); {} is unchanged",
            candidate.display(),
            errors_of(&findings).len(),
            target.display()
        );
        return 2;
    }

    if target.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup = PathBuf::from(format!("{}.bak-{stamp}", target.display()));
        if let Err(e) = std::fs::copy(target, &backup) {
            eprintln!(
                "cannot back up {} to {}: {e}",
                target.display(),
                backup.display()
            );
            return 2;
        }
        println!("previous configuration kept at {}", backup.display());
    }
    if let Err(e) = install_config(text.as_bytes(), target) {
        eprintln!("cannot install {}: {e}", target.display());
        return 2;
    }
    println!("installed {}", target.display());

    match birdnet_core::process::run_with_timeout(
        std::process::Command::new("systemctl").args(["restart", "birdnet-behavior"]),
        std::time::Duration::from_secs(90),
    ) {
        Ok(out) if out.status.success() => {
            println!(
                "birdnet-behavior restarted; if it fails to start on this file it will run on the last good one and say so at /station"
            );
        }
        Ok(out) => {
            println!(
                "systemctl restart birdnet-behavior exited {} ({}); restart the service yourself",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Err(e) => {
            println!("could not run systemctl ({e}); restart the service yourself");
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "LATITUDE=42.36\nLONGITUDE=-71.06\nSITENAME=Good\n";
    const BAD: &str = "LATITUDE=abc\nLONGITUDE=-71.06\nSITENAME=Bad\n";

    /// The installed file keeps the owner and group of the one it replaces.
    ///
    /// Needs root, because that is the situation (`sudo --apply-config`) and
    /// only root can hand a file to another user. Without it the test says so
    /// and passes; the unit test above still covers the content and mode.
    #[cfg(unix)]
    #[test]
    fn an_applied_config_keeps_the_owner_the_service_reads_it_as() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("birdnet.conf");
        std::fs::write(&target, GOOD).expect("write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).expect("mode");
        // The service user, as the installer leaves it: some other uid/gid.
        if let Err(e) = std::os::unix::fs::chown(&target, Some(65_534), Some(65_534)) {
            eprintln!("SKIP: cannot hand a file to another user without root ({e})");
            return;
        }

        install_config(GOOD.replace("Good", "Better").as_bytes(), &target).expect("install");

        let meta = std::fs::metadata(&target).expect("meta");
        assert_eq!(
            (meta.uid(), meta.gid()),
            (65_534, 65_534),
            "the service could no longer read its own configuration"
        );
        assert_eq!(meta.permissions().mode() & 0o777, 0o640);
        assert!(std::fs::read_to_string(&target).unwrap().contains("Better"));
    }

    /// The gate for LC-6's start-side half: a file with errors reverts to the
    /// last-good copy when there is one and runs web-only when there is not;
    /// a good file is loaded as it is.
    #[test]
    fn a_bad_file_reverts_to_the_last_good_copy_or_runs_web_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("birdnet.conf");

        let (cfg, decision) = choose(Some(Config::parse(GOOD).unwrap()), &path);
        assert_eq!(decision, ConfigDecision::Loaded);
        assert_eq!(cfg.unwrap().get("SITENAME"), Some("Good"));
        assert!(decision.anomaly().is_none());
        assert!(!decision.forces_web_only());

        let (cfg, decision) = choose(Some(Config::parse(BAD).unwrap()), &path);
        assert!(
            matches!(decision, ConfigDecision::Rejected { .. }),
            "no last-good copy: {decision:?}"
        );
        assert!(decision.forces_web_only());
        assert_eq!(
            cfg.unwrap().get("SITENAME"),
            Some("Bad"),
            "runs on the file as it is"
        );
        assert_eq!(decision.anomaly().map(|a| a.key()), Some("config_rejected"));

        std::fs::write(&path, GOOD).unwrap();
        record_last_good(&path).unwrap();
        assert!(last_good_path(&path).exists());
        assert_eq!(recovery_for(&path), Some(last_good_path(&path)));

        let (cfg, decision) = choose(Some(Config::parse(BAD).unwrap()), &path);
        assert!(
            matches!(&decision, ConfigDecision::Reverted { errors, .. } if errors.iter().any(|e| e.starts_with("LATITUDE"))),
            "{decision:?}"
        );
        assert!(!decision.forces_web_only());
        assert_eq!(
            cfg.unwrap().get("SITENAME"),
            Some("Good"),
            "the station must run on the last good file"
        );
        assert_eq!(decision.anomaly().map(|a| a.key()), Some("config_reverted"));

        // A last-good copy that is itself bad is no recovery.
        std::fs::write(last_good_path(&path), BAD).unwrap();
        assert_eq!(recovery_for(&path), None);
        let (_, decision) = choose(Some(Config::parse(BAD).unwrap()), &path);
        assert!(matches!(decision, ConfigDecision::Rejected { .. }));
    }

    /// Reverting runs the last-good file, and that file may predate the
    /// operator locking the station down. An edit that turned private mode on
    /// and set a password, with a typo somewhere else, brought the station up
    /// open and without the password — the typo alone decided who could reach
    /// it. The access keys are carried from the file on disk in the one
    /// direction that is always safe: never less private, never without a
    /// password the file sets.
    #[test]
    fn a_revert_is_never_less_protected_than_the_file_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("birdnet.conf");
        std::fs::write(&path, format!("{GOOD}PUBLIC_ACCESS=share,metrics\n")).unwrap();
        record_last_good(&path).unwrap();

        let locked = format!("{BAD}PRIVATE_MODE=true\nCADDY_PWD=new-secret\n");
        let (cfg, decision) = choose(Some(Config::parse(&locked).unwrap()), &path);
        assert!(
            matches!(decision, ConfigDecision::Reverted { .. }),
            "{decision:?}"
        );
        let cfg = cfg.unwrap();
        assert_eq!(cfg.get("SITENAME"), Some("Good"), "everything else reverts");
        assert_eq!(cfg.get("PRIVATE_MODE"), Some("true"));
        assert_eq!(cfg.get("CADDY_PWD"), Some("new-secret"));
        assert!(
            cfg.get("PUBLIC_ACCESS").is_none_or(str::is_empty),
            "the file on disk opens no carve-out: {:?}",
            cfg.get("PUBLIC_ACCESS")
        );

        // A last-good file that was private stays private under a broken edit
        // that turns it off.
        std::fs::write(&path, format!("{GOOD}PRIVATE_MODE=yes\nCADDY_PWD=old\n")).unwrap();
        record_last_good(&path).unwrap();
        let opened = format!("{BAD}PRIVATE_MODE=false\n");
        let (cfg, _) = choose(Some(Config::parse(&opened).unwrap()), &path);
        let cfg = cfg.unwrap();
        assert_eq!(cfg.get("PRIVATE_MODE"), Some("yes"));
        assert_eq!(cfg.get("CADDY_PWD"), Some("old"));

        // Both private: a broken edit cannot widen the carve-outs.
        std::fs::write(
            &path,
            format!("{GOOD}PRIVATE_MODE=true\nPUBLIC_ACCESS=share\n"),
        )
        .unwrap();
        record_last_good(&path).unwrap();
        let wider = format!("{BAD}PRIVATE_MODE=true\nPUBLIC_ACCESS=share,live_audio,metrics\n");
        let (cfg, _) = choose(Some(Config::parse(&wider).unwrap()), &path);
        assert_eq!(cfg.unwrap().get("PUBLIC_ACCESS"), Some("share"));

        // Counterpart: a broken edit that touches no access key reverts to the
        // copy exactly.
        std::fs::write(&path, format!("{GOOD}PUBLIC_ACCESS=share\n")).unwrap();
        record_last_good(&path).unwrap();
        let (cfg, _) = choose(Some(Config::parse(BAD).unwrap()), &path);
        let cfg = cfg.unwrap();
        assert_eq!(cfg.get("PRIVATE_MODE"), None);
        assert_eq!(cfg.get("PUBLIC_ACCESS"), Some("share"));
    }

    /// `--apply-config` refuses a candidate with errors and leaves the target
    /// alone; installs a good one behind a backup.
    #[test]
    fn apply_config_refuses_a_bad_candidate_and_installs_a_good_one_behind_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("birdnet.conf");
        std::fs::write(&target, GOOD).unwrap();
        let bad = dir.path().join("bad.conf");
        std::fs::write(&bad, BAD).unwrap();
        assert_eq!(run_apply_config(&bad, &target), 2);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            GOOD,
            "refused: unchanged"
        );

        let better = dir.path().join("better.conf");
        std::fs::write(&better, "LATITUDE=51.5\nLONGITUDE=-0.12\nSITENAME=Better\n").unwrap();
        assert_eq!(run_apply_config(&better, &target), 0);
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("SITENAME=Better"),
            "installed"
        );
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("birdnet.conf.bak-"))
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "one backup of the previous file: {backups:?}"
        );
        assert!(!dir.path().join("birdnet.conf.part").exists());
    }
}
