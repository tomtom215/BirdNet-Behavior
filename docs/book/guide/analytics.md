# Behavioral Analytics

BirdNet-Behavior turns a stream of detections into behavior. The analytics screens answer *when* the yard is alive and *who* sings with whom.

## Activity heatmap — "When the yard is alive"

The **Heatmap** page (`/heatmap`) stacks several bespoke visualizations:

![The activity heatmap, circadian polar and migration ridgeline](../images/heatmap.png)

- **Activity streamgraph** — species composition over time, drawn from a centered baseline.
- **Activity grid** — an hour × day-of-week mosaic where each cell deepens from the neutral surface through the warm **dawn** hue and into **rare** red for the busiest cells (a theme-aware OKLCH ramp, not a generic rainbow).
- **Circadian rhythm** — a polar plot where each species occupies its own concentric ribbon that swells where it's active across the 24-hour clock, with a night wedge, three-hour ticks, sunrise/sunset markers and a dashed "now" hand.
- **Detections by hour** — a simple bar chart of the day's totals.
- **Seasonal phenology** — a migration ridgeline, one gradient-filled ridge per species, with spring and fall season bands behind.

## Co-occurrence — "Who sings with whom"

The **Co-occurrence** page (`/correlation`) shows which species are detected together.

![The co-occurrence matrix and acoustic-network chord diagram](../images/correlation.png)

- **Co-occurrence matrix** — a grid of every species pair, labeled with four-letter banding codes, shaded by how often the pair is heard together.
- **The acoustic network** — the same data drawn as a **chord diagram**: each species gets an outer arc sized by its total connectedness, and gradient ribbons join the pairs that co-occur most. It's the prettiest way to see the yard's social graph at a glance.
- **Top co-occurring species** and a **companion lookup** let you ask "what's usually nearby when I hear X?"

> The richest DuckDB-powered views — activity sessions, species retention, next-species prediction, year-on-year trends — are **on by default**: every release has the analytics engine built in, and the installer and Docker compose enable it automatically. If you've turned it off on a low-RAM board, these panels explain what they'd show and how to switch it back on. See [Configuration](../getting-started/configuration.md).
