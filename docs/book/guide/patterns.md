# Patterns

**Patterns** (`/patterns`) is the analytics home — "when & where?". It gathers
the station's deeper readings behind one tab strip, each tab leading with a
single picture and a plain-English read, with the dense numbers tucked behind a
"see the numbers" disclosure. A hobbyist never meets six analytical surfaces at
once; a researcher still reaches every one in a click.

| Tab | Question it answers | Covered in |
|---|---|---|
| **When active** | when are they out? | the hour × day-of-week heatmap + by-hour totals |
| **Dawn chorus** | who sings, and when? | [The Dawn Chorus](./dawn-chorus.md) |
| **Migration** | arriving & leaving | [Migration & Phenology](./phenology.md) |
| **Who sings together** | who co-occurs? | the co-occurrence chord + matrix |
| **Trends** | busier or quieter? | weekly detections + species richness |
| **Behavior** | the research tier | [Behavioral Analytics](./analytics.md) |

> The old standalone addresses still work and redirect into the right tab:
> `/heatmap` → `When active`, `/analytics/dawn-chorus` → `Dawn chorus`,
> `/migration` → `Migration`, `/correlation` → `Who sings together`,
> `/timeseries` → `Trends`, `/analytics` → `Behavior`.

Every chart is rendered server-side as inline SVG — no JavaScript charting
runtime — so the page stays light on a Raspberry Pi and works offline. The
three deepest readings have their own pages, linked above.
