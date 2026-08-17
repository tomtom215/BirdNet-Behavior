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
| `birdnet_audio_source_up` | gauge | `1` if an audio source is producing samples, else `0`, labeled by `source` (e.g. `local`, `cam1`). One series per capture source. |
| `birdnet_detection_silence_seconds` | gauge | Seconds since the most recent stored detection — the end-to-end "is it actually detecting?" freshness signal (see [System Health](../admin/system.md)). Absent until the first measurement / on a station with no detections yet. |
| `birdnet_outbound_queue_depth` | gauge | Store-and-forward uploads parked for replay after a network failure, labeled by `kind` (e.g. `birdweather`). A depth that only grows means the uplink or token has been broken for a while. |
| `birdnet_watchdog_pings_total` | counter | Successful systemd `WATCHDOG=1` notifications sent. |

The freshness and queue-depth gauges are the two you want alerts on for an
unattended station:
`birdnet_detection_silence_seconds > <your quiet period>` catches a station
that has gone deaf even though every process looks healthy, and a steadily
climbing `birdnet_outbound_queue_depth` catches a broken uplink before a
season's uploads pile up. A starter Grafana dashboard lives at
[`docs/grafana-dashboard.json`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/grafana-dashboard.json).

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

Uploads are **resilient to a flaky uplink**: a post that fails after its
in-flight retries is parked in the local database and replayed **oldest-first**
when the network returns (bounded so a months-long outage can't fill the disk).
Detection never blocks on the network. Watch the
`birdnet_outbound_queue_depth{kind="birdweather"}` gauge and the "Queued
Uploads" row on the [System](../admin/system.md) page; full mechanics are in the
[Field Deployment Runbook](../field/deployment.md#9-remote-diagnostics-and-monitoring).

### Self-hosted ingest (sensitive species)

Programmes tracking rare or endangered species — where a public observation map
is a poaching risk — can keep observation locations under their own governance
by pointing uploads at their own endpoint that implements the BirdWeather
station API shape:

```dotenv
# config key BIRDWEATHER_URL, or env BIRDNET_BIRDWEATHER_URL
BIRDNET_BIRDWEATHER_URL=https://ingest.example.org/api/v1
```

Only the host changes — the `/stations/<token>/...` path shape is preserved, and
the offline queue and ordered replay above come with it. The active endpoint is
logged at startup so a misdirected station is visible in the first journal
lines, not after a silent season.

## Apprise & webhooks

For notifications to 80+ services (Telegram, Slack, Discord, Pushover, ntfy, email…), set a single `BIRDNET_APPRISE_URL`. For fully custom integrations, the [alert-rules engine](../admin/notifications.md#alert-rules) can fire a **webhook** with a configurable method and body when a detection matches your conditions (species pattern, confidence range, hour, day-of-week).

See [Notifications & Integrations](../admin/notifications.md) for the channel walkthrough.
