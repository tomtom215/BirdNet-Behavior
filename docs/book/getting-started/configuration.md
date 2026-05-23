# Configuration

Settings are read in this priority order (highest wins):

```text
CLI flags  >  environment variables  >  /etc/birdnet/birdnet.conf  >  built-in defaults
```

On top of that, a **SQLite settings table** managed through the web UI at **`/admin/settings`** is the canonical place for everything that has no CLI flag or env var — detection confidence threshold, per-species thresholds, sensitivity, email SMTP, quarantine rules, and so on. You do **not** need to touch any of these to get started.

## Environment variables & CLI flags

The full list lives in `.env.example` and `birdnet-behavior --help`. Each row shows the environment variable, the matching CLI flag, and the `birdnet.conf` INI key (for BirdNET-Pi compatibility).

| Env var | CLI flag | `birdnet.conf` key | Default |
|---|---|---|---|
| `BIRDNET_LATITUDE` / `BIRDNET_LONGITUDE` | `--latitude` / `--longitude` | `LATITUDE` / `LONGITUDE` | — |
| `BIRDNET_ALSA_DEVICE` | `--alsa-device` | `ALSA_CARD` | — |
| `BIRDNET_PIPEWIRE_DEVICE` | `--pipewire-device` | — | — |
| `BIRDNET_RTSP_URL` / `BIRDNET_RTSP_URLS` | `--rtsp-url` / `--rtsp-urls` | `RTSP_URL` | — |
| `BIRDNET_LISTEN` | `--listen` | — | `127.0.0.1:8502` |
| `BIRDNET_RECORDING_SCHEDULE` | `--recording-schedule` | — | `all-day` |
| `BIRDNET_SEGMENT_DURATION` | `--segment-duration` | `RECORDING_LENGTH` | `15` |
| `BIRDNET_OVERLAP` | `--overlap` | `OVERLAP` | `0.0` |
| `BIRDNET_SF_THRESH` | `--sf-thresh` | `SF_THRESH` | `0.03` |
| `BIRDNET_PRIVACY_THRESHOLD` | `--privacy-threshold` | `PRIVACY_THRESHOLD` | `0.0` |
| `BIRDNET_QUALITY_FILTER` | `--quality-filter` | — | disabled |
| `BIRDNET_APPRISE_URL` | `--apprise-url` | `APPRISE_URL` | — |
| `BIRDNET_NOTIFY_CONFIDENCE` | `--notify-confidence` | — | `0.8` |
| `BIRDNET_BIRDWEATHER_TOKEN` | `--birdweather-token` | `BIRDWEATHER_TOKEN` | — |
| `BIRDNET_MQTT_HOST` | `--mqtt-host` | `MQTT_HOST` | — |
| `BIRDNET_MQTT_HA_DISCOVERY` | `--mqtt-ha-discovery` | — | disabled |
| `BIRDNET_MAX_FILES_PER_SPECIES` | `--max-files-per-species` | `MAX_FILES_SPECIES` | `0` |

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
