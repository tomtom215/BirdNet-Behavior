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
6. **Auditable**: Full `MigrationReport` with counts, failures, duration returned to the user

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
└── birdnet_pi/
    ├── mod.rs               # Public entry points
    ├── validator.rs         # Required + advisory integrity checks
    ├── importer.rs          # Batch transactional insert
    ├── csv_importer.rs      # BirdDB.txt CSV importer
    ├── detector.rs          # Schema detector
    └── species_report.rs    # Pre- and post-migration species report
```

### Migrator Trait

```rust
pub trait Migrator {
    type Source;
    type Report;
    type SchemaInfo;

    /// Validate source and return schema summary + species report + migration preview
    fn validate_source(
        &self,
        source: &Self::Source,
    ) -> Result<(Self::SchemaInfo, Self::Report, MigrationReport), MigrateError>;

    /// Execute the migration; returns final MigrationReport
    fn migrate(
        &self,
        source: &Self::Source,
        target: &rusqlite::Connection,
    ) -> Result<MigrationReport, MigrateError>;
}
```

### BirdNET-Pi Migrator

`BirdNetPiMigrator` implements `Migrator` for the BirdNET-Pi SQLite database format:

```rust
pub struct BirdNetPiMigrator;

impl Migrator for BirdNetPiMigrator {
    type Source = Path;             // Path to source .db file
    type Report = SpeciesReport;    // Per-species detection summary
    type SchemaInfo = SchemaInfo;   // Schema version, table names, column list
    // ...
}
```

### MigrationReport

```rust
pub struct MigrationReport {
    pub source_path: PathBuf,
    pub total_rows_read: u64,
    pub rows_imported: u64,
    pub rows_skipped: u64,
    pub rows_failed: u64,
    pub duration_ms: u64,
    pub errors: Vec<String>,
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

Detection rows are inserted into the target database using upsert logic:

```sql
INSERT OR IGNORE INTO detections
    (Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Cutoff, Week, Sens, Overlap, File_Name)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);
```

`INSERT OR IGNORE` makes the operation idempotent — re-running migration is safe.

### Step 4: Report

The `MigrationReport` is returned and displayed: rows read, imported, skipped,
failed, duration, and any error messages.

## Web UI Workflow

The migration UI is accessible at `/admin/migrate`:

```
1. Upload: "Choose .db file" or "Enter file path on Pi"
   └── POST /admin/migrate/upload (multipart form)

2. Preview: Show SpeciesReport and SchemaInfo
   └── GET /admin/migrate/preview (HTMX partial)
   ├── Total detections: 847,293
   ├── Unique species: 142
   ├── Date range: 2022-04-01 → 2026-03-13
   └── Top species: American Robin (12,445), ...

3. Confirm: "Import N detections from 142 species"
   └── POST /admin/migrate/confirm (starts async import)

4. Progress: Live progress via SSE
   └── GET /admin/migrate/stream (SSE: { percent, current, total })

5. Result: MigrationReport with success/failure counts
```

## Validation & Safety

### Path Safety

When uploading, the server validates:
- File extension must be `.db`
- File name must not contain `..`, `/`, `\`
- Canonical path must be within the allowed upload directory

### Source Integrity

Before migration:
```sql
PRAGMA integrity_check;   -- must return 'ok'
PRAGMA quick_check;       -- fast pre-flight
```

If the source database is corrupt, migration is refused with a clear error message.

### Atomicity

The import runs inside a transaction:

```rust
conn.execute_batch("BEGIN IMMEDIATE")?;
// ... batch insert all rows ...
conn.execute_batch("COMMIT")?;
// On error: ROLLBACK
```

This ensures the target database is never left in a partial state.

## Schema Compatibility

| Aspect | Compatibility |
|--------|--------------|
| Detection table schema | ✅ Identical columns — no transformation needed |
| `birdnet.conf` format | ✅ INI parser handles PHP-style quoted values |
| API endpoint paths | ✅ Same paths as BirdNET-Pi FastAPI |
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
2. If migration fails, target database is rolled back to its pre-import state
3. Original BirdNET-Pi can be restarted immediately: `systemctl start birdnet_analysis birdnet_web`
4. Both installations use independent SQLite files

No data loss is possible from a failed or interrupted migration.

---

[← Deployment](10-deployment.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Risks →](12-risks.md)
