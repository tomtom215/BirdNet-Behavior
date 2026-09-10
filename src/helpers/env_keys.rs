//! The `BIRDNET_*` / `BNB_*` environment variables this binary reads, and the
//! ones set in its environment that it does not (LC-7).
//!
//! clap reads the variables named in `src/cli.rs`'s `env = "…"` attributes
//! and ignores every other one, so `BIRDNET_CONFIDENC=0.9` in a unit file or
//! a `.env` is accepted by everything and changes nothing. The known set is
//! built from the clap definition itself plus the handful of variables read
//! directly; the unit tests below scan the source for those direct reads and
//! compare `.env.example` against the whole set in both directions, so the
//! documented list, the code and this check cannot drift apart.

use std::collections::BTreeSet;

use clap::CommandFactory as _;

use crate::cli::Cli;

/// Variables read with `std::env::var` rather than through clap. Pinned by
/// `direct_reads_match_the_source`, which scans for the literals.
const DIRECT_ENV_KEYS: &[&str] = &[
    "BIRDNET_BASE_PATH",
    "BIRDNET_BIRDWEATHER_URL",
    "BIRDNET_CLIP_PEAK_CEILING_DBFS",
    "BIRDNET_CLIP_TARGET_LUFS",
    "BIRDNET_CORS_ALLOWED_ORIGINS",
    "BIRDNET_DUCKDB_MEMORY_LIMIT",
    "BIRDNET_DYNAMIC_THRESHOLD",
    "BIRDNET_DYNAMIC_THRESHOLD_HOURS",
    "BIRDNET_DYNAMIC_THRESHOLD_MIN",
    "BIRDNET_DYNAMIC_THRESHOLD_TRIGGER",
    "BIRDNET_REQUIRE_LIVE_EXTENSION",
    "BIRDNET_SITENAME",
    "BIRDNET_SPL_CALIBRATION_DB",
    "BIRDNET_TRUSTED_PROXIES",
    "BNB_BASE_URL",
    "BNB_HELP_DIR",
    "BNB_INSTANCE_LOCK_GRACE_SECS",
    "BNB_PUBLIC_URL",
    "BNB_SESSION_SECRET",
    "BNB_SESSION_TTL_DAYS",
    "BNB_SHARE_SECRET",
    "BNB_STATION_LAT",
    "BNB_STATION_LON",
    "BNB_WEATHER_BASE_URL",
    "BNB_WEATHER_ENABLED",
];

/// The prefixes that mark a variable as meant for this station.
const OURS: &[&str] = &["BIRDNET_", "BNB_"];

/// Every environment variable name this binary reads.
#[must_use]
pub fn known_env_names() -> BTreeSet<String> {
    let mut out: BTreeSet<String> = Cli::command()
        .get_arguments()
        .filter_map(|a| a.get_env())
        .map(|e| e.to_string_lossy().into_owned())
        .collect();
    out.extend(DIRECT_ENV_KEYS.iter().map(|k| (*k).to_owned()));
    // Read through a constant rather than a literal.
    out.insert(birdnet_web::api_token::API_TOKEN_KEY.to_owned());
    // The offsite backup reads `BIRDNET_<key>` for each of its config keys.
    out.extend(
        birdnet_core::config::known_keys::KNOWN_CONFIG_KEYS
            .iter()
            .filter(|k| k.starts_with("OFFSITE_"))
            .map(|k| format!("BIRDNET_{k}")),
    );
    // Each credential that can be supplied as a mounted file reads a
    // `BIRDNET_<KEY>_FILE` variable, built from the key rather than written out
    // — so the list here cannot fall behind the list that is actually read.
    out.extend(birdnet_core::config::secret_file::file_env_names());
    // The capture watchdog's knobs are read through a table, so their names
    // never appear as `env::var("…")` literals for the scan below to find.
    out.extend(
        crate::capture::watchdog::WATCHDOG_ENV_KEYS
            .iter()
            .map(|k| (*k).to_owned()),
    );
    out
}

/// A variable set in the environment that this binary does not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEnvVar {
    /// The variable's name.
    pub name: String,
    /// The known variable it is closest to, when one is close enough.
    pub did_you_mean: Option<String>,
}

impl std::fmt::Display for UnknownEnvVar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not a variable this station reads", self.name)?;
        if let Some(meant) = &self.did_you_mean {
            write!(f, "; did you mean {meant}?")?;
        }
        Ok(())
    }
}

/// The `BIRDNET_*` and `BNB_*` variables in this process's environment that
/// nothing reads, sorted by name.
#[must_use]
pub fn unknown_env_vars() -> Vec<UnknownEnvVar> {
    let names: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    unknown_among(&names, &known_env_names())
}

/// The names in `set` with one of our prefixes that are not in `known`.
fn unknown_among(set: &[String], known: &BTreeSet<String>) -> Vec<UnknownEnvVar> {
    let mut out: Vec<UnknownEnvVar> = set
        .iter()
        .filter(|n| OURS.iter().any(|p| n.starts_with(p)))
        .filter(|n| !known.contains(*n))
        .map(|n| UnknownEnvVar {
            name: n.clone(),
            did_you_mean: nearest(n, known),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup();
    out
}

/// The known name within two edits of `name`, if any; the closest when
/// several are.
fn nearest(name: &str, known: &BTreeSet<String>) -> Option<String> {
    known
        .iter()
        .map(|k| (edit_distance(name, k), k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k.clone())
}

/// Levenshtein distance, by chars.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn production_sources() -> Vec<PathBuf> {
        let root = workspace_root();
        let mut roots = vec![root.join("src")];
        for entry in std::fs::read_dir(root.join("crates")).expect("crates/") {
            let dir = entry.expect("entry").path();
            if dir.join("src").is_dir() {
                roots.push(dir.join("src"));
            }
        }
        let mut files = Vec::new();
        for r in roots {
            walk(&r, &mut files);
        }
        files
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_none_or(|n| n != "tests.rs")
            {
                out.push(path);
            }
        }
    }

    /// The source with every `#[cfg(test)]`-attributed item removed by brace
    /// depth (a `mod tests { … }` block or a one-line `mod tests;`), and with
    /// comment lines dropped so prose about a read is not a read.
    fn production_half(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let mut lines = src.lines();
        while let Some(line) = lines.next() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.trim_start().starts_with("#[cfg(test)]") {
                let mut depth: i32 = 0;
                let mut seen_open = false;
                for l in lines.by_ref() {
                    depth += i32::try_from(l.matches('{').count()).unwrap_or(0);
                    depth -= i32::try_from(l.matches('}').count()).unwrap_or(0);
                    if l.contains('{') {
                        seen_open = true;
                    }
                    if l.trim_start().starts_with('#') && !seen_open {
                        continue;
                    }
                    if (seen_open && depth <= 0) || (!seen_open && l.trim_end().ends_with(';')) {
                        break;
                    }
                }
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    fn is_ours(name: &str) -> bool {
        OURS.iter().any(|p| name.starts_with(p))
    }

    /// `BIRDNET_*` / `BNB_*` names read directly in the production source:
    /// `env::var("X")`, `env::var_os("X")`, and the dynamic-threshold
    /// closures `flag("X")` / `number("X")`.
    fn direct_reads_in_source() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for path in production_sources() {
            let src = std::fs::read_to_string(&path).expect("read");
            let src = production_half(&src);
            let src = src.as_str();
            for shape in [
                "env::var(\"",
                "env::var_os(\"",
                "flag(\"",
                "number(\"",
                "resolve_decimal_env(\"",
            ] {
                for (idx, _) in src.match_indices(shape) {
                    let rest = &src[idx + shape.len()..];
                    if let Some(end) = rest.find('"') {
                        let name = &rest[..end];
                        if is_ours(name) {
                            out.insert(name.to_owned());
                        }
                    }
                }
            }
        }
        out
    }

    /// Keys named in `.env.example`, commented or not.
    fn env_example_keys() -> BTreeSet<String> {
        let text = std::fs::read_to_string(workspace_root().join(".env.example")).expect("read");
        text.lines()
            .filter_map(|l| {
                let l = l.trim_start_matches('#').trim_start();
                let (key, _) = l.split_once('=')?;
                let key = key.trim();
                (!key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .then(|| key.to_owned())
            })
            .collect()
    }

    /// Names the container tooling consumes without the binary reading them.
    fn docker_only_names() -> BTreeSet<String> {
        let root = workspace_root();
        let mut files = vec![root.join("Dockerfile")];
        for entry in std::fs::read_dir(&root).expect("root") {
            let p = entry.expect("entry").path();
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("docker-compose"))
            {
                files.push(p);
            }
        }
        if let Ok(rd) = std::fs::read_dir(root.join("docker")) {
            files.extend(rd.map(|e| e.expect("entry").path()));
        }
        let mut out = BTreeSet::new();
        for f in files {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let bytes = text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i].is_ascii_uppercase()
                    && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                {
                    let start = i;
                    while i < bytes.len()
                        && (bytes[i].is_ascii_uppercase()
                            || bytes[i].is_ascii_digit()
                            || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    out.insert(text[start..i].to_owned());
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    /// `DIRECT_ENV_KEYS` is exactly the set of our variables the source reads
    /// without clap, in both directions.
    #[test]
    fn direct_reads_match_the_source() {
        let in_source = direct_reads_in_source();
        let listed: BTreeSet<String> = DIRECT_ENV_KEYS.iter().map(|k| (*k).to_owned()).collect();
        let missing: Vec<&String> = in_source.difference(&listed).collect();
        let stale: Vec<&String> = listed.difference(&in_source).collect();
        assert!(in_source.len() >= 15, "the scan found only {in_source:?}");
        assert!(
            missing.is_empty(),
            "read with env::var in the source and not in DIRECT_ENV_KEYS, so the \
             station would call them unknown: {missing:?}"
        );
        assert!(
            stale.is_empty(),
            "in DIRECT_ENV_KEYS and read nowhere: {stale:?}"
        );
    }

    /// `.env.example` documents every variable the binary reads, and names
    /// nothing the binary, the container tooling or the config file reads.
    #[test]
    fn env_example_matches_what_is_read() {
        let documented = env_example_keys();
        let known = known_env_names();
        let docker = docker_only_names();
        let conf: BTreeSet<String> = birdnet_core::config::known_keys::KNOWN_CONFIG_KEYS
            .iter()
            .map(|k| (*k).to_owned())
            .collect();

        // Every clap and direct read is documented.
        let readable_undocumented: Vec<&String> = known
            .iter()
            .filter(|k| !documented.contains(*k))
            // The offsite `BIRDNET_OFFSITE_*` names are documented as a family;
            // each one is, in fact, listed, so no exemption is needed here.
            .collect();
        assert!(
            readable_undocumented.is_empty(),
            "read by the binary and not in .env.example: {readable_undocumented:?}"
        );

        // Everything documented is read by something: the binary, the
        // container tooling (entrypoint, compose, Dockerfile), or — for the
        // config-file-only keys the file documents as such — the config parser.
        let dead: Vec<&String> = documented
            .iter()
            .filter(|k| !known.contains(*k) && !docker.contains(*k) && !conf.contains(*k))
            .filter(|k| !matches!(k.as_str(), "CADDY_USER" | "CADDY_PWD"))
            .collect();
        assert!(
            dead.is_empty(),
            "in .env.example and read by nothing: {dead:?}"
        );
    }

    #[test]
    fn an_unknown_variable_is_reported_with_its_nearest_known_name() {
        let known = known_env_names();
        let set = vec![
            "BIRDNET_CONFIDENC".to_owned(),
            "BIRDNET_LATITUDE".to_owned(),
            "PATH".to_owned(),
            "BNB_FROBNICATE".to_owned(),
        ];
        let unknown = unknown_among(&set, &known);
        assert_eq!(unknown.len(), 2, "{unknown:?}");
        assert_eq!(unknown[0].name, "BIRDNET_CONFIDENC");
        assert!(
            unknown[0].did_you_mean.is_none(),
            "BIRDNET_CONFIDENCE is not a variable (confidence lives in the settings table), \
             so nothing should be suggested: {unknown:?}"
        );
        assert_eq!(unknown[1].name, "BNB_FROBNICATE");
        assert_eq!(unknown[1].did_you_mean, None);

        let typo = unknown_among(&["BIRDNET_LATITUD".to_owned()], &known);
        assert_eq!(typo[0].did_you_mean.as_deref(), Some("BIRDNET_LATITUDE"));
        assert_eq!(
            typo[0].to_string(),
            "BIRDNET_LATITUD is not a variable this station reads; did you mean BIRDNET_LATITUDE?"
        );
    }
}
