# Releasing BirdNet-Behavior

This document is the release runbook. Follow it to cut a new version. Nothing
reads it mechanically, so when a workflow changes, read the section that names it.

## TL;DR — what a maintainer does

```bash
# 0. Refresh the lockfile and re-check advisories against the result.
#    Dependabot only proposes bumps for *declared* dependencies; the lockfile
#    is what actually ships, and it drifts silently. (Measured once: 150
#    packages behind, including rustls, hyper, aws-lc-rs and webpki-roots.)
#    NOTE: plain `cargo update`, NOT `cargo update --workspace` — the latter
#    restricts the relock to workspace members, so it refreshes no third-party
#    dependency, which is the entire point of this step. (It reports
#    "0 packages" only when the workspace versions have not moved; run right
#    after a version bump it relocks each member, which is step 1's job, not
#    this one. Observed with cargo 1.98.1.)
#    Read what it moved, and re-run the whole gate on the result, not just the
#    advisories: on 2026-10-20 `cargo update --dry-run` moved `ort` and `ort-sys`
#    from 2.0.0-rc.14 to 2.0.0-rc.15 (the inference engine itself, a release
#    candidate whose requirement is `^2.0.0-rc.14`, so every refresh takes the
#    newest candidate) and `tokio` from 1.53.3 to 1.54.0. The CI job `Inference
#    against the real model` is the one that tells you the new ort still runs the
#    model, and a refresh like this is easier to bisect on a pull request of its
#    own than inside the version bump.
cargo update
cargo deny check advisories        # or: cargo audit
cargo test --workspace --all-features
# 1. Bump the version + roll the changelog (on a branch, via PR).
#    Cargo.toml workspace.package.version = X.Y.Z
#    Cargo.toml [workspace.dependencies]: `version` of the eight `birdnet-*` entries
#                (release validate runs `cargo package --workspace` and fails if
#                they disagree with the crates' version)
#    CITATION.cff:  version + date-released   (release validate enforces this)
#    crates/birdnet-web/openapi.json: info.version  (release validate enforces this)
#    CHANGELOG.md: [Unreleased] -> ## [X.Y.Z] - YYYY-MM-DD  (+ fresh [Unreleased])
# 2. Get CI green on main.
# 3. Rehearse via the dry run — see below. It is the only workflow that builds the
#    aarch64-apple-darwin target (no pull-request job does); v0.18.0's first tag failed
#    there, after the rest of the release had passed.
# 4. Tag and push:
git checkout main && git pull origin main
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
#    Tag and push — do NOT create the Release in the GitHub web UI. Publishing
#    a draft there creates the tag as a side effect, which triggers this same
#    workflow against a Release that already exists and already has a body.
#    That is how v0.16.0 shipped: the auto-generated notes were left in place
#    and the CHANGELOG body that should have replaced them was rejected.
# 5. Watch Actions, then verify the published artifacts (see "Verifying").
```

That single tag push drives everything else.

## Publishing to crates.io (the maintainer, not a workflow)

Nothing publishes to crates.io automatically, and the project has never been
published; the nine names (`birdnet-behavior` and the eight `birdnet-*` library
crates) were unclaimed on 2026-10-15. What is ready: every crate packages
(`cargo package --workspace`, run by the `crates are publishable` job and by
`validate`), every package is under 3 MB compressed (the largest is `birdnet-web`, 2.6 MiB;
`cargo package` reports it, and crates.io's limit is 10 MiB), the internal
dependencies carry versions, every tarball carries `LICENSE`, `LICENSE-UPSTREAM` and `NOTICE` and the
workspace `homepage` (`scripts/check-crate-licences.py` reads the tarballs in both
of those places), and `cargo package --workspace --no-default-features` also
*builds* all nine from their tarballs (it was run once; the bundled-DuckDB build
was not).

The licence files in the eight library crates are symlinks to the ones at the
repository root, so there is one copy to edit; `cargo package` archives the file a
link points at. A checkout that drops symlinks (`core.symlinks=false`, a zip
download) would package a 13-byte text file instead, which is why the check reads
the tarballs and not the tree.

To publish, after the tag is pushed and the release is verified, from the tagged
commit, with a crates.io token in `CARGO_REGISTRY_TOKEN`:

```bash
cargo publish --workspace --locked   # dependency order is worked out by cargo
```

**Not in one go: crates.io rate-limits *new* crates** to a burst of five and then
one more every ten minutes (`PublishNew` in `rust-lang/crates.io`'s
`src/rate_limiter.rs`: burst 5, refill 10 minutes; read there on 2026-10-20), and
these are nine new names. The sixth upload of a single `--workspace` run is
therefore refused (HTTP 429; not tried here, since nothing has been published).
Publish in two batches along the dependency layers, which `cargo metadata` gives as
five crates that depend on no other workspace crate and four that depend on them:

```bash
# batch 1: uses the whole burst
cargo publish --locked -p birdnet-core -p birdnet-db -p birdnet-behavioral \
    -p birdnet-timeseries -p birdnet-scheduler
# batch 2: wait until four tokens have refilled (about 40 minutes after batch 1)
cargo publish --locked -p birdnet-integrations -p birdnet-migrate -p birdnet-web \
    -p birdnet-behavior
```

Within a batch `cargo` publishes in dependency order. A crate that is refused is
simply not on the registry: name the missing ones with `-p` and run again once the
limit has refilled, rather than re-running a command that names crates already
uploaded (a published version cannot be uploaded twice).

Publishing cannot be undone (a version can only be yanked), so look first at what
it would do (`cargo publish --workspace --dry-run`) and at these:

- **The licence is CC BY-NC-SA 4.0**, which forbids commercial use. crates.io
  accepts the SPDX identifier; whether a non-commercial licence belongs on a
  registry that most consumers read as open source is yours to decide.
- **The library crates are internal.** Their READMEs say so and `STABILITY.md`
  promises nothing about their API; they are published only so the binary can be.
- **`cargo install birdnet-behavior --locked`, say so wherever it is documented.**
  `cargo install` ignores the `Cargo.lock` shipped in a crate unless `--locked` is
  passed, and `ort = "2.0.0-rc.14"` is the requirement `^2.0.0-rc.14`: a release candidate
  that accepts every later candidate (and 2.0.0 itself). Without `--locked` an
  install resolves to the newest `ort` of the day, which this project has not run,
  while the tarball's `Cargo.lock` names the one it has. The same is true of every
  other dependency, but `ort` is the one whose requirement reaches across
  pre-releases of the inference engine.
- **`cargo install birdnet-behavior` builds the bundled DuckDB** (minutes of C++)
  and downloads ONNX Runtime. A package build does not render the in-app manual
  (`build.rs` skips it: a build script may not write into the registry source), so
  `/help` shows its "docs unavailable" panel until `BNB_HELP_DIR` points at a
  rendered copy such as a release archive's `help/`. It also embeds neither DuckDB
  extension (the release workflow fetches `behavioral` and `icu` and sets
  `BIRDNET_BUNDLED_EXTENSION_FILE` / `BIRDNET_BUNDLED_ICU_FILE`; a package build
  has no such files), so the first analytics query `INSTALL`s them into
  `~/.duckdb`: it needs the network once and a writable home, and under the
  shipped systemd unit (`ProtectHome=read-only`) it would fail the way v0.13.1 did.
  `README.md` says this to the reader; `--doctor` still describes the extension as
  "embedded in release binaries" on such a build.
- **docs.rs** builds the two crates whose default features pull in the bundled
  engine without it (`no-default-features` in their `[package.metadata.docs.rs]`);
  that was reasoned from docs.rs's offline builds, not tried (one part was read:
  `ort-sys`'s build script returns early when `DOCS_RS` is set, in
  `build/main.rs`). Those two crates, and `birdnet-behavioral` and
  `birdnet-timeseries`, whose `analytics` feature is off by default, therefore
  show a reduced API there. Each library crate's
  `documentation` is `https://docs.rs/<name>`, the binary's is the operator manual,
  and all nine share the manual as `homepage`.
- **`birdnet-core`'s tarball contains the bird-name table** (`data/l18n/`:
  `species_names.tsv`, `SOURCES.tsv` and a README; no other tarball does, the binary
  gets it through the `birdnet-core` dependency). Its provenance and the open
  question about some of the names are in that README and in `docs/HANDOFF.md`.

## When ONNX Runtime, DuckDB or htmx moves

`THIRD_PARTY_LICENSES.txt` carries the licence texts the binary's third-party
code requires, and release archives and the image ship it with `NOTICE`. It is
generated (`python3 scripts/gen-third-party-licenses.py`, after
`cargo fetch --locked`) from `Cargo.lock` and from the directories in
`third_party/`, which are named for the version they belong to. The `third-party
licence notices` job and the release `validate` job fail when the committed file
is stale, when any package's licence text cannot be found, or when the
ONNX Runtime, DuckDB or htmx the build links is not the version a
`third_party/` directory is for. `third_party/README.md` says how to replace one.
A new or removed dependency needs only a regeneration.

## When DuckDB or an extension moves

The `duckdb` pin in the root `Cargo.toml` decides which engine ships, and the
`behavioral` and `icu` extensions embedded in the binary must be the builds made
for it. The engine version `v1.5.6` is written out in four files — three fetch
URLs in `ci.yml`, two in `upgrade.yml`, the two `ddb_ver` lines in `release.yml`
and `BEHAVIORAL_EXTENSION_DUCKDB_VERSION` in the `Dockerfile` — and
`crates/birdnet-behavioral/extension_pins.rs` pins the sha256 of each build
(`build.rs` refuses to embed any other). Update them together:

1. Change the version in those places.
2. Download each build, load it into a stock DuckDB of the same version with
   unsigned extensions disallowed (that is the signature check), and replace the
   rows in `extension_pins.rs`; its header gives the steps and says which
   platforms could and could not be checked that way.
3. `cargo test -p birdnet-behavioral --all-features --lib extension_pins`, then
   a build with `BIRDNET_BUNDLED_EXTENSION_FILE` and `BIRDNET_BUNDLED_ICU_FILE`
   set to the new files. A file the table does not name stops the build.

Release and Docker builds fail when an extension cannot be fetched; the Docker
build takes `--build-arg BEHAVIORAL_EXTENSION_OPTIONAL=1` to build without them.

## What each workflow emits

Pushing a `vX.Y.Z` tag triggers **three** workflows in parallel (two for a
pre-release tag: `docs.yml` builds stable tags only):

### `release.yml` — binaries, SBOM, attestation, GitHub Release

```
validate ─┬─► ci ──► build (matrix) ─┬─► package ───────────────┐
          │                          │                          ├─► github-release ─► verify-release
          └─► upgrade-sources ───────┴─► upgrade (one per row) ─┘   (tag push only)   (tag push only)
```

`github-release` needs `package` **and every `upgrade` row**, so a release that
cannot be updated to is not published.

| Job | Emits |
|-----|-------|
| `validate` | Confirms the tag is valid semver (see "Tag form" below the table) and that four files agree with it: `Cargo.toml` `workspace.package.version`, `CITATION.cff` `version` (the step also tells you to bump `date-released`), `crates/birdnet-web/openapi.json` `info.version`, and a non-empty `## [X.Y.Z] - YYYY-MM-DD` section in `CHANGELOG.md` (the heading must carry its date). On a tag push it also requires the tag to be **annotated** (`git tag -a`; a lightweight tag is refused) and its commit to be in the default branch's history. Detects `-pre` suffixes and marks the release as a pre-release. It also checks, before any build starts, that `Cargo.lock` is in step with the manifests (`cargo update --workspace --locked`), that every crate packages for crates.io (`cargo package --workspace --locked --no-verify`) with `LICENSE`, `LICENSE-UPSTREAM` and `NOTICE` in each tarball, and that `THIRD_PARTY_LICENSES.txt` is current. Also reports the `CHANGELOG` section's size and warns, without failing, when it is large enough to be truncated in the Release body. |
| `ci` | Full quality gate: `fmt`, `clippy -D warnings`, `test`, Rustdoc (`-D warnings`, private intra-doc links), and an MSRV (Rust 1.95) check. |
| `build` | A release binary per target, built with **`--features analytics`** (DuckDB statically linked in and on by default, in `<database>.duckdb` beside the SQLite file; `--analytics-db ""` turns it off). Stripped, archived as `.tar.gz` with a per-archive SHA-256. |
| `package` | Combined `SHA256SUMS`; a CycloneDX 1.5 SBOM (JSON + XML); and a **SLSA build-provenance attestation** over the archives, the SBOMs, `install.sh`, `uninstall.sh` and `SHA256SUMS`, signed via GitHub OIDC. |
| `upgrade-sources` | Picks the four newest published releases older than this one (`scripts/upgrade-sources.sh`: not drafts, not pre-releases, and only those that carry `install.sh`, `SHA256SUMS` and an x86_64 archive) as the rows of `upgrade`. Fails if there is none. |
| `upgrade` | One row per release picked (`Upgrade from vA.B.C`, `fail-fast: false` so every row reports): installs that published release under real systemd, records detections, updates in place to **this build's** x86_64 archive, then rolls back with the command the update printed (`installer/test/upgrade-e2e.sh`). 0.17.0 was published without this check and broke stations that updated in place. |
| `github-release` | Needs `package` and every `upgrade` row. Creates/updates the GitHub Release idempotently, **attaches the archives, `SHA256SUMS`, `install.sh`, `uninstall.sh` and SBOMs first**, then sets the body from the extracted `CHANGELOG` section plus appended install, Docker, verify and (for a stable release) documentation notes, the last linking that version's own manual at `/vX.Y.Z/`. GitHub caps a release body at 125,000 characters, so an oversized section is truncated at a `###` heading boundary and linked to `CHANGELOG.md` for the remainder. The upload deliberately precedes the body edit: in v0.16.0 the order was reversed, a 270,796-character body was rejected with HTTP 422, and `bash -e` killed the step before a single binary was attached. |
| `verify-release` | Reads the published Release back and refuses to call it done unless it is usable: not a draft, body not still the pre-upload placeholder, and every file the published `SHA256SUMS` names downloadable **unauthenticated** from the public URL and matching its checksum, and each of them (and `SHA256SUMS`) carrying a valid build-provenance attestation from this repository (`gh attestation verify`). Driven by the manifest, not a hard-coded list, so a new build target is covered as soon as it joins the matrix. |

**Tag form.** A release tag is `vX.Y.Z`, or `vX.Y.Z-<suffix>` for a pre-release.
`validate`'s regex also accepts SemVer *build metadata* (`1.2.3-rc.1+b5`), but the
image workflow's gate (`scripts/check-release-tag.sh`) does not, and a Docker tag
cannot contain `+`: such a tag would publish a GitHub Release with archives and no
image. (Read from the two regular expressions, which accept and refuse it as
described, and from the tag filters, which as GitHub defines them let a `+` reach
`release.yml` only after a `-suffix`; no such tag was pushed.) Do not put build
metadata in a release tag.

**Build targets** (three — there is no armv7 and no musl):

| Target | Platform | Built |
|--------|----------|-------|
| `aarch64-unknown-linux-gnu` | Raspberry Pi 4 / 5 / 400, ARM64 Linux | cross, on the x86_64 runner via the native GCC 13 aarch64 toolchain |
| `x86_64-unknown-linux-gnu` | Standard 64-bit Linux | native |
| `aarch64-apple-darwin` | macOS on Apple Silicon | native, on the `macos-14` runner |

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

A tag push first passes `scripts/check-release-tag.sh` (an exact `vX.Y.Z` or
`vX.Y.Z-pre.N`, a `Cargo.toml` that agrees, a commit on the default branch), because
this workflow runs beside `release.yml`, not after it, and its tag glob also matches
`v1.2.3foo`. `latest` is never applied to a pre-release tag. A manual run
(`workflow_dispatch`) **builds only** unless you tick *Push*, because a push from a
branch is signed with that branch's identity and verifies against the documented
recipe.

### `docs.yml` — the versioned manual on GitHub Pages

Rebuilds the whole site on every run with `scripts/build-versioned-docs.sh`: the
root is the latest stable release's manual, which is also kept at `/vX.Y.Z/`;
earlier versioned releases are rebuilt beside it, each from its own tag; and
`/dev/` is `origin/main` (a pull request's run builds the pull request's own tree
instead, to gate it, and deploys nothing). Every tree is link-checked. Only
`/dev/` pages carry "Suggest an edit": a release cannot be edited, and a link to a
page later renamed on `main` would fail the link check for an unrelated reason. A
stable release's GitHub Release body links its own `/vX.Y.Z/`. Nothing to do by
hand — see "What is NOT automated" for what this means for a docs fix.

What "a manual per release" does and does not promise today:

- **It does not wait for the release.** `docs.yml` is triggered by the stable tag
  push itself, not by `release.yml`. Two things are refused first, by
  `scripts/check-release-tag.sh` (the gate `docker.yml` also applies): a tag on a
  commit that is not on `main`, and a tag whose `Cargo.toml` names another version.
  What is *not* refused is a tag that `validate` rejects for another reason (an
  unannotated tag, a missing changelog heading) or whose build, upgrade or publish
  jobs later fail: that tag still moves the site root to its manual. To take it
  back, delete the tag and run *Docs* from the Actions tab: the site is rebuilt
  from the tags that exist.
- **The built site is loaded in a browser before it deploys.**
  `scripts/check-docs-site.mjs` checks the version switcher on the root, on every
  version and on `/dev/`, each page's banner, switching between versions, the 404
  page for an unknown version, search results, and the phone menu bar. It was
  watched failing against the switcher assets from before it existed.
- **It is rebuilt, not archived.** Every run re-renders every release from its
  tag with today's tooling: the mdBook version pinned in `docs.yml`, the version
  switcher and accessibility assets, and `check-book-links.py` from the checkout.
  A page's words are the tag's; how it renders, and whether it passes the link
  check, are today's. A tooling change that an old tag's pages fail makes *every*
  deploy fail, and a tag cannot be edited: fix the tooling, or raise
  `FIRST_VERSIONED_RELEASE` in the script to drop that version from the site.
- **Pre-releases have no manual.** A `vX.Y.Z-rc.N` tag neither triggers
  `docs.yml` nor is rendered by it, and its Release body carries no docs link.
  Nothing before `v0.17.1` is versioned (those binaries' own `/help` is their
  manual).
- **A mistake in a released manual stays there.** It is fixed in `/dev/` at once
  and at the root with the next release; correcting `/vX.Y.Z/` itself means a new
  patch release.

## The shared model release (`models-v3.0-preview3`)

The ~541 MB BirdNET+ V3.0 model + labels are **not** attached to each app
release. They live in one stable, arch-independent GitHub release —
`models-v3.0-preview3` — that every app release's `install.sh` and Docker
entrypoint pull from (sha256-verified), falling back to Zenodo (the upstream
source) when that asset is absent or unreachable. The model is identical across
app versions, so it is uploaded **once**, not re-pushed per patch (which also
keeps each app release lean and provenance focused on the binaries).

The same release also carries the **BirdNET geomodel** (~14 MB) and its label
file — the species occurrence filter, mirrored from
[`birdnet-team/geomodel`](https://github.com/birdnet-team/geomodel). It pairs
with this classifier, so it rides in the same release rather than making a
station reach a second host.

Publish or refresh it with the **`publish-model.yml`** workflow (Actions →
*Publish model release* → *Run workflow*). It mirrors the classifier from
Zenodo and the geomodel from its upstream release, **fails unless every
sha256 matches the values pinned in `installer/lib/10-config.sh`** (so the
published assets and the installer's verification hashes can never drift),
writes a `SHA256SUMS`, attaches a SLSA build-provenance attestation over all
four files, and creates/updates the release idempotently (marked non-latest so
it never shadows the app release's *Latest* badge).

Run it again only when a model file or its pinned checksums change: bump
`MODEL_RELEASE_TAG` + `MODEL_SHA256` + `LABELS_SHA256` (classifier) or
`GEOMODEL_VERSION` + `GEOMODEL_FILE` + `GEOMODEL_SHA256` +
`GEOMODEL_LABELS_SHA256` (geomodel) in `installer/lib/10-config.sh` — and the
matching constants in `docker/entrypoint.sh` — first, then dispatch the
workflow.

A release line whose model release has not been published yet still installs
cleanly. Each family has its own fallback origin and they degrade differently,
by design:

| Asset | Primary | Fallback | If both fail |
|---|---|---|---|
| Classifier + labels | this release | Zenodo | install **aborts** — a station without a classifier detects nothing |
| Geomodel + labels | this release | `birdnet-team/geomodel` | install **continues** with a warning; occurrence filtering is off, and `--doctor` says so |

> **Until this release carries the geomodel**, every install falls through to
> the upstream origin on the first fetch and succeeds — it is a fallback, not a
> failure. Re-running the workflow simply moves that fetch back onto our host.

## The V2.4 mirror (`models-v2.4`)

BirdNET V2.4 and its range filter v2 are fetched by the binary
(`birdnet-behavior --install-model birdnet-v24`, `birdnet-v24-range-v2`) from the
origins pinned in `crates/birdnet-integrations/src/model_catalog.rs`: Hugging Face
for the model, `raw.githubusercontent.com` for the range filter. The
**`publish-model.yml`** workflow with `classifier` set to `birdnet-v24` copies both
into the release `models-v2.4` of this repository. It downloads from the pinned
URLs, **fails unless sha256 and byte count match the catalogue**, writes
`SHA256SUMS`, attaches a SLSA attestation and creates or updates the release
(non-latest). The labels are not mirrored: the binary carries BirdNET-Pi's English
names for V2.4's classes.

It was run once, on 2026-10-20, from a feature branch. **Nothing fetches from the
release yet**: the catalogue holds one URL per file. Using it means giving
`CatalogEntry` a mirror-first, upstream-fallback pair and making
`fetch_unless_installed` try them in order with the same digest check on either.
Re-run the workflow from `main` once the branch that carries the `classifier` input
has merged, so the attestation names `main`, and again only when a pinned digest in
the catalogue changes.

## Rehearsing a release (dry run)

`release.yml` has a `workflow_dispatch` entry. Triggering it from the
Actions tab runs `validate → ci → build → package` exactly as a real
release — including the **DuckDB analytics cross-build** and the SBOM — and the
`upgrade-sources` and `upgrade` jobs too, so every `Upgrade from vA.B.C` row runs
against the build it just made. It **skips the SLSA attestation (a published claim),
`github-release` and `verify-release`, so nothing is published**. Use it to prove a
release will build cleanly, and can be updated to, before tagging (especially after
dependency bumps or toolchain changes).

- Leave the `version` input blank to rehearse the current
  `Cargo.toml` version, or set it (e.g. `0.2.0`) to rehearse a specific
  one. `validate` still checks `Cargo.toml`, `CITATION.cff`, `openapi.json`
  and `CHANGELOG.md`, so a dry run catches a forgotten bump in any of the
  four, or a missing changelog entry, too.

## Pre-release checklist

Copy-paste this into the release PR or issue and tick it off:

```text
[ ] workspace.package.version in Cargo.toml bumped to X.Y.Z
[ ] CITATION.cff: version bumped to X.Y.Z and date-released set to the
    release date  (gated by release validate — the tag fails without it)
[ ] crates/birdnet-web/openapi.json: info.version bumped to X.Y.Z  (gated by release validate)
    (a test asserts it tracks CARGO_PKG_VERSION, so a missed bump fails
     the suite rather than shipping a spec that lies about its version)
[ ] docs/book/reference/api.md: the sample /api/v2/health response
[ ] Cargo.lock refreshed (cargo update — NOT --workspace, see the TL;DR)
[ ] Cargo.lock's own workspace entries moved to X.Y.Z. `cargo update --workspace`
    does exactly this and nothing else — useful for a patch release that should
    not carry a dependency refresh. Confirm the diff is only the member version
    lines before committing.
[ ] CHANGELOG.md: [Unreleased] rolled into ## [X.Y.Z] - YYYY-MM-DD
[ ] CHANGELOG.md: the new section is under about 119,000 bytes (`validate` warns above
    120,000). Over that, `github-release` keeps whole `###` sections that fit and
    drops the rest behind a link: simulated on 2026-10-21 with the then-136,654-byte
    Unreleased section, it kept `### Added`, `### Changed` and `### Fixed`
    (116,868 bytes) and cut `### Security`, the section a reader most needs.
    Trim, or order the sections so the one that must be read comes first.
[ ] CHANGELOG.md: fresh empty [Unreleased] section added
[ ] CHANGELOG.md: link references at the foot updated
[ ] Local gate: cargo fmt --check --all
[ ] Local gate: cargo clippy --workspace --all-targets -- -D warnings
[ ] Local gate: cargo test --workspace
[ ] Local gate: RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
[ ] Version-bump PR merged to main; CI green on the merge commit
[ ] Dry run (workflow_dispatch on release.yml) is green, including every
    `Upgrade from vA.B.C` row: each installs a published release under real
    systemd, records detections, updates in place to this build, and rolls
    back with the command the update printed. The release does not publish
    without them.
[ ] git tag -a vX.Y.Z -m "Release vX.Y.Z" && git push origin vX.Y.Z
[ ] `verify-release` green (it downloads and checksum-verifies every published
    asset over the public URL; a red one means the Release is not installable)
[ ] Post-publish verification done (Docker, attestation, SBOM)
```

## Verifying a published release

```bash
# Checksums
sha256sum -c SHA256SUMS --ignore-missing

# SLSA build provenance on a binary archive, and on the scripts and the
# checksum manifest (releases after the one that introduced this check)
gh attestation verify \
  --repo tomtom215/BirdNet-Behavior \
  birdnet-behavior-X.Y.Z-aarch64-unknown-linux-gnu.tar.gz
gh attestation verify --repo tomtom215/BirdNet-Behavior install.sh

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
  `Cargo.toml`, `CITATION.cff`, `openapi.json` and `CHANGELOG.md` match
  the tag, but you make the edits (see the checklist). There is no auto-bump.
- **Tag creation/push.** The pipeline is tag-triggered; creating and
  pushing the tag is the manual "go" action.
- **Nothing for the docs.** The manual is versioned and needs no release step
  of its own: pushing a stable tag triggers `docs.yml`, which rebuilds the whole
  site with `scripts/build-versioned-docs.sh` — the root becomes the new
  release's manual, it is also kept at `/vX.Y.Z/`, earlier releases (from
  `v0.17.1`, the first versioned one) stay at theirs, and `main` is at `/dev/`
  (built from `origin/main`, also on a tag run). Each version is rendered from its
  own tag and link-checked. A docs fix merged after a release reaches `/dev/` at
  once and the root at the next release; to correct a released manual, cut a patch
  release. The in-app `/help/*` tree is always the manual for the binary serving
  it, so it needs no versioning. What this does and does not guarantee is listed
  under `docs.yml` above — chiefly that it runs whether or not the release
  succeeds.
- **Crates.io publishing.** No workflow publishes to crates.io; it is the
  maintainer's manual step, in two batches, described under "Publishing to
  crates.io" above. `validate` and `crates are publishable` only prove that the
  nine crates would package, with their licence files.
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
- **Docs didn't update after merging a docs change.** A merged change
  appears under `/dev/`, not at the root — the root is the latest release. If
  `/dev/` is stale too, check the `docs.yml` run on `main`; Pages must be
  enabled (Settings → Pages → Source: GitHub Actions) once for the repo.
