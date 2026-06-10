//! Fuzz the species-label parsers.
//!
//! Label files are user-supplied (custom/translated label sets are a
//! documented workflow), so both the plain and the CSV parser must reject
//! arbitrary text with a typed error, never a panic.

#![no_main]

use birdnet_core::inference::labels::LabelSet;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|content: &str| {
    let _ = LabelSet::parse(content);
    let _ = LabelSet::parse_csv(content);
});
