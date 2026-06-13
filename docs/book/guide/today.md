# Today

**Today** (`/`) is the home — the page you leave open. It answers "what's
happening in the yard right now?" before you ask, and it's the merge of what
used to be two separate screens (the Dashboard and the Today log): the live
"right now" view and the full searchable day are now one calm page.

> Coming from an older build (or from BirdNET-Pi)? The old `/today` address
> still works — it redirects here.

![Today — the merged home](../images/today.png)

## The five layers

**1 · The glance.** The hero leads with a plain-English, *comparative*
headline — "A *busy* morning." — computed from today's count against your
last 30 days, so a quiet day and a record day read differently at a glance.
Beside it, the **live signal** card shows the last 30 seconds of audio. It is
**honest**: it animates only while real audio is arriving from the capture
device and falls back to a flat baseline labelled **idle** when nothing has
been heard recently — never a fake waveform. A row of pills carries the
recording state, current weather, sunrise/sunset, and your station's name and
coordinates.

**2 · The nudge (only when it matters).** A calm strip appears under the hero
**only** when something needs your eye: rare detections waiting in the
[review queue](./reviews.md), or an **outage** warning when capture has gone
quiet during daylight. When all is well and quiet, it's absent entirely.

**3 · The shape of the day.** The **day strip** plots the whole day on one
24-hour timeline — an hourly histogram of how busy each hour was, an amber
temperature line overlaid on the same axis, labelled sunrise/sunset markers,
and a "now" line. The header carries the day's peak hour, dawn-chorus count
and total.

**4 · The heartbeat.** One unified log. The **live feed** streams new
detections as they arrive — each row a time, a species avatar (its four-letter
banding code in the species' own colour), the names, a mini call-waveform, the
confidence, and a play button; a freshly arrived detection rises in with a
brief moss pulse. The **full day** sits behind one "Show the full day"
disclosure, where you can search (prefix `NOT` to exclude), filter by **Rare /
First today / High confidence**, and **lock**, **re-label** or **delete** any
detection. The list auto-refreshes, so a page left open stays current.

**5 · The support rail.** Today's **top species** (with sparklines), the day's
**best recordings**, a one-line **station-health** readout, and quiet
"looking back" links into [Reports](./reports.md).

## First run

A brand-new station — one that has never heard a bird — shows a friendly
"getting ready" checklist in place of the live signal (microphone detected,
model loaded, disk headroom, listening…) plus illustrated empty states, so the
very first hour is an activation moment rather than a blank page. The page
comes alive the moment the first call lands.

## Light & dark

The whole UI honours your operating system's colour-scheme preference and
remembers any manual override. Dark mode is a cool "observatory" black tuned so
the moss and dawn accents glow rather than muddy.

![Today in dark mode](../images/dashboard-dark.png)

> Every page carries the same top navigation, theme toggle and global species
> search, so you're never more than one click from any home.
