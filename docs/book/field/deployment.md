# Field deployment runbook

> Practical guidance for running BirdNet-Behavior **unattended** on a
> single piece of hardware, in the field, for **months at a time**, with
> no operator on site to push a button when something goes wrong.

This is the document you read once before deploying and once again the
month before the next field season. It is intentionally opinionated —
the defaults pick the safe option at every fork.

## Contents

1. [Hardware checklist](#1-hardware-checklist)
2. [Power and thermals](#2-power-and-thermals)
3. [Storage planning](#3-storage-planning)
4. [Networking](#4-networking)
5. [System hardening](#5-system-hardening)
6. [Time synchronisation](#6-time-synchronisation)
7. [Watchdog and process supervision](#7-watchdog-and-process-supervision)
8. [Database and backup policy](#8-database-and-backup-policy)
9. [Remote diagnostics and monitoring](#9-remote-diagnostics-and-monitoring)
10. [Update strategy](#10-update-strategy)
11. [Pre-flight checklist](#11-pre-flight-checklist)
12. [Recovery runbook](#12-recovery-runbook)

---

## 1. Hardware checklist

Recommended baseline for unattended deployments:

| Component  | Recommended                                          | Notes                                               |
| ---------- | ---------------------------------------------------- | --------------------------------------------------- |
| Compute    | Raspberry Pi 5 4 GB, or Pi 4 4 GB                    | Pi Zero 2 W works but is tight; enable ZRAM         |
| Storage    | 64 GB **endurance-class** microSD (Pi) or SSD on USB | Consumer SD cards fail after ~6 months of WAL churn |
| Audio      | USB condenser mic with weatherproof housing          | Avoid Pi GPIO mics — they're noisy near the SoC      |
| Power      | Dedicated 5V/3A PSU + UPS HAT or LiFePO4 + MPPT     | Mains glitches are a top failure cause              |
| Enclosure  | IP65 or better                                       | Vent for thermals; route audio in via gland         |
| Network    | Wired Ethernet preferred; 4G dongle otherwise         | Wi-Fi mesh works but is the second-most-common fault |

The bare-metal installer's `--doctor` pass will tell you if any of the
sanity checks fail (CPU cores, disk space, audio device, etc.) before
you ever start the daemon.

## 2. Power and thermals

- **UPS or battery + charge controller.** A clean shutdown beats a
  corrupted database. The Pi 5 PSU draws ~5 W at idle; a 10000 mAh
  battery rides through a 15-minute mains glitch easily.
- **Thermal headroom.** Stick a passive heatsink on the Pi 5 SoC; in
  sealed enclosures add a 40 mm 5 V fan with a temperature switch. The
  Prometheus `/api/v2/metrics` endpoint exports `birdnet_cpu_count`
  and you can scrape `vcgencmd measure_temp` in parallel.
- **Solar deployments.** Size for **3× average daily watt-hours** to
  ride out cloudy weeks. Use a battery monitor (e.g. INA219 over
  I²C) and stop the service via `systemctl stop birdnet-behavior` when
  battery drops below 30 %.

## 3. Storage planning

| Workload                                  | Daily volume (typical)                                 |
| ----------------------------------------- | ------------------------------------------------------ |
| Detection rows in `birds.db`              | ~5–50 MB at moderate activity                          |
| Extracted detection clips (WAV)           | ~1–10 MB per clip × per-species cap                    |
| Raw rolling recordings (tmpfs)            | 0 on disk (configured to `/tmp/birdnet-stream`)        |
| Backups (`~/BirdNet-Behavior/backups/`)   | Up to 14 × `birds.db` size                             |

**Set the disk policy explicitly:**

```ini
# /etc/birdnet/birdnet.conf
RECS_DIR=/data/recordings
MAX_FILES_SPECIES=100         # cap clips per species
DISK_PURGE_THRESHOLD=85       # autopurge starts at 85 %
```

The bundled disk manager supervises **two** directories, once a minute, with
the retention each one needs:

- the **recordings directory**, holding your extracted clips beside
  `birds.db`. These are your data, so they are never removed by age. Only the
  disk-full backstop touches them: once usage exceeds `DISK_PURGE_THRESHOLD`
  it removes the oldest clips first, skipping any file the database has marked
  locked (`/admin/recordings` → "lock"). The locked set is re-read every cycle,
  so locking a clip takes effect immediately — no restart needed.
- the **raw capture directory** (`--watch-dir`, typically the RAM-backed
  `/tmp/birdnet-stream`), which the detector reads and never needs again. It is
  drained continuously by age and by a total-size ceiling
  (`STREAM_RETENTION_SECS`, `STREAM_MAX_MB`) so the tmpfs self-empties.

Two further limits are enforced from the database on the daily maintenance
tick, and both leave the detection rows intact — only the audio is reclaimed,
so your counts, species lists and analytics are unaffected:

- `MAX_FILES_SPECIES` — keep the newest N clips per species.
- `CLIP_RETENTION_DAYS` — reclaim audio older than N days. **Off by default**
  (`0` = keep forever); set it in **Settings → System** ("Keep Clip Audio") or
  the config file if you want a rolling window.

Locked clips are exempt from both. A reclaimed clip keeps its filename and
gains the date its audio was removed, so the record of what was captured
survives even though the file does not.

All of these can be set from the web UI — you never have to edit this file to
change them. A command-line flag or `BIRDNET_*` variable wins over the UI,
which wins over the config file.

**Maintenance runs on wall-clock time, not uptime.** The daily jobs (integrity
check, session prune, species cap) and the weekly backup + VACUUM record their
completion in the database, so a station that reboots often still runs them —
an overdue job fires shortly after the next boot rather than restarting its
timer.

**Use endurance-class SD cards.** A standard A1 card will write out
~3000 P/E cycles per cell; WAL on a busy station can wear that out in
a year. SanDisk High Endurance and Samsung PRO Endurance are tested
choices. SSD-on-USB is dramatically better if your power budget allows it.

## 4. Networking

The daemon assumes the network may be down or expensive, and **no network
failure can slow or block the detection pipeline** — every integration is
dispatched off the detection path. Verified behaviours:

- **BirdWeather uploads** — store-and-forward. A post that fails after its
  in-flight retries is parked in the local database (`outbound_queue`) and
  replayed **oldest-first** when the network returns: batches of 25 with
  200 ms spacing so a returning uplink isn't slammed, per-entry backoff
  1 min → 1 h, bounded to 5 000 entries / 48 attempts so a months-long
  outage can't fill the disk. Nothing is lost to a flaky link; see §9 for
  the queue-depth gauge and the self-hosted-ingest override.
- **Apprise notifications** — best-effort, dispatched off the detection
  path; non-delivery is logged but never blocks detection. Not queued by
  design — a look-now alert replayed hours later is noise.
- **MQTT publisher** — fire-and-forget per detection, opening a fresh
  bounded-timeout connection; dispatched off the detection path so an
  offline broker never stalls detection. Not queued by design (live
  telemetry).
- **Wikipedia image cache** — populated lazily; works fine with no
  network at all.
- **Auto-update check** — daily, non-blocking; failure logged at
  `debug`. The check reads bounded response bodies (it cannot be made to
  exhaust memory by a hostile or misbehaving endpoint).
- **Heartbeat URL** — fire-and-forget; no retry, by design (the
  monitoring side is the one that should care if it didn't arrive).

For cellular deployments, consider scheduling cron `pppd` up/down
windows so the modem only powers up for the daily upload batch.

## 5. System hardening

The installer's systemd unit (`install.sh`) ships hardened by default:

- `Type=notify` + `WatchdogSec=120` — systemd restarts the daemon if
  it stops calling `sd_notify(WATCHDOG=1)` for 2 min.
- `ProtectSystem=strict`, `ProtectHome=read-only`, explicit
  `ReadWritePaths` — the daemon can only write where it needs to.
- `PrivateTmp=yes`, `NoNewPrivileges=yes`, `LockPersonality=yes`,
  `MemoryDenyWriteExecute=yes`, `RestrictRealtime=yes`,
  `RestrictNamespaces=yes`.
- `SystemCallFilter=@system-service` minus the privileged / kernel
  / debug / reboot / mount / cpu-emulation / clock / module groups.
- `MemoryHigh=768M`, `MemoryMax=1G`, `TasksMax=512`, `LimitNOFILE=65536`,
  `LimitNPROC=256` — bounded resource ceilings; runaway processes can't
  take down the host. The 1 GiB ceiling is sized for the bundled DuckDB
  analytics engine, whose queries are memory-hungry under load; the FP32
  model is mmap'd, so its pages are reclaimable and don't count as
  anonymous RSS. On a 512 MB board physical RAM plus zram binds first.
- `OOMPolicy=stop` — under memory pressure the unit stops cleanly
  instead of being killed mid-write.

If you customise the unit, run `systemd-analyze security
birdnet-behavior.service` to check the score (the shipped unit is
graded around 1.5–2.0, "OK" on the systemd scale).

## 6. Time synchronisation

Every detection row carries a wall-clock timestamp. If the clock jumps
backwards, analytics break.

- Enable `systemd-timesyncd` (default on Raspberry Pi OS) **or**
  `chrony` (preferred for cellular deployments — handles long offline
  windows better).
- The unit waits for `time-sync.target` before launching so the daemon
  never sees an unsynchronised clock at boot.
- For deployments with no network at all, fit a battery-backed RTC
  module (DS3231 on I²C) and load `rtc-ds1307` at boot.

## 7. Watchdog and process supervision

This is the workhorse of unattended operation:

1. The daemon calls `sd_notify(READY=1)` once the HTTP server is
   listening.
2. A background tokio task calls `sd_notify(WATCHDOG=1)` every
   `WATCHDOG_USEC / 2` (usually 60 s).
3. If the daemon hangs (livelock, GPU deadlock, ML thread stuck), the
   pings stop. systemd kills it after `WatchdogSec` and restarts via
   `Restart=always`.
4. `StartLimitBurst=5` within `StartLimitIntervalSec=300` prevents a
   tight restart loop on a permanently-broken install — the unit
   parks itself in `failed` state for operator review.
5. `ExecStartPre` runs `birdnet-behavior --doctor`. Exit codes 0
   (clean) and 1 (warnings) allow the service to start; exit code 2
   (errors that will prevent operation) blocks startup so the journal
   shows *what is broken* instead of just "service kept restarting".

Smoke-test the watchdog after installation:

```bash
# Pause the daemon to simulate a hang. systemd should kill it after 2 min.
sudo kill -STOP "$(pidof birdnet-behavior)"
journalctl -u birdnet-behavior -f
# Expect: "Watchdog timeout" then "Restarting" within 120 s.
```

### Per-source gain and quiet window (manual hardware checks)

Each audio source in **Admin → Audio** carries a software **gain** (dB) and an
optional **quiet window**. Like every other per-source setting (device, sample
rate, RTSP transport), these are read when the capture subsystem starts, so
**restart the service after editing them** for the change to take effect:

```bash
sudo systemctl restart birdnet-behavior
```

**Gain (`gain_db`).** At unity gain (0 dB) a local microphone records through
`arecord` (the lightest path). A non-zero gain routes that source through
`ffmpeg` instead so the gain can actually be applied — verify on real hardware:

```bash
# Set a source's gain to e.g. +12 dB in Admin → Audio, restart, then:
ps -ww -C ffmpeg -o args= | grep -- '-af volume'
# USB mic with gain → ffmpeg: "-f alsa -i <device> … -af volume=12.00dB"
# Set the gain back to 0, restart, and confirm the mic is back on arecord:
pgrep -a arecord            # present again at unity gain
# RTSP / PipeWire sources are always ffmpeg; the filter appears only with gain.
```

A captured clip from the gained source should be audibly louder (or quieter for
a negative dB cut) than at unity gain.

**Quiet window (`schedule_quiet`).** Set a window that *currently* includes the
time of day, **in UTC** — the quiet window shares the recording schedule's clock
basis (the fixed/solar window is evaluated in UTC too). Within ~2 reconcile
ticks (≈ a few seconds) the source's capture subprocess stops:

```bash
journalctl -u birdnet-behavior -f
# Expect: "recording paused (outside schedule or in quiet window)" for the source,
# and the birdnet_audio_source_up{source=<id>} gauge drops to 0 on /metrics.
# Move the window so "now" is outside it, restart, and confirm the subprocess
# is respawned and the gauge returns to 1.
```

While the system clock looks unsynced (no RTC yet at boot, NTP not ready) the
quiet window is **not** enforced — capture fails open exactly as the schedule
does, so a bogus boot-time date can never silence a source.

### Multi-source resilience (USB + several RTSP at once)

You can run one or more local microphones (USB/ALSA or PipeWire) **and** any
number of RTSP streams simultaneously — set them up in **Admin → Audio**, or
seed them from the config file with `ALSA_CARDS` (`;`-separated) and
`RTSP_URLS` (`,`-separated). Each source is captured by its own subprocess and
recordings carry a per-source tag (`local`/`MIC_n` and `RTSP_n`, or the source
row's id), so detections, recordings, and the
`birdnet_audio_source_up{source=…}` gauge stay distinct per source.

Every source is **supervised independently** — the central property for an
unattended field station with flaky cameras:

- **One source failing never disturbs the others.** A dead subprocess (camera
  rebooted, USB mic unplugged, network blip) is restarted with **capped
  exponential backoff** (2 s → 4 s → … → 60 s, then every 60 s **forever** — a
  source down for an hour is still recording when it comes back on hour two).
  The other sources keep recording and the detection pipeline keeps running
  throughout.
- **Silent stalls are caught, not just crashes.** An RTSP camera (or a USB mic
  wedged after a re-enumeration) whose process stays *alive* but stops
  delivering audio is detected by watching each source's newest segment: no
  fresh segment for several segment-durations (floor 2 min) and the source is
  restarted, exactly like a crash. Stall detection fails open while the clock
  is unsynced (segment mtimes aren't trustworthy before NTP).
- **A network outage never blocks detection.** BirdWeather, Apprise, MQTT,
  email, and heartbeat are all dispatched off the detection path, so a dead
  broker or an offline uplink for days slows none of them down — detections
  keep landing in the local database and you reconcile from there.

Verify per-source isolation on real hardware before sealing the unit:

```bash
journalctl -u birdnet-behavior -f
# Unplug ONE RTSP camera (or one USB mic). Expect, for that source only:
#   "audio source DOWN … still trying to restart"  and the
#   birdnet_audio_source_up{source=<id>} gauge for THAT id drops to 0,
# while every other source's gauge stays 1 and detections keep flowing.
# Plug it back in: "audio source up" and the gauge returns to 1 on its own.
```

## 8. Database and backup policy

The daemon runs a scheduled maintenance task in the background (no
operator action needed):

- **Daily** `PRAGMA integrity_check`. Failure is logged at `ERROR` and
  appears in the `/api/v2/health` endpoint.
- **Weekly** WAL checkpoint + `VACUUM` to reclaim space and prevent
  long-term page fragmentation.
- **Weekly** backup snapshot taken just before VACUUM, written to
  `${DATA_DIR}/backups/`. The 14 most recent are kept; older ones are
  pruned automatically.
- **Manual** snapshot any time: `birdnet-behavior --backup-db`.

Restore from the most recent good backup (only do this after the
daemon is stopped):

```bash
sudo systemctl stop birdnet-behavior
ls -lt ~/BirdNet-Behavior/backups/ | head -3
cp ~/BirdNet-Behavior/backups/birds.db.backup.<timestamp> ~/BirdNet-Behavior/birds.db
sudo systemctl start birdnet-behavior
```

## 9. Remote diagnostics and monitoring

Once the unit is sealed and shipped, the loop is:

1. **Heartbeat URL** (`HEARTBEAT_URL=`) — set to a free
   <https://healthchecks.io> check. The daemon pings it after every
   analysis cycle. You get an email if it stops within your configured
   grace period (recommend: 15 min).
2. **`/api/v2/health`** — pulled by your monitoring (Uptime Kuma,
   Healthchecks remote endpoints, custom cron) over the LAN or via a
   port-forward.
3. **`/api/v2/metrics`** — Prometheus text format. Scrape with
   Prometheus, Grafana Agent, or VictoriaMetrics. Key series:
   `birdnet_uptime_seconds`, `birdnet_detections_total`,
   `birdnet_process_resident_memory_bytes`, `birdnet_species_total`,
   and the two field-health gauges below.
4. **Detection deadman** — the end-to-end "is it actually detecting?"
   check that no per-component gauge can answer. The station measures
   how long ago the last detection landed and exports it as
   `birdnet_detection_silence_seconds` (also `detection_silence_secs`
   on `/api/v2/health`, and the "Last Detection" row on `/system`).
   After `DEADMAN_HOURS` of silence (default 24; `0` disables; also
   `--deadman-hours` / `BIRDNET_DEADMAN_HOURS`) it logs a loud warning
   and — when Apprise is configured — pushes **one** alert per quiet
   episode, with a recovery notice when detections resume. Stations in
   sparse habitats should raise the threshold; a Grafana alert on the
   gauge gives finer control.
5. **Store-and-forward uploads** — `BirdWeather` posts that fail while
   the uplink is down are parked in the local database and replayed
   automatically when connectivity returns (oldest first — the upstream
   record's sequence matches what happened in the field — in capped
   batches, exponential backoff up to 1 h, bounded queue). Watch
   `birdnet_outbound_queue_depth{kind="birdweather"}` — a depth that
   only grows means the uplink (or token) has been broken for a while.
   The `/system` page shows a "Queued Uploads" row whenever the queue
   is non-empty. MQTT and Apprise/email are deliberately NOT queued:
   they are live telemetry and look-now alerts, and replaying them
   hours later is worse than dropping them — the local database is
   always the ground truth.

   **Self-hosted ingest (sensitive species).** Research programmes that
   must keep observation locations under their own governance — rare or
   endangered species where a public community map is a poaching risk —
   can redirect the same upload pipeline (including the offline queue
   and ordered replay) at their own endpoint that implements the
   `BirdWeather` station API shape:

   ```ini
   # /etc/birdnet/birdnet.conf
   BIRDWEATHER_URL=https://ingest.example.org/api/v1
   ```

   (Env equivalent: `BIRDNET_BIRDWEATHER_URL`; only the host changes —
   the `/stations/<token>/detections` path shape is preserved.) The
   active endpoint is logged at startup so a misdirected station is
   visible in the first journal lines.
6. **`birdnet-behavior --doctor-json`** — for monitoring scripts that
   speak JSON (Home Assistant command sensor, Nagios, Zabbix). Same
   exit codes as the human-readable mode.
7. **SSH tunnel via Tailscale / ZeroTier / Cloudflare Tunnel** —
   gives you the web UI from anywhere without exposing a port to the
   open internet. Recommended over plain port-forward.

### Station-health alerts

The detection deadman answers *"is the station detecting at all?"*. Station
health answers the questions that leave it quiet — the faults a station keeps
detecting straight through:

| Condition | Threshold |
|---|---|
| An audio source down while others keep recording | 15 min continuous |
| Disk full enough that recordings are being purged | ≥ 90 % used |
| CPU at or above the Pi throttling point | ≥ 80 °C |
| Backup or integrity check not completed | > 21 days |

Each alerts **once per episode**, with a recovery notice when it clears, through
the same Apprise notifier as the deadman. Every condition must persist for three
consecutive five-minute polls before it fires, so a mic that re-enumerates or a
disk that spikes during clip extraction stays silent.

A single-source station that goes fully down is deliberately *not* reported
here — the deadman covers that, with better wording, and two notifications for
one fault is how a channel gets muted. This check exists for what the deadman
structurally cannot see: some sources up, some down.

On by default. Disable with `--station-health-alerts false`, or
`STATION_HEALTH_ALERTS=false` in `/etc/birdnet/birdnet.conf`.

```bash
# Confirm it started.
journalctl -u birdnet-behavior | grep 'station-health notifier started'
```

## 10. Update strategy

Field-deployment philosophy: **don't auto-update**.

- The daemon checks for updates daily (logged at INFO when one is
  available) but **never** applies them automatically. The admin
  panel's "Update" button is the only way to upgrade.
- Test a new release on a bench unit first. Run it for at least
  72 h before pushing it to field units.
- When you do push, do it during the species' low-activity window
  (e.g. local 14:00 for songbirds) so a downtime blip costs the fewest
  detections.
- Keep the previous binary in `/usr/local/bin/birdnet-behavior.prev`
  so a one-line `mv` rollback is possible if the new build misbehaves.

## 11. Pre-flight checklist

Run through this the day you take the unit to the field:

```bash
# 1. Hardware and audio.
birdnet-behavior --doctor               # ALL green (or warnings only).

# 2. Time sync working.
timedatectl status | grep "System clock synchronized: yes"

# 3. Disk has > 5 GiB free.
df -h /data

# 4. Backup directory exists and is writable.
ls -la ~/BirdNet-Behavior/backups/

# 5. Watchdog smoke test (see § 7).
sudo kill -STOP "$(pidof birdnet-behavior)" && sleep 130 && \
  systemctl is-active birdnet-behavior

# 6. Reboot test — comes back clean?
sudo reboot
# After reboot, wait 2 min, then:
systemctl is-active birdnet-behavior   # expect "active"
curl -sf http://localhost:8502/api/v2/health | jq .status

# 7. Network failure tolerance.
sudo ip link set wlan0 down
sleep 60
journalctl -u birdnet-behavior --since "1 minute ago" | grep -i error
sudo ip link set wlan0 up

# 8. Heartbeat reaches your dead-man switch.
# Configure HEARTBEAT_URL, restart, then confirm in the dashboard at
# https://healthchecks.io/.
```

If any of those eight steps fails, fix it on the bench, **not in the
field**.

## 12. Recovery runbook

Symptom-driven, oldest-known-cause first:

| Symptom                                   | First action                                                                |
| ----------------------------------------- | ----------------------------------------------------------------------------|
| Heartbeat stopped                         | `ssh` in, `systemctl status birdnet-behavior`, then `journalctl -u … -n 200` |
| `journalctl` shows "Watchdog timeout"     | Look 200 lines earlier for the last activity — likely audio device hang     |
| `--doctor` reports `[ FAIL ]` for model   | Re-download: `rm /data/model/*.onnx`, then restart the service               |
| Disk full                                 | `birdnet-behavior --doctor` will say so; lower `MAX_FILES_SPECIES`           |
| Database integrity check fails            | Stop service; restore from `~/BirdNet-Behavior/backups/`                     |
| Web UI 500s                               | Check `/api/v2/health` for `status`; integrity check most common cause       |
| Detection latency growing                 | OOM throttling — check `birdnet_process_resident_memory_bytes` over time     |
| `Restart=always` stuck in failed state    | `StartLimitBurst` exceeded; fix root cause, then `systemctl reset-failed`    |

See also: [Troubleshooting](../guides/troubleshooting.md) for general
diagnostics not specific to field deployments.

---

## See also

- [Troubleshooting](../guides/troubleshooting.md) — symptom-organised
  problem solving (the fuller
  [`TROUBLESHOOTING.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/TROUBLESHOOTING.md)
  lives in the repository)
- [`SECURITY.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/SECURITY.md)
  — disclosure policy
- [`docs/architecture/12-risks.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/12-risks.md)
  — risk matrix
- [`docs/architecture/14-diagnostics.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/14-diagnostics.md)
  — design of the `--doctor` system
