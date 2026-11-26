//! The catalogue's own gates, and the install path's file handling. The bad
//! network is in `tests/network.rs`.

mod network;
mod origin;

use std::path::Path;

use super::{
    CATALOG, FREE_SPACE_MARGIN, FileSpec, FileState, InstallError, Rehash, Use, find, fits,
    free_space_bytes, install, installed_paths, installed_state, labels_origins, model_origins,
    record, require_digest, stage,
};

/// The sha256 of `body`, leaked to the `'static` the catalogue's fields are.
pub(super) fn digest_of(body: &[u8]) -> &'static str {
    use sha2::Digest as _;
    Box::leak(record::hex(&sha2::Sha256::digest(body)).into_boxed_str())
}

/// No staging file of any name is left in `dir`.
pub(super) fn assert_no_part_files(dir: &Path, after: &str) {
    let left: Vec<String> = std::fs::read_dir(dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|e| e == "part" || e == "download")
        })
        .collect();
    assert!(
        left.is_empty(),
        "staging files left after {after}: {left:?}"
    );
}

/// Every digest in the catalogue must be a well-formed sha256.
///
/// Observed failing with one hex character removed from the Perch digest:
/// the length assertion went red.
#[test]
fn every_catalogue_digest_is_a_well_formed_sha256() {
    assert!(!CATALOG.is_empty(), "an empty catalogue installs nothing");
    for entry in CATALOG {
        assert_eq!(
            entry.model_sha256.len(),
            64,
            "{}: model digest is not 64 characters",
            entry.id
        );
        assert!(
            entry
                .model_sha256
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{}: model digest is not lowercase hex",
            entry.id
        );
        assert!(
            require_digest(entry.id, entry.model_sha256).is_ok(),
            "{}",
            entry.id
        );
        if let Some(sha) = entry.labels_sha256 {
            assert!(require_digest(entry.id, sha).is_ok(), "{} labels", entry.id);
        }
    }
}

/// A labels source without its digest, or the reverse, would either install
/// an unverified file or silently skip one an operator was promised; a file
/// that is both downloaded and embedded has no answer to "which one did I
/// get"; and a downloaded file needs the size its origins are held to.
#[test]
fn labels_are_either_fully_specified_or_absent() {
    for entry in CATALOG {
        assert_eq!(
            entry.ships_labels(),
            entry.labels_sha256.is_some(),
            "{}: a labels source and its digest must travel together",
            entry.id
        );
        assert!(
            entry.labels_url.is_none() || entry.labels_embedded.is_none(),
            "{}: labels are downloaded or embedded, not both",
            entry.id
        );
        assert_eq!(
            entry.labels_url.is_some(),
            entry.labels_bytes > 0,
            "{}: a downloaded labels file carries its size, and only one does",
            entry.id
        );
        assert_eq!(
            entry.ships_labels(),
            !entry.labels_ext.is_empty(),
            "{}: labels have an extension, and only labels",
            entry.id
        );
        assert!(
            entry.labels_url.is_some() || entry.labels_mirror_urls.is_empty(),
            "{}: a mirror of labels nobody downloads",
            entry.id
        );
        if let Some(text) = entry.labels_embedded {
            // The extension follows the content: a CSV with its header, or one
            // class per line.
            let want = if text.starts_with("sci_name") {
                "csv"
            } else {
                "txt"
            };
            assert_eq!(entry.labels_ext, want, "{}", entry.id);
        }
    }
}

/// The embedded labels are the bytes the pinned digest names, and installing
/// them writes exactly those bytes — with no network at all.
#[test]
fn embedded_labels_install_without_a_network_and_verify() {
    let entry = find("birdnet-v24").expect("in the catalogue");
    let text = entry.labels_embedded.expect("V2.4's labels are embedded");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("birdnet-v24_labels.txt");
    stage::write_verified(text.as_bytes(), entry.labels_sha256.expect("digest"), &path)
        .expect("the embedded bytes match their own digest");
    assert_eq!(std::fs::read_to_string(&path).expect("read back"), text);
    let refused = dir.path().join("refused_labels.txt");
    let err = stage::write_verified(
        b"Pica pica_Common Magpie\n",
        entry.labels_sha256.expect("digest"),
        &refused,
    )
    .expect_err("wrong bytes must not verify");
    assert!(matches!(err, InstallError::Integrity { .. }), "{err:?}");
    assert!(!refused.exists());
}

/// Ids are what an operator types and what the installed file is named, so
/// two entries sharing one would overwrite each other.
#[test]
fn catalogue_ids_are_unique_and_filename_safe() {
    let mut ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate catalogue id");
    for entry in CATALOG {
        assert!(
            entry
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{}: id becomes a filename and a routing target",
            entry.id
        );
        assert!(
            entry.model_bytes > 0,
            "{}: size drives the space check",
            entry.id
        );
        if entry.usage == Use::RangeFilter {
            assert_eq!(
                entry.sample_rate, 0,
                "{}: a filter hears no audio",
                entry.id
            );
            let pair = entry
                .pairs_with
                .and_then(find)
                .unwrap_or_else(|| panic!("{}: a filter names its classifier", entry.id));
            assert_ne!(pair.usage, Use::RangeFilter, "{}", entry.id);
        } else {
            assert!(entry.sample_rate > 0, "{}", entry.id);
            assert!(entry.pairs_with.is_none(), "{}", entry.id);
        }
    }
}

/// Every URL is HTTPS.
#[test]
fn every_catalogue_url_is_https() {
    for entry in CATALOG {
        for url in model_origins(entry)
            .into_iter()
            .chain(labels_origins(entry))
        {
            assert!(url.starts_with("https://"), "{}: {url}", entry.id);
        }
    }
}

/// **A digest that cannot verify is refused before anything downloads.**
///
/// Observed failing with the length and hex checks removed from
/// `require_digest`: every malformed digest was accepted.
#[test]
fn a_malformed_digest_is_refused_before_the_network() {
    for bad in ["", "abc", &"z".repeat(64), &"a".repeat(63), &"A".repeat(65)] {
        let err = require_digest("x", bad).expect_err("must refuse");
        assert!(matches!(err, InstallError::Unverifiable { .. }), "{bad}");
        assert!(
            err.to_string().contains("could be anything"),
            "the reason must be legible: {err}"
        );
    }
    assert!(require_digest("x", &"a".repeat(64)).is_ok());
    assert!(require_digest("x", CATALOG[0].model_sha256).is_ok());
}

/// An unknown id names what is available.
#[test]
fn an_unknown_id_lists_what_is_available() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = install("perch-v3", dir.path()).expect_err("unknown id");
    let msg = err.to_string();
    assert!(msg.contains("perch-v3"), "{msg}");
    assert!(msg.contains("perch-v2"), "it must list the real ids: {msg}");
    assert!(msg.contains("birdnet-v3-geomodel"), "{msg}");
}

/// **The refusal that keeps a station recording.**
///
/// Observed failing with the `available < needed` comparison removed from
/// `fits`: a download far larger than the free space was admitted.
#[test]
fn a_download_that_would_not_fit_is_refused() {
    let err = fits(400 * 1024 * 1024, Some(100 * 1024 * 1024)).expect_err("400 MiB into 100 MiB");
    match &err {
        InstallError::NotEnoughSpace { needed, available } => {
            assert!(
                *needed > 400 * 1024 * 1024,
                "the margin must be counted too"
            );
            assert_eq!(*available, 100 * 1024 * 1024);
        }
        other => panic!("wrong error: {other:?}"),
    }
    assert!(
        err.to_string().contains("stop the station recording"),
        "{err}"
    );
}

/// Its counterpart: a download that comfortably fits is admitted.
#[test]
fn a_download_with_room_to_spare_is_admitted() {
    assert!(fits(400 * 1024 * 1024, Some(8 * 1024 * 1024 * 1024)).is_ok());
}

/// **The margin is real, not decorative.**
///
/// Observed failing with `FREE_SPACE_MARGIN` added as 0: an exact fit was
/// admitted.
#[test]
fn a_download_that_exactly_fills_the_disk_is_refused() {
    let size = 400 * 1024 * 1024;
    assert!(fits(size, Some(size)).is_err(), "exactly filling the card");
    assert!(fits(size, Some(size + FREE_SPACE_MARGIN)).is_ok());
}

/// **Free space that cannot be read is none, never plenty.**
///
/// Observed failing with `unwrap_or(0)` changed to `unwrap_or(u64::MAX)` in
/// `fits`.
#[test]
fn unreadable_free_space_is_not_read_as_plenty() {
    assert!(fits(1, None).is_err());
    assert_eq!(free_space_bytes(Path::new("/nonexistent-path-xyz")), None);
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(free_space_bytes(dir.path()).is_some());
}

/// The entries this build ships, pinned. Changing a URL, a digest or a size
/// is changing what a station downloads and what it checks it against, and
/// should be a deliberate act with this test in the diff.
#[test]
fn the_catalogue_is_what_this_build_was_verified_against() {
    let v3 = find("birdnet-v3").expect("birdnet-v3");
    assert_eq!(
        v3.model_sha256,
        "2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743"
    );
    assert_eq!(v3.model_bytes, 541_391_777);
    assert_eq!(v3.sample_rate, 32_000);
    assert_eq!(v3.labels_bytes, 809_172);

    let geo = find("birdnet-v3-geomodel").expect("the geomodel is in the catalogue");
    assert_eq!(
        geo.model_sha256,
        "b151f680a47de5371f39b3df129aea5946ac6baa039582274f833b42eaf992ea"
    );
    assert_eq!(geo.model_bytes, 14_700_313);
    assert_eq!(
        geo.labels_sha256,
        Some("c15818db07e55978d909a9bcd916cd0615b0183f789227d9516059151787c784")
    );
    assert_eq!(geo.labels_bytes, 571_793);
    assert_eq!(geo.usage, Use::RangeFilter);
    assert_eq!(geo.pairs_with, Some("birdnet-v3"));
    assert!(geo.licence.is_some_and(|l| l.starts_with("CC BY-SA 4.0")));

    let perch = find("perch-v2").expect("perch-v2");
    assert_eq!(
        perch.model_sha256,
        "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f"
    );
    assert_eq!(perch.model_bytes, 409_148_616);
    assert_eq!(perch.sample_rate, 32_000);
    assert!(perch.labels_url.is_none() && perch.labels_embedded.is_some());
    assert_eq!(
        perch.labels_sha256,
        Some(birdnet_core::inference::labels::PERCH_V2_LABELS_SHA256)
    );
    assert!(perch.notes.contains("14 795"), "{}", perch.notes);

    let v24 = find("birdnet-v24").expect("birdnet-v24");
    assert_eq!(
        v24.model_sha256,
        "3b72e88b3ad0c310a41adabccf8cf75b1a05daeeb40884ebd38038c91d0e423d"
    );
    assert_eq!(v24.model_bytes, 54_068_648);
    assert_eq!(v24.sample_rate, 48_000);
    assert_eq!(v24.usage, Use::PrimaryReplacement);
    assert_eq!(
        v24.labels_sha256,
        Some("3773334c2ad218bde0ac083dd0b6e839a5cf34e84a6017830b9f446fbb57a215")
    );
    assert!(v24.labels_url.is_none() && v24.labels_embedded.is_some());

    let range = find("birdnet-v24-range-v2").expect("the V2.4 range filter");
    assert_eq!(
        range.model_sha256,
        "b1810073d16c6c39df95a7419f9a11a02f98dbea562c96d4993c6af52b54b271"
    );
    assert_eq!(range.model_bytes, 29_524_170);
    assert_eq!(range.usage, Use::RangeFilter);
    assert_eq!(range.pairs_with, Some("birdnet-v24"));
    assert!(range.labels_url.is_none());
}

/// MDL-12: every downloaded file has its origins in order — this project's
/// release first, then the publisher's — and V3.0 and its labels fall back to
/// the Zenodo record the shell installers used, and the geomodel to the
/// upstream birdnet-team release. Perch's only origin is Hugging Face until a
/// release of this project's exists.
///
/// Observed failing on the catalogue before this change: the V3.0 model had
/// one origin, `["https://github.com/tomtom215/BirdNet-Behavior/releases/
/// download/models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_FP32.onnx"]`.
#[test]
fn every_download_has_its_origins_in_order() {
    const RELEASE: &str = "https://github.com/tomtom215/BirdNet-Behavior/releases/download/";
    const ZENODO: &str = "https://zenodo.org/api/records/18247420/files/";
    const GEO: &str = "https://github.com/birdnet-team/geomodel/releases/download/v3.0.2/";
    let v3 = find("birdnet-v3").expect("entry");
    assert_eq!(
        model_origins(v3),
        vec![
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_FP32.onnx",
            "https://zenodo.org/api/records/18247420/files/BirdNET+_V3.0-preview3_Global_11K_FP32.onnx/content",
        ]
    );
    assert_eq!(
        labels_origins(v3),
        vec![
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_Labels.csv",
            "https://zenodo.org/api/records/18247420/files/BirdNET+_V3.0-preview3_Global_11K_Labels.csv/content",
        ]
    );
    let geo = find("birdnet-v3-geomodel").expect("entry");
    for origins in [model_origins(geo), labels_origins(geo)] {
        assert_eq!(origins.len(), 2, "{origins:?}");
        assert!(origins[0].starts_with(&format!(
            "{RELEASE}models-v3.0-preview3/BirdNET%2B_Geomodel_V3.0.2_Global_12K_"
        )));
        assert!(origins[1].starts_with(&format!("{GEO}BirdNET+_Geomodel_V3.0.2_Global_12K_")));
    }
    for id in ["birdnet-v24", "birdnet-v24-range-v2"] {
        let e = find(id).expect("entry");
        assert_eq!(
            model_origins(e),
            vec![
                format!("{RELEASE}models-v2.4/{id}.onnx").as_str(),
                e.model_url
            ],
            "{id}"
        );
    }
    let perch = find("perch-v2").expect("entry");
    assert_eq!(model_origins(perch), vec![perch.model_url]);
    assert!(
        perch
            .model_url
            .starts_with("https://huggingface.co/justinchuby/Perch-onnx/resolve/")
    );
    // Every project copy is this project's release, under a fixed tag, and
    // comes before the publisher's file.
    for e in CATALOG {
        for m in e.mirror_urls.iter().chain(e.labels_mirror_urls) {
            assert!(m.starts_with(&format!("{RELEASE}models-")), "{}: {m}", e.id);
        }
        assert!(
            !e.model_url.starts_with(RELEASE),
            "{}: the publisher's file is last",
            e.id
        );
        assert!(!ZENODO.is_empty());
    }
}

/// No URL in the catalogue may name a moving ref.
#[test]
fn no_catalogue_url_follows_a_branch() {
    for e in CATALOG {
        for url in model_origins(e).into_iter().chain(labels_origins(e)) {
            if url.contains("huggingface.co") {
                assert!(!url.contains("/resolve/main/"), "{}: {url}", e.id);
            }
            if url.contains("raw.githubusercontent.com") {
                let rev = url.split('/').nth(5).unwrap_or("");
                assert!(
                    rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()),
                    "{}: not pinned to a commit: {url}",
                    e.id
                );
            }
            if url.contains("/releases/download/") {
                let tag = url
                    .split("/releases/download/")
                    .nth(1)
                    .and_then(|r| r.split('/').next());
                assert!(
                    tag.is_some_and(|t| t != "latest"),
                    "{}: a release URL names its tag: {url}",
                    e.id
                );
            }
        }
    }
}

#[test]
fn find_returns_nothing_for_an_id_that_is_not_there() {
    assert!(find("nope").is_none());
}

/// Each classifier with an occurrence filter finds it, and only its own.
#[test]
fn each_classifier_finds_its_own_occurrence_filter() {
    assert_eq!(
        super::occurrence_filter_for("birdnet-v24").map(|e| e.id),
        Some("birdnet-v24-range-v2")
    );
    assert_eq!(
        super::occurrence_filter_for("birdnet-v3").map(|e| e.id),
        Some("birdnet-v3-geomodel")
    );
    assert_eq!(super::occurrence_filter_for("perch-v2"), None);
}

/// Where the installer puts a file and where the station looks for it are the
/// same function, so they cannot drift.
#[test]
fn the_paths_the_installer_writes_are_the_paths_reported() {
    let dir = Path::new("/m");
    let path = |id: &str| installed_paths(find(id).expect("entry"), dir);
    assert_eq!(
        path("birdnet-v24"),
        (
            dir.join("birdnet-v24.onnx"),
            Some(dir.join("birdnet-v24_labels.txt"))
        )
    );
    assert_eq!(
        path("birdnet-v24-range-v2"),
        (dir.join("birdnet-v24-range-v2.onnx"), None)
    );
    assert_eq!(
        path("birdnet-v3"),
        (
            dir.join("birdnet-v3.onnx"),
            Some(dir.join("birdnet-v3_labels.csv"))
        )
    );
    assert_eq!(
        path("birdnet-v3-geomodel"),
        (
            dir.join("birdnet-v3-geomodel.onnx"),
            Some(dir.join("birdnet-v3-geomodel_labels.txt"))
        )
    );
    assert_eq!(
        path("perch-v2").1,
        Some(dir.join("perch-v2_labels.csv")),
        "a CSV is not installed under a .txt name"
    );
}

/// What the installer writes for Perch is the embedded file, and it verifies.
#[test]
fn the_perch_labels_the_installer_writes_verify_against_the_pinned_digest() {
    const WRONG_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    let perch = find("perch-v2").expect("entry");
    let text = perch.labels_embedded.expect("embedded");
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("perch-v2_labels.csv");
    stage::write_verified(text.as_bytes(), perch.labels_sha256.expect("digest"), &path)
        .expect("the embedded bytes have the pinned digest");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), text);
    let other = dir.path().join("other.csv");
    assert!(stage::write_verified(text.as_bytes(), WRONG_DIGEST, &other).is_err());
    assert!(!other.exists());
}

/// Two copies of one file never share a staging name.
#[test]
fn two_copies_of_one_file_stage_under_different_names() {
    let dest = Path::new("/m/birdnet-v24.onnx");
    let (a, b) = (stage::part_path(dest), stage::part_path(dest));
    assert_ne!(a, b);
    for p in [&a, &b] {
        let name = p.file_name().expect("name").to_string_lossy();
        assert!(name.starts_with("birdnet-v24.onnx."), "{name}");
        assert!(name.contains(&std::process::id().to_string()), "{name}");
        assert!(name.ends_with(".part"), "{name}");
    }
}

/// Cleaning up after a dead install leaves a live one, and a resumable
/// download, alone.
#[test]
fn a_dead_installs_staging_file_is_cleared_and_a_live_ones_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("m.onnx");
    let dead = dir.path().join("m.onnx.4000000000.0.part");
    let live = dir
        .path()
        .join(format!("m.onnx.{}.9.part", std::process::id()));
    let legacy = dir.path().join("m.part");
    let other = dir.path().join("other.onnx.4000000000.0.part");
    let resumable = dir.path().join("m.onnx.download");
    for f in [&dead, &live, &legacy, &other, &resumable] {
        std::fs::write(f, b"x").expect("write");
    }
    stage::clear_stale_parts(&dest);
    assert!(!dead.exists(), "a dead process's staging file");
    assert!(!legacy.exists(), "the old fixed name");
    assert!(live.exists(), "this process's may be in flight");
    assert!(other.exists(), "another file's staging is not ours");
    assert!(resumable.exists(), "a download to resume is not rubbish");
}

/// A spec for a local body, as the install path sees one.
fn spec_for(body: &[u8], legacy: Option<&'static str>) -> FileSpec {
    FileSpec {
        name: "installed.onnx".to_owned(),
        sha256: digest_of(body),
        bytes: body.len() as u64,
        origins: Vec::new(),
        legacy,
        embedded: None,
    }
}

/// OOB-20, a local file: installed when it has the pinned size and digest,
/// through a staging name, exactly as a download is.
#[test]
fn a_local_file_with_the_pinned_size_and_digest_is_installed() {
    const BODY: &[u8] = b"the model, carried in on a USB stick";
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("carried.onnx");
    std::fs::write(&src, BODY).expect("write");
    let dest = dir.path().join("installed.onnx");
    let spec = spec_for(BODY, None);
    stage::copy_verified(&src, spec.sha256, spec.bytes, &dest).expect("installs");
    assert_eq!(std::fs::read(&dest).expect("read"), BODY);
    assert_no_part_files(dir.path(), "a local install");
}

/// A local file of the wrong size is refused before it is read, and one of
/// the right size with other bytes is refused on its digest.
///
/// Observed failing with the size check removed from `copy_verified` (a
/// digest refusal where `WrongSize` was asserted), and with the digest
/// comparison made to pass (the impostor installed).
#[test]
fn a_local_file_that_is_not_the_pinned_one_is_refused() {
    const BODY: &[u8] = b"the model, carried in on a USB stick";
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("installed.onnx");
    let spec = spec_for(BODY, None);
    let short = dir.path().join("short.onnx");
    std::fs::write(&short, b"the model").expect("write");
    let err = stage::copy_verified(&short, spec.sha256, spec.bytes, &dest).expect_err("size");
    assert!(matches!(err, InstallError::WrongSize { .. }), "{err:?}");
    let impostor = dir.path().join("impostor.onnx");
    std::fs::write(&impostor, b"THE MODEL, CARRIED IN ON A USB STICK").expect("write");
    let err = stage::copy_verified(&impostor, spec.sha256, spec.bytes, &dest).expect_err("bytes");
    assert!(
        matches!(err, InstallError::NotThePinnedFile { .. }),
        "{err:?}"
    );
    assert!(!dest.exists());
    assert_no_part_files(dir.path(), "a refused local file");
}

/// The public entry point, end to end with no network: V2.4's model from a
/// file is held to the catalogue's real pins.
#[test]
fn install_from_file_holds_a_file_to_the_catalogues_real_pins() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("not-the-model.onnx");
    std::fs::write(&src, b"not 54 MB of BirdNET").expect("write");
    let dest = dir.path().join("models");
    match super::install_from_file("birdnet-v24", &src, &dest) {
        Err(InstallError::WrongSize { expected, actual }) => {
            assert_eq!(expected, 54_068_648);
            assert_eq!(actual, 20);
        }
        other => panic!("expected a refusal on size, got {other:?}"),
    }
    assert!(!dest.join("birdnet-v24.onnx").exists());
    assert!(!dest.join("birdnet-v24_labels.txt").exists());
}

/// MDL-09: a verified file an earlier installer left under its old name is
/// adopted — linked under the catalogue's name, so neither the network nor a
/// second copy's worth of disk is spent — and the old name still works until
/// its configuration has moved.
///
/// Observed failing with the `legacy` branch disabled in `ensure_file`: the
/// install went to the network, which this spec has none of, and failed.
#[test]
fn a_verified_file_under_an_earlier_installers_name_is_adopted_in_place() {
    use std::os::unix::fs::MetadataExt as _;
    const BODY: &[u8] = b"the V3.0 model, under install.sh's old name";
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join("BirdNET+_old_name.onnx");
    std::fs::write(&old, BODY).expect("seed");
    let spec = spec_for(BODY, Some("BirdNET+_old_name.onnx"));
    let mut net = super::download::Network::new(&super::FetchPolicy::default());
    super::ensure_file("x", &spec, dir.path(), None, &mut net, &mut |_| {})
        .expect("adopted without a network");
    let new = dir.path().join("installed.onnx");
    assert_eq!(std::fs::read(&new).expect("read"), BODY);
    assert_eq!(
        std::fs::metadata(&new).expect("new").ino(),
        std::fs::metadata(&old).expect("old").ino(),
        "the same file, not a copy"
    );
    assert_eq!(
        record::check(&new, spec.sha256, spec.bytes, Rehash::Never),
        FileState::Verified,
        "and recorded as verified"
    );
    assert_no_part_files(dir.path(), "an adoption");
}

/// The counterpart: an old-named file that is NOT the pinned one is left as
/// it was and not adopted; the install goes on to its other sources.
#[test]
fn a_damaged_file_under_an_old_name_is_not_adopted() {
    const BODY: &[u8] = b"the V3.0 model, under install.sh's old name";
    let dir = tempfile::tempdir().expect("tempdir");
    let old = dir.path().join("BirdNET+_old_name.onnx");
    let damaged = b"THE V3.0 MODEL, UNDER INSTALL.SH'S OLD NAME";
    std::fs::write(&old, damaged).expect("seed");
    let spec = spec_for(BODY, Some("BirdNET+_old_name.onnx"));
    let mut net = super::download::Network::new(&super::FetchPolicy::default());
    let err = super::ensure_file("x", &spec, dir.path(), None, &mut net, &mut |_| {})
        .expect_err("nothing verified and there is nowhere to download from");
    assert!(matches!(err, InstallError::Network(_)), "{err:?}");
    assert!(!dir.path().join("installed.onnx").exists());
    assert_eq!(std::fs::read(&old).expect("old"), damaged, "left as it was");
    assert_no_part_files(dir.path(), "a refused adoption");
}

/// **MDL-11: installed means the bytes verify.** A file of the pinned size
/// and the wrong bytes is not installed, whatever its name.
///
/// Observed failing on the code before this change: `looks_installed` (size
/// only) answered yes — "a file of zeros of the pinned size counts as
/// installed".
#[test]
fn a_file_of_the_right_size_and_the_wrong_bytes_is_not_installed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let range = find("birdnet-v24-range-v2").expect("entry");
    let (model, _) = installed_paths(range, dir.path());
    assert_eq!(
        installed_state(range, dir.path(), Rehash::IfChanged).model,
        FileState::Missing
    );
    let f = std::fs::File::create(&model).expect("create");
    f.set_len(range.model_bytes - 1).expect("len");
    assert_eq!(
        installed_state(range, dir.path(), Rehash::Never).model,
        FileState::Damaged,
        "the wrong size is damaged without being read"
    );
    f.set_len(range.model_bytes).expect("len");
    let polled = installed_state(range, dir.path(), Rehash::Never);
    assert_eq!(
        polled.model,
        FileState::Unchecked,
        "a poll does not hash 29 MB"
    );
    assert!(polled.is_present() && !polled.is_verified());
    assert!(
        !super::is_installed(range, dir.path()),
        "zeros are not the model"
    );
    let polled = installed_state(range, dir.path(), Rehash::Never);
    assert_eq!(
        polled.model,
        FileState::Damaged,
        "and that answer is recorded, so the next poll knows without hashing"
    );
    assert!(
        !polled.is_present(),
        "a known-damaged file is not offered as installed"
    );
}

/// The verification record: believed while the file is unchanged, not once it
/// has changed, and overruled by a full re-hash — which is what finds damage
/// done in place without the metadata changing.
///
/// Observed failing with the stamp comparison in `record::read` removed (a
/// record believed for a rewritten file): "assertion `left == right` failed",
/// `Verified` where `Unchecked` was asserted.
#[test]
fn the_verification_record_is_believed_only_while_the_file_is_unchanged() {
    const BODY: &[u8] = b"a model, verified";
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("m.onnx");
    std::fs::write(&path, BODY).expect("write");
    let (sha, n) = (digest_of(BODY), BODY.len() as u64);
    assert_eq!(
        record::check(&path, sha, n, Rehash::Never),
        FileState::Unchecked
    );
    assert_eq!(
        record::check(&path, sha, n, Rehash::IfChanged),
        FileState::Verified
    );
    assert!(
        record::record_path(&path).exists(),
        "the answer is recorded"
    );
    assert_eq!(
        record::check(&path, sha, n, Rehash::Never),
        FileState::Verified
    );

    // Rewritten with other bytes of the same length: the record no longer
    // describes it.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, b"a model, VERIFIED").expect("rewrite");
    assert_eq!(
        record::check(&path, sha, n, Rehash::Never),
        FileState::Unchecked
    );
    assert_eq!(
        record::check(&path, sha, n, Rehash::IfChanged),
        FileState::Damaged
    );

    // Damage in place with the modification time put back: only a full
    // re-hash can tell, and it does — and corrects the record.
    std::fs::write(&path, BODY).expect("restore");
    assert_eq!(
        record::check(&path, sha, n, Rehash::IfChanged),
        FileState::Verified
    );
    let mtime = std::fs::metadata(&path)
        .expect("meta")
        .modified()
        .expect("mtime");
    {
        use std::io::{Seek as _, Write as _};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open");
        f.seek(std::io::SeekFrom::Start(3)).expect("seek");
        f.write_all(b"X").expect("damage");
        f.set_modified(mtime).expect("put the time back");
    }
    assert_eq!(
        record::check(&path, sha, n, Rehash::IfChanged),
        FileState::Verified,
        "the record cannot see this — which is why the start re-hashes"
    );
    assert_eq!(
        record::check(&path, sha, n, Rehash::Always),
        FileState::Damaged
    );
    assert_eq!(
        record::check(&path, sha, n, Rehash::Never),
        FileState::Damaged
    );
}

/// An entry with labels is installed only when both files verify.
#[test]
fn an_entry_is_installed_only_with_its_labels() {
    let dir = tempfile::tempdir().expect("tempdir");
    let v24 = find("birdnet-v24").expect("entry");
    let (model, labels) = installed_paths(v24, dir.path());
    let labels = labels.expect("labels");
    std::fs::File::create(&model)
        .and_then(|f| f.set_len(v24.model_bytes))
        .expect("model");
    let state = installed_state(v24, dir.path(), Rehash::Never);
    assert_eq!(state.labels, Some(FileState::Missing));
    assert!(!state.is_verified());
    std::fs::write(&labels, v24.labels_embedded.expect("text")).expect("labels");
    let state = installed_state(v24, dir.path(), Rehash::IfChanged);
    assert_eq!(state.labels, Some(FileState::Verified));
    assert_eq!(state.model, FileState::Damaged, "54 MB of zeros");
    assert!(state.is_damaged() && !state.is_verified());
}

/// RES-32: the taxonomy table names only label files the catalogue ships.
#[test]
fn every_taxonomy_row_is_a_label_file_the_catalogue_ships() {
    for row in super::TAXONOMY_EDITIONS {
        assert!(
            CATALOG
                .iter()
                .any(|e| e.labels_sha256 == Some(row.labels_sha256)),
            "{} is no catalogue entry's labels",
            row.labels_sha256
        );
        assert!(!row.cited_from.is_empty() && !row.edition.is_empty());
    }
    let v24 = find("birdnet-v24")
        .and_then(|e| e.labels_sha256)
        .expect("labels");
    assert!(super::taxonomy_edition(v24).is_none());
    let perch = find("perch-v2")
        .and_then(|e| e.labels_sha256)
        .expect("labels");
    assert!(super::taxonomy_edition(perch).is_some_and(|t| t.edition.contains("iNaturalist 2024")));
}

/// MMP-20: every entry carries where it comes from and where its licence was
/// read.
#[test]
fn every_entry_says_where_it_comes_from_and_where_its_licence_was_read() {
    for e in CATALOG {
        assert!(!e.source.trim().is_empty(), "{}: no source", e.id);
        assert!(!e.licence_cited_from.trim().is_empty(), "{}", e.id);
    }
}

/// MDL-08: the geomodel is an entry the binary can install, under the names
/// the earlier installers gave it adopted, with both origins for both files.
///
/// Observed failing on the catalogue before this change: "no geomodel entry".
#[test]
fn the_geomodel_is_in_the_catalogue_with_the_installers_pins() {
    let geo = find(super::V3_GEOMODEL_ID).expect("no geomodel entry");
    assert_eq!(
        geo.legacy_model_name,
        Some("BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx")
    );
    assert_eq!(
        geo.legacy_labels_name,
        Some("BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt")
    );
    assert_eq!(labels_origins(geo).len(), 2);
    assert!(geo.ships_labels());
    let v3 = find(super::V3_CLASSIFIER_ID).expect("v3");
    assert_eq!(
        v3.legacy_model_name,
        Some("BirdNET+_V3.0-preview3_Global_11K_FP32.onnx")
    );
    assert_eq!(
        v3.legacy_labels_name,
        Some("BirdNET+_V3.0-preview3_Global_11K_Labels.csv")
    );
}
