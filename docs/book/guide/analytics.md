# Behavioral Analytics

BirdNet-Behavior turns a stream of detections into behavior. The analytics screens answer *when* the yard is alive and *who* sings with whom.

## Activity heatmap — "When the yard is alive"

The **When active** and **Dawn chorus** tabs of [Patterns](./patterns.md) (`/patterns`) stack several bespoke visualizations:

![The Patterns → When-active tab: the activity streamgraph, the hour × day-of-week grid and detections-by-hour](../images/heatmap.png)

- **Activity streamgraph** — species composition over time, drawn from a centered baseline. Each band is a species and its thickness is that day's count; the eight busiest are drawn one by one and every other species is one more band, so the whole is the day's real total. Point at a day (or use the arrow keys) for every species' count; point at a band, or at a name in the key, to bring that species forward and put the rest in the background. The curves between days are smooth but never rise above a peak or below a trough the counts did not have. A day the station was not listening is hatched, so it is not read as a silent day.
- **Activity grid** — an hour × day-of-week mosaic of rounded cells on the sequential sand-to-brick ramp (one hue family, steps by the square root of the count so one detection and ten are told apart). The colour is the average per day in that slot, so a 90-day window's thirteen Mondays are compared with its twelve Tuesdays fairly. The totals for each weekday are beside the grid and the totals for each hour are under it; the busiest cell is ringed and named. Three states are kept apart: a count is a colour, a quiet hour is an empty cell, and a weekday the station was off for is hatched; an hour of today that has not happened yet is an outline that says "Not yet".
- **Circadian rhythm** — a polar plot where each species occupies its own concentric ring that swells where it's active across the 24-hour clock, with a sky strip (night, first light, day), three-hour ticks, the sun and a solid "now" hand; see [The Dawn Chorus](./dawn-chorus.md).
- **Detections by hour** — the day's totals as columns on an axis from zero, with the night shaded behind them (from your station's sunrise and sunset) and dawn and dusk in colour. It is a bar chart rather than a clock because the question is *how much more*: length on a common baseline is read more accurately than an arc, and the dawn chorus tab already has the clock for *who sings when*. Only the busiest hour is labelled; point at any hour for its count, its share of the day and, on Today, its three busiest species.
- **Seasonal phenology** — a migration ridgeline, one gradient-filled ridge per species, with spring and fall season bands behind; see [Migration & Phenology](./phenology.md).

### Pointing at a chart

Every chart here answers to a pointer, a finger and a keyboard alike. Point at it (or touch it, or Tab to it and use the arrow keys) and a small card reads out what is under you: the day and every species' count, the hour and its share of the day, the cell and its count. **Left and Right** step along the chart, **Up and Down** step by row (the grid), by week (the calendar) or from species to species (the streamgraph), **Escape** lets go, and each step is read aloud to a screen reader. The same figures are always in the table under the chart, behind "Show as a table". Nothing moves when you do this, and with reduced motion on (in your system, or under Settings → Display → Motion) the charts simply appear instead of growing into place. The reasoning behind how these charts are drawn, and what they refuse to do, is in `docs/design/charts.md`.

## Co-occurrence — "Who sings with whom"

The **Who sings together** tab of [Patterns](./patterns.md) (`/patterns?tab=together`) shows which species are detected together — heard in the same five-minute block of the same day. A pair's strength is its overlap: the blocks they share, as a share of the blocks in which either was heard. A pair needs at least three shared blocks to be drawn.

![The co-occurrence matrix and acoustic-network chord diagram](../images/correlation.png)

- **The acoustic network** — the data drawn as a **chord diagram**: each species an arc, each pair that is heard together a ribbon whose width and colour are how much of their time they share. The species stand in the order that crosses the fewest ribbons, so groups that sing together sit together. It shows the 12 strongest links (choose 24 or All). **Point at a name** and only its ribbons stay in the foreground; point at a ribbon and only it and its two species do. The ring needs a wide screen: on a phone you get the **ladder** instead, a staircase matrix with each name at the foot of its own step (read across a row to its name, up a column to the other).
- **Co-occurrence matrix** and **strongest pairs** — the same numbers as a table behind "See the numbers".
- **Top co-occurring species** and a **companion lookup** let you ask "what's usually nearby when I hear X?"

> The richest DuckDB-powered views — activity sessions, species retention, next-species prediction, year-on-year trends — are **on by default**: every release has the analytics engine built in, and the installer and Docker compose enable it automatically. If you've turned it off on a low-RAM board, these panels explain what they'd show and how to switch it back on. See [Configuration](../getting-started/configuration.md).

## Behavior — the research tier

The **Behavior** tab of [Patterns](./patterns.md) (`/patterns?tab=behavior`) draws what the analytics engine computes; each picture keeps its numbers in a table.

- **Bursts of singing** — a lane per day with the hours along it and a bar for each burst, from the moment it began to the moment it ended; bursts of different species that overlap are stacked, never drawn over each other. The legend brings one species forward.
- **Who keeps coming back** — species grouped as resident, regular, migrant and rarity (how much of the station's time they were present), with the weeks each was heard and how often it was heard again a day, two, three, a week, a fortnight and a month later, each a cell that also prints its number.
- **Neighbouring species** — who is heard just before and just after the latest species, as a flow with that species in the middle (ranked bars on a phone); a ribbon's width is the share of sessions.
- **The morning running order** — a lane per voice of your dawn, the clock from 04:00 to 09:00 along it, a dot for every morning's first-heard time, thin lines joining the newest mornings from lane to lane, a tick at each voice's typical time and the newest complete morning drawn large. A line that slants steadily to the right as it goes down is a morning in the usual order.

