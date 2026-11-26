//! What a station does with nothing configured: look where the browser put things.
//!
//! A person who runs the binary with no `birdnet.conf`, no flags and no
//! environment used to get a web page and no listening at all: the detection
//! daemon needs a model, a labels file and a directory to watch, those came
//! only from `--model` / `--labels` / `--watch-dir` or `MODEL_PATH` /
//! `LABELS_PATH` / `RECS_DIR`, and the log said "configure --model, --labels,
//! --watch-dir … and restart". The setup wizard can now install the classifier
//! from the browser; this is the other half, so that what it installed is what
//! the next start runs.
//!
//! # The rule
//!
//! Applied once, right after the command line and the config file are read, and
//! only to what neither of them set:
//!
//! * **The classifier.** When neither `--model`/`--labels` nor `MODEL_PATH`/
//!   `LABELS_PATH` is set, and the default classifier (BirdNET V2.4) is in the
//!   station's model directory and verifies against its pins
//!   ([`is_installed`]), use it.
//! * **The occurrence filter.** When nothing names one (`--metadata-model`,
//!   `METADATA_MODEL_PATH`), and the classifier the station runs is a
//!   catalogue model at the name an install gives it, use the filter that
//!   belongs to that classifier ([`occurrence_filter_for`]) when it is
//!   installed beside it and verifies — with its labels file, for the
//!   geomodel, unless `METADATA_LABELS_PATH` names one. This is what makes a
//!   filter installed from the browser the one the next start runs (MDL-14):
//!   the service cannot write its configuration file (the installer leaves
//!   it `root`-owned and read-only to the service), an installer whose filter
//!   download failed leaves `METADATA_MODEL_PATH` commented out, and a
//!   container restarted from the dashboard runs again with the environment
//!   it started with, which named no filter.
//! * **The watch directory.** When neither `--watch-dir` nor `RECS_DIR` is set,
//!   the capture segments go to a `birdnet-stream` directory under the system
//!   temporary directory, which is RAM-backed on the Raspberry Pi images and is
//!   what the installer's own unit passes (`/tmp/birdnet-stream`).
//!
//! An explicit setting is never overridden, a classifier the operator chose is
//! never swapped for this one, and nothing is created here: the daemon makes its
//! own watch directory, and the model directory is the installer's.

use std::path::{Path, PathBuf};

use birdnet_core::config::Config;
use birdnet_integrations::model_catalog::{
    CATALOG, CatalogEntry, DEFAULT_CLASSIFIER_ID, find, installed_paths, is_installed,
    occurrence_filter_for,
};

use crate::cli::Cli;

/// The directory a classifier installed from the browser (or by
/// `--install-model`) lands in, and the daemon looks in.
///
/// In order: `BIRDNET_MODEL_DIR` (the name the shipped compose file sets), the
/// config's `MODEL_DIR`, `/data/model` when that directory exists (a container
/// that predates the variable), and otherwise a `models` directory beside the
/// database — which is where the installer puts it (`$DATA_DIR/models`), so a
/// bare binary and an installed station agree.
#[must_use]
pub fn model_dir(config: Option<&Config>) -> PathBuf {
    model_dir_with(
        std::env::var("BIRDNET_MODEL_DIR").ok().as_deref(),
        config,
        Path::new("/data/model").is_dir(),
        &crate::helpers::db_path_from_config(config),
    )
}

/// [`model_dir`], with the environment and the filesystem passed in.
#[must_use]
pub fn model_dir_with(
    env_dir: Option<&str>,
    config: Option<&Config>,
    legacy_container_dir_exists: bool,
    db_path: &Path,
) -> PathBuf {
    if let Some(dir) = env_dir.map(str::trim).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = config
        .and_then(|c| c.get("MODEL_DIR"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return PathBuf::from(dir);
    }
    if legacy_container_dir_exists {
        return PathBuf::from("/data/model");
    }
    db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("models"), |p| p.join("models"))
}

/// What [`apply`] decided, for the start-up log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The default classifier was found installed and will be used.
    pub classifier: bool,
    /// The occurrence filter that belongs to the classifier was found
    /// installed beside it and will be used.
    pub range_filter: bool,
    /// The watch directory that was filled in.
    pub watch_dir: Option<PathBuf>,
}

/// Fill in, on `cli`, the model, labels, range filter and watch directory that
/// neither the command line nor `config` named. See the module header.
///
/// A default watch directory is created here, not left to the detection daemon:
/// the daemon only starts once a classifier is installed, and the live
/// spectrogram on a station that has none yet exited with "No path was found"
/// for the directory nobody had made.
pub fn apply(cli: &mut Cli, config: Option<&Config>) -> Applied {
    let applied = apply_in(
        cli,
        config,
        &model_dir(config),
        &default_stream_dir(),
        &is_installed,
    );
    if let Some(dir) = &applied.watch_dir
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        tracing::warn!(error = %e, dir = %dir.display(), "could not create the default stream directory");
    }
    applied
}

/// [`apply`], with the model directory, the default stream directory and the
/// answer to "is this installed" passed in, so a test does not depend on the
/// machine it runs on or on 54 MB of real model.
pub fn apply_in(
    cli: &mut Cli,
    config: Option<&Config>,
    model_dir: &Path,
    stream_dir: &Path,
    installed: &dyn Fn(&CatalogEntry, &Path) -> bool,
) -> Applied {
    let mut applied = Applied::default();
    let set = |key: &str| {
        config
            .and_then(|c| c.get(key))
            .is_some_and(|v| !v.trim().is_empty())
    };

    // Both halves or neither: a station that named its own model but not its
    // labels is misconfigured, and quietly pairing its model with ours would
    // report species under other species' names.
    let nothing_named =
        cli.model.is_none() && cli.labels.is_none() && !set("MODEL_PATH") && !set("LABELS_PATH");
    if nothing_named
        && !cli.web_only
        && let Some(entry) = find(DEFAULT_CLASSIFIER_ID)
        && installed(entry, model_dir)
    {
        let (model, labels) = installed_paths(entry, model_dir);
        if let Some(labels) = labels {
            cli.model = Some(model);
            cli.labels = Some(labels);
            applied.classifier = true;
        }
    }

    // The filter of the classifier this start runs, from beside it.
    let classifier = cli.model.clone().or_else(|| {
        config
            .and_then(|c| c.get("MODEL_PATH"))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    });
    if cli.metadata_model.is_none()
        && !set("METADATA_MODEL_PATH")
        && !cli.web_only
        && let Some(classifier) = classifier
        && let Some(dir) = classifier.parent()
        && let Some(entry) = CATALOG
            .iter()
            .find(|e| installed_paths(e, dir).0 == classifier)
        && let Some(filter) = occurrence_filter_for(entry.id)
        && installed(filter, dir)
    {
        let (model, labels) = installed_paths(filter, dir);
        cli.metadata_model = Some(model);
        if let Some(labels) = labels
            && cli.metadata_labels.is_none()
            && !set("METADATA_LABELS_PATH")
        {
            cli.metadata_labels = Some(labels);
        }
        applied.range_filter = true;
    }

    if cli.watch_dir.is_none() && !set("RECS_DIR") && !cli.web_only {
        cli.watch_dir = Some(stream_dir.to_path_buf());
        applied.watch_dir = Some(stream_dir.to_path_buf());
    }
    applied
}

/// Where capture segments go when nothing says.
#[must_use]
pub fn default_stream_dir() -> PathBuf {
    std::env::temp_dir().join("birdnet-stream")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::test_support::config_with;
    use clap::Parser as _;

    fn bare() -> Cli {
        Cli::parse_from(["birdnet-behavior"])
    }

    /// Whether the entry's model file is there: the stand-in for the
    /// catalogue's verification in the tests of the rule itself.
    fn present(entry: &CatalogEntry, dir: &Path) -> bool {
        installed_paths(entry, dir).0.exists()
    }

    /// [`apply_in`] with [`present`] as the installed check.
    fn apply_here(cli: &mut Cli, config: Option<&Config>, dir: &Path) -> Applied {
        apply_in(cli, config, dir, Path::new("/tmp/s"), &present)
    }

    /// Put the default classifier (and optionally its range filter) in `dir`
    /// with the sizes the catalogue pins (and none of its bytes).
    fn install_default(dir: &Path, with_range: bool) {
        let model = find(DEFAULT_CLASSIFIER_ID).unwrap();
        let (m, l) = installed_paths(model, dir);
        let f = std::fs::File::create(&m).unwrap();
        f.set_len(model.model_bytes).unwrap();
        std::fs::write(l.unwrap(), "x\n").unwrap();
        if with_range {
            let range = find(birdnet_integrations::model_catalog::DEFAULT_RANGE_FILTER_ID).unwrap();
            let f = std::fs::File::create(installed_paths(range, dir).0).unwrap();
            f.set_len(range.model_bytes).unwrap();
        }
    }

    /// The reproduction of the bare-binary start: nothing configured anywhere,
    /// the classifier installed from the browser. The daemon needs all three
    /// paths, and gets them.
    #[test]
    fn an_installed_default_classifier_is_used_when_nothing_names_one() {
        let dir = tempfile::tempdir().unwrap();
        install_default(dir.path(), true);
        let mut cli = bare();
        let applied = apply_here(&mut cli, None, dir.path());
        assert!(applied.classifier && applied.range_filter, "{applied:?}");
        assert_eq!(
            cli.model.as_deref(),
            Some(dir.path().join("birdnet-v24.onnx").as_path())
        );
        assert_eq!(
            cli.labels.as_deref(),
            Some(dir.path().join("birdnet-v24_labels.txt").as_path())
        );
        assert_eq!(
            cli.metadata_model.as_deref(),
            Some(dir.path().join("birdnet-v24-range-v2.onnx").as_path())
        );
        assert_eq!(cli.watch_dir.as_deref(), Some(Path::new("/tmp/s")));
    }

    /// The counterpart: nothing installed means nothing is invented. A path to
    /// a file that is not there would turn "not installed" into a start-up
    /// failure with a worse message.
    #[test]
    fn nothing_installed_leaves_the_model_unset() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = bare();
        let applied = apply_here(&mut cli, None, dir.path());
        assert!(!applied.classifier && !applied.range_filter);
        assert!(cli.model.is_none() && cli.labels.is_none() && cli.metadata_model.is_none());
    }

    /// MDL-11: what counts as installed is the catalogue's verification, so a
    /// half-copied model, or one of the right size whose bytes are not the
    /// model, is not adopted at start.
    ///
    /// Observed failing with the size-only check this replaced
    /// (`installed_paths(e, d).0` of the pinned length): the 54 MB of zeros
    /// was adopted as the classifier.
    #[test]
    fn a_model_that_does_not_verify_is_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        install_default(dir.path(), true);
        let mut cli = bare();
        let applied = apply_in(
            &mut cli,
            None,
            dir.path(),
            Path::new("/tmp/s"),
            &is_installed,
        );
        assert!(
            !applied.classifier,
            "zeros of the pinned size are not the model"
        );
        assert!(cli.model.is_none() && cli.metadata_model.is_none());

        let model = find(DEFAULT_CLASSIFIER_ID).unwrap();
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(installed_paths(model, dir.path()).0)
            .unwrap();
        f.set_len(model.model_bytes - 1).unwrap();
        let mut cli = bare();
        assert!(
            !apply_in(
                &mut cli,
                None,
                dir.path(),
                Path::new("/tmp/s"),
                &is_installed
            )
            .classifier
        );
    }

    /// An operator's own model beats the default, whether they named it on the
    /// command line or in the file, and so does a labels file alone.
    #[test]
    fn a_named_model_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        install_default(dir.path(), true);
        for conf in [
            &[("MODEL_PATH", "/m/own.onnx")][..],
            &[("LABELS_PATH", "/m/own.txt")][..],
        ] {
            let cfg = config_with(conf);
            let mut cli = bare();
            let applied = apply_here(&mut cli, Some(&cfg), dir.path());
            assert!(!applied.classifier, "{conf:?}");
            assert!(cli.model.is_none() && cli.labels.is_none(), "{conf:?}");
        }
        let mut cli = Cli::parse_from(["birdnet-behavior", "--model", "/m/own.onnx"]);
        assert!(!apply_here(&mut cli, None, dir.path()).classifier);
        assert_eq!(cli.model.as_deref(), Some(Path::new("/m/own.onnx")));
        assert!(cli.labels.is_none());
    }

    /// The range filter is adopted only with the classifier it belongs to, and
    /// never over a filter the operator named.
    #[test]
    fn the_range_filter_follows_the_rules_of_the_classifier() {
        let dir = tempfile::tempdir().unwrap();
        install_default(dir.path(), true);
        let cfg = config_with(&[("METADATA_MODEL_PATH", "/m/geo.onnx")]);
        let mut cli = bare();
        let applied = apply_here(&mut cli, Some(&cfg), dir.path());
        assert!(applied.classifier && !applied.range_filter, "{applied:?}");
        assert!(cli.metadata_model.is_none());
    }

    /// MDL-14: a filter installed from the browser beside a classifier the
    /// configuration names — a bare-metal station whose installer left
    /// `METADATA_MODEL_PATH` commented out, or a container restarted from the
    /// dashboard with the environment it started with — is the one the next
    /// start runs: the V2.4 range filter for V2.4, the geomodel and its labels
    /// for V3.0, and never one for the other.
    #[test]
    fn the_filter_installed_beside_a_named_classifier_is_used() {
        use birdnet_integrations::model_catalog::{
            DEFAULT_RANGE_FILTER_ID, V3_CLASSIFIER_ID, V3_GEOMODEL_ID,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = |id: &str| installed_paths(find(id).unwrap(), dir.path());
        let touch = |p: &Path| std::fs::write(p, b"x").unwrap();
        touch(&path(V3_CLASSIFIER_ID).0);
        touch(&path(DEFAULT_RANGE_FILTER_ID).0);

        // V3.0 named, only V2.4's filter there: nothing adopted.
        let v3 = path(V3_CLASSIFIER_ID).0;
        let cfg = config_with(&[("MODEL_PATH", v3.to_str().unwrap())]);
        let mut cli = bare();
        assert!(!apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
        assert!(cli.metadata_model.is_none());

        // The geomodel installed beside it: adopted, with its labels.
        let (geo, geo_labels) = path(V3_GEOMODEL_ID);
        touch(&geo);
        let mut cli = bare();
        assert!(apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
        assert_eq!(cli.metadata_model.as_deref(), Some(geo.as_path()));
        assert_eq!(cli.metadata_labels, geo_labels);
        assert!(
            cli.model.is_none(),
            "the classifier stays the configuration's"
        );

        // The same through the environment a container passes.
        let mut cli = Cli::parse_from(["birdnet-behavior", "--model", v3.to_str().unwrap()]);
        assert!(apply_here(&mut cli, None, dir.path()).range_filter);
        assert_eq!(cli.metadata_model.as_deref(), Some(geo.as_path()));

        // V2.4 named: its own filter.
        install_default(dir.path(), true);
        let v24 = path(DEFAULT_CLASSIFIER_ID).0;
        let cfg = config_with(&[("MODEL_PATH", v24.to_str().unwrap())]);
        let mut cli = bare();
        assert!(apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
        assert_eq!(
            cli.metadata_model.as_deref(),
            Some(path(DEFAULT_RANGE_FILTER_ID).0.as_path())
        );
        assert!(cli.metadata_labels.is_none(), "the range filter has none");

        // The counterparts: a filter the operator named, labels the operator
        // named, and a classifier that is not the catalogue's file.
        let cfg = config_with(&[
            ("MODEL_PATH", v3.to_str().unwrap()),
            ("METADATA_MODEL_PATH", "/m/own-geo.onnx"),
        ]);
        let mut cli = bare();
        assert!(!apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
        assert!(cli.metadata_model.is_none());
        let cfg = config_with(&[
            ("MODEL_PATH", v3.to_str().unwrap()),
            ("METADATA_LABELS_PATH", "/m/own-labels.txt"),
        ]);
        let mut cli = bare();
        assert!(apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
        assert!(
            cli.metadata_labels.is_none(),
            "the named labels are the config's"
        );
        let own = dir.path().join("own.onnx");
        let cfg = config_with(&[("MODEL_PATH", own.to_str().unwrap())]);
        let mut cli = bare();
        assert!(!apply_here(&mut cli, Some(&cfg), dir.path()).range_filter);
    }

    /// An explicit watch directory, or a `RECS_DIR`, is left alone; web-only
    /// has no daemon to feed.
    #[test]
    fn the_watch_directory_is_filled_in_only_when_nothing_names_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = Cli::parse_from(["birdnet-behavior", "--watch-dir", "/w"]);
        assert!(apply_here(&mut cli, None, dir.path()).watch_dir.is_none());
        let cfg = config_with(&[("RECS_DIR", "/r")]);
        let mut cli = bare();
        assert!(
            apply_here(&mut cli, Some(&cfg), dir.path())
                .watch_dir
                .is_none()
        );
        let mut cli = Cli::parse_from(["birdnet-behavior", "--web-only"]);
        let applied = apply_here(&mut cli, None, dir.path());
        assert!(applied.watch_dir.is_none() && cli.watch_dir.is_none());
    }

    #[test]
    fn the_model_directory_follows_the_documented_order() {
        let db = Path::new("/home/u/BirdNet-Behavior/birds.db");
        let cfg = config_with(&[("MODEL_DIR", "/cfg/models")]);
        assert_eq!(
            model_dir_with(Some("/env/m"), Some(&cfg), true, db),
            Path::new("/env/m")
        );
        assert_eq!(
            model_dir_with(Some("  "), Some(&cfg), true, db),
            Path::new("/cfg/models"),
            "a blank variable is no variable"
        );
        assert_eq!(
            model_dir_with(None, None, true, db),
            Path::new("/data/model")
        );
        assert_eq!(
            model_dir_with(None, None, false, db),
            Path::new("/home/u/BirdNet-Behavior/models"),
            "beside the database, where the installer puts it"
        );
    }
}
