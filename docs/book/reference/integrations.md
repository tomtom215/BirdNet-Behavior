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
| `birdnet_detections_stored` | gauge | Detection rows currently in the database, rejections included. Falls when rows are deleted or purged, which is why it is a gauge and does **not** wear a `_total` suffix. |
| `birdnet_detections_rejected` | gauge | Of those, the ones a reviewer has marked rejected. `stored - rejected` is what the web UI displays. |
| `birdnet_species_distinct` | gauge | Distinct species detected, excluding rejected detections. |
| `birdnet_inference_duration_seconds` | histogram | Per-chunk inference latency (decode → prediction). |
| `birdnet_db_write_duration_seconds` | histogram | SQLite insert latency for one detection row. |
| `birdnet_files_analysed_total` | counter | Audio files the pipeline finished analysing, labeled by `source`. **The series that separates "the model is answering nothing" from "the pipeline is not running"** — every other signal is downstream of a prediction the model made, so both states leave them flat and empty. A 15-second segment length gives about 5 760 a day per source. |
| `birdnet_audio_source_up` | gauge | `1` if an audio source is producing samples, else `0`, labeled by `source` (e.g. `local`, `cam1`). One series per capture source. |
| `birdnet_detection_silence_seconds` | gauge | Seconds since the most recent stored detection — the end-to-end "is it actually detecting?" freshness signal (see [System Health](../admin/system.md)). Absent until the first measurement / on a station with no detections yet. |
| `birdnet_outbound_queue_depth` | gauge | Store-and-forward uploads parked for replay after a network failure, labeled by `kind` (e.g. `birdweather`). A depth that only grows means the uplink or token has been broken for a while. |
| `birdnet_watchdog_pings_total` | counter | Successful systemd `WATCHDOG=1` notifications sent. |
| `birdnet_detection_write_failures_total` | counter | Detections the model produced and the database refused — a detection the station heard and could not keep. Should stay `0`; see below. |
| `birdnet_notifications_dropped_total` | counter | Notifications that never left the station, labeled by `reason`: `circuit_open` (the destination is considered down after three consecutive failures), `rate_limited` (over `NOTIFY_RATE_PER_MINUTE`), `send_failed` (the destination refused or was unreachable), `no_destination` (nothing configured that this station can deliver to). Detection notifications dominate this on a busy station; an alert about the station itself is exempt from the rate limit and is retried at every poll until it lands, so `circuit_open` rising while a health condition is open means the operator is not being told. |
| `birdnet_noise_floor_dbfs` | gauge | The station's measured background noise floor per capture `source`, averaged over the last 7 days. Typical quiet outdoor background is −60 to −40 dBFS. |
| `birdnet_noise_floor_drift_db` | gauge | How far a source's noise floor has moved against **its own** preceding 30-day average, in dB. Absent for a source with no baseline yet — "never measured" is not "unchanged". |

> **Renamed.** Three gauges used to carry a `_total` suffix, and one
> of them — `birdnet_detections_total` — collided with the per-species counter
> above. A metric name may have only one type, so the exposition was rejected
> outright by `promtool check metrics`, Telegraf's `inputs.prometheus` and the
> Python client, and a Prometheus server silently merged a *decreasing* gauge
> into the counter. The gauges are now `birdnet_detections_stored`,
> `birdnet_detections_rejected` and `birdnet_species_distinct`; the counter
> keeps the `_total` name that the convention reserves for it. Update any
> dashboard or alert rule that referenced the old gauge names — the bundled
> `docs/grafana-dashboard.json` already is.

`GET /api/v2/health?strict=1` is the probe to point a **pager** at. The plain
`/api/v2/health` answers `200` whenever the database is serving, which is right
for the container health check — Docker restarts an unhealthy container, and a
station whose detection daemon is down is exactly the one that must stay up to
be diagnosed. The strict form additionally returns `503` when the detection
daemon is not running, so a monitor that should wake a human can get a red out
of the same endpoint. Both report `detection_daemon` and
`detection_silence_secs` in the body either way.

The freshness and queue-depth gauges are the two you want alerts on for an
unattended station:
`birdnet_detection_silence_seconds > <your quiet period>` catches a station
that has gone deaf even though every process looks healthy, and a steadily
climbing `birdnet_outbound_queue_depth` catches a broken uplink before a
season's uploads pile up. Alert on **any** increase in
`birdnet_detection_write_failures_total` as well: it is zero on a healthy
station, and non-zero means a full or read-only disk, a locked database, or the
one local hour daylight-saving repeats each autumn (see
[Time synchronisation](../field/deployment.md#6-time-synchronisation)). A
The two noise-floor series answer the question no other gauge here can. A
microphone that fails outright is caught by `birdnet_audio_source_up` and by the
detection deadman. A microphone that merely goes **deaf** — water in the capsule,
a spider's web across the port, a connector loosened by a year of thermal cycling
— keeps its process alive and its gauge at `1`, and shows up only as fewer
detections. So does the end of the breeding season. The background noise floor
does not stop when the birds do, so a large, sustained *negative*
`birdnet_noise_floor_drift_db` on one source, with nothing else changed, points at
the equipment.

Measured on this project's own test recording, a capsule at 2 % sensitivity reads
**35 dB lower** on the noise floor (−77.3 against −42.5 dBFS) while its SNR barely
moves (2.9 against 2.7 dB) — attenuation scales signal and background together, so
SNR is blind to it and the noise floor is not.

No threshold is shipped, deliberately: a noise floor moves for real reasons —
weather, season, a road, leaf-out — and a number picked without a season of real
recordings behind it would fire on all of them. Watch the series for a few weeks,
then set an alert from what your own site actually does. A starter Grafana
dashboard lives at
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

For Discord, Slack, Telegram, ntfy, Gotify, Pushover or a plain JSON webhook, set `BIRDNET_NOTIFY_URLS` — Apprise URL syntax, delivered in-process. For the remaining Apprise services, point `BIRDNET_APPRISE_URL` at an Apprise API server or `BIRDNET_APPRISE_CONFIG` at an `apprise` config file. For fully custom integrations, the [alert-rules engine](../admin/notifications.md#alert-rules) can fire a **webhook** with a configurable method and body when a detection matches your conditions (species pattern, confidence range, hour, day-of-week).

See [Notifications & Integrations](../admin/notifications.md) for the channel walkthrough.
