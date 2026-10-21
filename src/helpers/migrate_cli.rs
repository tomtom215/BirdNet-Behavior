//! `--import-birdnet-pi-conf` and `--import-birdnet-pi-clips`: moving a
//! BirdNET-Pi installation onto this station from a terminal.
//!
//! The same code as the browser's Station, Data page, reached another way: the
//! mapping table and the planner are `birdnet_migrate`, the writing is the
//! Settings form's own validation (`birdnet_web::routes::admin::migration`).
//! Both commands only **show** what they would do unless `--apply-import` is
//! given, and neither prints a credential: a secret is shown as its length.
//!
//! Exit codes: 0 for a plan shown or an import done, 1 when the import was
//! refused or failed (nothing is written in the first case), 2 when the
//! arguments are not usable.

use std::path::{Path, PathBuf};

use birdnet_migrate::clips::{self, Mode, Options, Wanted};
use birdnet_migrate::conf::{Row, Verdict};
use birdnet_web::routes::admin::migration::conf_import::{
    ApplyError, Bundle, Prepared, apply, apprise_file_path, config_lines_text, prepare,
};

use crate::cli::Cli;

/// Rows read from the database at a time.
const PAGE: i64 = 5_000;

fn settings_of(conn: &rusqlite::Connection) -> std::collections::HashMap<String, String> {
    birdnet_db::settings::list(conn, None)
        .map(|rows| rows.into_iter().map(|s| (s.key, s.value)).collect())
        .unwrap_or_default()
}

const fn order(v: Verdict) -> u8 {
    match v {
        Verdict::Refused => 0,
        Verdict::Attention => 1,
        Verdict::Converted => 2,
        Verdict::Imported => 3,
        Verdict::ConfigFile => 4,
        Verdict::NotCarried => 5,
        Verdict::Left => 6,
    }
}

fn print_row(r: &Row) {
    let value = if r.shown.is_empty() {
        "(blank)"
    } else {
        &r.shown
    };
    println!("{:<12} {} = {}", r.verdict.label(), r.pi_key, value);
    for (setting, v) in &r.becomes {
        println!("{:<12}   -> {setting} = {v}", "");
    }
    if !r.why.is_empty() {
        println!("{:<12}   {}", "", r.why);
    }
}

/// Print the plan for people.
fn print_prepared(p: &Prepared) {
    if let Some(plan) = &p.plan {
        let mut rows: Vec<&Row> = plan.rows.iter().collect();
        rows.sort_by_key(|r| order(r.verdict));
        for r in rows {
            print_row(r);
        }
        println!();
        let counts: Vec<String> = [
            Verdict::Imported,
            Verdict::Converted,
            Verdict::Attention,
            Verdict::ConfigFile,
            Verdict::Refused,
            Verdict::NotCarried,
            Verdict::Left,
        ]
        .into_iter()
        .filter(|v| plan.count(*v) > 0)
        .map(|v| format!("{} {}", plan.count(v), v.label().to_ascii_lowercase()))
        .collect();
        println!("birdnet.conf: {}.", counts.join(", "));
    }
    for l in &p.lists {
        println!(
            "{}: {} species ({}); {} new to this station's list.",
            l.kind.file_name(),
            l.file.species.len(),
            l.kind.meaning(),
            l.added
        );
    }
    if let Some(a) = &p.apprise {
        println!(
            "apprise.txt: {} notification address(es) ({}); copied to the data folder if applied.",
            a.destinations,
            a.schemes.join(", ")
        );
    }
    if p.confirmed_lines > 0 {
        println!(
            "confirmed_species_list.txt: {} species; no equivalent here, not carried over.",
            p.confirmed_lines
        );
    }
    for name in &p.ignored {
        println!("{name}: not a file this import uses; skipped.");
    }
}

/// `--import-birdnet-pi-conf PATH`.
#[must_use]
pub fn run_import_conf(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> i32 {
    let Some(path) = cli.import_birdnet_pi_conf.as_deref() else {
        return 2;
    };
    let bundle = match Bundle::from_dir(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let db_path = super::db_path_from_config(config);
    let data_dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    // Read-only for the preview, so asking cannot create or migrate anything.
    let current =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map(|c| settings_of(&c))
            .unwrap_or_default();
    let prepared = prepare(&bundle, &current, &data_dir);
    print_prepared(&prepared);

    if !cli.apply_import {
        println!(
            "\nThis changed nothing. Run again with --apply-import to write the {} setting(s) above.",
            prepared.assignments.len()
        );
        return 0;
    }
    if prepared.assignments.is_empty() {
        println!("\nThere is nothing to set.");
        return 0;
    }
    if let Err(e) = super::ensure_db_dir(&db_path) {
        eprintln!("{e}");
        return 1;
    }
    let conn = match birdnet_db::sqlite::open_or_create(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "The database {} could not be opened: {e}",
                db_path.display()
            );
            return 1;
        }
    };
    if let Some(text) = &prepared.apprise_file {
        let target = apprise_file_path(&data_dir);
        if let Err(e) = write_private(&target, text) {
            eprintln!(
                "{} could not be written: {e}. Nothing was changed.",
                target.display()
            );
            return 1;
        }
    }
    match apply(&conn, &prepared.assignments) {
        Ok(applied) => {
            println!(
                "\nDone: {} setting(s) saved. {} take effect after the station restarts.",
                applied.changed.len(),
                applied.need_restart.len()
            );
            if let Some(text) = config_lines_text(&prepared) {
                let target = data_dir.join("birdnet-from-birdnet-pi.conf");
                match write_private(&target, &text) {
                    Ok(()) => println!(
                        "Some keys are read only from birdnet.conf; their lines, with the real \
                         values, are in {} (private; add them to /etc/birdnet/birdnet.conf).",
                        target.display()
                    ),
                    Err(e) => eprintln!("{} could not be written: {e}", target.display()),
                }
            }
            0
        }
        Err(ApplyError::Refused(problems)) => {
            eprintln!("\nNothing was changed. This station would refuse:");
            for p in problems {
                eprintln!("  {p}");
            }
            1
        }
        Err(e) => {
            eprintln!("\nNothing was changed: {e}");
            1
        }
    }
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Reads the imported rows a page at a time over its own connection.
struct Pager {
    conn: rusqlite::Connection,
    after: i64,
    page: std::collections::VecDeque<Wanted>,
    done: bool,
}

impl Iterator for Pager {
    type Item = Wanted;

    fn next(&mut self) -> Option<Wanted> {
        if self.page.is_empty() && !self.done {
            match clips::wanted_page(&self.conn, self.after, PAGE) {
                Ok(rows) if !rows.is_empty() => {
                    self.after = rows.last().map_or(self.after, |(id, _)| *id);
                    self.page = rows.into_iter().map(|(_, w)| w).collect();
                }
                Ok(_) => self.done = true,
                Err(e) => {
                    eprintln!("The imported detections could not be read: {e}");
                    self.done = true;
                }
            }
        }
        self.page.pop_front()
    }
}

/// `--import-birdnet-pi-clips PATH`.
#[must_use]
pub fn run_import_clips(cli: &Cli, config: Option<&birdnet_core::config::Config>) -> i32 {
    let Some(source) = cli.import_birdnet_pi_clips.clone() else {
        return 2;
    };
    let Some(mode) = Mode::parse(&cli.clips_mode) else {
        eprintln!("--clips-mode takes copy, hardlink, move or leave.");
        return 2;
    };
    let db_path = super::db_path_from_config(config);
    if !db_path.exists() {
        eprintln!(
            "There is no database at {} yet, so there are no imported detections to bring \
             recordings for. Import the history first.",
            db_path.display()
        );
        return 2;
    }
    let open = || {
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    };
    let settings = match open() {
        Ok(c) => settings_of(&c),
        Err(e) => {
            eprintln!(
                "The database {} could not be opened: {e}",
                db_path.display()
            );
            return 1;
        }
    };
    let dest = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        .join("recordings");
    let threshold = settings
        .get("purge_threshold")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(95.0);
    let probe = if dest.is_dir() {
        dest.clone()
    } else {
        dest.parent()
            .map_or_else(|| dest.clone(), Path::to_path_buf)
    };
    let room = birdnet_core::audio::capture::disk_usage(&probe)
        .ok()
        .map(|u| clips::DiskRoom {
            used_bytes: u.used_bytes,
            available_bytes: u.available_bytes,
            purge_threshold_percent: threshold,
        });
    let options = Options {
        source,
        dest,
        mode,
        dry_run: !cli.apply_import,
        room,
        retention_cutoff: None,
    };
    let wanted = || -> Box<dyn Iterator<Item = Wanted>> {
        match open() {
            Ok(conn) => Box::new(Pager {
                conn,
                after: 0,
                page: std::collections::VecDeque::new(),
                done: false,
            }),
            Err(_) => Box::new(std::iter::empty()),
        }
    };
    match clips::run(&options, &wanted, None) {
        Ok(r) => {
            let verb = if options.dry_run { "would be" } else { "were" };
            println!(
                "{} recording(s) {verb} {} ({}); {} already here, {} not found, {} conflicting, {} failed; {} detections asked about.",
                r.done,
                match mode {
                    Mode::Copy => "copied",
                    Mode::Hardlink => "linked",
                    Mode::Move => "moved",
                    Mode::Leave => "found",
                },
                clips::human_bytes(r.bytes),
                r.already_there,
                r.missing,
                r.conflicts,
                r.failed,
                r.examined
            );
            for n in &r.missing_sample {
                println!("  not found: {n}");
            }
            for n in &r.failed_sample {
                println!("  failed: {n}");
            }
            if options.dry_run {
                println!("\nThis changed nothing. Run again with --apply-import to do it.");
            }
            i32::from(r.failed > 0)
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::default_cli;

    fn cli_for(conf_dir: &Path, apply: bool) -> Cli {
        let mut cli = default_cli();
        cli.import_birdnet_pi_conf = Some(conf_dir.to_path_buf());
        cli.apply_import = apply;
        cli
    }

    fn config_for(db: &Path) -> birdnet_core::config::Config {
        crate::helpers::test_support::config_with(&[("DB_PATH", &db.display().to_string())])
    }

    fn settings(db: &Path) -> std::collections::HashMap<String, String> {
        settings_of(&rusqlite::Connection::open(db).expect("open"))
    }

    fn fixture(dir: &Path) {
        std::fs::write(
            dir.join("birdnet.conf"),
            "LATITUDE=52.0\nLONGITUDE=5.0\nCONFIDENCE=0.65\nFULL_DISK=keep\nBIRDWEATHER_ID=bwSECRETid\n",
        )
        .expect("conf");
    }

    /// The default is to look: nothing is written, not even a database.
    #[test]
    fn without_apply_it_writes_nothing() {
        let work = tempfile::tempdir().expect("dir");
        let pi = work.path().join("pi");
        std::fs::create_dir(&pi).unwrap();
        fixture(&pi);
        let db = work.path().join("station/birds.db");
        let code = run_import_conf(&cli_for(&pi, false), Some(&config_for(&db)));
        assert_eq!(code, 0);
        assert!(!db.exists(), "a dry run created the database");
        assert!(!db.parent().unwrap().exists());
    }

    #[test]
    fn with_apply_the_settings_are_saved_through_the_settings_validation() {
        let work = tempfile::tempdir().expect("dir");
        let pi = work.path().join("pi");
        std::fs::create_dir(&pi).unwrap();
        fixture(&pi);
        let db = work.path().join("station/birds.db");
        let code = run_import_conf(&cli_for(&pi, true), Some(&config_for(&db)));
        assert_eq!(code, 0);
        let s = settings(&db);
        assert_eq!(
            s.get("confidence_threshold").map(String::as_str),
            Some("0.65")
        );
        assert_eq!(s.get("full_disk_action").map(String::as_str), Some("keep"));
        assert_eq!(
            s.get("birdweather_token").map(String::as_str),
            Some("bwSECRETid")
        );
    }

    /// A value the Settings page would refuse stops the whole import.
    #[test]
    fn a_refused_value_writes_nothing_and_exits_one() {
        let work = tempfile::tempdir().expect("dir");
        let pi = work.path().join("pi");
        std::fs::create_dir(&pi).unwrap();
        std::fs::write(
            pi.join("birdnet.conf"),
            "CONFIDENCE=0.65\nSENSITIVITY=1.2\n",
        )
        .unwrap();
        let db = work.path().join("birds.db");
        // 75 is refused by the preview, so the plan sets nothing for it and the
        // rest still apply; a *forced* refusal needs the form to refuse a value
        // the planner passed, which `apply` does for an out-of-range value
        // handed to it directly.
        let conn = birdnet_db::sqlite::open_or_create(&db).expect("db");
        let err = apply(
            &conn,
            &[
                ("sensitivity".to_owned(), "1.2".to_owned()),
                ("confidence_threshold".to_owned(), "75".to_owned()),
            ],
        )
        .expect_err("refused");
        assert!(matches!(err, ApplyError::Refused(_)));
        assert!(!settings(&db).contains_key("sensitivity"));
        // And the command on a conf the planner refuses sets nothing for that key.
        std::fs::write(pi.join("birdnet.conf"), "CONFIDENCE=75\n").unwrap();
        let code = run_import_conf(&cli_for(&pi, true), Some(&config_for(&db)));
        assert_eq!(code, 0, "nothing to set is not a failure");
        assert!(!settings(&db).contains_key("confidence_threshold"));
    }

    #[test]
    fn a_path_with_no_birdnet_pi_files_exits_two() {
        let work = tempfile::tempdir().expect("dir");
        let code = run_import_conf(&cli_for(work.path(), false), None);
        assert_eq!(code, 2);
    }
}
