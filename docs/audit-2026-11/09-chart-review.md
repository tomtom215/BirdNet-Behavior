# 09 — Chart review: what to keep, rebuild, replace, and what is missing

**Nothing was rendered for this review.** The fixture binary it was allowed to use
(a fixture binary from another lane's build) did not exist on 2026-11-24, and building one was out of
scope (disk). Every finding below is from reading source, so every Status is VERIFIED-BY-READING or
CONJECTURED; none is SEEN-IN-RENDER. CLAUDE.md says "reading a rendered picture is a check in its own right":
that check has not been made here, and each chart-type verdict should be confirmed against a render before a
rebuild is commissioned. The rendered evidence that exists is in `05-ui-rendered-walkthrough.md` (two days
older); where a point overlaps a UIR-nn or UIC-nn id it is cited and not repeated.

Read: `docs/design/charts.md`; the headings and chart entries of 05 and 06; `checklist.json` statuses; the
module docs and selected bodies of `crates/birdnet-web/src/routes/pages/viz/*.rs`; parts of `behavioral.rs`,
`behavioral_answers.rs`, `behavioral_assembly.rs`, `atoms.rs`, `dawn_chorus.rs`; the SQL builders in
`crates/birdnet-behavioral/src/queries.rs`, `phenology/abundance.rs`, `phenology/timing.rs`; the DuckDB mirror
schema (`connection/mod.rs`); the SQLite migrations for effort, weather and sound levels;
`crates/birdnet-web/src/security.rs` (CSP). Section 5 lists what was not opened.

Audiences: **B** = non-technical birder, **C** = chart lover, **R** = researcher.
"FOLKLORE" marks a statement about an outside standard, a library or the extension's behaviour that was not
verified in this repository's files.

## Counts at a glance

| | |
|---|---|
| Charts and figures inventoried | 43 rows (06's rows 1–42, with the five orphans 43–47 as one row) |
| Verdicts | keep 17 · refine 17 · rebuild 2 · replace 5 · remove 2 |
| Rendered and looked at | 0 |
| Findings | CHT-01 … CHT-24; method grounding G-01 … G-10 in section 7 |

The short version. The kit's *drawing* is careful (zero baselines, monotone curves, four-state cells). The
weakness is one level up, in **what is divided by what and compared with what**. Every relational and
behavioural figure (together, before/after, who answers whom, retention, running order, assembly) draws an
observed count or share with **no expectation beside it**, so the commonest species wins every one of them,
and none draws an interval although the code already computes one (`phenology/abundance.rs:517`). The
duckdb-behavioral functions are all called, but only in their dashboard shapes; the bird questions they
could answer (visit length, mixed-species bouts, return time, reaction to a predator) are not asked.

---

## 1. Inventory

Row numbers are 06's (`06-ui-code-charts-ia.md` §2), so the two tables can be read together; renderer file
and line are there. **Denominator**: *raw* = a count; *own-max* = scaled to that row's own largest value;
*share* = a fraction, with its base; *effort* = whether listening time divides anything (it never does on a
page; `pages/listening.rs:12-16` per UIC-02). A `*` on a verdict means the renderer itself was not opened in
this review and the verdict rests on 05/06 and the module doc only.

| # | Page · figure | Chart type | Question | Data and denominator | Verdict |
|---|---|---|---|---|---|
| 1 | Today · day strip | stacked columns by hour, temp strip, sun rail | when were birds heard today, and who | raw count per hour, top 8 + other | keep |
| 2 | Today · Every song | scatter on a sky (JS SVG) | every detection, by confidence | one mark per detection, capped | keep* |
| 3 | Today · hero pulse | live canvas trace | is it hearing anything now | signal | keep* |
| 4 | Today · top-species rail | sparkline, 14 days | is this bird up or down | raw, **own-max per row** (`atoms.rs:264-267`) | refine (CHT-01) |
| 5 | Today rows | confidence bar | how sure | model score, 0–1 | keep* |
| 6 | When · soundscape | days × hours grid (canvas), 3D on request | the year's daily rhythm | raw per day-hour; effort hatches | keep* |
| 7 | When · who's singing | centred streamgraph | composition over time | raw, top 8 + other | refine (CHT-02) |
| 8 | When · by hour | columns, 24 h | when in the day | raw summed over range; clock dawn (UIC-26) | refine (CHT-03) |
| 9 | When · hour × weekday | disc heat grid 24 × 7 | busiest hour of which weekday | raw per listened weekday | replace (CHT-04) |
| 10 | Dawn · rings | concentric radial ribbons | who sings when | **own-max per species** (`dawn_chorus.rs:77`), 60 d pooled | replace (CHT-05) |
| 11 | Dawn · strips | per-species hour strips | same, linear | own-max | refine (CHT-05) |
| 12 | Trends · detections/day | columns + moving average + ghost period | getting busier? | raw per day; outage hatched | refine (CHT-06) |
| 13 | Trends · richness | same form | more kinds? | raw species per day | refine (CHT-07) |
| 14 | Trends · peak windows | table | busiest 15 minutes | raw | keep |
| 15 | Trends · activity sessions | table | — | duplicate of #21 (UIC-07) | remove |
| 16 | Trends · anomalous days | z-score table | unusual days | raw vs rolling mean (UIC-03 done) | replace (CHT-06) |
| 17 | Together · chord | chord diagram | who is heard with whom | Jaccard on 5-min blocks, ≥3 shared (`correlation.rs:29`) | replace (CHT-08) |
| 18 | Together · ladder | triangular matrix | same, phone | same | refine (CHT-08) |
| 19 | Together · companions | table + bars | companions of one bird | raw shared days (UIC-04 in progress) | refine (UIC-04) |
| 20 | Migration · ridgeline + richness lane | ridgeline, 52 weeks | when each species is here | **own-max per species** (`viz/ridge.rs:6-10,330`), this year, raw | refine (CHT-09) |
| 21 | Visits · bursts | Gantt lanes per day | when each bird sang, how long | `sessionize` sessions, 7 days (`behavioral.rs:215`) | refine (CHT-10) |
| 22 | Visits · who keeps coming back | heat table | does it return | `retention` share of anchor days, uncensored | rebuild (CHT-11) |
| 23 | Sequences · before/after | two-sided ribbon flow | who precedes and follows X | `sequence_next_node`, share of sessions with a neighbour | rebuild (CHT-12) |
| 24 | Sequences · running order | lanes × clock, a polyline per morning | who opens the dawn | `window_funnel` + `sequence_match_events`, clock 04–09 | refine (CHT-13) |
| 25 | Sequences · who answers whom | disc matrix, directed | call and reply | `sequence_count` within a minute, raw, floor 3 | refine (CHT-14) |
| 26 | Sequences · chorus assembling | disc field, voices × time allowed | how fast the chorus forms | share of mornings, 14 windows evenly spaced | replace (CHT-15) |
| 27 | Species list | sparkline | 14-day shape | own-max | refine (CHT-01) |
| 28 | Life list · growth | rising curve | how many kinds so far | cumulative count, no estimator | refine (UIR-25, UIC-25 #4) |
| 29 | Life list · firsts | timeline (JS SVG) | when each joined | dates | keep* |
| 30 | Species · when most active | columns, 24 h | this bird's day | raw by clock hour | refine (CHT-03) |
| 31 | Species · last 14 days | columns | recent | raw; effort hatched | keep |
| 32 | Species · through the year | calendar heat | its season | raw per day; effort hatched | keep |
| 33 | Species · same days | table | companions | raw shared days | refine (UIC-04) |
| 34 | Recordings · live spectrogram | canvas | what is it hearing | audio | keep* |
| 35 | Clip spectrogram | server PNG | what did it hear | audio | keep* |
| 36 | Weekly · by day | columns | this week | raw, like-for-like days | keep |
| 37 | Year · calendar | calendar heat | the year | raw per day; effort hatched | keep |
| 38 | Year · who led each month | bump chart + bars (JS SVG) | rank through the year | rank of raw monthly count | keep* |
| 39 | History · month | HTML grid, own linear ramp | pick a day | raw (UIC-18, UIR-09) | refine (UIC-18) |
| 40 | History day | columns | that day | raw | keep |
| 41 | Kiosk plate | engraved plate | today at a distance | raw | keep* |
| 42 | Capture · EQ | curve | filter response | gain | keep* |
| 43–47 | orphans | histogram, widgets | — | — | remove (revive 43 per UIC-25 #7) |

---

## 2. Findings on charts that are not "keep"

### CHT-01: Sparklines are each scaled to their own maximum, so the rail compares shapes that are not comparable
- Severity: medium — Kind: encoding — Audience: B, C — NEW
- Evidence: `routes/pages/atoms.rs:264-267` (`hi = max of this row`, `max = hi.max(1)`), `:270-276` (y from
  `v / max`). A species heard 2, 1, 2 times and one heard 200, 100, 200 draw the same line. Rows 4 and 27.
  Status: VERIFIED-BY-READING
- Impact: B reads a tall wiggle on a rare bird as a big week. The count beside it corrects this only if read.
- Proposed change: keep own-max (a shared scale flattens every row but the commonest) but make the scale
  visible: end-dot on the last day plus a faint band for the previous 14 days' range, and draw rows whose
  14-day total is under about 10 as **dots per detection day** instead of a line, since a line through 0/1
  counts is noise. One line of caption on the rail: "each line is scaled to that bird's own busiest day".
- Acceptance gate: unit test: a series with total < threshold returns no `<path>` line; counterpart with a
  large series still draws one. Observe red against the current function.
- Effort: S

### CHT-02: The streamgraph answers "how much, of whom" for the top band only
- Severity: medium — Kind: chart type — Audience: C, R — NEW
- Evidence: `viz/timeline.rs` module (centred stream, top eight + other; charts.md honesty rule 4). In a
  centred stream every band but the middle one rides on a moving baseline, so a species' own trend cannot be
  read from its thickness against anything fixed (FOLKLORE: the standard criticism of stacked and stream
  graphs; check against a render with one declining species under a growing one).
  Status: CONJECTURED — probe: fixture with one species falling 50 % while the total rises; ask a reader which
  band fell.
- Proposed change: keep the stream as the opening picture (it is the one that draws a viewer in) and add a
  second view on the same card, toggled without script by a link: **small multiples, one lane per species on
  a common y-axis**, sorted by total, same x. That is the view R needs and it is what the legend's "bring
  one forward" gesture is approximating.
- Acceptance gate: `charts-edge.mjs` for the lanes; a test that every lane shares one `y` scale.
- Effort: M

### CHT-03: Hour-of-day columns pool a season of shifting sunrises onto a clock axis
- Severity: medium — Kind: scale — Audience: R, C — extends UIC-26, UIR-18 (not repeated)
- Evidence: rows 8 and 30 sum by clock hour over the range; sky drawn only ≤ 14 days (06 row 8). Hourly
  listening time exists in SQLite (`recording_effort_hourly`, `birdnet-db/src/migration.rs:2754-2761`) and is
  read only by `sqlite/queries/effort.rs` and `metrics.rs` (grep of `crates/*/src`); no chart divides by it.
  Status: VERIFIED-BY-READING
- Impact: over 90 days sunrise moves by more than an hour at mid-latitudes (FOLKLORE; the station can compute
  its own), so a dawn peak is smeared across two or three columns and reads as a broad morning.
- Proposed change: for ranges over 14 days offer the axis as **hours from sunrise** (negative before), bins of
  30 minutes, value = detections per listened hour from `recording_effort_hourly`; keep the clock axis as the
  default for B. A range with no hourly ledger falls back to counts and says so.
- Acceptance gate: fixture with a bird always 20 minutes after sunrise across 120 days: one bin holds ≥ 80 %
  of it on the solar axis; the clock axis spreads it over ≥ 3 columns (counterpart, stays as is).
- Effort: M. Blocked partly by: hourly effort is not mirrored to DuckDB (section 4, third smaller point); the query can stay in SQLite.

### CHT-04: Hour × weekday is the wrong second axis for birds
- Severity: medium — Kind: chart type — Audience: C, R, B — NEW (UIR-02 fixed the sentence, not the chart)
- Evidence: `docs/design/charts.md:95` prescribes the grid; `viz/heat.rs` draws 24 × 7. A weekday is a human
  cycle; a bird's schedule follows the sun and the season. The only real weekday signal at a garden station is
  human noise (FOLKLORE), which is a detectability artefact, not behaviour. The row that would show the
  season, week of year, is only on the terrain (#6) for all species pooled.
  Status: VERIFIED-BY-READING for what is drawn; the claim about signal is CONJECTURED — probe: on a real
  station, chi-square of weekday × hour against the product of margins.
- Proposed change: replace with an **actogram**: x = day of year (or the selected range), y = hour of day,
  cell = detections per listened hour, with the station's sunrise and sunset drawn as two curves across it,
  and a species picker (all species by default). It answers "when in the day, and how that moves through the
  year" in one picture, for one bird or all. Keep weekday as one sentence, printed only when it differs from
  chance, with "probably people, not birds".
- Acceptance gate: geometry test that the sunrise curve's y at each day equals `solar_times_local` for that
  day; the four cell states (heard, none, not listening, not yet) survive.
- Effort: M

### CHT-05: Dawn rings: radius is free, so identical activity is longer on an outer ring
- Severity: medium — Kind: chart type — Audience: C, R — NEW (UIR-36 covers the pills)
- Evidence: `dawn_chorus.rs:77` ("Each ring is one species; its ribbon swells outward at the hours it's most
  active, normalized to its own peak"); `docs/design/charts.md:91` itself records the price of a radial hour
  chart ("an arc whose area grows with its radius"). On concentric rings one hour is a longer arc on the
  outer ring than the inner, so ring order changes apparent magnitude; own-max normalisation then removes the
  one honest comparison left (who sings more).
  Status: VERIFIED-BY-READING of the encoding as documented; CONJECTURED that it misleads in the render.
- Proposed change: make the linear strips (#11) the figure: one lane per species, x = minutes from sunrise
  (−90 … +180), each lane a filled density of first-song and all-song times, ordered by median onset, with the
  median as a tick and the middle 50 % as a darker span. That is the textbook dawn-chorus figure
  (FOLKLORE) and reads on a phone. Keep one small ring as the tab's emblem if wanted, without numbers.
- Acceptance gate: lanes sorted by median onset (test); every lane's x maps the same minute to the same pixel.
- Effort: M

### CHT-06: Trends has no band, so "unusual" is a table two folds away from the chart it is about
- Severity: medium — Kind: missing uncertainty — Audience: C, R — extends UIR-34, UIC-03 (done)
- Evidence: `viz/trend.rs:1-15` (columns, moving average, ghost of the previous period, extremes); anomalies are
  a separate z-score table (06 row 16). `grep` for `interval|whisker|band` in `viz/kit.rs`, `trend.rs`,
  `bars.rs`: no interval mark exists in the kit.
  Status: VERIFIED-BY-READING
- Proposed change: draw the expected range **on** chart 12 as a band (rolling median ± a robust spread, e.g.
  the 10th–90th percentile of the previous 30 listened days), mark days outside it, and delete the table as a
  separate fold (it becomes the chart's "show as a table"). Counts are overdispersed, so a z-score on raw
  counts over-flags busy seasons; use the percentile band or work on `sqrt(count)`.
- Acceptance gate: a steady Poisson-like fixture flags at most the nominal share of days; a planted 5× day is
  flagged.
- Effort: M

### CHT-07: Richness per day is mostly a picture of how much was heard
- Severity: medium — Kind: denominator — Audience: R — NEW
- Evidence: chart 13 plots distinct species per day; `birdnet-timeseries/src/queries/diversity.rs` offers
  Shannon and evenness only (`grep -i 'chao|rarefact'`: none). Species counted rises with detections and with
  listening time regardless of what is present (FOLKLORE: species–effort relationship; standard remedy is
  rarefaction to a common sample size).
  Status: VERIFIED-BY-READING (no estimator); CONJECTURED size of the effect.
- Proposed change: plot richness **per week**, rarefied to a fixed number of listened days (resample days
  without replacement, median of N draws), with the raw count as a ghost. Print the method under it.
- Acceptance gate: fixture where week B has the same species pool and twice the listening days: rarefied
  richness equal within tolerance; raw differs.
- Effort: M

### CHT-08: The chord diagram cannot show the absence of association, and has no chance level
- Severity: high — Kind: chart type + null model — Audience: C, R — extends UIR-22 (lift proposed there; not done)
- Evidence: `viz/radial.rs:5-10`: ribbon width and colour are both the Jaccard share, an arc's length is the
  sum of its ribbons, "no arc has a stretch with nothing on it". So a species that is abundant and
  *avoids* others, or one that is independent of everyone, has no visible encoding: the form only draws
  positives. Pairs come from `birdnet-db/src/sqlite/queries/correlation.rs:44-100` (shared five-minute blocks,
  `MIN_SHARED_BLOCKS = 3`, `pages/correlation.rs:29`); no expected value is computed.
  Status: VERIFIED-BY-READING
- Impact: two dawn singers share blocks because both sing at dawn. The picture says "together"; the data says
  "same hour". R will not accept it; C is shown structure that is mostly the daily cycle.
- Proposed change: make the **ordered matrix the primary figure at every width** (the ladder exists,
  `viz/matrix.rs`), coloured on a *diverging* ramp by log lift = observed shared blocks ÷ expected, where
  expected is computed **within hour-of-day strata** (for each clock hour, p_A·p_B·blocks listened in that
  hour, summed) so the daily cycle is removed. Cells whose interval covers 1 are left empty. The chord stays
  as an optional view fed the same lift, positive links only, and says it shows only positives.
- Acceptance gate: the independence fixture (CHT-24): no cell coloured. Counterpart: a planted pair that
  co-occurs at 3× chance is the darkest cell.
- Effort: M

### CHT-09: Ridgeline: own-max heights hide low-count ridges that are one false positive
- Severity: medium — Kind: missing uncertainty — Audience: R, B — extends UIC-02, UIC-25 #1, UIR-36
- Evidence: `viz/ridge.rs:330` (`counts[w] / max * amp`). A species with 3 detections all year gets a ridge as
  tall as one with 3,000. The effort-corrected weekly rate **with a Poisson interval** already exists
  (`phenology/abundance.rs:340`, interval `:517`, docs `:300-312`) and no page draws it.
  Status: VERIFIED-BY-READING
- Proposed change: beyond UIC-02's rate toggle: (a) ridge opacity or a hollow outline when the peak week's
  count is under a floor; (b) on focus of one species, redraw it alone on a true axis with the interval as a
  band and last year as a ghost; (c) mark `effort_gap_before_arrival` (`phenology/timing.rs:121-129`) as a
  hatched stretch before the first rise, so an "arrival" after an outage is not read as one.
- Acceptance gate: test that a species under the floor carries the low-evidence class; that the band's edges
  equal `poisson_interval_exact(k) / hours`.
- Effort: M

### CHT-10: Bursts shows seven days of bars and no summary of what a visit is
- Severity: low — Kind: missing "so what" — Audience: B, R — NEW
- Evidence: `viz/bursts.rs:1-14` (Gantt, one lane per day, bars under four minutes drawn at four);
  `behavioral.rs:215` (`DAYS = 7`). The distribution of session length and of visits per day, per species, is
  never drawn. Status: VERIFIED-BY-READING
- Proposed change: keep the timeline; add the missing figure in section 3 (M-02) beside it and a sentence
  from it ("a Robin's typical visit lasts 6 minutes; it comes 4 times a day").
- Effort: S for the sentence, M with M-02.

### CHT-11: Retention counts the end of the record as "did not return"
- Severity: high — Kind: statistical bug + chart type — Audience: R, B — NEW (UIR-03 is the missing colour)
- Evidence: `birdnet-behavioral/src/queries.rs:366-376`: every distinct detection day is an anchor and
  `retention(...)` asks whether a later day exists within N days; there is no condition that anchor + N lies
  inside the observed period (`grep -ri censor` in the crate: no hits). Intervals go to 30 days
  (`pages/behavioral.rs:411-418`). An anchor in the last 30 days of data cannot be observed to return within
  "1 mo" and is averaged in as a failure (`:362-364`, `AVG(CASE WHEN r[i] …)`). Nor is a day with no listening
  distinguished from a day of absence.
  Status: VERIFIED-BY-READING. Size CONJECTURED — probe: a `zz_probe` with a species detected every day for 40
  days ending today; expect the "1 mo" rate well under 100 %.
- Impact: every species' longer-interval rates are biased down, most for recent arrivals and on young
  stations, which is most stations. The doc comment at `:340-346` already found one way this aggregate
  misleads; this is a second.
- Proposed change: (1) restrict anchors per interval to `a.d + N <= max observed day` and to anchors whose
  following N days have listening; report the number of anchors beside each rate. (2) Rebuild the figure: a
  12 × 6 table of percentages is not the picture. Draw **a presence barcode per species** (one tick per day
  heard across the whole record, not-listening hatched) with the residency class as the row group, and for
  the return question a **step curve of "heard again within t days"** (t = 1 … 30, cumulative, with a Wilson
  band from the anchor count). The barcode is what B reads at once; the curve is what R can cite.
- Acceptance gate: the probe above reads 100 % at every interval after the fix (red before); a counterpart
  with a true 10-day absence still shows it.
- Effort: M

### CHT-12: Before/after ranks neighbours by frequency, so the commonest bird follows everything
- Severity: high — Kind: null model + chart type — Audience: R, C, B — NEW
- Evidence: `queries.rs:813-836`: `sequence_next_node` gives one neighbour per session; the result is
  `COUNT(*)` per neighbour over `SUM(COUNT(*)) OVER ()`, ordered by frequency. Nothing compares that share
  with the neighbour's share of all detections or sessions. `viz/flow.rs:9-12`: ribbons whose widths do not
  sum to the node, the tail "unaccounted for".
  Status: VERIFIED-BY-READING
- Impact: for any trigger the top "next" species is whoever is heard most. The figure looks like a finding
  and is the species list. A flow diagram also implies conservation (what goes in comes out), which this one
  explicitly does not have.
- Proposed change: compute, per neighbour, share after the trigger ÷ that neighbour's share as "next species"
  after **any** species in the same hours (one more `sequence_next_node` pass with `base = TRUE`, or a plain
  `LEAD(Com_Name)` over the sessionised timeline). Draw a **two-sided dot plot**: species on rows, x = log
  lift, left panel "before", right panel "after", dot size = sessions, a Wilson or bootstrap interval as a
  whisker, a rule at 1 ("as often as chance"), rows with n under a floor greyed. Sort by lift, not by count.
  The same bird calling again is its own labelled row, as now.
- Acceptance gate: independence fixture: no interval excludes 1 (allow the nominal error rate). Planted
  follower: it is the top row. Red against the current ordering, where the commonest species is top.
- Effort: M

### CHT-13: The running order is drawn on a clock that the dawn moves across
- Severity: medium — Kind: scale — Audience: R, C — NEW
- Evidence: `pages/behavioral.rs:955-963` (`DAWN_HOUR_START = 4`, `DAWN_HOUR_END = 8`, window = the whole
  span); `queries.rs:436` (`EXTRACT(HOUR FROM detection_timestamp) BETWEEN`); `viz/funnel.rs:4,9` (axis 04:00
  to 09:00, median tick over all mornings); voices chosen by count then ordered by opening time
  (`behavioral.rs:1145-1146`). Status: VERIFIED-BY-READING
- Impact: over the 90 days some of these figures use (`behavioral_assembly.rs:22`), each lane's dots drift
  with sunrise, so the scatter along a lane is mostly the calendar and the median tick is a mid-season time
  nobody observed. At high latitudes in June the chorus starts before 04:00 and is cut off.
- Proposed change: x = minutes from civil dawn (already computed for the Dawn tab, `dawn_chorus.rs:233`);
  select the window relative to it; show each lane's middle 50 % as a bar under the dots. Add the order's
  consistency as one number with its meaning ("on 7 mornings in 10 the Robin was first").
- Acceptance gate: fixture with fixed offsets from sunrise across 90 days: lane spread on the new axis under
  5 minutes; the old axis shows the drift (counterpart kept as a regression fixture for the query window).
- Effort: M

### CHT-14: Who answers whom draws raw exchange counts; two frequent singers "answer" by chance
- Severity: medium — Kind: null model — Audience: R — extends UIR-21
- Evidence: `birdnet-behavioral/src/types.rs:232-237` (`sequence_count` within `within_secs`, non-overlapping);
  disc radius from `v / max` (`behavioral_answers.rs:193-195`); floor `MIN_EXCHANGES = 3` (`:27`). No expected
  count. Status: VERIFIED-BY-READING
- Proposed change: a **time-shift null**: recompute the matrix with species B's timestamps shifted by each of
  ±5, ±10, ±15 minutes (same day), which keeps each bird's rate and daily shape and breaks sub-minute timing;
  disc area = observed ÷ mean shifted, drawn only where observed exceeds every shift. It is six more runs of
  the query that exists (`answers_sql`, `queries.rs:901`).
- Acceptance gate: independence fixture draws no disc; a planted 20-second reply does.
- Effort: M

### CHT-15: Chorus assembling encodes a cumulative share as disc area on an ordinal time axis
- Severity: medium — Kind: chart type — Audience: C, B, R — NEW
- Evidence: `viz/assembly.rs:3-12` (disc area = share of mornings with ≥ k voices within the time allowed),
  `:250` (radius from sqrt of share), `:14-16` ("the columns are the rungs of the ladder, evenly spaced, so
  no scale hides a stretch of time"); rungs are 1, 2, 3, 5, 8, 12, 20, 30, 45, 60, 90, 120, 180, 300 minutes
  (`birdnet-behavioral/src/types.rs:188`). Evenly spacing those is a roughly logarithmic axis that is not
  labelled as one. Area is the weakest channel for comparing shares that differ by a few points (FOLKLORE:
  Cleveland–McGill ranking), and the quantity is monotone in both directions, which is what a line shows best.
  Status: VERIFIED-BY-READING
- Proposed change: replace with **cumulative step curves**: x = time allowed on a labelled log axis, y = share
  of mornings (0–100 %), one step line per "at least k voices", direct-labelled at the right end, a binomial
  (Wilson) band from the number of mornings (≥ 7, `behavioral_assembly.rs:28`), and a rule at 50 % whose
  crossings are the sentence: "half of mornings have three voices within 12 minutes". The disc field's median
  line and quartile band become readable facts on this chart.
- Acceptance gate: each curve non-decreasing; curve k+1 never above curve k; band width shrinks with mornings.
- Effort: M

---

## 3. Missing visualizations and insights, ranked by value over effort

Tables checked: DuckDB mirror `detections` (`Date, Time, Sci_Name, Com_Name, Confidence, Lat, Lon, Week,
File_Name, import_batch_id, review_verdict, detected_at_utc, run_id, Source`,
`birdnet-behavioral/src/connection/mod.rs:642-660`), view `detections_ts` adding `detection_timestamp`,
`detection_instant`, `detection_date` and dropping rejected rows (`queries.rs:90-98`), and
`recording_effort(date, source, seconds)` (`connection/mod.rs:685-689`). SQLite only: `weather(at, temp_c,
precip_mm, wind_kt, …)` (`birdnet-db/src/migration.rs:425-434`), `recording_effort_hourly` (`:2754-2761`),
`sound_level_broadband(date, hour, source, samples, a_power_sum, …)` (`:1490-1498`).
Extension calls confirmed in use: `sessionize`, `retention`, `window_funnel`, `sequence_match`,
`sequence_count`, `sequence_match_events`, `sequence_next_node` (grep of `queries.rs`); pattern syntax
`(?N)`, `.*`, `(?t>0)`, `(?t<=secs)` (`queries.rs:953-970`). SQL below is a sketch, not run.

### CHT-16 (M-01): Reporting frequency through the year — the birder's bar chart
- Question: "in which weeks can I expect this bird?" for every species at once.
- Capability: plain DuckDB. `SELECT Com_Name, week(detection_date) w, COUNT(DISTINCT detection_date) AS heard
  FROM detections_ts GROUP BY ALL` joined to `SELECT week(CAST(date AS DATE)) w, COUNT(DISTINCT date) AS
  listened FROM recording_effort WHERE seconds > 0 GROUP BY w`; value = heard ÷ listened.
- Chart: one row per species, 52 thin bars whose thickness is the share of listened days (the form eBird's
  bar charts use — FOLKLORE), sorted taxonomically or by first week; weeks with no listening hatched.
- Honesty: Wilson interval on heard/listened in the tooltip and table; a week with under 3 listened days is
  drawn hollow. Share of days is robust to one noisy day in a way a count is not.
- Blocks: none. Stations without an effort ledger for a period fall back to "days with any detection" as the
  base and say so. Effort: M. **Highest value: B, C and R all read it.**

### CHT-17 (M-02): What a visit is — session length and visits per day, per species
- Question: "does it stay or pass through; how often does it come?"
- Capability: `sessionize(detection_instant, INTERVAL '30 MINUTE') OVER (PARTITION BY Sci_Name ORDER BY
  detection_instant)` (the existing `sessionize_sql`, `queries.rs:276`, without its `LIMIT`), then per species
  `quantile_cont(duration_secs, [0.25, 0.5, 0.75])` and sessions per listened day.
- Chart: one row per species; a box (quartiles) on a log time axis for visit length, and beside it a dot for
  visits per listened day. Sorted by median length: singers that hold a perch separate from flyovers.
- Honesty: single-detection sessions have length 0; show their share as a separate tick, do not put them on
  the log axis. Bootstrap interval on the median over days, not sessions (sessions within a day are not
  independent). Say that the 30-minute gap defines the answer and offer 10 and 60.
- Blocks: none. Effort: M.

### CHT-18 (M-03): Mixed-species bouts
- Question: "when the garden gets busy, who arrives together?" — a better "together" than a five-minute block.
- Capability: `sessionize` over the station's whole timeline with no species partition (already done inside
  `adjacent_species_sql`, `queries.rs:806-811`), then `COUNT(DISTINCT Com_Name)`, duration and
  `list(DISTINCT Com_Name)` per `sid`.
- Chart: histogram of species per bout; under it the ten most frequent species sets as rows of filled dots
  (an UpSet-style matrix) with their counts.
- Honesty: expected set frequency under independence from each species' per-bout rate, within hour strata;
  show only sets above it. Gap length stated and switchable.
- Blocks: none. Effort: M.

### CHT-19 (M-04): What happens when the hawk calls — an event-aligned plot
- Question: "do the others fall silent, or alarm, after a predator (or a jay, or a mower-loud minute)?"
- Capability: DuckDB range join: for each trigger detection (deduplicated to one per 20 minutes), count other
  species' detections in one-minute bins from −15 to +15 minutes; `sequence_count('(?1).*(?t>0)(?t<=300)(?2)',
  …)` gives the per-day count of trigger-then-alarm-species exchanges for a named pair.
- Chart: line of mean detections per minute around time 0, with matched control windows (same clock time, the
  same station, on days without the trigger) as a grey band; small multiples for the three species that
  change most.
- Honesty: the control band is the null; bootstrap over trigger events; print the number of events and hide
  the figure under about 10.
- Blocks: a maintainer decision on which species count as triggers (a default raptor/corvid list by family
  needs a taxonomy source; not checked whether one is stored). Raptor detections are few and error-prone.
  Effort: M–L.

### CHT-20 (M-05): Song onset against sunrise through the season
- Question: "is it starting earlier as the days lengthen; who tracks the sun and who the clock?"
- Capability: `MIN(detection_timestamp)` per species per day in DuckDB; sunrise per day from the station's
  own solar code (`solar_times_local`, used at `dawn_chorus.rs:233`), joined in Rust. Overlaps UIC-25 #6
  (a polar clock); this is the seasonal scatter that one cannot show.
- Chart: per species, x = date, y = minutes from sunrise of first song, one dot per morning, rolling median
  line and interquartile band; the clock-time alternative as a toggle to show which axis is flatter.
- Honesty: only mornings with listening from at least 60 minutes before sunrise (needs
  `recording_effort_hourly`); a first detection is sensitive to one false positive, so offer the 5th
  percentile of the morning's detections as in `timing.rs:106-118`.
- Blocks: station coordinates set; hourly effort mirrored or queried from SQLite. Effort: M.

### CHT-21 (M-06): Weather response
- Question: "do I hear less in wind and rain, and which birds do not care?"
- Capability: SQLite `weather` joined by hour to detections and `recording_effort_hourly`; rate per listened
  hour by bins of `wind_kt`, `precip_mm > 0`, `temp_c`.
- Chart: per variable, binned dots with intervals (rate ratio against the calm/dry bin), stratified by
  hour-of-day block and month so season and dawn do not masquerade as weather. Not an overlay on the trend
  (UIC-25 #9 proposes that strip; this is the response curve).
- Honesty: wind and rain lower *detectability* as well as activity; the caption must say the figure cannot
  separate them. Pair with `sound_level_broadband` to show the masking directly.
- Blocks: weather is fetched only where configured and is not in the DuckDB mirror (section 4, third smaller point). Effort: M–L.

### CHT-22 (M-07, lower): Return across years, and the arrival order of spring
- `retention(year = y0, year = y0 + 1, year = y0 + 2)` per species gives "came back the next year"; and
  `window_funnel` over each year's first-arrival days tests whether the spring arrival order repeats.
  `interannual_trend_sql` and `migration_window_sql` exist (`phenology/timing.rs:346,256`); UIC-25 #2 already
  proposes the arrival dot plot, so only the two extension uses are new. Blocked by: at least three years of
  data, so nothing to draw on most stations yet. Effort: M. Park until a station has the years.

Also supported by stored data and not proposed in detail here: a confidence reliability curve from
`review_verdict` (share confirmed by confidence bin, Wilson band) — reviews are not a random sample, and
whether `03-precision-accuracy.md` already covers it was not checked.

---

## 4. Cross-cutting system issues (not in UIR/UIC)

### CHT-23: The kit has no mark for uncertainty and no shared rule for "too few to say"
- Severity: high — Kind: system gap — Audience: R, C
- Evidence: no interval, band or whisker primitive in `viz/kit.rs`, `trend.rs`, `bars.rs` (grep); floors are
  per figure and unrelated: `MIN_EXCHANGES = 3` (`behavioral_answers.rs:27`), `MIN_MORNINGS = 7`
  (`behavioral_assembly.rs:28`), `MIN_SHARED_BLOCKS = 3` (`correlation.rs:29`), none on retention or
  before/after. An interval is computed and unused (`phenology/abundance.rs:517`).
  Status: VERIFIED-BY-READING
- Proposed change: three kit primitives with one look everywhere: `kit::band` (interval around a line),
  `kit::whisker` (interval on a dot), and one "too few" treatment (hollow mark + "n = 4, too few to say").
  Add to charts.md as honesty rule 10: *a share or rate is drawn with its n and its interval, or not drawn.*
- Acceptance gate: a test per figure that a datum under its floor carries the hollow class; a source scan
  that every figure printing a `%` passes an `n` to its tooltip.
- Effort: M

### CHT-24: No gate grades whether a relational figure would say "nothing" when nothing is there
- Severity: high — Kind: test gap — Audience: R
- Evidence: the gates listed in `docs/design/charts.md` ("Gates") grade geometry, contrast, motion and edge
  sizes; CLAUDE.md records that a tidy fixture could not show the chord's collisions. No fixture of
  statistically independent species exists in that list. Status: VERIFIED-BY-READING of the list; the fixture
  generator itself was not opened (CONJECTURED — probe: grep the screenshot fixture for how species are drawn).
- Proposed change: an **independence fixture**: each species an independent process with its own hourly
  profile and rate, 90 days, plus a second fixture with one planted pair, one planted follower and one planted
  reply. Every figure in CHT-08, -11, -12, -14 and M-03 must render its "nothing stands out" state on the
  first and name the planted effect on the second. This is the discriminator-plus-counterpart CLAUDE.md asks
  for, applied to statistics.
- Effort: M

Smaller system points:
- **Three own-max figures, one unlabelled.** Ridgeline and dawn rings say they are normalised to each row's
  peak (`ridge.rs:6-10`, `dawn_chorus.rs:77`); the sparkline does not (CHT-01). One caption phrase and one
  small glyph for "each row on its own scale" across all three.
- **A solar time axis as a kit scale.** CHT-03, -05, -13 and M-05 all need "minutes from sunrise"; build it
  once (`kit::solar_axis`) with one tick vocabulary. UIR-18/19 name the inconsistency; this is the scale.
- **The analytics store lacks three tables the best figures need.** Only `recording_effort` (daily) is
  mirrored (`connection/sync.rs:1006`; no `weather`, `sound_level`, or hourly effort in `sync.rs` by grep).
  Either mirror them or keep those figures' queries in SQLite; decide once.
- **Stale statement that would mislead a per-source chart.** `phenology/abundance.rs:336-337` says the
  analytics copy "has no source column yet"; the mirror has `Source` (`connection/mod.rs:659`).
- **A figure should leave the page with its method.** `static/figure-export.js:1-9` exports the table as CSV.
  Add "save figure" producing the SVG with a `<metadata>` block: query name and parameters, date range,
  listened days, n, software version, the null model used. UIC-10 covers CSV/JSON reach; this is the citable
  picture.
- **Motion that explains.** The entrance is decoration (`.ck-grow`, `.ck-reveal`). Reserve motion for a change
  of state the reader caused: clock axis → solar axis (CHT-03) and count → rate should *move* each mark to its
  new place so the reader sees what the correction did; off under reduced motion, where the two states are
  simply both available.

---

## 5. Not examined

- **Nothing was rendered**: no screenshot, no light/dark, no phone width, no gate run. All chart-type
  verdicts are from encodings as documented in module docs and code.
- Not opened at all: `static/skysong.js`, `discovery.js`, `seasons.js` (header only), `plate.js`,
  `chart-kit.js`, `chart-text.js`; `terrain.rs` (grep only); `today*.rs`; `history.rs`; `species_pages.rs`;
  `year_in_review.rs`; `weekly_report.rs`; `migration.rs` (grep only); `heatmap.rs`; `timeseries_dash.rs`;
  `model_board.rs`, `model_evidence.rs`, `confidence.rs`, `occurrence_page.rs`, `station_health.rs`;
  `docs/design/charts-relational.md`; `insights-and-organisation.md`; audit files 01–04, 07, 08.
- Read only as module docs plus a few lines: every file in `viz/` (no renderer was read end to end);
  `bars.rs`, `heat.rs`, `timeline.rs`, `kit.rs` bodies not read.
- `birdnet-timeseries`: file list and two greps only. `connection/live.rs` and `analytics.rs`: signatures only.
- The extension's own documentation was not found vendored and not searched for beyond the crate; what its
  functions do is taken from this repository's doc comments and self-test description
  (`connection/analytics.rs:602-617`).
- SQL sketches in section 3 were not executed. No number in this file is a measurement.
- Whether a taxonomy (family) column or table exists for CHT-19's trigger list.

---

## 6. Rendering technology for the next five years

Constraints, as found in the repository:
- **CSP**: `script-src 'nonce-…' 'strict-dynamic'`, `style-src 'self' 'nonce-…'`, `connect-src 'self'`,
  `default-src 'self'`, no `'unsafe-eval'` and no `'wasm-unsafe-eval'` (`crates/birdnet-web/src/security.rs:65-76`).
  Anything that calls `eval`/`new Function` fails (CLAUDE.md: htmx's `hx-on` already does), inline `style=""`
  is refused, and WebAssembly compilation is refused unless the policy is widened (FOLKLORE: that
  `script-src` without `'wasm-unsafe-eval'` blocks `WebAssembly.instantiate` in current browsers; check in
  Chromium and Safari).
- **One offline binary, vendored assets**: the only third-party chart code is a tree-shaken three.js, one ES
  module of 538,460 bytes in `static/vendor/three/`, built by `tools/vendor-three/` (esbuild over named
  re-exports, `entry.js:1-8`) with a size budget test, and loaded by dynamic `import()` only when the 3D
  terrain is asked for (`static/terrain.js:5-8`). Hand-written figure scripts are 53–84 kB each
  (`discovery.js`, `skysong.js`, `seasons.js`, `terrain.js`).
- **Complete without script**; reduced motion honoured; a table per chart; axe/layout/legibility gates
  (`docs/design/charts.md`, "The kit's contract", "Gates"). Server is a Pi; viewers are phones.

| Candidate | Unlocks here | Cost and risk | CSP | Recommendation |
|---|---|---|---|---|
| Server SVG + `chart-kit.js` (today) | everything in sections 2–3: all are ≤ a few thousand marks | Rust geometry is slow to write; no brushing | passes today | **adopt as the default, permanently** |
| Small enhancement layer on top (own code) | linked brushing between figures on a page, solar/clock and count/rate transitions (CHT-03), scrubbing a date range | must be written; keep each under the size of the current scripts | passes (nonce'd module) | **adopt** |
| D3 modules (scale, shape, transition, brush), vendored like three.js | the same layer with less hand-written maths; smooth, interruptible transitions | a second vendoring pipeline; duplicates scales that live in `kit.rs`, so server and client can disagree | FOLKLORE: d3 core modules do not use `eval`; check each chosen module's source and its inline-style writes (`selection.style` sets CSSOM properties, which CSP allows, but verify under this policy) | adopt for named figures only (brush, transitions) |
| Observable Plot | fast authoring of faceted statistical charts client-side | charts would exist only with script: breaks "complete with no script", the table alternative and the Rust property tests; its look is the generic one the maintainer does not want unless restyled | FOLKLORE: no eval known; size is several hundred kB with D3; check both | **reject** for the product; fine for exploratory notebooks outside it |
| Vega-Lite | declarative specs that double as a method record | compiles expressions with `new Function` by default; a CSP-safe interpreter exists (FOLKLORE: `vega-interpreter`, slower; check it covers the transforms used); large bundle; dashboard look | fails as shipped; interpreter build needed | **reject** |
| Canvas 2D | dense marks: every detection of a year as a dot (skysong at scale), the actogram at day × 5-minute resolution, spectrogram tiles | no DOM per mark: needs a parallel table and keyboard reading built by hand, as `terrain.js` already does | passes | adopt for named figures: actogram at fine grain, all-detections sky, spectrogram gallery |
| WebGL via the vendored three.js | today: only the soundscape terrain's 3D view. 3D is honest only where the data has three ordered measures of equal standing; hour × day × amount is one. A second honest use: a spectrogram "waterfall" of one dawn (time × frequency × energy) | already paid for; each new class imported adds to the 538 kB module and the budget test says how much; GPU on old phones | passes (it runs today) | keep for the terrain; adopt for at most one more figure; reject 3D bars, globes, orbits |
| WebGPU | nothing the data here needs | uneven phone support (FOLKLORE; check current Safari/Android) | n/a | reject for now |
| deck.gl / MapLibre | only a spatial story: the mirror has `Lat`/`Lon` but they are the station's own fixed point; multi-station (`docs/design/multi-station.md`, not opened) would be the first | large bundles, tile source needed offline, worker blobs (FOLKLORE: MapLibre needs `worker-src blob:` or its CSP build; check) | needs policy changes | **reject** until there are several stations |
| DuckDB-WASM in the browser | cross-filtering a year of detections on the phone without round trips | FOLKLORE: tens of MB of wasm, needs `'wasm-unsafe-eval'` and worker allowances, heavy on a phone; the behavioral extension would need its own wasm build (not checked that one exists); a second query engine to keep in agreement with the server's (the two-engine rule at `queries.rs:100-118` already costs care) | fails without widening the CSP | **reject**. Cross-filter by asking the Pi: htmx swaps of server SVG, cached |
| Scrollytelling (IntersectionObserver + CSS, own code) | Year in review as a told story: the year's calendar fills, the arrivals land on it, the bump chart draws month by month, each step a sentence from `insight.rs` | writing, not technology; every step must also be a static section in order for no-script, print/PDF and reduced motion | passes | **adopt for Year in review and a first-week "meet your station" page only** |

FOLKLORE throughout this table: every statement about a library's size, eval use or worker behaviour is from
general knowledge, not from files here; versions change these. Check by vendoring the candidate, loading it
under the real policy and listening for `securitypolicyviolation` (CLAUDE.md: `page.evaluate` cannot answer
this).

Recommended architecture:
1. Rust-rendered SVG stays the single source of every figure, with its table, method and tests; no figure exists only in script.
2. One small vendored enhancement layer (own code, optionally d3-brush/transition) adds linked selection and explanatory transitions.
3. Canvas 2D for the two or three figures with more marks than SVG carries, each with a server-rendered still and table behind it.
4. three.js stays lazy and confined to the terrain (and at most one spectrogram waterfall); no map, no in-browser database, no chart grammar library.
5. Narrative pages are ordinary sections revealed on scroll, so print, no-script and reduced motion get the same story.

---

## 7. Grounding in ornithology and research statistics

What the station is, statistically: **one fixed passive acoustic recorder**. It stores detections with a
score (`Confidence`), a daily effort ledger and an hourly one (`recording_effort`, `recording_effort_hourly`),
weather where configured (`weather`), hourly sound levels (`sound_levels`, `sound_level_broadband`) and review
verdicts (`detection_reviews`, status confirmed/rejected, `birdnet-db/src/migration.rs:265-273`; mirrored as
`review_verdict`). Four limits apply to every method below and must be printed with the figure:
(a) **one site, no spatial replication**: nothing here estimates a population, a density or a range;
(b) **scores are uncalibrated** (ACC-09, RES-30); (c) **reviews are not a random sample** (RES-07, ACC-08);
(d) **no record of analysed-and-empty windows or of sub-threshold scores** (RES-04), so recall is unknowable
and an absence is "not detected above threshold while listening".

Every sentence in this section about what the literature does or what a journal expects is **FOLKLORE**:
it is from general knowledge, cites no paper, and was not checked against a primary source. The "check"
given with each is the kind of source to confirm it in. Numbers G-01 … G-10 are method notes, not new defects.

### G-01: Reporting frequency per unit effort (build first)
- Method: share of listened days (or listened dawn hours) with at least one detection, per week of year.
  FOLKLORE: the standard summary of checklist and recorder data; check a methods text on citizen-science
  occurrence summaries.
- Data supports: fully (detections + `recording_effort`). Cannot say abundance.
- Uncertainty: Wilson interval on days heard ÷ days listened; hollow under 3 listened days.
- Connects: CHT-16, UIC-02, RES-26 (a "listened" day needs only 60 s: raise the floor first), RES-01.
- Gate: a week with half the listened days and the same heard days shows twice the frequency and a wider interval.

### G-02: Detection histories: naive occupancy versus detection-corrected
- Method: split a season into repeat "visits" (days, or dawn sessions); a species' history is 0/1 per visit;
  a single-season occupancy model separates "present" from "detected given present". FOLKLORE: the occupancy
  framework of the wildlife-monitoring literature; check a standard occupancy-modelling text, and the
  passive-acoustic literature on false positives in automated detections, which that basic model assumes away.
- Data supports: at one site occupancy itself is a single 0/1 and not estimable as a proportion; what *is*
  estimable is **per-visit detection probability p** for a species within a window where it is assumed
  present, and from it "how many silent days before absence is credible" (the day n where (1−p)^n < 0.05).
  That is the honest single-site product, and it is what tells B "not heard for 9 days: probably gone".
  With several stations (multi-station) the full model becomes possible. False positives bias p upward; use
  reviewed-confirmed species or a per-species score floor.
- Uncertainty: binomial interval on p; the closure assumption stated; figure = per species, a dot-and-whisker
  for p and a "days to 95 % confidence of absence" bar.
- Connects: CHT-11 (the presence barcode is the detection history drawn), RES-02 (daily effort resolution;
  the hourly ledger now exists, `migration.rs:2754`), RES-04, RES-19.
- Gate: simulated history with known p = 0.5 over 60 visits: the interval covers 0.5; a species with 3
  detections gets "too few to say" (CHT-23).

### G-03: Vocal activity rate, named as such
- Method: detections (or detection-positive minutes) per hour of analysed audio. FOLKLORE: the PAM literature
  calls this a vocal activity rate or index and warns that it is not abundance; check a review of acoustic
  indices of abundance.
- Data supports: yes, per day and per hour. Prefer **detection-positive minutes** to raw detections:
  overlapping windows multiply rows (ACC-19) and one singing bird fills a run.
- Uncertainty: the Poisson interval in `phenology/abundance.rs:517` is a floor (its own doc says so, `:308-312`);
  show a day-block bootstrap band instead, because detections cluster.
- Connects: RES-15 (rename "abundance"), RES-03, RES-18, UIC-02, CHT-09.
- Gate: doubling `OVERLAP` on the same audio leaves the positive-minute rate unchanged within tolerance.

### G-04: Diel activity as circular data in sun-relative time
- Method: time of day is an angle. Summaries are the mean direction and mean resultant length (how
  concentrated), and the display is a kernel density on the circle, computed in time relative to sunrise and
  sunset so seasons can be pooled. FOLKLORE: circular statistics as used for activity patterns in the
  camera-trap and acoustic literature; check a circular-statistics text for the estimator and for when a
  bimodal (dawn and dusk) distribution makes the mean direction meaningless.
- Data supports: yes (timestamps, station coordinates, hourly effort to weight by listening).
- Uncertainty: bootstrap band on the density over **days**; report the resultant length with n days; for
  bimodal species show the two modes, not a mean.
- Figure: drawn **unrolled** as a linear density per species on a sun-relative axis (CHT-05), which keeps the
  estimator circular and the picture readable; overlap between two species' densities as a single coefficient
  for "do they share the morning".
- Connects: CHT-03, CHT-05, CHT-13, RES-17, UIR-18, UIC-26.
- Gate: a species at 23:50 and 00:10 has its mode at midnight, not at noon (the arithmetic-mean failure).

### G-05: Phenology: passage dates with intervals, and a smoothed seasonal curve
- Method: first, median and last passage as quantiles of detection days, and a smooth seasonal curve
  (a penalised spline or similar) of the effort-corrected rate. FOLKLORE: quantile-based passage dates and
  smooth seasonal models are the migration-phenology convention because first dates depend on effort and
  abundance; check a phenology-methods review.
- Data supports: within-year curves now; between-year change only after several years. The repository already
  computes 5th/95th percentile days and an effort-gap flag (`phenology/timing.rs:106-129`).
- Uncertainty: bootstrap over days for each quantile date, drawn as a horizontal whisker; the curve with a
  band; an arrival preceded by an outage hatched, not dated.
- Connects: CHT-09, CHT-22, RES-19, UIC-25 #1-3. A smoother is a fitted model: say so on the figure, which
  the charts' "nothing is fitted" rule (`viz/assembly.rs:15`) does not yet allow for; that rule needs a clause.
- Gate: one planted false positive in January does not move the median date and widens only the first-date whisker.

### G-06: Species accumulation, rarefaction and richness estimators
- Method: accumulation of species over listened days, sample-based rarefaction to compare periods at equal
  effort, and an incidence-based estimate of species not yet detected. FOLKLORE: the rarefaction and
  nonparametric richness-estimator literature; check a biodiversity-statistics text for which estimator suits
  incidence (day-by-species) data.
- Data supports: yes from day × species incidence. False-positive species inflate every estimator: one wrong
  singleton raises the "unseen" estimate, so run it on reviewed or score-floored lists and show both.
- Uncertainty: the randomisation band from resampling day order; estimator shown as a range, labelled estimate.
- Connects: CHT-07, UIC-25 #4, UIR-25, ACC-18 (non-bird classes stored as species).
- Gate: two periods with the same pool and different effort have equal rarefied richness within the band.

### G-07: Hill numbers instead of raw Shannon
- Method: report diversity as effective numbers of species of order 0, 1 and 2 (richness, exponential of
  Shannon, inverse Simpson), which are in units of species and comparable. FOLKLORE: the diversity-partitioning
  literature's recommendation; check a primary source on Hill numbers. The page prints Shannon H′ and evenness
  (06 row 13; `birdnet-timeseries/src/queries/diversity.rs:1-4`).
- Data supports: the arithmetic, yes. But the "abundances" are detection counts, which measure vocal output,
  not individuals: a loud wren dominates. Compute on detection-positive minutes or on incidence, and call it
  "acoustic diversity".
- Figure: a diversity profile (order q on x, effective species on y) per season, three lines to compare.
- Uncertainty: day-block bootstrap band. Connects: CHT-07, RES-14.
- Gate: a community of 8 equally heard species gives 8 at every order.

### G-08: Score-to-probability calibration with a reliability diagram
- Method: from reviewed detections, share confirmed per score bin (or a logistic fit of confirmed on the
  logit score), per species; plot observed share against score with the diagonal. FOLKLORE: reliability
  diagrams from the forecast-verification and classifier-calibration literature, and the PAM practice of
  per-species score thresholds from a validated sample; check both.
- Data supports: only where reviews exist, and only honestly where they were **sampled at random within
  score bins**; today's queue is not (RES-07) and reviewers see the score (same id). Until a stratified random
  review mode exists the figure must be titled "of the detections you chose to review". Sub-threshold scores
  are not stored (RES-04), so the curve starts at the threshold.
- Uncertainty: Wilson interval per bin, n printed in each bin, bins under 10 hollow.
- Connects: ACC-03, ACC-07, ACC-08, ACC-09, RES-06, RES-30, UIC-25 #7.
- Gate: a simulated reviewer confirming with probability equal to the score yields points on the diagonal
  within their intervals.

### G-09: Weather and noise as covariates
- Method: a regression of positive minutes per listened hour on wind, rain, temperature and sound level,
  with hour-of-day and day-of-year as smooth terms, shown as partial-effect curves. FOLKLORE: generalised
  additive models are the usual tool for this in ecology; check a generalized-additive-model text, and the PAM literature on wind and
  rain masking detections.
- Data supports: partly. Weather is a forecast-model product of mixed provenance (RES-21); sound levels rest
  on an unverified offset and a ~5 % sample (RES-20), so use them as relative, not dB SPL. Effects mix
  behaviour with detectability and cannot be separated at one microphone: say so on the figure.
- Fitting a generalized additive model in Rust on a Pi is a real cost; the stratified binned display in CHT-21 gives most of the
  figure with no model and should come first.
- Uncertainty: band per curve; rug of observations under it so empty ranges are visible.
- Connects: CHT-21, UIC-25 #9, RES-20, RES-21.
- Gate: on a fixture with no weather effect the band covers zero effect across the range.

### G-10: Nocturnal flight call nights
- Method: count of calls per night hour between dusk and dawn in the migration windows, per night, against
  wind and cloud. FOLKLORE: nocturnal-flight-call monitoring practice; check its literature for which species
  a general classifier detects reliably at all, which is the weak point.
- Data supports: uncertain. The models are trained mainly on song (not verified here), the night filter's
  genus list is incomplete (ACC-11), and corroboration rules may discard single short calls (ACC-10). Build
  only after a reviewed sample of night detections exists.
- Figure: a night × hour strip for the season with calls as ticks, nights not listened hatched.
- Connects: ACC-11, ACC-10, CHT-04 (the actogram's night band shows the same data at no extra cost).
- Gate: none proposed until the reviewed sample exists.

### Top five to build first
1. **G-01 / CHT-16 reporting frequency** — everything needed is stored; every reader uses it.
2. **G-04 / CHT-05 sun-relative diel densities** — fixes three existing charts with one axis and one estimator.
3. **G-02 detection probability and "days to credible absence"**, drawn on CHT-11's presence barcode — the
   crossover figure no hobby station shows, honest at a single site.
4. **G-08 reliability diagram**, together with a random stratified review mode (RES-07) — without the
   sampling change it is not publishable; with it, it underwrites every other figure.
5. **G-05 phenology with intervals** on the existing timing query, then G-06/G-07 as one "acoustic diversity"
   card.
