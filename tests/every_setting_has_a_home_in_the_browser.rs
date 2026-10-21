//! Every setting a person can change has a place in the browser, or a written
//! reason that it does not (the coverage table in
//! `docs/book/reference/where-to-change-it.md`).
//!
//! # Why this is a test
//!
//! The aim is that a person can take a new station from first boot to listening,
//! and later change anything, with a browser alone. Without something checking,
//! that is true on the day it is written and false the next time a
//! `config.get("NEW_KEY")` or a `#[arg(long)]` lands: the new setting can only
//! be reached from a terminal, and nobody decided that. This reads the three
//! places a setting can come from and the table that accounts for them, and
//! fails when they disagree:
//!
//! * the **configuration-file keys** (`KNOWN_CONFIG_KEYS`),
//! * the **command-line flags** (what `--help` lists, from the built binary),
//! * the **saved settings** the settings pages write (`applies::APPLIES`, which
//!   the binary's own tests hold equal to its bridge table).
//!
//! A row either names where the setting is changed (a `field` that really is a
//! control on a page, checked against the pages' markup) or says why it is not,
//! in a sentence. "—" with no reason fails, and so does a row for a key that no
//! longer exists.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use birdnet_core::config::known_keys::KNOWN_CONFIG_KEYS;
use birdnet_web::routes::admin::settings::applies::every;

const DOC: &str = "docs/book/reference/where-to-change-it.md";

/// The shortest reason that can be a reason. A sentence, not a gesture.
const MIN_REASON: usize = 25;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The rows of the table under the `## heading`, as trimmed cells.
fn table(doc: &str, heading: &str) -> Vec<Vec<String>> {
    let mut in_section = false;
    let mut rows = Vec::new();
    for line in doc.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            in_section = h.trim() == heading;
            continue;
        }
        if !in_section || !line.starts_with("| `") {
            continue;
        }
        let cells: Vec<String> = line
            .trim()
            .trim_matches('|')
            .split(" | ")
            .map(|c| c.trim().to_owned())
            .collect();
        rows.push(cells);
    }
    rows
}

fn unquote(cell: &str) -> String {
    cell.trim().trim_matches('`').to_owned()
}

/// A row of the form `key | where | why not`: `where` is `—` exactly when
/// there is a reason, and the reason is a sentence.
fn check_home_or_reason(kind: &str, name: &str, home: &str, reason: &str, problems: &mut Vec<String>) {
    let no_home = home == "—";
    if no_home {
        if reason.chars().count() < MIN_REASON {
            problems.push(format!(
                "{kind} {name}: no place in the browser and no reason (found {reason:?})"
            ));
        }
    } else if home.is_empty() {
        problems.push(format!("{kind} {name}: the \"changed at\" cell is empty"));
    } else if !reason.is_empty() {
        problems.push(format!(
            "{kind} {name}: it has a place in the browser, so it needs no reason (found {reason:?})"
        ));
    }
}

fn doc() -> String {
    fs::read_to_string(root().join(DOC)).unwrap_or_else(|e| panic!("{DOC} is missing: {e}"))
}

#[test]
fn every_config_key_has_a_place_in_the_browser_or_a_reason() {
    let doc = doc();
    let rows = table(&doc, "Configuration-file keys");
    let mut by_key: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut problems = Vec::new();
    for r in &rows {
        if r.len() != 3 {
            problems.push(format!("a config-key row has {} cells, not 3: {r:?}", r.len()));
            continue;
        }
        let key = unquote(&r[0]);
        check_home_or_reason("config key", &key, &r[1], &r[2], &mut problems);
        if by_key.insert(key.clone(), (r[1].clone(), r[2].clone())).is_some() {
            problems.push(format!("config key {key} appears twice"));
        }
    }
    let known: BTreeSet<&str> = KNOWN_CONFIG_KEYS.iter().copied().collect();
    let listed: BTreeSet<&str> = by_key.keys().map(String::as_str).collect();
    let missing: Vec<_> = known.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&known).collect();
    assert!(
        rows.len() >= 100,
        "the table parsed to {} rows; it is no longer being read",
        rows.len()
    );
    assert!(
        missing.is_empty(),
        "these configuration keys are read by the station and have neither a place in the \
         browser nor a written reason in {DOC} (add a row to `## Configuration-file keys`; \
         if the browser cannot change it, say why in a sentence):\n  {}",
        missing.iter().map(|k| format!("| `{k}` | — | <why> |")).collect::<Vec<_>>().join("\n  ")
    );
    assert!(
        stale.is_empty(),
        "{DOC} has rows for keys the station no longer reads: {stale:?}"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The flags `--help` lists, with the environment variable each reads.
fn flags_from_help() -> BTreeMap<String, Option<String>> {
    let out = Command::new(env!("CARGO_BIN_EXE_birdnet-behavior"))
        .arg("--help")
        .output()
        .expect("run --help");
    assert!(out.status.success(), "--help failed");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut flags: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if (indent == 2 || indent == 6) && trimmed.starts_with('-') {
            // "-c, --config <CONFIG>" or "--listen <LISTEN>"
            let long = trimmed
                .split([' ', ','])
                .find(|w| w.starts_with("--"))
                .map(str::to_owned);
            if let Some(l) = &long {
                flags.entry(l.clone()).or_insert(None);
            }
            current = long;
        } else if let (Some(flag), Some(rest)) = (&current, trimmed.strip_prefix("[env: ")) {
            if let Some(name) = rest.split('=').next() {
                flags.insert(flag.clone(), Some(name.to_owned()));
            }
        }
    }
    flags.remove("--help");
    flags.remove("--version");
    flags
}

#[test]
fn every_command_line_flag_has_a_place_in_the_browser_or_a_reason() {
    let doc = doc();
    let rows = table(&doc, "Command-line flags");
    let mut listed: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut problems = Vec::new();
    for r in &rows {
        if r.len() != 4 {
            problems.push(format!("a flag row has {} cells, not 4: {r:?}", r.len()));
            continue;
        }
        let flag = unquote(&r[0]);
        let env = unquote(&r[1]);
        check_home_or_reason("flag", &flag, &r[2], &r[3], &mut problems);
        let env = (env != "—").then_some(env);
        if listed.insert(flag.clone(), env).is_some() {
            problems.push(format!("flag {flag} appears twice"));
        }
    }
    let actual = flags_from_help();
    assert!(actual.len() >= 100, "--help parsed to {} flags", actual.len());
    let missing: Vec<_> = actual.keys().filter(|f| !listed.contains_key(*f)).collect();
    let stale: Vec<_> = listed.keys().filter(|f| !actual.contains_key(*f)).collect();
    assert!(
        missing.is_empty(),
        "these command-line flags have neither a place in the browser nor a written reason in \
         {DOC} (add a row to `## Command-line flags`):\n  {}",
        missing
            .iter()
            .map(|f| format!("| `{f}` | `{}` | — | <why> |", actual[*f].as_deref().unwrap_or("—")))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(stale.is_empty(), "{DOC} has rows for flags that no longer exist: {stale:?}");
    for (flag, env) in &actual {
        assert_eq!(
            &listed[flag], env,
            "{flag}: the table names a different environment variable than --help does"
        );
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_table_says_when_each_saved_setting_applies_exactly_as_the_pages_do() {
    let doc = doc();
    let rows = table(&doc, "Saved settings");
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for r in &rows {
        assert_eq!(r.len(), 3, "a saved-setting row has {} cells, not 3: {r:?}", r.len());
        assert!(listed.insert(unquote(&r[0]), r[2].clone()).is_none(), "{} twice", r[0]);
        assert!(
            r[1] == "—" || !r[1].is_empty(),
            "{}: the \"changed at\" cell is empty",
            r[0]
        );
    }
    let all = every();
    let actual: BTreeMap<&str, &str> = all.iter().map(|e| (e.key, e.applies.badge())).collect();
    let missing: Vec<_> = actual.keys().filter(|k| !listed.contains_key(**k)).collect();
    let stale: Vec<_> = listed.keys().filter(|k| !actual.contains_key(k.as_str())).collect();
    assert!(
        missing.is_empty(),
        "these saved settings are not in {DOC}'s `## Saved settings` table:\n  {}",
        missing
            .iter()
            .map(|k| format!("| `{k}` | <where> | {} |", actual[**k]))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(stale.is_empty(), "{DOC} lists saved settings that no longer exist: {stale:?}");
    for (key, badge) in &actual {
        assert_eq!(
            &listed[*key], badge,
            "{key}: the table and the settings page say different things about when it applies"
        );
    }
}

/// Every control name the pages can render: scanned from the web crate's
/// markup, plus the settings form's own key list and the table-driven fields.
fn control_names() -> BTreeSet<String> {
    let mut names: BTreeSet<String> = birdnet_web::routes::admin::settings::form::SETTINGS_FORM_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    names.extend(birdnet_web::routes::admin::settings::extra::EXTRA_SETTINGS.iter().map(|e| e.key.to_owned()));
    let mut files: Vec<PathBuf> = Vec::new();
    collect(&root().join("crates/birdnet-web/src"), &mut files);
    collect(&root().join("crates/birdnet-web/templates"), &mut files);
    for f in files {
        let Ok(text) = fs::read_to_string(&f) else { continue };
        for attr in ["name=\"", "id=\""] {
            let mut rest = text.as_str();
            while let Some(i) = rest.find(attr) {
                rest = &rest[i + attr.len()..];
                if let Some(end) = rest.find('"') {
                    names.insert(rest[..end].to_owned());
                }
            }
        }
    }
    names
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "html") {
            out.push(p);
        }
    }
}

/// A "changed at" cell names its control in a code span (`latitude`). That
/// name has to be a control some page really renders, or the table is a
/// description of a page that does not exist.
#[test]
fn every_control_the_table_points_at_exists() {
    let doc = doc();
    let controls = control_names();
    assert!(controls.len() > 100, "the scan found only {} names", controls.len());
    let mut missing = Vec::new();
    for heading in ["Configuration-file keys", "Command-line flags", "Saved settings"] {
        for r in table(&doc, heading) {
            // The "changed at" cell: the second cell, or the third for flags.
            let cell = if heading == "Command-line flags" { &r[2] } else { &r[1] };
            let mut rest = cell.as_str();
            while let Some(i) = rest.find('`') {
                let after = &rest[i + 1..];
                let Some(j) = after.find('`') else { break };
                let token = &after[..j];
                rest = &after[j + 1..];
                let lowercase_name = !token.is_empty()
                    && token.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                let is_field = lowercase_name && (token.contains('_') || token == "latitude" || token == "longitude");
                if is_field && !controls.contains(token) {
                    missing.push(format!("{heading} / {}: `{token}`", r[0]));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "the table points at controls no page renders:\n  {}",
        missing.join("\n  ")
    );
}
