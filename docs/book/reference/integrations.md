# Integrations Reference

Wiring BirdNet-Behavior into the rest of your stack — MQTT, Home Assistant, Prometheus, BirdWeather and webhooks.

## MQTT

A pure-Rust MQTT 3.1.1 client publishes detections to any broker (Mosquitto, EMQX, Node-RED, …). Enable it with at least:

```dotenv
BIRDNET_MQTT_HOST=192.168.1.10
# optional: port, username/password, QoS, retain, and a topic prefix
```

### Topics

Topics are built from a configurable **prefix** (default `birdnet`):

| Topic | Payload |
|---|---|
| `birdnet/detection/<Scientific_Name>` | A detection (JSON, below). Spaces in the name become underscores. |
| `birdnet/status` | Online/offline status of the station. |
| `birdnet/stats/today` | Rolling daily totals. |

Subscribe to everything with `birdnet/detection/#`.

### Detection payload

```json
{
  "timestamp": "2026-05-22T21:10:31",
  "scientific_name": "Melospiza melodia",
  "common_name": "Song Sparrow",
  "confidence": 0.83,
  "confidence_pct": 83,
  "file_name": "Song_Sparrow-...wav",
  "rtsp_id": null
}
```

## Home Assistant

```dotenv
BIRDNET_MQTT_HOST=192.168.1.10
BIRDNET_MQTT_HA_DISCOVERY=1     # or the --mqtt-ha-discovery flag
```

With discovery enabled, the station publishes Home Assistant **MQTT discovery** config under the `homeassistant/` prefix, so it registers itself automatically — no YAML to write. The latest detection (species, confidence) and the daily stats appear as entities you can drop on a dashboard or trigger automations from ("flash the porch light when an owl is heard after dark").

## Prometheus metrics

`GET /api/v2/metrics` returns the standard Prometheus text format. The exported series:

| Metric | Type | Meaning |
|---|---|---|
| `birdnet_detections_total` | counter | Total detections since start, labeled by `species` and integer `chunk_offset` (s). |
| `birdnet_inference_duration_seconds` | histogram | Per-chunk inference latency (decode → prediction). |
| `birdnet_db_write_duration_seconds` | histogram | SQLite insert latency for one detection row. |
| `birdnet_audio_source_up` | gauge | `1` if an audio source is producing samples, else `0`. |
| `birdnet_watchdog_pings_total` | counter | Successful systemd `WATCHDOG=1` notifications sent. |

A starter Grafana dashboard lives at [`docs/grafana-dashboard.json`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/grafana-dashboard.json).

```yaml
# prometheus.yml
scrape_configs:
  - job_name: birdnet
    metrics_path: /api/v2/metrics
    static_configs:
      - targets: ["birdnet.lan:8502"]
```

## BirdWeather

Share detections with the [BirdWeather](https://www.birdweather.com/) network using your station token:

```dotenv
BIRDNET_BIRDWEATHER_TOKEN=your-station-token
```

## Apprise & webhooks

For notifications to 80+ services (Telegram, Slack, Discord, Pushover, ntfy, email…), set a single `BIRDNET_APPRISE_URL`. For fully custom integrations, the [alert-rules engine](../admin/notifications.md#alert-rules) can fire a **webhook** with a configurable method and body when a detection matches your conditions (species pattern, confidence range, hour, day-of-week).

See [Notifications & Integrations](../admin/notifications.md) for the channel walkthrough.
