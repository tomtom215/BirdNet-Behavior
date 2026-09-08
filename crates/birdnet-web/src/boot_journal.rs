//! A record of each start, kept outside the database, and what changed since
//! the last one (UP-3).
//!
//! A data volume that fails to mount leaves the station starting on an empty
//! directory on the boot disk, which looks exactly like a first run: a fresh
//! database, zero detections, every chart empty from here on. Nothing in the
//! database can say "you used to have 40 000 detections", because the
//! database is the thing that went missing. So the fact is written somewhere
//! else — the configuration directory when it can take the file, else beside
//! the database — and read back at the next start. `birdnet-go` keeps the same
//! journal (`internal/diagnostics`) and diffs consecutive boots for the same
//! four anomalies.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The journal's file name.
pub const FILE_NAME: &str = "boot-journal.json";

/// What one start saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootRecord {
    /// The binary's version.
    pub version: String,
    /// The database path the station was started with.
    pub db_path: String,
    /// Whether the database file existed at start.
    pub db_present: bool,
    /// Detection rows at start, `0` when the file was absent.
    pub detections: i64,
    /// Whether the data directory was its own mount; `None` when unknown.
    pub separate_mount: Option<bool>,
    /// Seconds since the Unix epoch.
    pub booted_at: u64,
}

/// Something that changed between two consecutive starts and should not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Anomaly {
    /// The database held detections at the last start and holds none, or is
    /// absent, now.
    DbLost {
        /// Rows at the last start.
        before: i64,
    },
    /// The station was started with a different database path.
    DbPathChanged {
        /// The previous path.
        from: String,
        /// This start's path.
        to: String,
    },
    /// The data directory was its own mount at the last start and is not
    /// now: the volume did not mount, and the station is on the disk beneath.
    MountLost,
    /// The binary is older than the one that last started.
    VersionRollback {
        /// The previous version.
        from: String,
        /// This start's version.
        to: String,
    },
}

impl Anomaly {
    /// The short key the health endpoint and the metric carry.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::DbLost { .. } => "db_lost",
            Self::DbPathChanged { .. } => "db_path_changed",
            Self::MountLost => "mount_lost",
            Self::VersionRollback { .. } => "version_rollback",
        }
    }

    /// What happened, for a person.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::DbLost { before } => format!(
                "the database held {before} detections at the last start and holds none now; \
                 if the data volume did not mount, everything is being written to the boot \
                 disk and the history is on the unmounted card"
            ),
            Self::DbPathChanged { from, to } => {
                format!("the station was started with database {to}; the last start used {from}")
            }
            Self::MountLost => "the data directory was its own mount at the last start and is \
                                not now: the data volume did not mount, and the station is \
                                writing to the disk beneath it"
                .to_owned(),
            Self::VersionRollback { from, to } => format!(
                "the binary is {to}, older than the {from} that last started; a downgrade \
                 does not know the newer schema"
            ),
        }
    }
}

/// Where the journal lives: the configuration directory when it exists and
/// takes a write, else `data_dir`.
///
/// The first survives the data volume failing to mount, which is the case the
/// journal exists for; the second is what a container with no configuration
/// directory gets.
#[must_use]
pub fn journal_path(config_path: &Path, data_dir: &Path) -> PathBuf {
    if let Some(config_dir) = config_path.parent()
        && config_dir.is_dir()
        && writable(config_dir)
    {
        return config_dir.join(FILE_NAME);
    }
    data_dir.join(FILE_NAME)
}

/// Whether a file can be created in `dir`.
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".bnb-journal-probe-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// The last start's record, or `None` for a first start or an unreadable
/// file (which is logged: a journal that cannot be read is worth a line, but
/// not a refusal to start).
#[must_use]
pub fn read(path: &Path) -> Option<BootRecord> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "boot journal could not be read");
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(record) => Some(record),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "boot journal is not readable; treating this as a first start");
            None
        }
    }
}

/// Write this start's record, whole or not at all.
///
/// # Errors
///
/// The I/O error, when the directory cannot take the file.
pub fn write(path: &Path, record: &BootRecord) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(record).map_err(std::io::Error::other)?;
    let part = path.with_extension("json.part");
    std::fs::write(&part, text)?;
    std::fs::rename(&part, path)
}

/// What changed between `previous` and `now` that should not have.
#[must_use]
pub fn compare(previous: &BootRecord, now: &BootRecord) -> Vec<Anomaly> {
    let mut out = Vec::new();
    if previous.db_path != now.db_path {
        out.push(Anomaly::DbPathChanged {
            from: previous.db_path.clone(),
            to: now.db_path.clone(),
        });
    } else if previous.db_present
        && previous.detections > 0
        && (!now.db_present || now.detections == 0)
    {
        out.push(Anomaly::DbLost {
            before: previous.detections,
        });
    }
    if previous.separate_mount == Some(true) && now.separate_mount == Some(false) {
        out.push(Anomaly::MountLost);
    }
    if let (Some(from), Some(to)) = (
        version_tuple(&previous.version),
        version_tuple(&now.version),
    ) && to < from
    {
        out.push(Anomaly::VersionRollback {
            from: previous.version.clone(),
            to: now.version.clone(),
        });
    }
    out
}

/// `MAJOR.MINOR.PATCH` from a version string, ignoring any pre-release or
/// build suffix; `None` when it does not start with three numbers.
fn version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Record this start and report what changed since the last one.
///
/// Returns the anomalies (empty on a first start or a normal one) and whether
/// the record could be written; a journal that cannot be written is reported
/// through the second and the station still starts.
pub fn record(path: &Path, now: &BootRecord) -> (Vec<Anomaly>, std::io::Result<()>) {
    let anomalies = read(path)
        .map(|previous| compare(&previous, now))
        .unwrap_or_default();
    let written = write(path, now);
    (anomalies, written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(version: &str, db: &str, present: bool, rows: i64, mount: Option<bool>) -> BootRecord {
        BootRecord {
            version: version.into(),
            db_path: db.into(),
            db_present: present,
            detections: rows,
            separate_mount: mount,
            booted_at: 1_788_973_200,
        }
    }

    /// The gate for UP-3's four anomalies, one at a time, and their absence.
    #[test]
    fn each_anomaly_is_found_and_a_normal_start_finds_none() {
        let last = rec("0.15.0", "/data/birds.db", true, 40_000, Some(true));

        assert_eq!(
            compare(
                &last,
                &rec("0.15.0", "/data/birds.db", false, 0, Some(false))
            ),
            vec![Anomaly::DbLost { before: 40_000 }, Anomaly::MountLost],
            "a volume that did not mount: the database is gone and the mount with it"
        );
        assert_eq!(
            compare(&last, &rec("0.15.0", "/data/birds.db", true, 0, Some(true))),
            vec![Anomaly::DbLost { before: 40_000 }],
            "a fresh empty database where a full one was"
        );
        assert_eq!(
            compare(
                &last,
                &rec("0.15.0", "/mnt/other/birds.db", true, 0, Some(true))
            ),
            vec![Anomaly::DbPathChanged {
                from: "/data/birds.db".into(),
                to: "/mnt/other/birds.db".into()
            }],
            "a changed path is reported as that, not as a loss"
        );
        assert_eq!(
            compare(
                &last,
                &rec("0.14.2", "/data/birds.db", true, 40_100, Some(true))
            ),
            vec![Anomaly::VersionRollback {
                from: "0.15.0".into(),
                to: "0.14.2".into()
            }]
        );

        assert!(
            compare(
                &last,
                &rec("0.15.0", "/data/birds.db", true, 40_100, Some(true))
            )
            .is_empty()
        );
        assert!(
            compare(
                &last,
                &rec("0.16.0-rc1", "/data/birds.db", true, 40_100, Some(true))
            )
            .is_empty(),
            "an upgrade is not a rollback"
        );
        assert!(
            compare(&last, &rec("0.15.0", "/data/birds.db", true, 40_100, None)).is_empty(),
            "an unknown mount shape says nothing"
        );
        let first = rec("0.15.0", "/data/birds.db", false, 0, Some(false));
        assert!(
            compare(
                &first,
                &rec("0.15.0", "/data/birds.db", true, 12, Some(false))
            )
            .is_empty(),
            "a station whose first start had no database is not a loss"
        );
    }

    #[test]
    fn the_journal_round_trips_and_a_first_start_has_no_previous() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert!(read(&path).is_none());
        let now = rec("0.15.0", "/data/birds.db", true, 5, Some(true));
        let (anomalies, written) = record(&path, &now);
        assert!(anomalies.is_empty());
        written.expect("written");
        assert_eq!(read(&path), Some(now));

        let later = rec("0.15.0", "/data/birds.db", false, 0, Some(false));
        let (anomalies, _) = record(&path, &later);
        assert_eq!(
            anomalies.iter().map(Anomaly::key).collect::<Vec<_>>(),
            vec!["db_lost", "mount_lost"]
        );
        assert_eq!(read(&path), Some(later));

        std::fs::write(&path, "not json").unwrap();
        assert!(
            read(&path).is_none(),
            "an unreadable journal is a first start"
        );
    }

    #[test]
    fn the_journal_prefers_a_writable_configuration_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("etc");
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        assert_eq!(
            journal_path(&config_dir.join("birdnet.conf"), &data_dir),
            config_dir.join(FILE_NAME)
        );
        assert_eq!(
            journal_path(Path::new("/nonexistent/etc/birdnet.conf"), &data_dir),
            data_dir.join(FILE_NAME),
            "no configuration directory: beside the database"
        );
    }

    #[test]
    fn version_tuples_ignore_suffixes() {
        assert_eq!(version_tuple("0.15.0"), Some((0, 15, 0)));
        assert_eq!(version_tuple("v1.2.3-rc1+build"), Some((1, 2, 3)));
        assert_eq!(version_tuple("nightly"), None);
    }
}
