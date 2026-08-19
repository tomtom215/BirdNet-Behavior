# Settings & Detection

The admin area lives under `/admin`. The **Settings** page (`/admin/settings`) persists everything you change into the SQLite settings table, and the station reads it on the next restart.

### How a setting is resolved

Most settings can be supplied three ways — a CLI flag (or its `BIRDNET_*`
environment variable), a line in `birdnet.conf`, and this page. When more than
one is set, the station resolves them in this order:

1. **An explicit CLI flag or `BIRDNET_*` variable** — if you passed it, it wins.
2. **This page** — what you save here beats the config file.
3. **`birdnet.conf`**
4. The built-in default.

So a Docker station pinned with `-e BIRDNET_SEGMENT_DURATION=30` keeps that value
no matter what the form says, while a bare-metal station that never passes the
flag is governed entirely by this page.

### Settings not on this page

Two things are deliberately configured elsewhere:

- **The admin password.** It is stored as an Argon2id hash in the accounts
  database, seeded from the `CADDY_PWD` environment variable — never as a
  settings row. See [Remote Access & Security](./remote-access.md).
- **Per-source audio properties** (channels, sample rate, gain, RTSP transport).
  Each microphone or stream carries its own, on
  [Audio & Microphones](./audio.md).

## Detection

- **Confidence threshold** — the minimum score a detection must clear to be logged. Raise it to cut false positives; lower it to catch faint calls.
- **Per-species thresholds** — override the global threshold for individual species (useful for a noisy local mimic, or to be stricter about a rare bird).
- **Sensitivity (0.5–1.5)** — the BirdNET sensitivity parameter (also `SENSITIVITY` in `birdnet.conf` for BirdNET-Pi compatibility).
- **Species-frequency filter** — uses your location and the week of the year to down-weight birds that shouldn't be present, with a configurable `SF_THRESH`.
- **Quality pre-filter** — optionally drops segments dominated by rain, wind or other broadband noise before they reach the model.

## Species & quarantine

Rare-bird **quarantine** rules decide which detections are held for manual review instead of being logged automatically — see the [review queue](../guide/reviews.md#reviews-vs-quarantine). You can also maintain per-species allow/exclude lists and image overrides here.

### Allow and exclude lists

The two lists on `/admin/species` decide which birds the station records at all:

- **Exclude** — these species are never recorded. Nothing is written to the
  database, no notification is sent, and nothing is uploaded to BirdWeather.
  Use it for a persistent local false positive, or for a species you would
  rather not log.
- **Allow** — when non-empty, *only* these species are recorded.

Enter either the common name or the scientific name; both work, and case and
surrounding spaces don't matter. Changes take effect within about half a minute
— no restart. `/admin/species/test` previews the decision for every species your
station has seen, using the same code the detection path runs, so what it shows
is what will happen.

An allow list whose entries match no species this model knows is ignored, and a
warning is logged, rather than being taken literally and suppressing everything.

## Other categories

The Settings sidebar also covers **Location**, **Audio** (see [Audio & Microphones](./audio.md)), **Notifications & Email** (see [Notifications & Integrations](./notifications.md)), **MQTT / Home Assistant**, and **System**.

## Data quality

The **Quality** dashboard (`/admin/quality`) summarizes the health of your detection database — the confidence distribution, a 30-day confidence trend, an hourly quality profile, and a ranked list of low-confidence species that are good candidates for a stricter per-species threshold.
