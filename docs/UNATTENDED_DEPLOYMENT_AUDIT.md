# What a year alone in a field would find

**Date:** 2026-09-03 · **Branch:** `claude/birdnet-deployment-gaps-e4ebxd` ·
**Base:** `35acd9e` (merge of PR #232, `v0.15.0` + the parity branch)

Three questions were asked of this project at once:

1. What is it missing against [`Nachtzuster/BirdNET-Pi`](https://github.com/Nachtzuster/BirdNET-Pi),
   the surviving maintained fork of the original it descends from?
2. What is it missing against [`tphakala/birdnet-go`](https://github.com/tphakala/birdnet-go),
   an independent Go rewrite of the same idea?
3. **What would block, interfere with, or jeopardise a remote 24/7/365
   unattended deployment in a real field?**

The first two continue [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md),
which closed its ten highest-ranked gaps. This pass re-verified that document at
both upstreams' current tips, found what its lens had missed, and then spent
most of its effort on the third question — which no previous document in this
repository had been asked in those words, and which turns out to be where the
severe findings are.

**133 findings**, counted from the register in §3: **5 P0**, 47 P1, 59 P2,
20 P3, and 2 recorded as deliberate divergences rather than gaps. The five P0s
are PS-1, PS-2, NT-1, LC-1 and LC-2 — in each of them the station keeps serving
a healthy dashboard while it loses, or has already lost, what it exists to
collect.

**All five, and six further findings, are fixed on this branch** — see
§4 Stages 0 and 1. Everything else is written down and ordered, not built.

---

## 0. How this was measured

Eight independent investigations ran in parallel over the same tree, each with
its own scope and each forbidden from trusting the others or the prior audits.
Every finding carries the file and line that is the evidence, and every claim is
labelled by how it was established:

| Label | Meaning |
|---|---|
| **VERIFIED** | Something was run and its output is quoted. |
| **READ** | The code was read, and is quoted with `file:line`. |
| **INFERRED** | Reasoned from those two, and marked as such. |

Numbers, re-derived here rather than carried forward:

| Project | Measured | Command |
|---|---|---|
| `Nachtzuster/BirdNET-Pi` @ `88985a3` (2026-02-28) | 188 tracked files, 24 961 lines of PHP + Python + shell | `find … \( -name '*.php' -o -name '*.py' -o -name '*.sh' \) \| xargs cat \| wc -l` |
| `tphakala/birdnet-go` @ `265b6455` (2026-09-02) | 540 967 lines of Go (264 273 excluding `_test.go`), **53** packages under `internal/` | same shape |
| **BirdNet-Behavior** @ `35acd9e` | 172 482 lines of Rust in `crates/` + `src/`, 186 040 with `tests/` | same shape |

> Two corrections to `FEATURE_GAP_ANALYSIS.md`'s own header while re-deriving
> these: it says 51 `internal/` packages (53 now — the tree moved) and "~19 k"
> for BirdNET-Pi against 24 961 measured here. Neither changes a conclusion.
> A third, methodological, is worth recording because it nearly produced a
> wrong number in this document: `find … | xargs wc -l | tail -1` reports only
> the **last** `xargs` batch's total, not the whole. It gave 240 697 for
> birdnet-go before `xargs cat | wc -l` gave 540 967.

Baseline before any change on this branch: **`cargo test --workspace` → 3 425
passed, 0 failed, 7 ignored**; `cargo clippy --workspace --all-targets` and
`cargo fmt --check --all` both clean, from a cold `target/`.

### What could not be run here

Unchanged from all four prior audits, and the reason several findings below are
READ rather than VERIFIED:

* **No Raspberry Pi, and no ARM execution at all.** CI's `cross-aarch64` job is
  `cargo check` only (`.github/workflows/ci.yml:484`). No aarch64 test has ever
  executed in this repository. See **ARM-1** — this is now closable.
* **No real audio hardware.** No `/dev/snd`, no `arecord`, no `ffmpeg` on this
  machine. Nothing has ever lost a USB microphone mid-stream.
* **No systemd** (`/run/systemd/system` absent) and **no Docker daemon**, so
  unit behaviour and image behaviour are READ plus namespace/cgroup simulation.
* **No multi-day soak.** `tests/soak.rs` remains a database-insert proxy; see
  **PR-13** for a measurement of what it can and cannot catch.

---

## 1. The three questions, answered

### 1.1 Against `Nachtzuster/BirdNET-Pi`

**Very little, and the gap is narrower than the previous pass recorded.** The
database schema is a strict superset of upstream's — no upstream column or table
is missing. Thirteen further items were checked and found at parity. The prior
document's four gaps verify as three-and-a-bit: **N-3 is stale**, because
`?source_id=` on `/api/v2/stream` shipped (`routes/livestream.rs:86`, `:216-221`),
the UI sends it (`templates/recordings.html:362-364`) and four tests gate it
(`tests/web_api_livestream.rs:90-189`); only the station-wide *default* is
missing.

What the previous pass's lens missed is a consequence of where it looked. It
started from `scripts/install_config.sh`, upstream's 57-setting config file,
which is a good index of upstream's *settings* and a poor one of its
*behaviours*. Reading all 57 files in `scripts/` instead produced thirteen
findings the config file cannot show — including **NP-1**, a link on our own
species pages that 404s for every species, and **NP-5**, the Raspberry Pi
undervoltage and throttling telemetry that upstream reads from `vcgencmd` and we
read from nothing. The same blind spot explains a detail in the prior
document worth recording: `ACTIVATE_FREQSHIFT_IN_LIVESTREAM` is not in
`install_config.sh` at all — it lives only in `advanced.php:73-74`.

### 1.2 Against `tphakala/birdnet-go`

**Materially more, but less than the raw line counts suggest, and in several
places our implementation is the better one.** Of the prior document's 34
findings, the ten marked Done verify as genuinely done, and four of them
(sound-level metering, pre-capture, solar quiet hours, dynamic thresholds) are
*better in substance* than upstream's: our third-octave meter is a three-section
cascade on exact IEC 61260 centres with IEC 61672 A-weighting and an energy
mean, against upstream's single biquad on rounded centres with no weighting and
an arithmetic mean of decibels.

Two of the prior verdicts are wrong in a way that matters:

* **G-30 (backup destinations) is stale and partly wrong.** Upstream has **no
  S3 target** — `internal/backup/targets/` holds `ftp gdrive local rsync sftp`
  — and the whole package is dormant, with one importer and no route. We have
  an encrypted S3-compatible path with in-tree SigV4 signing, wired into the
  maintenance loop and round-trip tested. Only the `rsync` recommendation
  survives, and on its own merits.
* **G-15 (taxonomy aliases) is understated.** Upstream applies aliases at the
  geomodel↔classifier join, so without them a mismatched scientific name is
  *permanently undetectable*, not merely double-counted.

And thirty-two findings are new, of which the sharpest is **O-1**: our entire
`/api/v2` surface is read-only. A mechanical check — every `post(`/`put(`/
`delete(`/`patch(` across all fourteen modules mounted under `/api/v2` — returns
nothing. Every mutation in the product is an HTMX form post returning HTML, so
no automation, Home Assistant action or scripted admin is possible without
scraping fragments and forging an `Origin` header.

### 1.3 What would jeopardise a year alone in a field

This is the question with the severe answers. Sorted by what actually happens:

**The station stops recording and reports itself healthy.**
`/api/v2/health` returns `200 "status":"healthy"` whenever SQLite answers
`SELECT 1` — verified live against the real binary while its own response body
said `"detection_daemon":"stopped"` (**OB-4/PR-5**). That is the endpoint the
container `HEALTHCHECK` polls and the one every off-the-shelf monitor gets
pointed at. A truncated model download reaches the same state and passes four
separate gates on the way (**LC-2**), because `doctor/model.rs:26-31` accepts
any file over one megabyte.

**Maintenance stops, silently and permanently, and the station keeps recording.**
Two independent mechanisms do this. `backup_database` uses SQLite's incremental
backup API with a 50 ms sleep between 100-page steps; SQLite restarts the copy
from page 0 on **every external write**, so on a station recording a detection
every 20 seconds it never finishes — measured still running after 300 s with
eight observed restarts (**PS-1**). And the offsite upload sets only
`connect_timeout`, which a probe against a server that connects and then stalls
mid-body proved does not bound the transfer (**NT-11**). Either one blocks the
single sequential maintenance loop for the life of the process, taking the daily
integrity check, `VACUUM`, and every retention job with it — converting
"recoverable corruption" into "total data loss".

**The clock is wrong and everything is filed under the wrong day, for ever.**
No write path checks clock plausibility (**NT-1**). A Pi with no RTC battery
boots at the epoch and records detections dated 1970-01-01 that poison
first-seen dates, phenology and the history calendar permanently, while the
audio evidence is later reclaimed by retention because it is older than any
cutoff. The forward direction is worse: a probe using the exact cutoff
expression from `maintenance.rs:601` showed a +50-year clock jump reclaiming the
entire clip library and the 400-day acoustic baseline in one pass, with the
detection rows surviving so the loss is invisible in every count and chart
(**NT-4**).

**An upgrade bricks a box nobody can reach.** `install -m 0755` **unlinks the
running binary and refills the path in place** — traced with `strace`, showing
`unlinkat` then `openat(O_CREAT|O_EXCL)` with no rename and no `fsync`
(**LC-1**). A power cut in that window leaves an absent or truncated binary,
`Restart=always` retries every five minutes for ever, and the `.prev` rollback
the field manual promises is created by nothing.

**Nobody finds out.** Of 25 realistic ways a station can be broken while looking
alive, **4 are surfaced cleanly, 6 partly, and 15 not at all** (§3.6). The one
push-based dead-man fired only when a bird was detected, so its absence could
not distinguish a dead box from a quiet winter night — fixed on this branch. The
audit log has no production writer at all: `/admin/audit` is permanently empty
(**O-2**). The admin log viewer streams a broadcast channel no `tracing` layer
was ever attached to, so it emits keep-alives for ever (**O-3**).

**And the monitoring itself was malformed.** `/api/v2/metrics` declared
`birdnet_detections_total` twice with two types, which `expfmt.TextParser`
rejects outright — so an agent using it got *nothing* from the station,
`birdnet_detection_silence_seconds` included (**OB-1/PR-10**). Fixed on this
branch.

---

## 2. Corrections to documents in this repository

Recorded first, because a stale claim in an audit is worse than a gap: it stops
the next person running the real check.

| Document | Claim | Correction |
|---|---|---|
| `FEATURE_GAP_ANALYSIS.md` N-3 | "`GET /api/v2/stream` taps whichever source the capture manager offers" | False. `?source_id=` shipped and is gated by four tests. Only the station-wide default is missing. |
| `FEATURE_GAP_ANALYSIS.md` G-30 | Upstream backup targets include `s3` | Upstream has no S3 target and the package is dormant. We have more here than they do. |
| `FEATURE_GAP_ANALYSIS.md` G-15 | Taxonomy aliases cause double-counting | Understated: without them a mismatched name is permanently undetectable. |
| `FEATURE_GAP_ANALYSIS.md` G-17 | Dog-bark `Remember` is in seconds | It is minutes. |
| `FEATURE_GAP_ANALYSIS.md` G-20 | "…`auth.rs`, and audit logging" | `crates/birdnet-web/src/auth.rs` does not exist, and the audit log has no writer (**O-2**). |
| `FEATURE_GAP_ANALYSIS.md` header | 51 `internal/` packages; ~19 k lines upstream | 53 packages; 24 961 lines. |
| Paths naming `internal/myaudio/`, `internal/birdnet/` | — | Neither exists at `265b6455`; they are now `internal/audiocore/`, `internal/classifier/`, `internal/inference/`. |
| `PRODUCTION_AUDIT.md` §2 | "the watchdog proves work, not liveness" | **Retracted.** The counter is bumped at the top of the poll loop (`detection/daemon/run.rs:285`), which cycles every 500 ms regardless of audio. It proves the loop cycles; a station that has written no file for four months keeps it satisfied. |
| `ENCLOSURE_READINESS_AUDIT.md` §2 | USB re-enumeration has no recovery path | Partly superseded: the installer now writes the stable `plughw:CARD=<id>` form (`installer/lib/70-station.sh:64`). `quickstart.sh:196-201` still writes the index (**LC-9**), and a *resolving* index still passes the doctor silently (**AU-1**). |
| `docs/book/field/deployment.md:200` | "The unit waits for `time-sync.target` … so the daemon never sees an unsynchronised clock at boot" | False. `After=` is ordering only; `time-sync.target` is reached when the NTP client *starts*. The unit that blocks is `systemd-time-wait-sync.service`, enabled nowhere here. |
| `docs/book/field/deployment.md:254`, `docs/architecture/10-deployment.md:137` | `StartLimitBurst=5`/`300`, and `60`/`3` | The shipped unit sets `StartLimitIntervalSec=0` and never parks in `failed`. Three documents, three values, none matching. |
| `docs/book/field/deployment.md:534` | "The admin panel's 'Update' button is the only way to upgrade" | There is no such button. `POST /admin/update/apply` has no UI caller and fails `EROFS` under `ProtectSystem=strict` (**LC-3**). |
| `docs/book/field/deployment.md:541` | "keep the previous binary in `…​.prev` so a one-line `mv` rollback is possible" | Nothing in the product ever creates that file (**LC-1**). |
| `docs/book/admin/remote-access.md:44` | the self-signed cert "is replaced a month before it expires" | Only if the process restarts inside the 367–397 day window (**NT-3**). |
| `crates/birdnet-web/src/security.rs:187` | "No HSTS: the binary serves plain HTTP" | Stale since `tls.rs` landed (1 531 lines, three TLS modes). The same file's opening line still says the UI uses HTTP Basic and no cookies. |
| `crates/birdnet-web/src/metrics.rs:154` | a spike in drop-reason `quality` / `occurrence` diagnoses the microphone | Neither reason is ever emitted in production; both appear only in that file's tests. |
| `crates/birdnet-integrations/src/offsite/s3.rs:42` | "A wedged connection is caught by this instead" (`connect_timeout`) | Disproved by probe: a body that stalls after headers hung past 45 s (**NT-11**). |
| `src/maintenance.rs:9` | integrity failures "also pinged to the heartbeat URL in future versions" | Removed on this branch; the heartbeat is now a liveness timer and says nothing about integrity. |

---

## 3. The finding register

Every finding from this pass, with nothing omitted. Severities:

* **P0** — the station dies or loses data, silently.
* **P1** — it degrades badly, or loses data noisily, or the failure is
  undiagnosable from 40 km away.
* **P2** — operational pain, or a defect with a workaround.
* **P3** — polish, or a latent defect with no current trigger.

Findings marked **[FIXED]** landed on this branch; the commit is named.

### 3.1 Power, storage and data durability (`PS-*`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **PS-1** | **P0** | VERIFIED | `backup_database`'s `run_to_completion(100, 50 ms)` uses SQLite's incremental backup API, which **restarts from page 0 on every external write**. On a 209 MB database taking one detection every 20 s it never finished — still running at 300 s, 8 observed restarts, 77 % → 0 each time. `resilience.rs:229`. Because `run_backup_and_vacuum` is awaited inline, this permanently stops the integrity check, `VACUUM`, clip retention, species cap and log retention. | **[FIXED]** — one `sqlite3_backup_step(-1)` inside a single read transaction, so there is no next call for a write to restart, under a ten-minute deadline. |
| **PS-2** | **P0** | VERIFIED | A **zero-length `birds.db` returns `quick_check = ok`**, so `check_and_recover` logs "database healthy", `migrate()` builds a fresh schema, and the station records into an empty database with five good backups beside it that rotate out in 35 days. | **[FIXED]** — `check_integrity` now requires SQLite's sixteen-byte magic before asking SQLite anything. |
| **PS-3** | P1 | VERIFIED | Weekly `VACUUM` writes **3.0× the file size** (274.7 MB measured for 91.3 MB), stages a full copy in the `PrivateTmp` tmpfs inside `MemoryMax=1G`, and holds the write lock; a detection blocked past `busy_timeout=5000` returns `database is locked` and is logged "it is lost". | `PRAGMA incremental_vacuum` with `auto_vacuum=INCREMENTAL`, or `VACUUM INTO` on the data partition; raise `busy_timeout` for the writer. |
| **PS-4** | P1 | VERIFIED | **82.6 KB written to the block layer per detection** against 577 B of row — measured at 12 k/52 k/202 k/502 k rows. ~1.05 GB/day at 1 000 detections/day, ~3.05 GB/day on a mature busy station. 55 % is database machinery. | Batch inserts inside one transaction; `PRAGMA wal_autocheckpoint` tuning; document the real card-wear budget. |
| **PS-5** | P1 | READ | The daily integrity check detects corruption, logs one `error!`, and the daemon **keeps writing to the corrupt file** until someone reboots it. The "never write to a corrupt database" policy exists only at startup. | On a failed check, quarantine and restore in place, or stop the writer and go read-only with a loud health state. |
| **PS-6** | P1 | READ | A quarantined `birds.db.corrupt.<ts>` — total history loss — is matched by **no** doctor scan (`doctor/analytics.rs:130` matches `.duckdb.corrupt.` only, and its test asserts the SQLite name is *not* matched), no `station_health` condition, and no prune. It sits on the card for ever. **[FIXED IN PART — "Alert on a backup that fails, not only on one that stops"]** The doctor scan now matches `.db.corrupt.` as well as `.duckdb.corrupt.` (excluding `-wal`/`-shm` sidecars), and `check_quarantined_stores` raises a condition whose title distinguishes a lost detection history from a rebuilt analytics store. **The prune is still not done**: a quarantined file still sits on the card for ever, which on a 32 GB card is the difference between one bad week and a full disk. That half is item 2.11. | Remaining: prune quarantined stores on a retention schedule. |
| **PS-7** | P1 | READ | `sync_all` appears **twice in the whole workspace**, neither in the audio path. Clips and segments are written non-atomically under their final names, so a power cut leaves truncated files the database points at for ever; and because both retention passes are database-driven, a clip whose row was lost is never deleted except by the 95 %-full purge. | Write to `.part` + `rename` + `sync_all` (the pattern `docker/entrypoint.sh:239` already uses); add an orphan-clip reconciliation pass (**S-14**). |
| **PS-8** | P1 | READ | `--doctor`'s only disk check and its "Recordings directory" check both read `--watch-dir` first, which the shipped unit **always** sets to the tmpfs — so the preflight measures a RAM disk while `/api/v2/system/disk` correctly measures the card. | Check the data partition explicitly, and report both. |
| **PS-9** | P1 | READ/VERIFIED | Nothing probes writability at runtime. On a read-only remount — what the kernel does after repeated I/O errors — `/api/v2/health` still answers `healthy` (a read-only `SELECT 1` succeeds; the integrity verdict freezes because *recording* it is a write) while every detection is classified and discarded. | A periodic write probe on the data partition, feeding a health condition and a metric. |
| **PS-10** | P2 | READ | `check_and_recover` sends `Err` ("could not verify") down the same branch as `Ok(false)` ("verified corrupt") and deletes the live database. Could **not** be provoked with a held write lock — WAL readers do not block — so latent rather than routine, and recorded as disproven. | Separate the two verdicts; refuse to destroy on "unknown". |
| **PS-11** | P2 | READ | `enforce_wal_mode` — the only code that checks the WAL pragma actually took — is called from nowhere but `restore_from_backup`. The live connection uses `execute_batch` and silently accepts `delete` mode. | Call it on the live connection and fail loudly. |
| **PS-12** | P2 | READ | No byte reserve for the database: one 95 % percentage on the recordings directory, with the DB, its WAL, the backup ring, quarantines and `VACUUM` scratch all outside every budget. `out_of_space.rs` explicitly does not cover WAL growth. | A reserved-bytes floor enforced by the purger, sized from the DB + WAL + one backup. |
| **PS-13** | P2 | READ | Zero checksums anywhere in `birdnet-core`/`birdnet-db`. `integrity_check` verifies B-tree structure only, so value-level bit rot is invisible and is copied into all five backups. | Store a content hash per clip in the RIFF INFO block; a periodic sampling verifier. |
| **PS-14** | P2 | READ | The DuckDB damaged-block probe is `SELECT COUNT(*)` — the one query answered from row-group metadata without touching data blocks — and its only test replaces the entire file, so the comment's claim has never been exercised. | Probe with an aggregate over a real column; gate with a byte-level corruption of one block. |
| **PS-15** | P2 | READ/VERIFIED | Effective backup retention is **5 weekly snapshots (35 days)**, not the 14 the constant, the startup log and `resilience.rs:470`'s comment all say. They live on the same card, there is no `--restore-db`, and nothing round-trips a restore through the real recovery path. | Correct the constant or the cadence so they agree; add `--restore-db`; extend `tests/offsite_backup_round_trip.rs` to a full wipe-and-restore. |
| **PS-16** | P2 | READ | `station_health`'s module doc lists five conditions it exists for and `evaluate` implements four: no quarantine condition, and nothing reads the `detection_write_failed` counter. | Implement the fifth; see **OB-7**. |
| **PS-17** | P2 | VERIFIED | Every boot reads the whole database twice plus a full aggregate **before binding** — 4.67 s + 1.46 s + 0.34 s on 262 MB on x86 NVMe, minutes on a Pi at 1.8 GB — paid on each of several brownouts a month. | Move the warm-up behind the listener; bound it. |
| **PS-18** | P3 | READ | `STREAM_MAX_MB` defaults to 512 while the installer mounts the stream dir at `size=256M`, so the byte cap can never bind. | Derive the default from the mount, or assert agreement in `installer/test/`. |
| **PS-19** | P3 | READ/INFERRED | journald storage and size limits are never configured, so the logs explaining a brownout are either volatile (gone at reboot) or unbounded writes to the same card. Same finding as **OB-15**. | Ship a journald drop-in with the unit. |

### 3.2 Process supervision and resource exhaustion (`PR-*`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **PR-1** | P1 | VERIFIED | The transient stream directory is drained on mtime and size alone; a probe against the shipped `DiskManagerConfig` **deleted a segment held open by a live reader**. Audio the pipeline has not analysed is destroyed with no counter and a log line identical to the healthy case. `disk/purge.rs:190,219`; the "never deletes an in-flight segment" comment at `:186` is an assumption, not a mechanism. Same finding as **S-3**. | A claim/lease on segments the daemon has opened; a `birdnet_segments_dropped_total` counter. |
| **PR-2** | P1 | READ | The capture→inference queue is a **directory, not a channel**: nothing throttles, nothing reads temperature and backs off, nothing measures whether the pipeline keeps up. "Inference slower than real time for an hour" resolves to "delete the oldest audio and say nothing". | A queue-depth gauge; a documented shed policy; a thermal read feeding a health condition. |
| **PR-3** | P1 | VERIFIED | `MemoryMax=1G` + `PrivateTmp=yes` + a 512 MiB stream ceiling draw on **the same budget**, and the tmpfs charge is unreclaimable. A cgroup probe (64 MiB limit, 96 MiB tmpfs write) got the writer SIGKILLed at exactly the limit; the same write outside the cgroup succeeded. | Size the tmpfs from `MemoryMax` minus a working-set allowance, and assert the relation in `installer/test/`. |
| **PR-4** | P1 | READ | The 1 GiB ceiling is justified in three places by "the FP32 model is mmap'd". `model.rs:180-184` sets no such option, `ort`'s `commit_from_file` goes straight to `OrtCreateSession(path)`, and `memmap` appears nowhere in the workspace or lockfile — for a 541 MB model. | Measure real RSS with the model loaded and re-derive the ceiling; correct all three comments. |
| **PR-5** | P1 | VERIFIED | `/api/v2/health` returned `200 "healthy"` with `detection_daemon:"stopped"`. The status is `db_ok` and nothing else, and the daemon flag is an `AtomicBool` written once at startup that no exit path clears. Same finding as **OB-4**. | A strict mode returning 503; clear the flag on daemon exit. |
| **PR-6** | P2 | READ | `LimitNPROC=256` is a **per-real-UID** limit on a unit running as the operator's login user, while tokio's default blocking pool is 512 and `pool.rs:325` **panics** when a thread cannot be created — which `panic = "abort"` turns into station death. | Run as a dedicated system user, or raise the limit and cap the blocking pool. |
| **PR-7** | P1 | VERIFIED | When the card fills for a reason that is **not** recordings (the database, analytics, 14 whole-DB backups), the purger deleted 10 % of the operator's clips every 60 s until all 100 were gone and the disk was still 96.9 % full. The counter-test (clips dominant) correctly stopped after three cycles. | Stop purging when the pass frees nothing; raise a distinct health condition naming the real consumer. |
| **PR-8** | P1 | READ | All 30 `Command` sites are reaped, so no zombies — but **none has a timeout**, and the `ffmpeg`/`sox` clip conversion runs inline on the single event-processor thread. One hung child fills the 1 024-slot channel, freezes the heartbeat, and gets the station SIGABRTed at `WatchdogSec=120`. | A `wait_timeout` wrapper on every spawn; move conversion off the event thread. |
| **PR-9** | P2 | READ | `CaptureManager::start` spawns the replacement at `:54` and only drops the old child at `:55`, and the supervisor's *death* path never calls `stop()` first — so a tee-thread death (full tmpfs) opens the exclusive ALSA device twice and logs the misleading "Device or resource busy". | Stop before start on every restart path. |
| **PR-10** | P2 | VERIFIED | `birdnet_detections_total` exported twice in one scrape, gauge and counter. Same finding as **OB-1**. **[FIXED — "Make /api/v2/metrics a document a Prometheus parser accepts"]** | Done. |
| **PR-11** | P2 | READ | `process_existing_files` runs after `READY=1` with the heartbeat frozen. The shipped unit is saved only accidentally, by `PrivateTmp` wiping the backlog — which does not hold for Docker, a persistent `--watch-dir`, or the `RECS_DIR` fallback. | Bump the heartbeat inside the backlog loop. |
| **PR-12** | P2 | READ | The weekly `VACUUM` holds an exclusive lock for minutes while every writer has `busy_timeout=5000`, so detections are refused for its duration — counted in `detection_write_failures_total` and surfaced in no alert, health state or condition. | Ties to **PS-3**; and alert on the counter. |
| **PR-13** | P2 | VERIFIED | `tests/soak.rs` touches no timer, thread pool or subsystem in this report, and its memory gate permits **6.5 KiB of growth per detection** — a leak that would OOM the unit in three months passes it comfortably. | Tighten the bound to what a real leak would exceed; add a long-running capture+inference+web soak. |
| **PR-14** | P3 | VERIFIED | `is_critical`/`is_low` use `available/total` nine lines after the doc comment forbidding that denominator; `/api/v2/system/disk` served **HTTP 503 "critical"** on a filesystem the same response called 70.2 % used. Same finding as **OB-6**. **[FIXED — "Grade a disk against the space it can actually reach"]** Both now read `used_percent()`, and the thresholds are named constants (`DiskUsage::CRITICAL_PERCENT` = 95, `LOW_PERCENT` = 90) that the station-health alert and the purge threshold are tied to rather than repeating. Reproduced on the filesystem this branch was written on: `df -Pk /` gave 77 % used, `used_percent()` agreed at 76.6 %, and `is_critical()` returned true. | Done; see Stage 2 landed. |
| **PR-15** | P3 | READ | `OOMScoreAdjust=200` nominates the station's sole purpose as the kernel's first OOM victim, though `MemoryMax=1G` already protects the host. | Remove it, or justify it in the unit. |
| **PR-16** | P3 | READ | The analytics fragment cache is capped at 256 *entries* of arbitrary-size HTML, where its sibling `SPECTROGRAM_CACHE` is correctly byte-budgeted at 32 MiB. | Byte-budget it. |
| **PR-17** | P3 | READ | Every accepted detection spawns up to five detached, unbounded notification tasks, one a `spawn_blocking` MQTT publish whose connect is bounded but whose preceding DNS lookup is not. Ties to **NT-14**. | A bounded dispatch queue; bound the resolution. |

### 3.3 Time, clock and scheduling (`NT-1` … `NT-10`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **NT-1** | **P0** | READ | **No clock-plausibility gate on any write path.** The machinery exists (`capture/schedule.rs:169` `secs_look_synced`) and is wired only to *scheduling*, failing open. A pre-NTP boot stores detections dated 1970-01-01: `species_summary` files them under hour 00 for ever, every species touched becomes "first seen 1970", the history calendar acquires a 56-year span, `detected_at_utc ≈ 0` orders them before everything, and retention later reclaims their audio because it is older than any cutoff. The evidence goes; the poisoned rows stay. | **[FIXED]** — quarantined with a new `implausible_clock` reason (migration 40) and counted as a drop. The station-health condition and gauge remain, as **OB-14**. |
| **NT-4** | P1 | VERIFIED | **Six date-relative purges read the OS clock with no upper bound.** Probe with `maintenance.rs:601`'s exact expression: correct clock reclaims nothing; a +50-year jump reclaims every clip. The 400-day acoustic baseline goes the same way. Detection rows survive, so the loss is invisible in every count and chart. The *backwards* direction was thought about and is safe; the forward one was not. | A shared `plausible_now_unix()` bounded above by build time + 20 years, threaded as an explicit `now` into all six sites, skipping with a `warn!` when implausible. |
| **NT-2** | P1 | READ | First boot before NTP mints a self-signed CA and leaf valid **1969-12-31 → 1971-02-01** (`validity_window(0, 397)`), and nothing regenerates it while the process runs — `spawn_reloader` starts only for `TlsMode::Manual`. `--doctor` reports `[ PASS ] … valid 397 days` because it prints the *configured* number, never `not_after`. Recovery is physical. | Refuse to mint below the clock floor and degrade to HTTP; make the doctor read `not_after`. |
| **NT-3** | P1 | READ | The self-signed leaf is renewed **only at process start**, so a station up past day 397 serves an expired certificate — exactly when someone finally drives out. `remote-access.md:44` states the opposite as a feature. | `tls::spawn_renewer` on a daily interval; the resolver is already built for hot replacement. |
| **NT-5** | P2 | VERIFIED | Two `CLOCK_SYNCED_FLOOR_SECS`, **1 461 days apart** (2020-01-01 vs 2024-01-01), with the doctor's comment claiming it mirrors the supervisor's. For any reading in 2020–2023 the doctor passes while the supervisor disables the schedule. Neither has an upper bound, so a clock reading 2076 is "synced". | **[FIXED]** — one `CLOCK_PLAUSIBLE_FLOOR_SECS` in `birdnet_core::civil`, with a 2018–2030 weekly sweep asserting the doctor and the supervisor agree at every point. |
| **NT-6** | P2 | READ | Local time comes entirely from host tzdata, which is never installed, refreshed or version-checked. Zero `tzdata` hits across `install.sh`, `installer/lib/*.sh`, `Dockerfile`, `entrypoint.sh` and every compose file; no `TZ` is set. And `--doctor`'s timezone check returns `None` — **silence, not a warning** — when `/etc/timezone` and `/etc/localtime` are both absent, which is exactly the containerised case where the answer is guaranteed wrong. | Add `tzdata` to the image, document `TZ`, make `None` a warning, and report the zoneinfo version. |
| **NT-7** | P1 | VERIFIED | `week` was hardcoded `0` at both production call sites; the geomodel's domain is `1..=48`. **[FIXED — "Ask the geomodel about the season the audio was recorded in"]** | Done. |
| **NT-8** | P2 | READ | Three documents describe systemd behaviour the shipped unit does not have: the clock wait, and three different restart-limit policies. See §2. | Rewrite, then parse the unit out of `installer/lib/65-service.sh` and assert every documented directive matches — the shape of `tests/documented_samples_match_the_build.rs`. |
| **NT-9** | P3 | VERIFIED | `local_utc_offset_secs()`'s cache compares a **signed** age, so after a backwards clock step the difference is negative and therefore `< 60`: the offset freezes for the full magnitude of the step. Simulated across a four-year correction. The module's own comment bounds staleness at "a minute, twice a year"; that bound does not hold. | `if (0..CACHE_SECS).contains(&age)`. One line. |
| **NT-10** | P3 | READ | The weekly report's duplicate guard is in-process (re-sends after every restart) and its weekday is UTC; alert rules take the hour from the detection's *local* time and the weekday from UTC `now` — two clocks in one predicate. | Give the report a `maintenance_runs` job key; derive the weekday from the detection's own date. |

### 3.4 Network, remote access and updates (`NT-11` … `NT-18`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **NT-11** | P1 | VERIFIED | The offsite S3 client sets only `connect_timeout`, and its comment claims "a wedged connection is caught by this instead". A probe with the workspace's own reqwest against a server that sends headers, one byte, then holds: **still hanging after 45 s**. `run_offsite` is awaited inline in the single sequential maintenance loop with no `tokio::time::timeout`, so one wedged socket stops integrity checks, `VACUUM`, backups and every retention job for the process lifetime — and the `warn!` is on the error path, never reached. SFTP has the same shape: `ConnectTimeout=30` and an unbounded `wait_with_output`. | **[FIXED]** — all four: a 120 s `read_timeout` on S3, `ServerAliveInterval=30`/`ServerAliveCountMax=6` on SFTP, a two-hour budget around the whole job, and the comment corrected. |
| **NT-12** | P2 | READ | Update download and verification are genuinely well built (checksum before any I/O, GitHub-only URL check, body capped by `Content-Length` *and* by reading one byte past it, atomic rename). But `smoke_test_binary` only runs `--version`, and the `{name}.bak` binary is **written and never read by anything** — `grep` finds only the two lines that create it. A binary that starts and then dies bricks an unreachable station under `Restart=always`. | Extend the smoke test to `--doctor --config`; add a rollback invoked from an `ExecStartPre` failure counter. |
| **NT-13** | P2 | READ | Two blocking network calls run on a tokio worker thread before the listener binds and before `READY=1`: DuckDB's `INSTALL … FROM community` (with the embedded fallback at stage 3, *after* the network attempt) and four sequential MQTT discovery publishes, each beginning with an unbounded `getaddrinfo`. On a box whose link is down at boot — every boot in this scenario — startup stalls for minutes with nothing in the log. | `spawn_blocking` the state build; `tokio::spawn` the discovery; bound the resolution. |
| **NT-14** | P2 | READ/INFERRED | No DNS cache, no negative cache, no resolver timeout anywhere (`grep` for `hickory`/`trust-dns` in `Cargo.lock`: 0 hits). HTTP clients are saved by their total timeouts; MQTT is not — it re-resolves **per detection** through an unbounded `getaddrinfo` on the shared blocking pool. | A small shared resolver cache with positive and negative TTLs; build each `reqwest::Client` once. |
| **NT-15** | P2 | READ | Species-image lookups have no negative cache, so with the link down every uncached species costs ~50 s of network **on every page view**, for ever — on the link that is already the problem. The retry also hand-rolls `2^attempt` instead of the shared jittered `retry::backoff_delay`. | Store a negative entry with a backoff window; honour `--offline`. |
| **NT-16** | P1 | READ | The heartbeat had one call site, per-detection. Same finding as **OB-2**. **[FIXED — "Ping the monitor because time passed, not because a bird sang"]** | Done. |
| **NT-17** | P2 | READ | No outbound control channel and no documented way to reach a CGNAT'd cellular station. `src/helpers/egress.rs` enumerates the complete egress inventory and it is all one-way telemetry; every management surface is inbound HTTP. The recovery runbook assumes shell access. A station that comes back *wrong* — a bad setting, **NT-2**'s dead certificate, **NT-12**'s bad update — has no remote path at all. | A "Reaching a station on cellular" page with a working `tailscaled`/`autossh` unit ordered `Before=` ours; later, a safe-mode boot after N failed starts that disables the settings overlay and TLS. |
| **NT-18** | P3 | READ | `sound_levels::prune` and `prune_quarantine` have **no production caller**, so both tables grow without bound over twelve months — despite a doc comment claiming the opposite. Same shape as the already-fixed D6. | **[FIXED]** — both wired into `run_log_retention` at 400 and 90 days; only *reviewed* quarantine rows are pruned, because an unreviewed one is the operator's queue. |

### 3.5 Install, upgrade, config and recovery (`LC-*`)

`install.sh` is **generated**; every fix below targets a module under
`installer/lib/` plus `installer/build.sh`, per `CONTRIBUTING.md:47-55`. It was
verified in sync (`installer/build.sh --check`), `shellcheck`-clean bar one
`SC2006` note, `bash -n`-clean, and `installer/test/run-ci.sh` all-pass.

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **LC-1** | **P0** | VERIFIED | `install -m 0755` at `installer/lib/50-binary.sh:124` **unlinks the working binary and refills the path in place**. Traced: `unlinkat("dst")` → `openat("dst", O_CREAT\|O_EXCL)`, no temp file, no `rename`, **no `fsync`**. ~100 MB to an SD card is a multi-second window, and an upgrade is exactly when a solar box browns out. Afterwards `ExecStartPre` and `ExecStart` both fail, `Restart=always` + `StartLimitIntervalSec=0` retries every 5 min for ever, and the previous binary was deleted rather than kept — so the `.prev` rollback `deployment.md:541` promises is impossible, because nothing ever creates that file. The Rust updater already does this correctly (`auto_update/mod.rs:427-441`). | **[FIXED]** — exactly that, gated by `installer/test/binary-swap-atomicity.sh`. |
| **LC-2** | **P0** | VERIFIED | A partial model download is **never resumed and never verified**. `55-model.sh:149-152` keeps the partial and tells the operator to re-run to resume; the guard at `:144` then skips the fetch because the file exists. Reproduced against the real `download_model` with only the network helpers stubbed: `exit=0`, one call (for the labels), model left at 29 bytes of `PARTIAL-GARBAGE`. `repair` says "Model present — skipping download" and never computes a checksum. Four downstream gates then pass on a 200 MB truncation of a 541 MB file: `doctor/model.rs:26-31` accepts anything over **1 MB**; `76-validate.sh` takes the doctor's exit code; `daemon/mod.rs:317-320` logs and carries on serving the web UI; `/api/v2/health` stays `200 healthy`. Operator seals the box, drives 40 km, dashboard green, not one bird ever recorded. | **[FIXED, in part]** — one `model_file_is_verified` helper now used by all three guards, gated by `installer/test/model-resume.sh`. The `doctor/model.rs` threshold and the health status are still open, as items 1.12 and 1.8. |
| **LC-3** | P1 | VERIFIED | **No working remote upgrade path on bare metal.** `POST /admin/update/apply` has no UI caller anywhere; the docs promise an "Update button" that does not exist; and a hand-crafted POST fails `EROFS` — reproduced in a mount namespace — because `ProtectSystem=strict` makes `/usr/local/bin` read-only and `ReadWritePaths` does not include it. | Either delete the endpoint and correct the docs, or stage into `DATA_DIR` and let a small root-owned `.path`/`.service` do the swap. ADR first: it is a privilege boundary. |
| **LC-4** | P1 | VERIFIED | **Zero occurrences** of `unattended-upgrades`, `apt-mark hold`, `needrestart` or `Automatic-Reboot` anywhere in the repository. Every apt package is unpinned and non-fatal. And for an ALSA station a missing `arecord` is a `Check::skip`, not a `Check::fail` — so an OS update that removes `alsa-utils` leaves a green station recording nothing, where the RTSP path correctly fails on a missing `ffmpeg`. | A real ALSA capture-backend check mirroring `environment.rs:106-123`, with the counterpart test that it stays `pass` when present; record resolved tool paths at install and warn when they move; write the operator guidance. |
| **LC-5** | P1 | VERIFIED | CI runs `install.sh` **once**, offline, model-less, on a container with no systemd. Never covered: a second run (idempotency — which `00-usage.sh:31` claims and **LC-2** disproves), `update`, `repair`, `uninstall`, the model path at all, a systemd host, reboot recovery. | A second invocation with a hash assertion; a systemd job; an N-1 → N upgrade job; `model-resume.sh` and `binary-swap-atomicity.sh` in `CI_TESTS`. |
| **LC-6** | P1 | READ | A bad config edit **takes the station down with no way back**: validation runs inside the new process after systemd killed the old one, and some settings abort before the socket binds. With `StartLimitIntervalSec=0` the station fails every five minutes for ever, with no web UI and no revert. The dry-run validator already exists — `--doctor --config` is what `ExecStartPre` runs — and nothing points at it. | Change the generated config header to a two-step form; ship `--apply-config` (validate, restart, restore on failure); consider an `ExecStartPre` fallback to `--web-only`. |
| **LC-7** | P1 | VERIFIED | **133 documented settings, 12 validated, zero unknown-key detection.** `CONFIDENC=0.90` is parsed, stored and ignored with no journal line, no doctor note and no UI hint. About a dozen settings would break a station if wrong; three of those are validated. Two documented settings are dead, one of them shipped **uncommented** (`BIRDNET_QUALITY_MIN_SNR=3.0`), and `src/cli.rs:938-948` records that the CLI half was removed for exactly that reason while the `.env.example` half survived. Same evidence as **O-7**. | Generate `KNOWN_CONFIG_KEYS` from `.env.example` + the clap `env =` attributes; warn on unknown keys with a "did you mean"; a bidirectional drift gate in the shape of `tests/documented_samples_match_the_build.rs`. |
| **LC-8** | P2 | READ | `write_config` early-returns on an existing file and there is no config schema version, so a release that adds a required setting can never reach an existing install. Three accidents currently prevent it biting. | `CONFIG_TEMPLATE_VERSION`; append new commented blocks under a dated banner rather than skipping. |
| **LC-9** | P2 | READ | `quickstart.sh` pins nothing (`main` + `:latest`), writes the **unstable ALSA card index** the bare-metal installer spends a paragraph warning against, and tells the operator to run `docker compose up -d`, which silently drops the `/dev/snd` overlay and leaves a green container with no microphone. | Reuse `70-station.sh`'s id-preferring detection; write `COMPOSE_FILE` into the generated `.env`; default the tag to the newest release. |
| **LC-10** | P2 | READ | The in-app updater replaces only the binary — not the unit, the bundled manual, or the pinned model hashes — and under Docker writes into the container layer, so the "update" silently reverts on the next recreate. | Make it a full-release applier, or refuse inside a container and point at `docker compose pull`. |
| **LC-11** | P2 | VERIFIED | Docker: nothing pinned by digest; the healthcheck cannot see "recording nothing" and **hardcodes port 8502** while the image derives it from `BIRDNET_LISTEN`; the base compose caps memory at 512M against the unit's 1G with analytics on, with no soft `MemoryHigh` and no `OOMPolicy`; and the entrypoint runs no `--doctor` where the systemd unit runs one as `ExecStartPre`. Measured from the GHCR manifest: **arm64 image = 246.9 MB compressed**, which fits a Pi easily. | Digest pinning; a strict health mode both healthchecks point at; derive the port; raise the base limit; run the doctor before `exec`. |
| **LC-12** | P2 | READ | Migrations are forward-only, per-step transactional, resumable after a power cut and downgrade-tolerant — genuinely good. But the pre-migration backup fires only for `HISTORY_REWRITES`, which has **one entry**, and where it does fire nothing opens the result and runs `PRAGMA integrity_check`, though `resilience.rs:161-178` provides exactly that. | Verify after `VACUUM INTO`; widen the trigger to any `up_sql` containing `DELETE`/`DROP`/`UPDATE` as a testable `const fn`. |
| **LC-13** | P2 | READ | No "the SD card died, here is a new one" procedure exists, and `/etc/birdnet/birdnet.conf` — coordinates, device, `CADDY_PWD`, `BIRDWEATHER_TOKEN`, and **the offsite credentials themselves** — is in no automatic backup. The one artefact carrying it is the full backup, which nothing takes automatically. Circular: the credentials needed to fetch the backup are only in the backup. | Include the config in the weekly offsite payload (already encrypted end to end); write `docs/book/field/disaster-recovery.md` with the store-elsewhere list at the top; test a full wipe-and-restore. |
| **LC-14** | P2 | READ | `install.sh uninstall` is a second, weaker uninstaller than `uninstall.sh`: the menu says "keep your data" and then offers to delete it with no plan, no dry-run and no per-category choice — and `rm -rf ${CONFIG_DIR}` sits **outside** the guard that refuses to delete from an unsafe data dir, so a station whose data dir was judged unsafe still loses `/etc/birdnet`, which per **LC-13** is unrecoverable. | Have `do_uninstall` `exec` the bundled `uninstall.sh`; move the config removal inside the guard. |
| **LC-15** | P3 | READ | `docker/entrypoint.sh:146-155` `verify_sha256` **returns success when `sha256sum` is missing** — the exact anti-pattern already closed in `installer/lib/55-model.sh:21-33`, whose replacement comment reads "a backstop that returns success is not one". The container copy was not updated. | Make it `die`; extend `installer/test/checksum-refusals.sh` to cover the entrypoint copy. |
| **LC-16** | P3 | READ | The documented install fetches an unversioned, unverified `install.sh` from `main`, while the release publishes a checksummed one nobody is pointed at. With no `--version`, a re-run six months later installs a different release than the bench unit — quietly undoing the manual's own "test on a bench unit first" advice. | A verified-install stanza in the README and the manual; print the resolved release and confirm when interactive. |

### 3.6 Observability, alerting and diagnosability (`OB-*`)

The central artefact of this section is the **silent-failure matrix**: 25 ways a
station can be broken while the process stays healthy. "Surfaced" means it
reaches a human 40 km away without them logging in — a journal line or an
unscraped Prometheus series is **not surfaced**.

| # | Failure | Surfaced? |
|---|---|---|
| 1 | Mic unplugged, single-source station | Partly — deadman, ≥24 h late, one push |
| 2 | One of several mics dead | **Yes**, 15 min |
| 3 | Mic alive, segments punctual, all zeros | **Yes**, 15 min |
| 4 | ADC/preamp wedged at one non-zero value | **Yes**, 15 min |
| 5 | Gain far too high, ≥20 % clipped | **Yes**, 15 min |
| 6 | **Gain collapsed / mic gone deaf** (water, loose connector) | **No** — measured, deliberately never alerted |
| 7 | Source flapping (restart loop, up at every poll) | **No** — station-health only sees `up == Some(false)` |
| 8 | Inference runs, always returns nothing | Partly, ≥24 h, and undiagnosable — see **OB-12** |
| 9 | Detection daemon never started | Partly — body says so, status stays `200 healthy` |
| 10 | SQLite refuses the write | **No** — counter only |
| 11 | DuckDB mirror write failing | **No** — a `warn!`, no counter, no gauge |
| 12 | Disk full, clips fail, rows still stored | Partly |
| 13 | **Clock wrong but plausible** — whole season misfiled | **No** — checked once, at `ExecStartPre` |
| 14 | Clock unsynced, NTP never lands | **No** — one WARN on transition |
| 15 | Retention purging too aggressively | **No** |
| 16 | Scheduled window resolves to zero minutes | Partly, wrong cause |
| 17 | Notifications failing silently | Was **No**, and could swallow the deadman; **[FIXED]** by **OB-5** — `birdnet_notifications_dropped_total{reason}`, and an undelivered alert is retried rather than latched |
| 18 | Web UI up, every page 5xx | **No** — a counter, and no log line at all |
| 19 | DuckDB sync stopped / store quarantined | **No** — `analytics:true` stays true |
| 20 | Audio quality degraded (wind screen gone) | **No** (as #6) |
| 21 | **Weekly backup fails every week for a year** | Was **No**; **[FIXED]** by **OB-7** — the verdict is recorded and a recorded failure is a condition immediately |
| 22 | Daily integrity check FAILS | Was **Partly** — reddened a page, pushed nothing; **[FIXED]** by **OB-7** |
| 23 | Offsite backup failing, no copy leaves the box | Was **No**; **[FIXED]** by **OB-7** — its own `JOB_OFFSITE_BACKUP` key and condition |
| 24 | Whole box dead | Was **weak**; **[FIXED]** by the heartbeat timer, and now also by the MQTT last will (**OB-8**) where a broker is configured |
| 25 | MQTT/HA "station online" sensor | **No** — advertised, never published |

**4 of 25 surfaced cleanly, 6 partly, 15 not at all.**

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **OB-1** | P1 | VERIFIED | `/api/v2/metrics` declared `birdnet_detections_total` twice, gauge and counter. `expfmt.TextParser` (`promtool`, Telegraf, the Python client) **rejects the whole document**; the Prometheus server merges a decreasing gauge into the counter the dashboard `rate()`s. **[FIXED]** | Done — and the gate found a third offender the audit had not. |
| **OB-2** | P1 | READ | The heartbeat fired only per detection. **[FIXED]** | Done. |
| **OB-4** | P1 | VERIFIED | `/api/v2/health` returns `200 "healthy"` while its own body says `detection_daemon: "stopped"`; the status code is gated on SQLite alone. This is the endpoint the container healthcheck polls and the one every monitor gets pointed at. | A `?strict=1` / `readyz` sibling returning 503 on a stopped daemon or a silence past `DEADMAN_HOURS`; keep the default 200 so a quiet season does not restart the container. |
| **OB-5** | P1 | READ | **An operational alert can be dropped and then never re-sent.** All three alerting loops set the episode latch *before* calling `notify()`, `notify()` swallows the failure with one `warn!`, and the send may not be attempted at all because the shared circuit breaker skips it with a **`debug!`** that the default filter drops for `birdnet_integrations`. Sequence: uplink drops, three detection notifications fail, circuit opens; 24 h later the deadman fires, latches, and its send is skipped silently; the link returns; `alerted` stays true for the process lifetime. And `apprise.rs:473` `skip_counts()` — which returns exactly the two counters that would answer "how many alerts were dropped" — has **zero production callers**. **[FIXED — "Latch an alert episode on delivery, not on the attempt"]** The defect was worse than described here: `send_notification_with_image` returned **`Ok(())`** for `(delivered: 0, first_error: None)`, which is precisely the fully-skipped case, so the send itself reported success. Two of the four prescriptions were adopted as written; the other two were changed on evidence. Forcing a probe through an open circuit was **not** done — the breaker already admits one probe per open period, and a caller that now retries rides that schedule and lands as soon as the destination comes back, so forcing would only add traffic to a dead endpoint. Raising the per-send skip to `warn!` was **reverted for detections**: at a detection every twenty seconds that is ~4 000 lines a day, which is why it was `debug` to begin with; the `warn` moved to the *transition* (`Breaker::on_failure` now reports the period it just opened for) and is kept per-send only for operational alerts. | Done; see Stage 2 landed. |
| **OB-7** | P1 | READ | **A backup that fails every week never alerts**, because `mark_ran` is called unconditionally after `run_backup_and_vacuum` and the station-health check reads `last_run_unix`, ignoring the `ok` column entirely. It can only detect the maintenance loop having *stopped*. And a **failed integrity check pushes nothing at all** — it reddens a badge and 503s an endpoint, but sends no notification, though `station_health.rs:19-20` names it as one of the two things the module exists for. Offsite failure is invisible everywhere: no counter, no `maintenance_runs` row, no health field, no alert. | Give the backup a verdict and use `mark_ran_with`; branch on `last_run_result`; add the recorded integrity failure as its own condition; give offsite its own job key. |
| **OB-12** | P1 | READ | **No chunk or file throughput counter exists**, and the one latency histogram is observed *per stored detection*, not per analysed chunk — its own HELP says so. So a station where inference runs perfectly and returns nothing (wrong labels, wrong sample rate, a model swapped by a bad update) has flat, empty latency series **identical to a station where inference is not running at all**. The four production drop reasons all live downstream of a prediction the model actually made. | `birdnet_files_analysed_total{source}` at the point the correlation id is minted (~5 760/day/source at the default segment length), plus `birdnet_chunks_analysed_total`. A flat counter with `audio_source_up == 1` is "capture writes, nothing analysed"; a rising counter with zero detections is "the model answers nothing" — the discrimination no surface can currently make. |
| **OB-14** | P1 | READ | Runtime clock correctness is **never re-checked**. `--doctor`'s clock checks run once, from `ExecStartPre`; at runtime capture tests only a floor and trusts anything above it absolutely. A Pi whose NTP has been unreachable for months, or whose timezone changed, records everything under the wrong hour with no runtime signal at all — a lost season that looks like a good one. Ties to **NT-1**, **NT-5**, **NT-6**. **[FIXED — "Notice a clock that has drifted off its time source"]** Both signals, as prescribed, with one change: `/run/systemd/timesync/synchronized` is a *fallback*, not a peer. It is created when `systemd-timesyncd` first synchronises and is never removed if synchronisation is later lost, so it answers "synced at some point since boot" — which is exactly the question this check must not ask, since the failure named here is a Pi whose NTP has been unreachable for months. `timedatectl show -p NTPSynchronized --value` reports the state now, so it is the authority. The probe has three outcomes, not two: `Unknown` (no systemd to ask) produces no condition and an absent metric, because a container's clock is the host's. Timezone drift is **not** covered — `doctor/clock.rs`'s `timezone_mismatch_check` still runs only at `ExecStartPre`. | Remaining: the timezone half, and the forward-jump detector (1.11). |
| **OB-3** | P2 | VERIFIED | The shipped Grafana dashboard has **`alert: []`** — not one rule — covers 8 of 21 metric families, omits `birdnet_detection_silence_seconds` (which the manual names first as the series to alert on) and 12 others, and labels a memory threshold at **half** the unit's real `MemoryHigh`. | Ship `alerting_rules.yml` with the four rules the docs already argue for; a CI gate that every name in the dashboard exists in the rendered exposition and vice-versa — which would also have caught **OB-1**. |
| **OB-6** | P2 | VERIFIED | Three surfaces grade the same disk three ways; one returned **HTTP 503 "critical" on a 69.6 %-full filesystem** whose own JSON body said 69.6 %. Every ext4 default has a 5 % root reserve, so a monitor polling that endpoint pages the operator on a healthy station — which is how a channel gets muted before the real alert arrives. Same as **PR-14**. **[FIXED]** With **PR-14**; see there. One thing the finding did not name: an existing unit test, `disk_usage_percent_with_reserved_space`, **asserted the defect** — same fixture, `assert!(u.is_critical(), "7/252 available is critical")` — which is what made `available < total / 20` look like a choice rather than a slip. The fixture is kept and the assertion inverted. |
| **OB-8** | P2 | VERIFIED | Home Assistant discovery advertises a `binary_sensor` with `device_class: connectivity` on `{prefix}/status`, and `publish_status()` / `publish_daily_stats()` have **zero production call sites**. Every station with discovery on registers a "Station Status" entity that is permanently unknown, so the obvious automation — *notify me when the station goes offline* — can never fire. There is also **no MQTT last-will**, and structurally cannot be: `publish()` opens a fresh connection per message at QoS 0, so no session exists for a broker to notice dying. **[FIXED — "Give the broker something to report when the station dies"]** Taken as prescribed, first option. A `PresenceSession` holds one otherwise-idle connection carrying a will (`{prefix}/status` = `offline`, retained, `QoS` 1) with a 30 s keepalive, so the broker publishes the will ~45 s after a station stops answering; the station publishes `online` retained on connect and `offline` retained on a clean stop. `publish_daily_stats` is fed from the same loop. Two things the finding did not reach: discovery configs were published **unretained**, so Home Assistant lost all four entities whenever *it* restarted — they are now always retained regardless of `MQTT_RETAIN`; and `MqttConfig::qos` had **zero readers** in the workspace while `publisher.rs`'s own module doc claimed a `QoS`-1-to-0 downgrade "after logging a warning" that did not exist, so `QoS` 1 now genuinely waits for a PUBACK. | Done; see Stage 2 landed. |
| **OB-9** | P2 | READ | "Test notifications" tests a code path the alerts do not use, and is **disabled for the configuration most stations have**: its button is enabled only when `apprise_url` (an Apprise *API server*) is set, so a station using native `ntfy://`/`discord://` routes sees "Not configured" and a disabled button while its alerts work fine. When enabled it builds a fresh client and POSTs directly, exercising neither the native routes, nor the CLI fallback, nor the circuit breaker, nor the rate limiter — i.e. none of the machinery that decides whether **OB-5**'s deadman alert leaves the box. No test at all for email or MQTT. **[FIXED — the push half; "Test notifications" now sends what an alert sends]** Taken as prescribed. `src/app.rs` hands the *same* `Arc<Mutex<apprise::Client>>` the three alert loops hold to the web layer as `birdnet_web::notifier::Notifier`, and the handler locks it and calls `send_operational_alert` — the identical call `announce::flush` makes, so the native routes, the `apprise` CLI fallback, the circuit breaker and the operational rate-limit bypass are all under the button. The button is enabled on *any* resolved destination, and the page names them (the labels are credential-free by construction). **The email and MQTT half is not done**: those channels still have no test of any kind, and this item did not add one. | Push done; see Stage 2 landed. Email and MQTT tests remain — 2.20. |
| **OB-10** | P2 | VERIFIED | The support bundle publishes `HEARTBEAT_URL` and `APPRISE_URL` **verbatim** — both bearer credentials carried as a path segment, which the name-based and `user:pass@`-based redactors both miss — in a file the tool invites the operator to attach to a bug report. Partly **[FIXED]**: the heartbeat URL is no longer logged at `INFO`, so it is out of `journal.log`; the `config.redacted` half remains. | Redact by shape: for any `scheme://host/rest`, keep scheme and host. Extend the existing `every_secret_key_is_redacted_from_the_support_bundle` gate. |
| **OB-11** | P2 | VERIFIED | Chaining the email redactor after the URL redactor **mangles every RTSP URL with a dotted host** into `***@host/path`, destroying the scheme and the username the first redactor deliberately kept. On an RTSP station that is the most diagnostic setting in the file. The gate is green only because its fixture hostname (`cam`) has no dot — the one hostname shape that never occurs in the field. | Apply the email redactor only when the value contains no `://`; re-point the fixture at `cam.example.com` and an IP. |
| **NL-1** | P1 | VERIFIED | **`NotifStatus::Queued` could not be stored, and had a production writer.** Migration 4 created `notification_log.status` with `CHECK(status IN ('sent','failed','skipped'))`. `Queued` was added to the enum later, documented at length — "accepted by this station but not yet delivered", parked for replay, deliberately distinct from `Failed` because *"an operator looking at a wall of red needs to know which one they are looking at before they go and climb a hill"* — and written in production by `daemon/processor.rs`'s store-and-forward path. Every insert was rejected by the CHECK, and `record_notification` discards the error at `debug!`, which the default filter drops. So a field station on flaky LTE produced exactly the bursts that comment describes and the Notification Center showed none of them: the distinction it draws was between one status that existed and one that never did. Found by running the code, not reading it — a new gate for the alert path asserted a `queued` row and got none. **[FIXED]** Migration 41 rebuilds the table (SQLite cannot alter a CHECK in place), and `every_notification_status_is_accepted_by_the_schema` enumerates `ALL_NOTIF_STATUSES` so a sixth status without a migration fails in CI. | Done. |
| **OB-13** | P2 | READ | The three alerting loops never call `record_notification`, so the notification log contains **every robin and no deadman**. An operator who suspects they missed an alert has no record to consult. **[FIXED — "Log the alerts about the station, not only the birds"]** One writer, in `announce::flush`, because 2.2 had already made that the single delivery path for all three loops — the finding's "shared `notify()`" now exists. `channel = "alert"` as prescribed. `NotifStatus::Queued` for an undelivered alert rather than `Failed`, and one row per episode rather than one per retry: the retry runs every poll, so a notifier down for a day would otherwise write ~288 rows for one alert. | Done; see Stage 2 landed and **NL-1**. |
| **OB-15** | P2 | VERIFIED | Measured **~11 520 baseline INFO lines/day/source** (two per analysed file at the default 15 s segment, 5 760 files/day; 236 B/line measured) — **1.6–2.8 GB/year**. No journald `Storage=` or `SystemMaxUse` is configured anywhere, so on a default Raspberry Pi OS without `/var/log/journal` the journal is **volatile**: ~30–45 days on a 2 GB Pi and **zero across a reboot**, so every watchdog bounce, power cut and update erases the evidence of what caused it. | A journald drop-in with the unit; demote the two per-file INFO lines to DEBUG — they are 92 % of the volume and carry nothing a counter would not carry better. Takes the year to ~150 MB. |
| **OB-16** | P2 | READ | Alert storms are genuinely well prevented — three-poll debounce, per-episode latch, recovery notices, a compile-time assertion that the debounce constant stays > 2. But **nothing ever re-notifies an open episode**: the only thing that re-arms one is a process restart. The posture is *one push, ever, per fault, over a channel never tested end to end (**OB-9**) that may drop it silently (**OB-5**)*. For a fault lasting four months that is the wrong side of the trade. | Re-notify open episodes with exponential spacing (24 h, 72 h, then weekly, capped), carrying "still broken, N days". |

**Diagnosability from 40 km.** Genuinely strong for four of five microphone
failure modes: the `/station` Health tab gives per-source chips, a rolling 24 h
uptime strip, time since last audio, retry/backoff state, vitals, and the
noise-floor panel that separates "gone deaf" from "quiet season". What is
missing is all CLI-only: no remote `--doctor` for the *hardware* half (738 lines
of device enumeration and RTSP reachability that cannot run from the browser),
no support bundle over HTTP, no journal history beyond a 200-line RAM ring, and
no "record 15 s from source X now and let me download it" — the one action that
settles a wind/water/gain question. Highest value per line: mount
`doctor::collect_json` at `GET /admin/doctor.json` (it already exists and is
already what the bundle embeds).

### 3.7 Parity with `Nachtzuster/BirdNET-Pi` (`NP-*`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **NP-1** | P1 | READ | **Every "View on eBird" link 404s.** `species_pages.rs:687` builds `https://ebird.org/species/{encoded_sci}`; eBird species pages key on the six-letter **species code**, and upstream ships a 6 525-line `scripts/ebird.php` mapping scientific name → code for exactly this (`common.php:492`). The code is *already on the station*: `labels.rs:166-169` documents column 1 of the geomodel label file as the eBird species code and drops it, with the comment "nothing downstream keys on it" — which is now false. | Keep column 1 when parsing the geomodel TSV; use it to build the link; fall back to the eBird search URL when unknown. |
| **NP-2** | P2 | READ | `FullDiskAction::Keep` exists (`disk/manager.rs:18-23`) and **both call sites hardcode `Purge`**, so an operator cannot choose "stop rather than delete my data" — upstream's `FULL_DISK=keep`. For an irreplaceable season that is the wrong default to be unable to change. | Wire the setting through; a station-health condition for the stopped state. |
| **NP-3** | P2 | READ | Spectrogram labels use a 5×7 ASCII bitmap font, so `Mésange` and every CJK/Cyrillic/Thai common name burns into the image as boxes. Plus a byte-vs-char background-bar bug at `font.rs:19`. | A small embedded Unicode font, or render labels as SVG/HTML overlay rather than into the PNG. |
| **NP-4** | P2 | READ | No preview of which species the geomodel admits at a candidate `SF_THRESH` (upstream `species.py --threshold`). `/admin/species/test` answers a different question. | A preview endpoint running the geomodel at the candidate threshold and diffing against the current one. |
| **NP-5** | P1 | VERIFIED | **No Raspberry Pi power or throttling telemetry.** `grep -rniE "get_throttled\|vcgencmd\|under.?voltage"` returns nothing. Upstream reads `vcgencmd get_throttled` (`extra_info.sh:7-16`). Undervoltage on a long mains run or a marginal solar budget is *the* commonest field failure on a Pi, it corrupts SD cards, and it presents as random instability with no other signal. | Read `vcgencmd get_throttled` (or `/sys/devices/platform/soc/soc:firmware/get_throttled`); export `birdnet_pi_throttled` and the sticky under-voltage bit; a station-health condition. |
| **NP-6** | P2 | READ | The support bundle stages only `uname`, `df` and `journalctl` — no `arecord -l`/`-L`, no `--dump-hw-params`, though `probe.rs:156` already runs the last of those. The bundle is the artefact designed to answer "what does the OS see", and it does not carry it. | Add the audio-device inventory to the bundle. |
| **NP-7** | P2 | READ | 36 languages are "supported", **zero label packs ship**, and `src/cli.rs:750` documents `labels_de.txt` while `i18n.rs:80` looks for `de_labels.txt` — so the documented filename cannot work. | Ship the packs, or say plainly that the operator must supply one; fix the filename in one place. |
| **NP-8** | P2 | READ | Relabelling updates database rows only; the clip keeps the old species in its filename and `recordings.rs:298` filters by that filename, so a relabelled detection's audio becomes unfindable under its new name. | Rename the clip alongside the row, or index by detection id rather than filename. |
| **NP-9** | P3 | READ | No station banner image, and the `custom_image_dir` hint wrongly cross-references upstream's `CUSTOM_IMAGE`. | Implement, or correct the hint. |
| **NP-10** | — | READ | Clock/timezone/NTP are not settable from the UI. **Correct divergence** — upstream needs `NOPASSWD: ALL` for it — but the Pi-without-RTC cost is unaddressed. Recorded, not planned; see **NT-1**/**NT-6** for the real answer. | — |
| **NP-11** | P3 | READ | `apply_update` is reachable but manual-only; no unattended path, no reboot control. Overlaps **LC-3**. | Resolve with **LC-3**. |
| **NP-12** | P3 | READ | The detection stream and the spectrogram stream both flow to the same page and nothing draws one on the other. | Overlay detections on the live spectrogram. |
| **NP-13** | P3 | READ | The accessibility pitch shift works on the live stream and **not on saved clips** — the same argument that made N-2 a Tier-1 item, left unfinished. | Apply the same shift in the clip player. |

### 3.8 Signal path, versus `tphakala/birdnet-go` (`S-*`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **S-1** | P1 | READ | **Analysis windows never cross a segment boundary**, so there is a seam every 15 s, and the tail window of every segment is zero-padded silence that still gets classified. A call spanning a boundary is analysed as two half-calls. | Carry the tail of each segment into the next window, reusing the neighbour-splicing machinery that already exists for pre-capture (G-3). |
| **S-2** | P1 | READ | A disk-full purge deletes the oldest 10 % globally, with **no minimum-clips-per-species floor** and no diversity or confidence ordering — so the single clip of the year's rarest bird goes first. | Order by (species clip count, confidence, age); never take a species below a floor. |
| **S-3** | P1 | VERIFIED | Unanalysed audio is deleted oldest-first with no counter and no queue gauge, and a doc comment asserts safety without checking it. Same as **PR-1**. | See **PR-1**. |
| **S-4** | P1 | READ | Clip WAVs are written straight to their final path — a torn write leaves a truncated file the database points at for ever. Same as **PS-7**. | See **PS-7**. |
| **S-5** | P1 | READ | **The privacy filter's threshold is inert**: `max(10, …)` at `top_n = 10` means the configured value never binds, so its real sensitivity is the *global detection threshold*. Tuning false positives silently changes privacy behaviour. | Make the threshold bind; gate the discrimination (a change in the privacy threshold must change privacy behaviour, and a change in the detection threshold must not). |
| **S-6** | P2 | READ | Nothing reports geomodel↔classifier vocabulary coverage, so the silent exclusions **G-15** describes are invisible; and there is no `pass_unmapped_species` escape hatch. | A coverage figure on `/admin/species` and in `--doctor`; an opt-out. |
| **S-7** | P3 | READ | `PolynomialDegree::Septic` resampling has **no anti-aliasing filter**. Latent today because of the 48 kHz schema cap, live for externally dropped files, and a trap the moment that cap is lifted for **G-14**. | A decimation low-pass before downsampling; a gate on a swept-sine alias. |
| **S-8** | P2 | READ | Inference threads hardcoded to 2, no config path, no inter-op setting — the cheapest available throughput lever, and overlap is throughput. | An `inference_threads` setting defaulting to today's 2. |
| **S-9** | P2 | READ | No INT8/ARM model path; upstream ships `BirdNET_INT8_ARM.onnx` and remaps to it on arm64. | Ship the INT8 model and select it on aarch64; measure the accuracy delta before defaulting to it. |
| **S-10** | P2 | READ | One global duplicate interval gates the database row **and** BirdWeather/MQTT together; the per-species cooldown seam in `apprise.rs` is populated by nothing. | Separate the persistence interval from the notification cooldown; populate the seam. |
| **S-11** | — | READ | Sound-level monitoring is a ~3 % duty-cycle sample, not a continuous series. Defensible; recorded as a decision, not a gap. | — |
| **S-12** | P2 | READ | `bit_depth` is stored, `CHECK`-constrained, defaulted to 24 and shown in the UI; the capture path **hardcodes `S16_LE` and never reads it**. Dead configuration that lies to the operator. | Honour it, or remove it from the schema and the UI. |
| **S-13** | P1 | READ | No `usb-id:`/bus-path device identity. Ties to **AU-1**. | See **AU-1**. |
| **S-14** | P2 | READ | No clip reconciliation: after any purge, `File_Name` rows point at deleted files and nothing counts them. | A reconciliation pass in `run_clip_retention`; a `birdnet_orphaned_clips` gauge. |
| **S-15** | P3 | READ | Clip export needs external ffmpeg/sox and has no Opus; upstream encodes natively. | Optional; low priority. |
| **S-16** | P2 | READ | The daylight filter runs only the night→diurnal direction and resolves species by **genus prefix against a hardcoded list**. | Both directions; resolve through the taxonomy work in **G-15**. |

### 3.9 Web, security and operations, versus `tphakala/birdnet-go` (`O-*`)

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **O-1** | P1 | VERIFIED | **The `/api/v2` surface is 100 % read-only.** No mutating route method in any of the fourteen modules mounted under it. Every mutation is an HTMX form post returning HTML behind a same-origin check — trivially satisfied by any script that sets a matching `Origin`, and therefore not a contract anyone can build on. Upstream has 54 mutating routes. Consequences: no supported automation; Home Assistant and Node-RED can read but never act; and our own front end is the only client, so a fragment-markup change silently breaks whatever automation exists in the wild. | Port the ~8 with operational weight, reusing the handlers already behind the HTMX routes: review, lock, delete, batch, `GET` and `PUT /settings`, `POST /control/restart`. Bearer auth, and the CSRF guard must **skip** bearer-authenticated requests — a header token is not attachable by a cross-site form, which is the entire premise of the check. |
| **O-2** | P1 | VERIFIED | **The audit log is never written.** Table, store, admin page and pruner all exist; `AuditLog::record` has **zero production callers** — every call site is inside its own `#[cfg(test)]` block. `/admin/audit` is permanently empty, which on a shared station reads as "nothing happened". The repo already caught half of this: the *pruner* was wired after being found to have no caller; the writer never was. **[FIXED — "Record who changed what"]** The helper as prescribed, called from every surface listed plus two the finding did not name: species filters and audio sources, which are what decide whether a season's gap is a real absence. 24 actions in all. One change to the prescription: settings values are not "redacted through the existing secret list", they are **never recorded at all** — only the names of the changed keys. `rtsp_url` is the reason a key-name allow-list would not have been enough: an RTSP URL carries `user:pass@` in its authority while its key name says nothing about a secret, which is the same trap `redact_url_credentials` exists for. A save that changed nothing writes no row, because the form posts every field every time. Destructive actions are recorded *before* the work, since a process that does not survive a restore has no "after" to write from. | Done; see Stage 2 landed. |
| **O-3** | P1 | VERIFIED | **The admin log viewer streams a channel nothing publishes to.** `src/main.rs:146-148` installs exactly two layers and no `tracing_subscriber::Layer` implementation exists anywhere in the crate; `LogBroadcaster::new()` is called **three separate times** in `state.rs`, so they are three distinct channels anyway. `GET /admin/system/logs` replays an empty backlog and then emits keep-alives for ever. On Docker, where `journalctl` is unavailable to the user, this page is the whole story. **[FIXED — "Show the operator what the station logged"]** Taken as prescribed, with one correction to this row: **"three separate times ... so they are three distinct channels anyway" is wrong.** The three `LogBroadcaster::new()` calls are in three *alternative* constructors — `AppState::new`, `new_with_analytics`, `from_connection` — and a run builds exactly one `AppState` (`src/app.rs:184`). There was one channel, and nothing published to it; the count was never the defect. `LogCapture` now implements `Layer`, is installed as a third `.with(...)`, and the broadcaster is built in `main` *before* the subscriber and handed to the state, because the layer has to exist at `init()` time and the state does not exist yet. `errors.jsonl` sits beside the database, takes ERROR and WARN only, is capped at 1 MB, and is a bundle member. URL credentials are stripped in the layer rather than per call site, because that file travels in the support bundle. | Done; see Stage 2 landed. |
| **O-4** | P1 | VERIFIED | **No private mode.** `grep` for `private_mode`/`public_access` returns zero hits. The dashboard, the whole API, the live audio stream and both WebSockets are unauthenticated with no configuration that changes it. On a station reachable through a tunnel or a port forward, anyone with the URL sees the full detection history and can **open a live microphone feed of somebody's garden**. `--listen 127.0.0.1` is not a substitute; that is what the tunnel connects to. The privacy argument used to decline Sentry applies here with more force, and this is on by default. | `BIRDNET_PRIVATE_MODE` plus `BIRDNET_PUBLIC_ACCESS=live_audio,share`, applied by moving `public_routes()` inside `auth_middleware::apply` with an exempt set. Gate both directions, including that an operator-minted share link keeps working. |
| **O-6** | P1 | READ | Login has **no dedicated throttle and no lockout**; the global limiter permits ~30 Argon2id guesses/second per IP. On a Pi that is a self-inflicted CPU denial of service on the box that is supposed to be running inference, and the passwords in play are installer-generated or hand-typed. Upstream allows 5 attempts per 15 minutes. | A second limiter on the `/login` POST keyed on the existing `ClientIp` extension, plus a per-username counter. Gate the discrimination: a sixth attempt from a *different* IP still gets through. |
| **O-16** | P2 | VERIFIED | **No per-source capture restart and no jobs endpoint.** A two-source station that loses its RTSP camera must restart the whole daemon, dropping the working microphone's in-flight audio and analysis buffers — remotely, the difference between a five-second recovery and losing the dawn chorus. The supervisor already has the per-source machinery; the seam is unexposed. | `POST /api/v2/control/restart-source/{id}` signalling the existing supervisor; `GET /api/v2/system/jobs` over `maintenance_runs` (a ten-line handler that answers "did the backup run?"). |
| **O-5** | P2 | READ | `X-Frame-Options: SAMEORIGIN` and `frame-ancestors 'self'` are hard-coded, so **our own `/kiosk` page cannot be embedded in Home Assistant** — the commonest second screen for a home station. The failure is a blank iframe and reads as a bug in the embedder. | `BIRDNET_FRAME_ANCESTORS`, with the `X-Frame-Options` insert made conditional (it has no allow-list form, so a non-default value means omitting it). |
| **O-7** | P2 | VERIFIED | `.env.example` and the code disagree about **33 keys**: 9 consumed but undocumented — including `BIRDNET_TRUSTED_PROXIES`, `BIRDNET_BASE_PATH`, `BIRDNET_CORS_ALLOWED_ORIGINS` and `BNB_SESSION_SECRET`, exactly the ones an operator behind a proxy needs — and two documented but consumed by nothing, one shipped **uncommented**. Same evidence as **LC-7**. | The drift gate in **LC-7**. |
| **O-8** | P2 | VERIFIED | `openapi.json` omits **8 live endpoints** including `/stream` and both WebSockets. The only gate checks `info.version`; nothing compares the path set, so the worse direction — a route deleted while its documentation survives — is unguarded too. | Set-equality between `openapi.json`'s paths and the router's, best done by having both consume one `const ROUTES`. |
| **O-9** | P2 | READ | Logs go to stderr only: no file sink, therefore no rotation; no JSON mode; no logger-level redaction. Secrets *are* handled well at the support-bundle boundary, but that is a per-call-site convention, not a property of the logger. On Docker with the default json-file driver and no `max-size`, growth is unbounded on the same card as the database. Ties to **OB-15**. | `tracing-appender` behind `BIRDNET_LOG_FILE` (unset = today's behaviour exactly), `BIRDNET_LOG_FORMAT=text\|json`, and a field visitor reusing the existing `SECRET_KEYS` list rather than inventing a second one. |
| **O-10** | P2 | READ | No HSTS, and the comment explaining its absence — "the binary serves plain HTTP" — is false since `tls.rs` landed. A station on `--tls-mode manual --tls-redirect` answers the first plain request with a 301 and never tells the browser to remember: the classic SSL-strip window. No COOP either. | Emit HSTS only when the request arrived over TLS **and** the mode is not self-signed — pinning HSTS onto a self-signed cert locks the operator out of their own station — and put that reason in the comment. |
| **O-11** | P2 | READ | No SSRF guard on operator-configured webhook URLs, and — only because of **O-4** — an anonymous visitor can create a rule and use the Test button as a blind port scanner against the host's network, including cloud metadata addresses. Mitigated by the test endpoint deliberately not echoing the body. | A blocked-target check reusing the already-tested `IpCidr` parser, plus a bounded redirect policy. Leave RFC1918 **allowed**: `http://homeassistant.local:8123/` is the commonest legitimate target. |
| **O-14** | P2 | READ | The "no password ⇒ open admin" bypass keys on the **seed `admin` row specifically**, so an operator who creates their own admin account and never sets `CADDY_PWD` leaves the panel open, with their new password protecting nothing. Three places decide this with three different predicates. It is announced (a `warn!`, and `--doctor` fails) but the accounts page the operator is looking at says nothing. | One predicate — "any enabled admin-role user has a real hash" — called from all three sites; a banner while the bypass is active. Gate: create a second admin with a password, leave the seed row empty, assert `/admin/settings` is not 200. That test fails today. |
| **O-12** | P3 | READ | Trusted-proxy and rate-limit settings are startup-only, absent from the admin UI, and the limits are compile-time constants. A mistyped CIDR is discovered only in `journalctl` and needs a restart. | Move both into `settings` with env override, read through an `ArcSwap`; validate the CIDR in the form. |
| **O-15** | P3 | READ | Prometheus metrics are on the public port with no auth, exporting per-source acoustic noise-floor drift — a fair fingerprint of what a garden sounds like and when someone is home. | Gate it with `/admin`, or bind a second loopback-only listener. |
| **O-13** | P3 | READ | English-only; `lang="en"` hard-coded in nine shells. Upstream ships 16 locales, and BirdNET-Pi's user base is heavily non-English. **Explicitly not recommended now**: retrofitting i18n into ~120 server-side render functions building HTML with `format!` is enormous, and half-done i18n is worse than none. | The honest first step at ~1 % of the cost: make `lang` a template variable driven by a `BIRDNET_UI_LANG` setting and get date/number formatting right — which fixes the screen-reader and browser-translation story. |

### 3.10 Found by this pass, outside the eight scopes

| ID | Sev | How | Finding | Fix |
|---|---|---|---|---|
| **ARM-1** | P1 | VERIFIED | **No aarch64 test has ever executed in this repository.** CI's `cross-aarch64` job is `cargo check --workspace --all-features --target aarch64-unknown-linux-gnu` and nothing more; all four prior audits list this as uncovered. It is now closable: this repository is public, and GitHub's `ubuntu-24.04-arm` runner is free and unlimited on public repositories at 4 vCPU / 16 GB RAM / 14 GB SSD (per GitHub's hosted-runner documentation, fetched 2026-09-03). The one real constraint is disk: `cargo build --workspace --all-targets` produces a **14 GB** `target/` here, measured with `du -sh`, so a naive full test build will not fit and the job must be scoped. | A native aarch64 test job over the crates where target-specific behaviour actually bites — `birdnet-core` (DSP, civil-date arithmetic, pointer width), `birdnet-scheduler`, `birdnet-timeseries` — with `debug = 0` and an explicit statement in the workflow of what is excluded and why. ARM is weakly ordered where x86 is TSO, so a missing `Acquire`/`Release` is invisible in every test run this project has ever done. |
| **AU-1** | P1 | READ | **A resolving ALSA card index passes the doctor silently, and still passes after it starts pointing at a different device.** `probe_alsa_device` returns `Check::pass` for `CardRef::Index(n)` whenever *some* card `n` exists — and `parse_card_ref`'s own doc comment, nine lines below, records that "the same microphone was `card 1` before a cold reboot on a Raspberry Pi 4 and `card 3` after it". After that re-enumeration the check still passes while the daemon records from whatever now occupies index 1. The installer writes the stable `plughw:CARD=<id>` form, but `quickstart.sh` does not (**LC-9**), and nothing tells a currently-working station to move. | On the `Index` pass path, emit an advisory naming the stable `plughw:CARD=<id>,DEV=<n>` form when the listing offers a usable id — `stable_form_hint` already composes exactly that string for the failure path. Stronger, and the real detector: record the resolved card **id** alongside the device string in `audio_sources` and alarm when it changes under a fixed index. |

### 3.11 Checked and found sound

Recorded so the next pass does not re-open them.

* **The capture supervisor** is the strongest module in the tree: per-source
  backoff that never permanently gives up, a schedule gate, a real uptime ring,
  and metrics driven from process health rather than from detections.
* **The crash-loop-to-death risk is genuinely closed.** `StartLimitIntervalSec=0`
  with `RestartMaxDelaySec` backoff, and eighteen lines in the unit explaining
  why a unit that gives up "would stay down until someone walked to it".
* **The watchdog is correctly gated on loop progress**, not on detections, so a
  quiet winter night does not restart the station.
* **`panic = "abort"`** rules out a silent zombie pipeline stage.
* **Systemd hardening** — `ProtectSystem=strict`, `PrivateTmp`,
  `NoNewPrivileges`, an empty `CapabilityBoundingSet`, a restricted
  `RestrictAddressFamilies` and a `SystemCallFilter` — is markedly better than
  upstream's, which ships no unit at all.
* **Release integrity**: `SHA256SUMS` over archives plus `install.sh` and
  `uninstall.sh`, SLSA provenance, keyless cosign, and fatal refusals for a
  missing sums file, a missing entry, a duplicate entry and a mismatch.
* **Supply chain**: `cargo-deny`, an independent `cargo-audit` run kept
  deliberately redundant, `cargo-machete`, and a weekly schedule so new
  advisories surface without a commit.
* **Migrations** are forward-only, per-step transactional, resumable after a
  power cut, and warn on a downgrade rather than corrupting.
* **DuckDB auto-quarantine-and-rebuild** with a three-signal drift check.
* **Database corruption recovery at startup** walks the whole backup ring
  newest-first and only starts fresh when every candidate fails.
* **BirdWeather store-and-forward** is the best-built network component here:
  persisted, capped, oldest-dropped-at-enqueue, jittered backoff, poison
  payloads dropped rather than retried for ever, depth exported as a gauge.
* **Monotonic time where it matters**: every rate limiter, circuit breaker,
  cooldown and watchdog uses `Instant`. No timer, backoff or rate limit runs on
  the wall clock.
* **Leap years** are swept 1800–2400 *and* the leap count asserted, so a
  both-say-no implementation cannot pass.
* **Atomics**: 61 `Relaxed` uses were reviewed for the release/acquire pattern
  that x86's TSO hides and ARM's weak ordering exposes. The shutdown flags and
  liveness counters are correct with `Relaxed`. One benign ordering hazard
  exists — `clock.rs`'s two independent atomics let a reader see a fresh
  `COMPUTED_AT` with a stale `OFFSET_SECS` — which is P3 and folded into
  **NT-9**.
* **Captive portals** are mostly handled by construction: BirdWeather parses
  JSON, image downloads check `Content-Type: image/`, and the chat routes check
  the response body, so an HTML portal page is an error rather than a success.

---

## 4. The plan

Ordered by *what a year in a field would actually cost*, not by severity label
alone: a finding that loses a season outranks one that loses a week, and one
that hides another finding outranks the one it hides.

Every item is done when it works end to end **and** carries a gate that was
observed failing against the code it was written for, with the failure text in
the commit message — `CLAUDE.md`'s rule, which several findings above exist
because nobody applied.

### Stages 0 and 1 — landed on this branch

**All five P0s are closed**, each with a gate observed failing against the code
it was written for and the failure text recorded in the commit message. The
workspace suite went from 3 425 passing at the branch point to **3 570** where
that work merged (`f33eb9e`), with nine Stage 2 items landed (3 465 at the end
of Stage 1); the installer suite from eight tests to eleven
(`installer/test/*.sh`, excluding the `run-ci.sh` harness), all passing. Every
figure here is from a run, not a running total — this sentence said "eight to
ten" until the count was taken again, and said "3 567 with seven Stage 2 items"
until both were taken again at the merge commit.

| # | Item | Finding | Gate, observed failing first |
|---|---|---|---|
| 0.1 | The geomodel is asked about the season the audio was recorded in | **NT-7** | 7 unit + 3 integration tests. Three mutants of `birdnet_week` killed; against a stub returning `0` — what `run.rs` passed — the discrimination test prints `left: 0, right: 0` for recordings six months apart. |
| 0.2 | `/api/v2/metrics` is a document a Prometheus parser accepts | **OB-1**, **PR-10** | 3 structural tests over the composed body. All three fail against the previous exposition, and rule 3 named a third offender the audit had missed: `birdnet_species_total` was also a gauge wearing `_total`. |
| 0.3 | The heartbeat is a timer, not a per-detection ping, and stops logging its own credential | **OB-2**, **NT-16**, half of **OB-10** | 3 loopback tests + 4 redaction tests. Against no loop all three fail; against a one-shot startup ping, "a ping arrives" passes and "the ping repeats" fails — the discrimination. |
| 1.1 | The weekly backup completes on a live station | **PS-1** (P0) | Reproduced independently: 30 s, 1 407 concurrent writes, no completion. After `step(-1)` the same work takes 0.65 s. The quiet-database counterpart passed both before and after, which is what makes the writer the discrimination. |
| 1.2 | The offsite upload cannot wedge the maintenance loop | **NT-11** | Reproduced: the previous client returned headers and then waited past 20 s for a body that never came. Two counterparts — a prompt server, and one dribbling a byte every 400 ms — pass, so this is a stall detector and not a shorter deadline. |
| 1.3 | A zero-length database is not "healthy" | **PS-2** (P0) | Reproduced: a zero-length `birds.db` reported `database integrity check passed`. Four of five tests fail against the previous code; the fifth passes both ways and is why the fix cannot be "return false for everything". |
| 1.4 | Clock plausibility on the detection write path | **NT-1** (P0) | With the check disabled a `1970-01-01` detection gave `detections = 1, quarantine = 0`; with it replaced by `if true` the counterpart failed with "a real date must still be filed". Migration 40 widens the quarantine CHECK, and the repo's own drift gate turned red before a line of it existed. |
| 1.5 | One clock floor, and no date-based deletion on an unset clock | **NT-5**, half of **NT-4** | The doctor/supervisor agreement sweep fails at `1578268800` against the old constants. See the caveat below. |
| 1.6 | Atomic binary swap, and the `.prev` the manual promises | **LC-1** (P0) | 7 of 10 assertions fail against `install -m 0755`, including "the live binary was unlinked; a power cut here leaves no binary at all". |
| 1.7 | The model is verified, and a partial download resumed | **LC-2** (P0) | "the truncated model was skipped — this is the defect", for the model and for the labels. The counterpart — a verified model must not be re-downloaded — passes either way. |
| 1.8 | A reachable red on `/api/v2/health` | **OB-4**, **PR-5** | Against the previous status logic, `left: 200, right: 503`; against a version making every request strict, `left: 503, right: 200` — the change that would put field stations into a Docker restart loop. |
| — | Two pruners with no production caller | **NT-18** | Both tables shrink; with the `reviewed = 1` condition removed the counterpart fails on the surviving row. |

> **What 1.5 does not do.** The floor catches a clock that is too *early*, which
> is the common case on RTC-less hardware. A clock far in the **future** — the
> direction the probe demonstrated reclaiming an entire clip library in one pass
> — is not caught, because catching it needs a reference the floor does not
> have: distinguishing "the clock jumped forward nineteen years" from "nineteen
> years passed" requires comparing the wall clock against the monotonic clock
> across one process lifetime. That limit is stated in the code rather than
> implied, and is item 1.11 below.

### Stage 1 — the rest

| # | Item | Finding | Why here |
|---|---|---|---|
| 1.9 | Never write to a database known to be corrupt | **PS-5** | Detected daily, then written to anyway until someone reboots. |
| 1.10 | Atomic clip and segment writes | **PS-7**, **S-4** | `.part` + `rename` + `sync_all`; the pattern is already in `entrypoint.sh`. |
| 1.11 | The monotonic-versus-wall step detector | **NT-4**, remaining half | The forward-jump direction 1.5 does not cover. |
| 1.12 | Raise `doctor/model.rs` off its one-megabyte threshold | **LC-2**, remaining half | The installer no longer *produces* a truncated model; the doctor still cannot *detect* one that arrived another way. |


### Stage 2 — a station can say what is wrong with it

Nothing here changes what the station records; everything here changes whether a
person 40 km away learns that it stopped.

**Landed so far:**

| # | Item | Finding | Gate, observed failing first |
|---|---|---|---|
| 2.1 | A throughput counter, so "hearing nothing" and "not running" are distinguishable | **OB-12** | The discrimination is an explicit assertion: a station that analysed ten files and detected nothing must not render identically to one that analysed none. The two `run.rs` call sites are not CI-reachable (they need the 541 MB model); both sit in the `Ok` arm of `process_and_infer_filtered`, so "analysed" cannot drift to mean "attempted". |
| 2.2 | Latch an episode only on a *delivered* alert; let operational alerts bypass the detection rate limiter | **OB-5** | 25 gates. Deleting the `(0, None)` arm fails four, one printing *"a notification that reached nobody was reported as sent"*. `admit_priority` delegating to `admit` fails *"an operational alert was dropped by the detection rate limit"*; returning `Send` unconditionally fails the counterpart, which is what stops the fix from hammering a retired webhook. `Outbox::settle` dropping the alert whatever happened — the shipped latch-on-attempt — fails *"an undelivered alert stays and is offered again"*. |
| 2.3 | Record the backup's verdict; alert on a *failed* integrity check, not only a stale one; give offsite a job key | **OB-7**, **PS-16** | Verdict ignored (the shipped code): *"a recorded failure is a fault the moment it is recorded"*. The counterpart — alert on every recorded run — fails with *"a successful run must produce nothing"*, which is the rule that would page a healthy station weekly. |
| 2.5 | A runtime clock-sync condition and gauge | **OB-14** | 9 gates. The first mutation — deleting `check_clock` from `evaluate`, which is the shipped state — **killed nothing**: every gate tested the policy function and none tested that anything called it. That hole is the finding inside the finding, and it is why `evaluate` now runs a named `CHECKS` table that `every_documented_condition_is_actually_checked` reads. Re-run against the table, the same mutation fails that gate alone. Five more killed: `Unknown` treated as broken (every Docker station would alert about its host's clock), the two clock faults given different episode keys, the plausibility floor skipped, the probe answering `Unsynced` when it cannot tell, and a tri-state gauge rendering `0` instead of being absent. |
| 2.6 | Wire a `tracing` layer to the log broadcaster; persist ERROR/WARN to a file the bundle carries | **O-3** | 12 gates. 6 mutations killed: no layer publishing (the shipped state — 5 fail, `left: 0, right: 1`), `with_log_broadcaster` made a no-op, which is the shipped *arrangement* (only the wiring gate fails, every layer gate stays green — so it tests the wiring, not the layer), fields dropped from the message, every level persisted (`only ERROR and WARN persist: {"level":"INFO",…}`), no URL redaction (`cannot reach rtsp://admin:hunter2@cam.local/stream`), and an uncapped file. |
| 2.7 | Write the audit log | **O-2** | 15 gates, 6 mutations killed. `audit()` writing nothing — the shipped state — fails 6 and leaves the two "must record nothing" gates green. The rest each fail one: login recording `fail` whatever happened, a settings save recording every submission, metadata carrying values (`rtsp_url`'s `user:pass@` is the fixture), a failure row not naming who was tried, and an action name with `threshold` misspelled, which the source-scanning vocabulary gate catches — the same lesson as 2.5's `CHECKS` table: a set expressed only as scattered call sites cannot be checked. |
| 2.10 | One disk denominator, everywhere | **OB-6**, **PR-14** | 6 gates. 4 mutations killed. The shipped predicates fail the reproduction (`a disk 76.6 % full is not critical (was: 9167069184 available < 13527658700 = total/20)`) and the swept property gate; `is_critical` returning `false` unconditionally fails the two full-disk counterparts, so the fix is not "stop reporting"; a `CRITICAL_PERCENT` of 98 fails the purge-threshold coherence gate. The fourth is the instructive one: making `used_percent()` divide by `total` **as well** leaves the property gate green — two surfaces agreeing on the same wrong number — and is caught only by the reproduction, which pins the answer to what `df` says. |
| 2.14 | Publish the MQTT status topic that discovery already advertises, with a last will | **OB-8** | 9 gates against a broker stub that *decodes* CONNECT and PUBLISH rather than matching bytes. 8 mutations killed, each by one gate: no will (`"a will was registered"`), a will on the stateless publish too (only the discrimination test fails), the will written after the username — a well-formed packet that publishes the password to whatever the broker reads as the topic — `ping` that writes and never reads (`"an unanswered ping must fail"`), `config.qos` ignored, which is the shipped code (`"an unacknowledged QoS 1 publish must not report success"`, while the `QoS` 0 counterpart stays green — the fix is not "every publish now blocks"), the retain override ignored, also shipped (`"override honoured"`), `shutdown` that disconnects without saying offline (`left: 1, right: 2`), and an unretained will. |
| 2.15 | Operational alerts reach the notification log | **OB-13**, and **NL-1** found while doing it | 11 gates, 6 mutations killed. `flush` recording nothing — the shipped state — fails 4 and leaves `no_notifier_configured_writes_nothing` green. Then: `Queued` written as `Failed`, a row per retry instead of one per episode, placeholder species columns, a loop sending inline again (the pre-2.2 shape, caught by the source scanner), and the CHECK left un-widened — `"the schema rejects the `queued` status this code writes: CHECK constraint failed"`. That last one is **NL-1**: the two behavioural gates were written, run, and failed against the shipped schema before the migration existed. |
| 2.16 | "Test notifications" sends what an alert sends, and is live whenever a destination resolved | **OB-9** | 11 gates — 5 behavioural against a local destination through the real admin router, 6 at the renderer. Against the shipped handler four of the five behavioural gates fail: the button reaches nothing (`left: 0, right: 1` requests at the station's own destination), it renders `class="btn-disabled" … disabled` for a station whose native route is working, an open circuit is reported as *"Apprise URL not configured"*, and the module holds two HTTP clients (`left: 2, right: 1`). The fifth — no notifier at all still yields a disabled button and an error, not a send — passes **both** ways, which is what stops the fix being "always enabled". The discrimination is the open circuit: with `Gate::admit_priority` returning `Send` unconditionally the other four stay green and that one fails, so this is a test that goes through the shared guards rather than one that merely reaches the destination. |

**Still to do:**

| # | Item | Finding |
|---|---|---|
| 2.4 | Undervoltage and throttling telemetry | **NP-5** |
| 2.8 | Re-notify open episodes on a widening schedule | **OB-16** |
| 2.9 | Alert rules and a dashboard/exposition agreement gate | **OB-3** |
| 2.11 | Prune quarantined stores on a retention schedule (detection and the condition landed with 2.3) | **PS-6**, remaining half |
| 2.12 | Read-only-remount detection | **PS-9** |
| 2.13 | `--doctor` measures the card, not the RAM disk | **PS-8** |
| 2.17 | Redact by shape in the support bundle; stop mangling RTSP URLs | **OB-10**, **OB-11** |
| 2.18 | journald drop-in; demote the two per-file INFO lines | **OB-15**, **PS-19**, **O-9** — the volatile-journal half of **OB-15** is now partly covered by `errors.jsonl` (2.6), which survives the reboot; the 1.6–2.8 GB/year of INFO is not |
| 2.19 | `GET /admin/doctor.json` and a support bundle over HTTP | §3.6 |
| 2.20 | A "Test notifications" path for email and MQTT | **OB-9**, remaining half |


### Stage 3 — a station survives its own maintenance and its own operator

| # | Item | Finding |
|---|---|---|
| 3.1 | `VACUUM` that does not lock out the writer or need 3× the file | **PS-3**, **PR-12** |
| 3.2 | A headroom reserve for the database | **PS-12** |
| 3.3 | Stop purging clips when the purge frees nothing, and name the real consumer | **PR-7** |
| 3.4 | A minimum-clips-per-species floor in the disk-full purge | **S-2** |
| 3.5 | Protect unanalysed audio; count what is dropped | **PR-1**, **S-3** |
| 3.6 | A timeout on every subprocess; move clip conversion off the event thread | **PR-8** |
| 3.7 | Stop before start on every capture restart path | **PR-9** |
| 3.8 | `--apply-config`: validate, restart, restore on failure | **LC-6** |
| 3.9 | Unknown-config-key detection and the `.env.example` drift gate | **LC-7**, **O-7** |
| 3.10 | Resolve the remote upgrade path: delete the dead endpoint or build the privilege boundary (ADR) | **LC-3**, **NP-11**, **LC-10** |
| 3.11 | An ALSA capture-backend check that fails; record and re-check tool paths | **LC-4** |
| 3.12 | Install lifecycle in CI: second run, systemd host, N-1 → N upgrade, model resume | **LC-5** |
| 3.13 | Verify the pre-migration backup; widen its trigger | **LC-12** |
| 3.14 | Disaster recovery: config in the automatic backup, a written procedure, a tested restore | **LC-13**, **PS-15** |
| 3.15 | `quickstart.sh`: stable device id, pinned tag, a `COMPOSE_FILE` that keeps the overlay | **LC-9** |
| 3.16 | Docker: digest pinning, a healthcheck that can see failure, a port derived from config, a memory limit that matches the unit, a doctor before `exec` | **LC-11** |
| 3.17 | The advisory on an unstable ALSA card index, and card-identity drift detection | **AU-1**, **S-13** |
| 3.18 | Per-source restart and a jobs endpoint | **O-16** |
| 3.19 | `FULL_DISK=keep` | **NP-2** |
| 3.20 | Uninstall: one implementation, config removal inside the guard | **LC-14** |
| 3.21 | `verify_sha256` in the container must not pass when it cannot check | **LC-15** |

### Stage 4 — correctness in the signal path

| # | Item | Finding |
|---|---|---|
| 4.1 | Windows that cross the segment boundary; no classification of the zero-padded tail | **S-1** |
| 4.2 | Make the privacy threshold bind | **S-5** |
| 4.3 | Orphan-clip reconciliation | **S-14** |
| 4.4 | eBird links built from the species code the label file already carries | **NP-1** |
| 4.5 | Taxonomy aliases at the geomodel↔classifier join, and a coverage report | G-15, **S-6** |
| 4.6 | Separate the persistence interval from the notification cooldown | **S-10** |
| 4.7 | `inference_threads` configuration | **S-8** |
| 4.8 | Honour `bit_depth`, or remove it | **S-12** |
| 4.9 | The daylight filter in both directions, resolved through the taxonomy | **S-16** |
| 4.10 | An anti-aliasing filter before decimation | **S-7** |
| 4.11 | Rename the clip when a detection is relabelled | **NP-8** |
| 4.12 | Unicode spectrogram labels | **NP-3** |
| 4.13 | An INT8 ARM model path, with the accuracy delta measured before it is defaulted | **S-9** |

### Stage 5 — the deployment surface, and reaching a broken station

| # | Item | Finding |
|---|---|---|
| 5.1 | Private mode and public-access carve-outs | **O-4** |
| 5.2 | A login throttle and lockout | **O-6** |
| 5.3 | One predicate for "is the admin panel protected" | **O-14** |
| 5.4 | A native aarch64 test job | **ARM-1** |
| 5.5 | Certificate renewal on a timer, and refusal to mint with an implausible clock | **NT-2**, **NT-3** |
| 5.6 | One clock floor constant | **NT-5** |
| 5.7 | `tzdata` in the image, `TZ` documented, a zoneinfo staleness check | **NT-6** |
| 5.8 | Move blocking network calls off the startup path | **NT-13** |
| 5.9 | A resolver cache with negative caching; shared HTTP clients | **NT-14**, **PR-17** |
| 5.10 | A negative cache for species images | **NT-15** |
| 5.11 | Rollback after a bad update; a smoke test that runs `--doctor` | **NT-12** |
| 5.12 | Cellular/CGNAT documentation, then a safe-mode boot | **NT-17** |
| 5.13 | Correct the systemd documentation and gate it against the unit | **NT-8** |
| 5.14 | Mutating endpoints under `/api/v2` with bearer auth | **O-1** |
| 5.15 | Configurable frame ancestors | **O-5** |
| 5.16 | OpenAPI path-set equality | **O-8** |
| 5.17 | HSTS and COOP, correctly conditioned | **O-10** |
| 5.18 | An SSRF guard that still allows RFC1918 | **O-11** |
| 5.19 | Hot-reloadable proxy and rate-limit settings | **O-12** |
| 5.20 | Metrics off the public port | **O-15** |
| 5.21 | Support-bundle audio inventory | **NP-6** |
| 5.22 | Weekly report and alert-rule schedules on one clock | **NT-10** |
| 5.23 | Verified install, pinned by release | **LC-16** |

### Stage 6 — resource ceilings that are true

| # | Item | Finding |
|---|---|---|
| 6.1 | Size the tmpfs from `MemoryMax`, and assert the relation | **PR-3**, **PS-18** |
| 6.2 | Measure real RSS with the model loaded; correct the three "mmap'd" comments | **PR-4** |
| 6.3 | A dedicated system user, or a raised `LimitNPROC` and a capped blocking pool | **PR-6** |
| 6.4 | Bump the heartbeat inside the backlog loop | **PR-11** |
| 6.5 | Backpressure and thermal policy, with a queue gauge | **PR-2** |
| 6.6 | A soak that can see a leak, and one that runs capture + inference + web together | **PR-13** |
| 6.7 | Byte-budget the analytics fragment cache | **PR-16** |
| 6.8 | Remove or justify `OOMScoreAdjust=200` | **PR-15** |
| 6.9 | Bound the startup read | **PS-17** |
| 6.10 | Batch the insert path; document the real card-wear budget | **PS-4** |
| 6.11 | Verify WAL mode took, on the connection that matters | **PS-11** |
| 6.12 | Separate "could not verify" from "verified corrupt" | **PS-10** |
| 6.13 | A DuckDB probe that reads a data block | **PS-14** |
| 6.14 | Per-clip content hashes and a sampling verifier | **PS-13** |
| 6.15 | A geomodel threshold preview | **NP-4** |

### Stage 7 — carried forward from `FEATURE_GAP_ANALYSIS.md`

Its Tier 2 and Tier 3 are unchanged by this pass except where noted above
(G-30 narrowed to `rsync` only; G-15 raised; the `internal/` paths corrected).
They are not repeated here. Three items in its Tier 2 are subsumed:
**G-29** by 2.9, **G-28** by 2.2, and **G-32** by 3.18 and 2.19.

### Deliberately not planned

| Item | Reason |
|---|---|
| **O-13** UI internationalisation | Retrofitting into ~120 `format!`-based render functions is enormous, and half-done i18n is worse than none. The `lang` attribute and locale-aware formatting are worth doing; the rest is a programme, and `docs/design/tier-c-proposals.md` already records the decision. |
| **NP-10** clock/timezone from the UI | Needs `NOPASSWD: ALL`. The real answer is **NT-1**, **NT-6** and **OB-14**. |
| **S-15** native Opus encoding | Real, small, and not worth a codec dependency yet. |
| **S-11** continuous sound-level series | A deliberate duty-cycle decision, recorded so it is not re-opened. |
| Adminer, file manager, web terminal, Sentry, MySQL, FTP, Google Drive | Unchanged from `FEATURE_GAP_ANALYSIS.md`. Upstream's own comment calls its browser terminal a security risk; on a device whose admin panel ships open by default (**O-4**) it is remote code execution behind a checkbox. |

---

## 5. What this pass did not cover

Stated so the next one starts from the truth rather than from an impression of
completeness.

* **No Raspberry Pi, no ARM execution, no real microphone, no modem, no
  systemd, no Docker daemon, no SD card.** Every timing here is x86_64 in a
  container with a warm page cache; the Pi numbers would be worse and this pass
  did not measure by how much. **ARM-1** is the item that closes the ARM half.
* **No multi-day run.** Nothing here ran capture, inference and the web server
  together for a week. Descriptor drift, DuckDB growth under continuous sync,
  SD-card write amplification over months and thermal behaviour in a sealed box
  remain unmeasured — **PS-4** measures the write amplification *per detection*
  arithmetically, which is not the same thing.
* **No real power cut.** **NT-1**, **NT-4** and **NT-9** are each reasoned from
  an isolated probe plus the code, not from a capture → inference → insert
  reproduction across a real `date -s` or a DST boundary.
* ~~**The alert paths were not exercised on the wire.**~~ **No longer true, and
  the correction is the point of this bullet.** **OB-8** is now driven against
  a stub broker that *decodes* CONNECT and PUBLISH — which is how the
  positional §3.1.3 payload trap was caught, since a will written after the
  username is a well-formed packet that publishes the station's password.
  **OB-5** and **OB-13** deliver to a real loopback HTTP destination, which is
  how **NL-1** surfaced: a `queued` row was asserted and the schema refused it.
  Still unexercised: a real Apprise *API server*, a real Healthchecks.io
  endpoint, and TLS to a real broker (the TLS half has its own loopback
  coverage in `mqtt_over_tls.rs` but not against a third-party broker).
* **No web server under concurrent load**, and none of it against a database
  larger than the fixtures.
* **Accessibility was not compared** against upstream; determining which is
  ahead needs rendering both, which was not done.
* **The DuckDB extension install's own retry budget was not measured.** The
  load-bearing claim in **NT-13** is only that this repository sets none and the
  call is on the startup path.

---

## 6. Handoff

Written for whoever picks this up next, including a later session of the same
author. Everything below is a statement about the branch as it stands, not a
plan.

### Where the numbers come from

`cargo test --workspace` on x86_64 in a container: **3 578 passed, 0 failed,
107 suites**. `cargo fmt --check --all` and
`cargo clippy --workspace --all-targets -- -D warnings` both exit 0. The same
command at the branch point `f33eb9e` reports 3 570 in 106 suites, so the
difference is this pass's own gates and nothing else. (This block read "3 567,
106 suites" until both were taken again; that figure predated the last commit
of the previous pass.)

Two gates in the template's list are **not** verified here, and should not be
claimed as local results:

* `cargo deny check` — `cargo-deny` is still not installed in this
  environment, and neither are `cargo-audit`, `cargo-machete`,
  `cargo-llvm-cov` or `cross`. The supply-chain question is nevertheless
  **answered, by CI rather than here**: the `Supply chain` workflow run on the
  merge commit `f33eb9e`
  ([33778570446](https://github.com/tomtom215/BirdNet-Behavior/actions/runs/33778570446))
  reports all six jobs green — `cargo-deny`, `cargo-audit`, `cargo-machete`,
  `Spelling (typos)`, `shellcheck (bootstrap scripts)` and `installer unit
  tests`. Read that from the run, not from this sentence, before relying on
  it; and note it is a statement about `f33eb9e`, not about whatever is in
  your working tree.
* `birdnet-behavior --doctor` exits **1**, not 0, in this container: 8 passed,
  9 warnings, 0 errors. Exit 1 means "worst severity is Warn"
  (`doctor/render.rs::summarise`), and the warnings are all
  unconfigured-environment ones — no admin password, no audio source, no
  HTTPS, no offsite backup, under 1 GiB free. Reaching 0 needs a configured
  station, which this container is not. **Still true**, and still not to be
  ticked off without one.

### What to do first

1. **2.8 (`OB-16`) — re-notify open episodes on a widening schedule.** The
   posture after 2.2 is still *one delivered push, ever, per fault*. For a
   fault lasting four months that is the wrong side of the trade.
2. Then the rest of Stage 2 in any order; nothing in it blocks anything else.

**2.16 (`OB-9`) is done** and is no longer at the head of this list: the push
test now locks the notifier the alert loops hold and calls
`send_operational_alert` on it, and the button is live for any resolved
destination. Its remaining half — email and MQTT have no test of any kind — is
item **2.20**, and is a smaller thing than the one that was fixed.

### Two claims in this document that were found to be wrong

Recorded because a corrections log is worth more than a clean one:

* **`O-3` said three `LogBroadcaster::new()` calls were "three distinct
  channels anyway".** They are three *alternative* constructors — `AppState::new`,
  `new_with_analytics`, `from_connection` — and one run builds one `AppState`
  (`src/app.rs:184`). There was one channel and nothing wrote to it. The count
  was never the defect and no deduplication was needed.
* **`OB-14` treated its two NTP signals as peers.**
  `/run/systemd/timesync/synchronized` is created on first sync and never
  removed if sync is later lost, so it answers "synced at some point since
  boot" — which is precisely the question this check must not ask, given that
  the failure it exists for is a Pi whose NTP has been unreachable for months.
  It is a fallback, not an authority.

### One lesson worth carrying, from 2.5

The first mutation applied to the clock work — deleting `check_clock` from
`evaluate`, which was the shipped state — **killed nothing**. All 31 tests
passed. Every gate exercised the policy function; none checked that anything
called it, and a check dropped in a refactor produces no failure, no warning
and no condition, which is what a healthy station produces.

`evaluate` now runs a named `CHECKS` table that a gate reads against the six
conditions the module doc promises. The audit-log work reuses the same shape: a
source scanner reads every action literal and compares it against a documented
list. **A set expressed only as scattered call sites cannot be checked**, so
write it down once.
