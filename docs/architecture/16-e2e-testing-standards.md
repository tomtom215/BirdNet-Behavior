# End-to-end testing standards

> The standard a change must meet before it is considered "done" in this
> repository — derived from real bug investigations that **819 unit tests
> failed to catch**.

## Why this document exists

In one session of focused end-to-end testing we found six production bugs
that the existing 819-test suite all happily passed against:

1. **`_watcher` dropped immediately on daemon start** — daemon exited
   in milliseconds because the `notify` watcher was bound to a local
   that the spawned thread didn't capture.
2. **`INSERT INTO detections VALUES (?1..?12)` broke** when migration 7
   added a 13th column. Every detection insert errored.
3. **`DetectionRecord.lat: &'a str = ""`** stored TEXT in a REAL
   column. Every typed read of the column returned HTTP 500.
4. **3.0 s chunk size** with the V3.0 dynamic-shape model halved the
   per-species confidence (52 % vs 72 % on the bundled Magpie WAV).
5. **`UNIQUE(Date, Time, Sci_Name)`** collapsed every chunk of one
   recording into a single row. Stations only saw the first chunk's
   confidence — usually the lowest.
6. **Audio-clip extraction computed `start > stop`** ("invalid sample
   range: 1224000..720000") whenever a detection sat past the
   operator-configured `recording_length`, silently dropping the clip.

Each bug looked locally correct in code review. Each bug had unit-test
coverage that exercised the *function* but not its *integration* with the
real model, real audio, real database schema, and real file watcher.

The rest of this document codifies how we'll catch the seventh bug.

---

## The four-layer testing standard

A non-trivial change touching audio, inference, or persistence must
demonstrate that it works at **all four** layers before it ships.

### Layer 1 — Unit tests (necessary, not sufficient)

- Cover the failure-mode space, not just the happy path.
- Mock-free where practical. Prefer in-memory databases and temp dirs
  over mocks.
- Property-based tests (`proptest`) for any pure function whose input
  domain is well-defined (range checks, parsers, serialisers).
- Mutation tests (`cargo mutants`) for any module on the
  configuration-validation path.

### Layer 2 — Reference-implementation parity

When the code talks to an external model, format, or protocol, run the
**reference implementation** on the same input and assert the outputs
agree within a tight tolerance.

For ML inference, that means:

```bash
python3 -m birdnet_analyzer <fixture.wav>   # V2.4 reference
python3 reference_v3.py     <fixture.wav>   # V3.0 reference
cargo test --test inference_e2e             # our pipeline
```

If our pipeline is more than ~0.5 percentage points off the reference on
any species, **the pipeline is wrong** — investigate before moving on.

The V3.0 chunk-length bug (#4 above) surfaced exactly this way: Python
ONNX Runtime on the same model returned 52 % at 3 s chunks and 72 % at
4.5 s chunks. Our Rust pipeline at 3 s returned 52 %. The Rust pipeline
was correct *for 3 s chunks* — the bug was that we were picking 3 s.
Reference parity made the diagnosis a 30-minute job, not a multi-day
hunt.

### Layer 3 — Subprocess integration tests

The binary, not just the library. Cargo populates
`CARGO_BIN_EXE_birdnet-behavior` for tests under `tests/`. Use it.

The existing `tests/doctor_smoke.rs` is the template. Build the actual
release binary, spawn it as a subprocess, drop a fixture in the watch
directory, poll the REST API, assert on the database. This is the layer
that caught the watcher-drop bug — every unit test passed and the daemon
still died in milliseconds.

### Layer 4 — Live end-to-end with the real model

Once per substantial change to the audio path, inference path, or
persistence path:

```bash
# 1. Fetch the real model (Zenodo or installer)
./install.sh                  # bare metal
docker compose up -d          # Docker

# 2. Drop the canonical fixture
cp tests/testdata/Pica_pica_30s.wav \
   /var/lib/birdnet/recordings/2026-05-19-birdnet-09:00:00.wav

# 3. Verify against the REST API + the DB directly
curl -sf http://localhost:8502/api/v2/detections | jq '.detections[].sci_name' | sort -u
sqlite3 /var/lib/birdnet/birds.db \
    "SELECT Sci_Name, Com_Name, ROUND(Confidence*100,1), chunk_offset_secs
       FROM detections WHERE Sci_Name LIKE 'Pica%' ORDER BY chunk_offset_secs;"

# 4. Watch the daemon log for warnings
grep -iE "(failed|error|warn)" /var/log/birdnet/daemon.log | head -20
```

If the DB shows the expected confidence and **zero** WARN-level inserts
or extraction errors, the change is verified.

---

## Filename-format trap

The detection daemon parses **date and time from the source-file name**
and refuses anything that doesn't match
`YYYY-MM-DD-birdnet[-rtsp_id]-HH:MM:SS.{wav,flac,mp3}`. The
`Pica_pica_30s.wav` fixture in the repo will be ignored unless you
rename it to a canonical name in your test harness:

```bash
cp tests/testdata/Pica_pica_30s.wav \
   /var/lib/birdnet/recordings/2026-05-19-birdnet-09:00:00.wav
```

A test that drops the file under its repository name will silently see
zero detections — *not because inference broke*, but because the file
was never opened.

---

## Anti-patterns this standard exists to prevent

### `INSERT INTO X VALUES (?1, ..., ?N)` — no column list

Schema additions break the insert silently. **Always** name the columns.

```sql
-- Wrong: positional VALUES, future-schema-additions break.
INSERT INTO detections VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12);

-- Right: explicit column list, additions are forward-compatible.
INSERT INTO detections
    (Date, Time, Sci_Name, Com_Name, Confidence,
     Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, chunk_offset_secs)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13);
```

### Empty string passed for a numeric column

SQLite stores `""` as TEXT in any column, including ones declared
`REAL` or `INTEGER`. Every subsequent typed read then fails with
*"Invalid column type Text at index N"*. **Use `Option<f64>` /
`Option<i64>` in the record struct** and pass `None` when the value is
unknown — SQLite will store `NULL` and the typed reader will return
`None` cleanly.

### Hand-coded schema in test fixtures duplicating the migration

`open_or_create()` used to declare its own `CREATE TABLE detections`
with only the migration-1 columns. Tests ran against an old schema and
gave a false-green every time the migration list grew. **There must be
exactly one declaration of every schema.** The migration list owns
it; everything else applies migrations.

### Local binding with `_` prefix that the closure should capture

```rust
// Wrong: _watcher gets dropped when the function returns, killing the
// notify backend, before the spawned thread has a chance to use file_rx.
let (_watcher, file_rx) = watch_directory(&dir)?;
thread::spawn(move || { /* uses file_rx, not _watcher */ });
```

```rust
// Right: shadow the watcher inside the closure so move-capture keeps it
// alive for the thread's lifetime.
let (file_watcher, file_rx) = watch_directory(&dir)?;
thread::spawn(move || {
    let _watcher = file_watcher;
    // ... use file_rx ...
});
```

### Range arithmetic without clamping to the actual buffer

```rust
// Wrong: safe_stop clamps to *configured* recording_length, which can be
// smaller than the *actual* audio. safe_start can then end up past
// safe_stop and the slice fails with "invalid sample range".
let safe_stop = (detection.stop + spacer).min(self.config.recording_length);
let clip = &audio.samples[start_sample..stop_sample];
```

```rust
// Right: clamp to actual decoded length, and re-verify the invariant at
// the slice boundary so a wrong arithmetic step can't reach unsafe code.
let actual = audio.samples.len() as f32 / audio.sample_rate as f32;
let safe_stop = (detection.stop + spacer).max(safe_start).min(actual);
if start_sample >= stop_sample { return Err(...); }
let clip = &audio.samples[start_sample..stop_sample];
```

### `UNIQUE(Date, Time, Sci_Name)` on chunked data

The filename's `Time` is constant across all chunks of one recording,
so the key collapses every chunk to one row. **Include a chunk
disambiguator** (`chunk_offset_secs`) in any UNIQUE constraint over a
table that holds chunked data.

### Auto-adjustment that doesn't include all the related parameters

The daemon auto-adjusts `sample_rate` and `raw_audio_input` when the
model differs from defaults but used to leave `chunk_duration_secs`
alone. Three of those parameters define one contract — the model's
input window — so they all auto-adjust together or none do.

---

## Pre-merge checklist

Before merging a change that touches audio, inference, or persistence:

```bash
# Static gates
cargo fmt --check --all
cargo clippy --workspace --all-targets -- -D warnings

# Test gates
cargo test --workspace --tests
cargo mutants --package birdnet-core --file crates/birdnet-core/src/config/validate.rs
cargo llvm-cov --workspace --summary-only

# Live gate — only required for audio / inference / persistence changes
./target/release/birdnet-behavior --doctor
# 1. Start daemon against the real model
# 2. Drop tests/testdata/Pica_pica_30s.wav (correctly named) into the watch dir
# 3. Confirm in the DB that ALL chunks show up with sensible chunk_offset_secs,
#    that the top Pica pica row's confidence is ≥ 0.70, and that zero
#    "invalid sample range" or "Invalid column type" entries appear in the
#    daemon log.
```

If a step fails or skips, **do not merge**. Open an issue if the failure
seems unrelated, but the bug almost certainly is related — most of the
six fixes in this branch were found by exactly this sequence.

---

## See also

- [`docs/architecture/15-model-chunking.md`](15-model-chunking.md) —
  the chunk-length investigation that drove this document
- [`tests/doctor_smoke.rs`](../../tests/doctor_smoke.rs) — the binary
  subprocess test template
- [`tests/inference_e2e.rs`](../../tests/inference_e2e.rs) —
  reference-parity inference test
- [`docs/FIELD_DEPLOYMENT.md`](../FIELD_DEPLOYMENT.md) — operator
  perspective on the same verification surface
