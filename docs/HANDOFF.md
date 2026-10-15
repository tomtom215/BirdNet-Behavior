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
  derives from BirdNET-Pi; `deny.toml` still says the project "is not published
  to crates.io". Whether to publish, under which crate names, and who holds the
  account is not something the repository can decide. See section 4 for what is
  and is not ready.
- **Release signing.** Tags and `SHA256SUMS` are unsigned; the in-app updater
  trusts the GitHub release. A signing key and a verifier in the updater would
  close that, and the key is yours to create.

## 3. What the last session changed, and how far it was checked

Everything is on branch `claude/vigilant-goldberg-grdjk1`. **No pull request has
been opened, so no GitHub Actions workflow has run on any of it**; the
`pull_request`-triggered gates never see an un-PR'd branch. The first thing to
do is open a draft pull request and read what CI says.

Checked locally, by running it (details in each commit message):

- the whole Rust workspace: `fmt`, `clippy --all-targets --all-features -D
  warnings`, library and binary tests, every integration test of `birdnet-web`
  and of the root crate, doc tests, `cargo doc` with warnings denied;
- the browser gates (axe, interactions, base path, visual sweep, chart-label
  legibility) against the CI screenshot fixture;
- `actionlint` on every workflow, the hermetic installer test suite, and the
  parts of `scripts/check-ci-config.py` that do not need `cargo-mutants` or a
  token;
- the extension pins against freshly downloaded extension builds, and the
  licence generator against the real dependency tree.

**Not run, anywhere:** any workflow on GitHub; the Docker build and its offline
boot test; the release workflow; `cargo-mutants`; `typos`; the container image's
new `/usr/share/doc/birdnet-behavior/` files; anything on Raspberry Pi hardware
(the cgroup memory-limit fix in `memory.rs` is tested against a directory tree
laid out as the kernel lays it out, not against a unit under cgroup v2).

## 4. Where to start

1. Open a draft PR from `claude/vigilant-goldberg-grdjk1` and fix what CI finds.
   Expect the `third-party licence notices`, `CI config` and `a11y` jobs to be
   the first to run something new.
2. Do the settings in section 1.
3. Read the open items below; each is real and none was started unless it says so.

## 5. Open items

Not done, in the order they seem to matter:

- **crates.io readiness.** The workspace crates are not publishable as they stand
  (path dependencies without versions, no `include` lists, no per-crate READMEs);
  a `cargo package` dry run has not been done.
- **Feature parity.** The gap analysis against birdnet-go and BirdNET-Pi is in
  [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md); species-name translation
  (`DATABASE_LANG`) was left alone because its data licence and its effect on
  species identity need a decision first.
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
