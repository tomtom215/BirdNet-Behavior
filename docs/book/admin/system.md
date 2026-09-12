# Station Health

The Station **Health** tab (`/station`) is the station's vital-signs monitor — the operator's "is it working?" screen, and the public, login-free heir to the old `/system` page (which now permanently redirects here).

![The Station Health tab](../images/system-health.png)

- **Status banner** — one line that stays green while nothing needs attention, and flips to amber naming the problem when storage runs low, the database integrity check fails, no audio sources are configured, or uploads are backed up.
- **Audio sources** — a per-source panel fed live by the capture supervisor: a state chip (Live · Stalled · Backing off · Paused), a rolling 24-hour uptime strip, how long since audio last arrived, today's detections for the source and the current retry/backoff line. When the web server runs without the capture supervisor (`--web-only`, or tooling) it falls back to an *activity* view — how many detections each source produced today and how recently — and never fakes a live/stalled chip.
- **Vitals** — CPU, memory, temperature (with a graceful "no sensor" state where a probe isn't available) and disk, each with a meter. The disk figure follows `df`'s "used of reachable space", so reserved blocks or a container quota don't understate it.
- **Microphone health** — the station's own background **noise floor** per source over the last 7 days, and how far it has moved against that source's own 30-day average. This is the one signal that separates *a season going quiet* from *a microphone going deaf*: a failing capsule keeps its process alive and its status green, and shows up only as fewer detections — exactly like autumn. Ambient background does not stop when the birds do, so a large, sustained **drop** here, with nothing else changed, points at the equipment. The panel is absent until the station has sampled something, and says "building a baseline" rather than reporting a change it has nothing to compare against. No threshold is applied and no alert is sent: a noise floor moves for weather, season, a road and leaf-out, and a number picked without a season of real recordings behind it would fire on all of them. Same figures are exported as `birdnet_noise_floor_dbfs` and `birdnet_noise_floor_drift_db` for anyone who wants to draw their own line.
- **Pipeline** — the **last detection** (the plain-English answer to "is it actually working right now?" — every other gauge can read healthy while the station records silence, but a fresh detection proves the whole chain from microphone to database is alive), queued uploads (shown only when a network outage backs them up), the service uptime, total detections, and the **species filter**: whether the occurrence filter is running and how many species it currently admits. A filter admitting zero species means nothing the station hears can be recorded, and the status banner says so.
- **Diagnostics** — a short checklist (audio sources · disk headroom · database integrity) with a link to the station's full `doctor` report at `/admin/doctor`: the same checks `birdnet-behavior --doctor` runs — audio device listing, model, clock, TLS, database, offsite, disk — run on request and read-only, with **Download as JSON** (`/admin/doctor.json`, what `--doctor-json` prints) and **Download a support bundle** (`/admin/support-bundle`, the same redacted archive `--support-bundle` writes) beside it. Both are behind the admin login, so a station in a field can be diagnosed from a phone without SSH.

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
[Field Deployment Runbook](../field/deployment.md#9-remote-diagnostics-and-monitoring)
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

## How much memory the analytics engine gets

DuckDB treats its memory limit as permission to use that much, so on a small
board a limit that is a large share of physical RAM is a standing invitation to
be OOM-killed in the middle of a dashboard query — at three in the morning,
when nobody is looking.

The station sizes the pool itself. Unset, `BIRDNET_DUCKDB_MEMORY_LIMIT` becomes
**a quarter of whatever this process may actually use**: the smaller of physical
RAM and any cgroup limit it is under (the systemd unit ships `MemoryMax=1G`; a
container has its own). That is capped at 256 MiB and floored at 64 MiB.

- On the shipped unit that comes to exactly 256 MiB — the flat value the station
  used before, so nothing changes on the hardware it was written for.
- On a 512 MB board it comes to 128 MiB, instead of handing one subsystem half
  the machine.
- Below a 256 MiB ceiling the floor cannot be met, and **analytics is refused at
  startup** with a line in the journal saying so. The station still records,
  classifies and serves; it just has no behavioural dashboards. Refusing is the
  point: the alternative is starting something that will be killed mid-query.

Set the variable and it is used verbatim, on any machine — including one the
sizing would have refused. Someone who has measured their own station knows more
than this does.

Where the two numbers come from, since neither is a preference:

- **A quarter** is the proportion the shipped unit already implies —
  `MemoryMax=1G` with a 256 MB pool — so the default hardware is unchanged by
  construction. A `const` assertion in the source fails the build if either
  number moves without the other.
- **64 MiB** was measured, not chosen. A sessionisation shaped like the
  behavioural queries — `lag` and a running sum over 1.5 million detections
  partitioned by species, then aggregated — ran out of memory at 8, 16 and
  32 MiB and succeeded from 48 MiB up on DuckDB 1.5. 64 MiB is the next step
  above the smallest observed working value.

What is *not* measured is the rest of the process's footprint on a Raspberry Pi,
which would need a Pi. So this sizes a proportion, not a budget: it makes the
analytics engine's share scale with the machine, and does not claim to know the
remainder is enough. `--doctor` reports which case applied under **Analytics
memory**.

## Metrics & logs

- **Prometheus metrics** are exposed at `/api/v2/metrics`.
- A **live log viewer** (`/admin/system/logs/page`) streams the service log over SSE with level filtering. A connecting client is replayed the last 200 lines first, so you see what led up to now rather than only what happens next. This is the whole picture in Docker, where there is no `journalctl` to fall back on.
- **`errors.jsonl`**, beside the database, keeps ERROR and WARN lines only, one JSON object per line, capped at 1 MB. It exists because a default Raspberry Pi OS has no `/var/log/journal`: the journal is volatile, so every watchdog bounce, power cut and update erases the evidence of what caused it — including the reboot you are trying to explain. `--support-bundle` carries this file.

> **If you ran an earlier version.** The log viewer streamed nothing at all. Its
> backing channel existed and the page connected to it, but no `tracing` layer
> was ever installed, so it replayed an empty backlog and then emitted
> keep-alives for ever. The page now shows what the station logged.

## Audit log

`/admin/audit` records who changed what. Rows are kept for 180 days and pruned
by the maintenance loop.

Actions are dotted and hierarchical, so the page's filter selects a family with
a prefix — `auth.%` for every sign-in, `species.%` for every filter change:

| Family | Actions |
|---|---|
| `auth.` | `login.ok`, `login.fail`, `login.throttled`, `logout` |
| `account.` | `user.create`, `user.delete`, `password.set`, `session.revoke`, `session.revoke_others` |
| `settings.` | `update` |
| `species.` | `include.add`, `include.remove`, `exclude.add`, `exclude.remove`, `threshold.set`, `threshold.delete`, `detections.delete`, `clips.delete` |
| `audio.` | `source.create`, `source.update`, `source.delete`, `source.restart`, `source.listen_default` |
| `rule.` | `create`, `delete`, `toggle`, `import` |
| `data.` | `detections.clear`, `recordings.clear`, `database.restore`, `backup.run` |
| `system.` | `restart`, `update.apply` |

`species.detections.delete` and `species.clips.delete` are the bulk actions on
[Species storage](settings.md#species-storage). Each records how many rows it
changed **and** how many locked detections it deliberately kept, so the log
explains a count that would otherwise look wrong.

A failed sign-in records the *submitted* username and no actor — "someone tried
to sign in as `admin` sixty times last night" is the thing worth knowing, and a
username that does not exist is as interesting as one that does.
`login.throttled` is an attempt the station refused without checking the
password: after five failures from one address inside fifteen minutes, that
address is answered `429` until its oldest failure is a quarter-hour old, and
each refused attempt is recorded here too. A successful sign-in clears the
address; a restart forgives everything.

**Values are never recorded.** A settings save lists the names of the keys that
changed and nothing else. `rtsp_url` is why: an RTSP URL routinely carries
`user:pass@` in its authority, and this page renders its rows verbatim. A save
that changed nothing writes no row at all, because the settings form posts every
field on every submission and recording each one would turn the log into a click
counter.

Destructive actions — clearing detections, restoring a database, restarting,
applying an update — are recorded *before* the work starts, not after. If the
process does not survive the operation there is no "after" to record from, and a
station whose history vanished with nothing in the audit log is
indistinguishable from one that was never used.

> **If you ran an earlier version.** This page was permanently empty. The table,
> the store, the page and the 180-day pruner all existed, and the one function
> that writes a row had no callers outside its own tests — so on a shared
> station the page did not read as "the log is broken", it read as "nothing
> happened".
