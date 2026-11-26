//! Installing a model, without ever installing the wrong one (`G-10` Stage 5,
//! and the one install mechanism every installer calls: `MDL-09`).
//!
//! Stage 2 let a station run more than one classifier. Getting a second one
//! onto the station was still "download 400 MB from somewhere and put it in
//! the right directory", which on an unattended box at the end of a domestic
//! uplink is several ways to end up with a station that is quietly broken.
//! Every model a station runs — the classifiers, the V2.4 range filter and the
//! V3.0 geomodel — is installed here, by `birdnet-behavior --install-model`,
//! by the dashboard, by `install.sh` and by the container's entrypoint alike.
//!
//! # What this refuses to do
//!
//! **Install anything it cannot verify.** Every catalogue entry carries a
//! sha256 and a size measured from the actual file. A download that does not
//! match is deleted, not installed, and not reported as a warning to be ignored
//! later. This is the same posture [`crate::auto_update`] takes toward the
//! binary, and for the same reason: "we could not check this" and "this is
//! fine" are different sentences.
//!
//! **Download onto a disk that cannot hold it.** A 400 MB model onto a card
//! with 300 MB free does not fail cleanly — it fills the card, and a station
//! whose disk is full stops recording birds, which is a far worse outcome than
//! not having a second classifier. Free space is checked first, with room to
//! spare.
//!
//! **Leave a half-written file where the loader will find it.** A download
//! lands on `<name>.download`, a name the station never looks at, and is
//! renamed into place only after the whole file — including any part an
//! earlier attempt fetched — hashes to the pin. It is stopped at the file's
//! pinned size, so an origin that never stops cannot fill the disk.
//!
//! **Wait for ever on a bad line.** Each origin has a connect timeout and a
//! stall timeout (no bytes for that long is a failure), a dropped transfer is
//! resumed with an HTTP `Range` request where the origin allows it, and the
//! origins are tried again a bounded number of times with waits between the
//! rounds ([`FetchPolicy`]).
//!
//! # What "installed" means
//!
//! The bytes verify. [`installed_state`] answers it for every caller, and a
//! verification record beside each file (its size, modification time, inode
//! and the sha256 it hashed to) lets a status poll answer it without hashing
//! 541 MB, for as long as the file has not changed since. `--install-model`
//! and the station's start re-hash in full ([`Rehash::Always`]).
//!
//! # Where this differs from the binary updater, and why
//!
//! [`crate::auto_update::apply_update`] verifies the asset **before** writing
//! anything to disk — it reads the whole thing into memory first. That is
//! right for a 20 MB binary and wrong here: the models this installs are 409
//! and 541 MB, and buffering one on a board with a gigabyte of RAM is the
//! out-of-memory kill that `G-33`'s whole memory policy exists to avoid.
//!
//! So the bytes stream to disk and the digest is computed as they go. The
//! safety property is preserved by a different means: what lands on disk
//! unverified has a name the station cannot load, and only a verified file is
//! ever given the real one.
//!
//! # Why the catalogue is compiled in rather than fetched
//!
//! Upstream fetches a catalogue from Hugging Face with a configurable endpoint.
//! A compiled-in list is the safer shape for a station nobody is watching: a
//! fetched catalogue is a second network dependency in the path of installing
//! a model, and — more to the point — it is a remote document that decides
//! which URL the station downloads hundreds of megabytes from and which digest
//! it checks them against. Pinning both here means the checksum is a promise
//! this repository makes, verifiable by anyone reading the file, rather than
//! whatever the endpoint said today.
//!
//! The cost is that a new model needs a release. That is the right cost.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

mod download;
mod policy;
mod record;
mod stage;

pub use policy::{
    CONNECT_TIMEOUT_ENV, FetchPolicy, MIRROR_ENV, POLICY_ENV_KEYS, REPAIR_INTERVAL_ENV,
    RETRY_WAITS_ENV, STALL_TIMEOUT_ENV,
};
pub use record::{FileState, InstallState, Rehash};

/// The catalogue id of the classifier a new station runs: BirdNET V2.4, the
/// network BirdNET-Pi and birdnet-go run.
pub const DEFAULT_CLASSIFIER_ID: &str = "birdnet-v24";

/// The catalogue id of the species-occurrence ("range") filter that pairs with
/// [`DEFAULT_CLASSIFIER_ID`].
pub const DEFAULT_RANGE_FILTER_ID: &str = "birdnet-v24-range-v2";

/// The catalogue id of the BirdNET+ V3.0 classifier.
pub const V3_CLASSIFIER_ID: &str = "birdnet-v3";

/// The catalogue id of the occurrence filter that pairs with
/// [`V3_CLASSIFIER_ID`]: the BirdNET geomodel.
pub const V3_GEOMODEL_ID: &str = "birdnet-v3-geomodel";

/// Free space required beyond the model's own size, in bytes.
///
/// A download that exactly fits leaves a station with nothing in hand, and the
/// same card carries the database, the recordings and the write-ahead log. 512
/// MiB is roughly one more model's worth of headroom — deliberately generous,
/// because the failure this prevents (a full card stops the station recording)
/// is much worse than the one it causes (an install refused on a tight disk).
const FREE_SPACE_MARGIN: u64 = 512 * 1024 * 1024;

/// What an installed file is for, which decides what the operator is told to
/// configure it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use {
    /// A classifier that can run beside the primary (`MODEL_2_PATH`, ...). At
    /// another sample rate than the primary's, each recording is resampled
    /// once more for it.
    Classifier,
    /// A classifier that replaces the primary (`MODEL_PATH`, `LABELS_PATH`):
    /// another BirdNET, which would mostly repeat the one it replaces.
    PrimaryReplacement,
    /// A species-occurrence ("range") filter, for `METADATA_MODEL_PATH` (and
    /// `METADATA_LABELS_PATH` when it ships labels of its own). It belongs to
    /// the classifier [`CatalogEntry::pairs_with`] names and to no other.
    RangeFilter,
}

impl Use {
    /// The stable name the API reports.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classifier => "classifier",
            Self::PrimaryReplacement => "primary_classifier",
            Self::RangeFilter => "range_filter",
        }
    }
}

/// One model file a station can install, with its labels when it has any.
///
/// Every file is fetched from an ordered list of origins: this project's own
/// copies first (`mirror_urls`, `labels_mirror_urls`, in order), then the file
/// as its publisher put it out (`model_url`, `labels_url`). A file with no copy
/// of this project's yet (Perch) has an empty mirror list, and the day one is
/// published its URL goes first in that list and nothing else changes. Every
/// origin is held to the same sha256 and size, so an origin that served
/// anything else is refused exactly as a bad upstream is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    /// Stable id, used as the installed filename and as a routing target.
    pub id: &'static str,
    /// What to call it to an operator.
    pub name: &'static str,
    /// The file as its publisher distributes it, at a fixed revision: the last
    /// origin tried.
    pub model_url: &'static str,
    /// This project's copies of that file, tried before [`Self::model_url`],
    /// in this order (OOB-20, MDL-12). Each was seen to carry the pinned digest
    /// and size before it was listed.
    pub mirror_urls: &'static [&'static str],
    /// Who made the model and where it is published (MMP-20).
    pub source: &'static str,
    /// The licence the model file is published under, by its short name, or
    /// `None` when no source this repository can cite records one (MMP-20).
    /// Never a guess: a station that sells what it records needs the real one.
    pub licence: Option<&'static str>,
    /// Where [`Self::licence`] was read, so anyone can check it again.
    pub licence_cited_from: &'static str,
    /// Lowercase hex sha256 of that file, measured from the file itself.
    pub model_sha256: &'static str,
    /// Its size in bytes, for the free-space check before downloading and the
    /// cap on what an origin may send.
    pub model_bytes: u64,
    /// The name an earlier installer gave this file in the same directory
    /// (`install.sh` and the container named the V3.0 files after their
    /// upstream names). A file under it that verifies is adopted in place —
    /// linked under the catalogue's name, not downloaded again (MDL-09).
    pub legacy_model_name: Option<&'static str>,
    /// Where the labels file is published, when it is downloaded: the last
    /// labels origin tried. Never set together with [`Self::labels_embedded`].
    pub labels_url: Option<&'static str>,
    /// This project's copies of the labels file, tried before
    /// [`Self::labels_url`], in this order.
    pub labels_mirror_urls: &'static [&'static str],
    /// The labels file itself, when it is compiled into the binary instead of
    /// downloaded. Written to disk by [`install`] and verified against
    /// [`Self::labels_sha256`] like a download, so a binary that carries the
    /// wrong bytes refuses to install them rather than naming the wrong birds.
    pub labels_embedded: Option<&'static str>,
    /// Lowercase hex sha256 of the labels file.
    pub labels_sha256: Option<&'static str>,
    /// The size of a downloaded labels file in bytes (0 when the labels are
    /// embedded, whose size is the text's, or when there are none).
    pub labels_bytes: u64,
    /// The installed labels file's extension (`csv`, `txt`), or `""` when the
    /// entry ships no labels.
    pub labels_ext: &'static str,
    /// The name an earlier installer gave the labels file, adopted like
    /// [`Self::legacy_model_name`].
    pub legacy_labels_name: Option<&'static str>,
    /// Other labels files a station may already hold under the installed name
    /// and keeps: each a digest and a size. Such a file names the same classes
    /// in the same order, so the model's scores land on the right species, and
    /// differs only in the common names it gives them; replacing it would put
    /// one species under two names in that station's history. Never written by
    /// an install, only left in place.
    pub labels_also_accepted: &'static [(&'static str, u64)],
    /// The sample rate it needs — what `MODEL_n_SAMPLE_RATE` should be set to.
    ///
    /// Carried here because it is not derivable from the tensor: Perch v2's
    /// `[-1, 160_000]` is 5 s at 32 kHz and equally 3⅓ s at 48 kHz. See
    /// `G-10` Stage 4. 0 for an occurrence filter, which hears no audio.
    pub sample_rate: u32,
    /// What it is for.
    pub usage: Use,
    /// For an occurrence filter, the classifier it belongs to. A filter is
    /// indexed or named for one network and means nothing to another.
    pub pairs_with: Option<&'static str>,
    /// Anything an operator needs to know before installing it.
    pub notes: &'static str,
}

impl CatalogEntry {
    /// Whether installing this entry also installs a labels file.
    #[must_use]
    pub const fn ships_labels(&self) -> bool {
        self.labels_url.is_some() || self.labels_embedded.is_some()
    }
}

/// The models this build knows how to install.
///
/// Every digest and size here was measured from the file, not copied from a
/// model card. A gate asserts each digest is a well-formed 64-character
/// lowercase hex digest, because a malformed one would make the entry
/// uninstallable in a way nothing else would notice until somebody tried it in
/// the field. `installer/catalogue-field.sh` reads this table from the shell
/// (the release workflow and the installer's pin test), so each field stays on
/// its own line, named as it is here.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "birdnet-v3",
        name: "BirdNET+ V3.0 preview3 (global, 11K species)",
        model_url: "https://zenodo.org/api/records/18247420/files/\
                    BirdNET+_V3.0-preview3_Global_11K_FP32.onnx/content",
        mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
                        models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_FP32.onnx",
        ],
        source: "BirdNET+ V3.0 developer preview 3 (Lasseck, Eibl, Klinck, Kahl; K. Lisa Yang \
                 Center for Conservation Bioacoustics), published on Zenodo (record 18247420); \
                 mirrored in this project's models-v3.0-preview3 release",
        licence: Some("CC BY-SA 4.0"),
        licence_cited_from: "the models-v3.0-preview3 release notes written by \
                             .github/workflows/publish-model.yml (\"licensed CC-BY-SA-4.0\", \
                             pointing to the upstream record's TERMS_OF_USE.txt), and \
                             docs/book/reference/taxonomy-overlap.md for its labels",
        model_sha256: "2a0f9efba1a98e3193ad3dfcb8323116a7de88e39545f3619a7ea46e3bb7d743",
        model_bytes: 541_391_777,
        legacy_model_name: Some("BirdNET+_V3.0-preview3_Global_11K_FP32.onnx"),
        labels_url: Some(
            "https://zenodo.org/api/records/18247420/files/\
             BirdNET+_V3.0-preview3_Global_11K_Labels.csv/content",
        ),
        labels_mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
                               models-v3.0-preview3/BirdNET%2B_V3.0-preview3_Global_11K_Labels.csv",
        ],
        labels_embedded: None,
        labels_sha256: Some("8124b0ea2d187104c5e2cd95a0f937165647e20349c8fd34d4d5ef991821f8f0"),
        labels_bytes: 809_172,
        labels_ext: "csv",
        legacy_labels_name: Some("BirdNET+_V3.0-preview3_Global_11K_Labels.csv"),
        labels_also_accepted: &[],
        sample_rate: 32_000,
        usage: Use::Classifier,
        pairs_with: None,
        notes: "Preview release. Reports a dynamic input shape, which resolves \
                to 4.5 s windows at 32 kHz. Pair it with birdnet-v3-geomodel.",
    },
    CatalogEntry {
        id: "birdnet-v3-geomodel",
        name: "BirdNET+ Geomodel V3.0.2 species-occurrence filter (global, 12K species)",
        model_url: "https://github.com/birdnet-team/geomodel/releases/download/v3.0.2/\
                    BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx",
        mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
                        models-v3.0-preview3/BirdNET%2B_Geomodel_V3.0.2_Global_12K_FP32.onnx",
        ],
        source: "The BirdNET geomodel V3.0.2 (Stefan Kahl, K. Lisa Yang Center for Conservation \
                 Bioacoustics), published in the v3.0.2 release of \
                 github.com/birdnet-team/geomodel; mirrored in this project's \
                 models-v3.0-preview3 release",
        licence: Some("CC BY-SA 4.0, with prohibited uses (poaching; any military purpose)"),
        licence_cited_from: "MODEL_LICENSE.txt in the v3.0.2 release of \
                             github.com/birdnet-team/geomodel, read 2026-11-25 (\"licensed under \
                             Creative Commons Attribution-ShareAlike 4.0 International\", and its \
                             PROHIBITED USES section)",
        model_sha256: "b151f680a47de5371f39b3df129aea5946ac6baa039582274f833b42eaf992ea",
        model_bytes: 14_700_313,
        legacy_model_name: Some("BirdNET+_Geomodel_V3.0.2_Global_12K_FP32.onnx"),
        labels_url: Some(
            "https://github.com/birdnet-team/geomodel/releases/download/v3.0.2/\
             BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt",
        ),
        labels_mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
                               models-v3.0-preview3/BirdNET%2B_Geomodel_V3.0.2_Global_12K_Labels.txt",
        ],
        labels_embedded: None,
        labels_sha256: Some("c15818db07e55978d909a9bcd916cd0615b0183f789227d9516059151787c784"),
        labels_bytes: 571_793,
        labels_ext: "txt",
        legacy_labels_name: Some("BirdNET+_Geomodel_V3.0.2_Global_12K_Labels.txt"),
        labels_also_accepted: &[],
        sample_rate: 0,
        usage: Use::RangeFilter,
        pairs_with: Some("birdnet-v3"),
        notes: "Takes latitude, longitude and the week and returns, for each of 12 012 \
                species, how likely it is to occur there. It scores a different list \
                from the V3.0 classifier's, so its own labels file is what maps one \
                onto the other: set METADATA_LABELS_PATH to it as well. FP32 rather \
                than FP16: upstream's recommended variant, and twice as fast to load. \
                Do not pair it with BirdNET V2.4; that has its own range filter.",
    },
    CatalogEntry {
        id: "perch-v2",
        name: "Google Perch v2 (14 795 classes)",
        model_url: "https://huggingface.co/justinchuby/Perch-onnx/resolve/\
                    da88bf8c37c0b3068fc706fe509f633b9fbd28c1/perch_v2.onnx",
        // No copy of this project's yet: the ONNX conversion's only origin is
        // its Hugging Face repository. A `models-perch-v2` release would go
        // first here.
        mirror_urls: &[],
        source: "Google's Perch v2 (bird-vocalization-classifier v2, \
                 kaggle.com/models/google/bird-vocalization-classifier), as the ONNX \
                 conversion published at huggingface.co/justinchuby/Perch-onnx",
        licence: Some("Apache-2.0"),
        licence_cited_from: "the model card of huggingface.co/justinchuby/Perch-onnx at the \
                             pinned revision (license: apache-2.0), read 2026-11-25; the \
                             labels' source, cgeorgiaw/Perch, declares the same \
                             (crates/birdnet-core/data/labels/README.md)",
        model_sha256: "bf0c8467a924cb074663970ca4a0ab1e143602121930209657d0dff5d5cefa1f",
        model_bytes: 409_148_616,
        legacy_model_name: None,
        // No labels file accompanies the ONNX conversion. Google's list of the
        // 14 795 classes is compiled into the binary in the form the station
        // reads (`scripts/build-perch-labels.py`, `data/labels/README.md`), so
        // nobody has to find it and reshape it by hand: the classes are
        // positional, and a hand-made file that is wrong is not an error but a
        // station naming birds as other birds.
        labels_url: None,
        labels_mirror_urls: &[],
        labels_embedded: Some(birdnet_core::inference::labels::PERCH_V2_LABELS),
        labels_sha256: Some(birdnet_core::inference::labels::PERCH_V2_LABELS_SHA256),
        labels_bytes: 0,
        labels_ext: "csv",
        legacy_labels_name: None,
        labels_also_accepted: &[],
        sample_rate: 32_000,
        usage: Use::Classifier,
        pairs_with: None,
        notes: "Materially better than BirdNET in the tropics. Wants 5 s windows \
                at 32 kHz where BirdNET wants 4.5 s; the two run together, the chunk \
                cut to the longer window and stepped by the shorter. Beside the \
                48 kHz BirdNET V2.4 each recording is also resampled to 32 kHz for \
                it, which costs one more resample per recording. Set \
                MODEL_n_SAMPLE_RATE=32000 — its tensor shape alone cannot say. Its \
                14 795 classes are not all species: 200 name a kind \
                of sound (Engine, Vehicle, Animal). Its scores are shares of one \
                softmax over those classes, so they sum to 1 in a window and two \
                species singing at once split them; judge them against a bar of \
                their own (MODEL_n_THRESHOLD), not the station's.",
    },
    CatalogEntry {
        id: "birdnet-v24",
        name: "BirdNET V2.4 (global, 6 522 species)",
        // birdnet-go's published ONNX build of the network BirdNET-Pi and
        // birdnet-go run, at a fixed revision of the repository rather than
        // `main`, so the bytes behind the URL cannot change under the digest.
        model_url: "https://huggingface.co/tphakala/BirdNET-v2.4/resolve/\
                    acd49768d227c726ac94550c94f0effb3e1dd04c/BirdNET_v2.4_fp32_dfttrunc.onnx",
        mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
             models-v2.4/birdnet-v24.onnx",
        ],
        source: "BirdNET V2.4 (K. Lisa Yang Center for Conservation Bioacoustics, Cornell Lab of \
                 Ornithology, and Chemnitz University of Technology), as birdnet-go's FP32 ONNX \
                 conversion at huggingface.co/tphakala/BirdNET-v2.4; mirrored in this \
                 project's models-v2.4 release",
        licence: Some("CC BY-NC-SA 4.0"),
        licence_cited_from: "the model card of huggingface.co/tphakala/BirdNET-v2.4 at the \
                             pinned revision (license: cc-by-nc-sa-4.0), read 2026-11-25; \
                             also the models-v2.4 release notes",
        model_sha256: "3b72e88b3ad0c310a41adabccf8cf75b1a05daeeb40884ebd38038c91d0e423d",
        model_bytes: 54_068_648,
        legacy_model_name: None,
        // Not downloaded: the birdnet-go file this model is paired with names 460
        // of its 6 522 classes differently from BirdNET-Pi ("Common Magpie"
        // against "Eurasian Magpie"), which would split a species in two the
        // moment a BirdNET-Pi history is imported. The embedded file is that
        // file's class order with BirdNET-Pi's English names
        // (`scripts/build-v24-labels.py`, and a gate that holds the order to
        // birdnet-go's file line for line).
        labels_url: None,
        labels_mirror_urls: &[],
        labels_embedded: Some(birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS),
        labels_sha256: Some(birdnet_core::inference::labels::BIRDNET_V24_ENGLISH_LABELS_SHA256),
        labels_bytes: 0,
        labels_ext: "txt",
        legacy_labels_name: None,
        // birdnet-go's own `labels.txt` at the pinned revision, which installs
        // before 0.18.0 downloaded: the same 6 522 classes in the same order
        // (`data/labels/README.md`), 460 of them under another English name.
        labels_also_accepted: &[(
            "487937b6ad132b8506215523209a87b86adc9dce8e5ed3048ce9268189dddd3d",
            259_894,
        )],
        sample_rate: 48_000,
        usage: Use::PrimaryReplacement,
        pairs_with: None,
        notes: "The classifier BirdNET-Pi and birdnet-go run. Its scores are \
                BirdNET-Pi's to within a few thousandths (it is the FP32 build; \
                BirdNET-Pi runs FP16 weights) once CONFIDENCE and SENSITIVITY are \
                set as BirdNET-Pi sets them, and its labels carry BirdNET-Pi's \
                English names. 48 kHz, 3 s windows, no calibration: its scores \
                are raw logits put through SENSITIVITY. Replaces the \
                BirdNET+ V3.0 classifier as the station's primary. Pair it with \
                birdnet-v24-range-v2.",
    },
    CatalogEntry {
        id: "birdnet-v24-range-v2",
        name: "BirdNET V2.4 species-occurrence (range) filter, v2",
        // The same file birdnet-go ships for the V2.4 filter ("latest"),
        // converted from the MData V2 FP16 model BirdNET-Pi also ships. Fixed
        // to a commit of that repository for the same reason as above.
        model_url: "https://raw.githubusercontent.com/tphakala/birdnet-go/\
                    f4f019a3d79cb547b432bd8729fb4cf9c71aa2e7/internal/classifier/data/\
                    BirdNET_MData_V2.onnx",
        mirror_urls: &[
            "https://github.com/tomtom215/BirdNet-Behavior/releases/download/\
             models-v2.4/birdnet-v24-range-v2.onnx",
        ],
        source: "BirdNET's species-occurrence (metadata) model V2, by the authors of BirdNET \
                 V2.4, as the ONNX file birdnet-go ships (github.com/tphakala/birdnet-go); \
                 mirrored in this project's models-v2.4 release",
        licence: Some("CC BY-NC-SA 4.0"),
        licence_cited_from: "the models-v2.4 release notes written by \
                             .github/workflows/publish-model.yml (\"BirdNET V2.4 and its \
                             metadata (range) model ... CC BY-NC-SA 4.0\"); no upstream page \
                             for this file was read",
        model_sha256: "b1810073d16c6c39df95a7419f9a11a02f98dbea562c96d4993c6af52b54b271",
        model_bytes: 29_524_170,
        legacy_model_name: None,
        labels_url: None,
        labels_mirror_urls: &[],
        labels_embedded: None,
        labels_sha256: None,
        labels_bytes: 0,
        labels_ext: "",
        legacy_labels_name: None,
        labels_also_accepted: &[],
        sample_rate: 0,
        usage: Use::RangeFilter,
        pairs_with: Some("birdnet-v24"),
        notes: "Takes latitude, longitude and the week and returns, for each V2.4 \
                species, how likely it is to occur there. Indexed like the V2.4 \
                labels, so it needs no labels of its own. Not the filter BirdNET-Pi \
                uses by default (its DATA_MODEL_VERSION=1 is the older v1, which \
                this catalogue does not carry): v2 lets about a quarter more species \
                through at the same SF_THRESH. Do not pair it with the V3.0 \
                classifier; that has its own geomodel.",
    },
];

/// Look an entry up by id.
#[must_use]
pub fn find(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// The occurrence filter that belongs to the classifier `classifier_id`, when
/// the catalogue carries one.
#[must_use]
pub fn occurrence_filter_for(classifier_id: &str) -> Option<&'static CatalogEntry> {
    CATALOG
        .iter()
        .find(|e| e.usage == Use::RangeFilter && e.pairs_with == Some(classifier_id))
}

/// Which taxonomy a labels file follows, where that can be cited (RES-32).
///
/// A row's scientific names are free text, and `labels_sha256` in an analysis
/// run says which label file named them, not which checklist those names come
/// from. This table is that second step, keyed by the label file's digest. A
/// label file not in it has no recorded edition, and says so: an edition is
/// written here only where a source can be cited, never inferred from what the
/// names look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaxonomyEdition {
    /// Lowercase hex sha256 of the labels file, as [`CatalogEntry::labels_sha256`].
    pub labels_sha256: &'static str,
    /// The taxonomy and its edition.
    pub edition: &'static str,
    /// Where that was read.
    pub cited_from: &'static str,
}

/// The label files whose taxonomy edition can be cited.
///
/// Every other label file's is not recorded: BirdNET V2.4's (neither its model
/// card nor the BirdNET-Analyzer README, read 2026-11-25, names one), BirdNET+
/// V3.0's and the geomodel's (nothing in this repository names one).
pub const TAXONOMY_EDITIONS: &[TaxonomyEdition] = &[TaxonomyEdition {
    labels_sha256: birdnet_core::inference::labels::PERCH_V2_LABELS_SHA256,
    edition: "iNaturalist 2024 taxa (Google's label set inat2024_fsd50k), plus 200 FSD50K \
              sound classes; English names from BirdNET V2.4's, then IOC World Bird List 15.2",
    cited_from: "crates/birdnet-core/data/labels/README.md (the label set's first line, and \
                 \"14,595 classes are Latin binomials (iNaturalist 2024 taxa ...)\")",
}];

/// The taxonomy edition a labels file follows, by its sha256, when it can be
/// cited; `None` when it is not recorded.
#[must_use]
pub fn taxonomy_edition(labels_sha256: &str) -> Option<&'static TaxonomyEdition> {
    TAXONOMY_EDITIONS
        .iter()
        .find(|t| t.labels_sha256.eq_ignore_ascii_case(labels_sha256))
}

/// Where an entry's model file is fetched from, in the order they are tried:
/// this project's copies first, then the publisher's file.
#[must_use]
pub fn model_origins(entry: &CatalogEntry) -> Vec<&'static str> {
    entry
        .mirror_urls
        .iter()
        .copied()
        .chain(std::iter::once(entry.model_url))
        .collect()
}

/// Where an entry's labels file is fetched from, in the order they are tried;
/// empty when its labels are embedded or it has none.
#[must_use]
pub fn labels_origins(entry: &CatalogEntry) -> Vec<&'static str> {
    entry.labels_url.map_or_else(Vec::new, |url| {
        entry
            .labels_mirror_urls
            .iter()
            .copied()
            .chain(std::iter::once(url))
            .collect()
    })
}

/// What can go wrong installing a model.
#[derive(Debug)]
pub enum InstallError {
    /// No catalogue entry by that id.
    Unknown {
        /// What was asked for.
        id: String,
        /// What is available, so the message is actionable.
        known: Vec<&'static str>,
    },
    /// The digest in the catalogue is not a usable sha256.
    Unverifiable {
        /// Which entry.
        id: &'static str,
        /// Why it cannot be checked.
        why: String,
    },
    /// Not enough free space to hold the download plus a margin.
    NotEnoughSpace {
        /// Bytes needed, including the margin.
        needed: u64,
        /// Bytes free.
        available: u64,
    },
    /// The download failed or the server answered with an error.
    Network(String),
    /// The downloaded bytes do not match the catalogue's digest.
    Integrity {
        /// What the catalogue says.
        expected: &'static str,
        /// What arrived.
        actual: String,
    },
    /// The server sent more bytes than the file is pinned to. The download was
    /// stopped and deleted.
    TooLarge {
        /// The most the file may be.
        limit: u64,
    },
    /// A local model file is not the size the catalogue pins, so it is not
    /// the file (checked before it is read).
    WrongSize {
        /// What the catalogue says.
        expected: u64,
        /// What the file is.
        actual: u64,
    },
    /// A local model file of the right size whose sha256 is not the pinned
    /// one. Nothing was installed.
    NotThePinnedFile {
        /// What the catalogue says.
        expected: &'static str,
        /// What the file has.
        actual: String,
    },
    /// Another install of the same file is downloading it right now.
    Busy {
        /// The file being installed.
        file: String,
    },
    /// Every origin failed; each with the last reason it gave, in the order
    /// tried.
    AllOrigins(Vec<(String, Self)>),
    /// A filesystem operation failed.
    Io(std::io::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { id, known } => write!(
                f,
                "no model called `{id}` is in the catalogue (available: {})",
                known.join(", ")
            ),
            Self::Unverifiable { id, why } => write!(
                f,
                "model `{id}` cannot be verified: {why}. Refusing to install it — \
                 a model that cannot be checked against anything is a model that could \
                 be anything"
            ),
            Self::NotEnoughSpace { needed, available } => write!(
                f,
                "not enough free space: {} MiB needed (including a {} MiB margin), \
                 {} MiB available. Refusing to download — filling the card would stop \
                 the station recording, which is worse than not having this model",
                needed / (1024 * 1024),
                FREE_SPACE_MARGIN / (1024 * 1024),
                available / (1024 * 1024)
            ),
            Self::Network(why) => write!(f, "download failed: {why}"),
            Self::Integrity { expected, actual } => write!(
                f,
                "sha256 mismatch: expected {expected}, got {actual}. The download has \
                 been deleted. Either it was corrupted in transit or the file at that \
                 URL is not the one this build was built against"
            ),
            Self::TooLarge { limit } => write!(
                f,
                "the server sent more than the {limit} bytes this file is pinned to, so \
                 the download was stopped and deleted"
            ),
            Self::WrongSize { expected, actual } => write!(
                f,
                "the file is {actual} bytes and this model is {expected}, so it is not the \
                 model this build pins. Nothing was installed"
            ),
            Self::NotThePinnedFile { expected, actual } => write!(
                f,
                "sha256 mismatch: expected {expected}, got {actual}. The file is not the \
                 model this build pins, so it was refused and nothing was installed"
            ),
            Self::Busy { file } => write!(
                f,
                "another install of {file} is downloading it now; wait for it to finish \
                 and run this again"
            ),
            Self::AllOrigins(tried) => {
                write!(f, "no origin served the verified file:")?;
                for (url, e) in tried {
                    write!(f, " {url}: {e};")?;
                }
                Ok(())
            }
            Self::Io(e) => write!(f, "filesystem error: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<std::io::Error> for InstallError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Where an installed model ended up, and what to configure it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The catalogue id.
    pub id: &'static str,
    /// The installed ONNX file.
    pub model_path: PathBuf,
    /// The installed labels file, when the catalogue shipped one.
    pub labels_path: Option<PathBuf>,
    /// The sample rate this classifier needs (0 for an occurrence filter).
    pub sample_rate: u32,
    /// What it is for.
    pub usage: Use,
}

/// Check a catalogue digest is a usable sha256.
///
/// A malformed digest must be refused **before** anything is downloaded: an
/// entry that can never verify would otherwise pull hundreds of megabytes over
/// somebody's uplink and then throw them away.
fn require_digest(id: &'static str, hex: &str) -> Result<(), InstallError> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(InstallError::Unverifiable {
            id,
            why: format!("`{hex}` is not a 64-character hex sha256"),
        });
    }
    Ok(())
}

/// Whether a download of `model_bytes` fits, given what the filesystem says
/// is free.
///
/// A pure function so the policy can be tested without arranging a full disk.
/// The first version of this was inline in [`install`], and both gates written
/// for it passed with the check deleted: one accepted an unrelated I/O error
/// from `/proc` as success, and the other only ever exercised
/// [`free_space_bytes`]. Separating the decision from the measurement is what
/// makes it checkable.
///
/// `available: None` is **not** enough room. A filesystem that will not say
/// how much space it has cannot be shown to have any, and the reading that
/// treats unknown as plenty is the one that fills a station's card.
///
/// # Errors
///
/// [`InstallError::NotEnoughSpace`], carrying both numbers so the message can
/// show its working.
pub fn fits(model_bytes: u64, available: Option<u64>) -> Result<(), InstallError> {
    let needed = model_bytes.saturating_add(FREE_SPACE_MARGIN);
    let available = available.unwrap_or(0);
    if available < needed {
        return Err(InstallError::NotEnoughSpace { needed, available });
    }
    Ok(())
}

/// Free bytes on the filesystem holding `dir`.
///
/// `None` when it cannot be read, which callers must treat as "cannot show it
/// fits" rather than "plenty" — the same reading `G-10` Stage 2 takes of an
/// unknown memory ceiling, for the same reason.
#[must_use]
pub fn free_space_bytes(dir: &Path) -> Option<u64> {
    // Delegated rather than re-spawned: the core helper answers exactly this
    // behind `run_with_timeout`, so a dead network mount cannot hang the
    // install, and it passes `--` so a directory named `-x` is not a flag.
    birdnet_core::audio::capture::disk_usage(dir)
        .ok()
        .map(|usage| usage.available_bytes)
}

/// How far along a download is, as [`install_with_progress`] reports it.
///
/// A browser page that offers to install a classifier needs something to show
/// for a minute or two of silence; this is that something. `done` counts the
/// bytes of the file that are on the disk, including any an earlier attempt
/// fetched and this one resumed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The file being fetched, by name (`birdnet-v24.onnx`).
    pub file: String,
    /// Bytes of it on the disk so far.
    pub done: u64,
    /// Bytes expected in all (the pinned size).
    pub total: u64,
}

/// One file an install puts in place: its name there, its pins, and where its
/// bytes come from.
#[derive(Debug, Clone)]
struct FileSpec {
    /// The installed file name (`birdnet-v3.onnx`).
    name: String,
    /// The pinned digest.
    sha256: &'static str,
    /// The pinned size.
    bytes: u64,
    /// Origins, in the order tried (an operator's mirror first).
    origins: Vec<String>,
    /// An earlier installer's name for it, in the same directory.
    legacy: Option<&'static str>,
    /// The bytes themselves, when the binary carries them.
    embedded: Option<&'static str>,
    /// Other files kept when found in place ([`CatalogEntry::labels_also_accepted`]).
    also_accepted: &'static [(&'static str, u64)],
}

impl FileSpec {
    /// The state of this file in `dir`: verified when it is the pinned file or
    /// one of [`Self::also_accepted`].
    fn state_in(&self, dir: &Path, rehash: Rehash) -> FileState {
        let path = dir.join(&self.name);
        let pinned = record::check(&path, self.sha256, self.bytes, rehash);
        if pinned != FileState::Damaged {
            return pinned;
        }
        // Not the pinned file. One of the others is as good; one that cannot
        // be told without hashing is unchecked, as the pinned file would be.
        let others = self
            .also_accepted
            .iter()
            .map(|(sha256, bytes)| record::check(&path, sha256, *bytes, rehash));
        let mut state = FileState::Damaged;
        for other in others {
            match other {
                FileState::Verified => return FileState::Verified,
                FileState::Unchecked => state = FileState::Unchecked,
                FileState::Damaged | FileState::Missing => {}
            }
        }
        state
    }
}

impl CatalogEntry {
    /// The model file, as an install puts it in place.
    fn model_spec(&self, policy: &FetchPolicy) -> FileSpec {
        let name = format!("{}.onnx", self.id);
        FileSpec {
            origins: policy.origins_for(&name, &model_origins(self)),
            name,
            sha256: self.model_sha256,
            bytes: self.model_bytes,
            legacy: self.legacy_model_name,
            embedded: None,
            also_accepted: &[],
        }
    }

    /// The labels file, when the entry ships one.
    fn labels_spec(&self, policy: &FetchPolicy) -> Option<FileSpec> {
        let sha256 = self.labels_sha256?;
        let name = labels_file_name(self)?;
        let bytes = self
            .labels_embedded
            .map_or(self.labels_bytes, |text| text.len() as u64);
        let origins = if self.labels_embedded.is_some() {
            Vec::new()
        } else {
            policy.origins_for(&name, &labels_origins(self))
        };
        Some(FileSpec {
            name,
            sha256,
            bytes,
            origins,
            legacy: self.legacy_labels_name,
            embedded: self.labels_embedded,
            also_accepted: self.labels_also_accepted,
        })
    }
}

/// The name [`install`] gives an entry's labels file, or `None` when the entry
/// ships none.
fn labels_file_name(entry: &CatalogEntry) -> Option<String> {
    entry
        .ships_labels()
        .then(|| format!("{}_labels.{}", entry.id, entry.labels_ext))
}

/// Where [`install`] puts an entry's model and (if it ships one) its labels.
///
/// Pure: it names paths and touches nothing, so the setup page can say where a
/// classifier will land, and the daemon can look for it, using the one rule the
/// installer itself follows.
#[must_use]
pub fn installed_paths(entry: &CatalogEntry, dest_dir: &Path) -> (PathBuf, Option<PathBuf>) {
    (
        dest_dir.join(format!("{}.onnx", entry.id)),
        labels_file_name(entry).map(|n| dest_dir.join(n)),
    )
}

/// Whether `entry` is installed in `dir` and its files verify against the
/// catalogue's pins: THE answer to "is this model installed", for every
/// caller (MDL-11).
///
/// A file whose verification record still matches it (same size,
/// modification time and inode as when it was hashed) is answered from the
/// record; otherwise `rehash` decides. [`Rehash::Never`] is for a status poll,
/// which must not hash 541 MB and answers [`FileState::Unchecked`] instead;
/// [`Rehash::IfChanged`] hashes what the record cannot vouch for and records
/// the answer; [`Rehash::Always`] hashes regardless (the start of the station,
/// and `--install-model`). A file of the wrong size is damaged without being
/// read.
#[must_use]
pub fn installed_state(entry: &CatalogEntry, dir: &Path, rehash: Rehash) -> InstallState {
    let model = installed_paths(entry, dir).0;
    let labels = entry
        .labels_spec(&FetchPolicy::default())
        .map(|spec| spec.state_in(dir, rehash));
    InstallState {
        model: record::check(&model, entry.model_sha256, entry.model_bytes, rehash),
        labels,
    }
}

/// [`installed_state`] with [`Rehash::IfChanged`], as a yes or no.
#[must_use]
pub fn is_installed(entry: &CatalogEntry, dir: &Path) -> bool {
    installed_state(entry, dir, Rehash::IfChanged).is_verified()
}

/// Re-hash every catalogue entry whose model file is in `dir`.
///
/// Refreshes the verification records and returns each entry's state. What
/// the station runs once in the background at start (MDL-11): a file damaged
/// in place keeps its size and modification time, so only reading it finds
/// the damage.
#[must_use]
pub fn reverify_installed(dir: &Path) -> Vec<(&'static CatalogEntry, InstallState)> {
    CATALOG
        .iter()
        .filter(|e| installed_paths(e, dir).0.exists())
        .map(|e| (e, installed_state(e, dir, Rehash::Always)))
        .collect()
}

/// One of the files an entry installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The ONNX model.
    Model,
    /// Its labels file.
    Labels,
}

/// Which of `entry`'s installed files `path` is, by the name [`install`]
/// gives it in `dir`; `None` for any other name.
#[must_use]
pub fn part_at(entry: &CatalogEntry, dir: &Path, path: &Path) -> Option<Part> {
    let (model, labels) = installed_paths(entry, dir);
    if model == path {
        Some(Part::Model)
    } else if labels.as_deref() == Some(path) {
        Some(Part::Labels)
    } else {
        None
    }
}

/// [`installed_state`] for one of the entry's files; `None` for the labels
/// of an entry that ships none.
#[must_use]
pub fn part_state(
    entry: &CatalogEntry,
    dir: &Path,
    part: Part,
    rehash: Rehash,
) -> Option<FileState> {
    match part {
        Part::Model => Some(
            entry
                .model_spec(&FetchPolicy::default())
                .state_in(dir, rehash),
        ),
        Part::Labels => entry
            .labels_spec(&FetchPolicy::default())
            .map(|spec| spec.state_in(dir, rehash)),
    }
}

/// Install one of an entry's files and nothing else, as [`install`] does.
///
/// The station's repair of a file its configuration names (MDL-10), which
/// must not download a 541 MB model that nothing configured to replace the
/// labels file beside it.
///
/// # Errors
///
/// As [`install`]; [`InstallError::Unverifiable`] for the labels of an entry
/// that ships none.
pub fn install_part(
    id: &str,
    dest_dir: &Path,
    part: Part,
    policy: &FetchPolicy,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), InstallError> {
    let entry = lookup(id)?;
    let spec = match part {
        Part::Model => entry.model_spec(policy),
        Part::Labels => entry
            .labels_spec(policy)
            .ok_or_else(|| InstallError::Unverifiable {
                id: entry.id,
                why: "it ships no labels file".to_owned(),
            })?,
    };
    require_digest(entry.id, spec.sha256)?;
    fs::create_dir_all(dest_dir)?;
    let mut net = download::Network::new(policy);
    ensure_file(entry.id, &spec, dest_dir, None, &mut net, progress)
}

/// Install a model from the catalogue into `dest_dir`, with the network
/// policy the environment sets ([`FetchPolicy::from_env`]).
///
/// # Errors
///
/// [`InstallError`] for an unknown id, a digest that cannot verify, a disk
/// that cannot hold the download, a network failure, or a digest mismatch.
/// Nothing is left under a name the station would load unless it verified.
pub fn install(id: &str, dest_dir: &Path) -> Result<Installed, InstallError> {
    install_with_progress(id, dest_dir, &mut |_| {})
}

/// [`install`], telling `progress` how the download is going.
///
/// The browser's setup page installs a classifier through this: it runs on a
/// worker thread and publishes each report for the page to poll. The callback
/// is called at least once per megabyte of a download, and not at all for a
/// file that already verifies (there is nothing to wait for).
///
/// # Errors
///
/// As [`install`].
pub fn install_with_progress(
    id: &str,
    dest_dir: &Path,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Installed, InstallError> {
    install_with_policy(id, dest_dir, &FetchPolicy::from_env(), progress)
}

/// [`install_with_progress`] with an explicit network policy.
///
/// # Errors
///
/// As [`install`].
pub fn install_with_policy(
    id: &str,
    dest_dir: &Path,
    policy: &FetchPolicy,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Installed, InstallError> {
    install_entry(lookup(id)?, dest_dir, None, policy, progress)
}

/// [`install`], with the model taken from `model_file`, a copy already on this
/// machine, instead of the network (OOB-20: an install with no internet).
///
/// The file passes the same check a download does, and nothing weaker: its
/// size must be [`CatalogEntry::model_bytes`] (looked at first, so a wrong file
/// is refused without reading it) and its sha256 [`CatalogEntry::model_sha256`];
/// it is copied to a name the station never loads and renamed into place only
/// once it verifies. A file that does not match is refused, and nothing lands
/// under the model's name. The labels are installed as [`install`] installs
/// them (written from the binary for the entries that embed them, so those need
/// no network at all).
///
/// # Errors
///
/// As [`install`], and [`InstallError::WrongSize`] or
/// [`InstallError::NotThePinnedFile`] for a file that is not the pinned model.
pub fn install_from_file(
    id: &str,
    model_file: &Path,
    dest_dir: &Path,
) -> Result<Installed, InstallError> {
    install_entry(
        lookup(id)?,
        dest_dir,
        Some(model_file),
        &FetchPolicy::from_env(),
        &mut |_| {},
    )
}

/// The catalogue entry for `id`, or the error that lists what there is.
fn lookup(id: &str) -> Result<&'static CatalogEntry, InstallError> {
    find(id).ok_or_else(|| InstallError::Unknown {
        id: id.to_owned(),
        known: CATALOG.iter().map(|e| e.id).collect(),
    })
}

/// The one install path: the model from `model_file` when given, else from the
/// entry's origins; then its labels.
fn install_entry(
    entry: &'static CatalogEntry,
    dest_dir: &Path,
    model_file: Option<&Path>,
    policy: &FetchPolicy,
    progress: &mut dyn FnMut(&Progress),
) -> Result<Installed, InstallError> {
    // Before the network: an entry that can never verify must not cost
    // somebody 400 MB of a metered uplink to discover that.
    require_digest(entry.id, entry.model_sha256)?;
    if let Some(labels_sha) = entry.labels_sha256 {
        require_digest(entry.id, labels_sha)?;
    }
    fs::create_dir_all(dest_dir)?;

    let mut net = download::Network::new(policy);
    let model = entry.model_spec(policy);
    ensure_file(entry.id, &model, dest_dir, model_file, &mut net, progress)?;
    let labels_path = match entry.labels_spec(policy) {
        Some(spec) => {
            ensure_file(entry.id, &spec, dest_dir, None, &mut net, progress)?;
            Some(dest_dir.join(&spec.name))
        }
        None => None,
    };

    Ok(Installed {
        id: entry.id,
        model_path: dest_dir.join(&model.name),
        labels_path,
        sample_rate: entry.sample_rate,
        usage: entry.usage,
    })
}

/// Put one file in place under its catalogue name, by the cheapest route that
/// ends in verified bytes: already there; an earlier installer's copy beside
/// it; the binary's own bytes; the operator's local file; the network. Writes
/// the verification record once it is.
fn ensure_file(
    id: &'static str,
    spec: &FileSpec,
    dir: &Path,
    local: Option<&Path>,
    net: &mut download::Network,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(), InstallError> {
    let dest = dir.join(&spec.name);
    // Hashed in full, not taken from its record: `--install-model` is what an
    // operator runs when something is wrong, and a file damaged in place keeps
    // its size and its modification time.
    if spec.state_in(dir, Rehash::Always) == FileState::Verified {
        tracing::info!(id, path = %dest.display(), "already installed and verified; not fetching it again");
        return Ok(());
    }
    if let Some(legacy) = spec.legacy
        && stage::adopt(&dir.join(legacy), &dest, spec.sha256, spec.bytes)?
    {
        tracing::info!(id, from = legacy, path = %dest.display(), "adopted the verified file an earlier installer left under its old name");
        record::write(&dest, spec.sha256);
        return Ok(());
    }
    if let Some(text) = spec.embedded {
        stage::write_verified(text.as_bytes(), spec.sha256, &dest)?;
    } else if let Some(src) = local {
        tracing::info!(id, from = %src.display(), "installing from a local file");
        stage::require_size(src, spec.bytes)?;
        fits(spec.bytes, free_space_bytes(dir))?;
        stage::copy_verified(src, spec.sha256, spec.bytes, &dest)?;
    } else {
        tracing::info!(id, file = %spec.name, "downloading");
        net.fetch(spec, &dest, progress)?;
    }
    record::write(&dest, spec.sha256);
    tracing::info!(id, path = %dest.display(), "verified and installed");
    Ok(())
}

#[cfg(test)]
mod tests;
