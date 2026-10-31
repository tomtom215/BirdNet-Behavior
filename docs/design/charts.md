# Charts

How the station draws a count over time or a distribution, what a chart must never
do, and how to add one. The code is `crates/birdnet-web/src/routes/pages/viz/`
(`kit.rs`, `bars.rs`, `heat.rs`, `trend.rs`, `timeline.rs`) and
`crates/birdnet-web/static/chart-kit.js`; the colours are the chart tokens in
`docs/book/reference/design-tokens.md` ("Chart ramps and data ink").

Every chart is **server-rendered inline SVG that is complete with no script**. The
kit script only adds what a still picture cannot: say what is under the pointer, the
finger or the keyboard; put one series in the foreground; arrive instead of appear.
Under the strict CSP (script-src nonce + strict-dynamic, no inline `style=""`)
dynamic values travel as SVG presentation attributes, CSS classes, or `data-style`.

## The honesty rules

These are the reason the charts are not simply "prettier". Each one is held by a test.

1. **The baseline is zero.** A bar chart's axis starts at zero and tops out on a round
   number at or above the data (`kit::count_ceiling`, `kit::count_ticks`). A count axis
   never has a fractional rule.
2. **Smoothing cannot invent a peak.** Curves are monotone cubics
   (`kit::monotone_path`, Fritsch-Carlson): between two data points the curve stays
   between them, so a spike is never overshot and a plateau stays flat. Two stacked
   bands share one edge curve, so they never part or overlap.
   (`the_monotone_curve_never_leaves_the_range_of_its_neighbours`.)
3. **Quiet is not off, and neither is zero.** A day with no detections is two facts:
   the station listened and heard nothing (an empty cell), or it was not recording
   (a hatched cell that says "The station was not listening"). The second is read from
   the `recording_effort` ledger (`pages/listening.rs`). A station with no ledger
   in the period claims nothing: no hatch, no "off".
   A day that has not happened yet is "Not yet", which is a third thing.
4. **Thickness is the real total.** A streamgraph shows the eight busiest species as
   bands and **every other species as one more band**, so a day's total thickness is
   that day's detections and not "the top eight's".
5. **A second measure gets its own axis.** Temperature is a labelled strip beneath
   Today's hour bars, never a second scale on the same plot (a dual axis invents a
   correlation).
6. **Label the extremes, not every point.** Only the peak (or the busiest and quietest
   day) is written on a chart. A number over every bar is a table drawn badly; the table
   is behind "Show as a table", and the tooltip has the rest.
7. **A label is placed only where the whole text fits** (`timeline::place_in_band`): a
   band too thin gets no direct label and the legend carries it; text is never clipped
   or struck through by its own mark.
8. **A tooltip enhances and never gates.** Everything a tooltip says is also in the
   table, the legend or the readout.
9. **Text never wears a series colour.** The mark carries the identity; the label stays
   in `--chart-label` or `--chart-annot`. (A label set on a band uses `--fg` with a
   `--chart-gap` halo.)

## The vocabulary beyond the bar

Five marks the kit draws besides bars and lines. Each is drawn from the chart tokens only, and each has
a promise a test holds.

* **The sky** (`kit::sky_gradient`, `--sky-night/-dawn/-day/-dusk`). Behind a 24-hour axis, a left-to-right
  wash placed by the station's *own* sunrise and sunset: night, a dawn wash, day, a dusk wash, night. It
  replaces the old grey night rectangles. It is drawn only where the station has a location and the sun
  rises before it sets; with no location the plot has no sky, never an invented one
  (`night_is_shaded_only_when_the_sun_is_known`). The stops' offsets never go backwards, so a short winter
  day squeezes the day wash out and does not invert it.
* **Notches** (`kit::notch_unit`, `kit::notch_pattern`). A seam in the card's colour across a column at every
  whole step of the count, so a column is something to count. The step is 1, 2 or 5 times a power of ten,
  the smallest that leaves at most twelve seams, and the chart says it ("Each notch is 20 detections"). The
  column's height is the true value; the seams only mark the steps. A day chart draws them only on bars at
  least 8 units wide: across ninety thin bars they would be a texture, not a tally.
* **Who a column is made of** (`bars::Stack`). Today's hour columns are stacked by species: the day's eight
  most heard each in their own chart colour from the baseline up in a fixed order, and everyone else as one
  "Other species" band whose height is the hour's total minus the named species. A column is exactly as tall
  as the hour's count (honesty rule 4); the bands add up to it. The key names each colour and says what a
  notch is.
* **Discs** (`heat::disc`, `ck-disc`). A heat grid cell is a disc whose area and colour step are the same
  number, both the square root of the value, so one detection beside two hundred is drawn. A cell with
  something has at least a 2.2 to 2.4 unit disc. A cell with none is a small empty mark, a day the station was
  not listening is hatched, an hour that has not happened is an outline: four states, four marks. Halos
  (a radial gradient in the disc's own colour, laid under every disc so no halo covers a neighbour) are only on
  the weekday grid; on a year of days they would be a smear.
* **Notes and the rail** (`ck-note`, `ck-lead`, `ck-rail`). Where the old chart wrote a bare number over its
  peak, it writes one note in the display face's italic beside the bar ("busiest hour, 9 heard") with a leader
  to it, standing on whichever side has room and clear of the "now" marker. If neither side has room the bare
  number says it instead: the peak is never unlabelled and never written twice
  (`a_peak_with_no_room_for_its_note_is_labelled_with_its_number`). Under the hours, the sun's rail: the day as
  a line, bright between sunrise and sunset, the sun and the moon at its ends, each captioned with its time, and
  the day's length written between them when the captions leave room. The strip on Today is drawn twice, 960
  units for a card and 360 for a phone (the same pair the daily chart uses), and the phone's rail carries bare
  times.

## Choosing the form

* *Totals by hour of the day* are a comparison of magnitudes: columns on a common
  baseline, with the night shaded behind them. A radial "clock" would put 23:00 and
  00:00 next to each other, which is the one thing it has over columns, at the price of
  encoding the same number as an arc whose area grows with its radius; the dawn-chorus
  tab already has the radial chart for the question it suits (who sings when).
* *Hour by weekday* is a grid of cells on the sequential sun ramp, with the totals for
  each weekday and each hour beside it.
* *A year of days* is a calendar (53 weeks by 7 days) on the moss ramp.
* *Who is singing, over time* is a centred stream; *how busy, over time* is a line or
  column chart; *how many kinds, so far* is a rising curve.

Colour follows the job: identity is `--chart-1` .. `--chart-8` in fixed order (a ninth
series is `--chart-other`, never a generated hue); magnitude is `--seq-*` (or
`--seq-moss-*`), stepped by the **square root** of the count and spaced so one detection
and ten are told apart; a state is hatched. The colour key is printed under every heat
map with the counts its steps start at.

## The kit's contract

A chart is `<div class="ck"> <svg data-ck="cols|marks" …> … </svg> [legend] [readout] </div>`.
The attributes are documented at the top of `static/chart-kit.js`; in short:

| Attribute | Meaning |
|---|---|
| `data-ck="cols"` | A reading per column (a day, a month): crosshair, tooltip of every series. |
| `data-ck="marks"` | Each mark carries its own reading (`data-tip`): bars, cells. |
| `data-ck-cols`, `data-ck-series` | JSON: heading, a value per series (`null` = no data), a note; name and key colour per series. Built by `kit::cols_attrs`. |
| `data-ck-x0`, `data-ck-x1`, `data-ck-pos` | Where the columns sit in viewBox units. |
| `data-tip` | `"Heading\nLabel\tValue\tcolour\nNote"`, built by `kit::tip` (escaped, untrusted text). |
| `data-ck-i` | A mark's index; everything sharing one lights together (a bar and its hit box). |
| `data-ck-s` | A mark that belongs to series *k*: hover dims the rest. |
| `data-ck-grid`, `data-ck-major` | Length of a line of marks and its direction, so the arrows move by row or by week. |
| `data-ck-id` | Identity for the entrance: a chart swapped in again under the same id does not replay it. |
| `data-ck-dim="series\|index"` | What falls to `--chart-dim` when one thing is in focus. |
| `.ck-leg[data-ck-s]` | Legend buttons: hover/focus preview, click pins (`aria-pressed`). |
| `.ck-cursor`, `.ck-dot`, `.ck-hit` | The crosshair, the line's marker, and the invisible box over a slot. |

What it does: pointer, touch and keyboard give the same reading (one tab stop per chart;
Left/Right/Home/End, and Up/Down by row, week or series; Escape lets go) and every move
is announced in an `aria-live` region; one series or mark comes forward; the entrance is
CSS only (`.ck-grow`, `.ck-reveal`, `.ck-drawn`, `.ck-fadein` in the `/* charts */` region
of `app.css`) so it is switched off by `prefers-reduced-motion` and by the operator's
Motion setting in the same two places as every other animation, and the kit marks a
refresh `.ck-still` (on `htmx:afterSwap`, before paint) so a 30-second poll does not replay
it. Theme changes need nothing: every colour is a custom property.

The text-fitting script (`chart-text.js`) still enlarges any label that would render under
9.5 px on a phone and hides one that then collides; its `svg[data-readout]` readout
remains for charts that have not moved to the kit.

## Adding a chart

1. Pick the form and the colour job above. Decide what "none heard" and "not listening"
   look like for this chart, and whether there is a second measure that needs its own axis.
2. Write a renderer in `viz/` that returns a `String`: take scales and ticks from
   `kit.rs`; draw bars with `kit::bar_path`, curves with `kit::monotone_path`; colour with
   classes (`ck-c1`..`ck-c8`, `ck-q1`..`ck-q7`, `ck-m1`..`ck-m7`) or `var(--chart-*)`
   references, never a literal. The empty case returns `viz::EMPTY`.
3. Emit the kit attributes (`kit::cols_attrs` or `data-tip` per mark), a `<title>` and
   `<desc>` (`svg_a11y`), the legend as `.ck-leg` buttons when there is more than one
   series, and a table or readout that carries every value.
4. Give gradient ids a prefix from the chart's own id (`kit::id_safe`): a duplicated id on
   a page is a defect (`qa.mjs` counts them).
5. Write the tests that assert the *property*, not a shape: bars inside the viewBox,
   labels inside their band, no overlap, the axis from zero, "off" is not zero.
   Then run the gates below and look at the chart at 1440 and 390 px, light and dark.

## Gates

`tools/visual-qa/charts.mjs` (this kit: hover, keyboard, dim, legend, no layout shift,
reduced motion three ways, no replay, theme), `legibility.mjs`, `layout.mjs`, `axe.mjs`,
`cls.mjs`, `focus.mjs`, `motion.mjs`, `chart-ramps.mjs`, `tokens.mjs`, and
`charts-edge.mjs` (an empty station, one detection, a 1.6-million-detection hour, very
long species names, sixty species, a 365-day range: every chart must render without
overlap). `charts.mjs` supports `MUTATE=notip|nodim|motion|noreplay` to watch each check
go red.

The relational, radial and behavioural charts (the chord diagram and its phone ladder, the dawn
rings, the migration ridgeline, the Visits and Sequences tabs with their four-layer cards and the chorus
disc field, and the soundscape terrain) are described in
[charts-relational.md](charts-relational.md), with their own gates (`relational.mjs`,
`terrain.mjs`).
