# Database Architecture

> Dual-database design: SQLite for operations, DuckDB for analytics.

## Dual-Database Architecture

```
SQLite (OLTP)                    DuckDB (OLAP)
─────────────                    ──────────────
Real-time writes                 Analytical queries
Detection inserts                Trend analysis
Live detection feed              Species aggregations
Web API read queries             Confidence distributions
Small, fast, embedded            Columnar, vectorized
WAL for crash safety             Append-only analysis
```

## SQLite (Operational Database)

**Status: Fully implemented** in `crates/birdnet-db/`

### Connection Management

- WAL mode enforced on every connection
- PRAGMAs: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`,
  `cache_size=-2000` (2MB), `foreign_keys=ON`
- Single connection wrapped in `Arc<Mutex<Connection>>`
- No connection pool needed for embedded single-binary use

### Schema

```sql
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

-- Performance indexes
CREATE INDEX idx_detections_date ON detections(Date);
CREATE INDEX idx_detections_com_name ON detections(Com_Name);
CREATE INDEX idx_detections_sci_name ON detections(Sci_Name);
CREATE INDEX idx_detections_confidence ON detections(Confidence);
CREATE INDEX idx_detections_datetime ON detections(Date, Time);
```

### Migration Framework

Sequential migration system with `schema_version` tracking table:
- Version 1: Create detections table
- Version 2: Add performance indexes
- Version 3: Add composite datetime index

Migrations are idempotent and run automatically on startup.

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

## DuckDB (Analytics Database)

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

### Analytics Endpoints

```
GET /api/v2/analytics/trends
  → Detections per hour/day/week with moving averages

GET /api/v2/analytics/species-activity
  → Activity heatmap (species × hour-of-day)

GET /api/v2/analytics/confidence-distribution
  → Histogram of confidence scores by species

GET /api/v2/analytics/seasonal-patterns
  → Species arrival/departure dates across years

GET /api/v2/analytics/site-comparison
  → Multi-station comparison (for fleet deployments)
```

## Cross-Compilation Notes

- **rusqlite** with `bundled` feature: bundles SQLite C source, compiles anywhere
- **DuckDB**: Needs C++ cross-toolchain; custom `cross` Docker image needed
- Alternative: DuckDB queries run server-side only, so native compilation on Pi is viable

---

[← ML Inference](06-ml-inference.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Behavioral Analytics →](08-behavioral-analytics.md)
