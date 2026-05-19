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

The bundled disk manager monitors continuously; once usage exceeds
`DISK_PURGE_THRESHOLD` it removes the oldest clips first, skipping any
file the database has marked locked (`/admin/recordings` → "lock").

**Use endurance-class SD cards.** A standard A1 card will write out
~3000 P/E cycles per cell; WAL on a busy station can wear that out in
a year. SanDisk High Endurance and Samsung PRO Endurance are tested
choices. SSD-on-USB is dramatically better if your power budget allows it.

## 4. Networking

The daemon assumes the network may be down or expensive. Verified
behaviours:

- **BirdWeather uploads** — buffered locally; retried with exponential
  backoff. No bandwidth wasted on a dead link.
- **Apprise notifications** — best-effort; non-delivery is logged but
  never blocks detection.
- **Wikipedia image cache** — populated lazily; works fine with no
  network at all.
- **Auto-update check** — daily, non-blocking; failure logged at
  `debug`.
- **MQTT publisher** — auto-reconnects with backoff.
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
- `MemoryMax=512M`, `TasksMax=512`, `LimitNPROC=256` — bounded
  resource ceilings; runaway processes can't take down the host.
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
   `birdnet_process_resident_memory_bytes`, `birdnet_species_total`.
4. **`birdnet-behavior --doctor-json`** — for monitoring scripts that
   speak JSON (Home Assistant command sensor, Nagios, Zabbix). Same
   exit codes as the human-readable mode.
5. **SSH tunnel via Tailscale / ZeroTier / Cloudflare Tunnel** —
   gives you the web UI from anywhere without exposing a port to the
   open internet. Recommended over plain port-forward.

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

See also: [`TROUBLESHOOTING.md`](../TROUBLESHOOTING.md) for general
diagnostics not specific to field deployments.

---

## See also

- [`TROUBLESHOOTING.md`](../TROUBLESHOOTING.md) — symptom-organised
  problem solving
- [`SECURITY.md`](../SECURITY.md) — disclosure policy
- [`docs/architecture/12-risks.md`](architecture/12-risks.md) — risk
  matrix
- [`docs/architecture/14-diagnostics.md`](architecture/14-diagnostics.md)
  — design of the `--doctor` system
