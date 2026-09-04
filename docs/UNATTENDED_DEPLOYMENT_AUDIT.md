# What a year alone in a field would find

**Date:** 2026-09-03 · **Branch:** `claude/birdnet-deployment-gaps-e4ebxd` ·
**Base:** `35acd9e` (merge of PR #232, `v0.15.0` + the parity branch)

**Reconciled 2026-09-04** against `ee795ed` (merge of PR #234) on branch
`claude/birdnet-audit-reconciliation-jtgz6k`. Every one of the 134 rows in §3
was re-checked against the source rather than against this document; §2 gained
the corrections that pass found, §3.12 the findings it turned up that no row
covered, and §6 a fresh handoff. Rows whose status changed say so in place.
Nothing was carried forward: the counts in this header and in §6 were re-derived
by command, and two of them were wrong.

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

**134 findings**, counted from the register in §3: **5 P0**, 48 P1, 59 P2,
20 P3, and 2 recorded as deliberate divergences rather than gaps. (This said
"133 … 47 P1" until the reconciliation pass counted the rows instead of
trusting the sentence: `^\| \*\*([A-Z]+-[0-9]+)\*\* \| (sev) \|` over §3
gives 134 distinct ids, no duplicates, summing 5 + 48 + 59 + 20 + 2. Per
section: PS 19, PR 17, NT 18, LC 16, OB+NL 17, NP 13, S 16, O 16, ARM+AU 2.)
The reconciliation pass adds **35** rows in §3.12 (`RC-*`) and **86** in §3.13
(`ON-*` onboarding, `R-*` research credibility, `AD-*` adversity, `OP-*`
operability, `UX-*` interface and accessibility, `FR-*`/`UP-*`/`WE-*` against the
two references), for **255** rows in the register — counted by first id per row,
which is the right measure because a few rows deliberately group several ids
(`WE-1 … WE-5`, and three of the accessibility rows). Per prefix: PS 19, PR 17,
NT 18, LC 16, OB 16, NL 1, NP 13, S 16, O 16, ARM 1, AU 1, RC 35, ON 20, R 11,
AD 9, OP 16, UX 15, FR 6, UP 8, WE 1. Ten are fixed on the reconciliation
branch, each with a gate observed failing against the code it guards. The five P0s
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
`/api/v2` surface was read-only. A mechanical check — every `post(`/`put(`/
`delete(`/`patch(` across all fourteen modules mounted under `/api/v2` — returned
nothing. **Closed**: seven bearer-authenticated write endpoints now exist — the
four `/detections/*` writes, `POST /api/v2/detections/batch`,
`PUT /api/v2/settings` and `POST /api/v2/control/restart` — along with
`GET /api/v2/settings` behind the same token (item 5.14 below). Before them every mutation in the product was
an HTMX form post returning HTML, so no automation, Home Assistant action or
scripted admin was possible without scraping fragments and forging an `Origin`
header.

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

### 2.1 Found by the reconciliation pass, 2026-09-04

Including three claims in **this** document, and one of its own corrections that
was recorded and never applied.

| Document | Claim | Correction |
|---|---|---|
| This document, header | "133 findings … 5 P0, 47 P1, 59 P2, 20 P3" | 134 rows, 48 P1. Counted from §3 rather than carried; the other three figures hold. |
| This document, §3.11 | "61 `Relaxed` uses were reviewed" | True at `35acd9e` and stated without a commit, so it reads as current. 73 at `f33eb9e`, **81** at `ee795ed`, and zero `Acquire`/`Release`/`AcqRel` anywhere. See **RC-9**. |
| This document, **PS-18** | the 512 MiB cap "can never bind" because the installer mounts at `size=256M` | The mount is not in the service's namespace — `PrivateTmp=yes` gives it a fresh `/tmp` — and `60-dirs.sh:31` skips creating it entirely when `/tmp` is already tmpfs. The cap is the *only* ceiling, not an unreachable one, which makes **PR-3** worse rather than better. The PS-18 row now carries this. |
| This document, §2 (above) | `security.rs:187`'s "No HSTS: the binary serves plain HTTP" is stale | Still stale, and still there. The same module doc's *opening* line — "the web UI authenticates with HTTP Basic Auth and keeps no cookies or sessions" — is also false and is load-bearing, because the CSRF rationale is built on it. See **RC-5**. Of §2's four code-comment corrections, three were applied (`metrics.rs:180`, `offsite/s3.rs:47`, `src/maintenance.rs`); this one was not. |
| `crates/birdnet-web/src/routes/pages/dawn_chorus.rs` | the inline predicate is the "same predicate the view applies … and `dawn_chorus_excludes_rejected_detections` holds the two in step" | Neither half was true: the provenance clause was missing and that test only covered the verdict. Fixed, with a gate that holds the two against each other on the same rows (**RC-4**). |
| `src/doctor.rs:24` | the check submodules are "(`config`, `database`, `paths`, `audio`, `model`, `environment`, `disk`, `watchdog`)" | Fourteen exist; the doc knows eight. Missing: `analytics`, `clock`, `fix`, `offsite`, `tls` — including the two a field operator most needs. See **RC-8**. |
| `.env.example:99` | "Web UI authentication (HTTP Basic)" | A session cookie. The same mislabel was corrected in five documents on this branch; this is the file every Docker operator copies. See **RC-12**. |
| `crates/birdnet-timeseries/src/executor/mod.rs` | `TimeSeriesDb::new` "ensures the `detections_ts` view is present" | It *replaced* it, dropping a rule the other crate had installed. Now checks rather than creates (**RC-2**). |

One correction runs the other way, and is recorded because a reconciliation that
only ever downgrades is not being honest about its own error rate:

| Document | Claim | Correction |
|---|---|---|
| **NP-13** | the pitch shift works on the live stream and not on saved clips | **Confirmed, and upgraded from READ to VERIFIED.** An intermediate reading of this pass had it as stale; that was wrong. `routes/livestream.rs:79` carries `freq_shift_hz` through `freq_shift_filter` (`:126`) as ffmpeg `asetrate` + `aresample`, settable from the admin form; the clip player's only pitch-adjacent control is `routes/pages/audio_player.rs:247` `audio.playbackRate`, which is not the same thing and has no `preservesPitch`. |


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
| **PS-5** | P1 | READ | The daily integrity check detects corruption, logs one `error!`, and the daemon **keeps writing to the corrupt file** until someone reboots it. The "never write to a corrupt database" policy exists only at startup. **[FIXED — the second option, narrowed]** The remedy's second branch, with one deliberate narrowing: **not** `PRAGMA query_only`. Login sessions are rows in this database, so a read-only writer would lock the operator out of the admin UI that exists to tell them what is wrong, and would stop the notification log recording the alerts about the corruption — self-defeating on exactly the station this audit is about. The line is drawn at the writes that *record a detection event* (`insert_detection`, `insert_quarantine`, `outbound_queue::enqueue`), through `AppState::with_ingest_db`; settings, sessions, the audit log, the notification log and the maintenance-run record that makes the health endpoint go red all keep working. `/api/v2/health` reports `"detection_writes": "halted"` and answers 503. The latch is one-way — a file does not heal itself, and a flapping check would flap the station — so recovery is a restart, where the startup path restores from backup or quarantines. One thing the finding did not reach: `backup_database` refuses to snapshot a corrupt source, so during all of that the backup ring had *also* stopped producing restore points, which is why every hour of it made recovery worse rather than better. | Done; see Stages 0 and 1 landed. The first branch of the remedy — quarantine and restore *in place*, at runtime — is not done and is now unblocked by this: stopping the ingest writer is its prerequisite. |
| **PS-6** | P1 | READ | A quarantined `birds.db.corrupt.<ts>` — total history loss — is matched by **no** doctor scan (`doctor/analytics.rs:130` matches `.duckdb.corrupt.` only, and its test asserts the SQLite name is *not* matched), no `station_health` condition, and no prune. It sits on the card for ever. **[FIXED IN PART — "Alert on a backup that fails, not only on one that stops"]** The doctor scan now matches `.db.corrupt.` as well as `.duckdb.corrupt.` (excluding `-wal`/`-shm` sidecars), and `check_quarantined_stores` raises a condition whose title distinguishes a lost detection history from a rebuilt analytics store. **The prune is still not done**: a quarantined file still sits on the card for ever, which on a 32 GB card is the difference between one bad week and a full disk. That half is item 2.11. | Remaining: prune quarantined stores on a retention schedule. |
| **PS-7** | P1 | READ | `sync_all` appears **twice in the whole workspace**, neither in the audio path. Clips and segments are written non-atomically under their final names, so a power cut leaves truncated files the database points at for ever; and because both retention passes are database-driven, a clip whose row was lost is never deleted except by the 95 %-full purge. | Write to `.part` + `rename` + `sync_all`, and add an orphan-clip reconciliation pass (**S-14**). Note the exemplar this row used to cite: `docker/entrypoint.sh:239` is `mv "${tmpfile}" "${dest}"`, a rename with no sync. Copied verbatim it buys atomicity of the *name* and not of the bytes, which is the half that matters after a power cut. |
| **PS-8** | P1 | READ | `--doctor`'s only disk check and its "Recordings directory" check both read `--watch-dir` first, which the shipped unit **always** sets to the tmpfs — so the preflight measures a RAM disk while `/api/v2/system/disk` correctly measures the card. | Check the data partition explicitly, and report both. |
| **PS-9** | P1 | READ/VERIFIED | Nothing probes writability at runtime. On a read-only remount — what the kernel does after repeated I/O errors — `/api/v2/health` still answers `healthy` (a read-only `SELECT 1` succeeds; the integrity verdict freezes because *recording* it is a write) while every detection is classified and discarded. | A periodic write probe on the data partition, feeding a health condition and a metric. |
| **PS-10** | P2 | READ | `check_and_recover` sends `Err` ("could not verify") down the same branch as `Ok(false)` ("verified corrupt"), and the live database is then deleted — one call deeper than this row said: `check_and_recover` itself deletes nothing, `restore_from_backup` does, at `crates/birdnet-db/src/resilience.rs:444` `std::fs::remove_file(db_path)?`. The conclusion holds, but a fix applied to `check_and_recover` alone would not be enough. Could **not** be provoked with a held write lock — WAL readers do not block — so latent rather than routine, and recorded as disproven. | Separate the two verdicts; refuse to destroy on "unknown". |
| **PS-11** | P2 | READ | `enforce_wal_mode` — the only code that checks the WAL pragma actually took — is called from nowhere but `restore_from_backup`. The live connection uses `execute_batch` and silently accepts `delete` mode. | Call it on the live connection and fail loudly. |
| **PS-12** | P2 | READ | No byte reserve for the database: one 95 % percentage on the recordings directory, with the DB, its WAL, the backup ring, quarantines and `VACUUM` scratch all outside every budget. `out_of_space.rs` explicitly does not cover WAL growth. | A reserved-bytes floor enforced by the purger, sized from the DB + WAL + one backup. |
| **PS-13** | P2 | READ | Zero checksums anywhere in `birdnet-core`/`birdnet-db`. `integrity_check` verifies B-tree structure only, so value-level bit rot is invisible and is copied into all five backups. | Store a content hash per clip in the RIFF INFO block; a periodic sampling verifier. |
| **PS-14** | P2 | READ | The DuckDB damaged-block probe is `SELECT COUNT(*)` — the one query answered from row-group metadata without touching data blocks — and its only test replaces the entire file, so the comment's claim has never been exercised. | Probe with an aggregate over a real column; gate with a byte-level corruption of one block. |
| **PS-15** | P2 | READ/VERIFIED | Effective backup retention is **5 weekly snapshots (35 days)**, not the 14 the constant, the startup log and `resilience.rs:576`'s comment all say (the line has moved since, and it is inside `check_and_recover` — "away the entire point of keeping a ring of fourteen" — so it is *recovery* code telling its reader there are fourteen restore points when `MAX_BACKUP_FILES` at `:12` keeps five). They live on the same card, there is no `--restore-db`, and nothing round-trips a restore through the real recovery path. | Correct the constant or the cadence so they agree; add `--restore-db`; extend `tests/offsite_backup_round_trip.rs` to a full wipe-and-restore. |
| **PS-16** | P2 | READ | Half of this is done and the row did not say so. `evaluate` now implements **six** conditions, not four, from a named table — `src/integrations/station_health.rs:222 const CHECKS: [(&str, Check); 6]` — including `("quarantined-stores", …)` and `("clock", …)`, and `every_documented_condition_is_actually_checked` (`:817`) now stops the module doc and the table drifting apart at all. What survives of the finding is only its second clause: **nothing reads the `detection_write_failed` counter**, which reaches the Prometheus exposition (`crates/birdnet-web/src/metrics.rs:783`) and no health condition, alert or dashboard. | Alert on `detection_write_failed`; see **OB-7** and **PR-12**. |
| **PS-17** | P2 | VERIFIED | Every boot reads the whole database twice plus a full aggregate **before binding** — 4.67 s + 1.46 s + 0.34 s on 262 MB on x86 NVMe, minutes on a Pi at 1.8 GB — paid on each of several brownouts a month. | Move the warm-up behind the listener; bound it. |
| **PS-18** | P3 | READ | **The stated mechanism is wrong, in the direction that makes the exposure larger.** `STREAM_MAX_MB` does default to 512 against an installer mount of `size=256M`, and the two disagreeing is still a real config defect — but the 256 MiB mount is not what binds at runtime, and the cap is not unreachable. `installer/lib/65-service.sh:161` sets `PrivateTmp=yes`, which mounts a **fresh** tmpfs over `/tmp` inside the service's own namespace, so the installer's `/tmp/birdnet-stream` mount unit (`installer/lib/60-dirs.sh:44-51`) is not in that namespace at all; `65-service.sh:92-100` says as much in prose. Worse, `60-dirs.sh:31` skips creating the capped mount entirely on any host whose `/tmp` is already tmpfs. What actually binds is systemd's private-tmp default of about half of RAM, with `STREAM_MAX_MB=512` the only real ceiling — an unreclaimable 512 MiB charge inside `MemoryMax=1G`, which makes **PR-3** worse than PR-3 states. | Derive the default from the mount *and* fix the mount so it is in the service's namespace; assert the relation in `installer/test/`. Re-read **PR-3** with this in mind before sizing anything. |
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
| **LC-16** | P3 | READ | The documented install fetches an unversioned, unverified `install.sh` from `main`, while the release publishes a checksummed one nobody is pointed at. The `--version` flag does exist (`installer/lib/90-args.sh:27`); what the documented one-liner does not do is *use* it, so a re-run six months later installs a different release than the bench unit — quietly undoing the manual's own "test on a bench unit first" advice. | A verified-install stanza in the README and the manual; print the resolved release and confirm when interactive. |

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
| **OB-4** | P1 | VERIFIED | `/api/v2/health` returns `200 "healthy"` while its own body says `detection_daemon: "stopped"`; the status code is gated on SQLite alone. This is the endpoint the container healthcheck polls and the one every monitor gets pointed at. **[FIXED — the `?strict=1` sibling]** `routes/system.rs:223` returns 503 on a stopped daemon; gated by `tests/web_api.rs:145`, whose own comment records the pre-fix failure ("the strict request returned 200"). The default stays 200 so a quiet season does not restart the container. Reconciliation 2026-09-04: this row was still written as open. | Done for the endpoint. What is left is the *flag*, and it is **PR-5**'s second clause, not this one: `src/app.rs:439` is the only writer of the daemon `AtomicBool` in the workspace and no exit path clears it, so `?strict=1` reports a daemon that died as running. |
| **OB-5** | P1 | READ | **An operational alert can be dropped and then never re-sent.** All three alerting loops set the episode latch *before* calling `notify()`, `notify()` swallows the failure with one `warn!`, and the send may not be attempted at all because the shared circuit breaker skips it with a **`debug!`** that the default filter drops for `birdnet_integrations`. Sequence: uplink drops, three detection notifications fail, circuit opens; 24 h later the deadman fires, latches, and its send is skipped silently; the link returns; `alerted` stays true for the process lifetime. And `apprise.rs:473` `skip_counts()` — which returns exactly the two counters that would answer "how many alerts were dropped" — has **zero production callers**. **[FIXED — "Latch an alert episode on delivery, not on the attempt"]** The defect was worse than described here: `send_notification_with_image` returned **`Ok(())`** for `(delivered: 0, first_error: None)`, which is precisely the fully-skipped case, so the send itself reported success. Two of the four prescriptions were adopted as written; the other two were changed on evidence. Forcing a probe through an open circuit was **not** done — the breaker already admits one probe per open period, and a caller that now retries rides that schedule and lands as soon as the destination comes back, so forcing would only add traffic to a dead endpoint. Raising the per-send skip to `warn!` was **reverted for detections**: at a detection every twenty seconds that is ~4 000 lines a day, which is why it was `debug` to begin with; the `warn` moved to the *transition* (`Breaker::on_failure` now reports the period it just opened for) and is kept per-send only for operational alerts. | Done; see Stage 2 landed. |
| **OB-7** | P1 | READ | **A backup that fails every week never alerts**, because `mark_ran` is called unconditionally after `run_backup_and_vacuum` and the station-health check reads `last_run_unix`, ignoring the `ok` column entirely. It can only detect the maintenance loop having *stopped*. And a **failed integrity check pushes nothing at all** — it reddens a badge and 503s an endpoint, but sends no notification, though `station_health.rs:19-20` names it as one of the two things the module exists for. Offsite failure is invisible everywhere: no counter, no `maintenance_runs` row, no health field, no alert. | Give the backup a verdict and use `mark_ran_with`; branch on `last_run_result`; add the recorded integrity failure as its own condition; give offsite its own job key. |
| **OB-12** | P1 | READ | **[FIXED — `birdnet_files_analysed_total`]** `crates/birdnet-web/src/metrics.rs:652` exports it and `src/daemon/mod.rs:274` bumps it; landed as §4 item 2.1. Reconciliation 2026-09-04: this row was still written as open. The chunk-level counter the remedy also named is not done. Original finding follows. **No chunk or file throughput counter exists**, and the one latency histogram is observed *per stored detection*, not per analysed chunk — its own HELP says so. So a station where inference runs perfectly and returns nothing (wrong labels, wrong sample rate, a model swapped by a bad update) has flat, empty latency series **identical to a station where inference is not running at all**. The four production drop reasons all live downstream of a prediction the model actually made. | `birdnet_files_analysed_total{source}` at the point the correlation id is minted (~5 760/day/source at the default segment length), plus `birdnet_chunks_analysed_total`. A flat counter with `audio_source_up == 1` is "capture writes, nothing analysed"; a rising counter with zero detections is "the model answers nothing" — the discrimination no surface can currently make. |
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
| **OB-16** | P2 | READ | Alert storms are genuinely well prevented — three-poll debounce, per-episode latch, recovery notices, a compile-time assertion that the debounce constant stays > 2. But **nothing ever re-notifies an open episode**: the only thing that re-arms one is a process restart. The posture is *one push, ever, per fault, over a channel never tested end to end (**OB-9**) that may drop it silently (**OB-5**)*. For a fault lasting four months that is the wrong side of the trade. **[FIXED — an open episode is said again on a widening schedule]** Taken as prescribed: 24 h, 72 h, then one a week, in `src/integrations/reminder.rs`, carrying "Still unresolved after N days" and the condition's *current* body rather than the one the episode opened with. All three loops re-announce — the deadman through a new `Transition::StillBroken` arm (the shipped code returned `Transition::None`, which is indistinguishable from a healthy station), station health through a pure `due_reminders`, acoustic health through `FaultWatch::reported` becoming a map of clocks rather than a `HashSet`. One thing the finding did not reach: a station that is **off** for a month comes back with several steps of the schedule behind it, and a counter advancing one step per call replays them at one per five-minute poll. `Reminders::due` skips every step the gap swallowed. | Done; see Stage 2 landed. |

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
| **NP-13** | P3 | READ/VERIFIED | The accessibility pitch shift works on the live stream and **not on saved clips** — the same argument that made N-2 a Tier-1 item, left unfinished. Re-verified 2026-09-04 and upgraded from READ: the live path has a real shift (`routes/livestream.rs:79 pub freq_shift_hz: i32`, applied by `freq_shift_filter` at `:126` as ffmpeg `asetrate` + `aresample`, clamped to `MAX_STREAM_SHIFT_HZ`, set from the admin form at `admin/settings/render/audio.rs:64`), while the clip player has exactly one pitch-adjacent control: `routes/pages/audio_player.rs:247` `function setSpeed(v) { audio.playbackRate = parseFloat(v); }`. That is not the same thing — `playbackRate` changes duration and pitch together, and there is no `preservesPitch`, no `detune` and no shift control anywhere in the player. | Apply the same shift in the clip player. Note it cannot be `playbackRate`: the clip has to keep its duration or the spectrogram beside it stops lining up. |

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
| **O-1** | P1 | VERIFIED | **[FIXED]** Seven bearer-authenticated write endpoints now exist — `POST /api/v2/detections/{review,lock,unlock,delete}`, `POST /api/v2/detections/batch`, `PUT /api/v2/settings` and `POST /api/v2/control/restart` — plus `GET /api/v2/settings` behind the same token, in their own router, so `public_routes()` stays read-only, and behind `api_token::require_bearer`, so they never inherit the admin middleware's open-when-no-password bypass: a station with no `BNB_API_TOKEN` answers **404** on all seven. That is the opposite default from `CADDY_PWD`, deliberately. The CSRF guard skips exactly these paths, because a cross-site form cannot set an `Authorization` header. The settings read applies the support bundle's own redaction rules, moved into `birdnet-core` so there is one copy rather than two; the settings write reuses the admin page's `build_settings_items`, so the normalisation and the only-write-what-changed rule are the page's; the restart shares its systemd detection with the admin button. The batch endpoint applies one of the four detection operations to up to 500 keys, and is deliberately **not** a transaction: each key takes the same paired `AppState` write the single endpoint takes, because a shared `with_db` transaction would reach past the SQLite/DuckDB pairing that `tests/analytics_divergence.rs` exists to protect — two new gates there cover this route, and restoring the shortcut leaves the analytics copy holding rows SQLite no longer has. All eight of the remedy's endpoints are now in place. The original finding: **The `/api/v2` surface is 100 % read-only.** No mutating route method in any of the fourteen modules mounted under it. Every mutation is an HTMX form post returning HTML behind a same-origin check — trivially satisfied by any script that sets a matching `Origin`, and therefore not a contract anyone can build on. Upstream has 54 mutating routes. Consequences: no supported automation; Home Assistant and Node-RED can read but never act; and our own front end is the only client, so a fragment-markup change silently breaks whatever automation exists in the wild. | Port the ~8 with operational weight, reusing the handlers already behind the HTMX routes: review, lock, delete, batch, `GET` and `PUT /settings`, `POST /control/restart`. Bearer auth, and the CSRF guard must **skip** bearer-authenticated requests — a header token is not attachable by a cross-site form, which is the entire premise of the check. |
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
* **Atomics**: 61 `Relaxed` uses **at `35acd9e`** were reviewed for the
  release/acquire pattern that x86's TSO hides and ARM's weak ordering exposes.
  That is a statement about a commit, not about the tree: the same count is 73
  at `f33eb9e` and **81** at `ee795ed`, so a quarter of them have never been
  looked at, and `Acquire`, `Release` and `AcqRel` appear nowhere in the
  workspace at all. See **RC-9**; the only thing that could *observe* a mistake
  here is **ARM-1**. The shutdown flags and
  liveness counters are correct with `Relaxed`. One benign ordering hazard
  exists — `clock.rs`'s two independent atomics let a reader see a fresh
  `COMPUTED_AT` with a stale `OFFSET_SECS` — which is P3 and folded into
  **NT-9**.
* **Captive portals** are mostly handled by construction: BirdWeather parses
  JSON, image downloads check `Content-Type: image/`, and the chat routes check
  the response body, so an HTML portal page is an error rather than a success.

### 3.12 Found by the reconciliation pass (2026-09-04)

Re-verifying the 134 rows above against `ee795ed` turned these up. None is
covered by an existing row. Severities use the same scale.

**Fixed on the reconciliation branch**, each with a gate observed failing
against the code it guards and the failure text in its commit message:

| ID | Sev | How | Finding | Status |
|---|---|---|---|---|
| **RC-1** | **P1** | VERIFIED | **`full_integrity_check` had no SQLite-header guard, so PS-2's fix never covered the running station.** PS-2 taught `check_integrity` to require the sixteen-byte magic; the guard went on that function alone. `full_integrity_check` is the one the runtime paths call — the daily scheduled check (`src/maintenance.rs:627`) whose verdict drives the PS-5 ingest halt, `--check-db` (`src/helpers/db.rs:133`), and `--doctor` (`src/doctor/database.rs:48`). Probed against SQLite on a zero-length file opened read-only: `PRAGMA integrity_check` → `ok`, `PRAGMA quick_check` → `ok`. So a `birds.db` truncated to zero *while the station was running* was reported healthy every day for the rest of the year, the ingest halt never tripped, and an operator who ran `--check-db` — what the failure message tells them to do — was told the database was fine. Only the next reboot could notice. The gate file `an_empty_database_is_not_a_healthy_one.rs` asserted on `check_integrity` at all five of its call sites and never touched the other function, which is why the hole was invisible. | **[FIXED]** — same guard, plus `every_public_integrity_entry_point_consults_the_header`, a source scan by brace depth so a third entry point inherits the rule instead of reopening the hole. |
| **RC-2** | **P1** | VERIFIED | **Opening any time-series page re-admitted another station's imported history into every behavioural analytic.** `birdnet-behavioral` and `birdnet-timeseries` create `detections_ts` with `CREATE OR REPLACE` on the *same* DuckDB connection. Migration 34 gave that view a second rule — `AND import_batch_id IS NULL` when `analytics_exclude_imports` is set — built by `detections_ts_view_sql(true)`, which has no counterpart constant in `birdnet-timeseries` because the flag lives in SQLite. `TimeSeriesDb::new` overwrote it on every construction, so sessionize, retention, funnel, next-species, co-occurrence and phenology silently began counting a foreign site's records until a later sync happened to reinstall the right definition. `tests/analytics_view_ownership.rs` could not see it: it holds `CREATE_DETECTIONS_TS_VIEW` equal to `ENSURE_TS_VIEW`, and those two *are* equal. Measured: 5 rows where 3 are correct, with the `AND (import_batch_id IS NULL)` clause visibly stripped from the catalog. | **[FIXED]** — the view has one owner. `TimeSeriesDb::new` checks the view exists and returns its existing `MissingView` error instead of replacing it. `constructing_the_executor_does_not_redefine_the_view` states the rule rather than one of its consequences, so a third clause added later inherits the protection. |
| **RC-3** | **P1** | VERIFIED | **The species list ranked a bird the station never heard as its commonest.** `species_summary` (migration 30) is a trigger-maintained rollup keyed `(Com_Name, Sci_Name, hour)` whose triggers filter on `review_verdict` alone. It could not learn migration 34's provenance rule, because that rule depends on a setting the operator can flip and the key has no provenance dimension. Probed on two of the station's own detections and three imported, exclusion on: `detections_analytic` reported 1 species / 2 rows, `species_count` reported 2, and `top_species` returned the imported *Parus major* first at 3 detections. `species_summary(conn, name)` — the per-species detail page — reads the view directly and was right throughout, so the list and the detail page disagreed about the same species. `tests/provenance_filter_two_stores.rs` compares the SQLite view against the DuckDB view; the rollup is a third implementation of the rule and was in neither comparison. | **[FIXED — reader side]** the five rollup readers take their source from `summary_source`, which substitutes an equivalent aggregate over `detections_analytic` only when the station both has imported rows and excludes them. **The lasting fix is not done**: a provenance dimension in the rollup's key, so both answers come from the rollup and no station pays the scan. That is a schema migration plus a rewrite of three triggers — item 3.22 below. |
| **RC-4** | P2 | VERIFIED | **The dawn chorus copied half the view's predicate and its comment claimed it had copied all of it.** The chorus is the one aggregate that cannot read `detections_analytic` (`INDEXED BY` is invalid on a view, and dropping the hint costs 60×), so it spells the predicate out inline. Migration 34's second clause was never copied, while the comment beside `CHORUS_SQL` went on saying "same predicate the view applies … and `dawn_chorus_excludes_rejected_detections` holds the two in step" — neither half true. The chorus was the single surface still counting an excluded site's records, and, being a chart, the most likely to be believed. | **[FIXED]** — both clauses inline, and `the_inline_predicate_and_the_view_admit_the_same_rows` now counts the same rows through both and requires them equal across all three settings, so a fourth clause fails here rather than needing to be remembered. The `INDEXED BY` plan gate stays green with the subquery added. |

**Recorded, not fixed.** Each is either smaller than a commit of its own or
larger than one reviewable change; the reason is in the row.

| ID | Sev | How | Finding | Remedy, and why not now |
|---|---|---|---|---|
| **RC-5** | P2 | VERIFIED | **`security.rs`'s module doc states the opposite of the code, and the whole CSRF rationale rests on it.** `crates/birdnet-web/src/security.rs:3`: *"The web UI authenticates with HTTP Basic Auth and keeps no cookies or sessions, so the classic CSRF vector … is mitigated here by a same-origin check … rather than by per-form synchroniser tokens. This is the OWASP-recommended defence for an app without a session token to bind to."* There is a session cookie: `session.rs:57 pub const COOKIE_NAME: &str = "bnb-session";`, issued at `:318` with `HttpOnly; SameSite=Lax`, and `routes/auth_pages.rs:13` documents the `Set-Cookie`. The live defence is `SameSite=Lax` **plus** the same-origin check; the doc tells a reader neither is load-bearing. §2 of this document already recorded the same file's stale HSTS line and it was never applied — this is the second stale claim in one module doc. | Rewrite the module doc to name both mitigations. Not done here only because it belongs with **RC-6**, which is the gate that makes the rewrite checkable. |
| **RC-6** | P2 | VERIFIED | **Nothing asserts the session cookie's `HttpOnly` or `SameSite` attributes.** `grep -rn "SameSite" crates/ src/ tests/ --include=*.rs` matches only the two `format!` templates inside `session.rs` itself. The three cookie tests there assert the cookie *name*, the `Secure` conditioning and `Max-Age` — not the two attributes that carry the CSRF and XSS-exfiltration defence. Deleting `SameSite=Lax` from `build_set_cookie` passes the entire suite. | Two assertions in `session.rs`'s existing tests, and the counterpart that a cleared cookie carries them too. Small; grouped with RC-5 so the doc and the gate land together. |
| **RC-7** | P2 | VERIFIED | **`--doctor` has the exact hole item 2.5 was written to close.** `src/doctor.rs:180 fn collect()` is twenty-one scattered `checks.push(…)` / `checks.extend(…)` call sites (`:186-211`). There is no named table and no gate reads them, so a check dropped in a refactor produces no failure, no warning and no missing output — which is precisely what `station_health`'s `const CHECKS: [(&str, Check); 6]` (`src/integrations/station_health.rs:222`) and its gate at `:817` were introduced to prevent. §6's own closing lesson says a set expressed only as scattered call sites cannot be checked; the doctor is that set. | A named table of check families that a source-scanning gate reads, in `station_health`'s shape. Not done here because it is a refactor of the doctor's entry point and deserves its own review. |
| **RC-8** | P3 | VERIFIED | **`src/doctor.rs`'s module doc knows eight of its fourteen check submodules.** `:24` lists "(`config`, `database`, `paths`, `audio`, `model`, `environment`, `disk`, `watchdog`)". The `mod` declarations at `:34-47` are `analytics`, `audio`, `clock`, `config`, `database`, `disk`, `environment`, `fix`, `model`, `offsite`, `paths`, `render`, `tls`, `watchdog`. Five check families the module doc does not know exist, including `clock` and `tls`, which are the two a field operator most needs. | One line, but it should land with RC-7's table so the doc is generated from the set rather than restating it. |
| **RC-9** | P2 | VERIFIED | **§3.11's atomics claim has decayed by a third and nothing holds it.** That section says "61 `Relaxed` uses were reviewed for the release/acquire pattern that x86's TSO hides and ARM's weak ordering exposes", with no commit qualifier, so it reads as current. Counted with one method across three commits (`git ls-tree` over each revision, filtered to `crates/` and `src/` `.rs` paths, then `git show REV:file` piped through `grep -o "Ordering::Relaxed"` and counted, summed — one method, three commits): `35acd9e` → **61**, exactly matching the claim; `f33eb9e` → **73**; `ee795ed` → **81**. Twenty uses added since the review. And `Ordering::Acquire`, `::Release` and `::AcqRel` appear **zero** times in `crates/`, `src/` and `tests/` — every happens-before edge here is `SeqCst` (31) or `Relaxed` (81), on a project whose primary target is ARM, and **ARM-1** records that no aarch64 test has ever executed, so nothing anywhere could observe a reordering bug. | The real detector is **ARM-1** / item 5.4, a native aarch64 test job. A cheap complement is a gate that pins the per-file `Relaxed` census so adding one to a new file forces the review rather than skipping it — a review-debt gate, not a correctness proof, and it should be labelled that way. §3.11's sentence should name the commit it was true at. |
| **RC-10** | P2 | VERIFIED | **Two backup-retention constants fight over the same directory.** `crates/birdnet-db/src/resilience.rs:12 const MAX_BACKUP_FILES: usize = 5;` is applied inline by `backup_database` (`:291`), while `src/maintenance.rs:62 const BACKUP_RETENTION: usize = 14;` prunes the same directory weekly. One `--backup-db`, or one press of "Create Backup Now", therefore cuts that database's ring from fourteen snapshots to five — discarding nine weeks of restore points as a side effect of taking a backup. This is the mechanism behind **PS-15**'s "effective retention is 5, not 14", stated as a cause rather than an observation. | One constant, or two that name their different jobs and a comment saying which wins. Sits with PS-15 and item 3.14. |
| **RC-11** | P2 | VERIFIED | **The compose healthcheck was not taught what the image's was.** `Dockerfile:355` derives the port it polls from the configured listener: `curl … "http://127.0.0.1:${BIRDNET_LISTEN##*:}/api/v2/health"`. `docker-compose.yml:153` overrides that with a hardcoded `http://localhost:8502/api/v2/health`, while `:68` passes `BIRDNET_LISTEN: ${BIRDNET_LISTEN:-0.0.0.0:8502}` straight through. So an operator who moves the in-container listener — `.env.example:681` invites exactly that — gets a station that records perfectly and reports itself permanently unhealthy, which trains them to ignore the health signal on the one device whose health signal is the product. (Note `BIRDNET_PORT` is *not* the trigger: it is the host side of the port map at `:114`, and `.env.example:93` says so.) | Derive the compose healthcheck from `BIRDNET_LISTEN` as the Dockerfile does, or delete the override and let the image's own healthcheck stand. Add it to `scripts/check-compose-startup.sh`, which already reads these files. |
| **RC-12** | P2 | VERIFIED | **`.env.example:99` still calls the admin gate "HTTP Basic".** *"Web UI authentication (HTTP Basic)."* It is a session cookie (`routes/auth_pages.rs:40`), and `book/admin/remote-access.md` warns in as many words that `curl -u user:pass` will not work. The reconciliation pass fixed the same mislabel in five documents; this one is in the file every Docker operator copies. | One line. Left because `.env.example` is also the subject of **O-7**'s missing drift gate, and both should land together. |
| **RC-13** | P3 | VERIFIED | **`.env.example` is offered to bare-metal operators as the complete list, and holds a Docker-only variable.** `book/getting-started/configuration.md` points at it as "the full list"; `BIRDNET_PORT` (`:94`) is read by `docker-compose.yml:114` and by nothing in the binary (`grep -rn BIRDNET_PORT crates/ src/ --include=*.rs` → 0). Its own comment says "Host port", so this is a discoverability defect rather than a false statement. | A Docker-only block, or a marker the drift gate of **O-7** can read. |
| **RC-14** | P2 | VERIFIED | **`O-7` is half done and its drift is already back.** The four keys that row named are now in `.env.example`, but the remedy *was* the drift gate and no drift gate exists: `tests/documented_samples_match_the_build.rs` compares the health sample, the confirmation table, the offsite keys and the README test count, and never the env-key set. Consumed and undocumented today: `BNB_HELP_DIR` (`crates/birdnet-web/src/routes/pages/help.rs:63`) and `BIRDNET_REQUIRE_LIVE_EXTENSION`. | Item 3.9 already plans the gate. This row exists so nobody reads O-7's "the four keys are there" as O-7 being closed. |
| **RC-15** | P2 | VERIFIED | **`O-8` landed in the direction the audit called the worse one.** `crates/birdnet-web/src/routes/openapi.rs:158 every_documented_path_is_routed` closes documented→routed. Routed→documented is ungated and the omission is live: `/ws/detections`, `/ws/spectrogram`, `/species/tracking`, `/`, `/soundlevel` and `/stream` are routed and undocumented. There is no `const ROUTES` both sides read — the same shape as `api_write.rs`'s two route tables, which is the pattern that works. | Item 5.16 asks for path-set *equality*; it is not satisfied by the half that shipped. |
| **RC-16** | P2 | VERIFIED | **`NT-11`'s outermost guard has no test.** `src/maintenance.rs:1005 const OFFSITE_BUDGET: … = from_secs(2 * 60 * 60);`, applied at `:1044-1047`. Grep finds the constant, its two uses and its doc comment — nothing else. This is the guard that stops a wedged socket being the last thing the maintenance loop ever does; the two *inner* transport timeouts both have gates. | One test that holds the loop past the budget and asserts it moves on. Small, and it belongs to whoever next touches the offsite path. |
| **RC-17** | P2 | VERIFIED | **Two date-relative purges run outside the clock floor `NT-4` installed.** `clock_is_safe_for_retention` (`src/maintenance.rs:447`) gates exactly two jobs, both inside the maintenance loop (`:180`, `:193`). `crates/birdnet-db/src/audio_levels.rs:234 prune` — the 400-day acoustic baseline **NT-4 names by name** — runs from `src/integrations/acoustic_health.rs:581`, a different loop with no clock check at all, and the weather pruner (`src/integrations/weather.rs:95`) is in the same position. Honest qualification, because the first draft of this row overstated it: the floor only catches a clock that is too *early*, and for `older-than` predicates an early clock deletes nothing. So threading the floor through these two would buy consistency, not data. The direction that actually destroys data is forward, and nothing catches that anywhere — which is **NT-4**'s remaining half, item 1.11. | Thread `clock_is_safe_for_retention` through both for consistency; the data-loss half is 1.11 and is not reduced by doing so. |
| **RC-18** | P2 | READ | **A whole-database read is back in a request path.** `crates/birdnet-web/src/routes/pages/station_health.rs:113` runs `PRAGMA quick_check` on every `/station` render — the read `FIELD_READINESS_AUDIT.md`'s F-7 removed from the badge — while `recorded_db_health` sits unused in the same crate at `routes/pages/health.rs:152`. On a 209 MB database that is the page's whole cost. | Read the recorded verdict, which the maintenance run already stores; that is what it is for. |
| **RC-19** | P2 | READ | **The login page renders a throttle that does not exist.** `routes/auth_pages.rs:347,354-364` renders a `rate_limited` state whose flag is set `true` only by that file's own test (`:427`); `login_submit` implements no throttle. So the UI promises a lockout the code does not have. This is **O-6** seen from the other side — O-6 says the throttle is missing; this says the product already claims it. | Implement the throttle (item 5.2) and let the existing rendering become true, rather than deleting the rendering. |
| **RC-20** | P1 | VERIFIED | **`OB-11`'s redaction mangling reaches the API, and a green test pins it.** `redact_email_local_part(&redact_url_credentials("rtsp://cam:secret@camera.local/stream"))` returns `***@camera.local/stream`: the URL rule produces `rtsp://cam:***REDACTED***@…`, and the email rule then reads its own output as an address, splits on `@`, and returns `format!("***@{domain}")` (`redact.rs:114`). An IP host mangles identically. `OB-11` recorded this for the support bundle; it also applies to `GET /api/v2/settings` (`api_write.rs:614`), so a station misreports its own camera URL to any authenticated client. And `api_write.rs:978` now asserts `out["apprise_url"] == "***@ntfy.example/topic"` with twelve lines of comment endorsing the composition — a gate deliberately written to pin the mangled value, which must change with the fix. | Item 2.17. This row exists to record two things that row does not: the API surface, and that the fix has a green test standing in front of it. |
| **RC-21** | P2 | VERIFIED | **`birdnet_detection_silence_seconds` is consumed by nothing this project ships.** `crates/birdnet-web/src/metrics.rs:774` exports it and the operator manual names it first among the series to alert on. `grep -rn grafana --include=*.rs --include=*.sh --include=*.yml .` returns zero hits — nothing in the workspace reads `docs/grafana-dashboard.json` — the dashboard covers 9 of 29 metric families, this series is not among them, there is no `alerting_rules.yml`, and the `alert: []` stanzas were removed rather than filled. `grafana-dashboard.json:322` also still says "Resident memory (MemoryHigh = 384 MiB)" against `installer/lib/65-service.sh:145 MemoryHigh=768M`. | Item 2.9 (**OB-3**), which is bigger than its one-line entry suggests: it needs the rules file, the dashboard's coverage raised, and a gate holding the dashboard's metric names to the exposition's. |
| **RC-22** | P3 | READ | **`restore_from_backup` still has the shape PS-1 removed, on the startup path.** `crates/birdnet-db/src/resilience.rs:471` `run_to_completion(100, Duration::from_millis(50), None)`. Not PS-1's bug — the source is a backup file nothing writes, so no restart can occur — but the same arithmetic PS-1's own comment spells out: `N/100 × 50 ms` of pure sleep, about 25 s on a 209 MB database, before the listener binds on a recovery boot. Compounds **PS-17**. | One line, by symmetry with `copy_whole_database`. |
| **RC-23** | P3 | READ | **Two different orderings prune the same backup directory.** `resilience.rs:363-390 prune_backups` sorts lexically on the timestamp embedded in the filename; `src/maintenance.rs:1120-1128 prune_old_backups_blocking` sorts on `metadata()…modified()` over a broader `contains(".backup.")` filter. Any operation that rewrites mtimes without rewriting names — a `cp` without `-p`, a restore of the backup directory, an rsync — makes the two disagree about which snapshot is newest, and the mtime pass is the one that deletes. | One ordering. Lands with **RC-10**. |
| **RC-24** | P2 | READ | **`ci.yml` pins the model's digest and not the labels'.** `.github/workflows/ci.yml:296` verifies the model checksum declared at `:275`; `labels.csv` is exported at `:298` unverified, while `install.sh:150`, `installer/lib/10-config.sh:87` and `docker/entrypoint.sh:104` all pin `LABELS_SHA256`. A labels file is what maps a model output index to a species name; a wrong one mislabels silently and for ever. | Pin it in CI as the three shipping paths already do. |
| **RC-25** | P2 | READ | **The release checklist omits a check the release hard-fails on.** `RELEASING.md`'s pre-release list has no `CITATION.cff` line, and `release.yml:86` fails `validate` on it. Tick every box, tag, and get a red tag. | One checklist line. |
| **RC-26** | P3 | READ | **`dependabot.yml:110` names a CI job that does not exist.** | One line. |
| **RC-27** | P2 | READ | **Seven of nine phenology builders are still unreachable**, against `PRODUCTION_AUDIT.md` A-5. Only `effort_corrected_abundance_sql` and `phenology_timing_sql` gained consumers (`crates/birdnet-web/src/routes/analytics.rs:692`). Dead analytical SQL is not inert: it is the shape a reviewer assumes is exercised. | Wire them or delete them; A-5 has been open across three audits without either. |
| **RC-28** | P2 | READ | **Solar overlays draw the configured station's sun over imported rows.** `crates/birdnet-web/src/routes/pages/mod.rs:375` reads only the `latitude`/`longitude` settings, so a merged site's detections are plotted against the wrong sunrise. Provenance again, in a third place. | Resolve the overlay per `import_batch_id`'s `source_lat`/`source_lon`, which migration 25 already stores. |
| **RC-29** | P2 | READ | **`weather` is never joined to `detections`.** `crates/birdnet-db/src/weather.rs:141`, `:155`, `:164`, `:170` are the only `FROM weather` / `DELETE FROM weather` sites in the workspace. The table is collected, pruned, and never used to explain a detection — which is the one thing weather is for in this product. | Either join it in the analytics or say in the schema comment that it is collected for export only. |
| **RC-30** | P2 | READ | **Six `/admin/*` pages still render the retired shell** (`admin/settings/mod.rs:22`, `admin/system.rs:234`, `admin/doctor.rs:37`, `admin/images.rs:121`, `admin/accounts.rs:593`, `admin/overview.rs:107`), and `book/reference/web-api.md:98-106` still points at them. `ENCLOSURE_READINESS_AUDIT.md` E-5/E-12 recorded this and it has not moved. | Port the six, or retire the old shell for real. |
| **RC-31** | P2 | READ | **Every imported detection keeps a dead audio player.** `routes/pages/detection_detail.rs:198-221` emits `<audio controls>` for any non-empty `File_Name` with no existence check, and `templates/layout.html:117` hides only `IMG` on error. An import brings filenames whose clips were never copied. | Check the file exists, or mark the row's audio as absent at import time. |
| **RC-32** | P2 | READ | **The fragile ALSA card index is still taught in the files operators copy.** `.env.example:62` and `install.sh:1430,2034,2782` write `plughw:1,0`, undercutting the installer's own stable `plughw:CARD=<id>` form and the documents that were corrected for it. `src/capture/supervisor.rs:387-391` shows nothing re-resolves the device after a USB re-enumeration. This is **AU-1** and **LC-9** seen in the examples rather than the code. | Change the examples with items 3.15 and 3.17. |
| **RC-33** | P3 | READ | **`openapi.rs:246`'s unrouted detector keys on a 404 body echoing the URL**, which a handler could defeat in either direction. A gate whose signal a change elsewhere can silently invert. | Read the router, not the response. |
| **RC-34** | P3 | READ | **`CADDY_USER` in `birdnet.conf` is inert** — no `EnvironmentFile=` reads it — yet `book/getting-started/configuration.md:71` lists it in that column, so an operator can set a username that has no effect on who can log in. | Read it, or refuse the key at startup. Documenting the trap away is the wrong fix, so this was left for the product decision rather than papered over. |
| **RC-35** | P3 | READ | **`POST /admin/update/apply` is mounted, replaces the running binary, and has no UI caller**, and a second duplicate update poller sits in `system_controls/update.rs`. Related to **LC-3**, which is about whether it can work at all; this is that it is reachable and undocumented. | Resolve with item 3.10's ADR. |


### 3.13 The dimensions the eleven documents under-covered (2026-09-04)

Everything above, in this document and in the ten beside it, is strong on
storage, power, capture and process supervision. This section is the second half
of the reconciliation pass: six investigations into the dimensions none of them
was asked about. They are kept in this register rather than in a twelfth
document, because eleven documents that had to be reconciled against each other
is the problem this session existed to fix.

Severities are the same scale. `VERIFIED` means something was run and its output
is quoted in the row; `READ` means the code was read and is cited.

#### 3.13.1 First run and onboarding (`ON-*`)

Install to first confident detection, and what a misconfiguration looks like
before it costs a season.

| ID | Sev | How | Finding | Remedy |
|---|---|---|---|---|
| **ON-1** | **P0** | VERIFIED | Docker adopted a truncated model, labels, geomodel and geo-labels as final on every restart: `ensure_model_file` returned on `[ -f "$dest" ]` alone. **[FIXED]** — the cached file is now verified, with `installer/test/container-model-cache.sh` observed failing first. | Done. |
| **ON-2** | **P0** | VERIFIED | The detection deadman was permanently blind to a station that had *never* detected: `None` freshness fell into "nothing to say" with no time bound. **[FIXED]** — it now measures against `recording_effort`, so a station that has listened past the threshold and heard nothing alarms, and a new one still does not. | Done. |
| **ON-3** | **P0** | READ | The same presence-only guard on bare metal, for the geomodel: `installer/lib/55-model.sh:237` `if [ -f "${model_dest}" ] && [ -f "${labels_dest}" ]; then` → `GEOMODEL_INSTALLED=1`, so a half-present pair skips verification entirely. The classifier half was fixed by LC-2; the geomodel half was not. | The same `model_file_is_verified` treatment. This is the third instance of one shape and is why **RC-7**'s "write the set down once" applies to checksums too. |
| **ON-4** | P1 | READ | **The wizard's answers do not reach the running station, and the wizard says the opposite.** `overlay_db_settings` runs once, at `src/app.rs:240`; `latitude` and `confidence_threshold` are bridged and take effect only on restart. The Done step says "Within a minute or two you'll see the first detections roll in" (`onboarding.rs:577`) — true, with the occurrence filter still off. `/admin/settings` does carry the restart notice (`settings/handler.rs:112`); onboarding does not. | Either apply the settings live or say what the settings page says. The single biggest gap in the first-run path. |
| **ON-5** | P1 | VERIFIED | **No upper bound on `CONFIDENCE` or `SF_THRESH`.** `config/validate.rs:75,77` range-check 0–1; only a *low* threshold warns. `CONFIDENCE=0.99` draws no finding, records nothing, and — before OB-2 — nobody was ever told. | Warn above a plausible ceiling, as the floor already does. |
| **ON-6** | P1 | READ | `POST /onboarding/save` is merged into the admin router and carries admin auth (`server.rs:143-144`), so a plain form POST on a station with a password gets a 401 dead end mid-wizard. | Exempt the onboarding save, or run the wizard behind the same session it creates. |
| **ON-7** | P1 | VERIFIED | **The Docker path has no timezone handling at all.** Occurrences of `timezone`, `localtime` or `TZ=` in `docker-compose*.yml`, `Dockerfile`, `docker/entrypoint.sh` and `.env.example`: zero, against a control grep that matched. Every container station files detections under UTC hours while its operator reads local ones. Same root as **NT-6**, in the path most new operators take. | `tzdata` in the image and `TZ` documented — item 5.7, which should be read as covering Docker specifically. |
| **ON-8** | P1 | READ | The doctor's timezone-mismatch check runs only if the operator used the wizard's auto-detect button: `doctor/clock.rs:59` guards on `detected_timezone(config)`, which reads a settings row only that button writes. | Compare against the host zone unconditionally. |
| **ON-9** | P1 | VERIFIED | `--doctor` checks model **size**, not integrity (`doctor/model.rs:24`, `> 1_000_000`), and the geomodel not even that (`doctor/config.rs:102` is an `exists()`). A 3 MB stand-in for a 541 MB model passes. This is **LC-2**'s remaining half, item 1.12, and ON-1 has now removed the other route to the same state. | Hash it, or load it and assert `outputs.len() == labels.len()`. |
| **ON-10** | P1 | READ | The first-run checklist ships a hard-coded green tick for the model: `pages/today.rs:291` renders `✓ Model bundled … included` unconditionally. On a station whose model never downloaded, the first screen says it is there. | Read the same predicate the doctor reads. |
| **ON-11** | P1 | READ | The compose `HEALTHCHECK` polls `/api/v2/health` without `?strict=1` (`docker-compose.yml:152-153`), so it cannot see a dead detection daemon — the endpoint answers 200 while its own body says `"detection_daemon":"stopped"`. Reproduced live during this pass against the real binary. | Use `?strict=1` there, or state in the file why not. Ties to **RC-11**. |
| **ON-12** | P1 | READ | The occurrence filter's live state reaches Prometheus (`src/daemon/mod.rs:255-263`) and no HTML surface, so the one number that would show a filter admitting zero species is invisible to an operator without a metrics stack. | Put it on `/station`. |
| **ON-13** | P2 | READ | Nothing asks what filesystem the recordings directory is on. `doctor/paths.rs:18-34` checks existence and writability; `is_tmpfs_mounted` exists (`audio/capture/tmpfs.rs:65`) with zero callers outside its own module. A `RECS_DIR` on tmpfs loses every clip at reboot. | Call it. |
| **ON-14** | P2 | VERIFIED | The container's `verify_sha256` returned success when `sha256sum` was missing. **[FIXED]** with ON-1; it is also **LC-15**. | Done. |
| **ON-15** | P2 | READ | A muted microphone gets a green tick on the first-run checklist for the fifteen minutes that matter; the silent-stream detector is good but its verdict lands on a page a new operator has no reason to open. | Surface the acoustic-health verdict on the first-run checklist. |
| **ON-16** | P2 | READ | The default headless install answers "no location" without asking: `installer/lib/95-main.sh:11` gates the prompt on `[ -t 1 ]`, and the non-interactive branch returns before reaching it. The occurrence filter is then off on every scripted install. | Refuse to finish without a location, or say loudly that the filter is off. |
| **ON-17** | P2 | READ | The installer prompt writes `LATITUDE` to `birdnet.conf`; the wizard writes `latitude` to the `settings` table. `seed_db_settings_from_config` (`src/app.rs:232`) bridges one direction only, so one silently cancels the other depending on order. | One store, or a documented precedence with a gate. |
| **ON-18** | P2 | READ | Nothing in the first-run path ever *listens* to confirm the microphone hears anything. `--channel-report` exists for exactly this question; the doctor's ALSA probe only confirms the card appears in `arecord -l`. | Offer a ten-second listen in the wizard, reporting level and clipping. |
| **ON-19** | P3 | READ | `--doctor` reports a "bundled default" model that does not exist (`doctor/model.rs:52`, emitted as a Skip); `resolve_required_paths` returns `None` and the daemon refuses to start. | Make it a failure and name the real path. |
| **ON-20** | P3 | READ | `check_occurrence_filter`'s doc comment (`doctor/config.rs:65-73`) is stale in both halves — the installer does fetch a geomodel now. | Rewrite. |

#### 3.13.2 Research-grade credibility (`R-*`)

Could a field ecologist publish from this station, and would a reviewer accept
it? This is where the project's differentiation is largest and least contested,
and no previous document asked.

| ID | Sev | How | Finding | Remedy |
|---|---|---|---|---|
| **R-1** | P1 | READ | **No model identity on any detection row.** `install.sh:149` pins `MODEL_SHA256` and it never reaches the database. The shipped model is a pre-release (`V3.0-preview3`). A season spanning a model upgrade cannot be split by which model produced what. | An `analysis_runs` table, below. |
| **R-2** | P1 | READ | **The threshold in force is never stored** — `processor.rs:538` writes `cutoff: None` — while `dynamic_thresholds` (migration 38) moves it per species mid-season. A species' apparent rise is indistinguishable from its threshold falling. | Same table. |
| **R-3** | P2 | READ | No stable primary key; `migration.rs:596` disclaims the rowid. An `occurrenceID` cannot be minted, so a corrected record cannot be matched to the one it replaces. | `id INTEGER PRIMARY KEY`. |
| **R-4** | P1 | READ | **The soundscape is drained after 600 s** (`system.rs:13,33-34`, "nothing in it is worth keeping"). Only audio that already triggered survives, so the archive is selection-biased by the very process under test and no re-scoring is possible. | A retention option that keeps raw audio, even at a duty cycle. |
| **R-5** | P1 | READ | **No re-analysis path exists.** Grep over `src/`, `crates/` and `docs/` finds no facility. When BirdNET ships a new version the old season cannot be brought onto it. | Re-analysis over retained audio, keyed to an `analysis_runs` row. |
| **R-8** | P2 | READ | **The UTC offset is discarded.** A station that changes timezone silently reinterprets its own history; `eventDate` cannot be made offset-bearing. | Store the offset per detection. |
| **R-10** | P2 | READ | `inference/model.rs:507` asserts the outputs are "calibrated probabilities". Nothing validates it, and V2.4's `sigmoid(sensitivity × logit)` is not one. The UI presents the number as a confidence percentage. | Either validate the claim or stop making it in the UI. |
| **R-15** | P2 | READ | Effort-corrected abundance SUMs effort across sources — the interpretation migration 27 explicitly flags as wrong for co-located microphones. | Per-source, or documented as an upper bound. |
| **R-17** | P1 | READ | **All four exports read `FROM detections`**, not `detections_analytic` (`read.rs:335` via `all_detections`). Rejected detections are re-exported, and three of the four carry no verdict column — undoing migration 26 at the one surface where the data leaves the station. | Read the view. This is the same class as **RC-3** and **RC-4**: a third and fourth place the provenance/verdict rule is re-implemented and gets it wrong. |
| **R-18** | P1 | READ | **The eBird export writes Null Island.** `export/ebird.rs:45` `let lat = query.lat.unwrap_or(0.0);` while `LATITUDE`/`LONGITUDE` sit configured in settings. | Read the settings. |
| **R-19** | P1 | READ | **The eBird export is a regression against BirdNET-Pi, at the surface that publishes to a public database.** No confidence floor, no one-per-hour dedup, raw detection tallies written into `Number` (one blackbird detected 200 times becomes "200 birds"), `Protocol=S` and `Observers=1` hard-coded. BirdNET-Pi does all of this correctly (`scripts/history.php:43,127`). | Fix it or withdraw it. Of everything in this section this is the one that damages someone other than the operator. |
| **R-DwC** | P1 | READ | **Darwin Core cannot be emitted.** The schema can fill `scientificName`, `vernacularName`, a date-only `eventDate`, `identificationVerificationStatus`, `associatedMedia`, `organismQuantity` (as detections, not individuals), `samplingEffort`, and the constants. It cannot fill `occurrenceID`, `decimalLatitude`, `decimalLongitude`, `coordinateUncertaintyInMeters`, an offset-bearing `eventDate`/`eventTime`, `datasetID`, `datasetName`, `institutionCode`, `collectionCode`, `recordedBy`, `identifiedBy`, `dateIdentified`, `taxonID`, `scientificNameID`, `taxonRank`, `country`, `countryCode`, `stateProvince`, `locality`, `minimumElevationInMeters`, `license`, `rightsHolder`, `accessRights` or `modified`. `individualCount` must be *omitted*, not guessed. Confidence has no Occurrence term at all. | Three changes get most of it: an `analysis_runs` table (model name/version/sha256, label sha256, threshold, sensitivity, UTC offset) with an FK from `detections`, closing R-1/R-2/R-8 together; an integer primary key plus lat/lon/uncertainty/dataset id written at insert, unblocking six of the seven blocking terms; then a `/export/dwc` route over `detections_analytic` joined to `analysis_runs` and `recording_effort`. |

#### 3.13.3 Stability under adversity (`AD-*`)

| ID | Sev | How | Finding | Remedy |
|---|---|---|---|---|
| **AD-1** | **P0** | READ | **`civil.rs:436` claims "every destructive retention job refuses to run" on an implausible clock, and `secs_look_synced` has no caller in `src/maintenance.rs` at all.** `clock_looks_plausible` is a floor, so a forward step is always "plausible"; `run_clip_retention` then reclaims the whole library while the rows survive, so the loss is invisible in every count. This is **NT-4**'s remaining half stated more sharply than NT-4 states it, plus a comment that is actively misleading about it. | Item 1.11, and correct that comment now rather than with the fix. |
| **AD-2** | **P1** | VERIFIED | **`quick_check` misses index corruption and it is the predicate that gates recovery.** Probe: corrupting a mid-file *index* page gives `quick_check: ok` against `integrity_check: 82 errors`; a table page is caught by both; `VACUUM` does not repair it (82 → 100). `check_integrity` (`resilience.rs:166`) uses `quick_check`, and it gates boot recovery, the backup-source guard and backup validation. So the daily `full_integrity_check` halts ingest, the operator restarts as instructed, `check_and_recover` says healthy, and the five-slot ring overwrites the last good backup within five weeks. (Measured on sqlite 3.45.1; the shipped binary bundles 3.50.x.) | Use `integrity_check` on the recovery path, or `quick_check` first and `integrity_check` before declaring healthy. Interacts directly with **RC-1** and with **RC-18**, where `/station` renders a green "Database integrity" tick from this same weaker check. |
| **AD-3** | **P1** | READ | **A flapping source is invisible to every operator-facing signal.** `clear_fault` (`supervisor.rs:346`, called `:455`) refunds `attempts_since_healthy` after one healthy 2 s tick, so a flapper is pinned at `BACKOFF_BASE` and never approaches the cap; `DOWN_WARN_AFTER` is 120 s *continuous* and never elapses; `UptimeRing::segments` paints the strip Up; `restart_attempts` reads 0–1. Backoff is correctly per-source and no source starves another — the defect is entirely in what is reported. | Count restarts over a window rather than consecutively. |
| **AD-4** | **P1** | READ | **PS-9 unchanged and wider.** No runtime writability probe; `db_health` (`system.rs:147`) is a read-only `SELECT 1` plus a frozen verdict. Further: `detection_silence_secs` is in the health body but not in `degraded` (`:223`), so a week of silence does not make even the strict endpoint go red. | Item 2.12, plus put silence into the strict predicate. |
| **AD-5** | P2 | READ | **One production `sync_all` in the workspace** (`auto_update/mod.rs:374`; the only other is `#[cfg(test)]`). With `synchronous=NORMAL` (`connection.rs:57`) there is no corruption but the last few MB of commits can roll back, undocumented. | **PS-7**, and say so in the durability documentation. |
| **AD-6** | P2 | READ | The purger sees only the recordings directory (`disk/manager.rs:39`). Nothing measures or bounds `birds.db-wal`, the five-copy backup ring, `birds.duckdb` (no retention job at all), or `birds.db.corrupt.*`. | **PS-12** and **PS-6**'s prune half. |
| **AD-7** | P2 | READ | Binary swap and schema migration are both strong. The gap is downgrade: a downgraded DuckDB is quarantined and rebuilt synchronously on the boot path. | Bound it, or do it behind the listener — ties to **PS-17**. |
| **AD-8** | P2 | READ | Nothing sets journald `Storage=` or `SystemMaxUse=`, so a default Pi has a volatile journal; the `errors.jsonl` mitigation is on the same partition as the database and truncates rather than rotates. | Item 2.18. |
| **AD-9** | **P0** | READ+VERIFIED | **The worst pair: partial corruption on a full card.** The restore path needs room for a second whole database; its failure is indistinguishable from "no good backup"; `app.rs:134-156` then quarantines a *recoverable* database and starts fresh — turning a recoverable fault into total history loss, on the failure combination a year in a field makes likely. | Check free space before restoring and distinguish "no room" from "no backup", which is also **PS-10**'s "separate the two verdicts" applied one level out. |

#### 3.13.4 Operability without SSH (`OP-*`)

Of the 33 failure modes enumerated, **8 surface only in the journal** and 2
surface nowhere at all. Against §3.6's original 25, only 4 remain journal-only —
the Stage 2 alerting work is real and this is the evidence for it. The gap has
changed shape: **5 modes are measured and rendered but never pushed**, so they
reach only an operator who is already looking.

| ID | Sev | How | Finding | Remedy |
|---|---|---|---|---|
| **OP-1** | **P0** | VERIFIED | **The entire diagnostic apparatus is reachable only by someone who can already SSH in.** `src/support.rs:124 run()` is called from `src/main.rs:98` and nowhere else; `support_bundle` has zero hits in `birdnet-web`. `doctor::collect_json` is written, tested, and already a bundle member, with no HTTP caller. | Item 2.19, and it is the single highest-value change in this section: both functions exist, the only missing piece is the route. |
| **OP-2** | P1 | VERIFIED | `?strict=1` reports a daemon that died as running — `src/app.rs:439` is the only writer of the flag and runs once at startup. **PR-5**'s surviving clause. | Clear it on daemon exit. |
| **OP-3** | P1 | VERIFIED | Disk, CPU temperature, maintenance outcome and scratch usage are all measured and none is exported as a metric. | Export the four. |
| **OP-4** | P1 | VERIFIED | The station-health conditions are push-only: `evaluate` is private, its sole caller the notifier. No endpoint answers "what is wrong right now?", so an operator who missed a push cannot ask. | An endpoint over the same `CHECKS` table. |
| **OP-5** | P1 | VERIFIED | No `Storage=persistent` / `SystemMaxUse=` drop-in anywhere in `installer/`, `packaging/` or `docker/`. | Item 2.18. |
| **OP-6** | P1 | READ | **`--doctor` never reads a maintenance verdict** — no `last_run_result`, no `JOB_` reference anywhere under `src/doctor/`. The diagnostic an operator runs cannot say "your backup has failed for a year". | The highest-value doctor addition; `station_health.rs:533` already makes the call. |
| **OP-7** | P1 | VERIFIED | A DuckDB mirror write failure is `warn!`-only (`processor.rs:687`) while `/api/v2/health` continues to assert `"analytics": true`. | A counter and a condition. |
| **OP-8** | P2 | VERIFIED | Clip-extraction failure warns, then stores the row against the source filename anyway (`processor.rs:513,523`). The detection is counted; the audio a researcher would check is gone. | Count it. |
| **OP-9** | P2 | VERIFIED | Retention deletion is logged and never counted, so **PR-7**'s runaway purge has no metric an alert could watch. | A counter. |
| **OP-10** | P2 | VERIFIED | Gone-deaf and flapping are measured and rendered but never pushed. | Route them through the same alert path as the other conditions. |
| **OP-11** | P2 | VERIFIED | The scratch filesystem has no alarm and no doctor check: `check_disk` reads only the database's parent, `doctor/environment.rs:30` tests writability and never free space. | **PS-8**, widened. |
| **OP-12** | P2 | VERIFIED | `log_capture.rs:33` says "the counter in `ErrorLog::dropped` is what an operator can see instead"; its only readers are tests. | Export it or correct the comment. |
| **OP-13** | P2 | VERIFIED | The model is checked as a file, not as a model. Same as **ON-9**. | A session load and an output-width assertion. |
| **OP-14** | P2 | VERIFIED | No alert rules ship, and the dashboard covers 9 of 29 families — omitting `birdnet_detection_silence_seconds`, the one series that answers "is this station still detecting?". Also **RC-21**. | Item 2.9. |
| **OP-16** | P2 | VERIFIED | `FullDiskAction::Keep` is unreachable: both non-test call sites hardcode `Purge`. **NP-2** confirmed from the other direction. | Item 3.19. |
| **OP-19** | P3 | VERIFIED | `/api/v2/metrics` is unauthenticated — nested inside `public_routes()` (`routes/mod.rs:83`). | Item 5.20. |

#### 3.13.5 UI, UX and accessibility (`S-*`)

Rendered with the repo's own harness against a real binary and a fresh database,
light and dark, 1440 px and 390 px. English-only (**O-13**) stays closed; the two
parts of it this document said were worth doing were checked and are below.

**`lang`**: eight HTML shells, all literal `lang="en"`, and nothing can change it
— no `documentElement.lang`, no `hreflang`, no locale setting. WCAG 3.1.1 passes.
The gap is 3.1.2: `app.css:130-131` deliberately stacks CJK and Devanagari fonts
for species names, and no species-name element carries `lang=`, so a Japanese
common name is read by an English voice.

**Locale formatting**: zero `Intl.`, zero `toLocaleString`, zero
`<time datetime>`. All formatting is server-side Rust in one hard-coded locale —
English month and day names in four independent tables (`today.rs:374`,
`history.rs:30`, `year_in_review.rs:53`, `viz/timeline.rs:256`), month-first
dates, a hard-coded Monday week start (`history.rs:208,213`), 24-hour only, `.`
decimal via 57 `{:.N}` specifiers with no thousands separator, and metric-only
units. The result is `en-US` text with a Monday week, which is no real locale.

**What the axe gate does not cover**, which is where the findings are:
`withTags(['wcag2a','wcag2aa','wcag21a','wcag21aa'])` drops every best-practice
rule, so `heading-order`, `region` and `landmark-*` are not checked;
`color-contrast` and `link-in-text-block` are explicitly disabled; a closed
`<dialog>` is `display:none`, so the command palette, help drawer and confirm
modal are never analysed on any route; the fixture is seeded with 365 days of
data, so no empty or error state is ever rendered; it runs at 1280 px only; a
clickable `<div>` is not an axe rule; and nothing presses Tab.

| ID | Sev | WCAG | Finding | Remedy |
|---|---|---|---|---|
| **UX-1** | P1 | 2.1.1 A, 4.1.2 A | **Seven onboarding preference cards are bare `<div>`s with a delegated click handler** (`onboarding.rs:548-551,563-565,665`) — no role, no `tabindex`, no `aria-checked`. A keyboard-only operator cannot set the confidence threshold or the notification mode during first-run setup. The a11y gate loads this route and passes it clean. | Real radio inputs, or `role="radio"` with keyboard handling. |
| **UX-2** | P1 | 2.1.1 A | The Access tab's help trigger is a `<span data-help-drawer>` (`admin_accounts.html:61`), not focusable. | A `<button>`. |
| **UX-3** | P1 | 2.4.3 A | "Show more" replaces **itself** (`recordings.rs:303`, `hx-target="this" hx-swap="outerHTML"`), dropping focus to `<body>`; the keyboard user must tab through every new row to reach it again. | Move focus deliberately after the swap. |
| **UX-4** | P1 | 3.3.1 A | **28 page partials return 500, and htmx does not swap a non-2xx response**, so the `aria-busy` skeleton stays forever. A failing station shows a permanent loading state instead of an error — in 28 places. `behavioral.rs:628` documents this exact hazard and the next arm does it anyway. | Return a rendered error fragment with 200, or configure htmx to swap errors. |
| **UX-5** | P2 | 2.2.2 AA | `#detections-table` carries `aria-live="polite"` **and** `hx-trigger="every 15s" hx-swap="innerHTML"`, so a screen reader re-speaks the whole feed every fifteen seconds. | Announce the delta, not the list. |
| **UX-8** | P2 | 1.3.1 A | **Zero `scope=` and zero `<caption>` across 36 tables and 157 `<th>`.** | Add both. |
| **UX-9** | P2 | 1.3.1 A | `table { display: block }` at ≤980 px strips the table role from the accessibility tree, so on any phone every data table stops being a table. | `overflow` on a wrapper, not on the table. |
| **UX-11**, **UX-12**, **UX-13** | P2 | 1.1.1 A | Every analytics chart is a bare `<svg>` with no role, label, title or desc; the spectrogram's alt text is the literal `"Spectrogram"` on the two screens where it is the subject; the live-signal canvas has no role or label. | A text summary of the same numbers. |
| **UX-16**, **UX-17** | P2 | 1.4.3 AA | `#fff` on `var(--accent)` is about 1.8:1 in dark; `#000` on `var(--warning)` about 3.5:1 in light. `--accent` is defined only in the light block and follows `--moss` into dark. | Define both in both blocks and re-measure. |
| **UX-18** | P2 | — | **`app.css` contains no `prefers-color-scheme` rule at all.** Theming is entirely `:root[data-theme]` set by an inline FOUC guard, so with JS disabled the light theme always wins while `<meta name="color-scheme">` tells the browser otherwise. | A `prefers-color-scheme` block, so the default is right without JS. |
| **UX-19**, **UX-20** | P2 | — | **Only `/search` distinguishes "nothing yet" from "nothing matched your filter".** Confirmed by rendering: a station with zero detections ever shows "**No species match this filter yet.**" on `/species` with the filter set to All — `species_pages.rs:366`, whose own doc comment says it is "An honest empty state for a search / filter that matched nothing" and which is used for the no-data case too. This is the first screen after onboarding. `no_life_list()` has no call site at all. | Take the unfiltered total and branch on it. |
| **UX-21** | P2 | 1.3.1 A | Heading levels skip `h1 → h3` on four templates; `today.html` and `recordings.html` contain no `h1` or `h2` in the template itself. | Fix the outline. |
| **UX-22** | P2 | 1.4.11 AA | `input:focus { outline: none }` (specificity 0,1,1) beats the global `:focus-visible` rule (0,1,0), so **every text field loses its focus indicator**. | Raise the `:focus-visible` specificity. |
| **UX-30** | P2 | — | **The mobile tab bar's glyphs are typographic characters, not icons.** `routes/pages/nav.rs:51-75` uses `⌂ ⌬ ▦ ♪ ¶` and a hash. Rendered at 390 px, `¶` for "Reports" reads as a pilcrow — a missing-glyph artefact rather than an icon — on the primary navigation of the mobile UI. `POST_0140_AUDIT.md` recorded this and it has not moved; this is the first time it has been looked at. | Real icons, or glyphs that read as symbols at 16 px. |
| **UX-24**, **UX-27**, **UX-28**, **UX-29** | P3 | 1.3.1 A, 1.4.1 A | `<header role="navigation">` overrides the implicit `banner`, so no page has a banner landmark; `.bnb-card { overflow-x: auto }` at ≤720 px makes every card a two-axis scroll box; the share-404 page has no theme guard and is always light; nothing supports `forced-colors`. | As stated. |

#### 3.13.6 Against both references, at their current tips (`FR-*`, `UP-*`, `WE-*`)

Read from their source at `Nachtzuster/BirdNET-Pi` `88985a3` and
`tphakala/birdnet-go` `b184f689`. Findings already in §3.7–§3.9 are not repeated.

**What neither project does that a field researcher needs**, which is the most
valuable of the three questions and the least explored: none of the three can
produce a defensible season. There is no record of the deployment, no retained
soundscape to re-score when the model changes, no effort denominator in any
export, and no output in a format the verification and archiving tools read. A
season yields a pile of species-and-timestamp rows whose provenance cannot be
reconstructed and whose zeros cannot be told apart from a dead recorder.

| ID | Sev | Finding |
|---|---|---|
| **FR-1** | P1 | **No output any verification or archiving tool reads.** No Raven selection table, no Audacity label track, no Darwin Core — verified absent by grep in all three trees. The ecologist's next step is always Raven; the tool that produces the detections cannot hand them over. |
| **FR-2** | P1 | **The soundscape is destroyed in all three**, so a season can never be re-analysed under a new model. Ours is **R-4**. |
| **FR-3** | P1 | **No deployment record anywhere**: microphone model, height, orientation, habitat, deploy and retrieve dates exist in none of the three. Ours (`migration.rs:362-387`) is the richest of the three and is purely signal-chain. Without it a methods section cannot be reconstructed from the station. |
| **FR-5** | P2 | **Effort never reaches an export, in any of the three** — so a zero cannot be told from a dead recorder. We are the only one that *has* the effort data (**WE-2**), which makes this the cheapest differentiator on the list. |
| **FR-6** | P2 | Level is reported referenced to the ADC, not to a sound pressure, in all three, so two stations cannot be compared. Ours is the only one with a calibration offset at all. |
| **FR-7** | P3 | No sampling design beyond "always on" in any of the three. A duty cycle is standard practice: it bounds compute, equalises effort across sites and reduces temporal autocorrelation. |
| **UP-1** | P1 | **Both references stamp provenance on every detection row and we write NULL.** `processor.rs:543-547` inserts `lat: None, lon: None, cutoff: None, sensitivity: None, overlap: None`. BirdNET-Pi writes all five (`scripts/utils/reporting.py:97-100`); birdnet-go persists four (`internal/detection/factory.go:51-54`). We inherited the schema *and* export its header (`export/csv.rs:173`), so every "BirdNET-Pi-compatible" CSV we emit has five permanently empty columns. Worse for us than for them, because our dynamic thresholds move the rule mid-season. |
| **UP-2** | P1 | **A restore replaces the live database under the running daemon, with no quiesce and no space check.** `admin/system_controls/backup.rs:424` runs `tar xzf … -C <data dir>` over open SQLite handles and then says "Restart the server". BirdNET-Pi verifies the archive's members and free space first, stops services, restores, restarts (`backup_data.sh:95-111,187-196`). Not covered by PS-5/PS-10/PS-15, which are all about retention. |
| **UP-3** | P1 | **birdnet-go detects silent data loss and we cannot.** `internal/diagnostics/doc.go:1-6` keeps a boot journal *outside* the database and diffs consecutive boots for `db_lost`, `db_path_changed`, `mount_changed`, `version_rollback` (`anomaly.go:11-20`). A volume that fails to mount is indistinguishable from a first run for us; the first symptom is an empty chart in October. **PS-2** is the narrower, fixed case. |
| **UP-4** | P2 | birdnet-go records which model produced each detection (`entities/detection.go:9`); we have no model column in 41 migrations. Ours is **R-1**. |
| **UP-5** | P2 | birdnet-go's first-run wizard picks the audio device and asks for a responsible-use acknowledgement; ours does neither (**ON-18**). |
| **UP-6** | P2 | birdnet-go attributes every detection to a named station (`processor.go:1223`); we cannot, which is what makes a multi-site deployment unmanageable. |
| **UP-7** | P2 | birdnet-go hands the operator copy-paste commands when it cannot do something itself — an elevation ladder for its BirdNET-Pi import; we return a message. |
| **UP-8** | P3 | BirdNET-Pi puts its misconfiguration warning where the operator is looking (`homepage/views.php:43-49`); ours is behind a CLI flag and an admin page. Same shape as **ON-10**. |
| **WE-1** … **WE-5** | — | Where we are ahead, recorded so it is not traded away: per-species thresholds learned from human review labels (`thresholds.rs:18-21`), which neither reference attempts; **recording effort**, which neither has at all; BirdWeather store-and-forward that survives an outage where both references drop the upload; the only working export surface of the three (four formats against birdnet-go's one CSV); and provenance labelling that tells the reader when a chart is not one station's data, a concept neither reference has. |

**Claims about upstream in our documents that are now wrong**: `G-30`'s `s3`
backup target does not exist in birdnet-go (`ls internal/backup/targets/` gives
ftp, gdrive, local, rsync, sftp); `G-23`'s `POST /api/v2/detections/:id/comments`
does not exist there either — their comments are stored and read-only; and
`O-1`'s "54 mutating routes" is now **77**.


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
| 1.9 | A database that failed its check stops taking detections | **PS-5** | 8 gates. Against the shipped write path — `with_ingest_db` never refusing, which is what `with_db` did — `a_halted_station_records_nothing` fails `left: 1, right: 0` and the discrimination test fails its own vacuity guard (*"the ingest gate must be closed, or this test is vacuous"*), while the control `a_healthy_station_records_the_detection` stays green. Against the shipped *maintenance* behaviour — the verdict changing nothing — two of the three decision gates fail and `nothing_else_halts_the_detection_writes` stays green, which is the counterpart that keeps a transient `Err` from stopping a station. The third mutation is the structural one: putting a single per-detection write back on `with_db` is invisible to every behavioural gate and is caught only by the source scan, by name and file — *"src/daemon/processor.rs: enqueue"*. |

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
| 2.8 | Re-notify an open episode on a widening schedule | **OB-16** | 15 gates across the schedule and all three alert loops. The shipped posture — `Reminders::due` never firing, which is exactly *one push, ever, per fault* — fails 7 of them and leaves every counterpart green: a condition that cleared, a condition still earning its debounce, a recovered source, and "nothing is due in the first day" all pass either way. The deadman's half is separate, because the decision lives in its pure `transition`: restoring the shipped arm gives `left: None, right: StillBroken { silent_hours: 26 }`. A third mutation — a loop that keeps the import and stops rendering the reminder — is caught only by the source-scanning gate (*"station_health imports the schedule but never renders a reminder"*), which is 2.5's lesson applied: none of the behavioural gates can see a loop that stops calling the policy. The fourth is the one that found a real defect rather than confirming a fix: the first version advanced the counter one step per call, so a station suspended for a month replayed every swallowed step, one per five-minute poll. The test that was supposed to catch it asserted only that one call returned one reminder — true of any implementation — and was green for a reason that had nothing to do with what it claimed. It now asks what the *next* poll does. |
| 2.10 | One disk denominator, everywhere | **OB-6**, **PR-14** | 6 gates. 4 mutations killed. The shipped predicates fail the reproduction (`a disk 76.6 % full is not critical (was: 9167069184 available < 13527658700 = total/20)`) and the swept property gate; `is_critical` returning `false` unconditionally fails the two full-disk counterparts, so the fix is not "stop reporting"; a `CRITICAL_PERCENT` of 98 fails the purge-threshold coherence gate. The fourth is the instructive one: making `used_percent()` divide by `total` **as well** leaves the property gate green — two surfaces agreeing on the same wrong number — and is caught only by the reproduction, which pins the answer to what `df` says. |
| 2.14 | Publish the MQTT status topic that discovery already advertises, with a last will | **OB-8** | 9 gates against a broker stub that *decodes* CONNECT and PUBLISH rather than matching bytes. 8 mutations killed, each by one gate: no will (`"a will was registered"`), a will on the stateless publish too (only the discrimination test fails), the will written after the username — a well-formed packet that publishes the password to whatever the broker reads as the topic — `ping` that writes and never reads (`"an unanswered ping must fail"`), `config.qos` ignored, which is the shipped code (`"an unacknowledged QoS 1 publish must not report success"`, while the `QoS` 0 counterpart stays green — the fix is not "every publish now blocks"), the retain override ignored, also shipped (`"override honoured"`), `shutdown` that disconnects without saying offline (`left: 1, right: 2`), and an unretained will. |
| 2.15 | Operational alerts reach the notification log | **OB-13**, and **NL-1** found while doing it | 11 gates, 6 mutations killed. `flush` recording nothing — the shipped state — fails 4 and leaves `no_notifier_configured_writes_nothing` green. Then: `Queued` written as `Failed`, a row per retry instead of one per episode, placeholder species columns, a loop sending inline again (the pre-2.2 shape, caught by the source scanner), and the CHECK left un-widened — `"the schema rejects the `queued` status this code writes: CHECK constraint failed"`. That last one is **NL-1**: the two behavioural gates were written, run, and failed against the shipped schema before the migration existed. |
| 2.16 | "Test notifications" sends what an alert sends, and is live whenever a destination resolved | **OB-9** | 11 gates — 5 behavioural against a local destination through the real admin router, 6 at the renderer. Against the shipped handler four of the five behavioural gates fail: the button reaches nothing (`left: 0, right: 1` requests at the station's own destination), it renders `class="btn-disabled" … disabled` for a station whose native route is working, an open circuit is reported as *"Apprise URL not configured"*, and the module holds two HTTP clients (`left: 2, right: 1`). The fifth — no notifier at all still yields a disabled button and an error, not a send — passes **both** ways, which is what stops the fix being "always enabled". The discrimination is the open circuit: with `Gate::admit_priority` returning `Send` unconditionally the other four stay green and that one fails, so this is a test that goes through the shared guards rather than one that merely reaches the destination. |

**Still to do:**

| # | Item | Finding |
|---|---|---|
| 2.4 | Undervoltage and throttling telemetry | **NP-5** |
| 2.9 | Alert rules and a dashboard/exposition agreement gate | **OB-3** |
| 2.11 | Prune quarantined stores on a retention schedule (detection and the condition landed with 2.3) | **PS-6**, remaining half |
| 2.12 | Read-only-remount detection | **PS-9** |
| 2.13 | `--doctor` measures the card, not the RAM disk | **PS-8** |
| 2.17 | Redact by shape in the support bundle; stop mangling RTSP URLs | **OB-10**, **OB-11** — the rules moved to `birdnet_core::config::redact` with 5.14, so this is now a one-place fix rather than two. The mangling is measured: `redact_email_local_part(&redact_url_credentials(v))` turns `rtsp://cam:secret@camera.local/stream` into `***@camera.local/stream`, losing the scheme and the username, because the second rule reads the first's output as an email address. `GET /api/v2/settings` discloses the same shape and its gate pins that exact string, so a fix here will show up there |
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
| 3.22 | A provenance dimension in `species_summary`'s key, so the rollup answers both questions and no station pays the scan | **RC-3**, remaining half |

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
| 5.14 | Mutating endpoints under `/api/v2` with bearer auth — **done**: the four `/detections/*` writes, `POST /detections/batch`, `GET`/`PUT /settings` and `POST /control/restart` | **O-1** |
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

### Stage 8 — from the reconciliation pass

Placed after the existing stages rather than interleaved, because the numbering
above is referenced from four other documents and from commit messages.

**§3.13 is its own queue and is deliberately not copied here.** Two copies of a
work queue is how one of them goes stale — §6 says so about Stage 2, and the
same applies to eighty-six rows. The three items from it that reorder the head
of *this* queue are named in §6's "What to do first"; everything else is ordered
within its own subsection, by the same rule as here: what a year in a field
would actually cost. Ordering
within this table is by what a year in a field would cost, same as everywhere
else.

| # | Item | Finding |
|---|---|---|
| 8.1 | Assert `HttpOnly` and `SameSite` on the session cookie; then rewrite `security.rs`'s module doc to name the two mitigations that actually exist | **RC-6**, **RC-5** |
| 8.2 | A named table of `--doctor` check families that a source-scanning gate reads, and a module doc generated from it rather than restating it | **RC-7**, **RC-8** |
| 8.3 | Derive the compose healthcheck's port from `BIRDNET_LISTEN` as the image's own already does, and cover it in `scripts/check-compose-startup.sh` | **RC-11** |
| 8.4 | One backup-retention constant, or two that name their different jobs; and one ordering for the two pruners | **RC-10**, **RC-23**, and the mechanism behind **PS-15** |
| 8.5 | Read the recorded database verdict on `/station` instead of running `PRAGMA quick_check` per render | **RC-18** |
| 8.6 | A test for `OFFSITE_BUDGET`, the outermost guard on the maintenance loop | **RC-16** |
| 8.7 | Pin `labels.csv`'s digest in CI as the three shipping paths already do | **RC-24** |
| 8.8 | `CITATION.cff` in the release checklist, since `release.yml` hard-fails on it | **RC-25** |
| 8.9 | Thread the clock floor through the two purges outside the maintenance loop — for consistency; the data-loss direction is 1.11 and this does not touch it | **RC-17** |
| 8.10 | A `Relaxed` census gate, labelled as review debt rather than as a correctness proof; and §3.11's sentence qualified by commit (done) | **RC-9** |
| 8.11 | Resolve the solar overlay per import batch, using the `source_lat`/`source_lon` migration 25 already stores | **RC-28** |
| 8.12 | Either join `weather` to detections or say in the schema that it is collected for export only | **RC-29** |
| 8.13 | Wire the seven unreachable phenology builders or delete them | **RC-27** |
| 8.14 | Check the clip file exists before rendering a player for it | **RC-31** |
| 8.15 | `.env.example`: the "HTTP Basic" mislabel, the `plughw:1,0` examples, and a Docker-only block — all of which the **O-7** drift gate (item 3.9) should then hold | **RC-12**, **RC-32**, **RC-13**, **RC-14** |
| 8.16 | Port the six `/admin/*` pages still on the retired shell, or retire the shell | **RC-30** |
| 8.17 | Make `openapi.rs`'s unrouted detector read the router rather than a 404 body | **RC-33** |
| 8.18 | `restore_from_backup`'s `step(-1)`, by symmetry with `copy_whole_database` | **RC-22** |
| 8.19 | Read `CADDY_USER` or refuse it; fix `dependabot.yml`'s dead job name | **RC-34**, **RC-26** |

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

`cargo test --workspace` on x86_64 in a container, re-run at `ee795ed` by the
reconciliation pass and matching what this block already said: **3 640 passed,
0 failed, 7 ignored, 111 suites**. `cargo fmt --check --all` exits 0 at that
commit. The same command at the branch point `f33eb9e` reports 3 570 in 106
suites, so the difference is the deployment pass's own gates and nothing else.
(This block read "3 567, 106 suites" and then "3 618" as that pass went on;
re-take it rather than carrying a figure forward — the count moves with every
commit here. Extract it with `grep "^test result:"` and sum the fields; a
`| tail -N` will report exit 0 over a run with failures inside it.)

The reconciliation branch takes the suite to **3 661 passed, 0 failed,
7 ignored** in **112** suites — twenty-one gates across six files, and one new
suite, `crates/birdnet-db/tests/the_species_list_honours_the_provenance_rule.rs`.
`--workspace --all-features` gives the same set as `--workspace` here, because
`analytics` is the only feature and it is on by default. (This block read
"3 653" between the fourth fix and the sixth; re-take it rather than carrying it,
which is what the paragraph above says and what this sentence is evidence for.)

Not in that count, because it is not a cargo test:
`installer/test/container-model-cache.sh`, run by
`installer/test/run-ci.sh` — whose accounting step fails if a file in that
directory is neither run nor excluded with a reason — and by CI's
`installer unit tests` job. `shellcheck 0.10.0 --severity=warning -x` is clean
over `docker/entrypoint.sh`, `installer/test/*.sh`, `quickstart.sh`,
`install.sh` and `scripts/*.sh`, and `installer/build.sh --check` reports
`install.sh` in sync with `installer/lib/*.sh`.

Line counts at `ee795ed`, one method
(`find crates src -name '*.rs' | xargs cat | wc -l`): **184 379** lines of Rust
under `crates/` and `src/` across 458 files, **199 417** with `tests/`. The
figures in §0 — 172 482 and 186 040 — were taken at `35acd9e` and are correct
for that commit; growth, not error. Upstream at the tips this pass measured:
`Nachtzuster/BirdNET-Pi` is still at `88985a3` and has not moved since §0 read
it; `tphakala/birdnet-go` has moved from `265b6455` to `b184f689`.

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

  A container fact worth carrying: `typos`, `shellcheck` and `cargo-mutants`
  are **not** in the base image either. They were installed by hand in this
  session (`cargo install typos-cli`, `cargo install cargo-mutants`, and the
  0.10.0 release tarball for `shellcheck`) and all three gates then pass
  locally — `typos` 1.50.1 against `./.typos.toml`, `shellcheck` 0.10.0 at
  `--severity=warning -x` over the 26 files CI checks, `cargo-mutants` 27.1.0.
  A previous handoff recorded them as "installed here"; they are not, and the
  next session will have to install them again before it can claim them.
* `birdnet-behavior --doctor` exits **1**, not 0, in a container. Exit 1 means
  "worst severity is Warn" (`doctor/render.rs::summarise`), and every warning
  is an unconfigured-environment one. **Still true**, and still not to be
  ticked off without a configured station.

  The *counts* move with the container **and** with the checks themselves, so
  do not carry them forward. This block has recorded, in order: "8 passed, 9
  warnings"; then 9 passed, 8 warnings; then — after 5.14 added an `API write
  surface` check — 10 passed, 8 warnings; and now **9 passed, 9 warnings, 0
  errors, 5 skipped**, because `Disk space` flipped back to WARN when the
  container dropped to 4 GiB free during a build. It warned under 1 GiB on the
  first container, passed at 13 GiB, and warns again at 4 GiB — the same check,
  the same binary, three answers. The nine warnings now are: configuration
  file, station location, species occurrence filter, admin authentication,
  HTTPS, database directory, offsite backup, audio source, disk space. Re-run
  it rather than quoting this list; that is the point of the paragraph.

### What to do first

*Rewritten by the reconciliation pass, 2026-09-04. The paragraphs after this
one are the previous handoff and are still accurate; this is what changed.*

**`AD-2` first: `quick_check` gates database recovery and cannot see index
corruption.** This is the one thing found in this pass that still destroys data
after everything on this branch. It was measured, not reasoned: corrupting a
mid-file index page gives `quick_check: ok` against `integrity_check: 82
errors`, and `VACUUM` does not repair it. `check_integrity` uses `quick_check`
and gates boot recovery, the backup-source guard and backup validation — so the
daily check halts ingest, the operator restarts as the message tells them to,
`check_and_recover` says "healthy", and the five-slot weekly ring overwrites the
last good backup inside five weeks. `RC-1` closed the truncation hole on the same
path this pass; this is the other hole in the same predicate, and it is worse
because the file still looks like a database. `AD-9` is the same failure with a
full card underneath it, where the restore path's "no room" is indistinguishable
from "no backup" and a *recoverable* database gets quarantined.

**Then `R-19`, because it is the only finding here that damages someone else.**
The eBird export applies no confidence floor, no one-per-hour deduplication,
writes raw detection tallies into `Number` — one blackbird detected two hundred
times becomes "200 birds" — and hard-codes `Protocol=S, Observers=1`. It also
writes latitude 0, longitude 0 while the real coordinates sit in settings.
BirdNET-Pi does all of this correctly, so this is a regression against upstream
at the one surface whose output leaves the station and enters a public database.
Fix it or withdraw it; shipping it is worse than not having it.

**Then `OP-1`, which is the cheapest large win in the document.** The support
bundle and `--doctor` are reachable only over SSH. `doctor::collect_json` is
written, tested, and already embedded in the bundle; `support::run` likewise.
Item 2.19 is therefore two routes, not a feature — and it converts a large
fraction of §3.13.4 from "the operator must drive out" to "the operator can look".

**Then `RC-3`'s remaining half — item 3.22, the provenance dimension in
`species_summary`'s key.** It is the only thing on this list where the shipped
fix is deliberately the second-best one. The reader-side substitution that
landed makes the numbers correct, and it does so by putting exactly the stations
that merged another site's history — which are the large ones — back onto the
unbounded scan migration 30 existed to remove. The lasting fix is to add an
`is_import` dimension to the rollup's primary key, so both answers come from the
rollup and nobody pays: at 200 species that is 9 600 rows rather than 4 800, and
the triggers stay exactly as reversible as they are now. It is a schema
migration plus a rewrite of three triggers, and the ordering note in migration
30's own comment about dropping the summary triggers before a bulk rewrite is
the thing to read before starting. The gates are already written: the four in
`the_species_list_honours_the_provenance_rule.rs` must stay green through it,
and `a_station_with_nothing_to_exclude_still_reads_the_rollup` is the one that
will tell you whether you have actually kept the fast path.

**Then `RC-5`, `RC-6`, `RC-7` and `RC-8` together**, because they are one
change with two halves. `security.rs`'s module doc argues that CSRF needs no
synchroniser token *because there is no session to bind to*, and there has been
a session cookie for some time; the actual defence is `SameSite=Lax` plus the
same-origin check, and nothing asserts either. Write the two assertions first,
then the doc, then the doctor's `CHECKS` table — which is the same lesson §6
closes with, applied to the one place in the tree that still has the hole
`station_health` had.

Three things about this queue that were not true when the previous handoff was
written, and one that never was:

* **`OB-4` and `OB-12` are done** and were still written as open. Their rows say
  so now. What is left of `OB-4` is `PR-5`'s second clause — the daemon
  `AtomicBool` at `src/app.rs:439` that no exit path clears — so `?strict=1`
  still reports a daemon that died as running. That is a smaller job than the
  row makes it look.
* **`PS-16` is two-thirds done**: `evaluate` runs six conditions from a named
  table with a gate holding the module doc to it. Only "nothing reads
  `detection_write_failed`" survives.
* **`O-7` and `O-8` shipped the visible half and not the gate**, which was the
  remedy in both cases. `O-7`'s drift is already back (`BNB_HELP_DIR`,
  `BIRDNET_REQUIRE_LIVE_EXTENSION`); `O-8` closed documented→routed and left
  routed→documented open with six live endpoints undocumented. Neither should
  be read as closed. See **RC-14** and **RC-15**.
* **`PS-18` was wrong about its own mechanism**, in the direction that makes
  **PR-3** worse. Anyone sizing the tmpfs should read the corrected PS-18 row
  before trusting PR-3's numbers.

A note on method, because it cost this pass real time. Nine independent
investigations produced ~470 verdicts, and the two that were wrong were both
wrong in the *fluent* direction — a claim that reads well and inverts a fact.
One said the supply-chain, coverage, install-smoke and mutation workflows "never
run on a `claude/**` PR"; `pull_request.branches` filters the PR's **base**, and
every one of them ran on this branch's own PR. The other said **NP-13** was
stale; it is not, and it is now VERIFIED rather than READ. Both were caught by
going to the artifact — the workflow's check-run list, and the two players'
source. Neither would have been caught by reading the claim again.

The two items this section used to name — **2.16 (`OB-9`)** and **2.8
(`OB-16`)** — are both done, so the head of the queue is open. Nothing left in
Stage 2 orders itself the way those two did, and nothing in it blocks anything
else, so any of them can be taken next; §4's **Stage 2 — still to do** table is
the list, and it is not repeated here because two copies of a work queue is how
one of them goes stale.

Two of those items are now cheaper than the table suggests, for the same
reason. **2.17** (redact by shape in the support bundle) is a one-place fix
since 5.14 moved the rules into `birdnet_core::config::redact`, and the
mangling it names is measured rather than suspected — see that row. **2.20** (a
test path for email and MQTT) is the one item still holding `OB-9` open.

Stage 1 still has 1.10 (`PS-7`/`S-4`), 1.11 (`NT-4` remaining half) and 1.12
(`LC-2` remaining half) outstanding. Those are about *keeping the data*, which
outranks everything in Stage 2 on a station that is already failing — take them
first if you have no other reason to choose. 1.9 (`PS-5`) is done, in its
narrowed form; the runtime quarantine-and-restore branch of that remedy is not,
and stopping the ingest writer was its prerequisite.

**5.14 (`O-1`)** is done — all eight endpoints the remedy named. Anyone adding
a ninth should read `crates/birdnet-web/src/routes/api_write.rs` first: the two
route tables at the top are what the CSRF guard, the router-mount gate and the
two `openapi.json` gates all read, so an endpoint added without an entry there
is invisible to every one of them. They are also the reason a gate named
`the_route_table_is_the_router` had to be renamed: it did not check the router,
and passed with a route unmounted. The second thing to read is
`tests/analytics_divergence.rs`, which is why the batch endpoint loops over the
paired `AppState` writes instead of taking one transaction.

### A gap in the mutation matrix — a proposal, not a change

`.github/workflows/mutation.yml` has 25 rows covering **ten** file patterns:

| Package | Pattern |
|---|---|
| `birdnet-core` | `config/validate.rs`, `inference/model.rs`, `audio/extraction/{extractor,convert}.rs`, `civil.rs` |
| `birdnet-db` | `migration.rs`, `sqlite/queries/detections/*.rs` |
| `birdnet-behavior` | `src/daemon/*.rs`, `src/capture/{schedule,supervisor}.rs` |

**Nothing in `crates/birdnet-integrations/` or `src/integrations/` is mutated**,
and both hold delivery decisions the covered code branches on.

This is not hypothetical. In the previous pass a mutant survived at
`src/daemon/processor.rs`'s `if !e.nothing_was_attempted()` call site. The
tempting fix was to move the decision onto `AppriseError` as a positively-named
predicate — which would have made the mutant **vanish by relocating it into an
unmutated file**, turning the gate green while testing less. It was rejected and
the test written instead, but the hazard is structural: `apprise.rs`'s
`drop_reason` and `nothing_was_attempted`, and `announce.rs`'s `Outbox::settle`
and `flush`, are exactly the shape of decision a survivor can be hidden in.

Counts, from `cargo mutants --list --package P --file F | wc -l`
(cargo-mutants 27.1.0), not estimated:

| Package | File | Mutants |
|---|---|---|
| `birdnet-integrations` | `src/apprise.rs` | **93** |
| `birdnet-integrations` | `src/dispatch/limit.rs` | 36 |
| `birdnet-integrations` | `src/dispatch/parse.rs` | 23 |
| `birdnet-integrations` | `src/dispatch/plan.rs` | 17 |
| `birdnet-behavior` | `src/integrations/station_health.rs` | 65 |
| `birdnet-behavior` | `src/integrations/acoustic_health.rs` | 55 |
| `birdnet-behavior` | `src/integrations/reminder.rs` | 31 |
| `birdnet-behavior` | `src/integrations/announce.rs` | 21 |
| `birdnet-behavior` | `src/integrations/deadman.rs` | 16 |

**The proposal, for the maintainer to accept or decline** — six new jobs is real
CI time, and that is not a call to make unilaterally:

* `crates/birdnet-integrations/src/apprise.rs`, **4 shards**. It is a library
  row: `birdnet-integrations` depends on `birdnet-db` (bundled `rusqlite`) but
  **not** on `birdnet-behavioral`, so no DuckDB and no ONNX are linked, and it
  belongs in the same cost class as `db/migration.rs` — which the matrix sizes
  at ~28 mutants a shard. 93 over 4 measures **24 24 24 21**
  (`--list --shard k/4`, every shard non-empty); the empty-tail guard the CI
  config check enforces is `3 × 24 = 72 < 93`. Three shards also works and
  measures **31 31 31**, slightly above the documented slice size.
* `src/integrations/announce.rs`, **2 shards**. A binary-crate row, so every
  mutant relinks DuckDB and ONNX and the matrix sizes those at ~13 a shard. 21
  over 2 measures **11 10**; guard `1 × 11 = 11 < 21`.

Both need their paths adding to the workflow's two `paths:` filters as well, or
a PR touching them will not run the job.

Two things not measured here, and neither should be guessed at:

* **Per-mutant wall-clock for these files.** No sweep was run — the container's
  disk does not have room for `target/mutants` beside a 16 GB `target/`. The
  shard counts above come from the matrix's own documented slice sizes, not from
  a timing of these files.
* **The first run is not a measurement.** The workflow's own comment on
  `civil.rs` records that a *new* matrix label has no cache and restores an
  arbitrary other row's `target/mutants/`; eight rows once spent 45 minutes each
  learning that. Expect one bad run after adding these, and size nothing from it.

The remaining files in the table are listed so the next person does not have to
re-derive them, not as part of the proposal.

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
