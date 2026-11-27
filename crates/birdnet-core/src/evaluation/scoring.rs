//! Scoring station output against annotations (audit ACC-03; the threshold
//! sweep serves ACC-20 and MMP-15).
//!
//! # The rule
//!
//! Both sides are intervals in one recording, named by species.
//!
//! - **Precision** counts *detections*: a detection at or above the threshold is
//!   a true positive when it overlaps, by more than [`MatchRule::min_overlap_secs`],
//!   an annotation of the same species in the same recording. Precision is true
//!   positives over detections. With `OVERLAP` above 0 one call is several
//!   detections, and each is judged; that is what a reader of the rows sees.
//! - **Recall** counts *annotations*: an annotation is recalled when at least
//!   one detection at or above the threshold overlaps it as above. Recall is
//!   recalled annotations over annotations.
//!
//! Only recordings named as annotated are scored. A recording annotated with
//! no sound of a species is evidence about that species (every detection of it
//! there is a false positive); a recording nobody annotated is not, and its
//! detections are left out rather than counted against the station.
//!
//! # The intervals
//!
//! - **Wilson score interval** (Wilson 1927) at the chosen `z` (1.96 for 95%)
//!   on each proportion. It treats detections (for precision) or annotations
//!   (for recall) as independent trials, which overlapping windows of one
//!   recording are not, so it is too narrow when they cluster.
//! - **Recording-cluster bootstrap** (percentile method) on the pooled
//!   proportions: recordings are resampled with replacement, `replicates`
//!   times, from a fixed seed, and the 2.5th and 97.5th percentiles reported.
//!   It respects the dependence within a recording, and is the interval to
//!   quote when the two differ. It says nothing about recordings unlike the
//!   ones sampled — another site, season or microphone.
//!
//! A score here is agreement with an annotator on a set of recordings. It is
//! not the probability that a detection is correct, and a recall below 1 is
//! not a statement about which birds were absent.

use std::collections::{BTreeMap, BTreeSet};

use crate::evaluation::selection_table::Annotation;

/// One detection, as the scorer reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// The recording, by file stem (see
    /// [`crate::evaluation::selection_table::recording_stem`]).
    pub recording: String,
    /// The detection's window start, seconds into the recording.
    pub begin_secs: f64,
    /// The detection's window end.
    pub end_secs: f64,
    /// Scientific name.
    pub species: String,
    /// The stored confidence, on its classifier's own scale.
    pub score: f32,
}

/// When a detection and an annotation are the same sound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchRule {
    /// They must overlap by more than this many seconds; `0.0` is any overlap
    /// of positive length.
    pub min_overlap_secs: f64,
}

impl Default for MatchRule {
    fn default() -> Self {
        Self {
            min_overlap_secs: 0.0,
        }
    }
}

impl MatchRule {
    fn matches(self, p: &Prediction, a: &Annotation) -> bool {
        p.recording == a.recording
            && p.species == a.species
            && p.end_secs.min(a.end_secs) - p.begin_secs.max(a.begin_secs) > self.min_overlap_secs
    }
}

/// Successes out of trials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Proportion {
    /// Successes.
    pub successes: u64,
    /// Trials.
    pub trials: u64,
}

impl Proportion {
    /// The point estimate, `None` with no trials.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn estimate(self) -> Option<f64> {
        (self.trials > 0).then(|| self.successes as f64 / self.trials as f64)
    }

    /// The Wilson score interval at `z` (1.96 for 95%), `None` with no trials.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn wilson(self, z: f64) -> Option<(f64, f64)> {
        if self.trials == 0 {
            return None;
        }
        let n = self.trials as f64;
        let p = self.successes as f64 / n;
        let z2 = z * z;
        let centre = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
        let half = (z / (1.0 + z2 / n)) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
        Some(((centre - half).max(0.0), (centre + half).min(1.0)))
    }
}

/// One species (or all of them pooled) at one threshold.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreRow {
    /// The species, or `None` for every species pooled.
    pub species: Option<String>,
    /// Detections at or above this were counted.
    pub threshold: f32,
    /// True-positive detections out of detections.
    pub precision: Proportion,
    /// Recalled annotations out of annotations.
    pub recall: Proportion,
    /// The recording-cluster bootstrap's 95% percentile interval on precision
    /// (pooled rows only, and only when asked for).
    pub precision_bootstrap: Option<(f64, f64)>,
    /// The same, on recall.
    pub recall_bootstrap: Option<(f64, f64)>,
}

/// What [`score`] is asked to do beyond the rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoringPlan {
    /// The thresholds to report, each as its own row set.
    pub thresholds: Vec<f32>,
    /// The match rule.
    pub rule: MatchRule,
    /// Bootstrap replicates for the pooled rows; `0` for none.
    pub replicates: u32,
    /// The bootstrap's seed, so a report is reproducible.
    pub seed: u64,
}

/// Per recording, the counts that pool into a [`ScoreRow`].
#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    tp: u64,
    predicted: u64,
    recalled: u64,
    annotated: u64,
}

impl std::ops::AddAssign for Counts {
    fn add_assign(&mut self, o: Self) {
        self.tp += o.tp;
        self.predicted += o.predicted;
        self.recalled += o.recalled;
        self.annotated += o.annotated;
    }
}

fn counts(
    annotations: &[&Annotation],
    predictions: &[&Prediction],
    threshold: f32,
    rule: MatchRule,
) -> Counts {
    let kept: Vec<&&Prediction> = predictions
        .iter()
        .filter(|p| p.score >= threshold)
        .collect();
    Counts {
        tp: kept
            .iter()
            .filter(|p| annotations.iter().any(|a| rule.matches(p, a)))
            .count() as u64,
        predicted: kept.len() as u64,
        recalled: annotations
            .iter()
            .filter(|a| kept.iter().any(|p| rule.matches(p, a)))
            .count() as u64,
        annotated: annotations.len() as u64,
    }
}

/// Score `predictions` against `annotations` over the recordings in
/// `annotated` (see the module documentation).
///
/// Rows come per threshold: the
/// pooled row first, then one per species in name order. Species are every
/// name annotated or predicted within the annotated recordings.
#[must_use]
pub fn score(
    annotated: &BTreeSet<String>,
    annotations: &[Annotation],
    predictions: &[Prediction],
    plan: &ScoringPlan,
) -> Vec<ScoreRow> {
    // recording -> species -> (annotations, predictions)
    type Cell<'a> = (Vec<&'a Annotation>, Vec<&'a Prediction>);
    let mut by: BTreeMap<&str, BTreeMap<&str, Cell<'_>>> = BTreeMap::new();
    for r in annotated {
        by.entry(r.as_str()).or_default();
    }
    for a in annotations
        .iter()
        .filter(|a| annotated.contains(&a.recording))
    {
        by.entry(&a.recording)
            .or_default()
            .entry(&a.species)
            .or_default()
            .0
            .push(a);
    }
    for p in predictions
        .iter()
        .filter(|p| annotated.contains(&p.recording))
    {
        by.entry(&p.recording)
            .or_default()
            .entry(&p.species)
            .or_default()
            .1
            .push(p);
    }
    let species: BTreeSet<&str> = by.values().flat_map(|m| m.keys().copied()).collect();
    let recordings: Vec<&str> = by.keys().copied().collect();

    let mut rows = Vec::new();
    for &threshold in &plan.thresholds {
        // Per recording, per species.
        let per: Vec<BTreeMap<&str, Counts>> = recordings
            .iter()
            .map(|r| {
                by[r]
                    .iter()
                    .map(|(s, (a, p))| (*s, counts(a, p, threshold, plan.rule)))
                    .collect()
            })
            .collect();
        let pooled_of = |rec: &BTreeMap<&str, Counts>| {
            let mut c = Counts::default();
            for v in rec.values() {
                c += *v;
            }
            c
        };
        let per_recording: Vec<Counts> = per.iter().map(pooled_of).collect();
        let mut all = Counts::default();
        for c in &per_recording {
            all += *c;
        }
        let (pb, rb) = if plan.replicates > 0 {
            bootstrap(&per_recording, plan.replicates, plan.seed)
        } else {
            (None, None)
        };
        rows.push(row(None, threshold, all, pb, rb));
        for s in &species {
            let mut c = Counts::default();
            for rec in &per {
                if let Some(v) = rec.get(s) {
                    c += *v;
                }
            }
            rows.push(row(Some((*s).to_owned()), threshold, c, None, None));
        }
    }
    rows
}

const fn row(
    species: Option<String>,
    threshold: f32,
    c: Counts,
    precision_bootstrap: Option<(f64, f64)>,
    recall_bootstrap: Option<(f64, f64)>,
) -> ScoreRow {
    ScoreRow {
        species,
        threshold,
        precision: Proportion {
            successes: c.tp,
            trials: c.predicted,
        },
        recall: Proportion {
            successes: c.recalled,
            trials: c.annotated,
        },
        precision_bootstrap,
        recall_bootstrap,
    }
}

/// A percentile interval: (2.5th, 97.5th).
type Interval = Option<(f64, f64)>;

/// `SplitMix64`: a fixed, dependency-free generator, so a seed names one
/// sequence on every platform.
const fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Percentile interval of pooled precision and recall over recordings
/// resampled with replacement. A replicate with no detections (or no
/// annotations) has no estimate and is left out of that side's percentiles.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn bootstrap(recordings: &[Counts], replicates: u32, seed: u64) -> (Interval, Interval) {
    if recordings.is_empty() {
        return (None, None);
    }
    let n = recordings.len() as u64;
    let mut state = seed;
    let mut p = Vec::with_capacity(replicates as usize);
    let mut r = Vec::with_capacity(replicates as usize);
    for _ in 0..replicates {
        let mut c = Counts::default();
        for _ in 0..n {
            c += recordings[(splitmix(&mut state) % n) as usize];
        }
        if let Some(e) = (Proportion {
            successes: c.tp,
            trials: c.predicted,
        })
        .estimate()
        {
            p.push(e);
        }
        if let Some(e) = (Proportion {
            successes: c.recalled,
            trials: c.annotated,
        })
        .estimate()
        {
            r.push(e);
        }
    }
    let interval = |mut v: Vec<f64>| {
        if v.is_empty() {
            return None;
        }
        v.sort_by(f64::total_cmp);
        let at = |q: f64| v[((q * (v.len() - 1) as f64).round() as usize).min(v.len() - 1)];
        Some((at(0.025), at(0.975)))
    };
    (interval(p), interval(r))
}

/// The rows as tab-separated text with a header, for a spreadsheet or a diff.
/// Missing estimates and intervals are empty cells, never zeros.
#[must_use]
pub fn to_tsv(rows: &[ScoreRow]) -> String {
    use std::fmt::Write as _;
    let f = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.4}"));
    let pair = |v: Option<(f64, f64)>| (f(v.map(|p| p.0)), f(v.map(|p| p.1)));
    let mut out = String::from(
        "species\tthreshold\ttp\tdetections\tprecision\tprecision_wilson_lo\tprecision_wilson_hi\t\
         precision_boot_lo\tprecision_boot_hi\trecalled\tannotations\trecall\trecall_wilson_lo\t\
         recall_wilson_hi\trecall_boot_lo\trecall_boot_hi\n",
    );
    for r in rows {
        let (pwl, pwh) = pair(r.precision.wilson(1.96));
        let (pbl, pbh) = pair(r.precision_bootstrap);
        let (rwl, rwh) = pair(r.recall.wilson(1.96));
        let (rbl, rbh) = pair(r.recall_bootstrap);
        let _ = writeln!(
            out,
            "{}\t{:.3}\t{}\t{}\t{}\t{pwl}\t{pwh}\t{pbl}\t{pbh}\t{}\t{}\t{}\t{rwl}\t{rwh}\t{rbl}\t{rbh}",
            r.species.as_deref().unwrap_or("(all)"),
            r.threshold,
            r.precision.successes,
            r.precision.trials,
            f(r.precision.estimate()),
            r.recall.successes,
            r.recall.trials,
            f(r.recall.estimate()),
        );
    }
    out
}

/// The header [`to_predictions_tsv`] writes and [`parse_predictions_tsv`] reads.
pub const PREDICTIONS_HEADER: &str = "recording\tbegin_secs\tend_secs\tspecies\tscore";

/// Predictions as tab-separated text, one per line.
///
/// The lines follow [`PREDICTIONS_HEADER`]: the interchange format, so predictions from any
/// producer (this crate's `model_check` example, another station, another
/// tool) can be scored the same way.
#[must_use]
pub fn to_predictions_tsv(predictions: &[Prediction]) -> String {
    use std::fmt::Write as _;
    let mut out = format!("{PREDICTIONS_HEADER}\n");
    for p in predictions {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            p.recording, p.begin_secs, p.end_secs, p.species, p.score
        );
    }
    out
}

/// Read [`to_predictions_tsv`]'s format.
///
/// # Errors
///
/// A header other than [`PREDICTIONS_HEADER`], or a line that does not have
/// five cells that parse, with its 1-based line number.
pub fn parse_predictions_tsv(content: &str) -> Result<Vec<Prediction>, String> {
    let mut lines = content
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (_, header) = lines.next().ok_or("the predictions are empty")?;
    if header.trim() != PREDICTIONS_HEADER {
        return Err(format!("the header is not `{PREDICTIONS_HEADER}`"));
    }
    lines
        .map(|(i, line)| {
            let c: Vec<&str> = line.split('\t').map(str::trim).collect();
            let bad = || format!("line {}: `{line}`", i + 1);
            if c.len() != 5 {
                return Err(bad());
            }
            Ok(Prediction {
                recording: c[0].to_owned(),
                begin_secs: c[1].parse().map_err(|_| bad())?,
                end_secs: c[2].parse().map_err(|_| bad())?,
                species: c[3].to_owned(),
                score: c[4].parse().map_err(|_| bad())?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predictions_survive_their_interchange_format() {
        let p = vec![pred("site 1", 4.5, "Turdus merula", 0.812_3)];
        let back = parse_predictions_tsv(&to_predictions_tsv(&p)).unwrap();
        assert_eq!(back, p);
        assert!(parse_predictions_tsv("recording\tbegin\n").is_err());
        let bad = format!("{PREDICTIONS_HEADER}\nr\tx\t1\tA\t0.5\n");
        assert!(parse_predictions_tsv(&bad).is_err());
    }

    fn ann(rec: &str, b: f64, e: f64, s: &str) -> Annotation {
        Annotation {
            recording: rec.into(),
            begin_secs: b,
            end_secs: e,
            species: s.into(),
        }
    }

    fn pred(rec: &str, b: f64, s: &str, score: f32) -> Prediction {
        Prediction {
            recording: rec.into(),
            begin_secs: b,
            end_secs: b + 3.0,
            species: s.into(),
            score,
        }
    }

    fn plan(thresholds: &[f32]) -> ScoringPlan {
        ScoringPlan {
            thresholds: thresholds.to_vec(),
            rule: MatchRule::default(),
            replicates: 0,
            seed: 1,
        }
    }

    #[test]
    fn wilson_matches_the_published_worked_values() {
        // 81/263 at 95%: (0.2553, 0.3662), as computed by the formula in
        // Newcombe (1998), method 3 (no continuity correction).
        let (lo, hi) = Proportion {
            successes: 81,
            trials: 263,
        }
        .wilson(1.959_964)
        .unwrap();
        assert!((lo - 0.2553).abs() < 5e-4, "{lo}");
        assert!((hi - 0.3662).abs() < 5e-4, "{hi}");
        // 0/10: the lower bound is 0, the upper one is not.
        let (lo, hi) = Proportion {
            successes: 0,
            trials: 10,
        }
        .wilson(1.96)
        .unwrap();
        assert!(lo.abs() < 1e-12);
        assert!((hi - 0.2775).abs() < 5e-4, "{hi}");
        assert_eq!(
            Proportion::default().wilson(1.96),
            None,
            "no trials, no interval"
        );
    }

    #[test]
    fn precision_counts_detections_and_recall_counts_annotations() {
        let annotated = BTreeSet::from(["r1".to_owned(), "r2".to_owned()]);
        let annotations = [
            ann("r1", 1.0, 2.0, "A"),
            ann("r1", 10.0, 11.0, "A"),
            ann("r2", 0.5, 1.0, "B"),
        ];
        let predictions = [
            pred("r1", 0.0, "A", 0.9),  // hits the first A
            pred("r1", 1.5, "A", 0.8),  // the same A again: a TP too
            pred("r1", 20.0, "A", 0.7), // nothing there
            pred("r2", 6.0, "B", 0.9),  // misses B
            pred("r3", 0.0, "A", 0.9),  // r3 is not annotated: not scored
        ];
        let rows = score(&annotated, &annotations, &predictions, &plan(&[0.0, 0.85]));
        let all = &rows[0];
        assert_eq!(all.species, None);
        assert_eq!(
            all.precision,
            Proportion {
                successes: 2,
                trials: 4
            }
        );
        assert_eq!(
            all.recall,
            Proportion {
                successes: 1,
                trials: 3
            }
        );
        let a = rows
            .iter()
            .find(|r| r.species.as_deref() == Some("A") && r.threshold == 0.0)
            .unwrap();
        assert_eq!(
            a.precision,
            Proportion {
                successes: 2,
                trials: 3
            }
        );
        assert_eq!(
            a.recall,
            Proportion {
                successes: 1,
                trials: 2
            }
        );
        // At 0.85 only the 0.9 detections count.
        let hi = rows
            .iter()
            .find(|r| r.species.is_none() && r.threshold > 0.5)
            .unwrap();
        assert_eq!(
            hi.precision,
            Proportion {
                successes: 1,
                trials: 2
            }
        );
    }

    #[test]
    fn touching_is_not_overlapping_and_another_species_is_not_a_match() {
        let annotated = BTreeSet::from(["r".to_owned()]);
        let rows = score(
            &annotated,
            &[ann("r", 3.0, 4.0, "A")],
            &[pred("r", 0.0, "A", 0.9), pred("r", 3.0, "B", 0.9)],
            &plan(&[0.0]),
        );
        assert_eq!(
            rows[0].precision.successes, 0,
            "[0,3) ends where [3,4) begins"
        );
        assert_eq!(rows[0].recall.successes, 0);
    }

    #[test]
    fn an_empty_side_has_no_estimate_and_prints_an_empty_cell() {
        let annotated = BTreeSet::from(["r".to_owned()]);
        let rows = score(&annotated, &[ann("r", 0.0, 1.0, "A")], &[], &plan(&[0.5]));
        assert_eq!(rows[0].precision.estimate(), None);
        let tsv = to_tsv(&rows);
        let line = tsv.lines().nth(1).unwrap();
        let cells: Vec<&str> = line.split('\t').collect();
        assert_eq!(cells[3], "0", "no detections");
        assert_eq!(cells[4], "", "no precision, not 0");
        assert_eq!(cells[11], "0.0000", "recall 0 of 1 is a measured 0");
    }

    #[test]
    fn the_bootstrap_is_reproducible_from_its_seed_and_brackets_the_estimate() {
        let annotated: BTreeSet<String> = (0..20).map(|i| format!("r{i}")).collect();
        let annotations: Vec<Annotation> = (0..20)
            .map(|i| ann(&format!("r{i}"), 0.0, 1.0, "A"))
            .collect();
        let predictions: Vec<Prediction> = (0..20)
            .map(|i| {
                pred(
                    &format!("r{i}"),
                    if i % 4 == 0 { 9.0 } else { 0.0 },
                    "A",
                    0.9,
                )
            })
            .collect();
        let mut p = plan(&[0.0]);
        p.replicates = 500;
        let a = score(&annotated, &annotations, &predictions, &p);
        let b = score(&annotated, &annotations, &predictions, &p);
        assert_eq!(a, b, "same seed, same interval");
        let (lo, hi) = a[0].precision_bootstrap.unwrap();
        let est = a[0].precision.estimate().unwrap();
        assert!((est - 0.75).abs() < 1e-12);
        assert!(lo < est && est < hi, "({lo}, {hi}) around {est}");
        p.seed = 2;
        assert_ne!(
            score(&annotated, &annotations, &predictions, &p)[0].precision_bootstrap,
            a[0].precision_bootstrap
        );
    }
}
