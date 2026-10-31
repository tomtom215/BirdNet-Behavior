# Insights and organisation: a plan

Written 2026-10-31 for three requests: better organisation of the pages and charts, **all** of what
`duckdb-behavioral` offers, and insights from the casual birder up to a researcher. Each claim says how
it is known: **VERIFIED** (a command run in this tree, named), **DOCUMENTED** (the extension's own
documentation at tag v0.10.1, read by a fetch agent and not by hand), **IDEA** (reasoned, not tried).
Nothing here is built unless the last column says so.

## 1. What the extension is, and how much of it the product uses

**VERIFIED** (a scratch test that loads the embedded extension into a fresh DuckDB 2.0.0 and diffs
`duckdb_functions()` before and after `LOAD`): it registers **nine** names: `sessionize`, `retention`,
`window_funnel`, `window_funnel_events`, `sequence_match`, `sequence_count`, `sequence_match_events`,
`sequence_next_node`, `behavioral_version`. Version 0.11.0 (released 2026-10-30) changes no signature and
adds no function, and is not on the community channel the build embeds from, so it is not the question.

| Function | In the engine and REST API | Drawn by a page | Options it has that nothing uses (**DOCUMENTED**) |
|---|---|---|---|
| `sessionize` | yes (`/analytics/sessions`) | Behavior: bursts of singing | a different gap per row (night vs day); how the count moves with the gap |
| `retention` | yes (`/analytics/retention`) | Behavior: who keeps coming back | up to 32 conditions; cohorts of *first-heard-in-month* species |
| `window_funnel` | yes (`/analytics/funnel`) | Behavior: morning running order | modes: only `strict_increase` is used; `strict_order`, `strict_once`, `strict_deduplication`, `allow_reentry` never |
| `window_funnel_events` | yes (`/analytics/funnel-events`) | **no page** | the timestamp of every step: step-to-step lag |
| `sequence_match` | yes (`/analytics/patterns`) | **no page** | the whole pattern language |
| `sequence_count` | yes (`/analytics/sequence-count`) | Behavior (a count) | non-overlapping repeats of a pattern, per group |
| `sequence_match_events` | yes | Behavior: morning order | the timestamps of a partial chain |
| `sequence_next_node` | yes (`/analytics/next-species`, `previous-species`) | Behavior: neighbouring species | bases: only `first_match` is used; `head`, `tail`, `last_match` never |
| `behavioral_version` | status endpoint | no | diagnostic only |

Pattern syntax in use: `(?N)`, `.*`, `(?t>0)`, `(?t<=N)`. Available and unused: `.`, `(?t>=N)`, `(?t<N)`,
`(?t==N)`, `(?t!=N)`. **VERIFIED** by grep of `crates/birdnet-behavioral/src/queries.rs`
(`FUNNEL_MODE = "'strict_increase'"`; the only base string is `'first_match'`).

**Caution before building on a mode.** The extension's two documentation pages disagree about what
`allow_reentry`, `timestamp_dedup`, `strict_order` and `strict` do (the function page and the quick
reference say different things). This repository's rule is that a scratch probe settles it, so every
insight below that names a mode starts with a probe against DuckDB 2.0.0 + v0.10.1, and its test pins
what the probe printed.

## 2. How the pages are organised today

**VERIFIED** (`nav.rs`; a Playwright count of visible charts and cards per route on the fixture; the
chart count is a lower bound because some charts arrive by HTMX after the count):

* Six homes already, folded from nine tabs plus fourteen "More" destinations: Today, Species, Patterns,
  Recordings, Reports, Station. The top level is not the problem.
* Patterns carries six topic tabs (When active, Dawn chorus, Migration, Who sings together, Trends,
  Behavior) on pages 1,100 to 3,400 px tall holding two to six figures each. Behavior ("the deep tier")
  is last and is the only place the extension appears.
* The same question is answered in several homes: *when* is on Today (hour columns), Patterns/When
  (terrain, streamgraph, grids), Dawn chorus, Species detail (circadian) and Reports; *who* is on Species,
  Together, Migration, Year in review.
* A reader has no way to say how deep they want to go. A first-time reader and a researcher get the same
  sequence; the researcher's controls (the gap, the window, the modes, the export) are not on the page.

## 3. Organisation: three options

**A. Keep the six homes; give every analytic page the same four layers (recommended).**
At a glance (one or two sentences the chart already holds, `pages/insight.rs`) · The picture · Look closer
(the second and third pictures) · Method and export (the function used and its version, the parameters in
force, the table, a CSV and JSON link). Group the Patterns tabs under three labels so the bar says what
kind of question each answers: **When** (When active, Dawn chorus, Trends), **Who** (Together, Migration),
**How** (Behavior, split into *Visits* and *Sequences*). No URL changes (`/patterns?tab=` keys stay).
Cost: low to medium; reversible; no redirects.

**B. A "Depth" switch per page (Glance, Explore, Research).** Hides figures until asked. Simpler to read,
but a reader never finds what a switch hides, and every figure needs a rule for which depth it belongs to.
Not recommended.

**C. A new home, "Insights": a curated feed of findings across everything.** Engaging for casual readers and
a good home for the items in section 4, but a seventh home on a phone bar that holds six exactly, and it
duplicates what the pages say. Better as a strip at the top of Today and each Patterns tab than a destination.

**Decision for the owner:** A, with C's idea as the "At a glance" strip. The URL and tab keys stay either
way; nothing in the nav manifest changes except the grouping labels.

## 4. The insight catalogue

Ordered by who it delights. *Fn* is the extension function that makes it possible; "none" means plain SQL.
Status: **built** (in this branch), **designed** (the data and form are settled), **idea**.

### Curious (anyone who looks out of the window)

| Insight | Question in plain words | Form | Fn | Status |
|---|---|---|---|---|
| Sky of songs | When was each bird heard today, and how sure was the station? | every song on the day's own sky | none | built |
| When each bird joined | When did each bird join my list, and who is newest? | timeline of firsts with portraits | none | built |
| Who led each month | Which birds led which months, and when did that change? | rank-over-time with a month scrubber | none | built |
| The first and last voice | Who sings first at dawn and last at dusk, and how often? | two ranked lists with a share each | `sequence_next_node` `head`/`tail` | idea |
| Your bird's day in one line | When does this bird sing, and is that early or late for the season? | a sentence plus the sun-tracking strip | none | idea |

### Birder (keeps a list, learns calls)

| Insight | Question | Form | Fn | Status |
|---|---|---|---|---|
| Who answers whom | After one bird sings, who is heard within a minute? | a grid of discs, row speaks and column answers (not symmetric) | `sequence_count` with `(?t<=60)` | **built** (engine test watched failing; no REST route yet) |
| The chorus assembling | How long after the first bird do the others join? | step-to-step lag per morning, median and spread | `window_funnel_events` | designed |
| Do new arrivals stay? | Of the birds first heard last month, how many are still here? | retention of first-heard cohorts | `retention` | designed |
| Visit anatomy | How long does a visit last, how many a day, how far apart? | strip plot per species | `sessionize` | designed (partly on the Behavior tab) |
| Closing time | Who is the last voice after a given bird's last song? | a short ranked list | `sequence_next_node` `last_match` | idea |

### Researcher

| Insight | Question | Form | Fn | Status |
|---|---|---|---|---|
| Gap sensitivity | How much does "number of visits" depend on the chosen gap? | counts against gap, 5 to 120 minutes | `sessionize` | designed |
| Order stability | Does the dawn order hold from morning to morning? | a score per morning against the modal order | `window_funnel` `strict_order` | idea (probe the mode first) |
| Pair latency distributions | When B follows A, how long is it, and has that moved over the season? | density per pair, small multiples | `sequence_match_events` | idea |
| Method and export panel | Exactly what was computed, with what parameters, on what version? | a collapsible panel and CSV/JSON links on every analytic figure | `behavioral_version` and the existing REST routes | designed |
| Detection-level export | Can I take the numbers away? | CSV of what the figure drew | none | designed |

## 5. What I would do next, in order

1. Finish and gate what exists (the three new views, the 2D vocabulary, "Who answers whom"), because a
   half-merged chart is worse than none. Done for the views and the grid; the repo's browser gates are listed in
   the handoff with what they printed.
2. "The chorus assembling" next: `window_funnel_events` is in the API and no page draws it; one query and one
   picture. ("Who answers whom" is built, and uses `sequence_count`, which a page drew only as a bare count.)
3. The Method and export panel and the Patterns grouping (option A), because they organise everything
   that comes after.
4. Cohort retention of new arrivals and gap sensitivity.
5. The modes (`strict_order`, `allow_reentry`) only after a probe per mode.

## 6. Not known

* The 540-overload figure in the 0.11.0 notes was not checked against 0.10.1; the nine names were.
* Whether a station with few detections (under about 200 a day) gives "who answers whom" enough pairs to be
  more than noise; the grid must print its own sample size and grey out cells below a floor. **IDEA**.
* Run time of `sequence_count` over 56 ordered pairs on a Raspberry Pi: the extension's own timings are
  from a different machine, were not re-measured upstream since 0.8.0, and say nothing about a Pi.
