//! When a test is allowed to skip, and when skipping is itself the failure.
//!
//! # The hole this closes
//!
//! Several tests in this crate are gated on something that may not be there:
//! the `behavioral` extension fetched from the DuckDB community CDN, and the
//! ICU extension fetched from the DuckDB extension CDN. When it is absent they
//! print a line and return, which is right on a developer's laptop — a
//! contributor with no network should not see red for it.
//!
//! It was also right in CI, and that was the problem. CI fetches both
//! extensions best-effort, and on failure emitted `::warning::` and carried on:
//!
//! ```text
//! echo "::warning::could not fetch the behavioral extension; offline-load test will skip"
//! ```
//!
//! So a CDN outage turned the gate protecting the feature this project is named
//! for into nothing at all, and the job stayed green. Nothing distinguishes
//! "the extension loads offline" from "we never checked" in a green tick, and
//! `CLAUDE.md` names this exact shape: *a gate satisfied for reasons that had
//! nothing to do with what it claimed to assert.*
//!
//! # The rule
//!
//! `BIRDNET_REQUIRE_LIVE_EXTENSION=1` turns every such skip into a failure.
//! CI sets it on pushes to `main` — the runs whose green is a claim about
//! shippability — and leaves it unset on pull requests, where an upstream
//! outage is not the contributor's problem to solve.
//!
//! The variable is deliberately *not* set by the fetch step on success. A flag
//! the fetch controls could not catch the fetch failing, which is the whole
//! defect. It is a constant of the run, decided before anything is downloaded.

/// Whether a gated test may skip.
///
/// `false` — skips forbidden — when `BIRDNET_REQUIRE_LIVE_EXTENSION` is set to
/// anything other than the empty string. An empty value counts as unset so the
/// workflow can write `${{ … && '1' || '' }}` without a second `if:`.
#[must_use]
pub fn skips_allowed() -> bool {
    std::env::var("BIRDNET_REQUIRE_LIVE_EXTENSION")
        .ok()
        .is_none_or(|v| v.trim().is_empty())
}

/// Skip a gated test, or fail it if this run forbids skipping.
///
/// `what` names the missing prerequisite; `why` is whatever the attempt to
/// obtain it reported. Both end up in the failure message, because "a test
/// skipped" is not an actionable report and "the behavioral extension could not
/// be loaded: <error>" is.
///
/// # Panics
///
/// When `BIRDNET_REQUIRE_LIVE_EXTENSION` is set. That is the point.
pub fn skip_or_fail(what: &str, why: &str) {
    assert!(
        skips_allowed(),
        "{what} is unavailable ({why}), and BIRDNET_REQUIRE_LIVE_EXTENSION is \
         set — this run is not allowed to skip it.\n\
         \n\
         Either the CDN fetch in CI failed (re-run, or check\n\
         community-extensions.duckdb.org / extensions.duckdb.org), or the embed\n\
         step stopped pointing the build at the downloaded file. Do not silence\n\
         this by unsetting the variable: a green tick that means \"we never\n\
         checked\" is what it exists to prevent."
    );
    eprintln!("[skip] {what} is unavailable ({why}); skips are permitted on this run");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default, and what a laptop with no network sees.
    ///
    /// Reading rather than setting the variable: `std::env::set_var` is `unsafe`
    /// on this edition and the workspace forbids `unsafe`, so the two states are
    /// covered by running the suite twice — once plain, once with the variable
    /// set, which is exactly what CI does. This test asserts whichever state it
    /// finds, and says which.
    #[test]
    fn the_rule_matches_the_environment_it_is_running_in() {
        let set =
            std::env::var("BIRDNET_REQUIRE_LIVE_EXTENSION").is_ok_and(|v| !v.trim().is_empty());
        assert_eq!(
            skips_allowed(),
            !set,
            "BIRDNET_REQUIRE_LIVE_EXTENSION={:?} but skips_allowed()={}",
            std::env::var("BIRDNET_REQUIRE_LIVE_EXTENSION").ok(),
            skips_allowed()
        );
        if set {
            eprintln!("[gating] skips are FORBIDDEN on this run");
        } else {
            eprintln!("[gating] skips are permitted on this run");
        }
    }

    /// An empty value counts as unset, so the workflow's
    /// `${{ cond && '1' || '' }}` does not forbid skips on every run.
    #[test]
    fn an_empty_value_does_not_forbid_skipping() {
        // The parse rule on its own, without touching the process environment.
        let rule = |v: Option<&str>| v.is_none_or(|s| s.trim().is_empty());
        assert!(rule(None), "unset permits skipping");
        assert!(rule(Some("")), "empty permits skipping");
        assert!(rule(Some("   ")), "whitespace permits skipping");
        assert!(!rule(Some("1")), "'1' forbids skipping");
        assert!(!rule(Some("true")), "any non-empty value forbids skipping");
    }
}
