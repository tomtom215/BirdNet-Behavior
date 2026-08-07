# Pre-Release Audit and Execution Plan

**Status:** current. Supersedes `docs/RELEASE_PUNCHLIST.md` (audited 2026-05-29) and
`docs/RELEASE_READINESS.md` (audited 2026-06-03) — both were written against the
`claude/gallant-feynman-bJs95` integration branch, which no longer exists. Everything
now merges to `main`.

**Audited:** 2026-08-07, against `main` tip `070db00` (merge of PR #194), which is also
the base of `claude/pre-release-audit-plan-if7qrp`.

**Target:** the next public release (`v0.10.0`) is field-deployable on an unattended
station with no operator intervention for a full season.

---

## 0. What was actually run

Nothing below is inferred. Every claim carries the command that produced it, on
x86_64 Linux, 4 cores, 15 GB RAM, rustc 1.97.1.

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --workspace --all-targets --all-features` | **exit 0** — 9 m 51 s |
| Format | `cargo fmt --check --all` | **exit 0** |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **exit 0** — 6 m 12 s, zero warnings |
| Tests | `cargo test --workspace --all-features` | **exit 0** — 39 suites, **1847 passed, 0 failed**, 5 ignored (all `ignore`-marked doctests) |
| Installer sync gate | `installer/build.sh --check` | in sync |
| Shell syntax | `bash -n` over `install.sh`, `quickstart.sh`, `uninstall.sh`, `installer/lib/*.sh`, `scripts/*.sh`, `docker/*.sh` | all clean |
| Doctor | `birdnet-behavior --doctor --config <station>` | works; **correctly** detected a genuinely full root filesystem (verified against `statvfs`) |
| Cold boot | `--web-only` on a fresh station | clean: 22 migrations applied, `DuckDB v1.5.5` + `behavioral v0.9.1` extension **loaded**, admin hash bootstrapped, disk manager started |
| HTTP surface | 30 endpoints probed | pages 200, `/admin/*` correctly 303 to login when unauthenticated, `/api/v2/health` + `/api/v2/metrics` 200 |
| In-app help | `/help/`, `/help/guide/today`, `/help/admin/settings` with `BNB_HELP_DIR` set | **200** — the release tarball ships `help/` (`release.yml:394`) and the unit sets `BNB_HELP_DIR` (`installer/lib/65-service.sh:70`), so this is correct in the field |
| Behavioral analytics | all 8 `/api/v2/analytics/*` endpoints against a **1 000 000-row** station | all 200; 0.14 s–1.53 s; `next-species` correctly 400s with a usage hint on a missing param |
| Scale probe | initial SQLite→DuckDB sync, peak RSS sampled from `/proc/<pid>/status` | **1 M rows → 541 MiB**, **2 M rows → 967 MiB** (see F-04) |

**CI on `main` at `070db00`:** CI, Coverage, Docker, Docs, Install smoke, Supply chain,
A11y & Visual QA, and Mutation testing are **all green**. The scheduled Mutation failure of
2026-08-03 (`validate.rs` shard) was fixed by `1e9102a` and the shard is green at tip.
There are **0 open issues** and 7 open PRs, all Dependabot.

**Two things the local gate does not cover by default** — both are covered by CI:

1. The **scientific core**. `tests/inference_e2e.rs`, `tests/pipeline_e2e.rs` and (new)
   `tests/species_filter_e2e.rs` skip unless `BIRDNET_TEST_MODEL`/`BIRDNET_TEST_LABELS` are
   set. CI's "Inference against the real model" job runs them against the sha256-pinned
   541 MB model and is green at tip.
   **Update (Slice 2 onward):** also run locally. The model was fetched, its sha256 verified
   against the CI-pinned digest, and the whole suite re-run with it — **1921 passed, 0 failed,
   0 runtime skips** as of Slice 3. To repeat:
   ```bash
   export BIRDNET_TEST_MODEL=/path/to/model.onnx BIRDNET_TEST_LABELS=/path/to/labels.csv
   cargo test --workspace --all-features
   ```
2. **Live behavioral-extension** verification. CI embeds the community extension and asserts
   the offline `LOAD`. Verified independently here at runtime (`behavioral v0.9.1` loaded).

**Verdict: the engineering substrate is in good shape.** Build, lint, test, mutation, supply
chain and CI are all clean, and the resilience layer (WAL + integrity check + backup ring +
quarantine, capped-backoff capture supervisor, disk purging, bounded queues, sd_notify
watchdog, verified auto-update) is real and tested. What is *not* ready is a specific,
bounded set of **field-behaviour defects** where the product tells the operator one thing and
does another. Those are below, and they are the release blockers.

---

## 1. Findings

Severity: **P1** = a field station silently does the wrong thing, or goes down.
**P2** = degrades or misleads without data loss. **P3** = polish / latent.

### Priority summary

| ID | Finding | Sev | Effort |
|----|---------|-----|--------|
| ~~F-01~~ | ~~20 admin-UI settings are persisted but never reach the runtime~~ — **fixed, Slice 1** | P1 | M |
| ~~F-02~~ | ~~Species include/exclude lists never filter detections~~ — **fixed, Slice 2** | P1 | S–M |
| ~~F-11~~ | ~~Species-frequency filter never ran on a normally-installed station~~ — **found and fixed in Slice 2** | P1 | S |
| ~~F-03~~ | ~~Apprise + BirdWeather: UI fields inert, but the "Test" button works~~ — **fixed, Slice 1** | P1 | S |
| ~~F-04~~ | ~~Initial DuckDB sync loads every detection into RAM — OOM at ≈2.1 M rows~~ — **fixed, Slice 3** | P1 | M |
| ~~F-05~~ | ~~A corrupt analytics DB disables analytics permanently and silently~~ — **fixed, Slice 3** | P2 | S |
| ~~F-06~~ | ~~Daily GitHub update check with no way to turn it off~~ — **fixed, Slice 4** | P2 | XS |
| ~~F-07~~ | ~~Two pre/post twilight offsets in the UI, one `--twilight-offset` at runtime~~ — **fixed, Slice 1** | P3 | S |
| ~~F-08~~ | ~~`partial_cmp().unwrap()` on floats in two web handlers~~ — **fixed, Slice 4** | P3 | XS |
| ~~F-12~~ | ~~`panic = "abort"` + no catch-panic layer makes any handler panic a station outage~~ — **found in Slice 4; class closed, posture documented** | P2 | — |
| ~~F-09~~ | ~~Version and release docs not rolled for `v0.10.0`~~ — **done, Slice 5** | P1 (release) | S |
| ~~F-10~~ | ~~Debug all-targets build needs ~21 GB of disk~~ — **fixed, Slice 5** | P3 (dev-ex) | S |

---

### F-01 — Twenty admin-UI settings are persisted but never reach the runtime · **P1**

**Evidence.** The settings form writes 53 keys. `SETTING_SPECS`
(`src/helpers/settings_overlay.rs:44`) bridges only 20 of them into the runtime config.
Cross-referencing every form key against every consumer outside the settings page itself
leaves **20 keys with no runtime consumer at all**:

```
apprise_config      audio_channels      auth_username        custom_image_dir
freq_shift_hz       night_inhibit       notify_body_template notify_confidence
notify_cooldown     notify_image        notify_species_exclude
notify_species_only notify_title_template notify_trigger     post_sunset_offset
pre_sunrise_offset  rtsp_urls           segment_duration     weekly_report_schedule
auth_password
```

They render as ordinary editable inputs — e.g. `segment_duration`
(`.../settings/render/audio.rs:44`), `freq_shift_hz` (`:66`), `night_inhibit`
(`.../render/location.rs:43`), `notify_trigger` (`.../render/notifications.rs:59`) — and the
page tells the operator *"Most settings require a restart to take effect"*
(`.../settings/render/mod.rs:149`). For these 20, **no restart ever makes them take effect.**
The runtime reads the corresponding CLI flag / `BIRDNET_*` env var instead
(`cli.segment_duration` → `src/daemon/config.rs:114`; `cli.freq_shift_hz` → `:115`;
`cli.night_inhibit` / `cli.twilight_offset` → `src/capture/schedule.rs:31-56`).

**Proven at runtime.** Six keys were written straight into the `settings` table and the
station restarted:

| key set | reached the runtime? |
|---|---|
| `purge_threshold=80` | ✅ `disk manager configured … purge_threshold=80` |
| `max_files_per_species=7` | ✅ `… max_files_per_species=7` |
| `segment_duration=30` | ❌ |
| `freq_shift_hz=2500` | ❌ |
| `night_inhibit=true` | ❌ |
| `notify_confidence=0.95` | ❌ |

The overlay logged `count=7` — the 5 pre-seeded values plus exactly the two wired keys. The
other four were dropped silently.

**Root cause.** Two configuration namespaces (`settings` table vs config file + CLI) joined
by a hand-maintained allow-list, with no mechanism that fails when a form field is added
without a matching bridge entry. This is the same defect class already fixed four times
(`clip_retention_days`, `MAX_FILES_SPECIES`, `DISK_PURGE_THRESHOLD`, per-species thresholds)
— fixed case by case, never at the root.

**Fix.**
1. Make the mapping **total and enforced**. Turn `SETTING_SPECS` into the single source of
   truth for *every* form field, with an explicit third state per key —
   `Bridged(config_key)`, `OwnedBySubsystem(&'static str)` (what `email_*` already is, see
   below), or `NotWired`. Add a test that iterates the `SettingsForm` field list and fails
   if any field has no classification. A new form field then cannot ship inert.
2. Bridge the ones that should work: `segment_duration`, `audio_channels`, `freq_shift_hz`,
   `night_inhibit`, `pre_sunrise_offset`/`post_sunset_offset` (see F-07), `rtsp_urls`,
   `custom_image_dir`, `weekly_report_schedule`. These need the CLI-vs-DB precedence helper
   that `src/helpers/system.rs:76` already uses for `disk_purge_threshold` (explicit CLI flag
   wins; otherwise the DB value; otherwise the config file).
3. For anything deliberately not wired, **remove the input** or render it disabled with a
   one-line explanation naming the env var that does work. An inert control is worse than an
   absent one.

**Verify.** Extend the A/B above into a test: write each bridged key to the `settings` table,
build the runtime config through `overlay_db_settings`, assert the resolved value changed.
Red before, green after.

---

### F-11 — Species-frequency filter never ran on a normally-installed station · **P1** · *found in Slice 2, fixed there*

**Not in the original audit** — surfaced only by tracing the value end to end while fixing
F-02, which is the argument for doing that rather than trusting the layer above.

**Evidence.** The daemon set `latitude: cli.latitude, longitude: cli.longitude`
(`src/daemon/mod.rs:149-150`) with **no config fallback**, while every other consumer has one
(`capture::schedule::resolve_location`, `create_birdweather_client`). The bare-metal installer
writes `LATITUDE`/`LONGITUDE` into `birdnet.conf`, and `/admin/settings` writes the settings
table the overlay layers onto it — neither of which sets a CLI flag. So on a normal install
the daemon received `None`.

`process_and_infer_filtered` then did `if let (Some(lat), Some(lon)) = (lat, lon)` and skipped
the species filter entirely, meaning the metadata model never ran and **`SF_THRESH` — a
documented, BirdNET-Pi-parity headline feature with a slider on the Detection settings page —
did nothing at all** unless the operator happened to pass `--latitude` explicitly.

**Fixed** by `resolve_station_coords` (CLI → config, per axis) plus making only the *model*
stage depend on having a location. Pinned by four unit tests including the half-configured
case, where one axis alone must not resolve to a `(lat, 0.0)` location.

### F-02 — Species include/exclude lists never filter detections · **P1**

**Evidence.** `build_species_filter_config` (`src/daemon/config.rs:89`) sets only
`sf_thresh` and takes everything else from `SpeciesFilterConfig::default()`, whose
`include_list`/`exclude_list` are `Vec::new()`
(`crates/birdnet-core/src/inference/species_filter.rs:38-39`). It is the sole construction
site feeding the daemon (`src/daemon/mod.rs:147`). **Nothing anywhere in production code ever
pushes into those vectors.** The filter logic itself
(`species_filter.rs:213-221`) is correct and unit-tested — it is simply never given any data.

Meanwhile `/admin/species` offers Add/Remove for both lists
(`.../admin/species/handler.rs:66-108`), persists them, and ships a preview page at
`/admin/species/test` (`.../admin/species/mod.rs:29`) that the Station page links as
*"preview the filter before it affects live detections"*.

**Why this matters in the field.** An operator who excludes a species — for privacy, to
suppress a noise class, or to stop a persistent false positive — keeps getting those
detections, stored, counted, notified on, and uploaded to BirdWeather. The one control that
looks like it addresses the problem does nothing, and the preview page confirms the
operator's (wrong) belief that it took effect.

Note `b9a4f84` already removed an *unmounted* simulation-backed tester for precisely this
"loaded gun" reason. The remaining gap is the other half: the live lists are real, and the
pipeline ignores them.

**Fix.** Populate `include_list`/`exclude_list` in `build_species_filter_config` from the
`settings` table (`species_include` / `species_exclude`, already parsed by
`.../admin/species/handler.rs:200-201`), threading the DB handle in the same way
`src/integrations/email.rs:20` does. Then either accept restart-to-apply and say so on the
page, or reload on the same signal per-species thresholds now use (`67f99ef`).

**Verify.** Integration test: seed an exclude entry, run a detection whose label matches
through the production `insert_detection` path, assert no row lands and no notification
fires. Red before, green after.

---

### F-03 — Apprise and BirdWeather: inert UI fields, working Test button · **P1**

**Evidence.** The runtime clients read **only** CLI flags and the config file:
`create_apprise_client` (`src/integrations/apprise.rs:17` → `cli.apprise_url` or
`APPRISE_URL`) and `create_birdweather_client` (`src/integrations/birdweather.rs:12` →
`cli.birdweather_token` or `BIRDWEATHER_TOKEN`). Neither ever reads the `settings` table.

But the settings page renders `apprise_url` (`.../render/notifications.rs:42`) and
`birdweather_token` (`:118`) as editable inputs, and **`/admin/notification-test` reads those
rows from the database** (`.../admin/notification_test.rs:55, 246`) — so the Test button
sends a real notification using a value the detection pipeline will never look at.

**Why this matters.** The operator pastes a Telegram/ntfy URL, clicks Test, receives the test
message, and concludes notifications are on. No detection notification ever arrives. The same
applies to BirdWeather: no observation is ever uploaded. Neither failure produces an error
anywhere — the station just goes quiet. `docs/book/admin/notifications.md:13` correctly says
to set `BIRDNET_APPRISE_URL`; the UI contradicts the docs.

The seeding direction is broken too: because notification keys are excluded from
`SETTING_SPECS`, a station configured with `BIRDNET_APPRISE_URL` shows an **empty** Apprise
field, inviting the operator to "fix" it by typing the URL into the inert box.

**Fix.** Follow the pattern `src/integrations/email.rs:11-60` already establishes — it reads
its whole config from the `settings` table and works correctly from the UI. Make
`create_apprise_client` and `create_birdweather_client` take `&AppState` and resolve
`CLI flag → settings table → config file`, and seed both directions so an env-configured
station displays its real values.

**Verify.** Set `apprise_url` only in the `settings` table, restart, assert
`create_apprise_client` returns `Some` and the notify path uses it. Add a
`store_forward_e2e`-style assertion that a detection reaches the stub endpoint.

---

### F-04 — Initial DuckDB sync loads every detection into RAM · **P1**

**Measured, not estimated.** A station was booted against synthetic databases of known size
with a fresh analytics DB, sampling `VmRSS` once a second:

| detections | peak RSS | time to serving |
|---|---|---|
| 1 000 000 | **541 MiB** | ~20 s |
| 2 000 000 | **967 MiB** | **32 s** |

That is ≈115 MiB base + **≈426 MiB per million rows**. The systemd unit sets
`MemoryMax=1G` (= 1024 MiB) with `OOMPolicy=stop` and `Restart=always`
(`installer/lib/65-service.sh:121-128`).

**→ A station crosses the memory ceiling at roughly 2.1 million detections, and then cannot
start.** `Restart=always` turns that into a restart loop.

**Root cause.** `read_sqlite_detections` (`crates/birdnet-behavioral/src/connection/sync.rs:243`)
collects the entire result set into a `Vec<SyncRow>` before a single row is appended. When
DuckDB is empty the cutoff is `None` (`sync.rs:22-38`), so *every* row is materialised.
DuckDB itself is innocent — its buffer pool is correctly capped at 256 MB
(`connection/mod.rs:74`) and the resulting file was only 40 MB. The 967 MiB is the Rust-side
`Vec`.

**When a real station hits this.**
- First start after a **BirdNET-Pi migration** — the whole point of `birdnet-migrate`, and a
  multi-year BirdNET-Pi database is exactly this size. `full_resync_from_sqlite`
  (`sync.rs:85`) takes the same unbounded path.
- First start with analytics on a station that has been recording for a year at a busy site
  (~6 000 detections/day reaches 2.1 M in about a year).
- Any restart after the analytics DB is deleted or rebuilt — which is what F-05's fix will
  do, so **F-04 must land before or with F-05.**

**Fix.** Stream in batches instead of materialising. Iterate the `rusqlite` rows and flush to
the DuckDB appender every N (10 000 is ample — the appender is already the fast path), so
peak RSS becomes O(batch), not O(rows). Apply to both `sync_from_sqlite` and
`full_resync_from_sqlite`; the staging-table swap in the latter is unaffected.

**Verify.** Extend `tests/soak.rs` with a sync-scale case: build a ≥1 M-row SQLite DB, run the
initial sync, assert `VmRSS` stays under a fixed bound (e.g. 256 MiB) and the DuckDB row
count matches. That test fails today at 541 MiB and passes after batching.

---

### F-05 — A corrupt analytics DB disables analytics permanently and silently · **P2**

**Evidence.** `AppState::new_with_analytics` (`crates/birdnet-web/src/state.rs:203-237`)
treats `AnalyticsDb::open` failure as `tracing::warn!(… "not available (non-fatal)")` and
stores `None`. There is no quarantine and no rebuild. Every subsequent start repeats the
warning, and every analytics page stays empty until a human notices and deletes the file by
hand — which no unattended field station has.

This is the long-open **G-11** from `docs/RELEASE_READINESS.md`, and it is worth closing now
because the DuckDB store is **purely derived** from SQLite: throwing it away is always safe.

**Fix.** Mirror the SQLite path (`src/app.rs:113-147`): on open failure, move the file aside
with a timestamped `.corrupt` suffix, recreate, and let the existing startup sync repopulate
it. Surface a doctor check (`src/doctor/analytics.rs` already exists) reporting the
quarantine so it is visible on `/admin/doctor` rather than only in the journal.

**Verify.** Fault injection: write garbage over `birds.duckdb`, start, assert the file is
quarantined, a fresh DB is created, the row count matches SQLite, and analytics endpoints
return 200.

---

### F-06 — Daily GitHub update check with no way to turn it off · **P2**

**Evidence.** `src/app.rs:384-416` unconditionally spawns a task that calls
`api.github.com` 60 s after start and every 24 h thereafter, for the life of the process.
There is **no CLI flag, no env var, and no setting** to disable it — `grep -i update`
over `src/cli.rs` and `.env.example` returns nothing.

Failures are handled gracefully (`tracing::debug!`, non-fatal), so this is unwanted egress
rather than a functional break. But it is the station's only *unconditional, undocumented*
outbound connection. Wikipedia image fetching — the other default-on egress — is documented
and opt-out-able via `--image-cache-dir ""`. Metered cellular links, air-gapped research
deployments, and institutional review all care about this.

**Fix.** Add `--no-update-check` / `BIRDNET_NO_UPDATE_CHECK=1`, honour it, and document the
station's complete default-on egress list (GitHub update check, Wikipedia images) in
`docs/book/getting-started/configuration.md` with the flag that disables each.

**Verify.** Unit-test the flag gate; assert the task is not spawned when set.

---

### F-07 — Two twilight offsets in the UI, one at runtime · **P3**

**Evidence.** The Location page renders independent `pre_sunrise_offset`
(`.../render/location.rs:51`) and `post_sunset_offset` (`:54`) inputs. The runtime has a
**single** `cli.twilight_offset` applied to both ends
(`src/capture/schedule.rs:31-32, 54-55`). Even via the CLI the two cannot differ. Rolled into
F-01's inert set, but the fix is a modelling decision, not just wiring: either add a second
flag and honour both, or collapse the UI to one field.

**Recommendation:** honour both — asymmetric dawn/dusk windows are a normal acoustic-monitoring
requirement, and `ScheduleConfig` already carries the two fields separately.

---

### F-08 — `partial_cmp().unwrap()` on floats in two web handlers · **P3**

`crates/birdnet-web/src/routes/pages/migration.rs:122` and `:130`, and
`crates/birdnet-web/src/routes/pages/dawn_chorus.rs:106`. Both operate on `f32` accumulated
from integer counts (`n as f32`), so **no reachable input is NaN today** — this is latent, not
live. It is still an unwrap in a request handler on a `#![forbid(unsafe_code)]`,
pedantic+nursery codebase. Swap to `f32::total_cmp`; it is a one-line change per site with no
behaviour difference for non-NaN input.

---

### F-09 — Version and release docs not rolled · **P1 (release-blocking)**

`Cargo.toml` is still `0.9.0`, which was published 2026-06-23. Since then `main` has taken
~30 commits and the `[Unreleased]` changelog section holds **26 entries**, many of them
significant field fixes (retention, disk management, maintenance scheduling, per-species
caps). CI's `validate` job refuses to release if the tag, `Cargo.toml` and `CHANGELOG.md`
disagree (`release.yml:117-129`), so this is caught rather than shipped — but it must be done.

Also stale: `docs/RELEASE_PUNCHLIST.md` and `docs/RELEASE_READINESS.md` both instruct the
reader to open PRs against `claude/gallant-feynman-bJs95`, a branch that no longer exists,
and both claim "no CI on this repo". Anyone picking the repo up cold is misled on the first
page. Mark both superseded by this document (this change does so).

**Do:** bump to `0.10.0` (new user-facing behaviour, pre-1.0 → minor), roll
`[Unreleased]` into `## [0.10.0] - <date>`, add a fresh empty `[Unreleased]`, update the
link refs at the foot, then follow `RELEASING.md`. Run the release workflow's dry-run
(`workflow_dispatch`) before tagging.

---

### F-10 — Debug all-targets build needs ~21 GB of disk · **P3 (developer experience)**

Measured here: `cargo build --workspace --all-targets --all-features` produced a **21 GB**
`target/` and filled a 29 GB volume, because every test binary statically links ONNX Runtime
and libduckdb — **~1 GB each**, and there are ~20. Setting `CARGO_PROFILE_DEV_DEBUG=none`
brought the same build to **2.1 GB** and the binary from 1.1 GB to 244 MB, with no loss for
any gate that is not a debugger session. CI already works around this by deleting 25–30 GB of
SDKs from the runner (`ci.yml`, "Free up runner disk space").

**Fix.** Add `[profile.dev] debug = "line-tables-only"` (keeps backtraces, drops the bulk) or
document `CARGO_PROFILE_DEV_DEBUG=none` as the standard way to run the local gate in
`CONTRIBUTING.md` / `CLAUDE.md`. Cheap, and it removes a real "why did my machine die" moment
for contributors and for future sessions in this sandbox.

---

## 2. Execution plan

Each slice is independently shippable, has its own gate, and is ordered so nothing blocks on
work that comes later. Branch `claude/pre-release-audit-plan-if7qrp`, PRs into `main`.

### ✅ Slice 1 — Stop the settings page from lying (F-01, F-03, F-07) — landed in `1af5434`

1. **The guard-rail.** `SETTINGS_FORM_KEYS` is exported from `birdnet-web`, pinned to
   `SettingsForm`'s own fields (enumerated through serde rather than hand-listed twice) and
   to `build_settings_items`. Every key must be classified in `SETTING_SPECS` as
   `Wiring::Bridged(config_key)` or `Wiring::OwnedBy(subsystem)`; tests fail on an
   unclassified key, an orphaned spec, a duplicate mapping, or a returning credential key.
2. **Exact CLI-source tracking replaced the sentinels.** `Cli::explicit` records what clap
   saw as `CommandLine`/`EnvVariable`, and `helpers::resolve` applies one rule everywhere:
   *explicit flag/env → admin settings → config file → default*. This also retired the
   `(notify_confidence - 0.8).abs() > EPSILON` hack, which mis-handled an operator who
   explicitly typed the default.
3. **Bridged:** `segment_duration`, `freq_shift_hz`, `night_inhibit`, `rtsp_urls`,
   `custom_image_dir`, `weekly_report_schedule`, plus `apprise_url`, `apprise_config`,
   `birdweather_token` and every `notify_*`.
4. **Twilight split:** `--pre-sunrise-offset` / `--post-sunset-offset`, each falling back to
   `--twilight-offset`, so stations that set neither keep today's symmetric behaviour.
5. **Removed rather than wired:** the Web Authentication card (see below),
   `audio_channels` (a duplicate of the working per-source control on `/admin/audio`, which
   is where `sources.rs:192` actually reads the channel count), and `notify_image` (no
   consumer anywhere in the notification stack).

**One finding got worse on closer reading, and is fixed here.** The Web Authentication card
stored the typed password as a **plaintext** `settings` row, rendered it back into the page
HTML on every later load, and changed no credential — the admin password is an Argon2id
hash in the accounts table seeded from `CADDY_PWD` — while telling the operator that
clearing the field would "disable HTTP Basic Auth". The card now explains where the
credential lives, and `purge_legacy_credential_settings` deletes any row an earlier build
left behind, on the next start.

**One place the plan over-specified the fix.** It proposed moving Apprise/BirdWeather onto
the `email.rs` direct-settings-read pattern. Unnecessary: both constructors already fall
back to `APPRISE_URL` / `BIRDWEATHER_TOKEN` in the config, and `overlay_db_settings` runs at
`app.rs:204`, before they are built at `:236`. Bridging the key was the whole fix, so the
lighter change was taken.

**Gate — green.** `fmt`; `clippy --workspace --all-targets --all-features -- -D warnings`
(zero warnings); `cargo test --workspace --all-features` → **1891 passed, 0 failed** (was
1847; 44 new tests, which assert *effect* — that a value in the config reaches the
extractor, the schedule, the Apprise client and the notification filter — not just mapping).

**Verified on a live station**, repeating the probe that exposed F-01. With the same
settings written into the table, the overlay went from applying **7** to applying **15**,
and the journal shows `Apprise notifications enabled url=http://localhost:9999
min_confidence=0.95` (previously no client was built at all from a UI-set URL) and
`notification filter configured trigger=new-species` (previously always `each`). The
rendered page no longer contains `auth_password`, `auth_username`, `audio_channels` or
`notify_image`, still contains the working `email_smtp_pass`, and renders saved values back
(`segment_duration=30`, `apprise_url=http://localhost:9999`, `pre_sunrise_offset=60`).

**Deliberately left alone:** `daemon/config.rs`'s `resolve_f32_with_default`, the sentinel
used by the detection knobs. Those keys are bridged and do work today, and its only wrong
case — an operator explicitly passing the documented default — resolves in the safe
direction (the UI value wins). Migrating it to `helpers::resolve` is tidy-up, not a fix, and
would churn a working, mutation-tested path.

### ✅ Slice 2 — Make the species filter real (F-02, F-11) — landed in `1ef66e5`

The plan said "populate the lists; keep the existing filter logic untouched". Populating them
would have shipped a fix that changed nothing. Three further defects had to go first, each
found by tracing the value rather than trusting the layer above:

1. **Name-space mismatch.** `/admin/species` collects *common* names ("Add species common
   name"); `SpeciesFilter` compares *scientific* names. A populated list would have matched
   nothing an operator could enter through the UI — and every test written against scientific
   names would have passed. Entries now match either form, case- and whitespace-insensitively.
2. **The preview lied in the other direction.** `/admin/species/test` ran its own
   common-name-only comparison, so once the lists worked, a scientific-name entry would have
   shown "Pass" while the runtime blocked it. The page now calls the detection path's own
   `matches_species`, because a page advertised as "preview the filter before it affects live
   detections" is only truthful while it is the same code.
3. **F-11, a new P1.** `process_and_infer_filtered` skipped the filter entirely unless *both*
   coordinates were set, and separately the daemon read `cli.latitude`/`cli.longitude` with no
   config fallback. So on a station configured the normal way — the installer writes
   `LATITUDE`/`LONGITUDE` to `birdnet.conf` — the daemon got `None`, never ran the metadata
   model, and left `SF_THRESH` inert. That is the headline species-frequency feature doing
   nothing on most real installs. Coordinates now resolve CLI-then-config (the rule
   `capture::schedule::resolve_location` always used), and only the *model* needs a location:
   the operator's lists apply either way.

Also delivered beyond the plan: **live reload**. The lists refresh on a 30-second TTL inside
the daemon loop through an injected `SpeciesListsProvider`, mirroring `LockedFilesProvider`
and the per-species threshold cache. And an include list matching no known species is ignored
with a warning rather than intersected to nothing, so one misspelt name cannot take a station
off the air.

**Gate — green.** `fmt`; `clippy --workspace --all-targets --all-features -- -D warnings`
(zero warnings); `cargo test --workspace --all-features` → **1915 passed, 0 failed, 0 runtime
skips** — the model-gated suites actually ran (see §0).

**Verified on a live station** running the real 11,560-species model, driving files through
the daemon's own watcher:

| step | result |
|---|---|
| Drop the bundled Magpie recording | **5** Eurasian Magpie + 1 Great Horned Owl |
| `POST /admin/species/exclude/add name=Eurasian Magpie`, **no restart** | stored |
| Drop an identical recording after the TTL | **0** Magpie; the Owl in the same file still came through |
| Also exclude `Bubo virginianus` (scientific name), drop a third | **0** detections |

The preview page agreed with the runtime at every step, for both name forms.

### ✅ Slice 3 — Bound the analytics sync, then make analytics self-heal (F-04, F-05) — landed in `6fa90ac`

The ordering the plan called out was real: F-05's rebuild path re-runs a full sync, so
streaming had to land first or self-healing would have introduced the OOM it exists to survive.

1. **Streaming sync.** Rows go straight into the DuckDB appender in batches of 10 000, so peak
   memory tracks the batch and not the row count. `read_sqlite_detections` and `SyncRow` are
   gone — 60 lines of buffering removed rather than tuned.
2. **A partial failure is now recoverable.** The next sync recomputes its cutoff from what
   DuckDB actually holds and resumes; the previous all-or-nothing append meant a station that
   died mid-sync started over.
3. **Quarantine and rebuild** on an unusable analytics file, with the `.wal` sidecar moved
   alongside it, plus a **probe read** — opening is not proof of health, because DuckDB can
   attach to a damaged file and only fail once a query touches the broken block.
4. **A doctor check** (`Analytics (quarantined files)`) so the recovery is visible on
   `--doctor` and `/admin/doctor` rather than buried in the journal.

**The regression test was written twice, and that is the point.** The first version asserted a
192 MiB bound at 200 000 rows — and **passed on the broken code**, because the old path only
needed ~87 MiB there. Measuring both implementations at three row counts gave:

| rows | buffering (old) | streaming (new) |
|---|---|---|
| 200 000 | 87 MiB | 51 MiB |
| 400 000 | **162 MiB** | **56 MiB** |
| 1 000 000 | ~420 MiB (extrapolated) | 62 MiB |

The default is now 400 000 with a 112 MiB bound — between the two curves where they differ by
~3×. Confirmed by checking the old implementation back out: **fails at 167 MiB, passes at
52 MiB on the new one.**

**Gate — green.** `fmt`; `clippy --workspace --all-targets --all-features -- -D warnings`;
`cargo test --workspace --all-features` → **1921 passed, 0 failed, 0 runtime skips** (with the
real model).

**Verified on a live station.** 500 detections synced and served; `birds.duckdb` overwritten
with 30 bytes of garbage; the next start logged *"analytics database is unusable; quarantining
it and rebuilding from SQLite"*, moved the file and its `.wal` aside as
`birds.duckdb.corrupt.<stamp>`, resynced all 500 rows, loaded the behavioral extension, and
served the same session data as before. `--doctor` then reported the leftover as a `WARN` with
the "no detections were lost" remediation.

### ✅ Slice 4 — Egress control and latent-panic cleanup (F-06, F-08, F-12) — landed

**F-08 was scoped as "three `total_cmp` swaps". The sweep mattered more than the swaps.**
Auditing every panicking construct reachable from a request handler — `unwrap`, `expect`,
`panic!`, `unreachable!`, slice indexing — across all of `birdnet-web` turned up 24 candidate
sites. All but the three named are structurally safe: the remaining `expect`s are on
`HmacSha256::new_from_slice` (accepts any key length) and signal-handler installation at
startup, and every `[0]` index is guarded by a length check, a `match` on length, or a
fixed-size array. So F-08 did name the complete set — but that is now known rather than
assumed.

**F-12, a new finding the sweep produced.** `[profile.release]` sets `panic = "abort"` and the
server mounts **no catch-panic layer**. A panic in a request handler is therefore not a 500 —
it aborts the whole process, web server and detection daemon together, and systemd restarts
it. A reachable handler panic is a station outage, and a repeatable one is a repeatable
outage. That is a deliberate, documented choice (`installer/lib/65-service.sh:97` explains
`panic=abort` as making panics visible as SIGABRT), so the fix is to keep the class empty
rather than to switch to unwinding: the two modules now carry
`#![deny(clippy::unwrap_used, clippy::expect_used)]` with the reasoning in the module docs.

**F-06 was scoped as `--no-update-check`. The inventory came first.** Enumerating every
outbound host in non-test production code found 13 literals, of which most are *not* egress —
`www.w3.org` is an SVG xmlns, `ebird.org` and `allaboutbirds.org` are link hrefs the browser
follows, and the `github.com` hits are a User-Agent string and text in rendered HTML. The real
surface is two default-on connections (`api.github.com`, Wikipedia), one first-run-only
fallback (`extensions.duckdb.org`), and seven that are off until configured.

Delivered: `--no-update-check` (narrow) **and `--offline`** (master switch, implies it and also
stops image downloads). Integrations you configured explicitly are deliberately untouched —
silently muting a configured alert channel would be a worse surprise than an unwanted GitHub
request, and a test pins that. `--doctor` reports the posture under **Outbound connections**,
and the full inventory is documented in *Configuration → What the station connects to*.

**Verified on a live binary**, all three postures:

| flags | doctor reports |
|---|---|
| *(none)* | `on by default: api.github.com (daily release check); en.wikipedia.org / upload.wikimedia.org …` |
| `--no-update-check` | `on by default: en.wikipedia.org / upload.wikimedia.org …` |
| `--offline` | `none — this station makes no unsolicited outbound connections.` |

and confirmed in the journal that `--offline` actually suppresses both (`daily update check
disabled; this station will not contact api.github.com`, `species image downloads disabled by
offline mode`), rather than only changing what the doctor says.

### ✅ Slice 5 — Release mechanics (F-09, F-10) — landed

**F-09.** `workspace.package.version` → `0.10.0`; `[Unreleased]` rolled into
`## [0.10.0] - 2026-08-07` with a fresh empty `[Unreleased]`; link refs updated; the API
doc's sample `/api/v2/health` response brought in line (it still advertised `0.9.0`).

**The release workflow was rehearsed end to end against this branch, not just spot-checked.**
`workflow_dispatch` run [31224283342](https://github.com/tomtom215/BirdNet-Behavior/actions/runs/31224283342)
on `ba84180`, **10/10 jobs green in 33 m 41 s**:

| Job | | Wall clock |
|---|---|---|
| Validate release tag | ✅ | 5 s |
| CI gate — fmt | ✅ | 54 s |
| CI gate — msrv (`cargo check` on 1.95) | ✅ | 9 m 44 s |
| CI gate — clippy (`--all-targets --all-features -D warnings`) | ✅ | 10 m 50 s |
| CI gate — doc (`RUSTDOCFLAGS=-D warnings`) | ✅ | 11 m 9 s |
| CI gate — test (`--workspace --all-features`) | ✅ | 12 m 6 s |
| Build aarch64-apple-darwin | ✅ | 11 m 13 s |
| Build x86_64-unknown-linux-gnu | ✅ | 16 m 49 s |
| Build aarch64-unknown-linux-gnu (cross) | ✅ | 20 m 58 s |
| Package and attest (SBOM, SHA256SUMS, SLSA provenance) | ✅ | 20 s |

That covers the two things a `validate`-only check cannot: **all three release targets
actually link** — including the aarch64 Linux cross-build that produces the Pi binary, the
slowest job and the one most likely to break — and the packaging step that generates the
CycloneDX SBOM, the combined checksums and the SLSA build-provenance attestation runs
against the real artifacts.

`Publish GitHub Release` is gated `if: github.event_name == 'push'` (`release.yml:593`), so
the dry run stopped after `package`. **Nothing was published**: the newest tag in the repo
is still `v0.9.0`.

**This rehearsal caught a real defect.** The first dispatch (run
[31221771538](https://github.com/tomtom215/BirdNet-Behavior/actions/runs/31221771538)) failed
its `CI gate — doc` job on two broken intra-doc links — `[Cli::parse_tracked_from]`, which
I had put behind `#[cfg(test)]` while leaving the link in a non-test doc comment, and
`[SETTINGS_FORM_KEYS]`, not in scope at the link site. Root cause: across four slices I
never once ran `cargo doc` locally, so `RUSTDOCFLAGS="-D warnings"` was the only thing
looking. The exact CI command is now part of the local gate:

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features
```

A second push was needed after that, for the same class of reason: `crates/birdnet-web/openapi.json`
still declared `0.9.0`, and a test asserts it tracks `CARGO_PKG_VERSION`. `RELEASING.md` now
names both non-obvious files a version bump has to touch beyond `Cargo.toml` — the OpenAPI
spec, and the sample `/api/v2/health` response in the API reference. Only one of the two
fails a test; the other is caught by nothing but the checklist.

**F-10 — the fix was not the one the plan proposed.** The plan said to use
`debug = "line-tables-only"`, on the reasoning that it keeps backtraces while dropping the
bulk. Measured, it does nothing:

| dev profile `debug` | `target/` | `birdnet-behavior` |
|---|---|---|
| `true` (full) | 21 GB | 1.1 GB |
| `"line-tables-only"` | **22 GB** | 752 MB |
| `0` (chosen) | **7.5 GB** | 233 MB |

Same command each time (`cargo build --workspace --all-targets --all-features`) from a clean
`target/`. Line tables do not help because the bulk is the statically-linked ONNX Runtime and
libduckdb, not Rust line tables.

**This table was wrong twice before it was right, both times by quoting a number that was not
what it claimed to be.** First the comment quoted 2.1 GB as the *line-tables* figure when it
came from `debug = "none"`. Then, corrected, it still quoted 2.1 GB for `debug = 0` — but that
number had been read off a `du` taken after a **`cargo clippy`** run, which type-checks and
links no test binaries, so it was never comparable to a full `--all-targets` build. The real
saving is ~2.8x (21 GB → 7.5 GB), not the order of magnitude the bad number implied. Worth
recording because the failure mode is generic: a `du` is only a measurement of the command
that produced it.

The profile sets `debug = 0`, carries the corrected table, and documents the per-package
`CARGO_PROFILE_DEV_DEBUG=full` escape hatch for when a backtrace's file/line is needed.

**Not done, deliberately: the tag.** `RELEASING.md` is explicit that creating and pushing
the tag is the manual "go" action, and it publishes a GitHub Release, Docker images, and
the install path real stations pull from. That is the maintainer's call, not this branch's.

### Not in scope for `v0.10.0`

- **Dependabot backlog** (#148, #176, #177, #186, #188, #190, #193). Take them *after* the
  release, not during — `password-hash 0.6` (#148) still needs `argon2 0.6` to exist
  (documented in `Cargo.toml`), and `ort 2.0.0-rc.13` (#193) moves the ONNX Runtime baseline,
  which is the one dependency that decides the glibc floor.
- **Bookworm / glibc 2.39 floor** (old G-14). The installer already refuses cleanly with
  Docker guidance (`installer/lib/30-platform.sh:146-166`) and Pi OS Trixie is the current
  release. Leave as documented-and-refused.
- **MQTT store-and-forward** (old G-12). `src/integrations/store_forward.rs` plus
  `tests/store_forward_e2e.rs` now exist; confirm coverage during Slice 4 and close the gap
  in the docs rather than opening new work.

---

## 3. Definition of done for `v0.10.0`

- [x] No form field in `/admin/settings` is editable-but-inert; the classification test enforces it
- [x] Species include/exclude demonstrably suppresses a detection end to end
- [x] Apprise + BirdWeather configured from the UI actually notify/upload; Test agrees with live
- [x] Initial analytics sync of ≥1 M detections stays under a fixed RSS bound (test-enforced)
- [x] A corrupted analytics DB is quarantined and rebuilt on the next start
- [x] Every default-on outbound connection is documented and individually disable-able
- [x] `Cargo.toml`, `CHANGELOG.md` and `openapi.json` agree; tag still to be pushed by the maintainer
- [x] The **whole release pipeline** rehearsed green at `ba84180` — validate, all five CI gates, all three build targets, SBOM + checksums + SLSA attestation (run 31224283342, 10/10)
- [x] Full gate green locally *and* in CI: `fmt`, `clippy --all-targets --all-features -D warnings`, `test --workspace --all-features`, `doc` with `-D warnings`, `check` on MSRV 1.95
- [x] The model-gated scientific core run against the real sha256-verified model — **1929 passed, 0 failed, 0 runtime skips** across 40 suites

**One honest caveat on the last box.** The real-model `inference` job lives in `ci.yml`, which
fires only on pushes to `main`/`master` and on pull requests. No PR is open for this branch,
so that job has **never run in CI here** — `ci.yml` has zero runs on
`claude/pre-release-audit-plan-if7qrp`. The coverage is real but it is *local*: the 541 MB
model was fetched, its sha256 checked against the digest CI pins (`ci.yml:175`), and
`inference_e2e`, `pipeline_e2e` and the new `species_filter_e2e` run rather than skip. Opening
the PR will run it in CI, and that is where the box gets independently confirmed.

_Keep this document current as slices land: flip the checkboxes, strike closed findings, and
re-run the §0 evidence table before tagging._
