# Troubleshooting BirdNet-Behavior

> "Why isn't it working?" — the page to read first, *before* opening an
> issue or asking on Discussions.

This guide is organised by symptom, not by cause. Find the section that
matches what you observed, then work the checks top to bottom.

If nothing in here helps, please [open a bug report](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose)
and paste the output of `birdnet-behavior --doctor` (see below).

## 0. Always start here: run the doctor

```bash
# Bare metal
sudo -u birdnet birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

The diagnostic prints a one-screen report covering CPU, configuration
values, audio source reachability, model file, database integrity, disk
space, tool dependencies, and network. Every finding includes a concrete
suggested fix. Exit code:

- `0` — all checks passed
- `1` — at least one warning (system will run, features degraded)
- `2` — at least one error (system will not work until fixed)

For monitoring scripts the same checks are available as a single line of
JSON:

```bash
birdnet-behavior --doctor-json | jq .
```

---

## 1. The service won't start

### 1.1 systemd reports a failure
```bash
sudo journalctl -u birdnet-behavior -n 100 --no-pager
sudo systemctl status birdnet-behavior
```

Common causes and fixes:

| Symptom in the logs                                            | Fix                                                                                                |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `config not found: /etc/birdnet/birdnet.conf`                  | Run `install.sh` again, or copy `.env.example` to `/etc/birdnet/birdnet.conf` and edit it          |
| `Failed to set up mount namespacing: /tmp/birdnet-stream: No such file or directory` (exit `226/NAMESPACE`) | An older unit listed the tmpfs stream dir in `ReadWritePaths=`, which conflicts with `PrivateTmp=`. Upgrade to the latest release, or run `sudo bash install.sh repair` to rewrite the unit. |
| `database recovery failed`                                     | Run `birdnet-behavior --check-db`; restore from `~/BirdNet-Behavior/backups/` if corruption is real |
| `failed to install Ctrl+C handler` / `SIGTERM handler`         | Likely running under a non-Unix or sandboxed environment without signal support                    |
| `address already in use`                                       | Another service is on port 8502; change `BIRDNET_PORT` or stop the conflicting service             |
| `permission denied` reading `/etc/birdnet/birdnet.conf`        | The config is `0640 root:<service-group>` (it holds secrets). Run `sudo bash install.sh repair` to restore correct ownership/permissions. |

Still stuck? `sudo bash install.sh repair` re-creates missing directories, fixes
ownership/permissions, rewrites the systemd unit, and restarts — fixing most
"won't start" cases without touching your data.

### 1.2 Container exits immediately

```bash
docker compose logs --tail=200 birdnet
```

The entrypoint will say *which* of the three failure modes it hit
(network, Zenodo, disk). If the message is unclear, run the doctor
inside a one-shot container:

```bash
docker compose run --rm birdnet birdnet-behavior --doctor
```

---

## 2. The web UI is not reachable

1. Confirm the binary is listening:
   ```bash
   ss -tlnp | grep 8502
   ```
   No row → the daemon never finished starting up. Go to section 1.

2. Confirm the firewall lets traffic through:
   ```bash
   sudo ufw status                # Ubuntu / Debian with ufw enabled
   sudo firewall-cmd --list-all   # RHEL / Fedora
   ```

3. Confirm you're using the right host:
   ```bash
   ip -4 addr show | awk '/inet / && !/127\.0\.0\.1/ {print $2}'
   ```
   Open `http://<that IP>:8502/` in a browser.

4. From a remote machine, confirm reachability with `curl`:
   ```bash
   curl -sf -o /dev/null -w "%{http_code}\n" http://<host>:8502/api/v2/health
   # Expected: 200
   ```

---

## 3. No detections are appearing

### 3.1 Audio source is silent or wrong

```bash
# ALSA (bare metal)
arecord -l
arecord -D plughw:1,0 -d 3 /tmp/test.wav && aplay /tmp/test.wav

# PulseAudio / PipeWire
pactl list short sources
parec --device=<source-name> /tmp/test.raw   # Ctrl-C after a few seconds

# RTSP — TCP probe (the doctor also does this)
nc -zv <camera-host> 554
```

If those work and BirdNet-Behavior still hears silence:

```bash
# Verify the config picked up the right source
grep -E "^ALSA_CARD|^RTSP_URL|^PIPEWIRE_DEVICE" /etc/birdnet/birdnet.conf

# Docker users
grep -E "^BIRDNET_(ALSA|PIPEWIRE|RTSP)" .env
```

Only **one** of those three should be set. If multiple are, the doctor
will warn and the daemon will pick the first one it finds.

### 3.2 Recordings exist but the database stays empty

```bash
ls -la "$(grep ^RECS_DIR /etc/birdnet/birdnet.conf | cut -d= -f2- | tr -d '"')"
sudo journalctl -u birdnet-behavior --since "1 hour ago" | grep -iE "inference|detection|model"
```

Look for `model failed to load` or `mel spectrogram` errors. The most
common causes:

- Model file truncated (run `birdnet-behavior --doctor` — the model
  check flags a file under 1 MB as suspicious; redownload by removing
  the file and restarting)
- `--watch-dir` does not match where recordings are written
- The ONNX runtime cannot allocate enough memory on a Pi Zero 2W — see
  section 5

### 3.3 Detections exist but confidence is always low

Audio quality is almost always the culprit. Check:

- USB mic placement (avoid wind exposure; use a foam windscreen)
- ALSA mic gain: `alsamixer -c 1` then F4 ("Capture") and set the gain
- Spectral content of recordings: open one in Audacity → "Spectrogram"
  view — bird calls should be clearly visible in the 1–8 kHz band

Adjust `CONFIDENCE`, `SENSITIVITY`, `SF_THRESH`, and per-species
thresholds in `/admin/settings`. Restart the daemon after each change.

---

## 4. Database errors

```bash
birdnet-behavior --check-db
```

Output meanings:

- *PASSED* — schema and pages are intact
- *FAILED — corruption detected* — restore from the most recent backup:

```bash
birdnet-behavior --backup-db                   # create a fresh backup of the broken file
ls -lt ~/BirdNet-Behavior/backups/             # find the newest known-good
cp ~/BirdNet-Behavior/backups/<file>.db ~/BirdNet-Behavior/birds.db
sudo systemctl restart birdnet-behavior
```

Backups are taken automatically at startup whenever WAL replay detects
recovery; manual backups are equivalent.

### 4.1 Behavioral analytics show "extension required"

Symptom: the **Analytics** page cards (Activity Sessions, Species Retention,
Next Species) report *"The `duckdb-behavioral` extension is required…"*, and the
startup log warns `duckdb-behavioral extension not loaded`.

Cause: the `duckdb-behavioral` community extension is compiled against a
**specific DuckDB version** and DuckDB refuses to load an extension built for a
different version (e.g. an extension built for DuckDB `v1.5.3` cannot load into a
binary that bundles `v1.5.5`). The mismatch can appear after the bundled DuckDB
is bumped.

This is **non-fatal** — the rest of the app is unaffected, and the
station-local analytics that read SQLite directly (Migration, the Dawn Chorus,
the Heatmap, Co-occurrence, and the Time-series page) keep working. Only the
extension-backed sessionize / retention / next-species queries are unavailable.

**Diagnose it in one command:**

```bash
birdnet-behavior --verify-extension
```

This opens a throwaway DuckDB database, loads the extension the way the station
does, and prints the engine version, the extension version, and what the
build-time embedded copy targets. It exits 0 when the extension loads and
non-zero when it does not, so it is also usable from a monitoring script.

Note `--doctor` will *not* answer this question: it deliberately never opens
DuckDB (a preflight open during `ExecStartPre` could contend with the running
service), so it reports whether analytics is *configured*, not whether the
extension actually loads.

To check the **offline** guarantee specifically — the one that matters on an
air-gapped or metered station — run it with networking disabled:

```bash
# Bare metal: run it in a network namespace with no interfaces.
unshare -rn birdnet-behavior --verify-extension

# Docker: the same check against the image you are running.
docker run --rm --network none \
    --entrypoint /usr/local/bin/birdnet-behavior \
    ghcr.io/tomtom215/birdnet-behavior:latest --verify-extension
```

With no network neither the extension cache nor the community registry can
satisfy the load, so a pass proves the *embedded* copy loaded. A station that
passes with network and fails without it has a packaging defect: the embedded
extension targets the wrong DuckDB version and the registry has been silently
covering for it.

Fix: rebuild and republish the extension for the bundled DuckDB version in the
[duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) repository,
or pin the `duckdb` crate to the version the published extension targets, then
rebuild with `--features analytics`. As an immediate workaround on a station
with network, `birdnet-behavior --refresh-extension` force-installs the correct
build from the community registry.

### 4.2 Every analytics dashboard is empty and nothing is logged as an error

Symptom: the Analytics and Time-series pages render but carry no data, the
health endpoint is green, `--doctor` is happy, and the journal shows no error.
It does not recover on its own — stations have sat like this for days.

This is **not** the same as 4.1. There the extension-backed cards degrade and
say so while everything else keeps working; here *every* dashboard is empty,
including the ones that need no extension at all.

**Look for this line:**

```bash
journalctl -u birdnet-behavior --no-pager | grep -i 'duckdb": Read-only'
# Failed to create directory "/home/pi/.duckdb": Read-only file system
```

Cause: every dashboard query filters on a look-back window, which reaches DuckDB
as `detection_date >= CURRENT_DATE - INTERVAL n DAYS`. `CURRENT_DATE` lives in
DuckDB's **ICU** extension — as does every other way to name the current local
date, including `today()` and `CAST(now() AS DATE)`. ICU is not statically
linked, so DuckDB fetches it, by default into `$HOME/.duckdb`. The shipped unit
sets `ProtectHome=read-only`, that write fails, and every date-ranged query
fails with it. The web layer renders a query error as a "temporarily
unavailable" fragment and caches it, which is why nothing reaches the log.

Fix: **upgrade past 0.13.1.** Two changes close it — DuckDB's extension
directory now sits beside the analytics database inside the data directory
(which the unit grants write access to), and the ICU binary is embedded in the
release binary, so nothing is downloaded and `$HOME` is never touched.

`--verify-extension` (see 4.1) now also runs the date-window query the
dashboards open with, so it answers this too — including under
`unshare -rn` / `--network none`, where a pass proves the embedded ICU is doing
the work.

Workaround if you cannot upgrade yet — point `HOME` at the data directory, the
one place the unit already allows writes:

```bash
sudo systemctl edit birdnet-behavior
# [Service]
# Environment=HOME=/home/pi/BirdNet-Behavior     # your data dir; pi → your user
sudo systemctl restart birdnet-behavior
curl -s -o /dev/null -w '%{http_code}\n' localhost:8080/api/v2/timeseries/daily
```

The override is harmless to leave in place after upgrading, and unnecessary.

---

## 5. Memory / CPU pressure on small hardware

Symptoms: kernel OOM-killer messages in `dmesg`, container restarts in a
loop, inference latency spikes, detections lag minutes behind audio.

Mitigations in order of effectiveness:

1. **Enable ZRAM.** The bare-metal installer offers this automatically
   on hosts with ≤ 2 GB RAM. To re-run: `SKIP_ZRAM=0 sudo install.sh`.
2. **Reduce overlap.** `BIRDNET_OVERLAP=0.0` halves inference cost
   compared with `OVERLAP=1.5`.
3. **Use a smaller model.** BirdNET V2.4 FP16 is ~50 MB vs BirdNET+
   V3.0's ~541 MB; both are accepted by the daemon.
4. **Disable analytics.** Analytics is on by default. On a tiny board,
   turn it off: bare-metal, remove the `--analytics-db` flag from the
   systemd unit; Docker, unset `BIRDNET_ANALYTICS_DB` in your `.env`.
   DuckDB roughly doubles RAM usage during sync.
5. **Throttle the disk manager.** Increase `DISK_PURGE_THRESHOLD` so
   purges happen less often on SD-card storage.

---

## 6. Notifications never arrive

```bash
# Test the Apprise endpoint independently of BirdNet
apprise -t "test" -b "from birdnet" <YOUR_APPRISE_URL>

# Confirm the daemon picked it up
birdnet-behavior --doctor 2>&1 | grep -i apprise
```

Common gotchas:

- `BIRDNET_NOTIFY_CONFIDENCE` defaults to `0.8`; you may need to lower
  it to see anything during testing
- `BIRDNET_NOTIFY_TRIGGER=new-species-daily` suppresses repeat
  notifications within the day — use `each` while debugging
- The Apprise URL must be URL-encoded if it contains special characters

---

## 7. Cross-cutting "huh, that's weird" checklist

When nothing else fits, run these and attach the output to your issue:

```bash
birdnet-behavior --version
birdnet-behavior --doctor
uname -a
free -m
df -h
sudo journalctl -u birdnet-behavior -n 200 --no-pager
docker --version 2>/dev/null
docker compose version 2>/dev/null
docker compose logs --tail=200 birdnet 2>/dev/null
```

That set of commands answers about 90% of the follow-up questions a
maintainer would otherwise have to ask.

---

## See also

- [`README.md` § Troubleshooting](README.md#troubleshooting) — concise
  inline version of this guide
- [`SUPPORT.md`](SUPPORT.md) — how to get help and what channel to use
- [`SECURITY.md`](SECURITY.md) — private disclosure for suspected
  vulnerabilities
- [`docs/architecture/12-risks.md`](docs/architecture/12-risks.md) — the
  project's threat model and risk register
- [`docs/architecture/14-diagnostics.md`](docs/architecture/14-diagnostics.md)
  — design rationale for the `--doctor` system
