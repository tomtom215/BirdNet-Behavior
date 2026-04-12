# Behavioral Analytics

> Applying duckdb-behavioral extension to bird detection data for ecological insights.

## Table of Contents

- [Concept](#concept)
- [Currently Implemented Analytics](#currently-implemented-analytics)
- [duckdb-behavioral Functions](#duckdb-behavioral-functions)
- [Implementation Status](#implementation-status)
- [API Endpoints](#api-endpoints)
- [Web UI Visualizations](#web-ui-visualizations)
- [Data Preparation](#data-preparation)

---

## Concept

[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral)
provides ClickHouse-inspired behavioral analytics functions. Applied to
bird detections, these reveal ecological patterns that simple aggregation
queries cannot capture.

The `birdnet-behavioral` crate provides the types, SQL builders, and
DuckDB connection helpers for the behavioral analytics layer. Queries
execute against a DuckDB file that is periodically synced from the
operational SQLite database.

## Currently Implemented Analytics

These are live and served by the web UI:

### Activity Heatmap (✅ Implemented)

SVG hour-of-day × day-of-week heatmap showing when birds are most active:

```
          Mon  Tue  Wed  Thu  Fri  Sat  Sun
05:00   [ 12][ 10][ 15][ 20][ 18][ 30][ 35]
06:00   [ 45][ 50][ 60][ 55][ 48][ 80][ 90]
07:00   [ 30][ 35][ 40][ 38][ 32][ 55][ 65]
...
```

Route: `GET /pages/heatmap` — full HTMX page with species filter
The SVG is generated server-side in `crates/birdnet-web/src/routes/pages/heatmap.rs`.

### Species Co-occurrence (✅ Implemented)

Which species appear together on the same days most often:

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
SELECT * FROM pairs ORDER BY shared_days DESC LIMIT 20;
```

### Daily Trends with Moving Average (✅ Implemented)

`birdnet-timeseries` computes 7-day rolling averages over detection counts:

```rust
pub fn rolling_mean(data: &[(Date, f64)], window: usize) -> Vec<(Date, f64)>;
pub fn detect_trend(data: &[(Date, f64)]) -> TrendDirection;
```

### Seasonal Patterns (✅ Implemented)

Month-by-month species activity grid showing peak months per species.

## duckdb-behavioral Functions

Types, SQL builders, and API endpoints are implemented in
`birdnet-behavioral`; the extension is loaded at startup when the
`analytics` feature is enabled.

| Function | Bird Behavior Use |
|----------|------------------|
| `sessionize` | Group continuous bird activity into sessions |
| `retention` | Track species return patterns (resident vs. migrant) |
| `window_funnel` | Analyse dawn chorus ordering and sequences |
| `sequence_next_node` | Predict which species follows a detected bird |
| `sequence_match` / `sequence_count` | Match and count specific activity patterns |

### 1. Activity Sessionization

Group continuous bird activity into sessions (gap > 30 minutes = new session):

```sql
LOAD behavioral;

SELECT
    Com_Name,
    sessionize(detection_timestamp, INTERVAL '30 MINUTE')
        OVER (PARTITION BY Sci_Name ORDER BY detection_timestamp)
        AS session_id,
    COUNT(*) as detections_in_session,
    MIN(detection_timestamp) as session_start,
    MAX(detection_timestamp) as session_end
FROM detections_ts
GROUP BY Com_Name, session_id
ORDER BY session_start DESC;
```

**Use case:** Distinguish dawn chorus (50 detections in 30 minutes) from
territorial calls (3 detections over 5 minutes).

### 2. Species Retention

Which species keep coming back day after day?

```sql
SELECT
    Com_Name,
    retention(detection_date, [1, 2, 3, 7, 14, 30]) AS retention_rates
FROM (
    SELECT DISTINCT Com_Name, CAST(Date AS DATE) AS detection_date
    FROM detections
)
GROUP BY Com_Name
ORDER BY retention_rates[1] DESC;
```

**Use case:** Classify species as residents (high 30-day retention), migrants
(appear for days then gone), or rarities (single-day events).

### 3. Dawn Chorus Funnel

Do species follow a predictable sequence at dawn?

```sql
SELECT window_funnel(
    INTERVAL '2 HOUR',
    detection_timestamp,
    [
        Com_Name = 'European Robin',
        Com_Name = 'Eurasian Blackbird',
        Com_Name = 'Song Thrush',
        Com_Name = 'Eurasian Wren',
        Com_Name = 'Great Tit'
    ]
) AS dawn_chorus_stage
FROM detections_ts
WHERE EXTRACT(HOUR FROM detection_timestamp) BETWEEN 4 AND 8
GROUP BY CAST(detection_timestamp AS DATE);
```

### 4. Next Species Prediction

After detecting a Robin, what typically follows?

```sql
SELECT sequence_next_node(
    detection_timestamp,
    INTERVAL '1 HOUR',
    Com_Name = 'European Robin',
    1,
    'strict'
) AS next_species,
COUNT(*) as frequency
FROM detections_ts
GROUP BY next_species
ORDER BY frequency DESC
LIMIT 10;
```

**Use case:** "What to expect next" prediction feature for the web UI.

## API Endpoints

```
GET /api/v2/analytics/trends           → daily count with rolling window
GET /api/v2/analytics/heatmap          → hour × weekday data
GET /api/v2/analytics/top-species      → species ranked by period
GET /api/v2/analytics/correlation      → co-occurrence matrix
GET /api/v2/analytics/seasonal         → month × species activity
GET /api/v2/analytics/sessions         → activity sessionization
GET /api/v2/analytics/retention        → species retention rates
GET /api/v2/analytics/funnel           → dawn chorus funnel
GET /api/v2/analytics/next-species     → "what's coming next" prediction
```

## Web UI

Analytics are surfaced through several HTMX pages:

- `/heatmap` — hour × weekday SVG activity grid
- `/correlation` — species co-occurrence pairs and companion lookup
- `/analytics` — sessions, retention, funnel, and next-species widgets
  (feature-gated)
- `/timeseries` — activity, diversity, trend, peak, gap, and session
  time-series dashboards

## Data Preparation

```sql
-- Timestamp view for behavioral functions
CREATE VIEW detections_ts AS
SELECT *, CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp
FROM detections;
```

---

[← Database](07-database.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Web Server →](09-web-server.md)
