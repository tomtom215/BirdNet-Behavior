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

## Web UI not reachable

```bash
sudo systemctl status birdnet-behavior
ss -tlnp | grep 8502
sudo ufw allow 8502/tcp   # if you use the Ubuntu firewall
```

Also confirm `BIRDNET_LISTEN` is `0.0.0.0:8502` (not `127.0.0.1:8502`) if you're connecting from another machine.

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

The `duckdb-behavioral` community extension is compiled for a **specific DuckDB version**, and DuckDB refuses to load an extension built for a different one (an extension built for DuckDB `v1.5.1` won't load into a binary that bundles `v1.5.3`). This is **non-fatal**: everything that reads SQLite directly — Migration, the Dawn Chorus, the Heatmap, Co-occurrence, and the whole Time-series page — keeps working; only the extension-backed sessionize / retention / next-species queries are unavailable. The Analytics status badge reflects this — it reports the database is *connected* but does not claim behavioral analytics are active.

To fix it, rebuild and republish the extension for the bundled DuckDB version in the [duckdb-behavioral](https://github.com/tomtom215/duckdb-behavioral) repository (or pin the `duckdb` crate to the version the published extension targets) and rebuild with `--features analytics`. Confirm the bundled version with `birdnet-behavior --doctor`. See the full [TROUBLESHOOTING.md](https://github.com/tomtom215/BirdNet-Behavior/blob/main/TROUBLESHOOTING.md) entry for details.
