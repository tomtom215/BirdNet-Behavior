# Settings & Detection

The admin area lives under `/admin`. The **Settings** page (`/admin/settings`) is where everything that has no environment variable or CLI flag is configured and persisted in the SQLite settings table.

## Detection

- **Confidence threshold** — the minimum score a detection must clear to be logged. Raise it to cut false positives; lower it to catch faint calls.
- **Per-species thresholds** — override the global threshold for individual species (useful for a noisy local mimic, or to be stricter about a rare bird).
- **Sensitivity (0.5–1.5)** — the BirdNET sensitivity parameter (also `SENSITIVITY` in `birdnet.conf` for BirdNET-Pi compatibility).
- **Species-frequency filter** — uses your location and the week of the year to down-weight birds that shouldn't be present, with a configurable `SF_THRESH`.
- **Quality pre-filter** — optionally drops segments dominated by rain, wind or other broadband noise before they reach the model.

## Species & quarantine

Rare-bird **quarantine** rules decide which detections are held for manual review instead of being logged automatically — see the [review queue](../guide/today.md#rare-bird-review-queue). You can also maintain per-species allow/exclude lists and image overrides here.

## Other categories

The Settings sidebar also covers **Location**, **Audio** (see [Audio & Microphones](./audio.md)), **Notifications & Email** (see [Notifications & Integrations](./notifications.md)), **MQTT / Home Assistant**, and **System**.

## Data quality

The **Quality** dashboard (`/admin/quality`) summarizes the health of your detection database — the confidence distribution, a 30-day confidence trend, an hourly quality profile, and a ranked list of low-confidence species that are good candidates for a stricter per-species threshold.
