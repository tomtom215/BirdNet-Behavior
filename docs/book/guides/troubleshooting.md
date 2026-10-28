# Troubleshooting

The one troubleshooting guide, organised by symptom rather than by cause: find the section that matches what you saw, then work its checks top to bottom. Read it before opening an issue; if nothing here helps, [open a bug report](https://github.com/tomtom215/BirdNet-Behavior/issues/new/choose) with the doctor's output (below).

## Two things the commands on this page assume

**Bare metal.** The service runs as the account that ran `sudo bash install.sh` — there is **no `birdnet` user** — and keeps its data in `~/BirdNet-Behavior` of that account. Set two shell variables once and the commands below work for any account:

```bash
SVC=$(systemctl show -p User --value birdnet-behavior)        # the account the service runs as
DATA=$(getent passwd "$SVC" | cut -d: -f6)/BirdNet-Behavior    # the installer's data directory
```

**Docker.** Run `docker compose …` from the folder holding your `docker-compose.yml`. The data directory inside the container is `/data`.

`install.sh` below means the installer script. If you installed with the one-liner you do not have a copy; fetch one first:

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/tomtom215/BirdNet-Behavior/releases/latest/download/install.sh -o install.sh
```

## Start here: run the doctor

```bash
sudo -u "$SVC" birdnet-behavior --doctor                  # bare metal
docker compose exec birdnet birdnet-behavior --doctor     # Docker
```

It prints a one-screen report covering CPU, configuration, audio reachability, the model file, database integrity, disk space, tool dependencies and network — and **every problem comes with a concrete suggested fix.** Exit code: `0` = all good, `1` = at least one warning (the station runs, with features degraded), `2` = at least one error (it will not work until fixed).

- For monitoring scripts, the same checks as one line of JSON: `birdnet-behavior --doctor-json`.
- With a browser and no shell, sign in and open `/admin/doctor`: the same report, the JSON at `/admin/doctor.json`, and a **support bundle** to download (`/admin/support-bundle`).
- `birdnet-behavior --doctor --fix` first recreates any configured directory that is missing (the recordings folder, the image cache) — the commonest cause of "the service runs but nothing is recorded", for example after a tmpfs reset — and then runs the report.
- `birdnet-behavior --support-bundle [PATH]` writes the same bundle from the command line: the report (JSON and text), the version, a redacted copy of the configuration and the last 2000 journal lines in one `.tar.gz`. Passwords, tokens and URL credentials are masked, and the station's `LATITUDE` and `LONGITUDE` are shown **to one decimal place (about 11 km)**, both in the configuration section and wherever the journal printed them. Read the archive before posting it publicly all the same.

## Service won't start

```bash
sudo journalctl -u birdnet-behavior -n 100 --no-pager
sudo systemctl status birdnet-behavior
```

| What the log says | What it means and what to do |
|---|---|
| `config not found: /etc/birdnet/birdnet.conf` | Run `sudo bash install.sh reinstall`, which writes it. Do **not** copy `.env.example` there: that is the Docker environment file (`BIRDNET_*` keys), while `birdnet.conf` uses unprefixed keys (`CONFIDENCE=`, `RECS_DIR=`, …) and no prefix mapping is applied. |
| `Failed to set up mount namespacing: /tmp/birdnet-stream: No such file or directory` (exit `226/NAMESPACE`) | An older unit listed the tmpfs stream dir in `ReadWritePaths=`, which conflicts with `PrivateTmp=`. Upgrade, or run `sudo bash install.sh repair` to rewrite the unit. |
| `database recovery failed` | Run `birdnet-behavior --check-db`; restore from `$DATA/backups/` if the corruption is real (see [Database errors](#database-errors)). |
| `failed to install Ctrl+C handler` / `SIGTERM handler` | Probably a non-Unix or sandboxed environment without signal support. |
| `address already in use` | Another service is on port 8502. Change the bind address with `BIRDNET_LISTEN=0.0.0.0:<port>` (the binary reads `--listen` / `BIRDNET_LISTEN`; `BIRDNET_PORT` is only the Docker host-port mapping) or stop the other service. |
| `permission denied` reading `/etc/birdnet/birdnet.conf` | The config is `0640 root:<the service account's group>` because it holds secrets. `sudo bash install.sh repair` restores the ownership and permissions. |
| `another instance is running on this data directory (…/birdnet.lock is locked …)` | A previous process still holds the data directory — a restart overlapping a slow shutdown, or the binary started by hand beside the unit. The new process waits 30 s for it (`BNB_INSTANCE_LOCK_GRACE_SECS` to wait longer) and then refuses rather than open the same files. `sudo systemctl stop birdnet-behavior`, check `pgrep -a birdnet-behavior`, and start once. |

For most "won't start" cases — a stale unit, wrong permissions, a missing directory — the installer's repair fixes ownership and permissions, rewrites the unit and restarts, without touching your data:

```bash
sudo bash install.sh repair
```

**Docker: the container exits immediately.**

```bash
docker compose logs --tail=200 birdnet
docker compose run --rm birdnet birdnet-behavior --doctor   # the doctor in a one-shot container
```

When a model download fails, the entrypoint lists the common causes: no internet in the container, the model's host unreachable, or the volume out of disk. See [Model not found](#model-not-found).

## Web UI not reachable

The dashboard binds to all interfaces by default, so it is reachable from other devices on your LAN at `http://<pi-ip>:8502` (find the address with `hostname -I`). If it is not:

```bash
sudo systemctl status birdnet-behavior
ss -tlnp | grep 8502
sudo ufw allow 8502/tcp        # Ubuntu / Debian firewall; use firewall-cmd on RHEL / Fedora
ip -4 addr show | awk '/inet / && !/127\.0\.0\.1/ {print $2}'    # the address to browse to
curl -sf -o /dev/null -w "%{http_code}\n" http://<host>:8502/api/v2/health   # from another machine; expect 200
```

No row from `ss` means the daemon never finished starting — see [Service won't start](#service-wont-start). If you deliberately restricted access, `BIRDNET_LISTEN` will be `127.0.0.1:8502` (reachable only from the station itself); set it back to `0.0.0.0:8502` for LAN access. Viewing needs no login; `/admin*`, the Settings management tabs and every action that changes something do — use the auto-generated password from the install summary ([the admin password FAQ](./faq.md#how-do-i-find-or-reset-the-admin-password)).

## No detections appearing

First rule out the audio source.

**Bare metal (ALSA).**

```bash
arecord -l                                                     # list capture devices
# Use the card *id* — the word after "card N:" — not the index:
arecord -D plughw:CARD=PRO,DEV=0 -d 3 /tmp/test.wav && aplay /tmp/test.wav
grep -E '^(ALSA_CARD|RTSP_URL|PIPEWIRE_DEVICE)' /etc/birdnet/birdnet.conf   # what the station was told to use
sudo cp /etc/birdnet/birdnet.conf /tmp/birdnet.conf && sudo nano /tmp/birdnet.conf   # ALSA_CARD=plughw:CARD=<id>,DEV=0
sudo birdnet-behavior --apply-config /tmp/birdnet.conf         # validates, backs up, installs, restarts
```

Other sources: `pactl list short sources` and `parec --device=<source-name> /tmp/test.raw` (Ctrl-C after a few seconds) for PulseAudio / PipeWire; `nc -zv <camera-host> 554` for an RTSP camera (the doctor does the same probe). If PipeWire and ALSA are both set, PipeWire takes precedence; a local microphone runs alongside any number of RTSP streams, and the doctor lists every source the daemon will start.

**Docker.**

```bash
docker compose logs birdnet | grep -i 'audio source'      # which source was picked up?
grep -E '^BIRDNET_(ALSA|PIPEWIRE|RTSP)' .env              # is exactly one set?
docker compose restart birdnet
```

**Docker: no audio at all.**

```bash
ls -la /dev/snd/                                          # is the device visible on the host?
docker compose -f docker-compose.yml -f docker-compose.alsa.yml up -d   # use the ALSA overlay
docker compose logs -f birdnet
```

**Recordings exist but the database stays empty.**

```bash
ls -la "$(grep ^RECS_DIR /etc/birdnet/birdnet.conf | cut -d= -f2- | tr -d '"')"
sudo journalctl -u birdnet-behavior --since "1 hour ago" | grep -iE "inference|detection|model"
```

Look for `model failed to load` or `mel spectrogram` errors. The common causes:

- The model file is truncated. The doctor flags a file under 1 MB as suspicious; reinstall it (see [Model not found](#model-not-found)).
- `--watch-dir` does not match where recordings are written.
- Not enough memory for the model on a small board — see [Memory and CPU pressure](#memory-and-cpu-pressure-on-small-hardware).
- On a new station, **no location is set**. That does not stop detection, but without coordinates the occurrence filter is off and every species the model knows stays a candidate, so expect birds that do not belong in your area; set `LATITUDE` and `LONGITUDE`.

**Detections exist but confidence is always low.** Audio quality is almost always the cause. Check the microphone placement (out of the wind, with a foam windscreen), the ALSA gain (`alsamixer -c 1`, F4 for Capture), and a recording in a spectrogram viewer such as Audacity — bird calls should be clearly visible in the 1–8 kHz band. Then see [Tuning Detection Accuracy](./tuning.md).

**Birds are heard but a conversation near the microphone leaves a gap.** That is the human-sound rule, on by default: a window in which a human class ranks in the model's top 10 classes, and the windows either side, are not recorded. Set *Privacy Rank* to `0` in the Detection section of the Settings page (or `PRIVACY_RANK=0`) to switch it off; see [Same audio, same detections](./parity.md#moving-an-existing-station-to-birdnet-pis-results).

## Model not found

```bash
sudo -u "$SVC" ls "$DATA/models/"              # bare metal
docker compose exec birdnet ls /data/model/    # Docker
```

A BirdNET V2.4 station (the default for a new install) has three files there: `birdnet-v24.onnx`, `birdnet-v24_labels.txt` and `birdnet-v24-range-v2.onnx`. A BirdNET+ V3.0 station has the V3.0 model and labels CSV, and the geomodel pair. If they are missing, restore them:

- **Bare metal:** `sudo bash install.sh repair` installs whatever the configured classifier is missing (it re-checks V3.0's files against their SHA-256; V2.4's are trusted if present and not empty). Or install the V2.4 files yourself: `sudo -u "$SVC" env BIRDNET_MODEL_DIR="$DATA/models" birdnet-behavior --install-model birdnet-v24` and the same with `birdnet-v24-range-v2`. Running it again is cheap: a file that already matches its SHA-256 is not fetched again.
- **Docker:** restart the container; the entrypoint installs what is missing and says what it is doing. The next start resumes a BirdNET+ V3.0 download from the partial file; a V2.4 install starts the file again.

If a download fails, the message names the cause: no internet (the V2.4 files come from `huggingface.co` and `raw.githubusercontent.com`, the V3.0 files from this project's GitHub release with Zenodo as the fallback), a full disk (the model installer wants the file's size plus 512 MiB free), or a SHA-256 mismatch, which means the bytes were wrong and nothing was kept. To bring your own model, mount it over `/data/model` and set `BIRDNET_SKIP_MODEL_DOWNLOAD=1` (Docker), or `BIRDNET_SKIP_MODEL=1` when running the installer.

## Database errors

```bash
sudo -u "$SVC" birdnet-behavior --check-db
```

*PASSED* means the schema and pages are intact. *FAILED — corruption detected* means restoring from a backup: the station takes a snapshot every 7 days into `$DATA/backups/` and keeps the most recent 14, and also takes one at start-up whenever it had to recover the write-ahead log. [Backups & Recovery](../admin/backups.md#restoring) says how to put one in place, and the [field runbook's database section](../field/deployment.md#8-database-and-backup-policy) says why the write-ahead log has to be removed first. `birdnet-behavior --backup-db` makes a snapshot on demand.

## Behavioral analytics show "extension required"

On the **Analytics** page, the Activity Sessions, Species Retention and Next Species cards read *"the `duckdb-behavioral` extension is required…"*, and the startup log warns `duckdb-behavioral extension not loaded`.

Up to v0.9.1 the `duckdb-behavioral` community extension was compiled for a **specific DuckDB version**, and DuckDB refuses to load such a build into any other one. From v0.10.0 it is built on DuckDB's stable C API (`abi: "C_STRUCT"` in `/api/v2/analytics/status`) and loads into any engine whose stable C API is at least the one it was built for (DuckDB 2.0.0 keeps that API at v1.5.6), but the community registry still publishes it only under the DuckDB releases it was built for. A copy of the extension is also embedded in every release binary, and that copy is what a station uses when the cache and the registry cannot supply one.

This is **non-fatal.** Everything that reads SQLite directly — Migration, the Dawn Chorus, the Heatmap, Co-occurrence and the whole Time-series page — keeps working; only the extension-backed sessionize / retention / next-species queries are unavailable. The Analytics status badge says *connected* without claiming behavioral analytics are active.

**Diagnose it in one command.** `--doctor` will not answer this: it deliberately never opens DuckDB (a preflight open could contend with the running service), so it reports whether analytics is *configured*, not whether the extension loads. Use:

```bash
birdnet-behavior --verify-extension
```

It opens a throwaway DuckDB database, loads the extension the way the station does, prints the engine version, the extension version and what the embedded copy targets, and exits `0` when it loads and non-zero when it does not. To prove the *offline* guarantee — the one that matters on an air-gapped or metered station — run it with no network:

```bash
unshare -rn birdnet-behavior --verify-extension          # bare metal: a network namespace with no interfaces
docker run --rm --network none --entrypoint /usr/local/bin/birdnet-behavior \
    ghcr.io/tomtom215/birdnet-behavior:latest --verify-extension   # the image you run
```

With no network neither the extension cache nor the community registry can supply either extension, so a pass proves the embedded copies loaded. A station that passes with a network and fails without one has a packaging defect — the embedded extension targets the wrong DuckDB version and the registry has been covering for it — which you cannot repair yourself: open an issue with the output.

**What you can do.** On a station with network access, `birdnet-behavior --refresh-extension` force-installs the build the community registry has for the bundled DuckDB version, loads it to check, and exits (it needs `--analytics-db` or `ANALYTICS_DB_PATH` set). Upgrading to a newer release, which bundles an engine and extension that were built and pinned together, is the other fix.

## Every analytics dashboard is empty, and the logs say nothing

The Analytics and Time-series pages render but show no data, the health endpoint is green, `--doctor` is happy, and nothing in `journalctl -u birdnet-behavior` looks like an error. It does not recover on its own — stations have sat like this for days. This is **not** the extension problem above: there the extension-backed cards degrade and say so; here *every* dashboard is empty, including the ones that need no extension.

Check the journal for this line:

```bash
journalctl -u birdnet-behavior --no-pager | grep -i 'duckdb": Read-only'
# Failed to create directory "/home/pi/.duckdb": Read-only file system
```

Every dashboard query filters on a look-back window, which reaches DuckDB as `detection_date >= CURRENT_DATE - INTERVAL n DAYS`. `CURRENT_DATE` lives in DuckDB's ICU extension, as does every other way to name the current local date. Before the fix below, ICU was fetched at run time, into `$HOME/.duckdb`; the shipped systemd unit sets `ProtectHome=read-only`, so that write failed and every date-ranged query failed with it. The web layer renders a query error as a cached "temporarily unavailable" fragment, which is why nothing surfaced as an error.

**Fixed in versions after 0.13.1**, two ways over: DuckDB's extension directory now sits beside the analytics database inside the data directory, and the ICU binary is embedded in the release binary, so nothing is downloaded and `$HOME` is never touched. Upgrading is the fix, and `birdnet-behavior --verify-extension` now also runs the date-window query the dashboards open with, so it answers this too (including under `unshare -rn` / `--network none`).

If you are stuck on 0.13.1 and cannot upgrade yet, pointing `HOME` at the data directory — the one place the unit already allows writes — restores analytics at once:

```bash
sudo systemctl edit birdnet-behavior
# [Service]
# Environment=HOME=/home/pi/BirdNet-Behavior     # your data directory; "pi" is your account
sudo systemctl restart birdnet-behavior
```

The override is harmless to leave in place after upgrading, and unnecessary.

## `--doctor` reports a quarantined analytics database

The startup log shows *"analytics database is unusable; quarantining it and rebuilding from SQLite"*, and `--doctor` warns about a `….duckdb.corrupt.<timestamp>` file.

**No detections were lost, and nothing needs doing.** The DuckDB analytics store holds nothing but a copy of rows that live in SQLite, so when the file cannot be read — a bad block on the SD card, a half-written file after a power cut, a DuckDB version change — the station moves it aside and rebuilds it from SQLite on that same start. Delete the quarantined file once you are satisfied nothing else is wrong. If it keeps happening, the storage is the suspect: check `dmesg` for I/O errors and consider replacing the card.

## Memory and CPU pressure on small hardware

Symptoms: kernel OOM-killer messages in `dmesg`, container restarts in a loop, inference latency spikes, detections that lag minutes behind the audio. Mitigations, in order of effectiveness:

1. **Enable ZRAM.** The bare-metal installer sets it up on hosts with 2 GB of RAM or less during `install` and `reinstall` (not `repair`); `SKIP_ZRAM=1` opts out. To run it again: `sudo bash install.sh reinstall`.
2. **Reduce overlap.** Analysis windows advance by the window length minus `OVERLAP`, so the inference work per recording is `window ÷ (window − overlap)` times the work at no overlap. With BirdNET V2.4's 3 s windows, `OVERLAP=1.5` is twice the work of `OVERLAP=0.0`, and `2.0` is three times.
3. **Use the smaller model.** BirdNET V2.4 (the default for a new install) is a 54 MB file; the BirdNET+ V3.0 preview is 541 MB. This page does not give a memory figure for V2.4, because none has been measured.
4. **Disable analytics.** Analytics is compiled into every release binary and on by default: with no `--analytics-db` the daemon still opens `<database>.duckdb` beside the SQLite file, so *removing* the flag does not turn it off. Pass an **empty** value instead. On bare metal change the unit's `--analytics-db <path>` to `--analytics-db ""` (`sudo systemctl edit birdnet-behavior`, then restart); in Docker add a compose override that sets `BIRDNET_ANALYTICS_DB: ""` (the compose file sets it under `environment:`, so `.env` alone cannot blank it). The daemon logs `DuckDB analytics disabled via empty --analytics-db` on start. A `--no-default-features` build leaves the engine out entirely. DuckDB roughly doubles RAM use during sync.

## Notifications never arrive

```bash
apprise -t "test" -b "from birdnet" <YOUR_APPRISE_URL>     # the Apprise tool, independent of this station
sudo -u "$SVC" birdnet-behavior --doctor 2>&1 | grep -i apprise
```

The **Send Test Push Notification** button under **Settings → Alerts** uses the values saved on the Settings page, so a passing test means live detections will notify too. Common gotchas:

- `BIRDNET_NOTIFY_CONFIDENCE` defaults to `0.8`; lower it to see anything during testing.
- `BIRDNET_NOTIFY_TRIGGER` defaults to `each`. `new-species-daily` notifies on the first detection of each species per day, and `new-species` only while a species has fewer than 5 detections this week; use `each` while debugging.
- The Apprise URL must be URL-encoded if it contains special characters.

## Non-Latin species names show as boxes

The dashboard ships all its Latin fonts self-hosted, but non-Latin scripts (CJK, Devanagari, …) rely on the **client device's** system fonts. On a desktop or phone these are present already; on an always-on Pi kiosk, install a system CJK font:

```bash
sudo apt-get install -y fonts-noto-cjk
```

## Nothing here fits

Run these and attach the output to your issue; it answers most of the questions a maintainer would otherwise have to ask:

```bash
birdnet-behavior --version
sudo -u "$SVC" birdnet-behavior --doctor
uname -a; free -m; df -h
sudo journalctl -u birdnet-behavior -n 200 --no-pager
docker --version; docker compose version; docker compose logs --tail=200 birdnet    # Docker only
```

[`SUPPORT.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/SUPPORT.md) says which channel to use, and the architecture notes on the [risk register](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/12-risks.md) and the [diagnostics design](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/14-diagnostics.md) explain why the doctor checks what it does.
