# Workflows

What each workflow in this directory does, when it runs, and which of its checks
a pull request must pass. `scripts/check-ci-config.py` (the `CI config` job in
`mutation.yml`) reads this file: every workflow must have a row, and every name
under *Required status checks* must be a job that exists and runs on every pull
request.

| File | Runs on | What it is for |
|---|---|---|
| `ci.yml` | every push and pull request | formatting, clippy (pedantic + nursery), tests on x86_64 and native aarch64, the real-model inference test, rustdoc, a debug build with all features, the MSRV check, the aarch64 cross-check |
| `supply-chain.yml` | every push and pull request, and Mondays | cargo-deny, cargo-audit, unused dependencies, third-party licence notices, spelling, shellcheck, the hermetic installer tests |
| `coverage.yml` | every push and pull request | line coverage with cargo-llvm-cov; informational |
| `codeql.yml` | every push and pull request, and Tuesdays | CodeQL (`security-extended`) over Rust, the workflows themselves, the JavaScript the UI serves, and the Python gate scripts; results in the Security tab |
| `fuzz.yml` | pull requests touching the audio decoder, the label parser or `fuzz/**`; Wednesdays | libFuzzer on the two parsers of untrusted input (the audio decoder and the label-file parser), with production arithmetic; 60 seconds a target on a pull request, 10 minutes a week |
| `osv.yml` | every push and pull request, and daily | OSV-Scanner over `Cargo.lock` and the browser-QA lockfile; any finding fails the job |
| `scorecard.yml` | pushes, branch-protection changes, and Mondays | the OpenSSF Scorecard of the repository, published and uploaded to code scanning; a dashboard, not a gate |
| `a11y.yml` | pull requests touching `crates/birdnet-web/**`, `tools/visual-qa/**`, or this file | axe accessibility, the interaction gate, the chart-label legibility gate, the base-path gate, the visual-QA sweep, in a real browser at desktop and phone size |
| `docker.yml` | pull requests touching `Dockerfile`, `.dockerignore`, `docker/**`; pushes except ones that touch only `docs/**`, Markdown, `ci.yml` or `release.yml`; version tags | the multi-arch image, booted with networking off to assert the extensions load; signed on release |
| `docs.yml` | pull requests touching `docs/book/**`, the book theme or its scripts; pushes to `main`; version tags | builds the versioned manual, link-checks every tree, deploys it to GitHub Pages |
| `install-smoke.yml` | pull requests touching `install.sh`, `installer/lib/**`, `src/**`, `crates/**` or the manifests | runs `install.sh` in a container with no systemd and no network and loads the web UI |
| `upgrade.yml` | pull requests touching the installer, `src/**`, `crates/**` or the manifests | upgrades from the previous releases on a real systemd host, then rolls back |
| `mutation.yml` | pull requests touching the mutation-tested paths or `.github/workflows/**`; Mondays | the CI-config checks, then cargo-mutants on the paths that decide what a station records |
| `release.yml` | version tags | validates the tag, re-runs the gates, builds each target, attests and packages, upgrades from the previous releases, then publishes and verifies the GitHub Release |
| `publish-model.yml` | manual | mirrors the model files to the shared model release, failing unless every sha256 matches the pins in `installer/lib/10-config.sh` |

## Required status checks

`main` should require these checks. They are the jobs of the workflows that run
on **every** pull request, with their names exactly as GitHub shows them.

- `Format (rustfmt)`
- `Clippy (pedantic + nursery)`
- `Tests (x86_64)`
- `Inference against the real model`
- `Rustdoc (no broken links)`
- `Build (debug, all features)`
- `MSRV (Rust 1.95)`
- `Cross-check (aarch64 / Raspberry Pi)`
- `Tests (aarch64, native)`
- `cargo-deny`
- `cargo-audit`
- `cargo-machete (unused dependencies)`
- `third-party licence notices`
- `Spelling (typos)`
- `shellcheck (bootstrap scripts)`
- `installer unit tests`

`codeql.yml` and `osv.yml` also run on every pull request and could join the
list once they have run clean on `main`; they are new and have not been run.

Two things about this list:

- A workflow skipped by a `paths:` filter leaves its checks *pending*, and a
  pending required check blocks the merge forever. So `a11y`, `docker`, `docs`,
  `install-smoke`, `upgrade` and `mutation` cannot be required as they are: they
  would need either an always-run job that depends on them or no path filter.
  Until then they are advisory, and a reviewer has to look at them when their
  paths change.
- A required check is matched by **name**. Renaming a job in the workflow
  silently drops the requirement. The `CI config` check fails when a name in
  this list is not a job name in a workflow that runs on every pull request, so
  rename a job and this file together.

Whether `main` actually enforces any of this is a repository setting, not a
file; see `docs/HANDOFF.md`.
