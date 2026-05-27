# O-22 · Quality dashboard audit + missing model-trust charts

<!-- BNB:STATUS-HEADER -->
> **Risk:** low · **Priority:** 3 · **Status:** ready for review
> Acceptance: VERIFY.md § O-22 · Rollback: ROLLBACK.md § O-22
<!-- BNB:STATUS-HEADER -->


## What

`admin/quality.rs` is a real, well-architected page (17 KB) that ships five panels: summary, confidence distribution, 30-day trend, hourly profile, and low-confidence species. That's a strong floor. Two issues from a careful read:

1. **The page rolls its own page chrome** instead of using `admin_shell()`. `<style>` block at the top defines `.card`, `.stat-grid`, `.stat-card`, `.stat-value`, `.bar-chart`, `.hour-bars` — every one of those classes is already in `app.css` and the admin shell already includes it. The local definitions duplicate (and slightly drift from) the design-system values. Lift them out, use the shipped classes.

2. **Two "model trust" panels are missing** that this codebase already has the data for:
   - **Review-verdict trend** — `pages/detection_reviews.rs` ships an approve / re-label / reject queue; the resulting verdicts live in the SQL table queried by `birdnet-db::sqlite::queries::detection_reviews`. Joining detections × reviews → a "human disagreement rate over time" panel that turns the quarantine flow into a feedback signal here.
   - **Per-species confidence vs review-verdict** — same join, per species, two stacked horizontal bars: model-judged vs human-judged. Tells the operator *which species the model is confident about that the human keeps overturning*. Today's "low-confidence species" panel surfaces consistently-low-confidence calls; this new panel surfaces consistently-overconfident ones, which are the higher-risk false positives.

Two more panels that *need new schema* are scoped as **follow-up** in this DIFF — listed but not built here, with the schema delta spelled out:

3. Mic-SNR distribution (needs `detections.snr_db` column emitted by the audio daemon).
4. Recording dropout / gap rate (needs `recording_sessions(start, end, source_id)` table populated by the audio daemon).

## Files

| Action | Path |
|---|---|
| Refactor | `crates/birdnet-web/src/routes/admin/quality.rs` — replace inline `<style>` block with design-system class names; route the page through `admin_shell()` instead of its own `<head>` |
| Add | `crates/birdnet-db/src/sqlite/queries/quality.rs` (or add functions to the existing `quality_*` set) — two new queries: `review_verdict_trend(days)` and `model_vs_review_by_species(limit)` |
| Add | `crates/birdnet-web/src/routes/admin/quality.rs::render_review_trend(...)` + `::render_model_vs_review(...)` |
| Append | `crates/birdnet-web/static/css/app.css` — see `css/app.css.append` |

## Query 1 — review-verdict trend (last 30 days)

Joins `detections` × `detection_reviews` (table already exists, see `queries/detection_reviews.rs`).

```sql
WITH per_day AS (
  SELECT d.Date AS day,
         COUNT(*)                                         AS total,
         SUM(CASE WHEN r.verdict = 'approved' THEN 1 END) AS approved,
         SUM(CASE WHEN r.verdict = 'rejected' THEN 1 END) AS rejected,
         SUM(CASE WHEN r.verdict = 'relabeled' THEN 1 END) AS relabeled,
         SUM(CASE WHEN r.verdict IS NULL THEN 1 END)      AS unreviewed
    FROM detections d
    LEFT JOIN detection_reviews r ON r.detection_id = d.id
   WHERE d.Date >= date('now', '-30 days')
   GROUP BY d.Date
)
SELECT day,
       total,
       COALESCE(approved, 0),
       COALESCE(rejected, 0),
       COALESCE(relabeled, 0),
       COALESCE(unreviewed, 0)
  FROM per_day
 ORDER BY day;
```

Rendered as a 30-bar grouped stack: rejected (rare) + relabeled (dawn) + approved (moss) + unreviewed (`--surface-2`). The horizontal hairline at *median reject rate* is the headline number.

## Query 2 — model-vs-review by species

```sql
WITH per_species AS (
  SELECT d.common_name,
         d.scientific_name,
         COUNT(*) AS total,
         AVG(d.confidence) AS model_avg,
         SUM(CASE WHEN r.verdict = 'rejected'  THEN 1 END) AS rejected,
         SUM(CASE WHEN r.verdict = 'approved'  THEN 1 END) AS approved,
         SUM(CASE WHEN r.verdict = 'relabeled' THEN 1 END) AS relabeled
    FROM detections d
    LEFT JOIN detection_reviews r ON r.detection_id = d.id
   GROUP BY d.common_name, d.scientific_name
  HAVING COUNT(*) >= 5 AND SUM(CASE WHEN r.verdict IS NOT NULL THEN 1 END) >= 3
)
SELECT common_name, scientific_name, total,
       model_avg,
       1.0 - CAST(COALESCE(rejected, 0) AS REAL) / NULLIF(COALESCE(rejected, 0) + COALESCE(approved, 0), 0) AS human_avg
  FROM per_species
 ORDER BY (model_avg - human_avg) DESC
 LIMIT 12;
```

The table renders two columns of confidence bars side-by-side: **Model** (`--moss`) vs **Human** (`--moss-ink`). The largest *gaps* (model loud, humans skeptical) bubble to the top. Each row is also a deep link into `/quarantine?species=…` so the operator can act on the signal.

## Schema follow-up (not in this PR)

The audio daemon already writes `confidence` to every detection. To unblock the two remaining panels:

- **`detections.snr_db REAL`** — populated by the audio daemon when a detection fires. Backfilled to `NULL` for legacy rows; histogram panel filters those out.
- **`recording_sessions(id, source_id, started_at, ended_at, reason)`** — appended on every clean shutdown / heartbeat. Gap-rate panel diffs the sessions to find unrecorded minutes.

Both belong to a follow-up audio-daemon PR; this opportunity intentionally stops at what the existing schema supports.

## Visual changes (skin pass)

The drop-in replacement of `quality.rs` makes these substitutions:

| Today's local class | Design-system class |
|---|---|
| `<div class="card">` | `<div class="bnb-card pad">` |
| `<h2>` inside a card | `<div class="bnb-eyebrow">{kicker}</div><h3>{title}</h3>` (matches `section-header` pattern elsewhere) |
| `.stat-grid` | `.stat-row` (4 — 6 column grid that's already in use across the app) |
| `.stat-card`, `.stat-value`, `.stat-label` | `.stat-tile` (also already used) |
| `.bar-chart` | inline SVG (no class needed); a `.bnb-skel-bars` lookalike but with `--moss` fills |
| `.hour-bars` | `.miniheat-cells` (already shipped) |
| Local `<nav>` (Overview / Settings / Rules / Quality / Notifications / System) | the standard `admin_shell()` sub-nav |

After the lift, the inline `<style>` block in `quality.rs` is empty and can be removed entirely — every visual rule comes from `app.css`.

## Risk

Low. The two new queries are additive — they hit only `detections` and `detection_reviews`, both of which already exist with the column shapes the SQL expects. The skin pass touches the same HTML the existing page renders; verified that every replacement class exists in `app.css`.

---

<!-- BNB:CROSSREF-FOOTER -->
## Related

* Companion to O-25 (inline-style audit): the `<style>` block in `quality.rs` is one of the bigger inline-CSS deltas in the codebase.
* The review-trend panel cross-links to `/quarantine` and `/review-queue`, both shipped.
<!-- BNB:CROSSREF-FOOTER -->
