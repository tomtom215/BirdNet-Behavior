# Production audit — permanent unattended deployment

**Audited:** 2026-08-17, against `main` tip `f025cbd` (merge of PR #212), the
`v0.14.0` release plus five commits.

**Question this audit asks.** Not "does it work" — it does — but: *if this station
is sealed into an outdoor enclosure and left for a year with nobody on site, what
does it get wrong, and would anybody find out?*

That framing changes what counts as a defect. A wrong number on a dashboard is a
bug on a desk and a corrupted season in a field. Anything that fails silently
ranks above anything that fails loudly, because a station nobody is watching only
ever reports what it volunteers.

> **Where this document now stands.** This is the 2026-08-17 pass, written
> against `f025cbd`. It was superseded twice — first by
> [`FIELD_READINESS_AUDIT.md`](FIELD_READINESS_AUDIT.md) §6, then by
> [`POST_0140_AUDIT.md`](POST_0140_AUDIT.md) — and its findings were worked
> through over the releases that followed. Re-checked against `main` at `ee795ed`
> (v0.15.0) on 2026-09-04: fifteen of the sixteen numbered findings are fixed,
> three of them with a residue named in place (A-3's auto-update alerting, A-6's
> solar overlays, A-7's prose-vs-unit CI check), and **A-5 is partly fixed** —
> two of nine phenology builders now have production consumers. The body is
> preserved as the record of what was wrong and why nothing noticed. Per-finding
> statuses and any statement a reader might still act on have been corrected in
> place; nothing else has been rewritten into the present tense.

**Status.** Fifteen findings are fixed, each with gates observed failing against
the code they were written for; A-5 is partly fixed, and its current split is
recorded in that section. The narrative is kept in the past tense deliberately:
the value of an audit is partly the record of what was wrong and why nothing
noticed, which a rewritten-to-present document loses.

**Method.** Every claim below was produced by running something — a gate, a
probe, a query plan, or a real browser. Where a previous
document's claim was re-checked rather than carried forward, that is stated. Two
findings in this pass (**A-1**, **A-2**) were invisible to a fully green
2190-test suite, which is the point of writing it down: a green gate proves only
what it executes.

---

## 0. What was actually run

x86_64 Linux, 4 cores, 15 GB RAM, from a cold `target/`.

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --workspace --all-targets` | **exit 0**, 0 warnings |
| Tests (baseline) | `cargo test --workspace --all-features` | **exit 0** — 46 suites, **2190 passed**, 0 failed, 5 ignored |
| Docs site | `curl` over the published GitHub Pages tree | **200** on index, theme CSS, and every sampled chapter |
| Day-boundary behaviour | SQLite `date()` under five real timezones, both skew directions | **skew reproduced** — see A-2 |
| Divergence behaviour | new `tests/analytics_divergence.rs` against unmodified code | **4 of 4 red** — see A-1 |

The exit code matters here. An earlier capture in this session used
`cargo test … \| tail -80`, which reports `tail`'s status, not the suite's —
exactly the trap `CLAUDE.md` warns about. The 2190/0 figure above comes from a
run that captured the real exit code.

**Verdict.** The engineering substrate is strong and mostly deserves its
reputation: the systemd unit is genuinely hardened (`CapabilityBoundingSet=`
empty, `SystemCallFilter` minus eight groups, watchdog gated on the detection
loop still *cycling*, so a hung pipeline is restarted rather than reported healthy
— see the retraction in §2 for what that does *not* prove), the detection deadman
closes the "everything is green and nothing is being detected" hole that most
stations have,
the disk/network/DB failure paths have fault-injection tests, and the docs are
better than most commercial products'.

What follows is what a year of unattended operation would expose anyway.

---

## 1. Findings

**P1** — the station silently records or reports something false.
**P2** — degrades or misleads, recoverable. **P3** — polish, latent, or doc-only.

| ID | Finding | Sev | Status |
|----|---------|-----|--------|
| A-1 | Operator edits never reach the analytics store, permanently and silently | **P1** | **fixed this pass** |
| A-2 | Five queries ask UTC for "today" while every detection is stamped in local time | **P1** | **fixed this pass** |
| A-3 | Operational alerting is one-dimensional: only "no detections at all" ever notifies | P2 | **fixed** |
| A-4 | Reviewer verdicts are collected and then applied to nothing | P2 | **fixed** |
| A-5 | The `phenology` module — 925 LOC, 12 exports — has no production consumer | P2 | **partly fixed** — 2 of 9 SQL builders wired |
| A-6 | Importing another station's history reconciles neither its location nor its clock | P2 | **fixed** |
| A-7 | The field runbook is not on the docs site, and its memory ceiling is wrong | P3 | **fixed** |
| A-8 | Live and synced `DuckDB` rows carry different columns | P3 | **fixed** |
| A-9 | The dawn-chorus window scanned the whole history, so it slowed every season | P2 | **fixed this pass** |

Interface findings are numbered separately in §1b:

| ID | Finding | Sev | Status |
|----|---------|-----|--------|
| U-1 | The visual-QA tooling has never rendered the mobile layout — every "mobile" screenshot is desktop | P2 | **fixed** |
| U-2 | `pointer: coarse` denies the phone layout to tablets-with-trackpads and narrow desktop windows | P2 | **fixed** |
| U-3 | Half the Patterns tabs are off-screen on a phone with no affordance that they scroll | P2 | **fixed** |
| U-4 | Chart series colours are a hash-to-hue at fixed lightness; collisions are near-certain | P2 | **fixed** |
| U-5 | The streamgraph has no axes, and the caption above it describes a different chart | P3 | **fixed** |
| U-6 | "Bursts of singing" lists sessions of 1 detection lasting 0s | P3 | **fixed** |
| U-7 | 243/335 controls under 44 px; lock/delete/download clipped at ≤360 px | P3 | **fixed** |

---

### A-1 — Operator edits never reach the analytics store · **P1** · fixed

**The most consequential finding in this pass, and the whole 2190-test suite was
green through it.**

`SQLite` is the source of truth; `DuckDB` is a derived copy that every
behavioural and time-series dashboard reads. New detections reach both —
`src/daemon/processor.rs` inserts into `DuckDB` right after the `SQLite` write
(that call sits at `:685` today; it was at `:274` when this ran). Four ordinary
operator actions did not:

| Action | Route | What it called |
|---|---|---|
| Delete a detection | `/pages/today-delete`, `/pages/recordings-delete` | `delete_detection` — `SQLite` only |
| Re-label a detection | `/pages/today-relabel` | `relabel_detection` — `SQLite` only |
| Clear all data | Admin → System → Clear detections | `DELETE FROM detections` — `SQLite` only |
| Approve a quarantined detection | `/pages/quarantine-*` | `approve_quarantine` — `SQLite` only |

**Nothing reconciled the difference afterwards.** The startup sync
(`crates/birdnet-behavioral/src/connection/sync.rs:43`) is *incremental*: its
cutoff is the newest row already in `DuckDB`, so it can only ever add newer rows.
It never removes one, never re-reads a changed one, and skips a back-dated one
entirely. `full_resync_from_sqlite` was the only repair, and it was reachable
from exactly one place in the product: finishing a BirdNET-Pi migration
(`crates/birdnet-web/src/routes/admin/migration.rs:319`). No CLI flag, no admin
button, no automatic check.

**Measured, not inferred.** `tests/analytics_divergence.rs` was written first and
run against unmodified code:

```
test result: FAILED. 0 passed; 4 failed
  deleting_a_detection_removes_it_from_the_olap_copy      left: 3, right: 2
  relabelling_a_detection_updates_the_olap_copy           left: 1, right: 0
  approving_a_quarantined_detection_admits_it_to_the_olap_copy  left: 3, right: 4
  clearing_detections_clears_the_olap_copy                left: 3, right: 0
```

**Field impact, in order of severity.**

- **Clear all detections** is the worst: the dashboard reports zero detections
  while Patterns, Analytics and Time-series render the station's entire history
  beside it. An operator who wipes a test period before sealing the enclosure
  carries that period forever, and would reasonably read "cleared" as *cleared*.
- **Approving a quarantined detection** could never be repaired even in
  principle. The quarantined row keeps its original timestamp, so it is
  back-dated relative to whatever `DuckDB` holds and the `>= cutoff` filter skips
  it on every future start. Every detection an operator rescued from quarantine
  was invisible to behavioural analytics permanently.
- **Delete** and **re-label** are the routine ones. Curating false positives is
  the main thing an operator *does* on a long-running station; none of it reached
  the analytics.

**Why no gate caught it.** Every test builds its fixture and syncs, then reads.
None mutated after syncing. Both stores answered every query they were asked —
just with different histories — so there was no error to notice, and
`/analytics/status` reported `DuckDB`'s count without ever comparing it to
`SQLite`'s.

**Fix (three parts; the third is what closes the class).**

1. `AnalyticsDb` gains `delete_detection`, `relabel_detection` and
   `clear_detections` mirrors (`sync.rs`).
2. `AppState` gains paired writes — `delete_detection`, `relabel_detection`,
   `approve_quarantine`, `clear_detections` — and the four routes call those
   instead of reaching past them to the raw `SQLite` helper. A mirror failure is
   logged rather than returned: the authoritative write has already happened.
3. **Startup drift repair.** After the incremental sync, the two row counts must
   agree; when they do not, something reached `SQLite` that the copy can never
   catch up to, and a full rebuild runs automatically. This costs two `COUNT(*)`s
   per start on a healthy station and — the point — self-heals every station
   already running a release that wrote to `SQLite` alone. No operator action, no
   new CLI surface, nothing to notice.

**Gates.** `tests/analytics_divergence.rs`, seven tests at two levels: four
contract tests on `AppState`, two that drive the real HTTP handlers (a new route
that reaches for `with_db(|c| birdnet_db::sqlite::…)` compiles and passes the
contract tests — only the route-level gate catches it), and one that creates an
already-diverged station and asserts the next start heals it. Each was observed
red against the code it was written for; the route gate was re-checked by
reverting `today.rs` to the old call and watching it fail.

---

### A-2 — "Today" means UTC's today, not the station's · **P1** · fixed

Every detection's `Date` is **local civil time**: capture stamps recording
filenames from the system's local clock and detection rows inherit it.
`crates/birdnet-db/src/clock.rs:8` states the rule in as many words — queries
must ask `date('now','localtime')`.

Five queries asked `date('now')`, which is UTC:

| Site | Query | Consequence |
|---|---|---|
| `routes/feeds.rs:96` | `WHERE Date = date('now')` | the RSS/iCal "today" feed |
| `routes/pages/dawn_chorus.rs:94` | `WHERE Date >= date('now', ?1)` | dawn-chorus rolling window |
| `routes/pages/migration.rs:651` | `WHERE Date >= date('now','-7 days')` | migration 7-day window |
| `queries/species.rs:251` | sparkline data window | species sparklines |
| `queries/species.rs:275,277` | sparkline **date axis** | species sparklines |

**Measured in both directions**, against real tzdata at a pinned UTC instant:

```
TZ=America/New_York   UTC 02:00 -> date('now')=2026-08-17  local=2026-08-16   UTC AHEAD
TZ=Pacific/Auckland   UTC 12:00 -> date('now')=2026-08-17  local=2026-08-18   UTC BEHIND
```

- **West of UTC** (the Americas) the UTC day rolls over during the local evening,
  so `WHERE Date = date('now')` matches a date no row carries yet: **the today
  feed is empty for the last hours of every evening.** New York loses 20:00 to
  midnight; Los Angeles loses 17:00 to midnight.
- **East of UTC** the local date runs ahead, so "today" is still yesterday. A
  UTC+13 station is a day behind for most of its day.
- The sparkline case is worse than a window shift: the *axis* is built from UTC
  dates and joined against locally-dated counts, so the two are keyed to
  different days regardless of window width.

**Fix.** All five sites now use `date('now','localtime')`, matching the
convention documented in `crates/birdnet-db/src/clock.rs` and used by the
retention cutoffs in `src/maintenance.rs:776`. At `ee795ed` the only bare
`date('now')` left in the tree are a benchmark
(`crates/birdnet-db/benches/db_queries.rs:248`) and a comment
(`src/maintenance.rs:424`).

**Gate.** `tests/local_day_boundary.rs`. SQLite's `localtime` reads the process
timezone through libc and `std::env::set_var` is `unsafe` in edition 2024 (which
this workspace forbids), so the test re-executes its own binary with `TZ` set.
The offset is picked from the current UTC hour — UTC+14 from 10:00 UTC onward,
UTC−12 before 12:00 UTC — so the local and UTC dates are guaranteed to differ
*whenever CI runs it*, and a fixture guard asserts they really do differ rather
than letting the test pass inert. The feed test drives the real HTTP handler; an
earlier draft re-implemented the query and passed for that reason, which is
exactly the failure mode `CLAUDE.md` warns about.

---

### A-3 — Only one thing can ever page you · P2 · fixed

For an enclosure in a field, the honest question is: *which failures reach a
human?* Exactly one does.

**What alerts.** `src/integrations/deadman.rs` — the detection deadman. It polls
detection freshness every 5 min, publishes
`birdnet_detection_silence_seconds`, and after a threshold (default 24 h) logs
loudly and sends one Apprise notification per silence episode with a recovery
notice. This is good, and it closes the biggest hole: "every component gauge is
green and the station is detecting nothing."

**What does not alert.** Everything else. Grepping the Apprise call sites finds
exactly three: per-detection species notifications
(`src/daemon/processor.rs:412`), the weekly report, and the deadman. So none of
these reaches anybody:

- an audio source down while others keep working — the station keeps detecting,
  the deadman stays quiet, and one of three microphones has been dead for a month
  (`birdnet_audio_source_up` gauge only);
- the disk purger deleting recordings to stay under threshold
  (`capture/disk/purge.rs:85`, `tracing::warn!`);
- a failed `SQLite` integrity check, or a failed weekly backup;
- the analytics database being quarantined and rebuilt
  (`connection/mod.rs:515`, `tracing::warn!`);
- CPU thermal throttling — the temperature *is* sampled
  (`system_info.rs:122`) and shown on the System page, but nothing watches it;
- an auto-update that failed or rolled back.

All of it is in the journal or on `/metrics`. On a station nobody logs into and
no Prometheus scrapes, the journal is a diary written for nobody. **The gap is
not instrumentation — it is that the notifier is wired to one condition.**

**Recommendation.** A station-health notifier alongside the deadman, sharing its
episode semantics (one alert per episode, recovery notice, never re-fire per
poll), covering: source down > N min, disk above purge threshold, integrity or
backup failure, analytics quarantine, sustained thermal throttling. Roughly one
new module of the same shape as `deadman.rs`, which is the right precedent to
copy.

> Since done: `src/integrations/station_health.rs` is that module — *"the
> conditions that end a season, other than silence"* — spawned beside the deadman
> at `src/app.rs:386`. It runs six checks (`station_health.rs:222`): `sources`,
> `disk`, `thermal`, `maintenance`, `quarantined-stores` and `clock`, sharing the
> deadman's episode semantics. `check_maintenance` (`:523`) reads each job's
> recorded *verdict* rather than its timestamp, which is what makes a backup that
> fails every week visible: `mark_ran` used to refresh the timestamp on failure
> too. The one item on the list above it does not cover is a failed or
> rolled-back auto-update.

---

### A-4 — Curation is collected and applied to nothing · P2 · fixed

`detection_reviews` (migration 13) stores a `confirmed` / `rejected` verdict per
detection, non-destructively and idempotently. The review queue is a real UI with
a real workflow.

Grepping every consumer: the verdicts are read by **one** surface — the quality
dashboard's own "Review verdict trend" panel
(`queries/analytics.rs:352`, `review_verdict_trend`). No other analytic joins the
table.

So a rejected detection still counts in the species list, the life list, the
heatmap, the dawn chorus, phenology, every behavioural analytic and every
time-series query. An operator can spend a season rejecting false positives and
every chart will look exactly as it did before. The only way to make a rejection
*mean* something today is to delete the detection instead — which discards the
evidence, and (until A-1) silently disagreed with the analytics.

This is the single largest gap between "a system that logs bird detections" and
"a system whose numbers a naturalist would cite."

**Recommendation.** A display preference — *"exclude rejected detections from
analytics"* — honoured by a shared predicate applied at the query layer in both
stores, defaulting to on. It needs the verdict mirrored into `DuckDB` (the
mechanism A-1 just built) and one filter clause threaded through the query
builders.

> Since done, as one view rather than one preference. Migration 26 denormalises
> the verdict onto `detections` and adds `detections_analytic` — `SELECT * FROM
> detections WHERE review_verdict IS NOT 'rejected'`
> (`crates/birdnet-db/src/migration.rs:769`) — with the `DuckDB` twin spelling
> the same rule as `review_verdict IS DISTINCT FROM 'rejected'`
> (`crates/birdnet-timeseries/src/queries/mod.rs:97`). The verdict is mirrored
> into the analytics copy by `AppState` (`state.rs:949`), which is the A-1
> mechanism doing the work. Migration 34 later added a second clause to that
> view, for import provenance; see the note under A-6. Two surfaces do **not**
> read the view, and both carry only the verdict half of it: the dawn chorus
> spells the predicate out inline because `INDEXED BY` is not valid against a
> view (`dawn_chorus.rs:119-130`), and `species_summary` is maintained by trigger
> (`migration.rs:960-967`). For rejections that is the whole rule and both are
> correct; for imports it is not — see A-6.

---

### A-5 — A whole analytics module nothing calls · P2 · partly fixed

`crates/birdnet-behavioral/src/phenology/` is 925 lines across four files,
exporting 12 symbols, with its own executing test suite
(`tests/phenology_execute.rs`).

Checked symbol by symbol: **every one has zero consumers outside the module and
its own tests.**

```
effort_corrected_abundance_sql : 0    first_detection_sql   : 0
monthly_totals_sql             : 0    interannual_trend_sql : 0
peak_weeks_sql                 : 0    phenology_timing_sql  : 0
weekly_abundance_sql           : 0    migration_window_sql  : 0
weekly_richness_sql            : 0
```

The web Migration page computes its own phenology from `SQLite` directly and
never touches this module.

The sharpest instance: `effort_corrected_abundance_sql`. Detection *counts* are
meaningless without recording effort — a windowed schedule, a week of downtime,
or a mic outage all move counts without moving abundance, and for a station meant
to run a full season that correction is the difference between a trend and an
artefact. The analytic exists, is tested, and is connected to nothing.

Alongside it, two correctness caveats — latent today precisely *because* the
module is unreachable, and blockers for wiring it up:

- **Year-crossing species.** `GROUP BY Com_Name, year` with
  `MIN`/`MAX(detection_date)` means an overwintering species (present
  Oct–Mar) reports `first_doy ≈ 1` and `last_doy ≈ 365` — "arrived 1 January,
  departed 31 December". The migration-window estimate is only meaningful for
  species absent across the calendar boundary, which is not stated anywhere.
- **`presence_days` is a span, not presence.** `last − first + 1` gives 365 to a
  species detected once in January and once in December. `min_detections`
  mitigates; it does not fix.

**Recommendation.** Decide, and act either way: wire it up (starting with
effort-corrected abundance, and fixing the two caveats first) or delete it.
Carrying tested, documented, unreachable code is the thing that makes an
"is this feature real?" question unanswerable from the outside.

> Since partly done — **two of the nine SQL builders are reachable, seven are
> not.** `effort_corrected_abundance_sql` and `phenology_timing_sql` are called
> from `crates/birdnet-web/src/routes/analytics.rs:692` and `:715`, behind
> `/analytics/abundance` and its sibling. Grepping `crates/`, `src/` and
> `tests/`, excluding the module and its own tests, the other seven still return
> nothing: `first_detection_sql`, `monthly_totals_sql`, `interannual_trend_sql`,
> `peak_weeks_sql`, `weekly_abundance_sql`, `migration_window_sql`,
> `weekly_richness_sql`.
>
> Both caveats above were closed by disclosure rather than by arithmetic, which
> is the right call for a span: `migration_window_sql` now returns
> `year_crossing` — true when a species was detected in both the first and last
> fortnight of the year, exactly when a calendar-year window stops describing a
> migration — and reports `detected_days` beside `presence_days` so the span and
> the occupancy cannot be confused. Both are documented at
> `crates/birdnet-behavioral/src/phenology/timing.rs:150-168`. The leap-day skew
> recorded in §3 was fixed in the same file, at `:36-76`.

---

### A-6 — Importing another station's history reconciles nothing · P2 · fixed

*Directly answering: "what happens if someone uploads historical BirdNET-Pi data
from a different station location?"*

**Verified by reading the importer and validator end to end.** The answer is: it
imports cleanly, reports success, and every location- and clock-dependent
analytic silently becomes a blend of two stations.

`BirdNetPiValidator::validate_source` runs exactly four checks
(`crates/birdnet-migrate/src/birdnet_pi/validator.rs:19`):

1. `detections` table readable
2. non-empty
3. `Date`/`Time` name a real point in time
4. confidence within `[0, 1]`

There is **no check involving `Lat`/`Lon` at all**, and none involving the
timezone. The importer copies per-row `Lat`/`Lon` through verbatim
(`importer.rs:212`), so imported rows keep the *source* station's coordinates
while the merged history is analysed against *this* station's settings. The
migration UI shows no warning.

What is actually wrong afterwards:

- **Clock.** BirdNET-Pi's `Date`/`Time` are local wall-clock with no offset
  recorded. Rows imported from another timezone are re-read as this station's
  local time. Every hour-of-day analytic — dawn chorus, hourly heatmap, activity
  streamgraph, sessionization — mixes two clocks with no marker. A 6-hour import
  offset puts the source station's dawn chorus in this station's afternoon.
- **Solar context.** Sunrise/sunset overlays and the recording-window logic use
  the *configured station* lat/lon, applied to detections recorded somewhere
  else.
- **Species plausibility.** The occurrence filter runs at detection time using
  station coordinates. Imported rows bypass it — correctly, they were filtered by
  the source station — so the merged dataset carries two different range-filter
  regimes with nothing recording which rows came from where.
- **Firsts and life list.** "First-of-year arrivals" and life-list firsts are
  computed over the merged table, so another site's species become this
  station's records.

Note the schema has no provenance column: after import there is no way to tell an
imported row from a locally-recorded one, so this is not repairable after the
fact.

**Recommendation, cheapest first.**

1. **Validate and warn.** The source database's `Lat`/`Lon` are already read.
   Compare the modal source coordinate against the configured station location
   and, past a threshold (say 25 km), show a pre-import warning naming the
   distance and listing exactly what will be wrong. This is a validator check
   plus a UI string, and it turns a silent corruption into an informed choice.
2. **Record provenance.** A nullable `Source_Station` (or reuse `Source`) stamped
   at import makes every later decision possible — filtering, per-site analytics,
   or undo — and costs one migration.
3. **Offer a clock offset at import.** A single "these recordings were made at
   UTC{±N}" field, applied during import, fixes the hour-of-day class outright.

Until then the honest position is to say so in the migration guide, which at the
time did not mention location or timezone at all.

> Since done, with one half outstanding. All three recommendations shipped, and
> the threshold shipped tighter than the one floated above: `DIFFERENT_SITE_KM`
> is **5 km, not 25** (`crates/birdnet-migrate/src/provenance.rs:52`), checked
> before the import from `crates/birdnet-migrate/src/birdnet_pi/mod.rs:183`.
> Provenance is recorded as `import_batch_id`
> (`crates/birdnet-db/src/migration.rs:701`), which is what later made an import
> removable and excludable — migration 34 puts `AND (import_batch_id IS NULL OR
> NOT EXISTS (SELECT 1 FROM settings WHERE key = 'analytics_exclude_imports' AND
> value = 'true'))` into `detections_analytic` (`migration.rs:1325-1330`), with
> the `DuckDB` twin in `crates/birdnet-behavioral/src/queries.rs:117-124`. The
> clock is converted per row from `source_utc_offset_secs` (`provenance.rs:263`)
> rather than by a flat shift, and the guide now opens on both facts
> (`docs/book/guides/migration.md:25`, `:43`).
>
> **Still open: the solar overlays.** `solar_times_local`
> (`crates/birdnet-web/src/routes/pages/mod.rs:375`) reads only the
> `latitude`/`longitude` settings, so sunrise and sunset markers drawn over
> imported rows are still this station's, not the source's. No per-batch
> coordinate is consulted anywhere.
>
> **And the provenance clause reaches only what reads the view.** The dawn chorus
> (`dawn_chorus.rs:126-130`) and `species_summary`
> (`crates/birdnet-db/src/sqlite/queries/species.rs:44`, `:472`;
> `migration.rs:960`) carry the verdict half of the predicate and not the
> provenance half, so with `analytics_exclude_imports` set they still count the
> excluded batch.

---

### A-7 — The field runbook is not on the docs site · P3 · fixed

`docs/FIELD_DEPLOYMENT.md` is the best document in the repository and the single
most relevant one to a permanent outdoor install: hardware, power and thermals,
storage sizing, networking, hardening, time sync, watchdog smoke tests, remote
diagnostics, update strategy, a pre-flight checklist and a recovery runbook.

It is **not in `SUMMARY.md`**, so it is not part of the published book. Verified:
`https://tomtom215.github.io/BirdNet-Behavior/FIELD_DEPLOYMENT.html` → **404**.
It is reachable only as a raw GitHub blob link from two book pages and the
README. The same is true of `SECURITY_HARDENING.md`, `HARDWARE_TEST.md`,
`MULTISTREAM_DEDUP.md` and `MACOS.md`.

The docs site therefore has an "Administration" section and no chapter on
*operating a permanent unattended station* — the project's headline use case.

And the runbook has drifted from the code it documents:

| `docs/FIELD_DEPLOYMENT.md:164` | `installer/lib/65-service.sh:122` |
|---|---|
| `MemoryMax=512M` | `MemoryMax=1G` (plus `MemoryHigh=768M`, undocumented) |

A 2× error in the memory ceiling, in the document an operator uses to decide
whether a board has enough RAM.

**Recommendation.** Promote the four operational documents into a *Field
Operations* part of the book (they are already Markdown; this is a `SUMMARY.md`
edit and link fixups), and add a CI check that the unit-file limits quoted in
prose match `65-service.sh` — the same shape as the existing installer sync-gate.

> Since done: the five runbooks now live in the book itself, under
> `docs/book/field/`, as the *Running a Permanent Station* part
> (`docs/book/SUMMARY.md:58-62`). The paths named above are where they were when
> the audit ran; none of them exists at the old location any more. The memory
> ceiling agrees too — `docs/book/field/deployment.md:180` quotes
> `MemoryHigh=768M`, `MemoryMax=1G`, which is what
> `installer/lib/65-service.sh:145-146` sets.
>
> **The recommended CI check was not built.** No workflow and no test compares
> the prose against `65-service.sh`. `scripts/hardening-check.sh:16` opens *"#
> Directives reproduced, from installer/lib/65-service.sh:"*, but it compares
> runtime sandbox behaviour rather than the quoted numbers — and no
> `.github/workflows/*.yml` invokes it, so the script is itself an unrun gate.
> The two sides agreeing today is exactly when the gate is cheapest to add.

---

### A-8 — Live and synced `DuckDB` rows carry different columns · P3 · fixed

`AnalyticsDb::insert_detection` (the live path) writes 6 columns; `SYNC_COLS`
(the bulk path) writes 12. Rows written live therefore have NULL `Lat`, `Lon`,
`Cutoff`, `Week`, `Sens`, `Overlap`, and the same detection gets different
contents depending on whether it arrived live or via a resync — including the
new drift rebuild, which will now *change* those columns on stations that trigger
it.

Checked: no `DuckDB`-side query reads any of the six, so nothing was wrong at the
time. It is recorded because it is a trap for the next analytic that wants
`Lat`/`Week`, and because "the same row means different things depending on how
it got here" is not a property to leave undocumented.

> Since done: the live insert writes the same thirteen columns as the bulk path,
> `Lat`/`Lon`/`Cutoff`/`Week`/`Sens`/`Overlap` among them
> (`crates/birdnet-behavioral/src/connection/sync.rs:400-402`).
> `import_batch_id` and `review_verdict` are the two deliberate exceptions,
> documented at `:389-392`: a live detection is by definition this station's own
> and unreviewed, so offering them as parameters would invite a caller to say
> otherwise.

---

### A-9 — A 30-day question that read four years of history · P2 · fixed

*Answering: "what is our worst performance?"*

Measured against a synthetic four-year station — 1 095 361 rows, 277 MB, the
size a BirdNET-Pi import produces — on x86_64, 4 cores:

| Query | Warm |
|---|---|
| Detections page (newest 50) | **< 1 ms** |
| 7-day sparklines | **1 ms** |
| Today feed | **< 1 ms** |
| Heat map, day-of-week × hour | 992 ms |
| Species list, lifetime totals | 1453 ms |
| Dawn chorus, **30-day** window | **1292 ms** |
| Seasonal phenology, all history | 2370 ms |

Indexed lookups are excellent. The aggregates are slow, and most of them are
*inherently* slow: a day-of-week × hour heat map over all history has to read all
history, and the 10-minute fragment cache plus the background pre-warmer are the
right answer for those.

**The dawn chorus is not in that category, and that is the finding.** It asks a
30-day question and was reading everything, because SQLite chose
`idx_detections_species` (to get `Com_Name` in GROUP BY order) over the perfectly
good `idx_detections_date_species` — then built the temp b-tree anyway, since
`hr` is an expression, so the choice bought nothing. Its cost therefore scaled
with total history rather than with the window:

```
history   60d (   45,943 rows) ->      72 ms
history  365d (  273,349 rows) ->     396 ms
history  730d (  546,625 rows) ->     810 ms
history 1460d (1,095,361 rows) ->    1711 ms
```

A permanent station's dawn chorus got measurably slower every season it ran. On
Pi-class hardware, multiply through.

`ANALYZE` does not change the plan — checked, not assumed; the planner is not
short of statistics, it is preferring index-order for the GROUP BY. Adding
`INDEXED BY idx_detections_date_species` takes it to a range seek:

```
as shipped                    1613 ms   SCAN … idx_detections_species
INDEXED BY (Date, Com_Name)     27 ms   SEARCH … (Date>?)
```

**60× faster, byte-identical results** (2097 groups both ways).

**Gate.** `dawn_chorus_window_uses_a_date_range_seek` asserts the *query plan*,
not a duration — a timing threshold on shared CI hardware is a flaky test, and
the plan is what regressed. Two rows suffice: the planner makes the same wrong
choice on a two-row table as on a million (verified both ways).

The first draft of that gate was worthless and the revert check is what proved
it. It re-typed the SQL into the test, so reverting the production query left it
green. The query now lives in a `CHORUS_SQL` const that both the handler and the
`EXPLAIN` read, and with the hint removed from *that* the gate fails as it
should. This is the second time in one session that a test passed for a reason
unrelated to what it claimed to assert.

---

## 1b. Interface findings

Every page was rendered in a real headless Chromium against the seeded
`screenshot_server` fixture, at 1440×900 and at four phone widths, in both
themes. The structural sweep — horizontal overflow, stuck loaders, broken
images, console errors, HTTP status — came back **clean on all 17 pages × 2
viewports × 2 themes**. The desktop interface is genuinely good: editorial
typography, real hierarchy, a calm palette, and empty states that say something.

The findings below are what that sweep cannot see.

### U-1 — The project has never screenshotted its own mobile layout · **P2**

The phone layout is gated on:

```css
@media (max-width: 720px) and (pointer: coarse) { … }
```

`scripts/visual_qa.mjs` and the `tools/visual-qa` suite set only a viewport
(`{width: 375, height: 812}`). Headless Chromium with a viewport but no
`hasTouch`/`isMobile` reports **`pointer: fine`**, so that media query never
matches. Measured, same page, same width:

| Context | `pointer: coarse` | bottom tab bar | top nav links | chrome above `<h1>` |
|---|---|---|---|---|
| viewport only — *what the tooling does* | `false` | `display: none` | visible | **231 px** |
| viewport + `hasTouch` — *a real phone* | `true` | `display: grid` | hidden | **160 px** |

So every "mobile" screenshot this project has ever taken — including in the
**A11y & Visual QA CI workflow** — is the *desktop* layout squeezed into a phone
viewport. The actual phone UI, bottom tab bar and all, has never been in front of
the gate that exists to check it. (It is good, incidentally; that is not the
point.)

**Fix.** Add `hasTouch: true, isMobile: true` to the mobile contexts in
`scripts/visual_qa.mjs` and `tools/visual-qa/*.mjs`. One line each, and it turns
an inert gate into a real one.

### U-2 — `pointer: coarse` denies the mobile layout to people who need it · P2

The same gate has a user-facing half. `pointer: coarse` is a property of the
*input device*, not the screen, so these all get the desktop nav in a phone-sized
window:

- an iPad with a keyboard/trackpad attached — **measured**: `pointer: coarse` is
  `false`, tab bar `none`, top nav visible;
- a desktop browser window dragged narrow, which is how a lot of people check a
  dashboard on a second monitor;
- any touchscreen laptop, which reports `fine`.

A layout that exists to serve *narrow viewports* should be gated on width, with
`pointer` at most a secondary hint.

### U-3 — Half the Patterns tabs are unreachable on a phone · **P2**

The Patterns sub-tab strip carries six tabs. On a 390 px phone, measured:

```
scrollWidth 696   clientWidth 342   →  354 px (51%) off-screen
```

**"Who sings together", "Trends" and "Behavior" are invisible.** The strip does
scroll horizontally — `overflow-x` is set, so no page-level overflow, which is
why the structural sweep passed it — but there is **no fade mask, no gradient, no
chevron** (`maskImage`/gradient check: none), and `scrollbar-width: thin` means
iOS and Android draw no scrollbar until a scroll is already in progress. The last
visible tab ends cleanly at the edge with no partial peek, so nothing signals
that more exists.

Behavioral Analytics — the feature the project is named for — is one of the three
a phone user cannot find.

**Fix.** A fade mask on the scrolling edge, or a partial-item peek, or wrap to
two rows on narrow viewports. Any of the three.

### U-4 — Series colours collide, by construction · **P2**

`species_color` (`routes/pages/atoms.rs:64`) is:

```rust
format!("oklch(62% 0.13 {})", species_hue(name))   // hue = FNV-1a(name) % 360
```

Lightness and chroma are constant; only hue varies, and the hue is a *hash* with
nothing spacing it. On the fixture's own eight-species streamgraph:

```
hue  89  Northern Cardinal          2 deg apart — indistinguishable
hue  91  American Robin
hue 120  Mourning Dove              3 deg apart — indistinguishable
hue 123  Tufted Titmouse
```

Both pairs are visible in the screenshots as the same olive and the same gold.
This is not bad luck with these species. Drawing N hues uniformly from 360 with
no spacing, the probability that *some* pair lands within 25° — about where fixed
lightness and chroma stop being separable — is:

```
N= 6  ->  92.7%
N= 8  ->  99.6%
N=12  -> 100.0%
```

Constant `L = 62%` also means the palette carries no lightness variation at all,
so it collapses in greyscale and for the common colour-vision deficiencies: two
series that differ only in hue are two identical greys.

**Diagnosis matters here.** Hashing to a stable colour is exactly right for the
*avatar chips*, where one species is shown on its own and only stability matters.
It is wrong for **multi-series charts**, where the colours are compared against
each other. The fix is to keep the hash for chips and assign chart colours by
*rank within that chart* — maximally spaced around the hue circle, alternating
lightness — so eight series are always eight distinguishable colours.

### U-5 — Charts without axes, and a caption describing a different chart · P3

The Activity streamgraph renders **no axes at all** — no dates along x, no counts
along y. A reader cannot tell what period is shown or whether a band is five
detections or five hundred; the shape is all there is, and with near-flat data it
is a stack of stripes.

Directly above it, the section's lead paragraph reads *"Darker cells mean more
birds heard that hour."* The streamgraph has no cells. That sentence describes
the **Activity grid**, which is the *next* card down. So the first chart on the
page is unexplained and the explanation that is present points at the wrong
thing.

### U-6 — "Bursts of singing" renders a table of non-bursts · P3

On the Behavior tab, the sessions table defines a session as *"a run of
detections with no gap longer than ~20 quiet minutes — one 'visit' of activity"*
and then lists rows that are, every one of them, **1 detection lasting 0s**.

Structurally correct, semantically empty: a one-detection zero-second session is
not a burst of singing. The flagship behavioural analytic reads as broken to
anyone who looks at it. Either filter to sessions of ≥2 detections, or say
plainly that no multi-detection sessions have occurred yet.

On the same tab, "Follow-on species — after hearing one species, which tends to
turn up **next**" lists **Red-winged Blackbird** among the follow-ons to
Red-winged Blackbird, at 14%. Either that is meaningful (the same bird calling
again) and needs saying, or it should be excluded.

### U-7 — Small controls, and a few clipped below 360 px · P3

Under the real phone layout, **243 of 335 non-inline controls are smaller than
44×44** — the Apple HIG and WCAG 2.5.5 (AAA) target. Most are 30×30, which does
clear WCAG 2.5.8 (AA, 24×24), so this is a comfort finding rather than a
conformance failure — but "comfort" on a phone held one-handed outdoors in the
cold is the actual use case.

Genuinely clipped past the right edge at narrow widths (measured on the
Recordings page): the lock, unlock, delete and download controls at **≤360 px** —
a very common Android width — and the Settings tab label at 320 px. These are
functions, not decoration.

### On the "too many collapsed sections" question

Checked, and the premise does not hold: there are **14 `<details>` elements in
the entire UI** — counted at `ee795ed` across `crates/birdnet-web/src` and
`crates/birdnet-web/templates`: `templates/timeseries.html` ×4,
`templates/admin_audio_sources.html` ×3, `routes/pages/correlation.rs` ×2, and
one each in `templates/dawn_chorus.html`, `templates/login.html`,
`templates/admin_accounts.html`, `routes/pages/behavioral.rs` and
`routes/admin/migration/render.rs`. (This pass first reported 12 and
`POST_0140_AUDIT.md` reported four; neither count was right.) They are used the
way progressive disclosure should be — "See the numbers" behind a chart, an
add-form behind a button, a password hint behind a link. Nothing a user needs is
hidden behind one.

The real navigational problem is the opposite. **Admin was a long flat scroll**:
`/admin/settings` was one column of full-width sections with no in-page nav, no
anchors and no way to jump. That half is since fixed — a sticky "On this page"
jump list now heads the page, one anchored entry per section
(`crates/birdnet-web/src/routes/admin/settings/render/mod.rs:47`, `:59`,
`:147`). Entering Admin still **replaces the whole app shell** —
the Today/Species/Patterns nav disappears, swapped for a dense 12-item admin bar
with six micro category labels, and the only route back is a breadcrumb. That
discontinuity, not disclosure, is what makes the settings area feel hard.

---

## 2. Things checked and found sound

Recorded because "we verified this" is worth as much as a finding, and because
three of these contradict claims made earlier in this same audit before they were
checked. One of them — the watchdog — was itself wrong, and is retracted below
rather than quietly dropped:

- **Live analytics are not stale.** An early read of the sync call sites
  suggested `DuckDB` was only populated at startup, which would have frozen every
  analytics page for the life of the process. False: `src/daemon/processor.rs`
  inserts per detection (`:685` today, `:274` when this ran). Verified by reading
  the processor.
- **There is a background pre-warmer.** `src/app.rs` drives `prewarm_analytics`
  (`:500` today, `:397` when this ran), so the heavy cached fragments stay hot
  without a visitor paying for them. An earlier grep scoped to the wrong
  directories missed it.
- **There is operational alerting** — the deadman (A-3). An earlier pass
  concluded there was none.
- **The systemd unit is genuinely hardened.** Read in full: empty
  `CapabilityBoundingSet`, `SystemCallFilter=@system-service` minus eight groups,
  `ProtectSystem=strict` with explicit `ReadWritePaths`, `UMask=0027`, and a
  documented, deliberate reason for *not* setting `ProcSubset=pid`.
- **The watchdog proves the loop is cycling — not that any work happened.**
  Recorded here as *"the watchdog proves work, not liveness"*, on the strength of
  `sd_notify.rs`'s own doc comment. **Retracted.** The mechanism is real —
  `spawn_watchdog_pinger` withholds `WATCHDOG=1` when the counter has not
  advanced (`src/sd_notify.rs:137-149`), so a hung or blocked pipeline is
  restarted rather than reported healthy — but the counter it watches is bumped
  at the top of the daemon's poll loop
  (`crates/birdnet-core/src/detection/daemon/run.rs:287`), and that loop cycles
  on a 500 ms `recv_timeout` whether or not a single file has arrived. A station
  that has recorded nothing for four months keeps it satisfied. The thing that
  notices silence is the detection deadman (A-3), not the watchdog.
  `UNATTENDED_DEPLOYMENT_AUDIT.md` §2 carries the same retraction; this is the
  document that made the claim, so it carries it too.
- **DST does not corrupt filenames.** `LocalOffset` is refreshed by the capture
  supervisor on every tick rather than snapshotted at start, so a station keeps
  naming files correctly across a daylight-saving change it never restarts for.
- **The published docs site is intact.** Index, custom theme CSS and sampled
  chapters all 200; a suspected broken `additional-css` path resolves correctly.
- **`docs/book/_generated/html/` is gitignored** — only `cli-help.txt` is
  tracked. Suspected committed build output; it is not.

---

## 3. Open questions this audit did not settle

Stated rather than guessed at, because the alternative is prose that reads
confident and is not:

- **Behaviour on real Pi hardware over months.** Everything here ran on x86_64 in
  a container. `docs/book/field/hardware-test.md` and `scripts/hardware-test.sh`
  exist for this and are the right instrument. Still unsettled at `ee795ed`:
  nothing in the tree records a run, and no CI job executes either. This is not a
  question source-reading can close.
- **Aggregate cost on Pi-class hardware.** A-9 measured the query shapes at
  four-year scale on x86_64 and fixed the one that was accidentally quadratic in
  history. The remaining full-history aggregates (heat map ~1 s, species
  lifetime ~1.5 s, seasonal phenology ~2.4 s) are inherently O(history) and are
  hidden from page loads by the cache and pre-warmer — but the pre-warmer still
  *runs* them, as background CPU competing with live inference, and that cost
  grows every year. Still not measured on a Pi: `.github/workflows/ci.yml:484`
  runs `cargo check --workspace --all-features --target aarch64-unknown-linux-gnu`,
  so no aggregate has ever *executed* on ARM in this repository.
- **Leap-day skew in day-of-year comparisons.** *Fixed.* DOY 60 is 29 February
  in a leap year and 1 March otherwise, so cross-year phenology carried a
  one-day skew after February. This was recorded as latent "while A-5 stands";
  A-5 was closed and the analytics were wired up, which made it live. The day
  numbers in `phenology/timing.rs` are now projected onto a common year before
  any cross-year comparison, gated by execution tests over a leap/common year
  pair in `tests/phenology_execute.rs`.
- **Whether the DuckDB drift rebuild is fast enough at 2 M rows.** The streaming
  appender bounds memory (measured previously: 1 M rows → 541 MiB before
  streaming, bounded after), but the wall-clock cost of a rebuild during startup
  on a Pi is not measured. It runs only when drift is detected. Still unmeasured
  at `ee795ed`: the workspace has two bench targets
  (`crates/birdnet-core/benches/audio_pipeline.rs`,
  `crates/birdnet-db/benches/db_queries.rs`) and neither touches `full_resync`.
