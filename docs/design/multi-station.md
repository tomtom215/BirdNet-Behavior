# Several stations, one store

**Status: proposal. Nothing in this file is implemented.** It needs the
decisions in the last section before any code is written.

The goal, as the maintainer put it: several stations writing to one store, so
that a remote station can feed a central one. This file says what exists today
that such a design can stand on, what the options are, which one is
recommended and why, what it would cost, and what it would rule out.

Labels used below: **VERIFIED** means a command or a file read in this tree
confirms it (the file is named); **DESIGN** is a claim about the proposed
system that nothing has tested yet.

## What the tree already provides (VERIFIED)

| Need | What exists | Where |
|---|---|---|
| A row's own identity | `UNIQUE(Date, Time, Sci_Name, COALESCE(File_Name,''), chunk_offset_secs)`; **no station dimension** | `crates/birdnet-db/src/migration.rs`, migration 23 |
| A point in time that survives time zones and DST | `detections.detected_at_utc`, beside the local wall-clock `Date`/`Time` | migration 32 |
| Which model made a row, comparably across stations | `analysis_runs` (model file SHA-256, label count, geomodel SHA-256, version, settings) and `detections.run_id` | migration 43 |
| Which stream inside a station | `detections.Source` | migration 18 |
| A way to carry another site's rows without losing where they came from | `import_batches` (source label, coordinates, UTC offset, applied clock shift) and `detections.import_batch_id` | migration 25 |
| A durable "send it later" queue | `outbound_queue`: bounded (5 000 rows per kind), 48 attempts, 1 min doubling to a 1 h backoff. Deliberately only for channels where late delivery is correct (BirdWeather) | `crates/birdnet-db/src/outbound_queue.rs` |
| A token-gated write surface that is closed unless configured | `BNB_API_TOKEN`, only a SHA-256 digest kept, a table of write routes (`WRITE_ROUTES`) read by the router, the CSRF guard and a gate test | `crates/birdnet-web/src/api_token.rs`, `routes/api_write.rs` |
| A TLS authority for a station's own names | a local CA constrained to the station's names, rustls verified | `crates/birdnet-web/src/tls.rs` |
| Sync from the system of record into the analytics copy | `sync_from_sqlite`, `insert_detection` (the DuckDB copy is rebuildable from SQLite) | `crates/birdnet-behavioral/src/connection/sync.rs` |
| Privacy applied before anything is stored | the human-sound filter runs before a detection is written, so a suppressed window never exists to be sent | `crates/birdnet-core/src/detection/privacy.rs` |

What does **not** exist: a station identity (there is `STATION_NAME`, a
display string; nothing mints or persists an id), any endpoint that accepts
detections from another process, any notion of a second station in the schema
other than a one-shot import, and any rule for what a clip's path means on a
machine that did not record it.

## What a design has to get right

These are the properties the options below are judged by. Each is also a test
that must exist before a phase is called done.

1. **Exactly-once effect.** Delivery is at-least-once (networks retry); the
   store's result must be as if each detection arrived once. Replays, reordering
   and partial batches must change nothing.
2. **No loss across an outage.** A detection is irreplaceable. The station's own
   database is the durable log; "pending" is a cursor into it, not a second
   bounded copy that can drop rows the way `outbound_queue` is allowed to.
3. **Time is unambiguous.** Every row carries `detected_at_utc` and its station's
   offset, and the store never reinterprets a station's wall clock. A station
   whose clock is wrong is detected (skew measured at handshake and on every
   batch), not trusted.
4. **Comparability is explicit.** Two stations with different models, thresholds
   or locations are not silently pooled: provenance travels with the rows.
5. **A station cannot do more than it was enrolled to do.** Revocable, ingest-only
   credentials per station; no credential that reads or writes another
   station's data.
6. **The station keeps working alone.** Losing the hub never stops detecting,
   recording, or serving the station's own pages.
7. **One place to look.** An operator sees every station's health ("last heard
   from", pending rows, version, model) without opening each one.

## The options

### A. Push to a hub; one database per station at the hub (recommended)

The same binary runs in two roles. A **hub** exposes a token-gated ingest API; a
**remote station** pushes its detections to it, in batches, from a cursor.

- *Transport (DESIGN).* HTTPS `POST` of a batch of rows with the station's own
  `run_id` set, each row identified by the existing unique key plus a station
  id. The station records the highest acknowledged row and resumes from it;
  nothing is dropped because the source of truth is the station's table, not a
  queue. The hub answers per row (`stored`, `duplicate`, `rejected` with a
  reason), `429` with `Retry-After` under load, and a protocol version so a
  hub and a station on different releases refuse each other legibly.
- *Storage (DESIGN).* The hub keeps **one SQLite file per remote station**, each
  with exactly today's schema, and one reader/writer pool per file. A remote
  station's pages are today's pages, pointed at its file. This gives each
  station its own writer, so several stations ingest at once without queueing
  behind one lock, and a station can be exported or deleted as a file. The
  cross-station view is the DuckDB analytics store, which gains a station
  column (it is already a rebuildable copy, so adding a dimension there
  changes no source of truth).
- *Enrolment (DESIGN).* The hub mints a one-time enrolment code (short expiry);
  the station redeems it for an ingest-only token, stored as a digest at the
  hub like `BNB_API_TOKEN`. The hub's CA or certificate fingerprint travels with
  the code, so the station pins what it talks to. Revocation is deleting the
  token.
- *Clips (DESIGN).* Policy per station: none, only rare or high-confidence rows,
  or all; uploaded after the row, verified by SHA-256, stored under the
  station's own directory. A clip that has not been delivered is protected from
  the disk purge until it has been, or until a stated limit says it will not be.
- *What it costs.* A `stations` concept in the schema and the UI (a switcher,
  an all-stations overview, a stale-station alert on the existing alert
  rules); an ingest module; a sync module on the station; a station id minted
  once and persisted; N files and pools at the hub with a bounded, lazily
  opened set; migrations run per file at hub upgrade.
- *What it rules out.* Cross-station SQL on the operational side: "all
  stations" is an analytics-store feature, so a hub built without analytics
  has per-station views only.

### B. Every station writes to a shared network database (PostgreSQL)

Simplest to describe; worst fit. It needs a database server beside a Raspberry
Pi appliance, hands every station a credential that can write every other
station's rows, loses the "keeps working alone" property the moment the link
drops (or needs a local copy and a sync layer anyway, which is option A with a
heavier hub), and requires porting 54 migrations and the SQLite-specific
queries. Rejected.

### C. DuckLake as the store

DuckLake (Parquet files plus a SQL catalog) does allow many processes to
read and write one dataset. Its own documentation says that depends on the
catalog: a DuckDB-file catalog is limited to one client, a SQLite catalog
serialises writers behind SQLite's write lock, and PostgreSQL is the
multi-user choice. Two plain appends to one table do not conflict; two
deletes from the same data file do, as does an insert into a table another
transaction has dropped, altered or deleted from, and the loser aborts
(VERIFIED by reading the DuckLake catalog and conflict-resolution pages and
the DuckDB concurrency page on 2026-10-20; not run here). It does not remove the need for A's protocol: it moves the
write path from "post rows to a hub" to "hold storage and catalog credentials
on every station", and a station that is offline still cannot commit.

It is a reasonable **later** home for the hub's *analytical* copy, if the
hub ever needs several ingest processes or analysts attaching read-only while
it runs. That is a swap behind the analytics store, not an alternative to the
protocol, and it should be decided by a measured spike (the `ducklake`
extension against the DuckDB version pinned here, and per-row insert behaviour)
rather than by this document.

### D. The hub pulls from stations

The read API already exists, so a hub could poll stations. It needs every station
reachable from the hub (home networks are not), has no outage story beyond
polling again, and leaves the hub's data dependent on stations being up. Useful
as a *LAN-only first step* because it needs almost no station code; wrong as the
general design.

### E. MQTT as the transport

The station already publishes to a broker. A broker adds a dependency, at-least-once
delivery needs the same idempotency as A, and payload size and clip transfer do
not fit. Reasonable for *live* events on a hub's dashboard; not the system of
record.

## Phases (DESIGN; each ends with a test a reviewer can run)

0. **Identity, no behaviour change.** Mint and persist a station id; show it and
   `STATION_NAME`; record it in `analysis_runs`. *Gate:* upgrade of an existing
   database keeps every row and mints exactly one id; two restarts give the same id.
1. **Ingest protocol and one-file-per-station store.** The hub accepts a batch from a
   scripted client. *Gates:* replay of a batch changes nothing; a batch cut at
   every byte boundary and resumed lands each row once; reordered batches give the
   same table; a row with a clock more than a stated skew from the hub's is
   flagged, not silently stored.
2. **Remote-station sync.** A cursor on the station, back-off, a visible
   "synced 3 s ago, 0 pending" line. *Gates:* an outage of N hours with M detections
   delivers M rows on return with no duplicates; killing the station process
   mid-batch loses nothing; the disk purge refuses an unsynced clip.
3. **Enrolment and revocation.** One-time code, ingest-only token, pinning.
   *Gates:* a token cannot read; a revoked token is refused on the next request;
   a station cannot post under another station's id.
4. **The hub's pages.** Switcher, all-stations overview, per-station health and a
   stale-station alert. *Gates:* the existing layout, axe and interaction gates
   over a fixture with three stations; a count on the overview equals the sum of
   the per-station counts, asserted by a test.
5. **Clips.** Policy, upload, verification. *Gates:* a corrupted upload is refused
   and retried; a clip is playable at the hub.
6. **Scale and soak.** N simulated stations at a stated rate against a hub on the
   smallest supported hardware, with the numbers recorded. No claim about how
   many stations a hub holds exists until this runs.

## Decisions needed before Phase 0

1. **Who are the stations?** Your own, under one owner (a single trust boundary,
   shared enrolment is fine) or contributed by other people (then per-station
   data ownership, location fuzzing at the hub, and abuse limits matter from the
   start).
2. **Where does the hub run?** On a Pi on the same network, or on a server
   reachable from home networks? It decides whether TLS and pinning are a
   requirement or a convenience.
3. **Clips: which policy is the default?** They are most of the bandwidth and
   all of the privacy exposure.
4. **Scale target.** Stations and detections per day the first release is
   expected to hold. It sets how much of Phase 6 is a gate and how much is a
   later optimisation.
5. **One hub, or hubs that feed hubs?** This proposal assumes one; a tree changes
   the identity model (a row would carry a path of stations).
