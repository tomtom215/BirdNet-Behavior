# Behavioral Analytics

> Applying duckdb-behavioral extension to bird detection data for ecological insights.

## Concept

[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) provides
ClickHouse-inspired behavioral analytics functions. Applied to bird detections,
these reveal ecological patterns invisible to simple aggregation queries.

**Status: Types and SQL builders implemented** in `crates/birdnet-behavioral/`

## Available Functions

| Function | Bird Behavior Use |
|----------|------------------|
| `sessionize` | Group continuous bird activity into sessions |
| `retention` | Track species return patterns (resident vs. migrant) |
| `window_funnel` | Analyze dawn chorus ordering and sequences |
| `sequence_match` | Find days matching specific bird activity patterns |
| `sequence_count` | Count pattern occurrences over time |
| `sequence_next_node` | Predict which species follows a detected bird |

## Queries

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

**Use case:** Validate well-known dawn chorus ordering. How many steps of the
expected sequence actually occur each morning?

### 4. Sequence Pattern Matching

Find days with specific activity patterns:

```sql
SELECT
    CAST(detection_timestamp AS DATE) AS detection_date,
    sequence_match(
        '(?1).*(?2).*(?3)',
        detection_timestamp,
        [
            Com_Name = 'European Robin',
            Com_Name = 'Eurasian Blackbird',
            Com_Name = 'Song Thrush'
        ]
    ) AS pattern_matched
FROM detections_ts
GROUP BY detection_date
HAVING pattern_matched = true;
```

**Use case:** Ecological research -- do certain species always appear in sequence?

### 5. Next Species Prediction

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
GET /api/v2/analytics/sessions?species=...&gap=30m
GET /api/v2/analytics/retention?species=...&periods=1,7,30
GET /api/v2/analytics/funnel?sequence=Robin,Blackbird,Thrush&window=2h
GET /api/v2/analytics/patterns?regex=(?1).*(?2)&conditions=...
GET /api/v2/analytics/next-species?after=Robin&window=1h
```

## Web UI Visualizations

- Activity session timeline
- Species retention heatmap
- Dawn chorus funnel chart
- "What's coming next?" prediction widget

## Data Preparation

```sql
-- Timestamp view for behavioral functions
CREATE VIEW detections_ts AS
SELECT *, CAST(Date || ' ' || Time AS TIMESTAMP) AS detection_timestamp
FROM detections;
```

## Implementation Status

| Component | Status |
|-----------|--------|
| Result types (`ActivitySession`, `SpeciesRetention`, etc.) | Complete |
| Parameter types with defaults | Complete |
| Residency classification logic | Complete |
| SQL builder functions | Complete |
| DuckDB connection and execution | Not started |
| API endpoint handlers (actual queries) | Placeholder only |
| Extension loading/bundling | Not started |

---

[← Database](07-database.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Web Server →](09-web-server.md)
