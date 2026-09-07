# Migration from BirdNET-Pi

> Zero-downtime, non-destructive import of existing BirdNET-Pi data.

## Table of Contents

- [Design Goals](#design-goals)
- [birdnet-migrate Crate](#birdnet-migrate-crate)
- [Migration Process](#migration-process)
- [Web UI Workflow](#web-ui-workflow)
- [Validation & Safety](#validation--safety)
- [Schema Compatibility](#schema-compatibility)
- [Migration Report](#migration-report)
- [Rollback Plan](#rollback-plan)

---

## Design Goals

1. **Non-destructive**: The source BirdNET-Pi installation is never modified
2. **Zero-downtime**: Migration can run while BirdNET-Pi is still active (SQLite WAL allows concurrent reads)
3. **Validated**: Schema is checked before import; per-species reports show what will be imported
4. **Simple**: Users upload a `.db` file via the web UI or point to a path; we handle the rest
5. **Deterministic**: Same input always produces the same output; re-running is safe (upsert logic)
6. **Auditable**: a `MigrationSummary` with rows read / imported / skipped, the schema name and the source path is returned to the user

## birdnet-migrate Crate

Implemented in `crates/birdnet-migrate/`.

### Module Structure

```
birdnet-migrate/src/
├── lib.rs                   # Public API
├── traits.rs                # Migrator / Validator / SchemaDetector traits
├── error.rs                 # MigrateError type
├── schema.rs                # Schema detection (SQLite and CSV)
├── progress.rs              # Thread-safe progress handle
├── provenance.rs            # Where an imported history came from vs. where the station stands
└── birdnet_pi/
    ├── mod.rs               # Public entry points
    ├── validator.rs         # Required + advisory integrity checks
    ├── importer.rs          # Batch transactional insert
    ├── csv_importer.rs      # BirdDB.txt CSV importer
    ├── detector.rs          # Schema detector
    └── species_report.rs    # Pre- and post-migration species report
```

### The three traits

Detection, validation and import are three separate one-job traits, not
associated types on a single trait:

```rust
/// Detects whether a SQLite file uses a known source schema.
pub trait SchemaDetector: Send + Sync {
    fn detect(&self, path: &Path) -> Result<DetectedSchema, MigrateError>;
}

/// Validates a source database before or after migration.
pub trait Validator: Send + Sync {
    fn validate_source(&self, source_path: &Path) -> Result<ValidationReport, MigrateError>;
    fn validate_destination(&self, source_path: &Path, dest_path: &Path)
        -> Result<ValidationReport, MigrateError>;
}

/// Imports data from a source database into the destination.
pub trait Migrator: Send + Sync {
    fn migrate(&self, source_path: &Path, dest_path: &Path, progress: &ProgressHandle)
        -> Result<MigrationSummary, MigrateError>;
}
```

### BirdNET-Pi Importer

`BirdNetPiImporter` (`birdnet_pi/importer.rs`) implements `Migrator` for the
BirdNET-Pi SQLite database format; `birdnet_pi/mod.rs` exposes
`run_migration` / `run_migration_with_options` on top of it:

```rust
pub struct BirdNetPiImporter;

impl Migrator for BirdNetPiImporter {
    fn migrate(&self, source_path: &Path, dest_path: &Path, progress: &ProgressHandle)
        -> Result<MigrationSummary, MigrateError> { /* … */ }
}
```

The per-species preview shown before import is a separate `MigrationReport`
in `birdnet_pi/species_report.rs` (total rows, unique species, date range,
top species, and the null-date / invalid-confidence / duplicate counts).

### MigrationSummary

```rust
pub struct MigrationSummary {
    pub source_rows: u64,
    pub imported_rows: u64,
    pub skipped_rows: u64,
    pub schema_name: String,
    pub source_path: String,
}
```

## Migration Process

### Step 1: Schema Validation

The migrator opens the source SQLite file and checks:
- `detections` table exists
- Required columns present: `Date`, `Time`, `Sci_Name`, `Com_Name`, `Confidence`
- Optional columns detected: `Lat`, `Lon`, `Cutoff`, `Week`, `Sens`, `Overlap`, `File_Name`

### Step 2: Source Report

A `SpeciesReport` is generated from the source database:
- Total detections by species
- Date range (first seen, last seen)
- Average confidence per species
- Top 20 species by count

This is displayed to the user before they confirm the import.

### Step 3: Import

Detection rows are inserted into the target database naming the
uniqueness conflict explicitly:

```sql
INSERT INTO detections
    (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name, …)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, …)
ON CONFLICT(Date, Time, Sci_Name, COALESCE(File_Name, ''), chunk_offset_secs) DO NOTHING;
```

`ON CONFLICT … DO NOTHING` absorbs only the duplicate-key case, which is
what makes a re-run idempotent. `INSERT OR IGNORE` is banned repo-wide
(`tests/or_ignore_guard.rs`) because it would also swallow a `NOT NULL` or
`CHECK` failure and report the row as a duplicate.

### Step 4: Report

A `MigrationSummary` is returned and displayed: `source_rows`,
`imported_rows`, `skipped_rows`, the detected `schema_name` and the
`source_path`. There is no per-row failure list — a row that cannot be
inserted fails the batch.

## Web UI Workflow

The migration UI is accessible at `/admin/migrate`:

```
1. Upload: "Choose .db file" or "Enter file path on Pi"
   └── POST /admin/migrate/upload (multipart form)

2. Preview: Show SpeciesReport and SchemaInfo
   └── POST /admin/migrate/validate (HTMX partial)
   ├── Total detections: 847,293
   ├── Unique species: 142
   ├── Date range: 2022-04-01 → 2026-03-13
   └── Top species: American Robin (12,445), ...

3. Confirm: "Import N detections from 142 species"
   └── POST /admin/migrate/upload/confirm, then POST /admin/migrate/run

4. Progress: polled by HTMX, not streamed
   └── GET /admin/migrate/progress

5. Result: MigrationSummary with rows read / imported / skipped
```

## Validation & Safety

### Path Safety

When uploading, the server validates:
- File extension must be `.db`
- File name must not contain `..`, `/`, `\`
- Canonical path must be within the allowed upload directory

### Source Integrity

The source is opened read-only (`SQLITE_OPEN_READ_ONLY`, `schema.rs`) and its
`detections` table is checked with `PRAGMA table_info` for the required
columns (`date`, `time`, `sci_name`, `com_name`, `confidence`, `lat`, `lon`,
`cutoff`, `week`, `sens`, …, case-insensitive). There is **no**
`PRAGMA integrity_check` / `quick_check` pre-flight in `birdnet-migrate`; a
corrupt source surfaces as a read error during validation or import.

### Atomicity

Each batch is inserted inside its own transaction
(`insert_batch_tagged` / `insert_batch` in `importer.rs`):

```rust
let tx = conn.transaction()?;
// ... insert this batch's rows with ON CONFLICT … DO NOTHING ...
tx.commit()?;
// On error the batch rolls back; earlier batches stay committed
```

A failure part-way therefore leaves the rows of completed batches in place.
Because the insert names its conflict target, re-running the import skips
those rows and continues, so the end state is the same as an uninterrupted
run.

## Schema Compatibility

| Aspect | Compatibility |
|--------|--------------|
| Detection table schema | ✅ Identical columns — no transformation needed |
| `birdnet.conf` format | ✅ INI parser handles PHP-style quoted values |
| API endpoint paths | ⚠️ Not preserved — this server exposes `/api/v2/*`, which BirdNET-Pi does not have |
| BirdDB.txt CSV format | ✅ Same format |
| Settings | ✅ Re-entered via web UI (config values imported from birdnet.conf) |
| Recording files | ⚠️ Not migrated (files stay at original path; paths stored in DB) |

## Migration Report

Example report shown to user after successful migration:

```
✅ Migration Complete

Source:       /home/pi/BirdSongs/BirdDB.db
Duration:     4.2 seconds
Rows read:    847,293
Imported:     847,293
Skipped:      0
Failed:       0

Top imported species:
  American Robin        12,445 detections
  Song Sparrow           8,892 detections
  House Finch            7,201 detections
  ...

Date range: 2022-04-01 → 2026-03-13
```

## Rollback Plan

At any phase, the original BirdNET-Pi installation is unchanged:

1. Migration reads the source database **read-only** — never writes to it
2. If migration fails, the in-flight batch is rolled back; completed batches stay and a re-run is idempotent
3. Original BirdNET-Pi can be restarted immediately: `systemctl start birdnet_analysis birdnet_web`
4. Both installations use independent SQLite files

No data loss is possible from a failed or interrupted migration.

---

[← Deployment](10-deployment.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Risks →](12-risks.md)
