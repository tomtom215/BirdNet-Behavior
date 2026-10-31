# Relational, radial and behavioural charts

The Patterns charts that are not a count over time: who is heard with whom (the chord diagram and
its phone ladder), the dawn-chorus rings, the migration ridgeline, the four pictures of the
Behavior tab, and the soundscape terrain. They use the chart kit (`docs/design/charts.md`) and
keep its honesty rules; this page is what they add, and why each picture is the shape it is.

The code is `crates/birdnet-web/src/routes/pages/viz/{cograph,radial,matrix,ridge,bursts,flow,funnel}.rs`,
`pages/{correlation,dawn_chorus,migration,behavioral,terrain}.rs` and `static/terrain.js`. The
styles are the `/* charts2 */` region of `app.css`, which is tokens and nothing else.

## Additions to the kit

`static/chart-kit.js` gained two attributes and two classes, all opt-in and none changing a chart that
does not write them:

| Attribute | Meaning |
|---|---|
| `data-ck-s="3 5"` | A mark can belong to several things: space-separated tokens (a ribbon between species 3 and 5). A single number is the old contract. |
| `data-ck-f="3 5"` | On a mark that has a reading: while it is read, only the marks whose `data-ck-s` holds **all** of these tokens stay in the foreground (`data-ck-dim` must name `series`); the rest fall to `--chart-dim`, and so do the legend buttons that are not in the set. |

`.ck-scroll-to` marks the thing a chart that scrolls sideways inside its card should open on (the
ridgeline's "today", so a phone in October does not open on January), and `.ck-sticky` marks row
names that stay at the card's left edge while the chart moves under them (an SVG has no
`position: sticky`, so the kit shifts them by the scroll offset). Both do nothing until the chart is
wider than its card.

A species' arc says its own number, its partners' and the id of each ribbon that ends on it (`r3`) in
its `data-ck-s`, so reading a species keeps its ribbons and the species they end on; a ribbon says
both of its species and its own id, and its focus is that id alone, so reading it keeps itself and
the two arcs that carry the id. (Its focus used to be the two species' numbers, which a third species
linked to both ends also holds: three arcs stayed in the foreground, not two. The test is
`reading_a_ribbon_does_not_keep_a_species_that_is_only_linked_to_both_ends`.)

## Who sings together

**The data.** Species are ranked by how much overlap they take part in (the sum of their pair
overlaps), the top ten are kept, and a pair's strength is its overlap scaled to the strongest pair
among those ten. The overlap is the pair's share of the five-minute blocks either was heard in
(Jaccard), and it travels with the picture: the reading says "38%", not "0.71 of the strongest".
The species' rank is its colour slot in every picture, so a species keeps its colour when the order
round the ring changes.

**The ring is for wide screens, the ladder for narrow ones.** The ring's names stand upright on
either side of it and the whole picture needs about 760 units of width. At 390 px that is drawn at
about 0.45 of its size, so a 12-unit name is 5.4 px against the 8.5 px floor of the legibility gate,
and a ring's worth of ribbons in 340 px is a knot in any order. A matrix has no such limit. So the
figure carries both, and the stylesheet shows the ring from 1000 px up (where a 12-unit name is
about 9 px) and the ladder below. This was decided by measuring the two against the gate, not by
taste: the ring on a phone fails `legibility.mjs` and the ladder passes.

**The order is chosen, not inherited.** `cograph::chord_order` searches for the arrangement round
the ring with the least weighted crossing (a local search from a dozen deterministic starts: swaps,
then moves; the minimum is NP-hard in general and for ten species the search finds it). The old
diagram stood the species in the order the query ranked them, which has nothing to do with who is
heard with whom; for two clusters that interleave in that ranking it crosses every chord of one with
chords of the other (`the_ring_order_separates_clusters_instead_of_inheriting_the_query_order`). The
ladder uses the same order, so groups that sing together are blocks of dark cells along its diagonal.

**What a mark says.**

* A ribbon's **width** and **colour** (the seven steps of the sequential sun ramp, `ck-q1`..`ck-q7`)
  are one number, how much of their time two species share. Both ends of a ribbon are as wide as the
  link is strong, on one angular scale for the whole ring, so an arc is exactly as long as the
  ribbons that end on it: no species owns a stretch of ring with nothing on it.
* The ribbons are cubic curves that dip towards the middle in proportion to how far apart their ends
  are: neighbours stay near the rim, opposite species pass close to the centre. They used to bend
  through the centre every time, which is what made them one tangle.
* The ring shows the 12 strongest links by default, with a "Links shown" control (12, 24, All; a
  choice that would show the same picture as another is not offered). Species with none of the links
  drawn are not on the ring; they are in the ranked list and the table, which hold every pair.
* Names stand upright, spread apart where arcs are close (`radial::spread_column`) and joined to
  their arc by a hairline. The old tangent names ran into one another. Nothing is written in the
  middle (the old caption sat over the ribbons and needed a halo).
* The ladder draws each pair once, as a staircase, with each name at the foot of its own step.
* "What does a ribbon mean" is under the picture: the ramp, the measured share at its two ends and
  what an arc's length is. A ranked list of the eight strongest pairs sits beside it.

## The dawn-chorus rings

Kept: the radial form, solid rings in the chart colours, the moon dial and the sun. Changed:

* **Every ring-hour is a mark.** A wedge as thick as the ring is at that hour (never thinner than a
  finger can find), in paint order, so the one that shows is the one that is read. The reading says
  the name, the hour span, the count, the hour's share of the bird's day and its busiest hour.
  Left/Right move by the hour, Up/Down from ring to ring (`data-ck-grid="24"`). A ring that is read,
  or whose name in the key beside the chart is hovered, comes forward and the others fall back.
* **The swell is where the bird was.** The count of an hour slot belongs to the slot's middle
  (06:30). The ring was drawn with it at 06:00: every swell half an hour early
  (`a_ring_swells_at_the_middle_of_its_hour`). The curve between hour middles is a monotone cubic
  (`kit::monotone_segments`), wrapped round midnight, so smoothing cannot invent a peak.
* **A sky strip** from the station's coordinates: night, first light (civil dawn), day, last light
  (civil dusk), and the sun on the strip where the colour changes. It is drawn only when the station
  has a location, and a night with no civil twilight (midsummer in the north) simply has no
  twilight band. The times are in the key, in the reader's clock.
* **The middle says something true**: the time now and the window ("60 days"), with the moon dial
  round it. The orphan "chorus 24 h" is gone. The now hand is a solid hairline in the annotation ink
  over a halo; it was a dotted black line that disappeared into the dark rings.
* **"Each bird's day"** replaces a card whose only content was a button: the same species laid flat,
  one row each, a column per hour, the dawn window shaded and the sun's day a line, with the numbers
  in a table under it.

## The migration ridgeline

* **Colour is the season a species peaks in** (spring window, fall window, neither): species are
  told apart by the name beside the ridge, so the colour is free to say something, and twelve
  species do not need twelve hues. Heights are each species' share of its own busiest week; the
  caption says so and the readout carries the real counts.
* **Hover reads a week** (the kit's `cols` mode): every species heard that week with its count, and
  how many different species were heard. The species-per-week strip that used to be a second chart
  is a lane of the same picture on the same weeks.
* **On a phone the ridgeline scrolls sideways** (it is drawn 720 px wide: at 340 px a week is a
  pixel and a name is 5 px), opens on today, and keeps the names at the left edge. A name longer
  than 22 characters is abbreviated the way a birder does ("R. Hummingbird"); the whole name is in
  the ridge's title, the readout and the table.
* **The future is hatched, not drawn as zero**, and the migration windows swap south of the equator.
* **The cut keeps the most seasonal species, not the earliest**: it used to sort by peak week and
  then truncate, which dropped the fall migrants first when more than twelve qualified.
* **"Earliest vs last year −356 d" was a defect.** The fixture's station began on 26 October of last
  year; a nuthatch first heard on 24 December of that year and on 3 January of this one was reported
  as an arrival 355 days early. The tile's guard asked only that the station had been recording a
  fortnight before the species' first date, which any station older than a fortnight passes. Now the
  prior year must have been recorded from its start (within 14 January), and a "first date" in the
  first four weeks of a year is a resident or a winter visitor, not an arrival. A station that cannot
  be compared says "last year was only partly recorded". "Still expected" says "needs a year of
  records" where there was no last year to expect from, and the "Missing" card, which was a permanent
  "Forecast model pending", is the list of birds that arrived around now last year and have not been
  heard yet.

## The Visits and Sequences tabs

The engine's cards are on two tabs of the Patterns home, both under the bar's **How** label: *Visits*
(`?tab=behavior`, the key it always had: bursts, who keeps coming back, neighbouring species) and *Sequences*
(`?tab=sequences`: the morning running order, the chorus assembling, who answers whom).

Each picture is made from the behavioural engine's own output; none invents data, and each keeps its
numbers as a table. Each card has the same four layers, in this order: one **sentence** of what the figure
found (a pure function of the numbers that says its basis and is silent when there is too little to say:
`pages/behavioral_glance.rs` for bursts, the ledger and the neighbours, `pages/behavioral_assembly.rs` for the
chorus), the **picture**, the **table** of the same numbers behind a disclosure, and a **Method and export**
panel (the function and its version, what was asked, what a number counts, the REST route that returns the
same numbers where there is one, and a button that saves the table as a CSV: `frame::Method`,
`static/figure-export.js`). The other Patterns tabs are regrouped under When / Who / How but do not yet have the
four layers.

| Card | Picture | From |
|---|---|---|
| Bursts of singing | A lane per day, the hours along it, a bar for each burst from where it began to where it ended, packed into sub-lanes so none overlaps; the legend brings one species forward | `sessionize` |
| Who keeps coming back | A ledger: species grouped as resident / regular / migrant / rarity, a bar of weeks heard, and six return rates on the moss ramp, each printing its number | `retention` |
| Neighbouring species | A flow with the species in the middle (wide screens) or ranked bars (narrow ones), ribbon width the share of the sessions that had a bird on that side (a session the bird opens has no bird before it, one it closes none after: the engine's denominator) | `sequence_next_node`, both directions |
| The morning running order | A lane per voice, the clock 04:00–09:00 along it, a dot per morning, a thin line joining one morning's dots, a tick for the typical time, the newest complete morning large | `window_funnel`, `sequence_match_events` |
| The chorus assembling | A field of discs: a column is a time allowed (1 minute to 5 hours, evenly spaced rungs), a row is how many of the station's usual dawn voices have joined, in their usual order, and a disc's *area* is the share of mornings with at least that many together within that time; a bold line round the cells is the typical morning (the most voices half of the mornings had), a pale band the middle half. A phone gets its own drawing | `window_funnel`, once per rung, `strict_increase` |

### The chorus assembling

*How long after the first bird do the others join?* The plan named `window_funnel_events` for this and the
probe (against `behavioral` 0.10.1, on DuckDB 2.0) showed it cannot answer it: it reports each step as the
**latest** occurrence of that voice before the next, so the gap between two steps depends on that tie-break and
not on the birds. `window_funnel` is a depth that is monotone in its window, so asking it at a ladder of
windows (`types::ASSEMBLY_WINDOWS`, 14 rungs from 1 to 300 minutes) gives the whole distribution with nothing
fitted or smoothed. Mode `strict_increase` is needed: two voices heard in one audio chunk share an instant and
would otherwise join in either order. The depth is unanchored (a chain may begin at any occurrence of the first
voice) and the boundary is inclusive. The voices are the station's own: those with at least ten dawn songs in
the season, most heard first, then in the order they typically open. The card is silent with fewer than three
voices or seven mornings. The disc's area, not its colour, carries the share, so the picture is readable in
greyscale; the colour only says which voice completes the row.

## The soundscape terrain

The lead card of the "When active" tab: the last 30, 90 or 365 days, the 24 hours across, the days
into the distance (the newest at the front), and the height and the colour of each patch the number
heard. It opens on a flat grid of the same numbers, drawn on a 2D canvas, which is also what a
browser with no WebGL keeps for good.

* **Honesty.** The surface reaches the height of every cell at that cell's middle and never rises
  above its neighbours between (a smoothstep, which cannot overshoot; tested under node). Height and
  colour are the square root of the count. A day the recording ledger says the station was not
  listening, and an hour that has not happened yet, are flat and hatched and read as such: not the
  same as quiet.
* **three.js is loaded only when 3D is asked for**, with a dynamic `import()` from the terrain's own
  module (the strict CSP's `'strict-dynamic'` admits it), from `static/vendor/three/` (see its README:
  version, hashes, how it was verified against the registry's tarball, how to rebuild).
  A test fails if anything else names the file, if the page asks for it before the click or if its
  gzipped size goes over budget.
* **Input.** Drag, pinch, wheel (once the terrain has focus; a bare wheel scrolls the page), arrow
  keys to turn, plus and minus to zoom, Home to reset, Shift and the arrows to move a reading
  cursor. Pointing reads the cell; every reading is also in an `aria-live` line and in the table.
* **Care.** It draws on demand (no animation loop), not while the tab is hidden or the card is off
  screen; the device pixel ratio is capped at 2; reduced motion (the OS or the operator's Motion
  setting) means no entrance; a lost context falls back to the grid; the GPU memory is freed when
  the card goes; the theme is re-read when it changes.
* **Frame time** is measured by `tools/visual-qa/terrain.mjs` in headless Chromium on software GL
  (swiftshader). That is a lower bound for a weak device in one sense (it has no GPU at all) and not
  in another (a Pi's browser has a real but slow one); the number is printed, not promised.

**Not built: a 3D waterfall for the live spectrogram.** The live spectrogram is a streaming canvas
with time along one axis, frequency up the other and energy as colour, drawn at one pixel per bin.
A height-field of it would put time into the distance and energy on the vertical, and the faint
harmonics that identify a bird would sit in the shadow of its own loud fundamental and of every
louder moment in front of it, which is the one thing a spectrogram is for. It would also keep a
WebGL context busy at the frame rate of a kiosk that does nothing else. The 2D canvas is better at
the job, so it stays.

**Measured on the fixture** (headless Chromium, software GL, no GPU): the bundle is 538,460 bytes,
134,086 gzipped at `-9` (budget 136,000, `tests/vendored_three_is_pinned_and_small.rs`); the grid
is on screen with no three.js; a frame while orbiting takes a median of 130 to 170 ms wall-clock
(`terrain.mjs`, 90 frames) of which the CPU submit is 0.5 to 0.8 ms: the wall-clock figure is
SwiftShader rasterising on the CPU and says what a machine with no GPU would feel, not what a Pi's
GPU does. A 2D canvas path would be about a tenth of the size; three.js was used because the brief
named it, and the cost is recorded here rather than argued away.

**Vendored code and the literal gate.** `tests/design_tokens_are_the_only_literals.rs` excuses
`static/vendor/` by name (third-party, hash-pinned, not ours) and nothing else: `static/terrain.js`
is scanned, reads every colour from the page's tokens at run time (`getComputedStyle`, re-read when
the theme changes) and falls back to the system text colour, never to one of ours. A test fails if
the exemption widens beyond `static/vendor/` or covers a library without a README that records its
hash.

## Views built for one question each

Four pictures that are not a count over time and are not the terrain, each the form that suits its question
(a 3D landscape was tried in the plan for each and rejected where it would add occlusion without helping
anyone read the data). Each is complete without script where it can be (a table behind a disclosure, and for
the first three a sentence), each has a phone drawing of its own, and each is held by the node tests that sit
beside its script in `tools/visual-qa/` and by the Rust tests beside its server side.

| View | Where | Question | Form | Script | Server side |
|---|---|---|---|---|---|
| Every song | Today, "When you heard them" (view toggle) | When was each bird heard today, and how sure was the station? | a scatter on the day's real sky: time across, how sure up, the sun's arc between sunrise and sunset, the station's cut-off drawn, "now" marked | `static/skysong.js` | `pages/today_songs.rs`: `GET /pages/today-songs`, JSON |
| Who led, and when it changed | Reports, Year in review | Which birds led which months? | a rank-over-time chart (the line is a rank, which hides the size of a gap) beside a leaderboard of true counts for the chosen month on one zero-based scale, with a scrubber and a Play button | `static/seasons.js` | `year_in_review::seasons_card`, `birdnet_db::monthly_species_counts` |
| When each bird joined your list | Species, Life list | When did each bird join my list, and who is newest? | portraits standing on the day each was first heard, stems landing on the rising list curve; on a phone a month-by-month vertical list | `static/discovery.js` | `pages/life_timeline.rs` |
| Who answers whom | Patterns, Behavior | After one bird is heard, which others are heard within a minute? | a grid of discs (row first, column after), not symmetric | none (server SVG) | `pages/behavioral_answers.rs`, `AnalyticsDb::answers` |

What each promises, and what a test holds it to:

* **Every song** puts each song at the time it was heard and the height of how sure the station was; the only
  nudge is sideways within the same minute and height, never changing the time the reading gives. Nothing below
  the station's cut-off is ever in the list. The sky is the station's own sunrise and sunset, or neutral with no
  location; there is never an invented sun. When the day holds more songs than one answer carries the feed says
  how many it holds and the summary says the picture shows part of them. A refresh keeps what is on screen if it
  fails, and says so.
* **Who led** ranks only birds that were heard that month: a zero is a gap in the line, not a plunge to last. A
  month that has not happened is not a month with none. Bars share one scale across the year, so lengths compare
  between months. Play stops at the end, pauses on any touch, and is not offered under reduced motion.
* **When each bird joined** takes every URL it is given through the base-path helper, falls back to initials for a
  picture that 404s, never requests a picture for a marker that is off screen, and is complete (every species
  reachable by pointer and keyboard) at 2,000 species by turning portraits into a density picture.
* **Who answers whom** counts, per ordered pair, how many times the first was heard and then the second within a
  minute on the same day, without overlap. The same instant is not a reply (without `(?t>0)` a Robin and a
  Blackbird heard in one audio chunk answer each other both ways: `live_answers` was watched failing against that
  pattern), a bird repeating itself is not an exchange, and a pair seen fewer than three times is a small ring and
  never a finding. A best pair at the floor is "nothing stands out yet". The grid prints its own sample size.

## Gates

`tools/visual-qa/relational.mjs` (chord, rings, ridgeline, the Behavior tab: what a pointer, a
finger and a keyboard do), `tools/visual-qa/terrain.mjs` (the mesh as pure geometry under node, and
the 3D view in a browser: lazy load, pixels, orbit, reading, CSP, no WebGL, reduced motion, hidden
tab, theme, pixel ratio, context loss, dispose, frame time), and the Rust tests next to each
renderer. `MUTATE=…` on each gate breaks one thing to watch the matching check go red.
