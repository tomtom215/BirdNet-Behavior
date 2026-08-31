# Configuration

Settings are read in this priority order (highest wins):

```text
CLI flag (or its BIRDNET_* env var)  >  /etc/birdnet/birdnet.conf  >  built-in defaults
```

A CLI flag and its matching `BIRDNET_*` environment variable are the **same**
setting — the env var is the flag's default, so set one or the other, not both.
The `birdnet.conf` INI keys (kept for BirdNET-Pi compatibility) are consulted
only when neither the flag nor the env var is given.

On top of that, a **SQLite settings table** managed through the web UI at **`/admin/settings`** is the canonical place for everything that has no CLI flag or env var — detection confidence threshold, per-species thresholds, sensitivity, email SMTP, quarantine rules, and so on. You do **not** need to touch any of these to get started.

## Environment variables & CLI flags

The full list lives in `.env.example` and `birdnet-behavior --help`. Each row shows the environment variable, the matching CLI flag, and the `birdnet.conf` INI key (for BirdNET-Pi compatibility).

| Env var | CLI flag | `birdnet.conf` key | Default |
|---|---|---|---|
| `BIRDNET_LATITUDE` / `BIRDNET_LONGITUDE` | `--latitude` / `--longitude` | `LATITUDE` / `LONGITUDE` | — |
| `BIRDNET_ALSA_DEVICE` / `BIRDNET_ALSA_DEVICES` | `--alsa-device` / `--alsa-devices` | `ALSA_CARD` / `ALSA_CARDS` | — |
| `BIRDNET_PIPEWIRE_DEVICE` | `--pipewire-device` | — | — |
| `BIRDNET_RTSP_URL` / `BIRDNET_RTSP_URLS` | `--rtsp-url` / `--rtsp-urls` | `RTSP_URL` / `RTSP_URLS` | — |
| `BIRDNET_LISTEN` | `--listen` | — | `0.0.0.0:8502` |
| `BIRDNET_RECORDING_SCHEDULE` | `--recording-schedule` | `RECORDING_SCHEDULE` | `all-day` |
| `BIRDNET_SEGMENT_DURATION` | `--segment-duration` | `RECORDING_LENGTH` | `15` |
| `BIRDNET_OVERLAP` | `--overlap` | `OVERLAP` | `0.0` |
| `BIRDNET_METADATA_MODEL` | `--metadata-model` | `METADATA_MODEL_PATH` | set by the installer / entrypoint |
| `BIRDNET_METADATA_LABELS` | `--metadata-labels` | `METADATA_LABELS_PATH` | set by the installer / entrypoint |
| `BIRDNET_SF_THRESH` | `--sf-thresh` | `SF_THRESH` | `0.03` (no effect without a metadata model) |
| `BIRDNET_PRIVACY_THRESHOLD` | `--privacy-threshold` | `PRIVACY_THRESHOLD` | `0.0` |
| `BIRDNET_APPRISE_URL` | `--apprise-url` | `APPRISE_URL` | — |
| `BIRDNET_NOTIFY_CONFIDENCE` | `--notify-confidence` | — | `0.8` |
| `BIRDNET_DEADMAN_HOURS` | `--deadman-hours` | `DEADMAN_HOURS` | `24` (`0` = off) |
| `BIRDNET_BIRDWEATHER_TOKEN` | `--birdweather-token` | `BIRDWEATHER_TOKEN` | — |
| `BIRDNET_BIRDWEATHER_URL` | — | `BIRDWEATHER_URL` | public BirdWeather |
| `BIRDNET_MQTT_HOST` | `--mqtt-host` | `MQTT_HOST` | — |
| `BIRDNET_MQTT_HA_DISCOVERY` | `--mqtt-ha-discovery` | — | disabled |
| `BIRDNET_MAX_FILES_PER_SPECIES` | `--max-files-per-species` | `MAX_FILES_SPECIES` | `0` |
| `BIRDNET_CLIP_RETENTION_DAYS` | `--clip-retention-days` | `CLIP_RETENTION_DAYS` | `0` (keep forever) |
| `BIRDNET_DISK_PURGE_THRESHOLD` | `--disk-purge-threshold` | `DISK_PURGE_THRESHOLD` | `95` |
| `BIRDNET_STREAM_RETENTION_SECS` | `--stream-retention-secs` | `STREAM_RETENTION_SECS` | `600` |
| `BIRDNET_STREAM_MAX_MB` | `--stream-max-mb` | `STREAM_MAX_MB` | `512` |
| `CADDY_PWD` / `CADDY_USER` | — | `CADDY_PWD` / `CADDY_USER` | `CADDY_PWD` auto-set on bare-metal install; sign in as `admin` (`CADDY_USER` is environment-only — see [Remote access](../admin/remote-access.md)) |
| `BIRDNET_CORS_ALLOWED_ORIGINS` | — | — | — (same-origin only) |

> **Invalid settings fail fast.** On startup the daemon validates the
> configuration and **refuses to start** on an out-of-range value (e.g. a
> latitude outside ±90, or a malformed `RECORDING_SCHEDULE`) instead of running
> silently degraded. Run `birdnet-behavior --doctor` to check a config before
> deploying it.

> **`BIRDNET_LISTEN` binds all interfaces by default** (`0.0.0.0:8502`), so the
> dashboard is reachable across your LAN out of the box. Viewing is open; only
> the `/admin` panel requires a password. A fresh bare-metal install
> auto-generates `CADDY_PWD` (username `birdnet`) and prints it once in the
> post-install summary — change it via `CADDY_PWD` in `birdnet.conf`. To restrict
> the dashboard to the local machine, set `BIRDNET_LISTEN=127.0.0.1:8502`. The
> authentication (`CADDY_PWD`) and cross-origin (`BIRDNET_CORS_ALLOWED_ORIGINS`)
> settings are covered in [Remote Access & Security](../admin/remote-access.md).

## What the station connects to

Two connections are made **on the station's own initiative**, with no
configuration:

| Host | Why | Turn it off with |
|---|---|---|
| `api.github.com` | Once 60 s after start and every 24 h after: checks whether a newer release exists and logs the answer. It never installs anything — updates are applied only from the admin panel. | `--no-update-check` / `BIRDNET_NO_UPDATE_CHECK=1` |
| `en.wikipedia.org`, `upload.wikimedia.org` | Downloads a photo the first time a species is detected, then serves it from the local cache for ever. | `--image-cache-dir ""` |

One more can happen **once, on a first run**: if the bundled
`duckdb-behavioral` extension cannot be loaded from the local cache or the
copy embedded in the binary, DuckDB tries the community registry
(`extensions.duckdb.org`). Release binaries carry the extension, so this
normally never fires.

Everything else is off until you configure it, and configuring it is the
consent: Apprise, BirdWeather, MQTT, SMTP e-mail, the heartbeat ping, and the
weather poll (`api.open-meteo.com`, itself opt-in via `BNB_WEATHER_ENABLED=1`).
The location button on the settings page calls `ip-api.com`, and only when you
press it.

### Offline mode

```bash
BIRDNET_OFFLINE=1        # or: --offline
```

Turns off **both** default-on connections at once — the update check and the
image downloads — so the station makes no outbound connection you did not ask
for. Integrations you configured explicitly keep working: offline mode is about
unsolicited traffic, not about muting your alerts.

Use it on metered or cellular links, on air-gapped deployments, and wherever
"what does this contact?" needs a single answer. Already-cached species images
are still served; only new downloads stop.

`birdnet-behavior --doctor` reports the current posture under **Outbound
connections**, so you can confirm it rather than infer it.

## Web-UI-only settings

These are stored in the SQLite settings table and have **no** environment variable or `birdnet.conf` equivalent. Set them at `/admin/settings` — see [Settings & Detection](../admin/settings.md).

| Setting | Where | Note |
|---|---|---|
| Detection confidence threshold | Detection | Per-species overrides also live here |
| Detection sensitivity (0.5–1.5) | Detection | Also `SENSITIVITY` in `birdnet.conf` for BirdNET-Pi compat |
| Email / SMTP notifications | Notifications | |
| Rare-bird quarantine rules | Species | |
| BirdWeather station details | BirdWeather | Token can also be set via env var |

> **Retention is off by default.** Nothing is deleted by age unless you ask for it: set **Keep Clip Audio (days)** in the UI (`CLIP_RETENTION_DAYS`) to reclaim the audio of older detections. Independently of that, the disk manager purges the oldest recordings once the disk hits `DISK_PURGE_THRESHOLD` (default 95%) and keeps at most `BIRDNET_MAX_FILES_PER_SPECIES` per species. Pin clips you want to keep with the **lock** action on any detection — locked clips are never purged by either limit.
>
> All three limits, plus the temporary streaming folder's retention and size ceiling, can be set from **Settings → System** in the web UI; you never have to edit a config file to change them. A value given on the command line or as a `BIRDNET_*` variable wins over the UI, which in turn wins over the config file.
>
> When a clip is purged the detection itself is kept — its filename and the date the audio was reclaimed stay on the record, so counts, species lists, trends and exports are unaffected. Only the audio goes.
