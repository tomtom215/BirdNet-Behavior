# Field-readiness audit — 2026-08-19

**Scope.** A second pass over the whole project after `v0.14.0`, asked with one
question in mind: *this station is about to be sealed into an outdoor enclosure
and left running.* Not "does it work" — it does — but "what does a year of
nobody looking expose, and would anyone find out?"

**Relationship to `PRODUCTION_AUDIT.md`.** That document (2026-08-17) is the
previous pass. This one does not repeat it. Where its findings have since moved,
§6 says so — three of its statuses are now stale.

> **Follow-up pass, same session.** F-6's residue, F-9, F-10, F-11 and F-13's
> `escape_html` are now fixed, along with three defects the pass turned up that
> are not in the original list — see §8. The findings below are left as first
> written; §8 records what moved and what is still open.

**Method.** Every claim below was produced by running something: a probe, a
query plan, the real binary under a real environment, or a gate watched failing
against the code it was written for. Where a hypothesis of mine turned out to be
wrong, it is recorded as wrong (§5) rather than quietly dropped — two of the
things I expected to find were not there, and knowing that is worth as much as
the findings.

---

## 0. What was actually run

x86_64 Linux, 4 cores, `rustc 1.97.1`, from a cold `target/`.

| Gate | Command | Result |
|---|---|---|
| Build | `cargo build --workspace --all-targets` | exit 0, 0 warnings |
| Tests (baseline) | `cargo test --workspace --all-features` | exit 0 — **2 269 passed**, 0 failed, 5 ignored |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| Docker quickstart | the real binary under the environment `docker compose config` resolves | **could not start** — see F-1 |
| Blank-env sweep | every blank `BIRDNET_*`/`BNB_*` key the shipped configs carry, one at a time | **21 of 39 refuse startup** — see F-2 |
| Scale probe | seeded 2 755 374 detections over 3 years (1.29 GB), real server, real pages | see §2 |

The suite being green is the point of writing this down. Every finding below
was invisible to 2 269 passing tests, and F-1 was invisible to CI entirely.

---

## 1. Findings

### F-1 — `docker compose up` cannot start the container · **P0** · fixed here

The documented Docker path is `cp .env.example .env` then `docker compose up -d`
(`docs/book/getting-started/docker.md:15,42`). It does not start.

`docker-compose.yml` interpolated fifteen optional settings as
`KEY: ${KEY:-}`, which puts the key in the container environment as an **empty
string** whether or not anyone set it — confirmed with `docker compose config`.
clap does not treat an empty environment variable as absent; it reads it as a
supplied value. So `BIRDNET_LATITUDE=` is not "no latitude", it is "the latitude
is the empty string", which fails to parse and exits 2 during argument parsing,
before any of the daemon's own error handling runs.

Running the real binary under exactly that environment (with `entrypoint.sh`'s
`:=` defaulting applied to `MODEL`/`LABELS`), removing one blocker per round:

```
round 1: EXIT=2   invalid value '' for '--latitude'
round 2: EXIT=2   invalid value '' for '--longitude'
round 3: EXIT=2   a value is required for '--mqtt-ha-discovery'
round 4: EXIT=101 panicked at src/integrations/apprise.rs:98
round 5: daemon stays up
```

`restart: unless-stopped` makes that a loop rather than a failure with a cause.
`quickstart.sh` — the advertised one-command bootstrap — fills in lat/lon and the
audio source and still dies at round 3.

**Why nothing caught it.** The only container check in `.github/workflows/docker.yml`
is `docker run --entrypoint /usr/local/bin/birdnet-behavior … --verify-extension`:
it bypasses the entrypoint and sets no environment at all. Nothing in CI has ever
run `docker compose up`, and the Rust suite never sees an environment variable.

**Fixed by** `docker/strip-blank-env.sh` (sourced by the entrypoint before it
reads anything), removing the empty interpolations from `docker-compose.yml`,
commenting the blank keys in `.env.example`, and `scripts/check-compose-startup.sh`
wired into `ci.yml`. The gate was watched failing on the unmodified files.

The entrypoint fix alone would not have been enough: `docker compose exec birdnet
birdnet-behavior --doctor` — the troubleshooting command the docs recommend —
gets the container's *configured* environment, which the entrypoint's `unset`
never touches. That is why the compose file had to change too.

### F-2 — 21 of 39 blank settings refuse startup · P1 · mitigated here

Tested one at a time against the real binary. Beyond the four above:
`APPRISE_CONFIG`, `CLIP_RETENTION_DAYS`, `CUSTOM_IMAGE_DIR`, `DISK_EXCLUDE`,
`DISK_PURGE_THRESHOLD`, `LABELS`, `LABELS_DIR`, `METADATA_MODEL`, `MODEL`,
`MQTT_RETAIN`, `NIGHT_INHIBIT`, `NO_UPDATE_CHECK`, `OFFLINE`,
`POST_SUNSET_OFFSET`, `PRE_SUNRISE_OFFSET`, `STREAM_MAX_MB`,
`STREAM_RETENTION_SECS`. The `BNB_*` keys are all safe — they are read with
`std::env::var`, not through clap, which is exactly where the difference comes
from.

The binary cannot defend itself here: `std::env::remove_var` is `unsafe` in
edition 2024 and the workspace sets `unsafe_code = "forbid"`, so scrubbing the
environment in-process is not available. The fix has to live where the blanks
are manufactured, which is what F-1's fix does.

`BIRDNET_IMAGE_CACHE_DIR` is deliberately exempt from the scrubber: an
explicitly empty value is the documented air-gapped opt-out, and `src/cli.rs`
carries a custom parser and a test specifically so it survives.

### F-3 — a config file could abort the daemon · P1 · fixed here

`APPRISE_URL=` in `birdnet.conf`, with no `APPRISE_CONFIG_FILE`, reached
`.expect("config file required when no URL")` and aborted during startup —
reproduced directly, exit 101. The admin settings page's own hint reads *"Leave
blank to disable HTTP push notifications"*.

Release builds are `panic = "abort"`, and the shipped unit pairs `Restart=always`
with `StartLimitBurst=5` / `StartLimitIntervalSec=300`. A station that panics on
start therefore burns its five restarts in fifty seconds and is left `failed`,
permanently, with no further attempts. On a sealed enclosure that is a site
visit.

**Fixed:** blank and whitespace-only values are treated as absent, with a
counterpart gate so "blank URL + config file" still builds the CLI-only client.

### F-4 — one time-series page silently un-applied every rejection · **P1** · fixed here

The sharpest finding in this pass, and the one I would not have believed without
running it.

`birdnet-behavioral` and `birdnet-timeseries` both create a `DuckDB` view called
`detections_ts`, with `CREATE OR REPLACE`, on the **same connection** —
`AppState::with_timeseries` hands `AnalyticsDb::conn()` straight to
`TimeSeriesDb::new`. The behavioural definition carried
`WHERE review_verdict IS DISTINCT FROM 'rejected'`; the time-series one did not.

So the last one to run decided what *both* crates saw, for the rest of that
connection's life. Measured on a three-detection fixture with one rejection:

```
detections_ts count: before=2   after one quiet_days() call=3
```

A rejected detection reappeared in sessionize, retention, funnel, next-species
and co-occurrence the moment anyone opened a time-series page, and stayed back
until the next full sync happened to restore the other definition. Which number
a dashboard showed depended on what the operator had browsed. Nothing reported
it, and `tests/analytics_divergence.rs` could not see it — both *stores* agreed;
the *view* changed underneath them.

**Fixed:** the definitions are now one statement, gated two ways in
`tests/analytics_view_ownership.rs` (texts must match; behaviour must hold), with
a counterpart proving unreviewed detections still survive. Both gates were
watched failing against the original definitions.

### F-5 — the gate that was supposed to catch F-4 was a tautology · P1 · fixed here

`tests/review_verdicts_apply.rs` asserted:

```rust
fn analytic_rows(state: &AppState) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM detections_analytic", ...)
}
```

That re-states the view's own `WHERE` clause back to itself. It passes whether or
not a single analytic ever reads the view. Its failure message claimed it covered
"species totals, the heat map, the dawn chorus, phenology"; it covered none of
them. This is precisely the failure mode `CLAUDE.md` warns about — *a test that
passes tells you nothing until you know why it passes* — and it is how F-6
survived.

**Fixed:** it now reads through the query layer, and a new gate asserts the four
headline dashboard tiles agree with each other.

### F-6 — the dashboard contradicted itself, tile by tile · P1 · partly fixed here

One row of the Today dashboard drew six numbers from both sides of the reviewer
filter:

| Tile | Source | Excluded rejections? |
|---|---|---|
| Detections (all time) | `detection_count` → `FROM detections` | no |
| Species | `species_count` → `detections_analytic` | yes |
| Today | `today_count` → `FROM detections` | no |
| Last hour | `last_hour_count` → `detections_analytic` | yes |
| Species today | inlined `FROM detections` | no |
| 12-day sparkline | `daily_counts` → `detections_analytic` | yes |

Adjacent tiles disagreed about the same day, by exactly the number of rejections
the operator had recorded — so the more carefully someone curated, the more the
screen contradicted itself. Gate output before the fix: `left: (3, 2, 3, 3)`,
`right: (2, 2, 2, 2)`.

**Fixed for this tile row.** `detection_count` itself is deliberately unchanged:
it is the store's row count and the SQLite-vs-`DuckDB` reconciliation depends on
it counting every row. New `analytic_*` counters serve the presentation side.

**Not fixed, and enumerated so it is not lost.** These still read raw
`detections` and will disagree with the screens above: `routes/feeds.rs` (5),
`routes/share.rs` (2, the public share pages), `pages/today.rs`,
`pages/today_phrase.rs` (3), `pages/species_pages.rs` (3), `pages/behavioral.rs`
(2), `pages/cmdk.rs` (2), `pages/history.rs` (2), `pages/detection_detail.rs`
(2), `routes/health.rs` (2). Record-level surfaces (`detections/read.rs`,
`detection_reviews.rs`, `quarantine.rs`) are correct to stay raw — a reviewer
must be able to find what they rejected.

### F-7 — the health badge read the whole database, on every page, twice a minute · **P1** · fixed here

`layout.html:66` mounts the health badge with `hx-trigger="load, every 30s"`. It
ran `PRAGMA quick_check`, which reads **every page of the database file**.

Measured on the 1.29 GB / 2.76 M-row station, warm cache, `NVMe`:

| | before | after |
|---|---|---|
| `/pages/health-badge` | **3.79 s** | **0.0037 s** |
| `PRAGMA quick_check` alone | 1.5–1.9 s | not run |

A Raspberry Pi reading the same file from an SD card at ~45 MB/s is looking at
roughly 30 s — *longer than the badge's own refresh interval*, so the scans would
overlap. Every open browser tab added a full read of the database twice a minute,
indefinitely, competing with the detection write path for the same card and
wearing it.

`/api/v2/health` did the same thing, and worse: the container `HEALTHCHECK` polls
it every 30 s with `curl --max-time 4` inside a 5 s timeout. At 3.4 s measured on
`NVMe`, a three-year station is already within half a second of failing its own
health check; on a Pi it cannot pass at all, and after three retries the
container is marked `unhealthy` and stays there.

**Fixed:** migration 28 stores the daily integrity check's verdict, which that
job was already computing and discarding. The badge and the endpoint read one
row. `/api/v2/health` still probes reachability on every request — that is the
part a probe genuinely has to sample — and now reports three states
(`ok` / `unchecked` / `error`) instead of collapsing "not yet verified" into
"broken". Three of four gates fail against the old implementation; the fourth is
the counterpart that keeps it from being a blanket alarm.

### F-8 — whole-history aggregates on every page load · P1 · improved here

The species list, the life list and the per-species hour histogram each aggregate
the **entire** detection history, uncached, with no time bound, on every page
load. The work grows with how long the station has been useful, which is exactly
backwards.

Measured on the same 2.76 M-row database, warm, `NVMe`:

| query | before | after (migration 29) |
|---|---|---|
| species list | 4.96 s | **1.31 s** |
| life-list firsts | 4.12 s | **0.58 s** |
| per-species hour histogram | 4.82 s | **1.15 s** |

The existing indexes are single-column, so every plan scanned an index and then
went back to the table for the other columns. Migration 29 adds two covering
indexes. Cost, measured rather than estimated: **+130.6 MB, 9.0 %** of the file;
inserts 0.20 → 0.27 ms per committed row (4 922 → 3 666 rows/s, three orders of
magnitude above what a station produces); ~7 s to build once.

A third index would take the species list to 0.31 s but push the total to 18.6 %,
which is the wrong trade on an SD card for a further second. The gate asserts the
*mechanism* (`EXPLAIN QUERY PLAN` must report `COVERING INDEX`) rather than a
timing, and was watched failing without the migration.

**Still open:** this makes the aggregates cheaper, not bounded. At 10 years the
species list is back where it started. The real answer is a materialised
per-species summary maintained on write, or a time bound with an explicit "all
time" opt-in.

### F-9 — import provenance is recorded and never read · P2 · open

*Directly answering "what happens if someone uploads historical BirdNET-Pi data
from a different station location?"*

Before the import, well: `crates/birdnet-migrate/src/provenance.rs` profiles the
source's modal coordinate (mode, not mean — a station that moved once has a mean
where it never stood), reports the haversine distance, flags a source that is
itself a merge of several sites, and offers a UTC-offset shift because
BirdNET-Pi stores local wall-clock with no zone. It never blocks; merging two
sites is a legitimate thing to want. That is good design and it works.

After the import, not at all. Migration 25 tags every imported row with
`import_batch_id` and `birdnet-behavioral` syncs the column into `DuckDB` —
and **no query, page or API filters or groups by it.** Verified: the only
references anywhere are the schema, the sync, and tests.

So the merged history is one station to every analytic. The life list, "first of
year", species-richness curves, phenology, the heat map and the dawn chorus all
read the union. There is no badge saying a screen contains imported data, no
toggle to exclude it, and no per-site breakdown. The provenance is a forensic
record for someone reading the table by hand.

The pattern for fixing it already exists in the codebase: migration 26 did
exactly this for reviewer verdicts — denormalise onto the detection, then filter
in one view both engines read. `detections_analytic` is the obvious place.

### F-10 — per-source quiet windows are unreachable, and UTC · P2 · open

`schedule_quiet_start` / `schedule_quiet_end` exist in the schema
(`migration.rs:378`), are parsed by `src/capture/sources.rs`, and are honoured by
the supervisor. **Nothing writes them**: no admin form, no API, no CLI flag —
every construction site in the tree passes `schedule_quiet: None`. The only way
to set one is direct SQL.

And when set, they are evaluated in **UTC**: `quiet_minute_of_day` in
`src/capture/runloop.rs` derives minute-of-day from the raw Unix timestamp with
no offset applied. So would a fixed recording window — `--doctor` warns about
that, the admin UI does not.

This is worth deciding either way. An outdoor enclosure near a road or a
neighbour is exactly the deployment that wants a quiet window, and right now the
feature is present in the schema, exercised by tests, and unreachable.

### F-11 — the same `df` parsed two different ways, one of them GNU-only · P2 · open

* `crates/birdnet-core/src/audio/capture/disk/mod.rs` — `df --output=size,used,avail -B1`
* `src/doctor/disk.rs` — `df -Pk --`

`--output=` and `-B1` are GNU coreutils extensions; neither exists in BSD `df`.
`docs/book/field/macos.md` documents macOS as a preview target, so the capture
disk manager's usage check cannot work there. The Debian-based image and
Raspberry Pi OS are fine.

Separately: this shells out per purge cycle, and `disk_used_percent` shells out
again on every health-badge request. `sysinfo` is already a workspace dependency
and would answer both without a subprocess (it currently has only the `system`
and `component` features enabled).

### F-12 — the published site and the in-app help are different renders · P2 · open

| | GitHub Pages | in-app `/help/*` |
|---|---|---|
| renderer | mdBook CLI **0.4.52** (`docs.yml`) | `mdbook-driver` **0.5.4** (`build.rs`) |
| theme | `light` / `navy` + `docs/book-theme/custom.css` | `rust` / `navy`, **no custom CSS** |
| fold | enabled | disabled |
| link check | `mdbook-linkcheck`, `warning-policy = "error"` | none |

Same Markdown, two engines two minor versions apart, and the in-app copy does not
get the project's own theme. The 0.4 pin exists because `mdbook-linkcheck` is a
0.4-era backend. Nothing gates that the 0.5 render is correct beyond `cargo build`
not failing.

The book's structure itself is sound: 44 pages, every one reachable from
`SUMMARY.md`, no orphans, no dangling entries (checked).

### F-13 — four kinds of duplicated primitive · P3 · open

*Directly answering "are we reinventing the wheel?"*

* **Hinnant's civil-date algorithm** — `birdnet-core/src/civil.rs` exists as "the
  one implementation that capture code shares", and the `146_097` constant
  appears in **ten** files: `civil.rs`, `routes/pages/mod.rs`, `admin/backup.rs`,
  `admin/accounts.rs`, `routes/share.rs`, `routes/auth_pages.rs`,
  `pages/cmdk.rs`, `examples/screenshot_server.rs`,
  `birdnet-timeseries/tests/queries_execute.rs`, `src/weekly_report.rs`.
* **`escape_html` — three implementations that are not the same.**
  `routes/pages/mod.rs` escapes `& < > " '`; `admin/migration/render.rs` and
  `admin/backup_recovery.rs` escape `& < > "` and **omit `'`**. Neither of the
  two weaker ones currently interpolates into a single-quoted attribute — checked
  — so this is latent, not exploitable. It is still three chances to be wrong
  differently, with nothing keeping them aligned.
* **URL escaping** — `urlencode_path` in `auth_middleware.rs` and again in
  `auth_pages.rs`, plus `simple_url_encode` in `pages/mod.rs`.
* **JSON escaping** — hand-rolled in `mqtt/discovery.rs` and `doctor/render.rs`,
  while `serde_json` is a workspace dependency.

On the specific question of a date crate: `chrono` **is already in the dependency
graph and already compiled into every release binary**, via `duckdb → arrow →
arrow-arith`. The "the workspace carries no date crate" premise in `civil.rs`'s
own module docs is true of the manifest and false of the artefact. That does not
by itself argue for adopting it — `birdnet-core` would gain a dependency it does
not have today, and the arithmetic is correct and property-tested — but it means
the trade is "one crate in `birdnet-core`'s tree", not "one crate in the binary".

Not reinvention, for the record: the hand-rolled Prometheus exposition (~6
transitive deps avoided for what is `println!`), the haversine (documented as
±0.5 %, far tighter than anything it decides), and `civil.rs` itself as a single
shared implementation. The problem is the copies, not the original.

---

## 2. Performance, measured

Seeded 2 755 374 detections over 3 years (80 species, dawn-weighted), 1.29 GB.
Debug build, so page-render numbers carry Rust overhead the release build does
not; the SQLite timings do not, and were taken separately.

| surface | measured | note |
|---|---|---|
| `/` (Today) | 0.036 s | fine |
| `/patterns`, `/reports`, `/help` | ~0.002 s | lazy htmx; cost is in the partials |
| `/pages/health-badge` | 3.79 s → **0.0037 s** | F-7, on **every** page |
| `/api/v2/health` | 3.44 s | F-7, fixed; health-check budget is 4 s |
| `/species` | 8.19 s | F-8 |
| `/recordings` | 8.20 s | not isolated — worth a follow-up |
| `/pages/life-accumulation` | 6.36 s | F-8 |
| `/system/changelog` | 0.006 s | fine |
| startup, cold `DuckDB` | **53 s**, of which ~48 s is the initial SQLite→`DuckDB` sync of 2.76 M rows | the web UI is unavailable throughout |

That last row is not a bug but it is a 24/7/365 fact worth stating: startup cost
grows linearly with history, the station is dark while it happens, and on a Pi it
will be several times worse. `TimeoutStartSec=900` covers it today; at ten years
of history that margin needs re-checking.

---

## 3. Things checked and found sound

Recording these so the next pass does not re-derive them.

* **DST.** `LocalOffset` is re-read every supervisor tick (cached one minute in
  `birdnet_db::clock`, which asks SQLite so the answer agrees with how detections
  are stored). A station keeps naming files correctly across a daylight-saving
  change it never restarts for.
* **Unsynced clock at boot.** Fails *open* — records continuously rather than
  trusting a bogus date, with a one-line log on each transition.
* **Maintenance scheduling** is persisted wall-clock, not uptime-relative, with a
  written rationale about stations that restart more often than the job period.
* **Panic surface.** 53 panic-capable calls outside tests, almost all in
  `build.rs` and test-support. Two were reachable; both are addressed here.
* **Channels.** No unbounded tokio channels. The `notify` watcher's `std::mpsc`
  is unbounded but carries paths, and the real backlog is bounded by the disk
  manager.
* **systemd unit.** Genuinely hardened — empty `CapabilityBoundingSet`,
  `ProtectSystem=strict`, watchdog gated on detection-loop progress, `MemoryMax`,
  `TasksMax`, `OOMPolicy`.
* **`recording_effort`** (migration 27) *is* populated — `src/integrations/effort.rs`
  writes it. I expected it to be another dead table; it is not.
* **The book's structure.** 44 pages, all reachable from `SUMMARY.md`, no orphans.
* **`BNB_*` environment variables.** All 8 blank ones start cleanly; the clap
  fallback is what makes `BIRDNET_*` different.

---

## 4. Interface

The previous audit's U-1…U-7 stand; this pass adds only what it verified.

**On "we have a lot of collapsed sections — is that the best design?"** The
premise does not hold: there are **twelve** `<details>` elements in the entire
UI. Counted and located, they fall into three groups:

* **Five that are worth reconsidering** — the `pt-disc` "See the numbers"
  disclosures on `correlation` (2), `dawn-chorus`, `behavioral` and `timeseries`.
  These hide the data table on pages whose *purpose* is the data. Progressive
  disclosure is right for a reference appendix and wrong for the primary content
  of an analytics screen; a reader who navigated to the co-occurrence page has
  already expressed the intent the disclosure is asking them to re-express.
* **Four that are fine** — add-forms on `admin_audio_sources` and
  `admin_accounts` (a `<details>` doing a modal's job; unconventional, works,
  keyboard-accessible), the `timeseries` API-endpoint reference, the migration
  preview.
* **Three that are right** — the login hint, and two genuinely secondary panels.

The settings page, notably, resisted the temptation: 54 controls across ~11
screens with a sticky "On this page" index and **nothing hidden**, with the
reasoning written down in `render/mod.rs`. That is the right instinct and the
"See the numbers" disclosures are the place it was not applied.

**Navigation** is six top-level journeys — Today, Patterns, Species, Recordings,
Reports, Station — plus Help. That is a clean information architecture; the
problems are inside the screens, not in the map.

---

## 5. Where I was wrong

Kept deliberately, because an audit that only records its hits is not a record of
what is true.

* **I expected the `detections_analytic` view to defeat the indexes.** It does
  not: `EXPLAIN QUERY PLAN` is byte-identical for the view, the base table and
  the inline predicate, and the timings match within noise. The species list is
  slow because it aggregates 2.76 M rows, not because of the view.
* **I expected `recording_effort` to be another table nothing writes.** It is
  written by `src/integrations/effort.rs`.
* **My first index measurements were wrong by 5×.** Measuring candidates one at
  a time and dropping between runs let SQLite reuse freed pages, so each looked
  like +25 MB. Measured together, the pair costs +130 MB. The numbers in F-8 are
  the second set.
* **My first attempt to enumerate the compose failure chain produced four bogus
  rounds**, because my YAML→env conversion kept the quotes and the binary was
  rejecting `"15"` rather than an empty string. F-1's chain is the corrected run.

---

## 6. Corrections to `PRODUCTION_AUDIT.md`

* **A-4 ("curation is collected and applied to nothing") is marked open and is
  largely fixed.** Migration 26 denormalises the verdict and adds
  `detections_analytic`; the behavioural `DuckDB` view filters. What remained was
  F-4 and F-6, both narrower and both partly addressed here.
* **A-5 ("a whole analytics module nothing calls") is partly fixed.**
  `effort_corrected_abundance_sql` and `phenology_timing_sql` now have external
  consumers in `routes/analytics.rs`. **Seven of nine exported builders still
  have zero** — `monthly_totals_sql`, `peak_weeks_sql`, `weekly_abundance_sql`,
  `weekly_richness_sql`, `first_detection_sql`, `interannual_trend_sql`,
  `migration_window_sql`. The two correctness caveats it recorded
  (year-crossing species; `presence_days` being a span) are unaddressed and are
  now reachable for the two that are wired.
* **A-6 ("importing another station's history reconciles nothing") is half
  fixed** — see F-9. The pre-import half is done and done well; the post-import
  half does not exist.

---

## 7. What I would do next, in order

1. **F-6 residue.** Thread the analytic counters through the ten remaining
   surfaces, starting with `routes/share.rs` — a public share link showing
   different totals from the dashboard behind it is the worst version of this.
2. **F-9.** Give `import_batch_id` the treatment `review_verdict` got: one
   filter, one view, one preference, and a badge on any screen whose data is
   partly imported.
3. **F-10.** Decide. Either surface quiet windows in the audio-source form and
   evaluate them in local time, or delete the columns.
4. **F-8, properly.** A maintained per-species summary, so the species screens
   stop being O(history).
5. **F-13's `escape_html`.** Three implementations, one of them correct. Collapse
   to one before the difference becomes exploitable rather than latent.
6. **A soak test.** Nothing here runs the station for a week. The findings above
   are all reachable in minutes; the ones that are not — file-descriptor drift,
   `DuckDB` file growth under continuous sync, SD-card write amplification — need
   elapsed time, and are exactly the class a permanent deployment meets first.


---

## 8. Follow-up pass — what moved

Written after acting on §7. The findings above are unedited; this is the delta.

### Closed

* **F-6 residue** — the published feeds (`/feeds/rare.rss`, `rare.ics`,
  `today.rss`), the Today phrase and its 30-day baseline, the command palette,
  the next-species trigger, the dawn-sequence derivation, the species page's
  showcase clip, and five whole-history aggregates in the query layer now read
  `detections_analytic`. `/api/v2/metrics` keeps `birdnet_detections_total` raw
  on purpose — it is pipeline throughput, not an analytic — and exports
  `birdnet_detections_rejected_total` beside it so both questions are
  answerable from one scrape.

  Record-level surfaces still show rejections, because the review queue holds
  only the **last 25 verdicts**: hiding a rejection everywhere else would make
  an older one unreachable through the UI entirely. That limit is now the
  weakest part of the curation loop and is listed below.

* **F-9** — `birdnet-db` gained the `import_batches` read API it never had, and
  the Patterns screens carry a note naming an imported foreign site, its
  distance and whether the clocks were reconciled. It stays silent for a
  station's own imported history, which is the common case; a banner that cries
  wolf on every import is one nobody reads when it matters.

* **F-10** — quiet windows are settable from the audio-source form, and both
  they and `fixed:HH:MM-HH:MM` recording windows are now evaluated in the
  station's **local** time. Solar schedules stay on UTC and must:
  `SolarDay` reports absolute instants. `DailySchedule::clock()` names which
  clock each gate wants so a caller cannot confuse them. `--doctor` now reports
  the window against the station's offset instead of warning about the old
  behaviour, and tells an operator who set UTC hours to compensate to set them
  back.

* **F-11** — one POSIX `df`, gated against GNU, BSD and BusyBox output.

* **F-13, `escape_html`** — one implementation, gated on all five characters.

### Found while fixing the above

* **The dawn-chorus sun markers were wrong three ways.** Wrong place (a
  hard-coded 40.0 N, 74.0 W unless two undocumented environment variables were
  set, while the station's real coordinates sat in the settings table), wrong
  day (`(unix_secs / 86_400) % 365 + 1`: 14 days out in 2026, −351 days on the
  winter solstice, moving sunrise 18–40 min depending on latitude), and wrong
  clock (UTC hours drawn over local-hour ribbons). The Today page's equivalent
  had been fixed; this copy had not. Both now share one helper backed by
  `birdnet_scheduler::SolarDay`.

  Its own tests asserted UTC while its doc comment claimed "local-civil hours",
  and the guide page told operators to run their station on UTC — which
  contradicted `--doctor`, telling them to set their local timezone. Three
  artefacts agreeing with each other and all disagreeing with the code is worth
  more attention than any of them individually.

* **A test I wrote was racy.** The first version of the "one `df`
  implementation" gate compared two live readings and failed in the full suite
  by 4096 bytes — one block, written by another test between the calls. Fixed
  to a tolerance that two genuinely different parsers could not sit inside.

### Still open

1. **The review queue shows only the last 25 verdicts.** Now the weakest link in
   curation: it is the only surface that lists rejected detections, so a
   rejection older than 25 verdicts is reachable only by a saved URL. Paginate
   it, or give the browsing lists a "show rejected" filter that marks them.
2. **Ten surfaces still read raw `detections`** — `share.rs` (public share
   pages) and `today.rs`'s capture-outage probe among them. The outage probe is
   correct to stay raw: a rejected detection still proves the microphone worked.
   The share pages are not.
3. **F-8 properly.** Migration 29 made the whole-history aggregates cheaper, not
   bounded. At ten years the species list is back where it started; the answer
   is a maintained per-species summary.
4. **F-12** — the published site and the in-app help are still rendered by
   different mdBook majors, and only the published one is link-checked.
5. **F-13's remainder** — ten copies of Hinnant's civil-date arithmetic, three
   URL escapers, two hand-rolled JSON escapers while `serde_json` is a
   dependency.
6. **`htmx_health_badge_returns_healthy_for_a_capturing_station` depends on the
   host's free disk.** It asserts the badge reads "Healthy", and the badge
   correctly grades a >90 % full filesystem as "Disk full" — so the test fails
   on a full build machine. Observed here after a scale probe filled the volume.
   Pre-existing, not introduced by this pass, and a latent CI flake: the disk
   reading needs injecting rather than sampling the host.
7. **The soak test.** Unchanged and still the largest gap: nothing here runs the
   station for a week. Every finding in this document was reachable in minutes.
