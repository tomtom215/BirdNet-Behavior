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

Sequential migration system with `schema_version` tracking table:
- **Version 1**: Create detections table
- **Version 2**: Add performance indexes
- **Version 3**: Add composite datetime index
- **Version 4**: Create settings key-value table

Migrations are idempotent and run automatically on startup.

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

### Co-occurrence SQL Fix

The self-join pattern for co-occurrence naturally double-counts:

```sql
-- Self-join generates 2 rows per pair per date (A→B and B→A)
-- After canonicalization to (min, max) species, both land in same GROUP BY bucket
-- COUNT(*) = 2×days; COUNT(DISTINCT a.Date) = correct days

WITH daily AS (
    SELECT DISTINCT Date, Com_Name FROM detections
),
pairs AS (
    SELECT
        MIN(a.Com_Name, b.Com_Name) AS species_a,
        MAX(a.Com_Name, b.Com_Name) AS species_b,
        COUNT(DISTINCT a.Date) AS shared_days   -- ← not COUNT(*)
    FROM daily a
    JOIN daily b ON a.Date = b.Date AND a.Com_Name != b.Com_Name
    GROUP BY species_a, species_b
)
SELECT * FROM pairs ORDER BY shared_days DESC;
```

## DuckDB (Analytics Database)

**Status: ✅ Queries implemented** in `crates/birdnet-db/src/duckdb/`

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

### Analytics Queries Implemented

| Query | Module | Description |
|-------|--------|-------------|
| Activity heatmap | `duckdb/queries/heatmap.rs` | Hour × day-of-week SVG heat map |
| Daily trends | `duckdb/queries/trends.rs` | Detections per day with 7-day moving average |
| Species correlation | `sqlite/queries/correlation.rs` | Co-occurrence shared-day count |
| Top species by period | `duckdb/queries/species.rs` | Ranked species for week/month/year |
| Confidence distribution | `duckdb/queries/confidence.rs` | Histogram by species |
| Seasonal patterns | `duckdb/queries/seasonal.rs` | Month-by-month species activity |

### Analytics API Endpoints

```
GET /api/v2/analytics/trends         → daily count + 7-day MA
GET /api/v2/analytics/heatmap        → hour×weekday SVG (or JSON data)
GET /api/v2/analytics/top-species    → species ranked by period
GET /api/v2/analytics/confidence     → confidence histogram
GET /api/v2/analytics/correlation    → species co-occurrence matrix
GET /api/v2/analytics/seasonal       → month×species activity grid
```

## Cross-Compilation Notes

- **rusqlite** with `bundled` feature: bundles SQLite C source, compiles anywhere
- **DuckDB**: Needs C++ cross-toolchain; custom `cross` Docker image needed
- Alternative: DuckDB queries run server-side only, so native compilation on Pi is viable

---

*Last updated: 2026-03-14*

[← ML Inference](06-ml-inference.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Behavioral Analytics →](08-behavioral-analytics.md)
