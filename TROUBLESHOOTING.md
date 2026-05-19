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
| `database recovery failed`                                     | Run `birdnet-behavior --check-db`; restore from `~/BirdNet-Behavior/backups/` if corruption is real |
| `failed to install Ctrl+C handler` / `SIGTERM handler`         | Likely running under a non-Unix or sandboxed environment without signal support                    |
| `address already in use`                                       | Another service is on port 8502; change `BIRDNET_PORT` or stop the conflicting service             |
| `permission denied` reading `/etc/birdnet/birdnet.conf`        | `sudo chmod 644 /etc/birdnet/birdnet.conf` (it's not secret — credentials live in `.env`)          |

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
4. **Stop the analytics feature.** Pull `latest` instead of
   `latest-analytics`; DuckDB doubles RAM usage during sync.
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
