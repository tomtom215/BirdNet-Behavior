# Troubleshooting

**Start here for any problem — run the built-in diagnostic:**

```bash
# Bare metal
sudo -u birdnet birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

It prints a one-screen report covering CPU, configuration, audio reachability, the model file, database integrity, disk space, tool dependencies and network — and **every problem comes with a concrete suggested fix.** Exit code: `0` = all good, `1` = warnings only, `2` = at least one error.

> For a deeper, symptom-organized guide, see [`TROUBLESHOOTING.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/TROUBLESHOOTING.md) in the repository.

## Service won't start

```bash
sudo journalctl -u birdnet-behavior -f
# Common cause: no audio source set in /etc/birdnet/birdnet.conf
```

For most "won't start" cases — a stale systemd unit, wrong permissions, a missing directory — run the installer's repair, which fixes ownership/permissions, rewrites the unit, and restarts without re-downloading or touching your data:

```bash
sudo bash install.sh repair
```

> An older unit that failed with `226/NAMESPACE` (a tmpfs stream-dir mount conflict) is fixed in current releases — upgrade, or run `install.sh repair` to rewrite the unit.

## Web UI not reachable

The dashboard binds to all interfaces by default, so it's reachable from other devices on your LAN out of the box at `http://<pi-ip>:8502` (find the address with `hostname -I`). If it's unreachable:

```bash
sudo systemctl status birdnet-behavior
ss -tlnp | grep 8502
sudo ufw allow 8502/tcp   # if you use the Ubuntu firewall
```

No row from `ss` means the daemon never finished starting — see [Service won't start](#service-wont-start) above. If you deliberately restricted access, `BIRDNET_LISTEN` will be `127.0.0.1:8502` (reachable only from the Pi); set it back to `0.0.0.0:8502` for LAN access. Viewing the dashboard needs no login; only `/admin` does — use the auto-generated password from the install summary (see [the admin password FAQ](./faq.md#how-do-i-find-or-reset-the-admin-password)).

## No detections appearing (bare metal)

```bash
arecord -l                                                       # list capture devices
arecord -D plughw:1,0 -d 3 /tmp/test.wav && aplay /tmp/test.wav  # test the mic
sudo nano /etc/birdnet/birdnet.conf                              # set ALSA_CARD=plughw:X,Y
sudo systemctl restart birdnet-behavior
```

## No detections appearing (Docker)

```bash
docker compose logs birdnet | grep -i 'audio source'      # which source was picked up?
grep -E '^BIRDNET_(ALSA|PIPEWIRE|RTSP)' .env              # is exactly one set?
docker compose restart birdnet
```

## Docker: no audio

```bash
ls -la /dev/snd/                                          # is the device visible on the host?
docker compose -f docker-compose.yml -f docker-compose.alsa.yml up -d   # use the ALSA overlay
docker compose logs -f birdnet
```

## Model not found

```bash
ls ~/BirdNet-Behavior/models/                  # bare metal
docker compose exec birdnet ls /data/model/    # Docker
# If empty, re-run the installer or restart the container — it auto-downloads.
```

If a download fails, the entrypoint prints the exact cause (no internet in the container, a Zenodo outage, or a full disk) and the next start **resumes** from the partial file. To bring your own model, mount it over `/data/model` and set `BIRDNET_SKIP_MODEL_DOWNLOAD=1`.

## Non-Latin species names show as boxes

The dashboard ships all its Latin fonts self-hosted, but non-Latin scripts (CJK, Devanagari, …) rely on the **client device's** system fonts. On a desktop or phone these are present already; on an always-on Pi kiosk, install a system CJK font:

```bash
sudo apt-get install -y fonts-noto-cjk
```

## Behavioral analytics show "extension required"

On the **Analytics** page, the Activity Sessions, Species Retention, and Next Species cards read *"the `duckdb-behavioral` extension is required…"* and the startup log warns `duckdb-behavioral extension not loaded`.

The `duckdb-behavioral` community extension is compiled for a **specific DuckDB version**, and DuckDB refuses to load an extension built for a different one (an extension built for DuckDB `v1.5.3` won't load into a binary that bundles `v1.5.5`). This is **non-fatal**: everything that reads SQLite directly — Migration, the Dawn Chorus, the Heatmap, Co-occurrence, and the whole Time-series page — keeps working; only the extension-backed sessionize / retention / next-species queries are unavailable. The Analytics status badge reflects this — it reports the database is *connected* but does not claim behavioral analytics are active.

To fix it, rebuild and republish the extension for the bundled DuckDB version in the [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) repository (or pin the `duckdb` crate to the version the published extension targets) and rebuild with `--features analytics`. Confirm the bundled version with `birdnet-behavior --doctor`. See the full [TROUBLESHOOTING.md](https://github.com/tomtom215/BirdNet-Behavior/blob/main/TROUBLESHOOTING.md) entry for details.

## Every analytics dashboard is empty, and the logs say nothing

The Analytics and Time-series pages render but show no data, the health endpoint
is green, and nothing in `journalctl -u birdnet-behavior` looks like an error.
It does not recover on its own — stations have sat like this for days.

Check the journal for this line:

```bash
journalctl -u birdnet-behavior --no-pager | grep -i 'duckdb": Read-only'
# Failed to create directory "/home/pi/.duckdb": Read-only file system
```

Every dashboard query filters on a date window, which reaches DuckDB as
`detection_date >= CURRENT_DATE - INTERVAL n DAYS`. `CURRENT_DATE` lives in
DuckDB's ICU extension, which is **not** statically linked, so DuckDB fetches it
— by default into `$HOME/.duckdb`. The shipped systemd unit sets
`ProtectHome=read-only`, so that write fails and every date-ranged query fails
with it. The web layer turns a query error into a cached "temporarily
unavailable" fragment, which is why nothing surfaces as an error.

**Fixed in versions after 0.13.1**, two ways over: DuckDB's extension directory
now sits beside the analytics database inside the data directory, and the ICU
binary is embedded in the release binary so no download is needed at all.
Upgrading is the fix.

To confirm a build is healthy — on the station, or in an image, with or without
network:

```bash
birdnet-behavior --verify-extension
```

It loads both extensions and runs the date-window query the dashboards open
with, exiting non-zero if either fails.

If you are stuck on 0.13.1 and cannot upgrade yet, pointing `HOME` at the
station's data directory restores analytics immediately. That directory is the
one place the unit already grants write access (`ReadWritePaths=`), so DuckDB's
`.duckdb` cache lands somewhere it is allowed to:

```bash
sudo systemctl edit birdnet-behavior
# [Service]
# Environment=HOME=/home/pi/BirdNet-Behavior     # your data dir; pi → your user
sudo systemctl restart birdnet-behavior
```

That override is harmless to leave in place after upgrading, and unnecessary.

## `--doctor` reports a quarantined analytics database

The startup log shows *"analytics database is unusable; quarantining it and
rebuilding from SQLite"*, and `--doctor` warns about a
`…​.duckdb.corrupt.<timestamp>` file.

**No detections were lost, and nothing needs doing.** The DuckDB analytics store
holds nothing but a copy of rows that live in SQLite, so when the file cannot be
read — a bad block on the SD card, a half-written file after a power cut, a
DuckDB version change — the station moves it aside and rebuilds it from SQLite
on that same start. Analytics comes back by itself.

Delete the quarantined file once you are satisfied nothing else is wrong; it is
kept only so you can look at it. If this keeps happening, the storage is the
suspect — check `dmesg` for I/O errors and consider replacing the card.
