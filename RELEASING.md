# Releasing BirdNet-Behavior

This document is the release runbook. Follow it to cut a new version.

`v0.1.0` shipped on 2026-04-12; everything below applies to `v0.2.0`
and later.

## TL;DR — what a maintainer does

```bash
# 1. Bump the version + roll the changelog (on a branch, via PR).
#    Cargo.toml workspace.package.version = X.Y.Z
#    CHANGELOG.md: [Unreleased] -> ## [X.Y.Z] - YYYY-MM-DD  (+ fresh [Unreleased])
# 2. Get CI green on main.
# 3. (Optional but recommended) Rehearse via the dry run — see below.
# 4. Tag and push:
git checkout main && git pull origin main
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
# 5. Watch Actions, then verify the published artifacts (see "Verifying").
```

That single tag push drives everything else.

## What each workflow emits

Pushing a `vX.Y.Z` tag triggers **two** workflows in parallel:

### `release.yml` — binaries, SBOM, attestation, GitHub Release

```
validate ──► ci ──► build (matrix) ──► package ──► github-release
                                                    (tag push only)
```

| Job | Emits |
|-----|-------|
| `validate` | Confirms the tag is valid semver, that `Cargo.toml` `workspace.package.version` equals the tag, and that a non-empty `## [X.Y.Z]` section exists in `CHANGELOG.md`. Detects `-pre` suffixes and marks the release as a pre-release. |
| `ci` | Full quality gate: `fmt`, `clippy -D warnings`, `test`, Rustdoc (`-D warnings`, private intra-doc links), and an MSRV (Rust 1.95) check. |
| `build` | A release binary per target, built with **`--features analytics`** (DuckDB statically linked in; dormant until `--analytics-db` is passed). Stripped, archived as `.tar.gz` with a per-archive SHA-256. |
| `package` | Combined `SHA256SUMS`; a CycloneDX 1.5 SBOM (JSON + XML); and a **SLSA build-provenance attestation** over the archives and SBOMs, signed via GitHub OIDC. |
| `github-release` | Creates/updates the GitHub Release idempotently, attaches the archives, `SHA256SUMS`, `install.sh`, and SBOMs, and uses the extracted `CHANGELOG` section (plus appended install/Docker/verify notes) as the body. |

**Build targets** (two — there is no armv7 and no musl):

| Target | Platform | Built |
|--------|----------|-------|
| `aarch64-unknown-linux-gnu` | Raspberry Pi 4 / 5 / 400, ARM64 Linux | cross, on the x86_64 runner via the native GCC 13 aarch64 toolchain |
| `x86_64-unknown-linux-gnu` | Standard 64-bit Linux | native |

We build on Ubuntu 24.04 (GCC 13, **glibc 2.39**) because pyke's prebuilt
ONNX Runtime needs glibc ≥ 2.38 and a GCC ≥ 13 libstdc++. We do **not**
use `cargo-zigbuild`: Zig's `-lstdc++` does not provide the full GNU
cxx11-ABI symbol set the pyke archives reference. Consequence: the
binaries require **glibc ≥ 2.39 at runtime** (Pi OS Trixie / Debian 13 /
Ubuntu 24.04). Pi OS **Bookworm** (glibc 2.36) is unsupported by the
native binary — those users should run the Docker image.

`armv7-unknown-linux-gnueabihf` is intentionally omitted: `ort` ships no
prebuilt ONNX Runtime for armv7. Pi 3 / Pi Zero 2 W users should run the
64-bit Pi OS and use the aarch64 binary.

### `docker.yml` — multi-arch container images

Builds a **single image** (DuckDB analytics compiled in, matching the
binaries — no separate `-analytics` tag) for **linux/amd64** and
**linux/arm64**, each natively on a matching runner (no QEMU), pushes by
digest, then merges to one multi-arch manifest tagged
`X.Y.Z` / `X.Y` / `X` / `latest` (and `edge` on `main`, `sha-<rev>` always).
The manifest is **signed with keyless cosign** (GitHub OIDC → Fulcio +
Rekor), with buildx `provenance` and `sbom` attestations attached.

## The shared model release (`models-v3.0-preview3`)

The ~541 MB BirdNET+ V3.0 model + labels are **not** attached to each app
release. They live in one stable, arch-independent GitHub release —
`models-v3.0-preview3` — that every app release's `install.sh` and Docker
entrypoint pull from (sha256-verified), falling back to Zenodo (the upstream
source) when that asset is absent or unreachable. The model is identical across
app versions, so it is uploaded **once**, not re-pushed per patch (which also
keeps each app release lean and provenance focused on the binaries).

Publish or refresh it with the **`publish-model.yml`** workflow (Actions →
*Publish model release* → *Run workflow*). It mirrors the files from Zenodo,
**fails unless their sha256 matches the values pinned in
`installer/lib/10-config.sh`** (so the published asset and the installer's
verification hash can never drift), writes a `SHA256SUMS`, attaches a SLSA
build-provenance attestation, and creates/updates the release idempotently
(marked non-latest so it never shadows the app release's *Latest* badge).

Run it again only when the model file or its pinned checksums change: bump
`MODEL_RELEASE_TAG` + `MODEL_SHA256` + `LABELS_SHA256` in
`installer/lib/10-config.sh` (and the matching constants in
`docker/entrypoint.sh`) first, then dispatch the workflow with the new tag.
A fresh release line whose model release has not been published yet still
installs cleanly — the installer simply falls back to Zenodo until it exists.

## Rehearsing a release (dry run)

`release.yml` has a `workflow_dispatch` entry. Triggering it from the
Actions tab runs `validate → ci → build → package` exactly as a real
release — including the **DuckDB analytics cross-build**, the SBOM, and
the SLSA attestation — but **skips `github-release`, so nothing is
published**. Use it to prove a release will build cleanly before tagging
(especially after dependency bumps or toolchain changes).

- Leave the `version` input blank to rehearse the current
  `Cargo.toml` version, or set it (e.g. `0.2.0`) to rehearse a specific
  one. `validate` still checks `Cargo.toml` and `CHANGELOG.md`, so a
  dry run catches a forgotten version bump or changelog entry too.

## Pre-release checklist

Copy-paste this into the release PR or issue and tick it off:

```text
[ ] workspace.package.version in Cargo.toml bumped to X.Y.Z
[ ] Cargo.lock refreshed (cargo update --workspace)
[ ] CHANGELOG.md: [Unreleased] rolled into ## [X.Y.Z] - YYYY-MM-DD
[ ] CHANGELOG.md: fresh empty [Unreleased] section added
[ ] CHANGELOG.md: link references at the foot updated
[ ] Local gate: cargo fmt --check --all
[ ] Local gate: cargo clippy --workspace --all-targets -- -D warnings
[ ] Local gate: cargo test --workspace
[ ] Local gate: RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
[ ] Version-bump PR merged to main; CI green on the merge commit
[ ] Dry run (workflow_dispatch on release.yml) is green
[ ] git tag -a vX.Y.Z -m "Release vX.Y.Z" && git push origin vX.Y.Z
[ ] Post-publish verification done (binaries, Docker, attestation, SBOM)
```

## Verifying a published release

```bash
# Checksums
sha256sum -c SHA256SUMS --ignore-missing

# SLSA build provenance on a binary archive
gh attestation verify \
  --repo tomtom215/BirdNet-Behavior \
  birdnet-behavior-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz

# Docker image signature (keyless cosign)
cosign verify \
  --certificate-identity-regexp '^https://github.com/tomtom215/BirdNet-Behavior/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  ghcr.io/tomtom215/birdnet-behavior:X.Y.Z
```

## Pre-releases

Tags of the form `vX.Y.Z-<suffix>` (e.g. `v0.2.0-rc.1`) are detected by
`validate`, which marks the GitHub Release as a pre-release. Docker still
publishes versioned tags for them but does **not** move `latest`.

## What is NOT automated

These are deliberate manual / out-of-scope steps — know them so a release
isn't half-done:

- **Version bump + changelog roll.** `validate` *checks* that
  `Cargo.toml` and `CHANGELOG.md` match the tag, but you make the edits
  (see the checklist). There is no auto-bump.
- **Tag creation/push.** The pipeline is tag-triggered; creating and
  pushing the tag is the manual "go" action.
- **Docs versioning + tag-time rebuild.** The mdBook documentation site is
  **"latest tracks `main`"** — it is not snapshotted per release. `docs.yml`
  rebuilds and deploys it on pushes to `main` that touch `docs/**`. There is
  no tag-triggered docs build, and it is not needed: a release is cut from
  `main`, so the live docs already reflect the released commit (the docs
  change deployed when its PR merged). Old versions are not archived.
  (Rationale: a single small appliance project; one accurate "current"
  manual beats N drifting snapshots. Revisit if the config/schema surface
  starts changing incompatibly between releases.)
- **Crates.io publishing.** Not published to crates.io (this is an
  application, not a library).
- **The shared model release.** `publish-model.yml` is a manual, one-shot
  workflow run per *model* version, not per app release — see "The shared model
  release" above. A normal app release never touches it.

## Troubleshooting

- **`validate` fails: "version does not match tag".**
  `workspace.package.version` must equal the tag minus the leading `v`.
  Fix the commit, re-tag.
- **`validate` fails: "no CHANGELOG entry" / "empty release notes".**
  Add a non-empty `## [X.Y.Z]` heading to `CHANGELOG.md` and re-push.
- **`build` fails on the aarch64 analytics link.** The bundled libduckdb
  is a large C++ static archive; the cross build links it on the 16 GB
  x86_64 runner. If it regresses, rehearse with the dry run and, if
  needed, fall back to building aarch64 natively on `ubuntu-24.04-arm`
  (as `docker.yml` does) — at the cost of relaxing LTO to fit the 8 GB
  arm runner.
- **The release already exists and needs regenerating.** Delete the
  GitHub Release (keep the tag), then re-run the workflow from the
  Actions tab. `github-release` is idempotent and re-creates it.
- **Docs didn't update after merging a docs change.** Check the
  `docs.yml` run on `main`; Pages must be enabled (Settings → Pages →
  Source: GitHub Actions) once for the repo.
