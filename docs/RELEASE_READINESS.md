# Release-Readiness Assessment

**Purpose.** A self-contained inventory of where BirdNet-Behavior stands against the
north-star — *"a non-technical person installs with one command and it runs 24/7/365 on a
Pi or Linux box, analytics on by default, surviving every realistic edge case with zero
maintenance"* — plus the concrete gap backlog and a sequenced plan to close it. Written so
it can be picked up cold: every item carries evidence (`file:line`), root cause, a fix plan,
effort/risk, blockers, and how to verify. Companion to `docs/RELEASE_PUNCHLIST.md` (the
functional punchlist, which is essentially complete — only P3-4 cosmetics remain there).

_Last audited: 2026-06-02, against working branch `claude/epic-wozniak-iLHW8` ==
integration tip `claude/gallant-feynman-bJs95` (`4116f64`, after PR #138). Re-run the
inventory greps if the tree has moved._

---

## 0. How to work this repo (read first if resuming cold)

**Branch model (squash-loop).**
- **Working branch:** harness-assigned each session (this cycle: `claude/epic-wozniak-iLHW8`) — commit here.
- **Integration branch:** `claude/gallant-feynman-bJs95` — open every PR with this as the **base**. Never target `main`.
- Per task: `git fetch origin claude/gallant-feynman-bJs95 && git reset --hard origin/claude/gallant-feynman-bJs95`, commit on the working branch, `git push --force-with-lease -u origin <work>`, open PR head→base, repeat after squash-merge.

**Gate before every commit (no CI runs on PRs into the integration branch yet — see G-02, so this local gate is the real guard):**
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
  (v0.4.0 → **v0.5.3** published).
- **7 active CI workflows** (CI, Coverage, Mutation, Supply-chain, Docs, Docker, Release),
  **632 runs**, green on `main`.
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
| Clean target runs one-liner → recording + classifying + analytics dashboard, zero manual steps | ✅ *with internet* (model is a separate 541 MB Zenodo fetch — see G-13) on **Trixie/glibc ≥ 2.39**; ❌ on Pi OS **Bookworm** natively (G-14) |
| Killing the process auto-restarts it | ✅ `Restart=always`, `RestartSec=10` |
| Reboot brings it back | ✅ `systemctl enable` |
| Audio unplug degrades gracefully + self-recovers | ✅ supervisor, capped backoff, UI gauge |
| Disk-full degrades gracefully + self-recovers | ✅ DiskManager purge + per-species caps |
| Network-loss degrades gracefully + self-recovers | ✅ capped-backoff integrations |
| DB corruption self-recovers | ✅ SQLite quarantine/restore (DuckDB analytics DB: G-11) |
| Soak run shows no resource growth | ✅ compressed soak test (G-06) drives 20k inserts through the real path, asserts bounded RSS/fd/DB |
| All gates + CI green | ✅ green on `main`; AI-branch PRs now gated (G-02 done); dependabot clippy red is a stale weekly target (G-03 deferred) |
| Cross-compiled artifacts build | ✅ release.yml + CI aarch64 cross-check |
| Docs let a non-technical user install/upgrade/troubleshoot | ✅ strong; minor drift (onboarding wizard claim — G-09) |
| Safe auto-update (verify + rollback) | ⚠️ atomic swap + `.bak` rollback, **no integrity verification** (G-01) |

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
| Models/labels embedded | ❌ | `installer/lib/55-model.sh:5` downloads ~541 MB from Zenodo on first install — **not** bundled (G-13) |
| Web assets / fonts | ✅ | server-rendered (axum/HTMX); self-hosted fonts; no separate bundling needed |
| Help docs embedded | ✅ | `build.rs` renders mdBook into `_generated/html/`, served at `/help/*` |
| Analytics ON by default | ✅ | `installer/lib/65-service.sh:90` hardcodes `--analytics-db …`; release built `--features analytics` |
| Audio device auto-detect | ✅ | `installer/lib/70-station.sh` `detect_first_audio_device()` (`arecord -l`) |
| Location / timezone auto-detect | ❌ | installer **prompts** lat/lon (optional); no IP-geolocation; no timezone inference (G-08) |
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
| DuckDB **analytics** DB resilience | 🟡 | SQLite is covered; the regenerable analytics DB has no explicit rebuild-on-corrupt path documented (G-11) |
| Network capped backoff / graceful offline | ✅ | BirdWeather `MAX_RETRIES=3`, Apprise `MAX_RETRIES=2` (+ cooldown-map prune), exp backoff, log-and-continue; MQTT fail-fast (re-queued by supervisor) |
| Bounded queues / backpressure | ✅ | detection `sync_channel` cap 1024, broadcast 256, log ring 512/200, rate-limiter cleanup — no unbounded growth vectors |
| Safe auto-update (verify + rollback) | 🟡 | `auto_update.rs:188-293` atomic temp→extract→`.bak`→rename; **no sha256/signature verification** (`:323-326` *skips* `SHA256SUMS`); no rollback-on-failed-start test (G-01) |

### Track C — Low-touch first-run UX

| Item | Verdict | Evidence |
|---|---|---|
| First-run admin password | ✅ | argon2id `crates/birdnet-db/src/accounts.rs`; `src/helpers/auth.rs` bootstrap; installer auto-generates a strong password (user `birdnet`) and prints it once |
| Health/doctor page (CLI + web) | ✅ | `src/doctor/*` (audio/model/db/paths/disk/env/config/watchdog) + `/admin/doctor`; plain-language remediation per finding |
| Doctor **self-heal** | ❌ | diagnostic-only by design; brief wants opt-in self-heal (recreate dirs, fix perms) (G-07) |
| Web onboarding wizard persists | 🟡 | `crates/birdnet-web/src/routes/pages/onboarding.rs:7` is an explicit client-side **stub** — renders 5 steps but does not POST/persist; installer prompts cover real first-run config (G-09) |
| Audio auto-detect at first run | ✅ (installer) / 🟡 (web) | installer detects; onboarding shows a **mockup** card not live state |
| Location/timezone/lat-lon defaults | ❌ | `crates/birdnet-core/src/config/validate.rs` **requires** lat/lon; no geolocation; solar is UTC-only (`crates/birdnet-scheduler/src/solar.rs`) (G-08) |

### Track D — Polish

| Item | Verdict | Evidence |
|---|---|---|
| P3-4 cosmetics | 🟡 | uptime pill wired; "migration-missing" deferred (`RELEASE_PUNCHLIST.md` P3-4) |
| a11y / responsive / dark-light / reduced-motion sweep | 🟡 | design system mature; no recent dedicated consistency/a11y audit (G-10) |

### Track E — Testing & CI

| Item | Verdict | Evidence |
|---|---|---|
| CI: fmt/clippy×2/test×4/doc/build/MSRV/aarch64-cross | ✅ | `.github/workflows/ci.yml` |
| CI gates the **integration branch** | ❌ | `ci.yml:3-7` triggers on `main`/`master` only → PRs into `gallant-feynman` run **no CI** (G-02) |
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
full gate at PR time before it is squash-merged toward `main`. Least-invasive (PR-time only, no
per-push cost; no-op for ordinary contributors). **Verify:** open a PR into a `claude/**` base and
confirm CI runs. *(Judgment call — flagged to maintainer; trivially reverted if AI-branch globs in
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

**G-07 — Doctor self-heal.** *(Track C · P2 · M · med)* **Fix:** opt-in `--doctor --fix` (or a
web button) for *safe* heals only: recreate missing dirs, fix ownership/perms, recreate tmpfs
stream dir, re-checkpoint WAL. Never destructive. **Verify:** unit tests per heal + a dry-run
default.

**G-08 — Geolocation + timezone.** *(Track C · P2 · M · med)* **Blocked-by:** D-4. **Fix:**
optional IP-geolocation to seed lat/lon with manual override; store IANA tz; localize solar.
**Verify:** real-run check that detection timestamps + dawn/dusk windows are correct off-UTC.

**G-09 — Web onboarding persistence.** *(Track C · P2 · M · low)* **Blocked-by:** D-5. **Fix:**
POST each step to settings/audio endpoints; reflect live audio detection; gate first paint on
"not yet onboarded". **Verify:** `web_api_pages`-style test that a POST persists and re-load
reflects it.

**G-10 — Polish sweep.** *(Track D · P3 · M · low)* Finish P3-4; a11y/responsive/dark-light/
reduced-motion pass (Playwright visual-QA in `tools/visual-qa/`). **Verify:** visual-regression
+ axe pass.

**G-11 — DuckDB analytics-DB resilience.** *(Track B · P3 · S · low)* **Fix:** on analytics-DB
open failure/corruption, quarantine + rebuild from SQLite (it's a derived store). **Verify:**
fault-injection: corrupt analytics.db → next start rebuilds.

**G-12 — MQTT offline buffering (optional).** *(Track B · P3 · S · low)* **Fix:** small bounded
store-and-forward queue for missed publishes (Apprise-style), or document fire-and-forget as
intended. **Verify:** broker-down test drops nothing within the bound.

**G-13 — Model bundling / one-fetch offline.** *(Track A · P2 · M · low)* **Blocked-by:** D-2.

**G-14 — glibc / Bookworm portability.** *(Track A · P1 · M–L · med)* **Blocked-by:** D-1.

---

## 5. Proposed sequenced plan (small, independently-shippable PRs)

> Each wave = one or more PRs head `claude/epic-wozniak-iLHW8` → base `claude/gallant-feynman-bJs95`.
> Re-base onto the integration tip before each.

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
- **Wave 3 — first-run UX *(pending D-4/D-5)*.**
  - PR7: **G-09** onboarding persistence.
  - PR8: **G-08** geolocation + timezone.
  - PR9: **G-07** doctor self-heal.
- **Wave 4 — portability & offline *(pending D-1/D-2)*.**
  - PR10: **G-14** glibc/Bookworm.
  - PR11: **G-13** model bundling.
- **Wave 5 — polish.**
  - PR12: **G-10** cosmetics + a11y sweep; **G-11** DuckDB resilience; **G-12** MQTT buffer.

---

_Maintain this doc as items land: flip verdicts, strike closed gaps, and keep the scorecard in §1 honest._
