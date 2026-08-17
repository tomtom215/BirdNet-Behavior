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

**Method.** Every claim below was produced by running something. Where a previous
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
empty, `SystemCallFilter` minus eight groups, watchdog gated on *detection-loop
progress* rather than mere liveness), the detection deadman closes the
"everything is green and nothing is being detected" hole that most stations have,
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
| A-3 | Operational alerting is one-dimensional: only "no detections at all" ever notifies | P2 | open |
| A-4 | Reviewer verdicts are collected and then applied to nothing | P2 | open |
| A-5 | The `phenology` module — 925 LOC, 12 exports — has no production consumer | P2 | open |
| A-6 | Importing another station's history reconciles neither its location nor its clock | P2 | open |
| A-7 | The field runbook is not on the docs site, and its memory ceiling is wrong | P3 | open |
| A-8 | Live and synced `DuckDB` rows carry different columns | P3 | latent |

---

### A-1 — Operator edits never reach the analytics store · **P1** · fixed

**The most consequential finding in this pass, and the whole 2190-test suite was
green through it.**

`SQLite` is the source of truth; `DuckDB` is a derived copy that every
behavioural and time-series dashboard reads. New detections reach both —
`src/daemon/processor.rs:274` inserts into `DuckDB` right after the `SQLite`
write. Four ordinary operator actions did not:

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
convention already documented in `clock.rs` and used by
`queries/detections/read.rs:114` and `src/maintenance.rs:415`.

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

### A-3 — Only one thing can ever page you · P2 · open

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

---

### A-4 — Curation is collected and applied to nothing · P2 · open

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

---

### A-5 — A whole analytics module nothing calls · P2 · open

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

---

### A-6 — Importing another station's history reconciles nothing · P2 · open

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

Until then the honest position is to say so in the migration guide, which
currently does not mention location or timezone at all.

---

### A-7 — The field runbook is not on the docs site · P3 · open

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

---

### A-8 — Live and synced `DuckDB` rows carry different columns · P3 · latent

`AnalyticsDb::insert_detection` (the live path) writes 6 columns; `SYNC_COLS`
(the bulk path) writes 12. Rows written live therefore have NULL `Lat`, `Lon`,
`Cutoff`, `Week`, `Sens`, `Overlap`, and the same detection gets different
contents depending on whether it arrived live or via a resync — including the
new drift rebuild, which will now *change* those columns on stations that trigger
it.

Checked: no `DuckDB`-side query reads any of the six, so nothing is wrong today.
It is recorded because it is a trap for the next analytic that wants
`Lat`/`Week`, and because "the same row means different things depending on how
it got here" is not a property to leave undocumented.

---

## 2. Things checked and found sound

Recorded because "we verified this" is worth as much as a finding, and because
three of these contradict claims made earlier in this same audit before they were
checked:

- **Live analytics are not stale.** An early read of the sync call sites
  suggested `DuckDB` was only populated at startup, which would have frozen every
  analytics page for the life of the process. False: `src/daemon/processor.rs:274`
  inserts per detection. Verified by reading the processor.
- **There is a background pre-warmer.** `src/app.rs:397` drives
  `prewarm_analytics`, so the heavy cached fragments stay hot without a visitor
  paying for them. An earlier grep scoped to the wrong directories missed it.
- **There is operational alerting** — the deadman (A-3). An earlier pass
  concluded there was none.
- **The systemd unit is genuinely hardened.** Read in full: empty
  `CapabilityBoundingSet`, `SystemCallFilter=@system-service` minus eight groups,
  `ProtectSystem=strict` with explicit `ReadWritePaths`, `UMask=0027`, and a
  documented, deliberate reason for *not* setting `ProcSubset=pid`.
- **The watchdog proves work, not liveness.** `sd_notify.rs:159` withholds the
  ping when the detection-loop counter has not advanced, so a hung pipeline is
  restarted rather than reported healthy.
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
  a container. `docs/HARDWARE_TEST.md` exists for this and is the right
  instrument; it has not been run this cycle.
- **Query cost at multi-year scale.** The hour-of-day and day-of-week analytics
  filter on `strftime(...)` expressions, which no index can serve, so they are
  full scans of `detections`. The 10-minute fragment cache plus the pre-warmer
  hides this from page loads. Not measured at 1 M+ rows on Pi-class hardware; on
  a station that has imported a multi-year BirdNET-Pi history, that is the
  interesting case and it is unmeasured.
- **Leap-day skew in day-of-year comparisons.** DOY 60 is 29 February in a leap
  year and 1 March otherwise, so cross-year phenology carries a one-day skew
  after February. Latent while A-5 stands; a blocker for wiring it up.
- **Whether the DuckDB drift rebuild is fast enough at 2 M rows.** The streaming
  appender bounds memory (measured previously: 1 M rows → 541 MiB before
  streaming, bounded after), but the wall-clock cost of a rebuild during startup
  on a Pi is not measured. It runs only when drift is detected.
