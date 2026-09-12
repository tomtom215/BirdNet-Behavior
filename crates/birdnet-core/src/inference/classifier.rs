//! The classifier seam (`G-10` Stage 1).
//!
//! One station, one model, for as long as this project has existed. That is
//! not a limitation anybody chose — it is the absence of a seam, and the
//! absence showed up as assumptions scattered through the pipeline about what
//! *the* model wants: 48 kHz, three seconds, a mel spectrogram.
//!
//! [`Classifier`] is that seam. It says what a thing must be able to do to
//! stand where `BirdNetModel` stands, and [`InputSpec`] makes the three
//! assumptions into declared facts the pipeline reads instead of assuming.
//!
//! # What this buys before a second model ever ships
//!
//! The gap analysis calls Stages 1–3 worth doing on their own merits, and this
//! one paid for itself immediately: writing down "what does the model want fed
//! to it" as a value, rather than a rate comparison, is what exposed that
//! BirdNET V2.4 had been receiving a mel spectrogram zero-padded into a
//! waveform tensor. The seam is also what Perch v2 (32 kHz, 5 s), a bat
//! classifier, and the Silero voice-activity gate each need, and what an
//! alternative inference backend would implement.
//!
//! # Why `infer` takes `&mut self`
//!
//! The gap analysis sketches `fn infer(&self, …)`. The ONNX Runtime session
//! this is implemented over takes `&mut self` to run, so the trait follows the
//! implementation rather than forcing every implementor through interior
//! mutability for a signature nobody needs.

use crate::inference::labels::LabelSet;
use crate::inference::model::{InferenceError, InputSpec};

/// Anything that can score a window of audio against a set of labels.
///
/// Implemented by [`crate::inference::model::BirdNetModel`]; the registry and
/// per-source routing of Stage 2, and every model beyond BirdNET, sit behind
/// this.
pub trait Classifier {
    /// The labels this classifier scores, in output order.
    ///
    /// Positional: index `i` of [`Classifier::infer`]'s output is label `i`.
    /// A classifier whose output width does not match this length is paired
    /// with the wrong labels file and every row it produces is suspect.
    fn labels(&self) -> &LabelSet;

    /// What this classifier needs fed to it — rate, window and format.
    ///
    /// Read by the pipeline to decide how to resample, how long to chunk, and
    /// whether to hand over the waveform or a transform of it. Three separate
    /// facts, because they vary independently across models.
    fn input_spec(&self) -> InputSpec;

    /// Score one prepared window, returning one value per label.
    ///
    /// The values are whatever the model emits — logits for BirdNET V2.4,
    /// calibrated probabilities for V3.0. Mapping them to a confidence is the
    /// caller's job, because it depends on which of those two a model is; see
    /// [`crate::inference::model::compute_confidence`].
    ///
    /// # Errors
    ///
    /// [`InferenceError`] if the runtime fails, or if `window` is not
    /// something this classifier can take — including a slice too short to be
    /// a partial window, which indicates the wrong kind of data rather than a
    /// ragged edge.
    fn infer(&mut self, window: &[f32]) -> Result<Vec<f32>, InferenceError>;
}
