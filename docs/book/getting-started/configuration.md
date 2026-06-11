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
| `BIRDNET_RECORDING_SCHEDULE` | `--recording-schedule` | — | `all-day` |
| `BIRDNET_SEGMENT_DURATION` | `--segment-duration` | `RECORDING_LENGTH` | `15` |
| `BIRDNET_OVERLAP` | `--overlap` | `OVERLAP` | `0.0` |
| `BIRDNET_SF_THRESH` | `--sf-thresh` | `SF_THRESH` | `0.03` |
| `BIRDNET_PRIVACY_THRESHOLD` | `--privacy-threshold` | `PRIVACY_THRESHOLD` | `0.0` |
| `BIRDNET_QUALITY_FILTER` | `--quality-filter` | — | disabled |
| `BIRDNET_APPRISE_URL` | `--apprise-url` | `APPRISE_URL` | — |
| `BIRDNET_NOTIFY_CONFIDENCE` | `--notify-confidence` | — | `0.8` |
| `BIRDNET_DEADMAN_HOURS` | `--deadman-hours` | `DEADMAN_HOURS` | `24` (`0` = off) |
| `BIRDNET_BIRDWEATHER_TOKEN` | `--birdweather-token` | `BIRDWEATHER_TOKEN` | — |
| `BIRDNET_BIRDWEATHER_URL` | — | `BIRDWEATHER_URL` | public BirdWeather |
| `BIRDNET_MQTT_HOST` | `--mqtt-host` | `MQTT_HOST` | — |
| `BIRDNET_MQTT_HA_DISCOVERY` | `--mqtt-ha-discovery` | — | disabled |
| `BIRDNET_MAX_FILES_PER_SPECIES` | `--max-files-per-species` | `MAX_FILES_SPECIES` | `0` |
| `CADDY_PWD` / `CADDY_USER` | — | `CADDY_PWD` / `CADDY_USER` | auto-set on bare-metal install; user `birdnet` |
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

## Web-UI-only settings

These are stored in the SQLite settings table and have **no** environment variable or `birdnet.conf` equivalent. Set them at `/admin/settings` — see [Settings & Detection](../admin/settings.md).

| Setting | Where | Note |
|---|---|---|
| Detection confidence threshold | Detection | Per-species overrides also live here |
| Detection sensitivity (0.5–1.5) | Detection | Also `SENSITIVITY` in `birdnet.conf` for BirdNET-Pi compat |
| Email / SMTP notifications | Notifications | |
| Rare-bird quarantine rules | Species | |
| BirdWeather station details | BirdWeather | Token can also be set via env var |

> **Data retention is not time-based.** There is no `recording_days` setting. The disk manager purges the oldest recordings once the disk hits `DISK_PURGE_THRESHOLD` (default 95%) and keeps at most `BIRDNET_MAX_FILES_PER_SPECIES` per species. Pin clips you want to keep with the **lock** action on any detection.
