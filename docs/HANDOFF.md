# Handoff

What only the maintainer can do, what the last working session did and did not
verify, and where the next one should start. Facts about GitHub are as read on
**2026-10-14** with read-only API calls; re-read them before relying on them.

## 1. Repository settings only the maintainer can change

These are settings, not files, so no commit can make them true.

| Setting | State on 2026-10-14 | Why it matters | Where |
|---|---|---|---|
| Private vulnerability reporting | **off** (`{"enabled":false}`) | `SECURITY.md` and the issue-template chooser both send reporters to *Report a vulnerability*, which does not exist while this is off | Settings → Code security → Private vulnerability reporting |
| Dependabot alerts | **off** (the API answers 403) | the weekly `cargo audit` / `cargo deny` run is the only advisory monitor; alerts would add a daily one | Settings → Code security → Dependabot alerts |
| Protection of `main` | **none**: `protected: false`, no required checks; the one ruleset, `protect-main`, is `disabled` | nothing stops a push that fails CI; no check is required | Settings → Rules → Rulesets: enable `protect-main`, add the checks listed in [`.github/workflows/README.md`](../.github/workflows/README.md) (mind its note on path-filtered workflows) |
| Annotated release tags | 20 of the 23 tag refs are lightweight commits; `v0.16.1`, `v0.17.0`, `v0.17.1` are tag objects | `RELEASING.md` prescribes `git tag -a`; nothing checks it | use `git tag -a` (or `-s`); `release.yml` does not yet refuse a lightweight tag |
| Repository licence detection | `license.spdx_id` is `NOASSERTION` | `LICENSE` is a summary that points at `LICENSE-UPSTREAM`; GitHub cannot identify it | replace `LICENSE` with the full CC BY-NC-SA 4.0 text and move the lineage paragraph to `NOTICE`, if that is acceptable to you |

Not readable from here, so unknown: Actions permissions, environments, CodeQL
default setup, secret scanning, who has write access.

## 2. Decisions that are the maintainer's

- **Accept, amend or drop [`STABILITY.md`](../STABILITY.md).** It is marked
  *proposed*; every number in it was re-measured from the tree on 2026-10-14
  (53 migrations, 147 configuration keys, 111 option lines in `--help`, 85
  OpenAPI paths and 87 operations, 24 releases).
- **Publishing to crates.io.** The licence is CC BY-NC-SA 4.0 because the work
  derives from BirdNET-Pi. Whether to publish, under which crate names, and who holds the
  account is not something the repository can decide. See section 5 for what is
  and is not ready.
- **Whether the bundled species names may be redistributed.** The 35 language
  tables behind *Common-name language* are BirdNET-Pi's `model/l18n/`, which its
  README credits to @patlevin and to Wikipedia without saying which name came
  from where. Wikipedia text is CC BY-SA 4.0; this project is CC BY-NC-SA 4.0.
  `NOTICE` and `crates/birdnet-core/data/l18n/README.md` give the credit and say
  the question is not settled here. It matters most before a crates.io release,
  because the 3.7 MB table ships inside `birdnet-core`.
- **Release signing.** Tags and `SHA256SUMS` are unsigned; the in-app updater
  trusts the GitHub release. A signing key and a verifier in the updater would
  close that, and the key is yours to create.

## 3. What the last session changed, and how far it was checked

Everything is on branch `claude/vigilant-goldberg-grdjk1`. The draft pull request
opened from it is the first thing to read: until it was opened no GitHub Actions
workflow had run on any of this (the `pull_request`-triggered gates never see an
un-PR'd branch), and what CI says about it is not recorded here.

Checked locally, by running it (details in each commit message). "The tested
tree" is that of `204e296`; the commits after it change `CLAUDE.md` and this file
only.

- **Static checks, on the tested tree:** `cargo fmt --check`; `cargo clippy
  --workspace --all-targets -- -D warnings` with default features,
  `--all-features` and `--no-default-features`; `cargo doc --no-deps
  --document-private-items --all-features` with warnings denied; `typos` 1.51.1
  (the binary's digest was not verified).
- **Tests, on the tested tree:** 110 steps — every library, binary and
  integration-test file of every crate, and the doctests — with 4,984 tests
  passing, 0 failing, 10 ignored. That figure is the logged 4,970 with the
  `birdnet-behavioral` lib step counted at its rerun's 125 (it logged 121 passed
  and 4 failed) plus the 10 tests of the chunk that did not link. Default
  features, except `birdnet-behavioral` and `birdnet-timeseries`, run with
  `--features analytics` (the workspace enables it through the root crate; a
  crate selected alone does not, and those tests then do not exist). The
  `--all-features` *test* build was not run; clippy with it was. Two steps failed
  once, both when the disk allowance was nearly spent, and passed on rerun: the
  four `birdnet-behavioral` ICU and extension tests that `CLAUDE.md` already lists
  as an out-of-space symptom (125 passed on each of three reruns; the cause was
  not proven), and a link step that died at 8 KB free.
- **Browser, on the final build:** the five steps of the `a11y` job replayed
  against the CI screenshot fixture (axe over 38 routes x 2 themes x 2 widths:
  152 legs, 0 violations; the interaction gate; the legibility gate; the
  base-path gate; the strict visual sweep: 0 pages with issues); axe over the
  same legs against a station running the real BirdNET+ V3.0 and geomodel set to
  German, and against an empty station (`QA_EMPTY=1`): 0 violations each; the
  visual sweep against the real-model station: 0 pages with issues.
- **Real models:** that station detected a magpie four times from
  `tests/testdata/Pica_pica_30s.wav`, stored "Eurasian Magpie", and drew "Elster"
  on every Today fragment; `--doctor --lang de` reported 6593 of 11560 species
  named. `--doctor` exited 1 there for five environment warnings (no admin
  password, no HTTPS, no offsite backup, no audio source, 1 GiB free); it
  reported no errors.
- **Earlier in this branch, not re-run on this tree:** `actionlint` on every
  workflow, the hermetic installer test suite, the extension pins against freshly
  downloaded builds, the licence generator against the real dependency tree.
  `scripts/check-ci-config.py` fails only where `cargo-mutants` is needed and is
  not installed here ("generates 0 mutants").

**Not run, anywhere:** any workflow on GitHub; the Docker build and its offline
boot test; the release workflow; `cargo-mutants`; the container image's new
`/usr/share/doc/birdnet-behavior/` files; `cargo publish` and a verifying
`cargo package` (`birdnet-core` was packaged without building: 102 files, 1.8 MiB
compressed, with the species-name table in it); anything on Raspberry Pi hardware
(the cgroup memory-limit fix in `memory.rs` is tested against a directory tree
laid out as the kernel lays it out, not against a unit under cgroup v2). Not
covered by a page fetch in the species-name gate: the share page, migration, the
behavioural and correlation pages, and `live-detections.js`.

**A note on the history.** `ff51acc` (the species-name languages) does not
compile its test targets on its own: a tool I used to split the batch into two
commits misplaced hunks in `species_pages.rs`, `detection_detail.rs` and
`settings/render/mod.rs`. `204e296` holds the whole tested tree and is the first
good commit of the pair, so a bisect should skip `ff51acc`. A corrected pair was
built and checked in the working clone (it compiles, lints and passes the
species-name gates, and its second commit has `204e296`'s tree), but replacing
the pushed commits needs a force-push, which the session's safety check declined,
so history was fixed forward instead.

## 4. Where to start

1. Read what CI says on the draft pull request from `claude/vigilant-goldberg-grdjk1`
   and fix what it finds.
   Expect the `third-party licence notices`, `CI config` and `a11y` jobs to be
   the first to run something new.
2. Do the settings in section 1.
3. Read the open items below; each is real and none was started unless it says so.

## 5. Open items

Not done, in the order they seem to matter:

- **crates.io.** Ready to publish, not published; the decision and the account
  are yours (section 2). What was done and what was not is in
  [`RELEASING.md`](../RELEASING.md#publishing-to-cratesio-the-maintainer-not-a-workflow):
  the bundled-DuckDB build of the packaged crates, docs.rs, and `cargo publish`
  itself were not run.
- **Feature parity.** The gap analysis against birdnet-go and BirdNET-Pi is in
  [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md). Species names in 35
  languages now work (N‑5 there); what that leaves open: the interface text is
  still English (birdnet-go localizes its own, see Part 5), pages carry
  `lang="en"` around names in other scripts (WCAG 3.1.2, which no automated
  check covers), the sort order is by the stored name, and the surfaces that
  need data to render (migration, the behavioural pages, the correlation matrix)
  are covered by the source scan but not by a page fetch.
- **The browser sweeps grade a populated station.** axe and `qa.mjs` run against
  a fixture that has detections, so a page's empty state is only reached where a
  fixture happens to be empty: a real station's empty Rare bird quarantine list
  carried a `heading-order` finding the sweep had never shown. The empty and
  failure fragments now have a unit gate; the pages that host them do not have a
  sweep over an empty station.
- **Workflows that have never run:** `codeql.yml`, `osv.yml` and `scorecard.yml`
  were adapted from a2a-rust's, which run, and checked with actionlint; the first
  run on GitHub is their test. Scorecard needs the repository to be public (it
  is) and will score low on branch protection and vulnerability reporting until
  section 1 is done. `fuzz.yml` (new, tested here by running its commands) is
  the same kind of untried.
- **Pins that are still floating:** the `ubuntu:24.04` container that
  `install-smoke.yml` starts with `docker run` is a tag (Dependabot does not read
  `docker run` arguments, so a digest there would go stale unseen);
  `taiki-e/install-action` and `EmbarkStudios/cargo-deny-action` choose their own
  tool builds from the version named. Nothing checked whether Dependabot honours
  the `ignore` rule for the builder's Rust tag: the first docker PR will say.
- **Dependencies:** `argon2`, `rubato` and htmx are behind their latest releases
  (`CHANGELOG.md` and `dependabot.yml` say why each was held).
- **The OpenAPI document is not complete** (`api.md` says so): eight REST paths
  and two WebSockets are routed but undocumented, and nothing checks that
  direction (the gates check documented-and-routed, and the bearer tables both
  ways). `redocly.yaml` is committed but no workflow runs `redocly lint`, and
  nothing diffs `openapi.json` against the previous release for breaking changes.
