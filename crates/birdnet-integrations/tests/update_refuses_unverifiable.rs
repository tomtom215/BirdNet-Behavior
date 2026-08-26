//! An update that cannot be verified must not be installed.
//!
//! # What this is defending
//!
//! `apply_update` used to treat "no checksum" as a degraded-but-acceptable
//! mode. With `expected_sha256 == None` it logged
//!
//! ```text
//! no sha256 checksum available for the update asset;
//! integrity not verified (relying on the staged-binary smoke test)
//! ```
//!
//! and then installed the binary over the running one anyway. `check_for_update`
//! fed that `None` in from a `fetch_expected_sha256` that collapsed *every*
//! failure — no `SHA256SUMS` asset, a non-2xx response, an unparseable body, a
//! missing line — into `None` with no reason attached.
//!
//! The shape is the one `CLAUDE.md` names and the one D13 closed in CI: **"we
//! could not check" taking the same branch as "it checked out."** Here it is
//! worse than a green tick, because the small `SHA256SUMS` request is the
//! cheapest thing on the wire for an on-path attacker to drop — so whoever
//! could serve a malicious binary could also decide that no verification would
//! happen. The fallback the old comment leaned on, `<binary> --version`, proves
//! a file executes; it says nothing about whose file it is.
//!
//! # How the fix was observed to be needed
//!
//! A scratch probe (`zz_probe_unverified.rs`, deleted) ran the three cases
//! against the unmodified code and printed:
//!
//! ```text
//! [probe] None      -> Network("download failed: error sending request for url (…)")
//! [probe] Some(hex) -> Network("download failed: error sending request for url (…)")
//! [probe] Some(bad) -> Network("download failed: error sending request for url (…)")
//! ```
//!
//! All three identical: the decision was made *after* the download, so a
//! missing checksum was indistinguishable from a good one right up to the
//! point where 75 MB had already been fetched. That is what these assertions
//! now separate.
//!
//! # Why this needs no network
//!
//! `birdnet-behavior-test-invalid.github.com` satisfies `validate_release_url`
//! (it ends in `.github.com`) and does not resolve — verified with `getent
//! hosts`, which exits 2. So a run with no network, a run with network, and a
//! run behind a proxy all agree: anything that reaches the fetch fails as
//! `Network`, and anything refused earlier cannot be. The assertions are on the
//! error *variant*, never on a message a proxy could reshape.

use std::path::PathBuf;

use birdnet_integrations::auto_update::{UpdateError, apply_update};

/// Passes the host allow-list, resolves nowhere.
const UNREACHABLE: &str = "https://birdnet-behavior-test-invalid.github.com/a.tar.gz";

/// A syntactically valid digest that will never match anything.
fn well_formed_digest() -> String {
    "0".repeat(64)
}

/// A stand-in for the running binary, in its own directory so the "did
/// anything get written next to it" check below is unambiguous.
fn staging_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let bin = dir.path().join(name);
    std::fs::write(&bin, b"#!/bin/sh\necho 0.0.0\n").expect("write stub binary");
    (dir, bin)
}

/// The regression itself.
#[test]
fn an_update_with_no_published_checksum_is_refused() {
    let (dir, bin) = staging_dir("birdnet-behavior");

    let err = apply_update(UNREACHABLE, &bin, None).expect_err("must not install");
    assert!(
        matches!(err, UpdateError::Unverifiable(_)),
        "an unverifiable update must be refused as such, not attempted: {err:?}"
    );

    // And refused *before* the network, which is the part that makes it a
    // refusal rather than a late abort: the URL is unreachable, so reaching the
    // fetch at all would have produced `Network` instead.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read staging dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "nothing may be staged next to the running binary: {entries:?}"
    );
}

/// The counterpart, so the gate above discriminates rather than reporting
/// "`apply_update` returns an error" — which it would for any input, since the
/// host does not resolve. A well-formed digest must get *past* the checksum
/// gate and fail at the network instead.
#[test]
fn a_well_formed_checksum_is_not_refused_by_the_checksum_gate() {
    let (_dir, bin) = staging_dir("birdnet-behavior");

    let err = apply_update(UNREACHABLE, &bin, Some(&well_formed_digest()))
        .expect_err("the host does not resolve, so this cannot succeed");
    assert!(
        !matches!(err, UpdateError::Unverifiable(_)),
        "a 64-hex digest is exactly what the release publishes; refusing it \
         would block every legitimate update: {err:?}"
    );
    assert!(
        matches!(err, UpdateError::Network(_)),
        "it should have got as far as the download and failed there: {err:?}"
    );
}

/// Uppercase is still a valid digest — `verify_integrity` compares
/// case-insensitively, so the shape check must not be stricter than the
/// comparison it guards.
#[test]
fn an_uppercase_digest_is_accepted_by_the_shape_check() {
    let (_dir, bin) = staging_dir("birdnet-behavior");
    let err = apply_update(UNREACHABLE, &bin, Some(&"ABCDEF01".repeat(8)))
        .expect_err("host does not resolve");
    assert!(
        matches!(err, UpdateError::Network(_)),
        "uppercase hex must pass the shape check: {err:?}"
    );
}

/// A malformed digest is a defect in the *source* of the checksum, and saying
/// so before the download beats surfacing it as a mismatch 75 MB later.
#[test]
fn a_malformed_digest_is_refused_rather_than_downloaded_against() {
    let (_dir, bin) = staging_dir("birdnet-behavior");

    for bad in [
        "",                              // empty
        "   ",                           // whitespace only
        "not-a-hash",                    // not hex
        &"a".repeat(63),                 // one short
        &"a".repeat(65),                 // one long
        &format!("{}g", "a".repeat(63)), // right length, not hex
    ] {
        let err = apply_update(UNREACHABLE, &bin, Some(bad)).expect_err("must not install");
        assert!(
            matches!(err, UpdateError::Unverifiable(_)),
            "{bad:?} is not a sha256 and must be refused before the fetch: {err:?}"
        );
    }
}

/// The refusal has to be legible. An operator whose update stops needs to know
/// it stopped because it could not be checked — and that nothing was touched.
#[test]
fn the_refusal_says_what_happened() {
    let (_dir, bin) = staging_dir("birdnet-behavior");
    let msg = apply_update(UNREACHABLE, &bin, None)
        .expect_err("must not install")
        .to_string();
    assert!(
        msg.contains("could not be verified"),
        "the message must name verification as the reason: {msg:?}"
    );
    assert!(
        msg.contains("SHA256SUMS") || msg.contains("sha256"),
        "and point at the artefact that is missing: {msg:?}"
    );
}
