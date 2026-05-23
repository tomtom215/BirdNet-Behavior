# System Health

The **System** page (`/system`) is the station's vital-signs monitor.

![The system health page](../images/system-health.png)

- **Live gauges** — CPU, memory and temperature as 3/4-arc gauges (with a graceful "no sensor" state where a temperature probe isn't available), plus system uptime.
- **Database** — total detections, unique species, days with data, and a live integrity check.
- **Version & runtime** — the build version, MSRV, and whether the analytics feature is compiled in.
- **Disk & audio pipeline** — the database path and size, recording directory, and clip count.

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
