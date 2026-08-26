# Enclosure-readiness audit — a third pass, from the outside in

**Date:** 2026-08-26 · **Branch:** `claude/production-readiness-audit-i1xko6` ·
**Base:** `7c6bf77` (`v0.14.0` + the two audit branches merged)

This pass was asked for in the terms an operator actually faces: *a station is
about to be sealed into an outdoor enclosure and left running.* It is the third
audit on this codebase. `docs/PRODUCTION_AUDIT.md` and
`docs/FIELD_READINESS_AUDIT.md` read the Rust and the deployment surface;
`docs/POST_0140_AUDIT.md` read the storage and clock layers. Those three found
and closed most of what reading the code finds.

So this one deliberately did not start by reading the code. It started by
**running the thing** — building it, serving it, fetching from it with a
browser and with `curl`, timing it, and measuring what came back — and then
went to the source only to explain a number. That is why almost everything
below is new: it is not visible in the source, only in the bytes on the wire and
in the packages that are and are not in an image.

Every number here was produced on this machine in this session and the command
is given. Where something could not be verified here, it says so instead of
guessing. Five conclusions I reached early and then disproved are marked
**RETRACTED** rather than quietly dropped.

---

## 0. What was actually run

| What | Result |
|---|---|
| `cargo build --workspace --all-targets` | exit 0, **12 m 25 s** cold (4-core / 15 GB x86_64 container, empty `target/`) |
| `cargo test --workspace` | exit 0, **2 516 passed, 0 failed, 7 ignored** (checked by summing `^test result:` lines, not by trusting the exit code) |
| `cargo fmt --check --all` | exit 0 |
| `cargo clippy --workspace --all-targets` | exit 0, no warnings |
| Live server | `examples/screenshot_server` (9 900 seeded detections, 20 demo clips) on `127.0.0.1:8502` |
| Page weight / compression | `curl` with and without `Accept-Encoding` |
| Accessibility | `@axe-core/playwright` 4.11 + the repo's own route table, both themes, with and without the two rules the shipped gate disables |
| Concurrency | 16 concurrent clients × 64 requests per route, Python `ThreadPoolExecutor` |
| Query cost at scale | synthetic 2 000 000-row `detections` (643 MB) with the shipped DDL, all shipped indexes and `ANALYZE`, stdlib `sqlite3` |
| PNG encoder cost | a real `/api/v2/spectrogram/…` response, IDAT inflated and re-deflated with `zlib` |
| Docker | **static only** — no Docker daemon is available in this container, so the image was audited by reading `Dockerfile`, `docker/entrypoint.sh` and the compose files, and by comparing them against what `install.sh` installs and why |

---

## 1. Defects, worst first

> **Status.** Ten of the fifteen are fixed on this branch: E-1, E-2, E-3, E-4,
> E-5, E-6, E-9, E-12 and two thirds of E-15. Each fix's gate was observed
> failing against the code it was written for, and the commit message records
> the exact failure text. Two of those gates found a further defect while being
> written — a `which` fork that Debian is retiring, and a compression layer
> whose *ordering* silently corrupted every HTML page — and both are described
> where they were found.
>
> The findings below are left as written. They are the record of what `7c6bf77`
> did; the `[FIXED]` line under each says what changed.


### E-1 — The Docker image cannot record. Every documented Docker capture path is inoperable, and the health check stays green · **P0**

**[FIXED]** — the runtime stage installs `alsa-utils`, `ffmpeg`, `sox` and `procps`; `tests/container_can_run_what_the_daemon_spawns.rs` cross-checks that list against every `Command::new` in non-test code, and `docker.yml` resolves each binary inside the built image on both architectures. Classifying the spawns for that gate turned up a second defect: `is_tool_available` forked `which`, which is not POSIX and which Debian's `debianutils` no longer ships — so on this very image the probe could fail with `ENOENT` and `CaptureManager::start` would refuse to record with `arecord not found in PATH` while `arecord` sat on the `PATH`. It is now a `PATH` walk that checks the execute bit, and `src/doctor.rs`'s second copy delegates to it.

`Dockerfile:242` starts the runtime stage from `debian:trixie-slim` and adds
exactly six packages:

```
ca-certificates  curl  libasound2t64  libgcc-s1  libstdc++6  tini
```

That is the complete set — there is no second `apt-get install` in the runtime
stage (`grep -n "apt-get install" Dockerfile` → two hits, one in the *builder*
stage at line 116, one here at 267), and `docker/entrypoint.sh` installs
nothing.

The daemon does not capture audio in-process. It shells out:

| Call site | Tool | Used for |
|---|---|---|
| `crates/birdnet-core/src/audio/capture/process.rs:242` | `arecord` | every ALSA microphone on Linux |
| `…/process.rs:415, 492, 545` | `ffmpeg` | RTSP and PipeWire sources |
| `crates/birdnet-web/src/routes/livestream.rs:252` | `ffmpeg` | Listen → Live |
| `crates/birdnet-core/src/audio/extraction/convert.rs:114, 143` | `ffmpeg`, `sox` | clip conversion |
| `src/doctor/audio.rs:164`, `src/channel_report.rs:355` | `arecord` | `--doctor`, `--channel-report` |
| `crates/birdnet-web/src/routes/admin/system_controls/service.rs:39` | `kill` | the admin **Restart** button |

`arecord` is in `alsa-utils`; `libasound2t64` is only the shared library.
`ffmpeg` and `sox` are not installed either, and neither is in a Debian base
image. So in the shipped image:

* `docker compose -f docker-compose.yml -f docker-compose.alsa.yml up -d` — a
  shipped overlay whose whole purpose is USB microphone capture, with a 40-line
  header explaining `/dev/snd` passthrough — starts a container that cannot
  spawn `arecord`.
* `BIRDNET_RTSP_URL`, documented in `docs/book/getting-started/docker.md:29`,
  cannot spawn `ffmpeg`.
* Listen → Live cannot spawn `ffmpeg`.
* The admin Restart button calls `Command::new("kill")` and discards the result
  (`let _ = … .status()`), so if `kill(1)` is absent it silently does nothing.

  > **Less certain than the other two, and stated as such.** `/bin/kill` comes
  > from `procps` on Debian, and the runtime stage does not install it — but
  > whether `debian:trixie-slim`'s own package set already carries `procps` was
  > **not verified here** (no Docker daemon). The fix adds it explicitly either
  > way, because a shipped binary the product depends on should be a declared
  > dependency rather than an inherited accident. `arecord`, `ffmpeg` and `sox`
  > need no such hedge: nothing in a Debian base image provides them.

**This is the exact failure `install.sh` was fixed to prevent.** `install.sh`
lines 716–754 carry the reasoning verbatim:

> The daemon shells out to one of two tools … Only ffmpeg used to be ensured
> here, on the reasoning that "an ALSA microphone needs no ffmpeg" — [Raspberry
> Pi OS] ships alsa-utils so the gap stayed invisible; **on a minimal Debian it
> produces** [the failure].

`debian:trixie-slim` is a minimal Debian. The bare-metal installer learned this
lesson; the image did not.

**Why nothing catches it.** `.github/workflows/docker.yml` contains no
`arecord`, `ffmpeg` or `doctor` string — the only container assertion is
`--verify-extension`, which checks the DuckDB extension and nothing else. And
the failure is silent by construction:

* the systemd unit runs `--doctor` as `ExecStartPre`; the container entrypoint
  does not (`docker/entrypoint.sh:342` is a bare `exec`), so the doctor's own
  ALSA check never runs;
* that check would not have failed anyway — `src/doctor/audio.rs:158` returns
  `Check::skip("arecord not installed; cannot verify --alsa-device exists")`,
  a *skip*, not an error;
* `HEALTHCHECK` (`Dockerfile:315`) curls `/api/v2/health`, and that endpoint
  (`crates/birdnet-web/src/routes/system.rs:95-127`) returns `200 healthy`
  whenever SQLite is serving. `detection_silence_secs` is in the body but does
  not affect the status code.

A Docker station therefore comes up, reports healthy to `docker ps`, serves a
complete dashboard, and records nothing. The only signal is the detection
deadman (`src/integrations/deadman.rs`, default 6 h) — and only if an Apprise
target is configured, which on Docker nothing sets up.

> **Not verified here:** no Docker daemon is available in this container, so
> the image was not built and the failure was not reproduced. The chain above
> is read from the `Dockerfile`, the call sites and `install.sh`'s own comment.
> The fix ships with a CI assertion (below) that will reproduce or refute it on
> a real runner.

### E-2 — Nothing is compressed. Measured: a 4.4× first load and a 6× Recordings page left on the table · **P1**

**[FIXED]** — `CompressionLayer` with an allow-list predicate (text, JSON, SVG, feeds; never a `206`, never `text/event-stream`, never already-compressed bodies). Measured after: the same eight paths went 596 712 → 144 832 bytes on the wire, **4.1×**. Writing the gate found the defect that mattered: placed *inside* `security_headers_middleware` — which buffers `text/html` and runs `String::from_utf8_lossy` to stamp CSP nonces — every gzip stream came back with its `0x8b` magic byte replaced by U+FFFD. Correct headers, plausible length, and not one page decodable. The layer is now outermost and the gate inflates the body rather than trusting the header.

`Cargo.toml:79` enables `tower-http` with `["cors", "trace", "fs"]`. There is
no `compression-*` feature, no `CompressionLayer`, and no hand-rolled
equivalent (`grep -rn "Compression\|gzip\|deflate\|brotli" crates/birdnet-web/src`
returns only a gzip *content-type* header on backup download and the PNG
encoder's own zlib framing).

Measured against the running server, requesting with `Accept-Encoding: gzip, br`:

| Path | bytes served | `Content-Encoding` | gzip -9 would be |
|---|---:|---|---:|
| `/` | 57 142 | *none* | 16 025 |
| `/species` | 50 958 | *none* | 13 011 |
| `/station/data` | 84 864 | *none* | 19 693 |
| `/recordings` | 109 150 | *none* | 20 722 |
| `/static/css/app.css` | 212 950 | *none* | 43 224 |
| `/static/htmx.min.js` | 50 917 | *none* | 16 326 |

A cold first load is HTML + `app.css` + `htmx.min.js` + two small scripts ≈
**330 KB**, against ≈ **75 KB** compressed. On the WiFi at the far end of a
garden, or a phone on rural cellular, that is the difference between a page
that appears and a page you wait for.

### E-3 — Spectrogram PNGs are stored, not compressed: 7.5× larger than they need to be, measured on a real response · **P1**

**[FIXED]** — `flate2` at level 6. Measured on the same served responses: full spectrogram 499 431 → **67 310** bytes, thumbnail 164 046 → **26 994**, and the `/recordings` grid 3.28 MB → **0.54 MB**. The CRC-32 table moved to a `LazyLock` (it was rebuilt per chunk) and the hand-rolled Adler-32 went with the hand-rolled DEFLATE.

`crates/birdnet-web/src/routes/spectrogram/png.rs:67` writes the zlib stream
with **type-0 (stored) DEFLATE blocks** — the comment says so: *"Store-only
deflate (type 0 blocks) — not great compression but no dependency and correct
output."*

Fetched a real full-size spectrogram from the running server and re-deflated its
IDAT:

```
PNG on the wire            499 431 bytes   (975 × 128, RGBA)
  IDAT (stored)            499 374
  raw scanlines            499 328
  re-deflate level 9        66 686   →  whole PNG ≈ 66 743  (13 %)
```

and the Recordings-grid thumbnail:

```
thumbnail on the wire      164 046 bytes
  re-deflate level 9        26 675   (16 %)
```

`/recordings` renders **20 thumbnails**, so one page load moves **3.28 MB of
PNG** where 0.53 MB would do. The 32 MiB `SPECTROGRAM_CACHE` holds ~200
thumbnails today and would hold ~1 200.

The "no dependency" premise is also no longer true: `flate2 1.1.9` and
`miniz_oxide 0.8.9` are already resolved in `Cargo.lock`. They arrive as a
*build*-dependency of `libduckdb-sys` (via `zip`), so they are compiled but not
linked — meaning adding `flate2` with `default-features = false,
features = ["rust_backend"]` is a pure-Rust addition at a version the lockfile
already vets, with no new C and no new build step.

### E-4 — `/station` blocks for 200 ms on a `thread::sleep`, and the Today rail pays it every 60 s · **P1**

**[FIXED]** — one process-wide `System` handle, refreshed on demand with a 2 s snapshot TTL, so the delta is "since the previous caller" and only the first call in a process waits. `cpu_temperature()` is now its own function, and the two callers that wanted only a temperature use it. Gates: five samples must take under 300 ms (they took **1.005 s** before) and a temperature read must not reach the CPU sampler (it took **201 ms** before).

Serial latency against the running server, five requests each, best-of:

```
/patterns          p50    4 ms
/reports           p50    4 ms
/api/v2/health     p50    4 ms
/species           p50    8 ms
/                  p50   12 ms
/recordings        p50   24 ms
/station           p50  238 ms      ← 60× the next slowest page
```

`crates/birdnet-web/src/system_info.rs:74-77`:

```rust
// Two-pass CPU measurement (sleep briefly for delta)
sys.refresh_cpu_usage();
std::thread::sleep(std::time::Duration::from_millis(200));
sys.refresh_cpu_usage();
```

That sleep *is* the 238 ms. The function's own doc comment, eighteen lines
above it, says what should happen instead:

> For a live dashboard, call this function on a background task with a regular
> interval.

Six call sites call it synchronously instead. Two of them
(`crates/birdnet-web/src/routes/pages/health.rs:77` and
`src/integrations/station_health.rs:242`) do it **only to read
`.cpu_temp_celsius`** — a value produced by `sample_cpu_temperature()`
(`system_info.rs:122`), which reads component sensors and the thermal-zone
sysfs and has nothing to do with CPU sampling. They pay a 200 ms sleep and a
full CPU + memory refresh for a sysfs read.

`health.rs:77` is inside `station_health_line_partial`, which
`templates/today.html:128` polls with `hx-trigger="load, every 60s"`. A kiosk
display left on the Today page therefore holds a blocking-pool thread for
200 ms once a minute, forever — 4.8 minutes of blocked thread per day, plus a
`df` fork (E-8) on the same tick, to render one temperature and one percentage.

### E-5 — The v3 navigation rewrite is 8/14 migrated, the QA table is written in URLs that no longer name what it tests, and four surfaces — `/login` among them — are in neither · **P1**

**[PARTLY FIXED]** — the QA route table is rewritten in the current URLs, so a row named `station-capture` screenshots the Station Capture tab and coverage no longer depends on `redirects.rs`; `/login`, `/station/settings`, `/admin/audit` and `/admin/overview` are now in it. `crates/birdnet-web/tests/qa_routes_cover_the_navigation.rs` fails if a home or a Station tab is missing, and reported all three homes, all six tabs and three standalone screens against the old table. **Still open:** the six `/admin/*` pages that render the retired shell. Redirecting them is a product decision — `/station/settings` is a task-scoped *slice* of the full settings form, not a superset — and this pass would not make it unilaterally.

`crates/birdnet-web/src/routes/redirects.rs` 308-redirects seventeen legacy
public paths to their v3 homes. I assumed from that file that the `/admin/*`
pages were simply left behind. **RETRACTED** — several redirect from inside
their own handlers instead, which `redirects.rs` does not show. Probed every
one against the running server:

| still renders the old admin shell | 308s to its Station home |
|---|---|
| `/admin/settings` | `/admin` → `/station` |
| `/admin/system` | `/admin/audio` → `/station/capture#audio` |
| `/admin/doctor` | `/admin/species` → `/station/capture#species` |
| `/admin/images` | `/admin/quality` → `/station/data#quality` |
| `/admin/audit` | `/admin/rules` → `/station/alerts#rules` |
| `/admin/overview` | `/admin/notifications` → `/station/alerts#notifications` |
| | `/admin/backups` → `/station/data#backups` |
| | `/admin/migrate` → `/station/data#import` |
| | `/admin/accounts` → `/station/access#accounts` |

Eight of fourteen are done. Six still ship a second front door into the same
station, with a different shell and different navigation — `/admin/settings`
most visibly, since it is the page the manual sends people to
(`docs/book/admin/settings.md:3`).

**The QA table is a second, subtler version of the same half-migration.**
`tools/visual-qa/qa.mjs`'s `ROUTES` — the one table both `axe.mjs` and
`qa.mjs` import — is written entirely in pre-v3 URLs: `heatmap`, `analytics`,
`migration`, `correlation`, `timeseries`, `weekly`, `year-in-review`,
`history`, `system`, `admin-audio`, `admin-backups`, …

I assumed that meant the homes were untested. **RETRACTED again**: Playwright
follows the 308s, so `['heatmap','/heatmap']` actually screenshots
`/patterns`, `['system','/system']` screenshots `/station`, and
`['admin-audio','/admin/audio']` screenshots `/station/capture`. All five
Patterns tabs, all three Reports tabs, both Species views and the Live
Recordings view are reached the same way. Coverage is real.

What is wrong is that it is **accidental and mislabelled**. A row named
`admin-audio` writes a screenshot of the Station Capture tab; a reviewer
reading `shots/` cannot tell what was tested. More to the point, the coverage is
now a property of `redirects.rs` rather than of the QA table: retarget or drop
one redirect and a home silently stops being gated, with no row changing and no
test failing.

And four product surfaces are in neither the table nor any redirect. I ran axe
over them (both themes, the gate's own rule set) — all **clean today**, which is
precisely why nobody has noticed they are ungated:

* **`/login`** — the first screen on every password-protected station, and the
  only one an unauthenticated stranger can reach.
* **`/station/settings`** — the General tab; `/admin/settings` does not redirect
  to it, so nothing routes a crawler there.
* `/admin/audit`, `/admin/overview`.

### E-6 — The migration **Upload** tab computes the "this file is from somewhere else" warning and throws it away · **P1**

**[FIXED]** — the Upload tab now stages the validated file and renders the same report the Server Path tab shows, and a separate `POST /admin/migrate/upload/confirm` is what imports. `_is_upload: bool` became the `UploadPreview` enum that decides what the button posts. Gates in `tests/web_api_migration.rs`: the upload response must carry `source_location`, the warning, the species preview and a confirm button, **and** the database must still be empty. Against the old handler the first assertion failed with `the location check is missing from the upload report: <div id="migrate-status">`. A second confirm is refused rather than re-importing. The `if / else if` that hid duplicates behind missing dates is fixed too.

This is the direct answer to *"what happens if someone uploads historical
BirdNET-Pi data from a different station location?"*, and the answer differs by
which of the two tabs they used.

The data model is good. Migration 25 added `import_batches` (source and station
coordinates, `distance_km`, `source_utc_offset_secs`, applied shift, row count)
and `detections.import_batch_id`; migration 31 made `detections_analytic`
honour an `analytics_exclude_imports` setting; `provenance.rs:location_check`
computes a haversine distance and *fails* a file that already contains several
distinct coordinates; the admin page renders a `NNN km away` pill per batch, a
"Keep imported detections out of the analytics" toggle, and a per-batch undo.
The importer converts each timestamp individually rather than applying one flat
shift, and the form explains, correctly and at length, what a single source
offset can and cannot do across daylight saving.

The **Server Path** tab exposes all of it: *Validate Only* renders the report —
schema, row count, unique species, date range, duplicate count, every check with
✔/⚠/✘, a top-species preview — and only then offers **Start Import**
(`crates/birdnet-web/src/routes/admin/migration/render.rs:366-500`).

The **Upload** tab does not. `upload_handler`
(`crates/birdnet-web/src/routes/admin/migration.rs:449-482`) runs the same
validation, rejects the file if a **required** check failed, and then:

```rust
let (schema, report, _migration_report) = match val_result { … };
if !report.passed { … return … }
```

`report` is never read again, and `_migration_report` is discarded at the
binding. `location_check` is deliberately **never** `required` — its own doc
comment explains why ("merging two sites is a legitimate thing to want … the
job is to make that a decision instead of an accident"). So on the Upload tab
the distance warning, the multi-site verdict, the duplicate count, the date
range and the species preview are all computed and then dropped, and the import
starts. The reader of `validation_result` even takes an `_is_upload: bool` it
does not use.

The manual describes the Server-Path journey as if it were both
(`docs/book/guides/migration.md`, step 4: *"Review the preview — top 20
species, the date range, and a data-quality report"*), which is not what the
Upload tab does.

### E-7 — After any import, thousands of detection pages offer an audio player with nothing behind it · **P2**

The importer copies rows, not audio — there is no file copy anywhere in
`crates/birdnet-migrate/src/birdnet_pi/importer.rs`. `File_Name` comes across
verbatim.

`build_audio_section` (`crates/birdnet-web/src/routes/pages/detection_detail.rs:239`)
renders the "The 3-second clip" card whenever `File_Name` is non-empty. It never
asks whether the file is on disk. The spectrogram `<img>` carries
`data-hide-on-error`, and `templates/layout.html:105` does hide it — but there
is no equivalent for `<audio>`, so the player renders with controls that do
nothing, on every imported detection, forever.

### E-8 — `df` is forked from ordinary page renders, on a premise that is not true · **P2**

`crates/birdnet-core/src/audio/capture/disk/mod.rs:99` explains itself:

> Shells out to `df` rather than calling `statvfs`, because this workspace sets
> `unsafe_code = "forbid"` and every safe wrapper for it is an FFI crate.

The second clause does not follow. `unsafe_code = "forbid"` is a lint on *this*
crate's own code; it says nothing about dependencies, and this workspace already
links `rusqlite`, `duckdb` and `ort` — three large C/C++ FFI surfaces. `sysinfo`,
*already a direct dependency*, exposes free space behind its `disk` feature. The
constraint that is cited does not exist; the choice may still be defensible, but
not for that reason. (`docs/FIELD_READINESS_AUDIT.md` F-11 found the same
function parsed two different ways; this is the layer underneath that.)

The cost is that `disk_usage` is called from ordinary request handlers —
`pages/today.rs:359`, `pages/health.rs:68`, `pages/station_health.rs:83` **and**
`:93` (twice per snapshot), `routes/system.rs:136`, `admin/system.rs:251`,
`admin/backup_recovery.rs:125` — so a kiosk on the Today page forks a process
once a minute forever, and `/station` forks two.

`crates/birdnet-web/src/routes/admin/system_controls/service.rs:100` does the
same for `getconf CLK_TCK` on every service-status render.

### E-9 — `Cache-Control: immutable` on an unversioned CSS URL · **P2**

**[FIXED]** — every stylesheet link carries `?v=<version>`, in the layout, the login page, the share page and its 404, the admin shell, the log viewer, onboarding, kiosk and the standalone audio player; `sw.js` precaches the same versioned URLs. `crates/birdnet-web/tests/versioned_assets.rs` fails on any `<link>` to `app.css`/`print.css` without the query, and found the share 404 page after the other eight were done.

`crates/birdnet-web/src/routes/static_files.rs:139` defines
`public, max-age=31536000, immutable` and line 282 applies it to
`/static/css/app.css` — a URL with no version, no hash and no query string
(`templates/layout.html:10`). `immutable` instructs the browser not to
revalidate even on an explicit reload.

The service worker versions its *own* caches by build hash
(`static/sw.js:12-24`) and precaches `app.css`, but `Cache.addAll()` fetches
through the ordinary HTTP cache unless the request is built with
`cache: 'reload'`, which it is not (`sw.js:59`).

So an operator who updates a station — via `/admin/update/apply`, `install.sh`,
or a new container — gets the new binary serving new HTML against **last
year's stylesheet** in every browser that has visited before, for up to a year.
`htmx.min.js` and `theme-guard.js` are `immutable` too; those are pinned
vendor files, so the risk there is lower, but `app.css` changes with almost
every release.

> **Not reproduced here.** Stating what the headers instruct, not a browser
> observation — a cache-eviction test needs a browser profile that survives
> across two server versions, which this session did not build.

### E-10 — The a11y gate's own excuse for its two disabled rules does not survive measurement · **P2**

`tools/visual-qa/axe.mjs:46` disables `color-contrast` and `link-in-text-block`
by default. The comment says why:

> Meeting AA there means changing locked design tokens / species colours — a
> design decision, not an a11y-batch one, and **an all-or-nothing one** (any
> remaining low-contrast node keeps the gate red).

I re-ran the repo's own runner over the repo's own route table, both themes,
with those two rules **on**, and collected the failing colour pairs rather than
a count. The "all-or-nothing" claim is the part that does not hold:

```
run A — color-contrast + link-in-text-block, shipped ROUTES + 24 extra routes:
    481 serious nodes

run B — color-contrast only, shipped ROUTES table alone:
    344 serious nodes    light 298 · dark 46
    44 distinct failing foreground/background pairs
```

Ranked, the debt is four clusters, and three of them are a nudge:

| nodes | fg / bg | ratio | needs | what it is |
|---:|---|---:|---:|---|
| 40 | `#fbfaf7` on `#488055` | **4.47** | 4.5 | the primary button (`.bnb-btn`) — misses by **0.03** |
| ~150 | `oklch(62% 0.13 H)` on `color-mix(… 22%, --surface)` | 2.6–3.0 | 4.5 | `.bnb-avatar` — the species banding code drawn in the species hue |
| 8 | `#7b8186` on `#171b1f` (dark) | **4.39** | 4.5 | muted table header — misses by **0.11** |
| 4 | `#76706a` on `#f5f3f0` | **4.41** | 4.5 | `.bnb-add-form__hint` — misses by **0.09** |

Three of the four biggest clusters miss AA by 0.03, 0.09 and 0.11 — a
one-token darkening each, invisible to the eye and not a design decision in any
meaningful sense. Only the avatar cluster is a real design question, and even
that has a bounded answer: `app.css:317` sets `color: var(--sp)` — the *same*
hue as the 22 %-tinted background — so the identity hue can stay as the
background while the four-letter banding code is drawn in an ink derived from
it. The code is meaningful (birders read `NOCA`), so hiding it from the
accessibility tree is not the way out.

The finding is not "the UI fails AA" — it is that **the recorded reason for not
fixing it is wrong**, and the wrong reason is what has kept the rule off. That
is exactly the class of confident prose `CLAUDE.md` warns about.

### E-11 — Whole-history aggregates carry a measured 20–35 % tax from the analytic view's settings subquery · **P3**

Built a 2 000 000-row `detections` (643 MB) with the shipped DDL, all shipped
indexes and `ANALYZE`, then compared `detections_analytic` (migration 31)
against the same view without the provenance clause:

| query | analytic view | verdict-only view |
|---|---:|---:|
| `COUNT(*)` | 185.9 ms | 137.0 ms |
| life list (`MIN(Date)` per `Sci_Name`) | 333.8 ms | 282.5 ms |
| hour histogram over all history | 1 169 ms | 1 119 ms |
| one day's species | 3.7 ms | 3.5 ms |

`EXPLAIN QUERY PLAN` shows SQLite materialises the settings lookup once
(`SCALAR SUBQUERY 3`), so the cost is the extra per-row branch, not a per-row
table read. Date-bounded queries are unaffected; only the whole-history
aggregates pay, and those are the ones that grow every year. Worth knowing
before someone reads a 1.1 s hour-histogram on x86 and assumes the Pi is fine.

### E-12 — Documentation drift, measured rather than sampled · **P2**

**[PARTLY FIXED]** — `/admin/recordings` is gone from `admin/backups.md`, replaced by the real control. `guides/migration.md` is rewritten around the features that exist: the source-station fields, what a single offset cannot do, the distance warning and the multi-site refusal, the analytics toggle, per-batch undo, that no audio comes across, and the post-import analytics rebuild. The ALSA examples in `docker.md`, `guides/troubleshooting.md` and `docker-compose.alsa.yml` now use `plughw:CARD=<id>,DEV=0` and say what happens if you do not. **Still open:** the `/admin/*` vs `/station/*` URL split in the manual, which follows the product decision left open in E-5.

I extracted every `` `/path` `` from `docs/book/**` (59; 51 are URLs rather than
filesystem paths) and matched each against every `.route(…)` / `.nest(…)` in
`birdnet-web` plus the legacy-redirect table. Results:

* **One documented URL has no route: `/admin/recordings`**
  (`docs/book/admin/backups.md:45`), which is where the manual tells the
  operator to go to *lock* a clip so retention never purges it — the one
  irreversible-data-loss control in the product. The real controls are
  `/pages/recordings-lock` / `-unlock`, driven from `/recordings`.
* Every other documented URL resolves, including the eight that resolve only
  because of a 308.
* Every link in `docs/book/SUMMARY.md` resolves to a file that exists (checked
  independently of `scripts/check-book-links.py`, which checks rendered HTML).

Three content gaps, each on a journey the operator will actually take:

1. **`docs/book/guides/migration.md` (189 words) documents none of the import
   features that exist.** Not the source-station UTC offset — the single most
   consequential field on the form, and the one that silently shifts an entire
   history if wrong. Not the source-station label. Not the `NNN km away` /
   `same site` provenance pill. Not the "Keep imported detections out of the
   analytics" toggle, which is the *only* answer to "I merged another site and
   now my life list is wrong". Not per-batch undo. Not the analytics rebuild
   that runs afterwards. Its step 4 describes a preview the Upload tab does not
   show (E-6).
2. **The manual disagrees with itself about `/admin/*` vs `/station/*`** —
   `admin/backups.md:3` already says `/station/data`; `admin/settings.md:3`,
   `admin/audio.md:3`, `admin/notifications.md:40`, `admin/system.md:50` and the
   whole URL table in `reference/web-api.md:70-78` still say `/admin/…`.
3. **The one setting that decides whether a field station survives a USB
   re-enumeration is documented three different ways.**
   `docs/book/admin/audio.md:94-130` gets it exactly right — it explains that a
   card *index* is assigned in detection order, and tells you to use
   `ALSA_CARD=plughw:CARD=<id>,DEV=0`. `install.sh:1587` agrees, emitting the
   stable form automatically when it can. But
   `docs/book/getting-started/docker.md:28,81`,
   `docs/book/guides/troubleshooting.md:48-49`, `docker-compose.alsa.yml:12`,
   and `install.sh`'s own prompt and summary text (`:1742`, `:2483`) all show
   the index form `plughw:1,0`. A Docker operator copying the documented line
   gets the fragile one.

   This matters because nothing recovers from it. `src/capture/supervisor.rs`
   holds the device string resolved at start and retries it forever with capped
   backoff; there is no re-resolution step, so a microphone that comes back as
   card 2 after a USB re-enumeration is down until a human intervenes. The
   supervisor logs `audio source DOWN — no recording from this source; still
   trying to restart`, which is true and does not name the likely cause.

### E-13 — Data collected and never analysed · **P3**

Two of these; both are "we already have the hard part".

* **Weather.** `birdnet-db/src/weather.rs` stores per-sample rows and
  `src/integrations/weather.rs` fills them. The only reader is the Today page
  (`pages/today.rs:492, 817`), which displays them. Nothing joins weather to
  detections — no "activity vs temperature", no "the chorus starts later when
  it rains", no wind/pressure covariate on the phenology curves. The join key
  exists and the data is being collected every hour, indefinitely.
* **Confidence calibration.** `detection_reviews` (migration 13) holds a human
  verdict per detection, and the quality dashboard shows a *verdict trend*. What
  it does not show is the one thing those verdicts make computable: **observed
  precision per confidence bucket, per species.** That is the analytic that
  answers "what threshold should I set for this species on this station" —
  which is the top tuning question in `docs/book/guides/tuning.md` — with this
  station's own data instead of a global default. `grep -rn "calibration"` over
  the workspace returns nothing.

### E-14 — Backups are a fixed count, not a fixed budget · **P3**

`src/maintenance.rs:59` keeps `BACKUP_RETENTION = 14` full snapshots, taken
weekly before each VACUUM. There is no free-space precondition on either step,
and pruning happens *after* the new snapshot is written, so peak usage is
fifteen copies.

For scale: the synthetic 2 000 000-row database built for E-11 — about 3½ years
at 1 600 detections/day, a busy but ordinary garden station — is **643 MB**.
Fifteen copies is 9.6 GB, on a card that is also holding the recordings, next
to a VACUUM that needs roughly another database's worth of free space to run.

The failure path is handled well when it arrives (`backup_database` refuses to
snapshot a source that fails `quick_check`, and deletes a truncated file on any
error), and `--doctor` grades free space. What is missing is the cheap thing:
refusing to start a backup that cannot fit, and expressing retention as "keep
N, or as many as fit in X GB, whichever is smaller".

### E-15 — Small, verified, cheap · **P3**

**[PARTLY FIXED]** — the CRC-32 table and Adler-32 went with E-3; the `Dockerfile` codename comment and the dead `_is_upload` parameter with E-1 and E-6. `is_leap_year` and `days_in_month` are now in `birdnet_core::civil`, and the two genuine duplicates go through them. The two deliberate copies stay, with `tests/leap_year_agrees_with_the_scheduler.rs` checking `birdnet-scheduler`'s against `civil`'s over every February from 1800 to 2400 — through `SolarDay::for_date`, since the predicate itself is private, plus a leap-year *count* so both saying "no" to everything cannot pass. Dropping the `/400` rule from `civil` makes it report `2000-02-29: civil::is_leap_year says false, the scheduler accepts it`.

* `crates/birdnet-web/src/routes/spectrogram/png.rs:114` calls
  `build_crc32_table()` — 256 × 8 iterations — **once per PNG chunk**, rather
  than once per process.
* `adler32` (`png.rs:104`) takes two `%` per byte; the standard NMAX=5552
  deferral is a three-line change. On a 499 KB image that is ~1 M modulo
  operations per render.
* `Dockerfile:14` says `DEBIAN_CODENAME   Debian base image codename (default:
  bookworm)`; `Dockerfile:35` is `ARG DEBIAN_CODENAME=trixie`.
* `validation_result` (`admin/migration/render.rs:366`) takes `_is_upload: bool`
  and never reads it — the dead half of E-6.
* `validation_result`'s data-quality line is an `if / else if`: a file with both
  null dates *and* duplicates reports only the null dates.
* Four hand-rolled leap-year predicates survive alongside the consolidated
  `birdnet_core::civil` — but **RETRACTED** on two of them, which are not
  duplication:

  | where | verdict |
  |---|---|
  | `birdnet-web/src/routes/pages/history.rs:545` + its `days_in_month` | genuine duplication |
  | `birdnet-web/examples/screenshot_server.rs:69` | genuine duplication |
  | `birdnet-scheduler/src/solar.rs:229` + `:235` | **deliberate.** That crate depends on `serde` and nothing else, so the solar arithmetic stays a pure-computation crate; taking `birdnet-core` for two `const fn`s would pull ONNX Runtime, `symphonia` and `rubato` into it |
  | `src/capture/schedule.rs:473` | **deliberate, and I missed the comment saying so.** It is the *oracle* the conversion beside it is checked against, and the file states in full why an oracle must not call the implementation it verifies |

  So `FIELD_READINESS_AUDIT.md` F-13's remaining tail is two copies, not six.
  The other two need a drift check rather than a merge.

---

## 2. Direct answers

**What are we missing?** Operationally, one thing above all others: *a way to
find out that the station stopped working that does not depend on someone
looking at it.* The deadman exists and is good, but it is opt-in on a
notification target that Docker never configures and the installer only offers.
`/api/v2/health` — the endpoint every external monitor will poll, and the one
the container health check uses — answers a narrower question than its name
promises: it is green whenever SQLite opens. A station whose microphone died in
March reports `healthy` in September.

Beyond that: an image that can record (E-1), compression (E-2, E-3), and the
two analytics whose inputs are already being collected (E-13).

**What am I not 100 % confident in?** Everything that has only ever run on
x86_64. That is not a hedge — it is the same list all three previous audits
ended on and it has not moved: no Raspberry Pi, no ARM test execution
(`cross-aarch64` is `cargo check`), no real microphone, no multi-day run. On top
of that, this pass adds: the Docker findings are read, not reproduced, because
no daemon was available here; and the `immutable`-cache consequence (E-9) is
what the header instructs, not something a browser was observed doing.

**What is our worst devex?** The build. A cold
`cargo build --workspace --all-targets` is **12 m 25 s** on four cores, almost
all of it bundled libduckdb (306 MB of build artefacts) and ONNX Runtime, and
any lockfile change pays it again. CI's `test` job is 22 m 51 s. Second worst:
`cargo bench` appears in **no** workflow, so the two criterion suites
(`audio_pipeline`, `db_queries`) are compiled by `--all-targets` and never run
— there is no performance regression gate at all, in a project whose previous
audit found a query accidentally quadratic in history. Third: coverage is
reported but never gated (no `--fail-under-*`), and it explicitly excludes
`crates/birdnet-migrate` and `crates/birdnet-behavioral` — so the importer, the
subject of two audit findings and one of this one, has no measured coverage.

**What is our worst performance?** In wall-clock terms, `/station` at 238 ms
serial (E-4) — 60× the next slowest page, all of it a `thread::sleep`. In bytes,
`/recordings` at 3.4 MB (E-3) where 0.64 MB would do. In cost-that-grows,
whole-history aggregates (E-11): 1.17 s for an hour histogram at 2 M rows on
x86 NVMe, and this is the class of query a station accumulates more of every
year.

**What misses the engineering-excellence bar?** Three patterns, not three bugs:

* *A comment that justifies a design with a constraint that does not exist.*
  E-8's `df` (the `unsafe_code` argument), E-10's "all-or-nothing" contrast
  claim, E-3's "no dependency" (`flate2` is already in the lockfile). Each
  reads as settled reasoning and each is wrong, and each has therefore
  prevented the obvious fix for longer than a stated open question would have.
* *A function whose own doc comment describes the correct usage while its
  callers do the opposite.* `system_info::sample` (E-4).
* *Work computed and discarded.* The Upload tab's validation report (E-6); the
  weather rows (E-13).

**What misses the production-ready bar?** E-1, unambiguously — a shipped,
documented deployment path with two compose overlays and a manual page, which
cannot perform the product's primary function. Nothing else on this list is in
that category.

**Where do the docs and the GH-Pages site fall short?** E-12. The site itself
is in good shape — one renderer, one `book.toml`, link-checked against rendered
HTML on every PR, and every `SUMMARY.md` entry resolves. The shortfall is
content: a 189-word migration guide for the most consequential and least
reversible operation in the product; a manual that names two different URLs for
the same page; and the ALSA device form documented correctly in one place and
fragilely in five.

**What would I do differently?** Put a gate on the *shape* of the thing rather
than on its internals. Every finding above would have been caught by one of
four cheap gates that do not exist:

| gate | would have caught |
|---|---|
| assert the runtime image can run every tool the daemon spawns | E-1 |
| assert every response over N KB carries a `Content-Encoding` | E-2, E-3 |
| assert no request handler exceeds M ms against the seeded fixture | E-4 |
| derive the QA route table from the router instead of hand-listing it | E-5, and the four ungated surfaces |

**Which parts am I least confident in?** The capture supervisor's behaviour
against real hardware failure. The code is careful — backoff, stall detection,
a watchdog that withholds its ping when the detection counter has not advanced
— and every one of those paths is exercised by a fake source, never by a device.
The one field failure I can name concretely (USB re-enumeration changing the
card index, E-12 §3) has no recovery path at all.

**Which parts fail the bar for telling the public it is complete and production
ready?** The Docker deployment (E-1). Everything else is honest as a v0.x: fast,
careful, well-documented, and with three audits' worth of known-and-recorded
open items.

**Which UI/UX journeys are not complete and polished?**

1. *Import from another station.* The best-designed data model in the codebase
   behind two tabs that do different things (E-6), a manual that describes
   neither accurately (E-12), and thousands of resulting detail pages with a
   dead audio player (E-7).
2. *Administer the station.* Six of fourteen admin pages still open the retired
   shell (E-5), so "where do I change a setting" has two answers.
3. *Log in.* `/login` is the only screen an unauthenticated visitor sees and it
   is in no QA gate (E-5).

**What needs redesigning for the best UI experience?** Not the disclosure
pattern — see below. The two concrete ones are the light-theme contrast tokens
(E-10: 298 of 344 failing nodes are light-theme, and four token nudges clear
most of them) and the migration Upload flow (E-6: make it the same
validate-then-import flow the path tab already has).

The one input that deserves a redesign in its own right is the source-station
UTC offset field: it is a `<input type="number">` asking for **seconds**, with
`step="900"` and a placeholder of `-18000`. Getting it wrong silently shifts an
entire multi-year history onto the wrong hours, and there is no way to check
afterwards except by eye. It should be a zone/offset picker showing "UTC−05:00 —
your imported 06:00 becomes 11:00 here", and it should be echoed back in the
preview before the import runs.

**"We have a lot of collapsed sections — is that really the best design?"**
Checked, and — as `PRODUCTION_AUDIT.md` §1b already found — the premise does not
hold. My own count: **11 `<details>` elements in the entire product** (28 grep
hits, minus comments and one test). They are: a top-species preview behind
"click to expand" on the import validator, three "See the numbers" tables under
charts, an API-details block, a password hint on the login card, and three
add-forms behind a button. Every one is supplementary to something already
visible. Nothing a user needs is hidden behind one, and the disclosure count has
not grown since that finding.

The navigational problem is the opposite one and it is E-5: not too much hidden,
but the same thing reachable two ways with two different shells.

**What happens if someone uploads historical BirdNET-Pi data from a different
station location?** Traced end to end. It depends entirely on which tab they
used:

* **Server Path tab, "Validate Only" first** — the good path, and it is genuinely
  good. They see the row count, date range, species preview, duplicate count and
  a ⚠ on `source_location` naming the distance. If the file itself contains
  several distinct coordinates, that check *fails* and the import is refused.
  They can set the source's UTC offset, and each timestamp is converted
  individually — source local → real instant → this host's local time for that
  instant — so the destination half is right on both sides of every DST
  boundary the destination observes. The batch records both coordinate pairs,
  the distance and the offset. Afterwards, the batch shows as `NNN km away`, one
  toggle removes every imported row from every analytic, and one button undoes
  the whole import.
* **Upload tab** — the same file, the same warning computed, and then discarded
  (E-6). The import runs. Nothing tells them the file is from 400 km away.
* **Either way**, three things then hold that the manual does not mention: the
  source's *own* DST is unrecoverable from a single offset (the form says so;
  the manual does not), no audio comes across so every imported detection has a
  dead player (E-7), and the `analytics_exclude_imports` toggle defaults to
  **off** — so until they find it, the foreign site's dawn chorus is averaged
  into theirs.

**What analytics are we missing?** The two in E-13 — weather×activity and
confidence calibration — are the ones whose inputs are already being collected
and stored, which makes them nearly free and makes their absence the most
striking. Beyond those, the honest list is short, because the coverage is
genuinely broad: sessionisation, retention, funnels, sequence matching,
next-species prediction, phenology with effort correction, Shannon H and Pielou
evenness, accumulation curves, trend, peak windows, gaps, anomalies,
co-occurrence and year-over-year all ship. What is missing is at the tail:
a species-richness *estimator* (Chao1 / jackknife) to sit beside the
accumulation curve and say how much is still unheard, and any significance
statement on a trend (a Mann-Kendall τ next to the slope).

**What is unverified / unprobed / not counter-tested?** Section 4.

---

## 3. Checked, and found sound

Recorded because "we looked and it was fine" is worth as much as a finding, and
because three of these were suspicions I had to abandon.

* **The systemd unit.** Read in full again. `Type=notify` with a watchdog that
  proves *work* rather than liveness, `StartLimitIntervalSec=0` with
  `RestartSteps`/`RestartMaxDelaySec` backoff (with the reasoning for turning
  the rate limit off written out), `RequiresMountsFor` on the data disk, an
  `ExecStartPre` doctor gate that accepts warnings but not errors, empty
  `CapabilityBoundingSet`, `ProtectSystem=strict`, `UMask=0027`, and a
  documented reason for *not* setting `ProcSubset=pid`. This is better than most
  production units I have read.
* **DuckDB is bounded.** `SET memory_limit` is applied at open with a 256 MB
  default and a validated override, well inside the unit's `MemoryMax=1G`. The
  literal is regex-gated against statement injection, with tests for the
  injection case.
* **In-memory caches are bounded.** `SPECTROGRAM_CACHE` is byte-budgeted
  (32 MiB) rather than entry-counted, with the reasoning given; render
  concurrency and live-stream slots are semaphore-capped.
* **Maintenance is scheduled against persisted wall-clock**, not an uptime
  timer — with the failure that motivated it written into the module header (a
  station that reboots daily never reaches a weekly timer). Integrity check,
  session prune, log retention, clip retention, species cap, summary audit and
  backup+VACUUM all hang off it.
* **The importer's clock conversion is per-row**, and the flat-shift
  alternative is documented with a six-row worked example showing three of six
  timestamps an hour out. That is the correct answer to a genuinely hard
  problem.
* **`detections_analytic` reaches the surfaces.** Spot-checked the feeds, the
  share pages and the query layer: the rejected-detection filter is applied,
  and `/api/v2/metrics` deliberately keeps the raw counter *and* exports a
  rejected counter beside it, which is the right call for a throughput signal.
* **The seeded UI has no serious a11y violations** under the gate's own rule
  set, across 61 routes × 2 themes — the 37 in the shipped table plus the
  24 this pass added.
* **Concurrency holds.** 16 concurrent clients × 64 requests: `/` 170 rps at
  p95 127 ms, `/patterns` 330 rps at p95 54 ms, `/api/v2/health` 322 rps at
  p95 48 ms, zero non-200s. The reader pool does not fall over, and no route
  degraded worse than linearly. (`/recordings` at 52 rps is the PNG weight of
  E-3, not a lock.)
* **RETRACTED — `docs/book/_generated/html/` is not committed.** 129 files are
  on disk after a build; `git ls-files` shows exactly one tracked file
  (`cli-help.txt`) and `.gitignore:70` covers the rest. `PRODUCTION_AUDIT.md`
  had already checked this and was right.
* **RETRACTED — the `/admin/*` pages are not all unmigrated.** Eight of
  fourteen redirect; see E-5.
* **RETRACTED — the six homes are not outside the QA gates.** They are reached
  through the legacy redirects; see E-5.

---

## 4. What this pass still does not cover

Unchanged from the previous two audits where it applies, plus what is new here.

* **No Raspberry Pi.** Every timing in this document is x86_64 in a container
  with a warm page cache. A Pi 4 on an SD card will be worse and this pass did
  not measure by how much. The 238 ms `/station` and the 1.17 s hour histogram
  are floors, not estimates.
* **aarch64 is `cargo check` only.** No ARM test has ever executed in this repo.
* **No Docker daemon here**, so E-1 is reasoned from the `Dockerfile`, the call
  sites and `install.sh`'s own comment rather than reproduced. The CI assertion
  that ships with the fix is what will settle it.
* **No real audio hardware.** Nothing has ever lost a USB microphone
  mid-stream, seen a card index change across re-enumeration, or run the ALSA
  path against a device at all.
* **No multi-day soak.** `tests/soak.rs` is a good DB-insert proxy and now
  covers restarts, corruption recovery and bounded analytics-sync memory, but
  nothing here runs capture, inference and the web server together for a week.
  Descriptor drift, DuckDB file growth under continuous sync, SD-card write
  amplification and thermal behaviour in a sealed box remain unmeasured.
* **No browser-cache experiment.** E-9 states what the headers instruct, not
  what a browser was observed doing across two versions.
* **No load beyond 16 concurrent clients**, and none of it against a database
  larger than the 9 900-row fixture. E-11's 2 M-row measurements were taken with
  `sqlite3` directly, not through the server.

---

## 5. Order of work

1. **E-1** — the image cannot record. Everything else is a matter of degree.
2. **E-2 + E-3** — one middleware and one encoder change; between them, a 4.4×
   first load and a 6× Recordings page. Highest ratio of measured benefit to
   risk in this document.
3. **E-4** — delete a `thread::sleep` from a request path.
4. **E-6 + E-12 §1** — make the Upload tab do what the manual already says it
   does, and rewrite the migration guide around the features that exist.
5. **E-5** — finish the six remaining `/admin/*` redirects, and derive the QA
   route table from the router so coverage stops being a side effect of
   `redirects.rs`.
6. **E-10** — the three token nudges; then decide the avatar question
   deliberately instead of by default.
7. **E-13, E-14, E-15** — when there is room.
