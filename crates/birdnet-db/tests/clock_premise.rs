//! `clock.rs`'s design note has to keep matching the dependency tree.
//!
//! # What this is defending
//!
//! The module used to justify itself with *"the workspace carries no
//! `chrono`/`time` dependency … so neither `localtime_r` nor a tz-database
//! parser is reachable."* That was false for the binary that ships: `chrono`
//! arrives transitively through `duckdb → arrow → arrow-arith`, brings
//! `iana-time-zone` with it, and the `analytics` feature that pulls `duckdb` is
//! on by default and compiled into every release build.
//!
//! Nothing failed, because nothing checks prose. The comment shaped a decision —
//! "we cannot use a date/time crate here" — on a cost that had already been
//! paid, and `CLAUDE.md` names this exact failure mode: *distrust confident
//! prose in this repo's history, including your own.*
//!
//! So the corrected comment states a checkable fact, and this checks it. If
//! `chrono` ever genuinely leaves the tree the assertion below fails, and
//! whoever removed it is told to fix the paragraph rather than leaving a second
//! generation of readers misinformed.
//!
//! # Why `Cargo.lock` rather than `cargo tree`
//!
//! `cargo tree` needs a working `cargo` and a network-reachable registry in the
//! middle of a test run. The lockfile is checked in, is what `cargo tree` reads
//! anyway, and is the thing a dependency change actually edits.

use std::path::Path;

/// The workspace lockfile, as text.
fn lockfile() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above crates/birdnet-db");
    std::fs::read_to_string(root.join("Cargo.lock")).expect("Cargo.lock is readable")
}

/// Whether the lockfile contains a package with this exact name.
fn locks(name: &str) -> bool {
    lockfile()
        .lines()
        .any(|l| l.trim() == format!("name = \"{name}\""))
}

/// The fact `clock.rs`'s note now rests on.
///
/// Observed failing against the note's previous wording in the only way prose
/// can be observed failing: `cargo tree -e normal | grep -c 'chrono v'` printed
/// **3** while the comment said the workspace carried no such dependency. This
/// test is what makes the corrected wording checkable instead of merely
/// better-informed.
#[test]
fn a_time_zone_database_reader_is_already_in_the_tree() {
    assert!(
        locks("chrono"),
        "`chrono` has left the dependency tree. That is fine — but \
         `crates/birdnet-db/src/clock.rs`'s design note explains its choice by \
         pointing out that a tz-database reader is *already* linked, and that \
         paragraph is now wrong. Fix the comment, then this test."
    );
    assert!(
        locks("iana-time-zone"),
        "`iana-time-zone` has left the dependency tree; see the note in \
         `crates/birdnet-db/src/clock.rs`, which cites it by name."
    );
}

/// The counterpart, so the gate above is a discrimination rather than "the
/// lockfile is non-empty": a package that is genuinely absent must read as
/// absent. `time` and `jiff` are the two crates the note says the workspace does
/// not carry, and it is right about those.
#[test]
fn the_crates_the_note_says_are_absent_really_are() {
    for absent in ["time", "jiff", "chrono-tz"] {
        assert!(
            !locks(absent),
            "`{absent}` is now in the tree — `clock.rs`'s note distinguishes it \
             from `chrono`, which arrives transitively, so the paragraph needs \
             revisiting"
        );
    }
}

/// And the reason it is in the tree is still the reason the note gives, so a
/// future reader chasing the citation lands somewhere real.
#[test]
fn chrono_still_arrives_through_the_analytics_stack() {
    let lock = lockfile();
    for via in ["arrow-arith", "duckdb"] {
        assert!(
            lock.lines()
                .any(|l| l.trim() == format!("name = \"{via}\"")),
            "`clock.rs` cites `duckdb → arrow → arrow-arith` as chrono's route \
             into the binary, and `{via}` is no longer there"
        );
    }
}
