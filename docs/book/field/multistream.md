# Multi-stream detection handling

How BirdNet-Behavior treats detections when several audio streams (RTSP mics /
cameras + the on-board mic) can hear the same bird at once.

## Status

- **Stage 1 — `Source` tagging: shipped.** Every detection is tagged with its
  source (the RTSP stream id, e.g. `cam1`, or `local` for the on-board mic).
  Non-destructive; historical / imported rows stay `NULL`. See migration 18 and
  `DetectionRecord.source`.
- **Stage 2 — the operator-facing UX: specified here, not yet implemented.** It
  is deliberately routed through the UI design pass (see the "per-source /
  multi-stream UI" item in [`docs/DESIGN_BRIEF.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/DESIGN_BRIEF.md)) so it ships as
  one coherent experience rather than bolted-on widgets.

## The problem

Multiple streams capture independently, so the *same* bird heard by two streams
becomes two detection rows. Two things make the timestamps not line up across
streams: (1) `Time` is the **segment start**, and streams segment on unaligned
boundaries (up to a full segment apart); (2) RTSP/network **clock drift** on
top. The *true* instant of a detection is recoverable, though:

```
instant = parse(Date + " " + Time) + chunk_offset_secs
```

(`chunk_offset_secs` is the sub-second moment within the segment.) So a robust
cross-stream comparison is possible — but it must be window-based, never an
exact-timestamp match.

## Design principle

**Non-destructive.** Raw detections are never merged or deleted. Every
multi-stream behaviour is a reversible *view* or *filter* over Stage 1's
`Source` column. Exports (CSV / BirdDB.txt) and the detection-detail page always
show raw rows.

## Recommended approach — ordered by value for a non-technical operator

Build in this order. Each stage is independently useful and lower-risk than the
next.

### 1. Corroboration (primary — non-destructive, always-on, zero config)

On a detection (detail page, optionally a feed badge), show the *other* sources
that heard the same species within a generous window of the true instant:

> **Also heard by:** cam2 · 06:00:01 · 0.91 · cam3 · 05:59:58 · 0.88

This reframes "duplicates" as **corroboration** — multiple mics agreeing is a
*stronger* signal a detection is real, which is exactly the reassurance a
non-technical owner wants. It hides nothing, needs no settings, and is a pure
read-time query.

*Sketch:* `concurrent_detections_from_other_sources(conn, date, time, sci_name,
exclude_source, window_secs)` → rows where `Sci_Name = ?`, `Source IS NOT NULL
AND Source <> exclude_source`, and `ABS(julianday(Date||' '||Time) -
julianday(?d||' '||?t)) * 86400 <= window` (a generous default window, ~30 s,
since segment starts are unaligned). No schema or write-path change.

### 2. Deterministic per-source analytics filter (opt-in, comprehensible)

Let the operator include/exclude a *whole* source from counts/analytics with
simple checkboxes ("count detections from: ☑ cam1 ☑ cam2 ☐ cam3-backup"). This
is the safe, non-technical answer to "a redundant backup mic doubles my counts":
deterministic, reversible, and — unlike a fuzzy time window — it can never hide a
*spatially distinct* bird. Implementation: a stored set of excluded sources +
`AND (Source IS NULL OR Source NOT IN (…))` on the count/analytics queries (the
indexed `Source` column from Stage 1 makes this cheap).

### 3. Time-window collapse (advanced, off by default — last resort)

Only for stations the operator *declares* co-located/redundant. Flag a detection
as a duplicate of an earlier one (same species, **different** source, true
instants within `DEDUP_WINDOW_SECS`); keep the row, set a nullable `dup_of`
(rowid of the canonical = highest-confidence in the group). Analytics add `AND
dup_of IS NULL` when enabled. `DEDUP_WINDOW_SECS` defaults to `0` (off).

This is **last** on purpose: it is the only option that can hide a real
detection if the operator misjudges their topology, and the one a non-technical
user can least reason about. Ship it only if (1) and (2) prove insufficient, and
only with the design pass's UX.

## Why not lead with auto-collapse

For a *mixed* topology the system cannot distinguish "two mics, one bird" from
"two mics, two birds in different trees." A fuzzy auto-collapse that guesses
wrong **silently drops real detections** — the worst failure for a detection
appliance, and the fastest way to lose a non-technical owner's trust. Leading
with corroboration (adds information) and deterministic source filtering (hides
nothing the operator didn't explicitly choose) gives the same "cleaner numbers"
outcome without the data-trust risk.

## Same-source is never collapsed

All cross-stream logic requires **different** `Source`. Two detections from the
*same* source within the window are either one stream's adjacent chunks (already
keyed apart by `chunk_offset_secs`, migration 11) or genuinely separate birds —
never folded.

## Route the UI through the design session

The per-source legend/filter, the corroboration display, and the (advanced)
collapse toggle are the "per-source / multi-stream UI" work item in
[`docs/DESIGN_BRIEF.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/DESIGN_BRIEF.md). Design them there as one coherent surface,
then implement against the agreed design.
