# Database Architecture

> Dual-database design: SQLite for operations, DuckDB for analytics.

## Table of Contents

- [Dual-Database Architecture](#dual-database-architecture)
- [SQLite (Operational Database)](#sqlite-operational-database)
- [DuckDB (Analytics Database)](#duckdb-analytics-database)
- [Cross-Compilation Notes](#cross-compilation-notes)

---

## Dual-Database Architecture

```
SQLite (OLTP)                    DuckDB (OLAP)
─────────────                    ──────────────
Real-time writes                 Analytical queries
Detection inserts                Trend analysis
Settings storage                 Species aggregations
Live detection feed              Confidence distributions
Web API read queries             Heatmap / co-occurrence
Small, fast, embedded            Columnar, vectorized
WAL for crash safety             Append-only analysis
```

## SQLite (Operational Database)

**Status: ✅ Fully implemented** in `crates/birdnet-db/src/sqlite/`

### Connection Management

- WAL mode enforced on every connection
- PRAGMAs: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`,
  `cache_size=-2000` (2 MB), `foreign_keys=ON`
- Single connection wrapped in `Arc<Mutex<Connection>>`
- No connection pool needed for embedded single-binary use

### Schema

```sql
-- Detections (migration v1)
CREATE TABLE IF NOT EXISTS detections (
    Date     TEXT NOT NULL,
    Time     TEXT NOT NULL,
    Sci_Name TEXT NOT NULL,
    Com_Name TEXT NOT NULL,
    Confidence REAL NOT NULL,
    Lat      REAL,
    Lon      REAL,
    Cutoff   REAL,
    Week     INTEGER,
    Sens     REAL,
    Overlap  REAL,
    File_Name TEXT
);

-- Settings key-value store (migration v4)
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

-- Performance indexes (migrations v2-v3)
CREATE INDEX idx_detections_date        ON detections(Date);
CREATE INDEX idx_detections_com_name    ON detections(Com_Name);
CREATE INDEX idx_detections_sci_name    ON detections(Sci_Name);
CREATE INDEX idx_detections_confidence  ON detections(Confidence);
CREATE INDEX idx_detections_datetime    ON detections(Date, Time);
```

### Migration Framework

Sequential migration system tracked in a `schema_version` table. Ten
versioned migrations are shipped in `crates/birdnet-db/src/migration.rs`
and cover the core `detections` table, composite indexes, the settings
key-value store, notification log, alert rules, species thresholds, and
the rare-bird quarantine table. Migrations are idempotent and run
automatically on startup.

### Settings Module

`crates/birdnet-db/src/sqlite/settings.rs` provides:

```rust
pub fn get_or(conn: &Connection, key: &str, default: &str)
    -> Result<String, SettingsError>;

pub fn set(conn: &Connection, key: &str, value: &str)
    -> Result<(), SettingsError>;
```

Settings used across the application:

| Category | Setting Keys |
|----------|-------------|
| Station | `latitude`, `longitude`, `location_name` |
| Audio | `microphone_device`, `recording_length`, `overlap`, `sensitivity` |
| Detection | `minimum_confidence`, `species_occurrence_threshold` |
| BirdWeather | `birdweather_id`, `birdweather_enabled` |
| Email | `email_smtp_host`, `email_smtp_port`, `email_smtp_user`, `email_smtp_pass`, `email_from`, `email_to`, `email_from_name`, `email_starttls`, `email_min_confidence`, `email_cooldown_secs` |
| Apprise | `apprise_enabled`, `apprise_url` |
| System | `backup_count`, `disk_limit_percent` |

### Resilience

- **WAL enforcement**: Crash-safe writes
- **Integrity checking**: `PRAGMA quick_check` and full `integrity_check`
- **Hot backup**: Via rusqlite's `Backup` API (timestamped backups)
- **Backup pruning**: Keep only N most recent (default: 5)
- **Auto-recovery**: On startup, check integrity → restore from backup if corrupt
- **Recovery result**: Reports whether database was healthy or recovered

### Query API

| Function | Purpose |
|----------|---------|
| `insert_detection()` | Write new detection |
| `detection_count()` | Total detection count |
| `species_count()` | Unique species count |
| `detections_by_date()` | All detections for a date |
| `recent_detections()` | Last N detections |
| `top_species()` | Species ranked by count with avg confidence |
| `hourly_activity()` | Detection count by hour |
| `species_on_date()` | All species detected on a given date |
| `confidence_histogram()` | Confidence score distribution |
| `co_occurrence_matrix()` | Species co-occurrence with `COUNT(DISTINCT a.Date)` fix |

### Species co-occurrence

```sql
WITH daily AS (
    SELECT DISTINCT Date, Com_Name FROM detections
),
pairs AS (
    SELECT
        MIN(a.Com_Name, b.Com_Name) AS species_a,
        MAX(a.Com_Name, b.Com_Name) AS species_b,
        COUNT(DISTINCT a.Date) AS shared_days
    FROM daily a
    JOIN daily b ON a.Date = b.Date AND a.Com_Name != b.Com_Name
    GROUP BY species_a, species_b
)
SELECT * FROM pairs ORDER BY shared_days DESC;
```

The canonical `(MIN, MAX)` pair is used to deduplicate the symmetric
self-join, and `COUNT(DISTINCT a.Date)` gives the number of shared days
rather than raw join row count.

## DuckDB (Analytics Database)

DuckDB powers the optional behavioral and time-series analytics layer
(`--features analytics`). Types and query builders live in
`crates/birdnet-behavioral/` and `crates/birdnet-timeseries/`; the
DuckDB connection and sync helpers live in
`crates/birdnet-behavioral/src/connection/`.

### Why DuckDB for Analytics

SQLite handles operational queries well but struggles with analytical workloads
on large datasets:

| Query Type | SQLite | DuckDB |
|------------|--------|--------|
| `SELECT * WHERE Date = today` | Fast | Fast |
| `GROUP BY species ORDER BY COUNT(*)` | Slow at 1M+ rows | Vectorized, instant |
| Window functions over time series | Possible but slow | Native, optimized |
| Confidence distribution histogram | Table scan | Columnar scan |
| Year-over-year comparison | Minutes | Seconds |

### ETL Pipeline

DuckDB can directly attach and query SQLite files:

```sql
-- Attach SQLite for live ETL
ATTACH 'birds.db' AS sqlite_db (TYPE SQLITE);

-- Incremental sync: only new rows since last sync
INSERT INTO detections
SELECT * FROM sqlite_db.detections
WHERE Date > ? OR (Date = ? AND Time > ?);

DETACH sqlite_db;
```

Sync runs periodically (configurable interval, default: every 5 minutes).

### Analytics queries implemented

Analytics queries are split across two crates:

| Query | Module | Description |
|-------|--------|-------------|
| Activity heatmap | `birdnet-db::sqlite::queries::heatmap` | Hour × day-of-week SVG heat map (served from SQLite) |
| Daily trends | `birdnet-timeseries::queries::activity` | Detections per day with rolling window |
| Species correlation | `birdnet-db::sqlite::queries::correlation` | Co-occurrence shared-day count |
| Confidence distribution | `birdnet-db::sqlite::queries::analytics` | Histogram by species |
| Sessionization | `birdnet-behavioral::queries` | Activity sessions with configurable gap |
| Retention / funnel | `birdnet-behavioral::queries` | Resident vs. migrant classification, dawn chorus sequences |
| Phenology timing | `birdnet-behavioral::phenology::timing` | Migration windows, first / last detection, year-over-year trend |
| Weekly abundance | `birdnet-behavioral::phenology::abundance` | Normalised abundance index and peak weeks |

## Cross-Compilation Notes

- **rusqlite** with the `bundled` feature: bundles SQLite C source, compiles anywhere.
- **DuckDB**: bundles DuckDB's C++ source, so a C++ toolchain is required on
  the build host. CI builds Docker images natively on `ubuntu-24.04` and
  `ubuntu-24.04-arm` runners to avoid QEMU emulation; release binaries are
  produced via `cargo-zigbuild` using Zig's universal linker.

---

[← ML Inference](06-ml-inference.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Behavioral Analytics →](08-behavioral-analytics.md)
