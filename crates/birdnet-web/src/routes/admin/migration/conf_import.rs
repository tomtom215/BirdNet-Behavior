//! Moving a BirdNET-Pi installation's **settings** onto this station.
//!
//! The database import (`../migration.rs`) brings the detections across. What
//! it does not bring is everything the owner configured: the position, the
//! thresholds, the notification rules, the species lists, the disk policy.
//! BirdNET-Pi keeps those in `birdnet.conf` and a handful of text files beside
//! it; this module reads them, says key by key what each becomes
//! ([`birdnet_migrate::conf`]), and, on confirmation, writes them **through the
//! settings form's own validation** so nothing goes in that the Settings page
//! would have refused.
//!
//! # The flow, and what is held in the meantime
//!
//! `preview` reads what the owner handed over and shows the plan; it changes
//! nothing. The files — which hold credentials — are kept in memory only, in
//! one slot, for [`STAGE_TTL`] and under a token the confirm button carries,
//! exactly as the database upload is staged: a second upload in between
//! replaces the first, and the token stops one admin confirming another's. Apply
//! takes the slot, so a double submit applies once.
//!
//! # What it never does
//!
//! It never writes a credential to a log, to the audit trail, or into the
//! preview's HTML: the audit row names the settings that changed and nothing
//! else, and a secret is shown as its length. It never writes `CADDY_PWD`
//! anywhere. It never overwrites a species list: it adds to it.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use birdnet_db::settings::{SettingsCategory, ensure_settings_table, list, set_many};
use birdnet_migrate::conf::lists::{AppriseSummary, ListKind, SpeciesList, merge_into};
use birdnet_migrate::conf::{ConfFile, Plan, PlanInput, Verdict, plan};

use super::super::settings::form::{SETTINGS_FORM_KEYS, SettingsForm};
use super::super::settings::handler::{Unset, build_settings_items, submission_problems};

/// How long a preview may wait for its confirmation.
pub(super) const STAGE_TTL: Duration = Duration::from_secs(15 * 60);

/// The largest file taken from an upload or a directory. A `birdnet.conf` is
/// about 10 KiB; a species list a few hundred lines.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

/// What the owner handed over: the files of a BirdNET-Pi installation.
///
/// `Debug` prints names and sizes, never contents: the files hold credentials.
#[derive(Default, Clone)]
pub struct Bundle {
    /// `birdnet.conf`.
    pub conf: Option<String>,
    /// `apprise.txt`: where notifications go.
    pub apprise: Option<String>,
    /// `body.txt`: the notification message.
    pub body: Option<String>,
    /// The three species list files.
    pub lists: BTreeMap<String, String>,
    /// `confirmed_species_list.txt`, which has no equivalent here.
    pub confirmed: Option<String>,
    /// File names that were handed over and are none of those.
    pub ignored: Vec<String>,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("conf_bytes", &self.conf.as_ref().map(String::len))
            .field("apprise_bytes", &self.apprise.as_ref().map(String::len))
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .field("confirmed_bytes", &self.confirmed.as_ref().map(String::len))
            .field("lists", &self.lists.keys().collect::<Vec<_>>())
            .field("ignored", &self.ignored)
            .finish()
    }
}

impl Bundle {
    /// Whether nothing useful was handed over.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.conf.is_none()
            && self.apprise.is_none()
            && self.body.is_none()
            && self.lists.is_empty()
    }

    /// File one file under what it is, by its name.
    ///
    /// A file is a `birdnet.conf` by its name — `birdnet.conf`, or anything
    /// ending `.conf` — or, failing that, by looking like one: a `LATITUDE=`
    /// or `CONFIDENCE=` line. The paste box has no name to go by.
    pub fn add_file(&mut self, name: &str, text: String) {
        let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let lower = base.to_ascii_lowercase();
        if let Some(kind) = ListKind::from_file_name(&lower) {
            self.lists.insert(kind.file_name().to_owned(), text);
        } else if lower == "apprise.txt" {
            self.apprise = Some(text);
        } else if lower == "body.txt" {
            self.body = Some(text);
        } else if lower == "confirmed_species_list.txt" {
            self.confirmed = Some(text);
        } else if has_conf_suffix(&lower) || looks_like_birdnet_conf(&text) {
            self.conf = Some(text);
        } else {
            self.ignored.push(base.to_owned());
        }
    }

    /// Read the files of a BirdNET-Pi installation from a directory on this
    /// device.
    ///
    /// Only the known names are read, each at most [`MAX_FILE_BYTES`], from the
    /// directory itself or its `scripts/` folder (where BirdNET-Pi links its
    /// lists). `path` may be that directory or a `.conf` file in it. This is
    /// not a general file reader: a path that names anything else is refused.
    ///
    /// # Errors
    ///
    /// In words for the owner: the path is not absolute, does not exist, or
    /// holds none of the files.
    pub fn from_dir(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "{} is not a full path. Start it with a slash, for example /home/pi/BirdNET-Pi.",
                path.display()
            ));
        }
        let (dir, conf_file) = if path.is_dir() {
            (path.to_path_buf(), None)
        } else if path.is_file() && path.extension().is_some_and(|e| e == "conf") {
            (
                path.parent().map(Path::to_path_buf).unwrap_or_default(),
                Some(path.to_path_buf()),
            )
        } else {
            return Err(format!(
                "{} is not a folder, or a birdnet.conf file, on this device.",
                path.display()
            ));
        };
        let mut bundle = Self::default();
        if let Some(file) = conf_file {
            bundle.add_file("birdnet.conf", read_small(&file)?);
        }
        let known = [
            "birdnet.conf",
            "apprise.txt",
            "body.txt",
            "include_species_list.txt",
            "exclude_species_list.txt",
            "whitelist_species_list.txt",
            "confirmed_species_list.txt",
        ];
        for name in known {
            if name == "birdnet.conf" && bundle.conf.is_some() {
                continue;
            }
            for candidate in [dir.join(name), dir.join("scripts").join(name)] {
                if candidate.is_file() {
                    bundle.add_file(name, read_small(&candidate)?);
                    break;
                }
            }
        }
        if bundle.is_empty() {
            return Err(format!(
                "{} holds none of the BirdNET-Pi files (birdnet.conf, apprise.txt, body.txt, \
                 and the species lists).",
                dir.display()
            ));
        }
        Ok(bundle)
    }
}

/// Whether an already lower-cased file name ends in `.conf`.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the caller lower-cased the name, so the exact match is a case-insensitive one"
)]
fn has_conf_suffix(lower: &str) -> bool {
    lower.ends_with(".conf")
}

fn looks_like_birdnet_conf(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim_start();
        l.starts_with("LATITUDE=") || l.starts_with("CONFIDENCE=") || l.starts_with("SF_THRESH=")
    })
}

fn read_small(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{} is {} bytes; a BirdNET-Pi settings file is a few kilobytes, so this is not one.",
            path.display(),
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// One species list, as it will be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPlan {
    /// Which list.
    pub kind: ListKind,
    /// What was read from BirdNET-Pi's file.
    pub file: SpeciesList,
    /// What this station's list holds now.
    pub now: Vec<String>,
    /// What it will hold: what is there, with the new names added.
    pub merged: String,
    /// How many names are new to it.
    pub added: usize,
}

/// What an import would do, ready to show and to apply.
///
/// `Debug` prints counts: `assignments` and `apprise_file` hold credentials.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Prepared {
    /// The key-by-key plan for `birdnet.conf`, when one was handed over.
    pub plan: Option<Plan>,
    /// The species lists.
    pub lists: Vec<ListPlan>,
    /// What `apprise.txt` holds, without its secrets.
    pub apprise: Option<AppriseSummary>,
    /// Text for the file to write for `apprise_config` to point at.
    pub apprise_file: Option<String>,
    /// Lines of `confirmed_species_list.txt`, which have no equivalent.
    pub confirmed_lines: usize,
    /// Files that were handed over and are none of those above.
    pub ignored: Vec<String>,
    /// Every `(setting, value)` to write, in one list, for [`apply`]. Secrets
    /// are in it; nothing prints it.
    pub assignments: Vec<(String, String)>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("rows", &self.plan.as_ref().map(|p| p.rows.len()))
            .field("lists", &self.lists.len())
            .field(
                "apprise_destinations",
                &self.apprise.as_ref().map(|a| a.destinations),
            )
            .field("assignments", &self.assignments.len())
            .finish_non_exhaustive()
    }
}

/// Where the Apprise file is written, beside the database.
#[must_use]
pub fn apprise_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("apprise-from-birdnet-pi.txt")
}

/// Whether this station would accept `value` for the setting `key`.
///
/// The Settings form's own validation, run on a form holding only this field:
/// the preview and the Apply button ask the same code, so they cannot disagree.
fn check_setting(key: &str, value: &str) -> Option<String> {
    let form = form_of(&[(key.to_owned(), value.to_owned())]).ok()?;
    submission_problems(&form).into_iter().next()
}

fn form_of(pairs: &[(String, String)]) -> Result<SettingsForm, ApplyError> {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        if !SETTINGS_FORM_KEYS.contains(&k.as_str()) {
            return Err(ApplyError::NotASetting(k.clone()));
        }
        map.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    serde_json::from_value(serde_json::Value::Object(map))
        .map_err(|e| ApplyError::NotASetting(e.to_string()))
}

/// Work out what importing `bundle` would do on a station whose settings are
/// `current` and whose data lives in `data_dir`. Reads nothing from disk.
#[must_use]
#[allow(
    clippy::implicit_hasher,
    reason = "the settings snapshot is the default-hasher map in every signature this calls; a generic hasher here would not type-check against them"
)]
pub fn prepare(bundle: &Bundle, current: &HashMap<String, String>, data_dir: &Path) -> Prepared {
    let mut out = Prepared {
        ignored: bundle.ignored.clone(),
        ..Prepared::default()
    };

    if let Some(text) = &bundle.conf {
        let conf = ConfFile::parse(text);
        let p = plan(&PlanInput {
            conf: &conf,
            current,
            check: &check_setting,
        });
        for a in &p.assignments {
            out.assignments
                .push((a.setting.to_owned(), a.value.clone()));
        }
        out.plan = Some(p);
    }

    for kind in ListKind::ALL {
        let Some(text) = bundle.lists.get(kind.file_name()) else {
            continue;
        };
        let file = SpeciesList::parse(text);
        let now_text = current.get(kind.setting()).map_or("", String::as_str);
        let now: Vec<String> = now_text
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let (merged, added) = merge_into(now_text, &file.species);
        if added > 0 {
            out.assignments
                .push((kind.setting().to_owned(), merged.clone()));
        }
        out.lists.push(ListPlan {
            kind,
            file,
            now,
            merged,
            added,
        });
    }

    if let Some(text) = &bundle.apprise
        && !text.trim().is_empty()
    {
        out.apprise = Some(AppriseSummary::of(text));
        out.apprise_file = Some(text.clone());
        out.assignments.push((
            "apprise_config".to_owned(),
            apprise_file_path(data_dir).display().to_string(),
        ));
    }
    if let Some(text) = &bundle.body {
        let body = text.trim_end();
        if !body.is_empty() && check_setting("notify_body_template", body).is_none() {
            out.assignments
                .push(("notify_body_template".to_owned(), body.to_owned()));
        }
    }
    out.confirmed_lines = bundle
        .confirmed
        .as_deref()
        .map_or(0, |t| t.lines().filter(|l| !l.trim().is_empty()).count());
    out
}

/// Why the settings could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// A name that is not a setting of this station. A bug in the mapping.
    NotASetting(String),
    /// The settings page would refuse these values; nothing was written.
    Refused(Vec<String>),
    /// The current settings or the write failed.
    Database(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotASetting(k) => write!(f, "{k} is not a setting of this station"),
            Self::Refused(problems) => write!(f, "{}", problems.join(" ")),
            Self::Database(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// What was written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    /// The settings whose stored value changed, in order.
    pub changed: Vec<&'static str>,
    /// Of those, the ones that take effect only after a restart.
    pub need_restart: Vec<&'static str>,
}

/// Write `assignments` through the Settings form's own path.
///
/// Builds the form the Settings page would have posted, runs its validation
/// over the **whole** submission (all or nothing), and saves what differs from
/// what is stored, with the page's own categories. Used by the browser and by
/// `--import-birdnet-pi-conf`.
///
/// # Errors
///
/// [`ApplyError::Refused`] with the page's own sentences when any value would
/// have been refused, [`ApplyError::NotASetting`] for a name the form does not
/// have, [`ApplyError::Database`] when reading or writing failed. Nothing is
/// written in any of those cases.
pub fn apply(
    conn: &rusqlite::Connection,
    assignments: &[(String, String)],
) -> Result<Applied, ApplyError> {
    let form = form_of(assignments)?;
    let problems = submission_problems(&form);
    if !problems.is_empty() {
        return Err(ApplyError::Refused(problems));
    }
    ensure_settings_table(conn).map_err(|e| ApplyError::Database(e.to_string()))?;
    let existing: HashMap<String, String> = list(conn, None)
        .map_err(|e| ApplyError::Database(e.to_string()))?
        .into_iter()
        .map(|s| (s.key, s.value))
        .collect();
    let items = build_settings_items(&form, &existing, Unset::AsEmpty);
    let refs: Vec<(&str, &str, SettingsCategory)> =
        items.iter().map(|(k, v, c)| (*k, v.as_str(), *c)).collect();
    set_many(conn, &refs).map_err(|e| ApplyError::Database(e.to_string()))?;
    let changed: Vec<&'static str> = items.iter().map(|(k, _, _)| *k).collect();
    let need_restart = changed
        .iter()
        .copied()
        .filter(|k| {
            super::super::settings::applies::applies_for(k)
                .is_some_and(super::super::settings::applies::Applies::needs_restart)
        })
        .collect();
    Ok(Applied {
        changed,
        need_restart,
    })
}

/// A preview waiting for its confirmation.
pub(super) struct Staged {
    pub(super) token: u64,
    pub(super) at: Instant,
    pub(super) prepared: Prepared,
}

/// One slot, as the database upload has one.
pub(super) type StagedConf = Arc<Mutex<Option<Staged>>>;

/// The lines for `birdnet.conf` from the last applied import, for download.
pub(super) type ConfigLines = Arc<Mutex<Option<(u64, String)>>>;

/// Source of tokens. Monotonic, not secret: the route is behind admin RBAC.
static TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_token() -> u64 {
    TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The lines to put in `birdnet.conf`, as a file.
pub fn config_lines_text(prepared: &Prepared) -> Option<String> {
    let plan = prepared.plan.as_ref()?;
    if plan.config_lines.is_empty() {
        return None;
    }
    let mut out = String::from(
        "# Lines from your BirdNET-Pi birdnet.conf that this station reads from its own\n\
         # birdnet.conf and not from the browser. Add them to /etc/birdnet/birdnet.conf,\n\
         # then restart the station.\n",
    );
    for line in &plan.config_lines {
        out.push_str(line.key);
        out.push('=');
        out.push_str(&line.value);
        out.push('\n');
    }
    Some(out)
}

/// How many rows of each verdict, for the summary sentence.
pub(super) fn tally(plan: &Plan) -> Vec<(Verdict, usize)> {
    [
        Verdict::Imported,
        Verdict::Converted,
        Verdict::Attention,
        Verdict::ConfigFile,
        Verdict::Refused,
        Verdict::NotCarried,
        Verdict::Left,
    ]
    .into_iter()
    .map(|v| (v, plan.count(v)))
    .filter(|(_, n)| *n > 0)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use birdnet_migrate::conf::RULES;
    use birdnet_migrate::conf::Target;

    fn conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open");
        ensure_settings_table(&conn).expect("table");
        conn
    }

    fn stored(conn: &rusqlite::Connection, key: &str) -> Option<String> {
        list(conn, None)
            .expect("list")
            .into_iter()
            .find(|s| s.key == key)
            .map(|s| s.value)
    }

    fn pi_default() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../birdnet-migrate/tests/data/birdnet_pi_default.conf");
        std::fs::read_to_string(path).expect("the fixture")
    }

    fn bundle_of(conf: &str) -> Bundle {
        let mut b = Bundle::default();
        b.add_file("birdnet.conf", conf.to_owned());
        b
    }

    #[test]
    fn files_are_recognised_by_their_names() {
        let mut b = Bundle::default();
        b.add_file("birdnet.conf", "LATITUDE=1\n".into());
        b.add_file("C:\\Users\\me\\apprise.txt", "discord://x/y\n".into());
        b.add_file("body.txt", "A $comname".into());
        b.add_file("include_species_list.txt", "Pica pica_Magpie\n".into());
        b.add_file("confirmed_species_list.txt", "Pica pica\n".into());
        b.add_file("vacation-photos.zip", "PK".into());
        assert!(b.conf.is_some() && b.apprise.is_some() && b.body.is_some());
        assert_eq!(b.lists.len(), 1);
        assert!(b.confirmed.is_some());
        assert_eq!(b.ignored, ["vacation-photos.zip"]);
        // A pasted conf has no name: it is recognised by what it holds.
        let mut pasted = Bundle::default();
        pasted.add_file("pasted", "CONFIDENCE=0.7\n".into());
        assert!(pasted.conf.is_some());
    }

    /// Reading from a folder is not a general file reader.
    #[test]
    fn a_folder_gives_up_only_the_known_files() {
        let dir = tempfile::tempdir().expect("dir");
        std::fs::write(dir.path().join("birdnet.conf"), "LATITUDE=52\n").unwrap();
        std::fs::write(dir.path().join("shadow"), "root:x:0:0\n").unwrap();
        std::fs::create_dir(dir.path().join("scripts")).unwrap();
        std::fs::write(
            dir.path().join("scripts/exclude_species_list.txt"),
            "Corvus corax_Raven\n",
        )
        .unwrap();
        let b = Bundle::from_dir(dir.path()).expect("read");
        assert!(b.conf.is_some());
        assert_eq!(b.lists.len(), 1, "the list under scripts/ is found");
        assert!(b.ignored.is_empty(), "nothing else was even opened");

        // A path to the file itself reads it and its neighbours.
        let b = Bundle::from_dir(&dir.path().join("birdnet.conf")).expect("read");
        assert!(b.conf.is_some());

        assert!(Bundle::from_dir(Path::new("relative/dir")).is_err());
        assert!(
            Bundle::from_dir(&dir.path().join("shadow")).is_err(),
            "not a folder or a .conf"
        );
        let empty = tempfile::tempdir().expect("dir");
        assert!(Bundle::from_dir(empty.path()).is_err());
    }

    #[test]
    fn a_file_far_too_large_to_be_settings_is_refused() {
        let dir = tempfile::tempdir().expect("dir");
        let big = dir.path().join("birdnet.conf");
        std::fs::write(
            &big,
            vec![b'#'; usize::try_from(MAX_FILE_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert!(
            Bundle::from_dir(dir.path())
                .unwrap_err()
                .contains("not one")
        );
    }

    /// The whole default file goes through the Settings form's own validation
    /// and lands in the table.
    #[test]
    fn a_default_birdnet_conf_is_applied_through_the_settings_validation() {
        let conn = conn();
        let prepared = prepare(
            &bundle_of(&pi_default()),
            &HashMap::new(),
            Path::new("/data"),
        );
        let plan = prepared.plan.as_ref().expect("a plan");
        assert!(
            plan.rows.iter().all(|r| r.verdict != Verdict::Refused),
            "the settings page refuses a value from BirdNET-Pi's default file: {:?}",
            plan.rows
                .iter()
                .filter(|r| r.verdict == Verdict::Refused)
                .map(|r| (&r.pi_key, &r.why))
                .collect::<Vec<_>>()
        );
        let applied = apply(&conn, &prepared.assignments).expect("applies");
        assert!(applied.changed.contains(&"confidence_threshold"));
        for (key, value) in [
            ("confidence_threshold", "0.7"),
            ("audio_format", "mp3"),
            ("full_disk_action", "purge"),
            ("notify_trigger", "off"),
            ("notify_confidence", "0"),
            ("notify_cooldown", "0"),
            ("latitude", "52.0"),
            ("privacy_rank", "10"),
            ("info_site", "allaboutbirds"),
        ] {
            assert_eq!(stored(&conn, key).as_deref(), Some(value), "{key}");
        }
        assert!(
            applied.need_restart.contains(&"full_disk_action"),
            "the disk policy is read at start: {:?}",
            applied.need_restart
        );
    }

    /// All or nothing: a value the page would refuse stops the whole import.
    #[test]
    fn a_value_the_page_would_refuse_writes_nothing() {
        let conn = conn();
        let result = apply(
            &conn,
            &[
                ("sensitivity".to_owned(), "1.1".to_owned()),
                ("confidence_threshold".to_owned(), "75".to_owned()),
            ],
        );
        let ApplyError::Refused(problems) = result.expect_err("refused") else {
            panic!("expected a refusal");
        };
        assert!(problems.iter().any(|p| p.contains("0.75")), "{problems:?}");
        assert_eq!(
            stored(&conn, "sensitivity"),
            None,
            "the good half was written"
        );
    }

    #[test]
    fn a_name_that_is_not_a_setting_is_refused() {
        let conn = conn();
        let err = apply(&conn, &[("no_such_setting".to_owned(), "1".to_owned())])
            .expect_err("not a setting");
        assert_eq!(err, ApplyError::NotASetting("no_such_setting".to_owned()));
    }

    /// The preview shows what the page would refuse.
    #[test]
    fn the_preview_refuses_the_percentage_slip() {
        let prepared = prepare(
            &bundle_of("CONFIDENCE=75\n"),
            &HashMap::new(),
            Path::new("/d"),
        );
        let plan = prepared.plan.expect("plan");
        let row = plan
            .rows
            .iter()
            .find(|r| r.pi_key == "CONFIDENCE")
            .expect("row");
        assert_eq!(row.verdict, Verdict::Refused);
        assert!(prepared.assignments.is_empty());
    }

    /// A list is added to and never replaced.
    #[test]
    fn species_lists_are_added_to_what_is_there() {
        let mut b = Bundle::default();
        b.add_file(
            "exclude_species_list.txt",
            "Pica pica_Eurasian Magpie\nTurdus merula_Eurasian Blackbird\n".into(),
        );
        let current: HashMap<String, String> = [(
            "species_exclude".to_owned(),
            "House Sparrow, Pica pica".to_owned(),
        )]
        .into();
        let prepared = prepare(&b, &current, Path::new("/d"));
        assert_eq!(prepared.lists.len(), 1);
        assert_eq!(prepared.lists[0].added, 1);
        assert_eq!(
            prepared.assignments,
            [(
                "species_exclude".to_owned(),
                "House Sparrow, Pica pica, Turdus merula".to_owned()
            )]
        );

        // Counterpart: nothing new, nothing written.
        let current: HashMap<String, String> = [(
            "species_exclude".to_owned(),
            "turdus merula, pica pica".to_owned(),
        )]
        .into();
        assert!(
            prepare(&b, &current, Path::new("/d"))
                .assignments
                .is_empty()
        );
    }

    /// The Apprise file is described by its services, and its addresses stay out
    /// of everything that prints.
    #[test]
    fn the_apprise_file_never_reaches_a_print() {
        let mut b = Bundle::default();
        b.add_file(
            "apprise.txt",
            "discord://12345/TOKEN-ABC\nntfy://topic-xyz\n".into(),
        );
        b.add_file("birdnet.conf", "BIRDWEATHER_ID=bwSECRET\n".into());
        let prepared = prepare(&b, &HashMap::new(), Path::new("/data"));
        let summary = prepared.apprise.as_ref().expect("summary");
        assert_eq!(summary.schemes, ["discord", "ntfy"]);
        assert!(prepared.assignments.contains(&(
            "apprise_config".to_owned(),
            "/data/apprise-from-birdnet-pi.txt".to_owned()
        )));
        for printed in [format!("{prepared:?}"), format!("{b:?}")] {
            assert!(
                !printed.contains("TOKEN-ABC")
                    && !printed.contains("bwSECRET")
                    && !printed.contains("topic-xyz"),
                "{printed}"
            );
        }
        let html = super::super::conf_view::preview(&prepared, 7);
        assert!(
            !html.contains("TOKEN-ABC") && !html.contains("bwSECRET"),
            "the preview shows a credential"
        );
        assert!(html.contains("discord, ntfy"), "{html}");
    }

    /// Drift gate: every setting the table can write is a setting the form has,
    /// so the import cannot silently write a row nothing reads.
    #[test]
    fn every_setting_the_table_writes_is_a_settings_form_key() {
        for rule in RULES {
            if let Target::Setting { key, convert } = rule.target {
                assert!(
                    SETTINGS_FORM_KEYS.contains(&key),
                    "{} writes `{key}`, which the Settings form does not have",
                    rule.pi_key
                );
                let _ = convert;
            }
        }
        for key in ["notify_confidence", "privacy_threshold"] {
            assert!(SETTINGS_FORM_KEYS.contains(&key), "{key}");
        }
    }

    /// The config-file lines come back, with the real values, for download only.
    #[test]
    fn config_file_keys_come_back_as_a_download_and_not_in_the_page() {
        let prepared = prepare(
            &bundle_of("IMAGE_PROVIDER=FLICKR\nFLICKR_API_KEY=flkSECRET\n"),
            &HashMap::new(),
            Path::new("/d"),
        );
        let text = config_lines_text(&prepared).expect("lines");
        assert!(text.contains("FLICKR_API_KEY=flkSECRET"));
        assert!(text.contains("IMAGE_PROVIDER=FLICKR"));
        let html = super::super::conf_view::preview(&prepared, 1);
        assert!(!html.contains("flkSECRET"));
    }
}
