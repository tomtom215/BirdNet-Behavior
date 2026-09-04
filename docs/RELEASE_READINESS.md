# Release-Readiness Assessment

> **⚠️ SUPERSEDED — historical record only.** Audited 2026-06-03, at a point long before
> the current `0.15.0` tree (`Cargo.toml` `workspace.package.version`). Its branch model
> (`claude/gallant-feynman-bJs95`) is dead — that branch does not exist on the remote, and
> all work merges to `main`, which is CI-gated and green.
> [`docs/RELEASE_PLAN.md`](./RELEASE_PLAN.md) (audited 2026-08-08) succeeded this file and
> is itself now a completed record of the `v0.10.x`/`0.11.0` cycle; the live picture is in
> the later audits, starting with `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`. Of the gaps this
> file left open: **G-11** (DuckDB analytics resilience) is **closed** —
> `AnalyticsDb::open_or_quarantine` in `crates/birdnet-behavioral/src/connection/mod.rs`
> quarantines an unusable analytics DB and rebuilds it from SQLite; **G-12** (MQTT
> buffering) is closed by its *own second option* — MQTT and Apprise/email are documented
> fire-and-forget by design, and `src/integrations/store_forward.rs` buffers
> `BirdWeather` only, not MQTT; **G-10** (a11y sweep) is closed by a standing gate,
> `.github/workflows/a11y.yml` "Accessibility gate (axe-core, WCAG 2.1 A/AA)"; and
> **G-14** (glibc/Bookworm) is settled as documented-and-refused by the installer. Kept
> for the inventory and the D-1…D-5 decision record.

**Purpose.** A self-contained inventory of where BirdNet-Behavior stands against the
north-star — *"a non-technical person installs with one command and it runs 24/7/365 on a
Pi or Linux box, analytics on by default, surviving every realistic edge case with zero
maintenance"* — plus the concrete gap backlog and a sequenced plan to close it. Written so
it can be picked up cold: every item carries evidence (`file:line`), root cause, a fix plan,
effort/risk, blockers, and how to verify. Companion to `docs/RELEASE_PUNCHLIST.md` (the
functional punchlist, which is essentially complete — only P3-4 cosmetics remain there).

_Last audited: 2026-06-03, against integration tip `claude/gallant-feynman-bJs95` (`dc7d3c1`,
after PR #139); G-13 (model bundling) landed this cycle. Re-run the inventory greps if the tree
has moved._

---

## 0. How to work this repo (read first if resuming cold)

**Branch model (squash-loop).** *Obsolete — kept to explain the shape of the work below.*
The integration branch it names no longer exists, and "never target `main`" is now exactly
backwards: `main` is the base for every PR. What it said at the time:
- **Working branch:** harness-assigned each session (this cycle: `claude/epic-wozniak-iLHW8`) — commit here.
- **Integration branch:** `claude/gallant-feynman-bJs95` — open every PR with this as the **base**. Never target `main`.
- Per task: `git fetch origin <integration> && git reset --hard origin/<integration>`, commit on the working branch, `git push --force-with-lease -u origin <work>`, open PR head→base, repeat after squash-merge.

**Gate before every commit.** The parenthetical here used to read "no CI runs on PRs into
the integration branch yet — see G-02"; that is no longer true. `ci.yml` and `a11y.yml`
both carry `claude/**` in their `pull_request.branches`. It is still the real guard for
everything they do not cover, because `coverage.yml`, `install-smoke.yml`, `mutation.yml`
and `supply-chain.yml` remain restricted to `main`/`master`:
```bash
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib --bins
cargo test -p birdnet-behavior --bins        # root crate is bin-only; --lib skips its unit tests
cargo test --workspace --tests               # integration tests (link libonnxruntime; boot smoke)
```
**ONNX cold build:** a TLS-intercepting sandbox proxy breaks `ort-sys`'s download with
`invalid peer certificate: UnknownIssuer`. The SessionStart hook runs
`scripts/setup-onnxruntime.sh` automatically; run it by hand if a cold build fails (pass a
target triple when cross-compiling). Builds are slow (~5 min cold, ~15–20 s incremental) —
use background tasks + until-loop monitors.

---

## 1. Executive summary — the reframe

**The original brief's premises are stale.** This is not a greenfield "build the installer
and CI" effort. As of `4116f64`, the project already ships the large majority of the
north-star:

- A working **one-liner installer** (`curl -fsSL …/install.sh | sudo bash`), generated from
  modular `installer/lib/*.sh`, with a CI sync-gate.
- A **release pipeline** that cross-compiles `aarch64` + `x86_64`, publishes GitHub Releases
  with per-arch tarballs, `SHA256SUMS`, **SLSA provenance**, and **CycloneDX SBOMs**
  (v0.4.0 → **v0.5.3** published *at the time of this audit*; the tree is now at `0.15.0`).
- **CI workflows** — this said "7 (CI, Coverage, Mutation, Supply-chain, Docs, Docker,
  Release), **632 runs**". There are now **ten** workflow files: the six real `main` gates
  named here plus `a11y.yml` (A11y & Visual QA) and `install-smoke.yml` (Install smoke
  test) make eight that gate a push to `main`; `release.yml` is tag-driven, not a `main`
  gate, and `publish-model.yml` is dispatch-only. The run count is a point-in-time
  GitHub figure and is long out of date.
- A deep **resilience** layer: systemd hardening + watchdog/sd_notify, audio hot-plug
  auto-reconnect with capped backoff, disk-full purging + per-species caps, SQLite
  WAL + integrity-check + hot-backup + corruption quarantine/recovery, bounded queues
  everywhere, capped-backoff network integrations — most with fault-injection tests.
- A comprehensive **doctor** (CLI + web) with plain-language remediation, and a mature docs
  site (mdBook on GitHub Pages) + README + TROUBLESHOOTING + SECURITY_HARDENING + FIELD_DEPLOYMENT.

**Overall verdict: the product is at a solid beta/GA-candidate bar.** What remains is a
focused set of gaps, not a rebuild. The highest-leverage items are (1) closing two genuine
correctness/safety gaps (auto-update integrity verification; CI not gating the integration
branch), (2) a small number of architectural decisions that affect "non-technical, one
command, offline" (glibc floor / Bookworm; model bundling; geolocation), and (3) filling
the integration-test and first-run-UX holes.

### Definition-of-done scorecard

| Done-bar criterion | Status |
|---|---|
| Clean target runs one-liner → recording + classifying + analytics dashboard, zero manual steps | ✅ *with internet* (binary + model both from one GitHub origin, sha256-verified, resumable, Zenodo fallback — G-13 done) on **Trixie/glibc ≥ 2.39**; ❌ on Pi OS **Bookworm** natively (G-14) |
| Killing the process auto-restarts it | ✅ `Restart=always`, `RestartSec=10` |
| Reboot brings it back | ✅ `systemctl enable` |
| Audio unplug degrades gracefully + self-recovers | ✅ supervisor, capped backoff, UI gauge |
| Disk-full degrades gracefully + self-recovers | ✅ DiskManager purge + per-species caps |
| Network-loss degrades gracefully + self-recovers | ✅ capped-backoff integrations |
| DB corruption self-recovers | ✅ SQLite quarantine/restore (DuckDB analytics DB: G-11) |
| Soak run shows no resource growth | ✅ compressed soak test (G-06) drives 20k inserts through the real path, asserts bounded RSS/fd/DB |
| All gates + CI green | ✅ green on `main`; AI-branch PRs now gated (G-02 done); dependabot clippy red is a stale weekly target (G-03 deferred) |
| Cross-compiled artifacts build | ✅ release.yml + CI aarch64 cross-check |
| Docs let a non-technical user install/upgrade/troubleshoot | ✅ strong; onboarding wizard now real (G-09) |
| Safe auto-update (verify + rollback) | ✅ atomic swap + `.bak` rollback, **plus** sha256 verification against the release's `SHA256SUMS` before anything touches disk and a smoke test of the staged binary before the swap (G-01 done) |

---

## 2. Inventory by work-track

Verdicts: ✅ EXISTS (solid) · 🟡 PARTIAL · ❌ MISSING. Evidence is `file:line` at the audit tip.

### Track A — One-liner install & packaging

| Item | Verdict | Evidence |
|---|---|---|
| `curl \| sudo bash` one-liner | ✅ | `install.sh:14`; generated from `installer/lib/*.sh` by `installer/build.sh` (CI sync-gate) |
| Arch detection (Pi 5/4B/400, x86_64) | ✅ | `installer/lib/30-platform.sh:50` `detect_arch()`; rejects armv6/armv7 with guidance |
| Prebuilt-binary fetch (no on-device compile) | ✅ | `installer/lib/50-binary.sh:11`; GH Releases URL; `sha256sum -c SHA256SUMS` verify |
| ONNX Runtime in the binary | ✅ static | `ort` `download-binaries` links `libonnxruntime.a` at **build** time; released tarball ships **binary only** and runs (empirically confirmed by v0.5.x field installs) |
| Models/labels as shared GitHub asset, sha256-verified | ✅ | `installer/lib/55-model.sh` + `docker/entrypoint.sh` fetch the ~541 MB model from the stable `models-v3.0-preview3` GitHub release (same origin as the binary, resumable), verify the pinned sha256, fall back to Zenodo; published once by `publish-model.yml` (G-13) |
| Web assets / fonts | ✅ | server-rendered (axum/HTMX); self-hosted fonts; no separate bundling needed |
| Help docs embedded | ✅ | `build.rs` renders mdBook into `_generated/html/`, served at `/help/*` |
| Analytics ON by default | ✅ | `installer/lib/65-service.sh:90` hardcodes `--analytics-db …`; release built `--features analytics` |
| Audio device auto-detect | ✅ | `installer/lib/70-station.sh` `detect_first_audio_device()` (`arecord -l`) |
| Location / timezone auto-detect | ✅ | IP-geolocation (`/admin/settings/detect-location` → ip-api.com) returns IANA tz (G-08) and is wired into the onboarding wizard's auto-detect (G-09); doctor clock/tz check (G-08) |
| systemd install + enable + dashboard URL print | ✅ | `65-service.sh`, `75-start.sh`, `80-summary.sh:38` (URL + mDNS + IP) |
| Release pipeline cross-compile + publish | ✅ | `release.yml`: aarch64 + x86_64 (GCC cross, **not** zigbuild — ONNX needs GNU libstdc++ cxx11 ABI); SHA256SUMS, SLSA, CycloneDX SBOM |
| Pi OS Bookworm (glibc 2.36) native support | ❌ | glibc ≥ 2.39 floor from ONNX Runtime baseline; Bookworm → Docker only (G-14) |

### Track B — 24/7/365 resilience

| Item | Verdict | Evidence |
|---|---|---|
| systemd hardening | ✅ | `65-service.sh:40-182`: `Restart=always`, `RestartSec=10`, `WatchdogSec=120`, `MemoryHigh=768M`/`MemoryMax=1G`, `OOMPolicy=stop`, `ProtectSystem=strict`, empty `CapabilityBoundingSet`, `SystemCallFilter`, journald rate-limit |
| Watchdog / sd_notify | ✅ | `src/sd_notify.rs` (READY/STOPPING/WATCHDOG; interval from `WATCHDOG_USEC`; stall withholds pings); `src/doctor/watchdog.rs`; tests present |
| Audio hot-plug auto-reconnect | ✅ | `src/capture/supervisor.rs` capped backoff 2s→60s (never gives up), `birdnet_audio_source_up` gauge, down-alerts; fault-injection tests `dead_source_is_restarted_and_recovers`, `backoff_doubles_then_caps` |
| Disk-full handling | ✅ | `crates/birdnet-core/src/audio/capture/disk/{manager,purge}.rs`: 95% purge of oldest 10%, per-species caps, Purge/Keep modes; tests present |
| SQLite WAL + corruption recovery + backups | ✅ | `crates/birdnet-db/src/resilience.rs`: WAL, `quick_check`/`integrity_check`, hot backup API, rotation (5), `check_and_recover()` restore-from-backup, quarantine corrupt DB; daily/weekly maintenance ticks; tests present |
| DuckDB **analytics** DB resilience | ✅ | was 🟡. `AnalyticsDb::open_or_quarantine` (`crates/birdnet-behavioral/src/connection/mod.rs`) quarantines an unusable analytics DB and rebuilds it from SQLite — "analytics database is unusable; quarantining it and rebuilding from SQLite" (G-11) |
| Network capped backoff / graceful offline | ✅ | BirdWeather `MAX_RETRIES=3`, Apprise `MAX_RETRIES=2` (+ cooldown-map prune), exp backoff, log-and-continue; MQTT fail-fast (re-queued by supervisor) |
| Bounded queues / backpressure | ✅ | detection `sync_channel` cap 1024, broadcast 256, log ring 512/200, rate-limiter cleanup — no unbounded growth vectors |
| Safe auto-update (verify + rollback) | ✅ | was 🟡 (atomic swap only, `SHA256SUMS` *skipped*). `crates/birdnet-integrations/src/auto_update/mod.rs` now reads the asset's digest from the release's `SHA256SUMS` (erroring when the release publishes none), verifies the download against it, and smoke-tests the staged binary before the swap — `UpdateError::Integrity` / `::SmokeTest` (G-01) |

### Track C — Low-touch first-run UX

| Item | Verdict | Evidence |
|---|---|---|
| First-run admin password | ✅ | argon2id `crates/birdnet-db/src/accounts.rs`; `src/helpers/auth.rs` bootstrap; installer auto-generates a strong password (user `admin`) and prints it once |
| Health/doctor page (CLI + web) | ✅ | `src/doctor/*` (audio/model/db/paths/disk/env/config/watchdog) + `/admin/doctor`; plain-language remediation per finding |
| Doctor **self-heal** | ✅ | `--fix` creates missing configured dirs (recordings + image-cache) before reporting; safe/idempotent, never needs root (G-07) |
| Web onboarding wizard persists | ✅ | `POST /onboarding/save` persists location/timezone/notify + `onboarding_complete`; fresh box is redirected to the wizard; auto-detect wired (G-09) |
| Audio auto-detect at first run | ✅ (installer) / 🟡 (web links to Settings → Audio) | installer detects the device; the wizard delegates audio config to Settings → Audio per D-5 |
| Location/timezone/lat-lon defaults | ✅ | lat/lon are *advisory* in `validate.rs`; IP-geolocation + IANA tz + doctor clock check (G-08); captured + persisted at first run by the wizard (G-09) |

### Track D — Polish

| Item | Verdict | Evidence |
|---|---|---|
| P3-4 cosmetics | 🟡 | uptime pill wired; "migration-missing" deferred (`RELEASE_PUNCHLIST.md` P3-4) |
| a11y / responsive / dark-light / reduced-motion sweep | ✅ | was 🟡 ("no recent dedicated audit"); a standing gate now runs on `main` and `claude/**` PRs — `.github/workflows/a11y.yml`, step "Accessibility gate (axe-core, WCAG 2.1 A/AA)" (G-10) |

### Track E — Testing & CI

| Item | Verdict | Evidence |
|---|---|---|
| CI: fmt/clippy×2/test×4/doc/build/MSRV/aarch64-cross | ✅ | `.github/workflows/ci.yml` |
| CI gates the **integration branch** | 🟡 | Was ❌ (`ci.yml` triggered on `main`/`master` only). `ci.yml` and `a11y.yml` now carry `claude/**` in `pull_request.branches`, which covers a PR whose *base* is a `claude/**` branch — a PR *from* one into `main` already ran every gate, because the filter reads the base. `coverage.yml`, `install-smoke.yml`, `mutation.yml` and `supply-chain.yml` still lack the glob, so only that stacked case is uncovered (G-02) |
| inline-style guard | ✅ | `crates/birdnet-web/tests/inline_style_guard.rs` (runs under `cargo test --tests`) |
| Coverage / Mutation / Supply-chain / Docs / Docker | ✅ | `coverage.yml` (llvm-cov), `mutation.yml` (cargo-mutants), `supply-chain.yml` (deny/audit/machete/typos/shellcheck), `docs.yml` (mdbook→Pages), `docker.yml` (multi-arch GHCR) |
| Dependabot CI green | ❌ | cargo-bump branch fails **Clippy (pedantic+nursery, -D warnings)** — `main` unaffected (G-03) |
| Full-pipeline E2E (audio→infer→DB→web) | ✅ | `tests/pipeline_e2e.rs`: CI layer (real decode/resample + real `insert_detection`→web read) + model-gated full chain (G-04) |
| BirdNET-Pi migration integration test | ✅ | `crates/birdnet-migrate/tests/migration_e2e.rs`: fixture→import→assert dest rows/values/schema + idempotency + clamping + CSV (G-05) |
| Longevity / soak test | ✅ | `tests/soak.rs`: 20k inserts, asserts bounded RSS/fd/DB (G-06) |

### Track F — Docs

| Item | Verdict | Evidence |
|---|---|---|
| README with one-liner | ✅ | `README.md:88-110` |
| Install / upgrade / troubleshoot guides | ✅ | mdBook site, `TROUBLESHOOTING.md`, `RELEASING.md`, `SECURITY_HARDENING.md`, `FIELD_DEPLOYMENT.md`, `MACOS.md` |
| This readiness doc kept current | ✅ (new) | `docs/RELEASE_READINESS.md` |

---

## 3. Open decisions (architecturally significant — need maintainer call before building)

> **Resolved 2026-06-02 (maintainer):** **D-1 →** spike an Ubuntu 22.04 (glibc 2.35)
> release build first; if ONNX Runtime still links, Bookworm is covered with no code
> change. **D-2 →** attach the model as a shared GitHub release asset, and *possibly* also
> ship a heavy offline bundle. **D-3 →** yes, gate the AI branches (done in this wave, see
> G-02). **D-4 / D-5** still open (deferred to their waves). Start point: **Wave 1**.

These shape the plan; I'll verify the facts and bring a recommendation, but the call is yours.

**D-1 — glibc floor / Pi OS Bookworm (G-14).** The native binary needs **glibc ≥ 2.39**
because the release is built on Ubuntu 24.04 to match the prebuilt ONNX Runtime baseline.
Pi OS **Bookworm** (glibc 2.36) — still the most common Pi OS in the field — can't run it
natively; the only path is Docker. Options: (a) build the Linux artifacts on an **older base
(Ubuntu 22.04, glibc 2.35)** if ONNX Runtime's prebuilt permits — this would cover Bookworm
with no code change; (b) a **musl-static** target (likely blocked: `ort`'s prebuilt ONNX
Runtime is glibc-only, would need a self-built musl ORT); (c) **accept Docker-only for
Bookworm** and make the installer's refusal message even clearer. *Recommend a short spike to
test (a) before deciding.*

**D-2 — model bundling / "offline after one fetch" (G-13).** Today the one command fetches
the binary **and** a separate ~541 MB model from Zenodo at first run; cut the network and a
fresh install can't complete. The model is arch-independent (one ONNX file). Options: (a)
keep installer-fetch (status quo; document "first run needs internet"); (b) attach the model
as a **single shared release asset** and have the installer pull it from GitHub (one origin,
resumable, provenance-covered); (c) ship a heavier **"offline bundle"** tarball variant
(binary + model). *Recommend (b) as the best effort/value.*

**D-3 — CI gating the integration branch (G-02).** Add `claude/gallant-feynman-*` (and PR
base) to `ci.yml` triggers so integration PRs run the full gate. Cost: each PR triggers the
DuckDB-compiling matrix (~10–15 min). *Recommend yes — it's the single biggest quality lever.*

**D-4 — geolocation + timezone (G-08).** For true zero-config, infer lat/lon (the onboarding
mockup references `ipapi.co`) and a timezone on first run. Decisions: which provider (privacy
+ offline fallback), and whether to add real timezone handling (solar is UTC-only today —
relevant to DST/polar correctness). *Recommend: optional IP-geolocation with manual override,
plus storing an IANA tz; verify current timestamp localization in a real run first.*

**D-5 — web onboarding wizard (G-09).** The README advertises "a first-run onboarding
wizard" but it's a non-persisting stub. Options: **wire it** to the existing settings/audio
endpoints, or **de-scope** it and adjust the README. *Recommend wiring it — the endpoints exist.*

---

## 4. Gap backlog

Each gap is independently shippable. `Blocked-by` references the decisions above.

**G-01 — Auto-update has no integrity verification.** *(Track B · P1 · S–M · low risk)* ✅ **DONE (this wave).**
`auto_update.rs` previously swapped the binary after a plain download and *skipped* `SHA256SUMS`.
Now: `check_for_update` parses the release's `SHA256SUMS` into `UpdateInfo.sha256`; `apply_update`
takes `expected_sha256` and **verifies the downloaded archive before anything touches disk**
(refusing on mismatch via the new `UpdateError::Integrity`), then **smoke-tests the staged binary**
(`<binary> --version`) before the swap — a wrong-arch/truncated/incompatible binary is discarded
and the running binary is left untouched (`UpdateError::SmokeTest`). Defense in depth: checksum
when available + always smoke-test. Pure helpers (`parse_sha256sums`, `sha256_hex`,
`verify_integrity`) and the smoke test are unit-tested (FIPS sha256("abc") vector, mismatch
rejection, exec-fail). Not a signature check — SLSA provenance remains the out-of-band authenticity path.

**G-02 — CI doesn't gate the AI integration branch.** *(Track E · P1 · XS · low)* ✅ **DONE (this wave).**
Nuance from the audit: the maintainer's real integration branch is **`main`** (dependabot,
docs, all gates target it) and it *is* CI-gated and green. The un-gated branch is the
**AI-session** integration branch (`claude/gallant-feynman-*`), since `ci.yml` only triggered on
`main`/`master`. Fix: added `claude/**` to `ci.yml` `pull_request.branches`, so a slice runs the
**CI** gate — fmt, clippy, tests, MSRV, aarch64 cross-check — at PR time before it is
squash-merged toward `main`.

What the `claude/**` glob actually buys is narrower than it looks, and in the
opposite direction to the obvious reading. `pull_request.branches` filters on the
PR's **base**, not its head, so a PR *from* a `claude/**` branch *into* `main`
already matched every workflow whose filter names `main` — which is all of them.
Observed on PR #235, head `claude/birdnet-audit-reconciliation-jtgz6k`, base
`main`: `cargo-deny`, `cargo-audit`, `cargo-machete`, `Spelling (typos)`,
`shellcheck (bootstrap scripts)` and `installer unit tests` (supply-chain.yml),
`cargo-llvm-cov` (coverage.yml), `install.sh → web UI` (install-smoke.yml) and
`Accessibility (axe) + visual-QA sweep` (a11y.yml) all ran, alongside every
ci.yml job. The glob covers the *other* case: a stacked PR whose base is itself a
`claude/**` integration branch, which previously matched nothing. `ci.yml`'s own
comment says exactly that.

So the residual gap is only that a PR based on a `claude/**` branch runs CI and
a11y but not coverage, supply-chain, install-smoke or mutation. `mutation.yml`
additionally carries `paths:` filters, so it is skipped whenever the diff misses
those files regardless of branch — which is the usual reason it does not appear.
Least-invasive (PR-time only, no per-push cost; no-op for ordinary contributors).
**Verify:** open a PR into a `claude/**` base and confirm CI runs. *(Judgment call — flagged to maintainer; trivially reverted if AI-branch globs in
committed CI are unwanted.)*

**G-03 — Dependabot CI red on clippy.** *(Track E · P2 · S · low)* ⏸️ **DEFERRED (documented).**
The failing run (`cargo-patch-and-minor`, sha `d72a1559`, 2026-06-01) is **stale** — its lockfile
predates the current tip (e.g. it still carries `sha2 0.10.9`; tip is on `sha2 0.11`). Dependabot
regenerates this branch weekly, so the specific clippy break is a moving, already-superseded target,
and `main` is unaffected (green). Chasing it isn't tractable or release-relevant; the right fix is
to address the lint **when a current bump trips it** (best handled in the maintainer's dependabot
flow). Re-open if a *fresh* dependabot bump lands red on the integration branch.

**G-04 — Full-pipeline E2E test.** *(Track E · P2 · M · low)* ✅ **DONE (Wave 2).**
`tests/pipeline_e2e.rs` has two layers: a **CI-runnable** layer (real `decode::decode_file` +
`resample` on the bundled `Pica_pica_30s.wav`; a detection written through the production
`insert_detection` path and read back over `/api/v2/detections` + `/api/v2/stats`), and a
**model-gated** layer (skipped unless `BIRDNET_TEST_MODEL`/`_LABELS` set, like `inference_e2e.rs`)
that runs decode→resample→inference→DB→web and asserts the Magpie surfaces on the API.

**G-05 — BirdNET-Pi migration integration test.** *(Track E · P2 · M · low)* ✅ **DONE (Wave 2).**
`crates/birdnet-migrate/tests/migration_e2e.rs` builds fixture legacy SQLite + CSV databases,
runs the public `run_migration`, then **opens the destination and asserts** rows/values landed,
the full migrated schema is present (>12 columns), idempotency (re-import inserts 0 — surfaced and
fixed the SQLite NULL-`File_Name` dedupe subtlety), confidence clamping/NULL→0, and the
`validate_source` preview. Pure SQLite, so it runs in CI without the model.

**G-06 — Soak / longevity test.** *(Track E · P2 · M · med)* ✅ **DONE (Wave 2).** `tests/soak.rs`
drives `BIRDNET_SOAK_N` (default 20k) detections through `insert_detection` on an on-disk DB and
asserts bounded growth: resident memory (`/proc/self/status` VmRSS) < 128 MiB, no fd leak
(`/proc/self/fd`), and WAL-inclusive DB size linear-bounded. Env-tunable for a heavier local soak.

**G-07 — Doctor self-heal.** *(Track C · P2 · M · med)* ✅ **DONE (Wave 3).** Added a `--fix`
flag (`src/doctor/fix.rs`) that implies the doctor and runs *safe, idempotent* repairs before the
diagnostic: it creates any missing configured directories (recordings/watch + image-cache — the #1
"service runs but nothing is recorded" cause after a tmpfs reset), reports each as a `Repair:`
check, and the subsequent checks reflect the healed state. Ownership/packages (root-only) are
reported, not changed, so `--fix` is safe as the unprivileged service user. Unit-tested (create /
idempotent / skip) + dispatch tests. *Deliberately scoped:* chown and WAL-checkpoint were left out
(root-only / already covered by the maintenance tick).

**G-08 — Geolocation + timezone.** *(Track C · P2 · M · med)* ✅ **DONE (Wave 3; decision D-4 =
"rely on OS clock, surface it").** Audit correction: IP-geolocation **already existed**
(`GET /admin/settings/detect-location` → ip-api.com) and lat/lon are *advisory* (a warning, not a
hard requirement) in `validate.rs`. This wave: (1) `detect_location` now also returns the IANA
`timezone` ip-api.com reports, so onboarding/settings can capture it; (2) a new doctor check
(`src/doctor/clock.rs`) surfaces the time stack in plain language — it warns when the system clock
reads before 2020 (unset/NTP-unsynced → wrong timestamps + continuous recording) and, since the
recording-window gate is evaluated in **UTC**, warns that a *fixed* window's hours mean UTC (solar
schedules are timezone-independent, so they pass). Verified time-frame finding: detection
timestamps come from the recording filenames (OS-local), and solar windows are correct in UTC; a
full chrono-tz refactor was deliberately **not** done per D-4. Wiring geolocation into the
onboarding wizard lands in G-09.

**G-09 — Web onboarding persistence.** *(Track C · P2 · M · low)* ✅ **DONE (Wave 3; decision D-5 =
"full: persist + first-boot redirect").** The wizard is now real: it submits to a new
`POST /onboarding/save` that persists latitude/longitude/timezone (Location) + the chosen
notification mode and sets an `onboarding_complete` flag, then 303s to `/`. The Location step's
auto-detect button calls the existing `/admin/settings/detect-location` (fetch, CSP-safe) to fill
coordinates + timezone; the Boston example became a *placeholder* so clicking through never
persists a wrong default. `GET /` now redirects a fresh station (no detections **and** not
onboarded) to `/onboarding`, failing safe on any DB error so the operator is never trapped. Audio
selection links to Settings → Audio per D-5. Tests: `tests/web_api_onboarding.rs` (redirect on/off,
save persists + completes, empty submit completes without writing blanks); `boot_smoke` updated to
accept the first-boot 303 and assert the wizard serves.

**G-10 — Polish sweep.** *(Track D · P3 · M · low)* ✅ **DONE (a11y half).** The
a11y/responsive/dark-light/reduced-motion pass is now a standing gate rather than a
one-off: `.github/workflows/a11y.yml` runs "Accessibility (axe) + visual-QA sweep",
including "Accessibility gate (axe-core, WCAG 2.1 A/AA)" (`node axe.mjs`), on `main` and
on `claude/**` PRs, path-scoped to `crates/birdnet-web/**` and `tools/visual-qa/**`. P3-4's
migration-missing stub remains deliberately out of scope (`RELEASE_PUNCHLIST.md` P3-4).

**G-11 — DuckDB analytics-DB resilience.** *(Track B · P3 · S · low)* ✅ **DONE.** Exactly the
proposed fix landed: `AnalyticsDb::open_or_quarantine`
(`crates/birdnet-behavioral/src/connection/mod.rs`) quarantines an unusable analytics DB via
`quarantine_file` and rebuilds it from SQLite — it is a derived store — logging "analytics
database is unusable; quarantining it and rebuilding from SQLite" and reporting which path
it took through `OpenOutcome`.

**G-12 — MQTT offline buffering (optional).** *(Track B · P3 · S · low)* ✅ **CLOSED by the
second option.** MQTT is documented fire-and-forget by design, on the grounds that it is live
telemetry: `src/integrations/store_forward.rs` states it directly — "MQTT and Apprise/email
stay fire-and-forget by design (live telemetry / look-now alerts)". The store-and-forward
queue that file implements (the `outbound_queue` table, migration 19) is for **BirdWeather**
only, the one channel where late delivery is correct. No MQTT buffer was built, and none is
planned.

**G-13 — Model bundling / one-fetch offline.** *(Track A · P2 · M · low)* ✅ **DONE (decision D-2 =
"stable shared GitHub release asset").** The ~541 MB BirdNET+ V3.0 model + labels now publish to a
single, stable, arch-independent GitHub release (`models-v3.0-preview3`) via the new
`.github/workflows/publish-model.yml` — it mirrors the files from Zenodo, **fails unless their
sha256 matches the values pinned in `installer/lib/10-config.sh`** (so the asset and the installer's
verification hash can never drift), writes `SHA256SUMS`, attaches a SLSA attestation, and
creates/updates the release idempotently (non-latest, so it never shadows the app release). The
bare-metal installer (`installer/lib/55-model.sh`) and the Docker entrypoint
(`docker/entrypoint.sh`) now fetch from that GitHub release **first** (same origin as the binary,
resumable `download_large`), **verify every file against the pinned sha256**, and **fall back to
Zenodo** when the asset is absent (older release lines) or unreachable — so the model is uploaded
once, not per app release, and a fresh install needs a single network origin and is offline-capable
afterwards. Verified in a real run (local GitHub stand-in + live Zenodo): GitHub-primary
fetch+verify (Zenodo untouched), a tampered asset is detected and falls back, a file that fails on
**both** origins is **never** left on disk (fatal), and a GitHub 404 falls back to live Zenodo —
11/11 assertions green. Mirrored in README, RELEASING.md ("The shared model release"), the release
notes, and quickstart. **Operational note:** publish `models-v3.0-preview3` (run the workflow) so
0.6.0+ installs hit GitHub first; until then they transparently fall back to Zenodo.

**G-14 — glibc / Bookworm portability.** *(Track A · P1 · M–L · med)* **Blocked-by:** D-1.

---

## 5. Proposed sequenced plan (small, independently-shippable PRs)

> Each wave = one or more PRs head `claude/epic-wozniak-iLHW8` → base
> `claude/gallant-feynman-bJs95`, re-based onto the integration tip before each. That base
> branch no longer exists; PRs now target `main`.

- **Wave 0 — decisions.** Resolve D-1…D-5 (this doc + the questions raised alongside it). Run
  the D-1 glibc spike (try an Ubuntu 22.04 build) so the call is fact-based.
- **Wave 1 — safety & gate (low ambiguity, high value).** ✅ in this branch.
  - PR1: **G-01** auto-update integrity verification + pre-swap smoke test — ✅ done.
  - PR2: **G-02** CI gates `claude/**` PRs — ✅ done.
  - PR3: **G-03** dependabot clippy red — ⏸️ deferred (stale weekly target; `main` green).
- **Wave 2 — prove the resilience that already exists.** ✅ in this branch.
  - PR4: **G-04** full-pipeline E2E test — ✅ done (`tests/pipeline_e2e.rs`).
  - PR5: **G-05** migration integration test — ✅ done (`crates/birdnet-migrate/tests/migration_e2e.rs`).
  - PR6: **G-06** soak/longevity harness — ✅ done (`tests/soak.rs`).
- **Wave 3 — first-run UX.** ✅ in this branch (decisions D-4 = rely-on-OS-clock, D-5 = full persist + redirect).
  - PR7: **G-09** onboarding persistence + first-boot redirect — ✅ done.
  - PR8: **G-08** geolocation timezone surfacing + doctor clock check — ✅ done.
  - PR9: **G-07** doctor self-heal (`--fix`) — ✅ done.
- **Wave 4 — portability & offline *(pending D-1/D-2)*.**
  - PR10: **G-14** glibc/Bookworm.
  - PR11: **G-13** model bundling — ✅ done (shared `models-v3.0-preview3` GitHub release; installer + Docker fetch GitHub-first, sha256-verified, resumable, Zenodo fallback; `publish-model.yml`).
- **Wave 5 — polish.**
  - PR12: **G-10** cosmetics + a11y sweep; **G-11** DuckDB resilience; **G-12** MQTT buffer.

---

_Maintain this doc as items land: flip verdicts, strike closed gaps, and keep the scorecard in §1 honest._
