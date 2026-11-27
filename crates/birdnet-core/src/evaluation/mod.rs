//! Instruments for measuring the station against ground truth (audit ACC-03,
//! ACC-20, MMP-15, ACC-12, RES-31, MMP-13).
//!
//! **No field measurement has been made with any of these.** They are the
//! harness, the formats and the arithmetic; the numbers they produce depend on
//! annotated recordings, hardware and a sampling design that this repository
//! does not contain. `docs/book/guides/evaluation.md` is the protocol.
//!
//! - [`selection_table`] reads Raven-style selection tables (the annotations).
//! - [`scoring`] scores station output against them: per-species precision and
//!   recall at each threshold, with Wilson score intervals and a
//!   recording-cluster bootstrap.
//! - [`golden`] records a classifier's raw output on a fixed input bit for bit,
//!   and compares two such records (aarch64 against x86-64, or one run against
//!   another).
//! - [`timing`] summarises per-window inference times with their spread.
//!
//! "Parity" elsewhere in this repository is *agreement with BirdNET-Pi*, not
//! accuracy. Precision here is agreement with an annotator, which is only as
//! good as the annotation; neither is a probability that a detection is right
//! at another site.

pub mod golden;
pub mod scoring;
pub mod selection_table;
pub mod timing;
