# Pre-Release Stability Audit and Execution Plan

**Status:** current. Supersedes the `v0.10.0` preparation plan (same path, git
history at `e98c8a0`), whose findings F-01…F-12 all landed and are re-verified
green below. Also supersedes `docs/RELEASE_PUNCHLIST.md` and
`docs/RELEASE_READINESS.md`.

**Audited:** 2026-08-08, against `main` tip `e98c8a0` (merge of PR #195).

**Target:** ship `v0.10.x` as a public, field-deployable release — an unattended
station that runs a full season with no operator on site, installed by *either*
documented path (bare-metal installer **or** Docker).

**Method note.** Every row below carries the command that produced it. Where a
previous cycle's claim was re-checked rather than assumed, that is stated. Two
findings in this pass (S-01, S-02) were invisible to the entire green CI matrix,
which is the point: a green gate only proves what it actually executes.

---

## 0. What was actually run

x86_64 Linux, 4 cores, 15 GB RAM, rustc 1.97.1, from a cold `target/`.

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --workspace --all-targets --all-features` | **exit 0** — 7 m 54 s, `target/` 7.2 GB, 0 warnings |
| Format | `cargo fmt --check --all` | **exit 0** |
| Lint (default) | `cargo clippy --workspace --all-targets -- -D warnings` | **exit 0** — 0 warnings |
| Lint (all features) | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **exit 0** — 0 warnings |
| Tests | `cargo test --workspace --all-features` | **exit 0** — 40 suites, **1933 passed, 0 failed**, 5 ignored |
| Tests + real model | same, with `BIRDNET_TEST_MODEL`/`_LABELS` set | **exit 0** — identical counts (see **S-04**) |
| Model integrity | `sha256sum model.onnx` / `labels.csv` | **both match** the digests pinned in `ci.yml`, `install.sh`, `installer/lib/10-config.sh`, `docker/entrypoint.sh` |
| Advisories | RustSec DB × `Cargo.lock` (510 packages) | **0 vulnerabilities, 0 unmaintained** |
| Installer sync | `installer/build.sh --check` | in sync |
| Shell syntax | `bash -n` over 30 scripts | clean |
| Doctor | `--doctor` on a fresh station | 10 passed, 2 warnings, 0 errors |
| HTTP surface | all 40 non-parameterised paths in `openapi.json` | **all 200 or 400** (400 = missing required param, by design) |
| Auth, password set | `CADDY_PWD` set, probe `/admin/*` | **303 → `/login`**; unauthenticated `POST` → **401**; public dashboard still 200 |
| Auth, no password | `CADDY_PWD` unset | open by design, documented, warned at startup — see **S-10** |
| Actions pinning | 24 `uses:` refs | 21 SHA-pinned; 3 `dtolnay/rust-toolchain` by ref — see **S-08** |

**CI on `main` at `e98c8a0`:** all nine workflows green (CI, Coverage, Docker,
Docs, Install smoke, Supply chain, A11y & Visual QA, Mutation, plus Dependabot).
0 open issues, 7 open PRs — all Dependabot.

**Verdict.** The engineering substrate is genuinely strong and the previous
cycle's fixes hold under re-verification: the settings bridge is total and
enforced (all 38 bridged config keys have real consumers — checked, not
assumed), the production panic surface is 2 provably-unreachable `unwrap`s, the
NTP/RTC-less-Pi handling is thorough, and session/share secrets are fail-secure.

What is **not** ready is below. Two are release blockers, and both are the same
shape as the defects that forced the last audit: *a thing the project promises
that it does not do*, in a path no gate executes.

---

## 1. Findings

**P1** = a field station silently does the wrong thing, or will not start.
**P2** = degrades or misleads without data loss. **P3** = polish / latent.

| ID | Finding | Sev | Effort |
|----|---------|-----|--------|
| ~~S-01~~ | ~~Docker images embed a behavioral extension built for **DuckDB 1.5.3** into a **1.5.5** engine~~ — **fixed, Slice 1** | **P1** | XS + gate |
| ~~S-02~~ | ~~The SQLite database's parent directory is never created; the station exits 1, after `--doctor` said *"no action needed"*~~ — **fixed, Slice 2** | **P1** | S |
| S-03 | 150 packages of transitive lockfile drift that no Dependabot PR surfaces — including `rustls`, `hyper`, `aws-lc-rs`, `webpki-roots` | **P1 (release)** | S — **done, see §2** |
| S-04 | The model-gated "scientific core" suites report `2 passed` whether or not they ran | P2 | S |
| S-05 | PR #196 is red: `clap` 4.6.6 changed help rendering, staling the committed CLI-help snapshot | P2 | XS — **done** |
| S-06 | PR #177 cannot merge alone (`audioadapter-buffers` ↔ `rubato` coupling) and is stale | P2 | M |
| S-07 | PR #148 cannot merge at all — `argon2` has no stable 0.6 | P2 | close it |
| S-08 | `dtolnay/rust-toolchain@master` is unpinned **in the release artifact-producing jobs** | P2 | XS |
| S-09 | `CITATION.cff` still says `0.8.0`; no gate checks it | P3 | XS |
| S-10 | `--doctor` never reports that `/admin` is open to the network | P3 | S |
| S-11 | Commits landed after the `[0.10.0]` changelog roll sit in no changelog section | P3 | XS |

---

### S-01 — Docker images ship an extension the engine refuses to load · **P1**

**The single highest-impact finding in this pass, and CI is green through it.**

`Dockerfile:142` pins the community extension to DuckDB **v1.5.3**:

```dockerfile
ARG BEHAVIORAL_EXTENSION_DUCKDB_VERSION="v1.5.3"
```

The workspace bundles DuckDB **1.5.5** (`duckdb = "~1.10505"`, where `10505` is
DuckDB 1.5.5). DuckDB refuses to `LOAD` an extension built for any other
version — `Cargo.toml:55-73` documents this at length, including that
`allow_extensions_metadata_mismatch` does not bypass the check.

**Measured, not inferred.** Both artifacts were downloaded and their footers read:

| URL | size | sha256 | footer declares |
|---|---|---|---|
| `…/v1.5.3/linux_amd64/behavioral.duckdb_extension.gz` | 405 990 | `f1f820ec…` | **behavioral v0.8.0, DuckDB v1.5.3** |
| `…/v1.5.5/linux_amd64/behavioral.duckdb_extension.gz` | 408 382 | `4777 9675…` | **behavioral v0.9.1, DuckDB v1.5.5** |

Different files. `v1.5.3` returns **HTTP 200**, so the Dockerfile's `curl`
*succeeds* and the wrong extension is embedded — the "download failed, fall
through to runtime INSTALL" branch never fires.

**Why no gate caught it.**
- `docker.yml:156` overrides only `BEHAVIORAL_EXTENSION_TARGET` (the arch),
  never `BEHAVIORAL_EXTENSION_DUCKDB_VERSION`.
- `crates/birdnet-behavioral/build.rs:57` embeds whatever bytes it is handed and
  **validates nothing**.
- `embedded_extension_loads_when_bundled` (`connection/mod.rs:423`) *would* catch
  it — it asserts `load_embedded(bytes).expect(…)` — but it only ever runs in the
  `ci.yml` test job, which fetches from the **correct** `v1.5.5` path. Nothing
  loads the extension inside the built image.

**History confirms a partial update.** `b35d4f5 "deps: bundle DuckDB 1.5.5 to
pick up behavioral v0.9.1"` moved `ci.yml`; `release.yml:332` also uses `v1.5.5`.
`git log -S"v1.5.5" -- Dockerfile` is **empty — the Dockerfile has never carried
it.** Three files hardcode the version; two were updated.

**Field impact.** Docker is one of two documented install paths, and the
*recommended* one for Pi OS Bookworm (glibc 2.36), which the native binary
refuses. Those stations either lose behavioural analytics entirely or silently
depend on a runtime `INSTALL … FROM community` — network egress at first run,
which is exactly the air-gap guarantee the embedding exists to provide.

A manual recovery path does exist (`--refresh-extension`, `src/cli.rs:174`,
surfaced in the empty-state hint at
`birdnet-web/src/routes/pages/behavioral.rs:565`), but it needs an operator to
notice that the analytics pages are empty, and network. On an unattended station
nobody is there to notice. It is a mitigation, not the fix.

**Fix (three parts — the third is what closes the class).**
1. `Dockerfile:142` → `v1.5.5`.
2. Make `crates/birdnet-behavioral/build.rs` parse the extension footer and
   **fail the build** when the declared DuckDB version differs from the linked
   engine. A wrong pin then cannot compile.
3. Add a step to `docker.yml` that runs the built image and asserts
   `LOAD behavioral` succeeds with networking disabled.

**Verified red-before-green, here, today.** Building `birdnet-behavioral` with
`BIRDNET_BUNDLED_EXTENSION_FILE` pointed at each artifact and running
`embedded_extension_loads_when_bundled`:

```
v1.5.3 (what the Dockerfile pins)      → test result: FAILED
  ExtensionLoad("load embedded: Invalid Input Error: Failed to load
  '…/behavioral.duckdb_extension', The file was built specifically for DuckDB
  version 'v1.5.3' and can only be loaded with that version of DuckDB.
  (this version of DuckDB is 'v1.5.5')")

v1.5.5 (what ci.yml and release.yml use) → test result: ok. 1 passed
```

That is DuckDB's own refusal, on the exact bytes the Docker build embeds. This
finding is measured, not inferred.

---

### S-02 — The database directory is never created; doctor says otherwise · **P1**

**Proven A/B, same command both times.**

```
A  parent directory absent → Error: "database error: sqlite error:
                             unable to open database file: …/birds.db"   exit 1
B  mkdir -p <parent>, rerun → GET / → 200, 23 migration/schema log lines
```

`--doctor` on state A reports, and exits **0**:

```
[ WARN ] Database directory — /root/BirdNet-Behavior does not exist yet —
         will be created on first run
         → no action needed unless you want to pre-create it with `mkdir -p`
```

It is never created. `src/app.rs:109` resolves the path
(`helpers::db_path_from_config` → `DB_PATH`, default `$HOME/BirdNet-Behavior/birds.db`)
and `:170` opens it, with **no `create_dir_all` anywhere on the path**.

**The asymmetry is what makes it a trap.** Every sibling directory *is*
auto-created — recordings (`helpers/system.rs:148`), watch dir
(`daemon/mod.rs:93`), the **DuckDB analytics** store
(`birdnet-behavioral/connection/mod.rs:339`), capture output, tmpfs. And
`--doctor --fix` repairs the recordings and image-cache directories
(`doctor/fix.rs:38-44`) but not this one. The only directory whose absence is
fatal is the only one nothing creates.

**Who hits it.** Not a stock bare-metal install — the installer pre-creates the
directory. It hits:
- **Docker** with a bind mount whose subdirectory does not exist;
- anyone relocating the database off the SD card, which
  `docs/FIELD_DEPLOYMENT.md:36` actively recommends ("SSD on USB — consumer SD
  cards fail after ~6 months of WAL churn") and whose storage section teaches the
  exact `RECS_DIR=/data/recordings` pattern. `RECS_DIR` is auto-created;
  `DB_PATH` is not;
- any manual or dev run.

**Fix.** `create_dir_all(parent)` before opening the database in `src/app.rs`,
matching the DuckDB path two modules over; surface a clear error if that fails
(permissions). Keep doctor's message — it becomes true.

**Verify.** Integration test: point `DB_PATH` at a nested non-existent path,
start, assert the station serves and the directory exists. Red before, green after.

---

### S-03 — 150 packages of drift that no Dependabot PR shows · **P1 (release)**

Dependabot opens PRs for **declared** dependencies. The lockfile — which is what
actually ships in every binary — had drifted far further:

```
cargo update --dry-run   →   Locking 150 packages to latest compatible versions
```

All semver-compatible, so no manifest edit is involved. It included the
security-relevant transitive floor of a networked field appliance:
`rustls 0.23.40 → 0.23.43`, `aws-lc-rs 1.17.0 → 1.18.0` (carrying
`aws-lc-sys 0.41.0 → 0.44.0`), `hyper 1.9.0 → 1.11.0`, `h2 0.4.14 → 0.4.15`,
`webpki-roots 1.0.7 → 1.0.9`, `zerocopy 0.8.48 → 0.8.56`, `regex 1.12.3 → 1.13.1`.

Three open PRs (#196, #193, #186) are strict **subsets** of this single update.

> **Caveat worth recording:** `cargo update --dry-run --workspace` reports
> "0 packages" — `--workspace` restricts the update to workspace *members*. The
> unqualified form is the one that tells the truth. That flag cost this audit a
> wrong answer before it gave the right one.

**Status: done on this branch — see §2 for the measured result.**

---

### S-04 — The scientific core passes whether or not it ran · **P2**

`model_env()` (`tests/inference_e2e.rs:28`) returns `None` when
`BIRDNET_TEST_MODEL`/`_LABELS` are unset, and each test returns early. The
harness counts that as **passed**. Proven side by side:

```
without model:  SKIP: BIRDNET_TEST_LABELS not set
                test result: ok. 2 passed … finished in 0.00s
with model:     Pica pica detected 3 time(s), best confidence: 93.5%
                test result: ok. 2 passed … finished in 2.94s
```

Same summary line; 0.00 s versus 2.94 s of real ONNX inference. Four suites
behave this way (`inference_e2e`, `pipeline_e2e`, `species_filter_e2e`, `soak`),
which is why the workspace totals are byte-identical with and without the model —
a reader cannot tell from `1933 passed` whether the classifier was verified at
all. CI does set the variables, so CI is honest; every other reader is not.

**Fix.** Have the suites honour `BIRDNET_REQUIRE_MODEL=1` (set it in `ci.yml`) and
**panic** with a clear message when the model is missing under that flag, so a
silent CI regression to "skipped" fails loudly. Locally, print the skip banner to
the summary rather than only under `--nocapture`.

---

### S-05 — PR #196 is red on the CLI-help drift gate · **P2**

`clap` 4.6.1 → 4.6.6 changed help rendering for a single alias:

```diff
-          [aliases: --preflight]
+          [alias: --preflight]
```

`ci.yml`'s "Build (debug, all features)" job runs `scripts/gen-cli-help.sh` and
fails on any diff, so the PR is blocked by a docs snapshot, not a code defect.
Reproduced exactly here after `cargo update`: one line, and regenerating the
snapshot clears it. **Done on this branch.**

---

### S-06 — PR #177 cannot merge alone, and is stale · **P2**

`crates/birdnet-core/src/audio/resample.rs:11-12` imports
`audioadapter_buffers::direct::InterleavedSlice` **and** `rubato::audioadapter::Adapter`
— the buffer type must implement the trait *rubato re-exports from its own
`audioadapter` dependency*. The lockfile shows why that couples them:

```
rubato 3.0.0               deps: audioadapter, audioadapter-buffers, …
audioadapter-buffers 3.0.0 deps: audioadapter, audioadapter-sample, num-traits
audioadapter 3.0.0
```

`rubato 3` pins `audioadapter 3` directly. Moving `audioadapter-buffers` to 4/5
brings `audioadapter` 4/5 with it, so the graph carries two versions of the crate
that defines `Adapter` — and the `InterleavedSlice` we hand to rubato implements
the wrong one.

The PR is also stale: it proposes 4.0.0, and the registry now has **5.1.0**.
This is the same shape as the documented `password-hash`/`argon2` coupling.

**Fix.** Take `rubato 3 → 4` and `audioadapter-buffers 3 → 5` **together** as one
change, behind the resampler's own tests; close #177 in favour of it.

---

### S-07 — PR #148 cannot merge · **P2 (close it)**

`Cargo.toml` already documents that `password-hash` must move in lock-step with
`argon2`. Checked against the registry index today: `argon2`'s newest published
version is **`0.6.0-rc.8`** — there is no stable 0.6. Taking `password-hash 0.6.1`
alone puts two distinct `PasswordHasher` traits in the graph and does not compile.

**Fix.** Close #148 with that reason and add an `ignore` for `password-hash` in
`.github/dependabot.yml` until `argon2 0.6.0` ships, so the backlog stops
carrying a PR that can never be green.

---

### S-08 — the release build's toolchain action is unpinned · **P2**

21 of 24 distinct `uses:` refs are SHA-pinned. Every exception is
`dtolnay/rust-toolchain`, used 17 times across 8 workflows under three refs —
`@stable` (13), `@1.95` (1), and `@master` (**3, all in `release.yml`**).

The `@master` three sit **inside the jobs that compile the binaries that get
SLSA-attested, cosign-signed and pulled by field stations**. A moving branch ref
in the artifact-producing path undercuts the rest of the supply-chain story.

`dependabot.yml` deliberately ignores *major* bumps of this action (the MSRV is
governed by `rust-toolchain.toml`); that is unrelated to pinning by digest.

**Fix.** Pin by SHA and keep selecting the channel via the `toolchain:` input.
Start with `release.yml`'s three, which are the ones that sign artifacts.

---

### S-09 — `CITATION.cff` is three releases behind · **P3**

`version: 0.8.0`, `date-released: "2026-06-11"`, while `Cargo.toml` and
`crates/birdnet-web/openapi.json` are both `0.10.0`. The file's own comment says
to bump it in lock-step. It was missed at 0.9.0 **and** 0.10.0 because
`release.yml`'s `validate` job checks only `Cargo.toml` and `CHANGELOG.md`.

For a scientific project this is the string GitHub's "Cite this repository"
widget and Zenodo hand to anyone citing the software.

**Fix.** Bump it, and extend `validate` to assert `CITATION.cff` `version` equals
the tag. (`docs/book/reference/api.md` was checked and is correct at 0.10.0.)

---

### S-10 — doctor never mentions that `/admin` is open · **P3**

Verified working as designed: with `CADDY_PWD` set, `/admin/*` → 303 `/login` and
unauthenticated `POST` → 401; with it unset, the middleware synthesises the seed
admin and `/admin/settings` returns **200 to anyone**
(`auth_middleware.rs:142-152`). That default is deliberate BirdNET-Pi parity, the
bare-metal installer auto-generates a strong password
(`installer/lib/70-station.sh:45-50`), `.env.example:89-93` warns that Docker does
**not**, and `app.rs:353` logs a warning when bound off-loopback without one.

The gap is only that `--doctor` — the tool the docs point operators at, which
already checks the listen address parses — has no check for it, and there is no
`src/doctor/auth.rs`.

**Fix.** Add a doctor check: non-loopback listen address + no admin password →
warn, naming `CADDY_PWD` and the loopback alternative.

---

### S-11 — post-roll commits are in no changelog section · **P3**

`[Unreleased]` is empty, `[0.10.0]` is dated 2026-08-07, and `ce54b61`
(daemon log-guard mutants + refreshed CLI help) and `14a5bb8` (typos config +
one spelling fix) landed after it. Tagging `v0.10.0` today passes `validate` and
ships commits its own changelog entry does not describe. Minor, but it is the
release notes.

**Fix.** Fold them into the entry when the version is rolled (§3, slice 6).

---

## 2. Dependency posture

### The seven open PRs

| PR | What | Verdict |
|----|------|---------|
| #196 | `cargo-patch-and-minor` × 8 (lockfile only) | **superseded** by the full update; was red on S-05 |
| #193 | `ort` rc.12 → rc.13 (lockfile only) | **superseded** — included in the update |
| #186 | `tokio` 1.52.3 → 1.53.1 (lockfile only) | **superseded** — included in the update |
| #188 | GitHub Actions × 10 (workflow files only) | **take** — review the SHAs, land with S-08 |
| #176 | `tower-http` 0.6.11 → 0.7.0 (manifest) | **evaluate** — we use `cors`, `trace`, `fs` |
| #177 | `audioadapter-buffers` 3 → 4 | **close** — see S-06, replace with the paired bump |
| #148 | `password-hash` 0.5 → 0.6.1 | **close** — see S-07, cannot compile |

### The lockfile convergence (done on this branch)

`cargo update` (unqualified), then the full gate re-run from the updated lockfile:

| Gate | Result |
|---|---|
| Lockfile delta | 404 insertions / 519 deletions; **150 packages** relocked; 501 total |
| Advisories | RustSec × new lockfile → **0 vulnerabilities** |
| Build `--all-targets --all-features` | **exit 0**, 0 warnings |
| Tests `--workspace --all-features` (+ real model) | **exit 0** — 40 suites, **1933 passed, 0 failed** — identical to baseline |
| Clippy, both feature sets, `-D warnings` | **exit 0**, 0 warnings |
| CLI-help snapshot | one line regenerated (S-05), committed |
| MSRV `cargo +1.95 check --workspace --all-targets --all-features` | **exit 0** — 6 m 35 s; no dependency raised the floor |

### Still behind a major (no PR, or PR superseded)

`audioadapter-buffers` 3 → 5.1.0 · `base64` 0.22 → 0.23.1 · `comfy-table` 7.1 →
7.2.2 · `generic-array` 0.14.7 → 0.14.9 · `matchit` 0.8.4 → 0.8.6 ·
`password-hash` 0.5 → 0.6.1 · `rubato` 3 → 4 · `tower-http` 0.6.11 → 0.7.0.

`base64` is three call sites on the standard `Engine` API
(`security.rs:56`, `session.rs:50`, `share.rs:33`) — low risk. `matchit` and
`generic-array` are transitive (axum, crypto) and move when their parents do.

---

## 3. Execution plan

Ordered so each slice is independently landable and independently verifiable.
Slices 1–2 are the release blockers.

### ✅ Slice 1 — S-01, the Docker extension (P1) · **landed**

1. **`Dockerfile:142` → `v1.5.5`**, with the history of the drift written next to
   it so the next reader knows why the line matters.
2. **`crates/birdnet-behavioral/build.rs` now parses the extension's metadata
   footer** and refuses to embed anything it cannot identify. The layout was
   *measured* from the published v1.5.3 and v1.5.5 artifacts, not taken from
   documentation: 8 NUL-padded 32-byte fields, extension version at `[128:160]`,
   DuckDB version at `[160:192]`, platform at `[192:224]`, footer format at
   `[224:256]`. It emits what the bytes target as generated constants, and logs
   `embedding behavioral v0.9.1 for DuckDB v1.5.5 (linux_amd64)` so the build log
   states the fact rather than implying it.

   *Why not a build-time version comparison?* Because it cannot be done: probing
   the build script's environment shows cargo exposes **no** `DEP_DUCKDB_*` to it
   (`libduckdb-sys` declares `links = "duckdb"` but emits no version key, and
   `DEP_*` reaches only direct dependents). So build.rs enforces what it can
   prove, and the cross-check lives where both facts exist — at run time.
3. **`AnalyticsDb::embedded_extension_mismatch()`** compares the embedded target
   against the linked engine. `load_extension()` logs an **error** on mismatch
   *before* attempting any stage, so a networked station that masks the defect by
   installing from the community registry still reports it.
4. **New `--verify-extension`** — opens a throwaway DuckDB, loads the extension
   the way the station does, reports engine/extension/embedded versions, exits
   0/1. Run with no network it proves the *offline* guarantee specifically.
   `--doctor` cannot answer this: it deliberately never opens DuckDB.
5. **`docker.yml`** loads the built image (cache hit) and runs
   `docker run --network none … --verify-extension`.
6. **Three new tests**, including `embedded_extension_targets_the_linked_engine`
   — the invariant nothing asserted.

**Measured, red-before-green:**

| | v1.5.3 embedded (what shipped) | v1.5.5 embedded |
|---|---|---|
| `embedded_extension_loads_when_bundled` | **FAILED** | ok |
| `embedded_extension_targets_the_linked_engine` | **FAILED** — *"targets DuckDB v1.5.3 but this binary links DuckDB v1.5.5"* | ok |
| `--verify-extension`, network **up** | **exit 1** | exit 0 |
| `--verify-extension`, `unshare -rn` (no network) | **exit 1** | **exit 0**, *"loaded behavioral extension from embedded bundle, bytes=408382"* |

The "network up → exit 1" row is the one that matters: the old code silently
succeeded there via the registry, which is exactly how this survived.

build.rs rejection paths were exercised too — a gzipped download (the realistic
mistake, since the CDN serves `.gz`), a truncated file, and random bytes each
fail the build with a message naming the cause.

### ✅ Slice 2 — S-02, the database directory (P1) · **landed**

1. **`helpers::ensure_db_dir`** creates the database's parent before anything
   touches the path, called from `src/app.rs` right after `db_path` resolves.
   A failure it cannot fix (read-only mount, ENOTDIR, permissions) returns an
   error naming the directory, the OS cause, and the remediation — it is not
   swallowed.
2. **Six new tests**: five unit tests (nested creation, multi-level, idempotence,
   bare relative filename, and the unfixable case — rooted at a regular file so
   it fails as ENOTDIR for *any* user, including root, where a permissions test
   would not fail at all), plus an integration test in `tests/boot_smoke.rs` that
   boots the real binary against a four-level-deep non-existent `DB_PATH`.

**Measured, red-before-green:** with `ensure_db_dir` temporarily neutered, the
new boot test fails with *"server exited during startup with exit status: 1"* —
the original defect exactly — and passes with it restored. The station now boots
against `…/mnt/ssd/birdnet/data/birds.db` with all four levels absent, serving
`GET / → 200` and logging `created database directory`.

`--doctor`'s message is deliberately unchanged: *"will be created on first run —
no action needed"* is now **true**.

### Slice 3 — S-03 + S-05, lockfile convergence *(done — verify in CI)*
Already on this branch with the full gate green. On merge, close #196, #193, #186
as superseded.

### Slice 4 — the dependency backlog (S-06, S-07, S-08, #176)
1. Close #148 (S-07) and add the `dependabot.yml` ignore.
2. Close #177 (S-06); land `rubato 3 → 4` + `audioadapter-buffers 3 → 5` as one
   change, gated on the resampler tests.
3. Evaluate `tower-http 0.7` (#176) against `cors`/`trace`/`fs`; take or defer
   with a written reason.
4. Take #188, and pin the three `dtolnay/rust-toolchain` refs (S-08).
5. `base64 0.23` — three call sites.

### Slice 5 — gate integrity (S-04, S-10)
1. `BIRDNET_REQUIRE_MODEL=1` in `ci.yml`; suites panic instead of skipping.
2. Doctor check for open-admin exposure.

### Slice 6 — the release itself (S-09, S-11)
1. Fold `ce54b61`/`14a5bb8` into the changelog; roll `[Unreleased]` → `[0.10.1]`
   (or `[0.11.0]` if slice 4 lands the resampler bump — that is a dependency
   behaviour change, not a patch).
2. Bump `CITATION.cff` **and** extend `release.yml`'s `validate` to check it.
3. Re-run the release dry-run (`workflow_dispatch`), then follow `RELEASING.md`.
   **The tag push stays the maintainer's manual "go".**

---

## 4. Definition of done

- [x] The extension load is **proven** offline rather than inferred:
      `--verify-extension` under `unshare -rn` exits 0 from the embedded copy,
      and `docker.yml` runs the same check with `--network none` on every image
- [x] A wrong extension pin cannot ship silently: unidentifiable bytes fail the
      build, and a version mismatch fails a test *and* logs an error at runtime
      even when a network install masks it
- [x] A station with a non-existent `DB_PATH` parent starts, and doctor's message
      is true
- [ ] `cargo update` is part of the release runbook, with the advisory scan re-run
      against the resulting lockfile
- [ ] The Dependabot queue contains only PRs that can actually merge
- [ ] Every `uses:` in `release.yml` is SHA-pinned
- [ ] A missing model makes CI **fail**, not pass quietly
- [ ] `Cargo.toml`, `CHANGELOG.md`, `openapi.json` **and** `CITATION.cff` agree,
      with `validate` enforcing all four
- [ ] Full gate green locally *and* in CI: fmt, clippy `-D warnings` both feature
      sets, `test --workspace --all-features`, rustdoc, MSRV 1.95, aarch64 cross

## 5. Still not established (and why)

- **Real Raspberry Pi hardware.** Throughput, thermals and live capture from a
  physical microphone remain unmeasured. qemu-user proved the aarch64 binary
  executes and serves; it is not the board.
- **Long-duration soak.** The soak suite bounds memory growth over minutes, not
  days. "A full season unattended" remains the target, not a measurement.
- **The Docker image built end to end here.** S-01 is measured at every level
  that does not need a container: DuckDB refuses the exact bytes the Dockerfile
  embedded, `--verify-extension` fails with the network both up and down, and the
  offline load succeeds once the pin is corrected. What was **not** done in this
  sandbox is building the image itself — so the `docker.yml` step added in
  Slice 1 is the thing to watch on the first run after merge. If it goes red, the
  image build differs from the local build in some way this audit did not see.
- **`--verify-extension` under systemd.** The documented offline recipes are
  `unshare -rn` (verified here) and `docker run --network none` (what CI runs).
  A `systemd-run -p PrivateNetwork=yes` form would suit a Pi station better, but
  this container is not systemd-booted, so it is deliberately not documented —
  an unverified command in a troubleshooting guide is worse than none.
