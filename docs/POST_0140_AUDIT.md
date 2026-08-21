# Post-0.14.0 audit: what a 24/7/365 enclosure will actually hit

**Date:** 2026-08-21 · **Branch:** `claude/production-readiness-audit-k7fzps` · **Base:** `9615b9c`

> **Status.** D1, D2, D4, D5, D6, D7 and D8 were fixed on this branch after the
> audit was written; each fix's gate was observed failing against the code it was
> written for, and the commit message records how. The findings below are left as
> written — they are the record of what the shipped 0.14.0 did. Items still open:
> **D3** (single `Mutex<Connection>`), **D9** (import segmentation and undo),
> **D10** (`iso_week`), **D11** (two remaining HTML escapers), **D12**
> (`clock.rs`'s premise), **D13** (gates that pass by skipping).

Everything below was verified first-hand in this session against the code on this
commit. Where I measured, the numbers and the method are given. Where I could not
verify, it says so. Two claims I made early and then disproved are marked
**RETRACTED** rather than quietly dropped.

---

## 0. Method, and what "verified" means here

| What | How |
|---|---|
| Build | `cargo build --workspace --all-targets` → exit 0, 6 m 29 s (4-core, 15 GB) |
| Tests | `cargo test --workspace` → exit 0, no `FAILED` lines |
| Clippy | `cargo clippy --workspace --all-targets` → exit 0, no warnings |
| Query cost | Synthetic 3-year station: 3 285 000 rows, 180 species (Zipf-weighted), 3 000 det/day, the shipped `detections` DDL and **all 16** shipped indexes, `ANALYZE` run, `PRAGMA cache_size=-2000` as shipped. x86 NVMe, warm page cache — **a Pi on SD will be worse, and I did not measure by how much.** |
| Storage split | `dbstat` on that database |
| WAL/restore behaviour | Reproduced with `python3` + stdlib `sqlite3`, scripts in the session scratchpad |
| Week semantics | Scratch probe compiled against the real bundled DuckDB, then deleted |

**Two facts to hold onto, because most of what follows descends from them:**
the whole application shares **one** `Mutex<Connection>`, and `Date`/`Time` are
local wall clock inherited from BirdNET-Pi.

---

## 1. Defects, worst first

### D1 — "Download full backup" produces a backup that is missing data, and "Restore" can silently do nothing

**[FIXED]** — `full_backup` now snapshots through `rusqlite::backup::Backup`; `restore_backup` removes the sidecars, vets the archive listing and `quick_check`s the result; both temp paths moved off `PrivateTmp`.

`crates/birdnet-web/src/routes/admin/system_controls/backup.rs`

There are **two** backup mechanisms in this codebase and they do not have the same
correctness properties.

The **scheduled** one (`birdnet_db::resilience::backup_database`) is right: it runs
`quick_check` on the source, refuses to snapshot a corrupt database, and copies via
`rusqlite::backup::Backup` — the SQLite online-backup API, which is
transaction-consistent and includes WAL content.

The **operator-facing** one (`full_backup`, :30–90) shells out to `tar czf` over the
live `birds.db` **only**:

* no `PRAGMA wal_checkpoint` first, and `birds.db-wal` is not in the archive — so
  every transaction still sitting in the WAL is **absent from the backup**;
* no isolation — `tar` reads the file while the daemon writes to it;
* the archive is built in `std::env::temp_dir()`, which under the shipped unit's
  `PrivateTmp=yes` is a **tmpfs charged to the cgroup**, against `MemoryMax=1G`.
  A station with a few GB of clips can OOM itself by pressing "download backup";
* cleanup is a detached `tokio::spawn` that sleeps 300 s, so a slow field download
  races it, and a restart inside the window leaks a multi-GB file in RAM.

The `restore_backup` handler (:140–265) is worse. `grep` for
`wal|shm|restore_from_backup|integrity|quick_check|schema_version|strip-components`
in that file returns **zero hits**. It:

* `tar xzf`s straight into the data directory — the only validation is "some member
  ends in `.db`". No member whitelist, no `--no-same-owner`, no `--strip-components`;
* leaves the running daemon's `birds.db-wal` / `-shm` in place beside the file it
  just replaced;
* never checks integrity or `schema_version` on the restored database;
* never checks free space before extracting;
* extracts over a database the process still holds open, then prints
  "Restart the server to load the restored data".

`birdnet_db::resilience::restore_from_backup` (`resilience.rs:336–360`) already
removes `-wal`/`-shm` correctly, and even carries a comment about the
`with_extension("db-wal")` trap. It is not used here.

### D2 — The documented manual recovery procedure can silently restore nothing, and report "ok"

**[FIXED]** — the runbook deletes `-wal`/`-shm` first and verifies the result, with the reproduction recorded beside it.

`docs/book/field/deployment.md` §8 tells the operator, in the runbook they will
read at the worst possible moment:

```bash
cp ~/BirdNet-Behavior/backups/birds.db.backup.<timestamp> ~/BirdNet-Behavior/birds.db
```

**Reproduced.** Backup taken after a `VACUUM` (1 000 rows); station then diverged to
9 000 rows plus a new table; WAL left uncheckpointed at 3.7 MB; `cp` run; `-shm`
removed (which is what a reboot does). On reopen:

```
integrity_check: ok      detections rows: 9000      extra rows: 5000
```

The operator got back the database they were trying to replace — including a table
the backup does not contain — with a green light. An earlier run that left `-shm`
in place *did* restore correctly, so **the failure is intermittent**, which is worse
than deterministic.

Fix is one line in the doc (`rm -f birds.db-wal birds.db-shm` before the `cp`) and
one call in the code (`resilience::restore_from_backup`).

### D3 — One `Mutex<Connection>` serialises the entire application

`crates/birdnet-web/src/state.rs:53` — `db: Mutex<Connection>`; `:512` — `with_db`.

The detection **write** path uses the same handle
(`src/daemon/processor.rs:270`), as do every page, the 30-second health-badge poll
on every open tab, `/metrics`, and the live feed. WAL is enabled
(`connection.rs:56`) but a single connection uses none of its multi-reader
concurrency.

Measured hold times on the 3-year database (shipped SQL, exact text):

| Surface | Query | Measured |
|---|---|---|
| Reports → History calendar | `detections_per_day` | **1 271 ms** |
| Life List | `species_first_seen` | **375 ms** |
| any `detections_analytic` count | `COUNT(*)` through the view | 132 ms |

A detection landing while someone opens the History calendar waits behind it. On a
Pi, longer. This is the single largest structural item in the report.

### D4 — Four user-facing surfaces read a table nothing writes

**[FIXED]** — all four dispatch channels now record their outcome; `NotifStatus` gained `Queued` for BirdWeather's store-and-forward case. No `skipped` rows, deliberately.

`birdnet_db::notifications::log_notification` has **zero production callers**
(`grep -rn log_notification . --include=*.rs` → its own definition and its own unit
tests, nothing else). `NotifRecord` likewise has no use outside its module.

Three surfaces read `notification_log` and will be permanently empty on every real
station:

* `crates/birdnet-web/src/routes/pages/notification_center.rs` — the whole page
* `crates/birdnet-web/src/routes/admin/notifications.rs`
* `crates/birdnet-web/src/routes/pages/homes/station_tabs.rs:122`

The **only** writer in the repository is
`crates/birdnet-web/examples/screenshot_server.rs:552`. So
`docs/book/images/notifications.png` shows a populated Notification Center that no
operator can ever reach.

### D5 — "Today · Top species" is not today

**[FIXED]** — the card reads `species_for_date`. An existing integration test was asserting the bug and now pins the contract.

`crates/birdnet-web/templates/today.html:110` renders eyebrow **"Today"** over
heading **"Top species"**. It is filled by `/pages/top-species` →
`top_species(conn, 6)`, which reads `species_summary` — a rollup keyed
`(Com_Name, Sci_Name, hour)` with **no date dimension at all**. It cannot be
today-scoped.

The shipped screenshot proves it: header reads "30 detections · 12 species", the
card beneath reads 1444 / 1332 / 1207. The weekly report's equivalent card
(`weekly_report.rs:215`) *is* correctly scoped via `weekly_top_species`, so this is
one card, not a pattern.

### D6 — `audit_log` grows forever

**[FIXED]** — `JOB_LOG_RETENTION`, daily, prunes `audit_log` at 180 days and `notification_log` at 90.

`AuditLog::prune()` (`crates/birdnet-db/src/accounts/audit.rs:142`) has no
production caller; only its own test at `:277`. `notification_log`'s prune is
reachable only when an operator loads `/admin/notifications`
(`routes/admin/notifications.rs:62`) — never on a headless station.

For contrast, and to be fair: `audio_levels::prune` **is** wired
(`src/integrations/acoustic_health.rs:202`) and `weather::prune_older_than_days`
**is** wired (`src/integrations/weather.rs:95`). The maintenance loop covers
integrity check, session prune, clip retention, species cap, and backup+VACUUM.
The two above are the ones that fell through.

### D7 — `species_summary` can drift and nothing will notice

**[FIXED]** — `JOB_SUMMARY_AUDIT`, daily, checks drift and rebuilds when it finds any.

`species_summary_drift` exists and is well tested, but its only caller is
`src/helpers/db.rs:188`, reached from `--rebuild-species-summary` on the command
line. The daily `PRAGMA integrity_check` cannot see logical drift between a table
and its trigger-maintained rollup. On a sealed station, the species list and every
count derived from it can go quietly wrong and stay wrong.

### D8 — 73.7 % of the database file is index

**[FIXED, and larger than it looked]** — migration 33. Three indexes nothing reads are dropped and two become partial: 164.6 MB (9.0 %) off the file, and the locked-clip read that runs every 60 s goes from a 267.6 ms full scan to 0.16 ms.

Measured with `dbstat` on the 3-year database (1.83 GB total):

```
detections (table)                 481.5 MB  26.3 %
idx_detections_unique              279.5 MB  15.2 %
idx_detections_species_hour_cover  197.6 MB  10.8 %
idx_detections_sci_first_cover     155.0 MB   8.5 %
idx_detections_date_species        111.8 MB   6.1 %
idx_detections_datetime             92.2 MB   5.0 %
idx_detections_sci_name             84.4 MB   4.6 %
idx_detections_species              75.4 MB   4.1 %
idx_detections_date                 62.6 MB   3.4 %
idx_detections_confidence           56.1 MB   3.1 %
idx_detections_source               46.1 MB   2.5 %
idx_detections_utc                  42.9 MB   2.3 %
idx_detections_review_verdict       30.2 MB   1.6 %
idx_detections_locked               29.6 MB   1.6 %
idx_detections_import_batch         29.6 MB   1.6 %
idx_detections_correlation_id       29.6 MB   1.6 %
idx_detections_chunk_offset         29.6 MB   1.6 %
-------------------------------------------------
ALL INDEXES                       1352.3 MB  73.7 %
```

Five of those index a column that is a single value or NULL for essentially every
row, and SQLite indexes NULLs at full width. As `WHERE`-clause **partial** indexes
they would cost near zero:

`idx_detections_locked`, `idx_detections_chunk_offset`,
`idx_detections_correlation_id`, `idx_detections_import_batch`,
`idx_detections_source` — together **~165 MB** and five B-tree writes on every
insert, on an SD card whose endurance the deployment runbook is already worried
about.

The indexing work in migrations 29/30 was clearly benchmarked and is good. This is
the accumulated tail nobody re-measured afterwards.

### D9 — Importing another station's history is a one-way merge

This is the direct answer to *"what happens if someone uploads historical
BirdNET-Pi data from a different station location?"*, traced end to end.

**What works.** `birdnet-migrate/src/provenance.rs` profiles the source (modal
coordinate, distinct sites, date range), compares it to the configured station,
flags >5 km as a different site, and warns before the import. The importer tags
every row with `import_batch_id` and records the batch. The importer is genuinely
robust about dirty upstream data (`lenient_f64`/`lenient_i64`).

**What does not.**

1. **No analytic filters on `import_batch_id`.** It is written, indexed, and synced
   to DuckDB, and then no query in `birdnet-db`, `birdnet-web`, `birdnet-timeseries`
   or `birdnet-behavioral` uses it. Life list, first-of-year, species richness,
   phenology, heatmap, correlation, dawn chorus and sessionisation all read the
   union of two sites as one station. `provenance.rs`'s own module doc names
   exactly this damage as its reason for existing — and it only warns *beforehand*.
2. **No undo.** There is no "delete this import batch" anywhere. The only
   destructive option offered is "delete ALL detections".
3. **The clock reconciliation is a single DST-naive scalar.**
   `routes/admin/migration.rs:157` computes `shift = here − src`, where `here` is
   `local_utc_offset_secs()` — **today's** offset — and `src` is one number the
   operator types. That one shift is applied uniformly across a multi-year history,
   so it is an hour wrong for roughly half of it whenever either station observes
   DST.
4. **Solar overlays use the configured coordinates** applied to the other site's
   detections, so sunrise markers are drawn for the wrong place.
5. After the import, the surviving signal is `/pages/provenance-note` — one
   sentence.

The dangerous property is the one the module doc already states: the damage is not
detectable after the fact. A merged dataset cannot be repaired, only discarded —
and right now it cannot even be discarded selectively.

### D10 — `iso_week` is not the ISO week

`crates/birdnet-behavioral/src/phenology/abundance.rs:29` —
`WEEK_EXPR = strftime(detection_date, '%W')`, aliased throughout as `iso_week`.

Probed against the real bundled DuckDB (and cross-checked against SQLite, which
agrees exactly — so this is **not** a cross-engine bug):

```
2024-12-30:  %W=53  %V(iso)=01  %Y=2024  %G=2025
2024-12-31:  %W=53  %V(iso)=01  %Y=2024  %G=2025
2025-01-01:  %W=00  %V(iso)=01  %Y=2025  %G=2025
2025-01-05:  %W=00  %V(iso)=01  %Y=2025  %G=2025
2025-01-06:  %W=01  %V(iso)=02  %Y=2025  %G=2025
```

Consequences, in a chart whose entire purpose is comparing week-of-year *across
years*: every year has a partial "week 00" (1–6 days) and often a partial week 53,
they are drawn at full height beside complete weeks, and one real week is split
across two years' charts. The effort join uses `%W` on both sides, so the ratio
within a bucket is consistent — only the buckets at the boundaries are.

`migration.rs:803` already records killing an earlier, worse week number. This is
the successor, and it is mislabelled rather than wrong.

### D11 — Three HTML escapers, two divergent, in code that says there is one

`routes/pages/mod.rs:272`'s doc comment reads: *"There were three, and they were
not the same… Escaping is not a place to have three answers."* Two remain, and both
omit the apostrophe the consolidated one added:

* `crates/birdnet-web/src/routes/admin/backup.rs:87`
* `crates/birdnet-web/src/routes/admin/logs.rs:194`

Neither call site interpolates into a single-quoted attribute today, so this is
latent, not exploitable — which is exactly the status the comment claims to have
retired. Confident prose in this repo, again.

### D12 — `clock.rs`'s stated premise is false for the shipping binary

`crates/birdnet-db/src/clock.rs`'s module doc: *"The workspace carries no
`chrono`/`time` dependency and forbids `unsafe`, so neither `localtime_r` nor a
tz-database parser is reachable."*

`cargo tree -e normal | grep -c "chrono v"` → **3**, via
`duckdb → arrow → arrow-arith → chrono`, which also pulls `iana-time-zone`.
`--no-default-features` → 0. Release binaries ship `analytics`, so **chrono and a
tz-database reader are already compiled and linked into every shipped binary.**

That does not make the SQLite-based approach wrong — asking SQLite guarantees the
offset agrees with how detections are stored, which is a real argument. It makes the
*reason given* wrong, and it means adding a proper date/time crate costs zero extra
build or binary weight in the shipping configuration. That changes the calculus for
everything in §3.

### D13 — Gates that pass by skipping

* `.github/workflows/ci.yml:141` — if the `behavioral` extension cannot be fetched
  from the community CDN, CI emits `::warning::` and the offline-load test
  **skips**. A CDN outage silently downgrades the gate to nothing.
* `crates/birdnet-behavioral/src/connection/live.rs` — the only tests that validate
  the generated SQL against the *real* published function signatures skip when the
  extension is unavailable. Nothing asserts that at least one of them ran.

This is precisely the failure mode `CLAUDE.md` warns about, in the gate that
protects the feature the project is named for.

---

## 2. Direct answers

**What are we missing?** Selective provenance (D9) — the ability to exclude or
delete an imported batch. A restore path that is safe (D1, D2). Retention on
`audit_log` (D6). A drift alarm on `species_summary` (D7). Effort normalisation
anywhere other than `/analytics/abundance` — migration 27's own comment argues that
raw counts across seasons measure the station as much as the birds, and then exactly
one chart applies the correction. Precision-by-confidence from `detection_reviews`:
the ground truth is being collected and used **only to hide rows**
(`detection_reviews.rs` exposes counts and nothing else). Weather is stored,
displayed as a temperature line on Today, and **never correlated with detections**.

**What am I not 100 % confident in?** Pi-class performance — every number here is
x86/NVMe with a warm cache; I have no Pi and did not extrapolate. The audio capture
and inference path: I read it, I did not run it, and nothing in this session
exercised a real microphone, a real model, or a multi-day run. Whether
`OOMPolicy=stop` + `Restart=always` actually recovers rather than latching failed —
I did not test it and would not assert either way. The `install.sh` / `uninstall.sh`
pair (122 KB and 16 KB of shell) got no review at all.

**Worst devex.** The build. 502 dependencies, 6 m 29 s for a workspace build on a
4-core box, 13 GB `target/`, and the bundled `libduckdb` C++ compile alone is ~6 min
on any cold touch of `birdnet-behavioral` (measured: 6 m 06 s to build one probe
test). Every contributor pays that before their first edit. `--no-default-features`
exists as an escape hatch but then the analytics half of the codebase isn't compiled
or linted, which is why CI has to lint both cfg branches.

**Worst performance.** D3 and its two victims, the History calendar (1 271 ms) and
the Life List (375 ms), both holding the global lock, both bypassing the analytics
cache that exists three modules away. `cached_fragment` is used by
`timeseries_dash`, `heatmap` and `correlation`; `grep -c cached_fragment` is **0**
for `history.rs`, `life_list.rs`, `species_pages.rs`, `year_in_review.rs`,
`today.rs` and `dashboard/`.

**What misses the engineering-excellence bar?** Not the things I expected. Build,
tests and clippy are all green. `unsafe` is forbidden. Non-test production code
across ~103 000 lines contains roughly **40** `unwrap()`s total — that is genuinely
disciplined, and my first, cruder count of "1 536" was wrong; I am correcting it
here. What misses the bar is narrower and consistent: **work declared finished that
was not** (D11, D12, and the "consolidation" comments generally), and **features
built end-to-end except for the last wire** (D4, D6, D7, D9's batch filter).

**What misses the production-ready bar?** D1 and D2 together. A station in a sealed
enclosure will eventually need a restore, and today both the button and the runbook
can fail silently. Everything else on this list degrades data or performance; those
two lose it.

**Documentation and Pages.** The infrastructure is good — one renderer for both the
site and the in-app `/help` tree, internal links checked against rendered HTML, the
build runs on PRs. Shortfalls: §8's restore procedure is unsafe (D2). §3's storage
table gives ~5–50 MB/day for detection rows; I measure ~557 bytes/row all-in, so
3 000 detections/day is ~1.7 MB/day — the table overstates by 3–30×, and nowhere
says a three-year station lands near 1.8 GB or that three quarters of it is index.
Screenshots are dated 17 Aug and predate the nav, health-badge and clock changes
that landed 20–21 Aug; `notifications.png` shows a screen no station can produce
(D4). External links are not checked at all.

**Which parts fail the bar for telling the public it is complete?** The Notification
Center (D4). Backup/restore (D1, D2). Cross-station import (D9) — it should say
"tags imports and warns", not "supports merging". Everything else I would ship and
document honestly.

**UI/UX journeys that are not complete.**

* *Restore.* `/admin/backups` does host both halves — a "download full backup"
  button and a restore form posting to `/admin/system/restore`
  (`backup_recovery.rs:305,314`) — and per D1 neither is safe. There is no
  `--restore-db` CLI flag to pair with `--backup-db`, so the headless recovery path
  is the runbook, which is D2.
* *Import.* Ends at a warning sentence. No batch list with a delete, no
  "exclude imported data" toggle on any chart.
* *Notification Center.* Empty forever.
* *Today.* The "LIVE SIGNAL · LAST 30 S" panel is the largest element above the fold
  and, when idle, is a blank rectangle with no empty state — the one place a new
  operator looks to answer "is it working?".
* *Today, again.* The "6 rare sightings are waiting for your eye" banner is styled
  in the alert/error register. Rare sightings are the good news.
* *Nav glyphs.* `⌂ ⌬ ▦ ♪ ¶` as bottom-bar icons. `⌬` is a benzene ring standing in
  for Species and `¶` a pilcrow for Reports; these are text glyphs whose rendering
  and metrics vary by platform font, and two of them carry no meaning to a general
  audience.

**Are the collapsed sections the right design?** I counted them rather than guessing:
**four** `<details>` elements in the whole app (`correlation.rs` ×2,
`behavioral.rs`, `admin/migration/render.rs`), plus a handful of CSS disclosure
patterns (`.bnb-add-form`, `.tsh-api-details`, `.pt-disc`). That is not a lot, and
where they are used — an add-form, an API reference, a methodology note — collapse
is the right call: secondary content that would otherwise push the primary content
below the fold. If the app *feels* full of collapsed sections, the cause is more
likely the sub-tab layer inside the six homes (Patterns has 6, Station has 6), which
hides content behind a click without the affordance of a disclosure triangle. That
is worth a hard look; the `<details>` are not.

**Are we reinventing the wheel?** Yes, and the specific answer matters more than the
general one.

* **Date and time: entirely hand-rolled.** No `chrono`, `time` or `jiff` in any
  manifest. `birdnet-core/src/civil.rs` is 830 lines implementing Hinnant's civil
  algorithms. A previous session consolidated *nine* copies of `days_from_civil`
  into it — good — but **`days_in_month`/leap-year logic still exists three more
  times** in `birdnet-scheduler/src/solar.rs:235`,
  `birdnet-web/src/routes/pages/history.rs:539` and
  `birdnet-web/examples/screenshot_server.rs:69`. And per D12, chrono is already in
  the binary.
* **Time zones: there is no time-zone handling at all**, only a single scalar
  "current offset" read from SQLite and cached for 60 s. That is not a bug in the
  function; it is a data model that cannot express a historical instant. It is why
  D9.3 exists and why migration 32 had to be written.
* **HTML escaping: three implementations** (D11).
* Two `backoff_delay`s (`birdnet-integrations/src/retry.rs:37`,
  `src/capture/supervisor.rs:118`), two `url_encode`s.
* Justified hand-rolls, for balance: `AnalyticsCache` (a `Mutex<HashMap>` instead of
  `moka`), the xorshift PRNG in the screenshot fixture, and the solar solver — all
  small, all with a stated reason, none duplicated.

**What is unverified / unprobed?** The live behavioral-extension signature tests
skip silently (D13). No test asserts the shipped `full_backup` archive can actually
be restored — a round-trip test would have caught D1 immediately. Nothing exercises
a DST transition end-to-end through capture → filename → insert → query, though
migration 32 and `session_clock.rs` cover pieces. No long-running soak: no evidence
in the repo of a multi-day run, an SD-card-full run, or a clock-step (NTP jump) run.
I found no test that opens the app against a multi-million-row database — every DB
test I read builds tens or hundreds of rows, which is why D3's cost only shows up
when you build the big one on purpose.

**What should be done differently?** Three things.

1. **Split the connection.** One writer, a small pool of readers. WAL already
   supports it; the code throws it away. Nothing else on this list is worth as much
   per line changed.
2. **Stop shipping two implementations of the same operation.** Backup has two
   (D1). Escaping has three (D11). Week-of-year has one implementation and two
   names (D10). Each divergence was introduced by a change that fixed the important
   copy and left the others.
3. **Make "declared done" mean tested-done.** D4, D6, D7 and D9's batch filter are
   all the same shape: the mechanism was built and the last wire was never
   connected, and in each case a single assertion would have caught it —
   "`log_notification` has at least one caller", "`audit_log` shrinks after the
   maintenance tick", "the drift check runs on a schedule", "an import batch can be
   excluded from a query".

---

## 3. Suggested order of work

| # | Item | Why first |
|---|---|---|
| 1 | D2 — fix the runbook (`rm -f *-wal *-shm`) | One line, prevents data loss today |
| 2 | D1 — route `full_backup` through the online-backup API; route `restore_backup` through `resilience::restore_from_backup`; add a round-trip test | The backup you cannot restore is not a backup |
| 3 | D4 — either wire `log_notification` or delete the three surfaces | A permanently empty page is worse than no page |
| 4 | D5 — scope or relabel "Today · Top species" | Visible wrong number |
| 5 | D6, D7 — add `audit_log` prune and a drift check to the maintenance loop | Two entries in an existing loop |
| 6 | D3 — reader pool | Largest win, largest change |
| 7 | D9 — batch filter + batch delete | Makes cross-station import honest |
| 8 | D8 — partial indexes on the five sparse columns | ~165 MB and five B-tree writes per insert |
| 9 | D10, D11, D12 — rename `iso_week`, finish the escaper consolidation, correct `clock.rs`'s premise | Cheap, and each removes a future trap |
| 10 | D13 — fail CI when the extension cannot be fetched; assert a live test ran | Restores a gate that currently passes for free |

Per `CLAUDE.md`: every gate written for these must be observed failing against the
code it was written for, and the commit must say how.
