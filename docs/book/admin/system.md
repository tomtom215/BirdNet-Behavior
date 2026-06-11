# System Health

The **System** page (`/system`) is the station's vital-signs monitor.

![The system health page](../images/system-health.png)

- **Live gauges** — CPU, memory and temperature as 3/4-arc gauges (with a graceful "no sensor" state where a temperature probe isn't available), plus system uptime.
- **Database** — total detections, unique species, days with data, a live integrity check, and a **Last Detection** row (how long ago the most recent detection landed). That row is the plain-English answer to "is it actually working right now?" — every other gauge can read healthy while the station records silence, but a fresh detection proves the whole chain from microphone to database is alive. When uploads are backed up behind a network outage, a **Queued Uploads** row appears too (and only then — a healthy station shows no noise).
- **Version & runtime** — the build version, MSRV, and whether the analytics feature is compiled in.
- **Disk & audio pipeline** — the database path and size (the disk-usage figure follows `df`'s "used of reachable space", so reserved blocks or a container quota don't understate it), recording directory, and clip count.

## The detection deadman

Behind the "Last Detection" row is a watchdog that turns silence into an
alert. It measures the seconds since the most recent detection and, past a
threshold you set with `--deadman-hours` (env `BIRDNET_DEADMAN_HOURS`, config
key `DEADMAN_HOURS`; default **24 h**, `0` disables the alert), logs a loud
warning and sends **one** notification per quiet episode through Apprise — with
a recovery notice when detections resume. It never cries wolf on a brand-new
station that has not detected anything yet, and a silent *night* is well within
the default. Raise the threshold for genuinely sparse habitats. The same
freshness value is exported as the `birdnet_detection_silence_seconds`
Prometheus gauge and as `detection_silence_secs` on `/api/v2/health`, so you can
alert on it from Grafana or any uptime monitor. See the
[Field Deployment Runbook](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/FIELD_DEPLOYMENT.md#9-remote-diagnostics-and-monitoring)
for the monitoring playbook.

## The built-in doctor

For a deeper, scriptable check, run the diagnostic from the CLI. It prints a one-screen report covering CPU, configuration, audio reachability, the model file, database integrity, disk space, tool dependencies and network — each problem with a concrete suggested fix.

```bash
sudo -u birdnet birdnet-behavior --doctor          # bare metal
docker compose exec birdnet birdnet-behavior --doctor   # Docker
```

Exit code: `0` = all good, `1` = warnings only, `2` = at least one error.

For monitoring (Nagios / Zabbix / a Home Assistant command sensor / a Prometheus textfile collector) the same checks are available as one line of JSON:

```bash
birdnet-behavior --doctor-json | jq .
```

## Metrics & logs

- **Prometheus metrics** are exposed at `/api/v2/metrics`.
- A **live log viewer** (`/admin/system/logs/page`) streams the service log over SSE with level filtering.
