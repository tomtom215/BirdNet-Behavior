# Releasing BirdNet-Behavior

This document is the release runbook. Follow it to cut a new version.

## Overview

Releases are driven entirely by pushing a semver tag (`vX.Y.Z`). The
`.github/workflows/release.yml` workflow validates the tag, runs the
full quality gate, cross-compiles release binaries for three
architectures, generates a SLSA build provenance attestation, and
publishes a GitHub Release with the binaries, checksums, and release
notes extracted from `CHANGELOG.md`.

A separate workflow, `.github/workflows/docker.yml`, runs in parallel
on the same tag push and publishes multi-architecture container images
to GHCR.

## Pre-flight checklist

1. **Bump the workspace version.** Edit `Cargo.toml` and update
   `workspace.package.version` to the new version. Run `cargo build`
   or `cargo check --workspace` to refresh `Cargo.lock`.

2. **Update `CHANGELOG.md`.** Convert the `[Unreleased]` section into a
   dated version heading and start a fresh `[Unreleased]` block above
   it. The release workflow will extract release notes from the
   section matching the tag version.

   ```markdown
   ## [Unreleased]

   ## [0.2.0] - 2026-05-01

   ### Added
   - ...
   ```

   Update the link references at the bottom of the file.

3. **Run the full quality gate locally.**

   ```bash
   cargo fmt --check --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
   ```

4. **Commit the version bump and changelog** on a release branch, open
   a pull request, and merge it to `main` once CI is green.

## Cutting the release

From the merged commit on `main`:

```bash
git checkout main
git pull origin main

# Use an annotated tag — `release.yml` triggers on tags matching
# v[0-9]+.[0-9]+.[0-9]+ or v[0-9]+.[0-9]+.[0-9]+-*
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

The tag push triggers `release.yml`, which runs these jobs:

| Job | Purpose |
|-----|---------|
| `validate` | Validate semver tag, verify `Cargo.toml` version matches the tag, verify a `CHANGELOG.md` entry exists for the version |
| `ci` | Full quality gate — `fmt`, `clippy`, `test`, `doc`, and MSRV check |
| `build` | Cross-compile release binaries for `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`, and `armv7-unknown-linux-gnueabihf` via `cargo-zigbuild`, strip debug symbols, and archive as `.tar.gz` with per-archive SHA-256 |
| `package` | Aggregate all build artifacts, generate a combined `SHA256SUMS` file, and produce a SLSA build provenance attestation signed via GitHub OIDC |
| `github-release` | Create or update the GitHub Release idempotently, attach the archives, `SHA256SUMS`, and `install.sh`, and extract release notes from `CHANGELOG.md` |

If a release for the tag already exists (for example, created by a
different tool while testing), the workflow updates it in place rather
than failing.

## Pre-releases

Pre-release tags follow the pattern `vX.Y.Z-<suffix>`, for example
`v0.2.0-rc.1`. The workflow detects the suffix, marks the GitHub
Release as a pre-release, and publishes it alongside stable releases.

## Verifying a release

Every release artifact is covered by a SLSA build provenance
attestation. End users can verify the attestation against a binary
archive with the GitHub CLI:

```bash
gh attestation verify \
  --repo tomtom215/BirdNet-Behavior \
  birdnet-behavior-aarch64-unknown-linux-gnu.tar.gz
```

Checksums can be verified against `SHA256SUMS`:

```bash
sha256sum -c SHA256SUMS --ignore-missing
```

## Troubleshooting

- **`validate` fails with "version does not match tag".** The
  `workspace.package.version` field in `Cargo.toml` must equal the tag
  minus the leading `v`. Fix the commit, tag again.
- **`validate` fails with "no CHANGELOG entry".** Add a
  `## [X.Y.Z]` heading to `CHANGELOG.md` and re-push the tag.
- **`build` fails on a single architecture.** The Zig linker used by
  `cargo-zigbuild` occasionally regresses on minor versions. Pin a
  known-good Zig version in the `Install Zig` step of
  `.github/workflows/release.yml`.
- **The release already exists and needs to be regenerated.** Delete
  the GitHub Release (keep the tag), then re-run the workflow from the
  Actions tab. The `github-release` job is idempotent and will recreate
  the release and re-upload assets.
