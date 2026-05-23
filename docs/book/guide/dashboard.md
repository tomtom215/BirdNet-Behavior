# The Dashboard

The dashboard (`/`) is the right-now view — the page you leave open. It answers "what's happening in the yard this minute?" before you ask.

![The dashboard](../images/dashboard.png)

## What you're looking at

- **The hero** leads with a plain-English headline ("The yard is *singing*.") and a live signal card showing the last 30 seconds of audio as an animated waveform, with the input device, sample rate and model version.
- **The stat row** gives the four numbers that matter: detections all-time, species, today's count, and a rolling 60-minute "last hour" tally.
- **Live feed** ("Detections as they happen") streams new detections as they arrive. Each row shows the time, a species avatar (its four-letter banding code in the species' own color), the common and scientific name, a mini call-waveform, the confidence, and a play button. A freshly arrived detection rises in with a brief moss-colored pulse.
- **Top species** ranks today's most-heard birds with per-species sparklines.
- **Species × hour** is a compact heatmap of the day's activity by species and hour.

## Light & dark

The whole UI honors your operating system's color-scheme preference and remembers any manual override. Dark mode is a cool "observatory" black tuned so the moss and dawn accents glow rather than muddy.

![The dashboard in dark mode](../images/dashboard-dark.png)

> Every page carries the same top navigation, theme toggle and global species search, so you're never more than one click from any screen.
