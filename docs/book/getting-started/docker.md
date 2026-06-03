# Running with Docker

The [quick-start script](./installation.md#option-1--docker-quick-start-recommended) is the easiest path. If you prefer to do it by hand, you only need to decide **two** things:

1. **Station location** — latitude and longitude.
2. **Audio source** — one of: USB/ALSA mic, PulseAudio/PipeWire, or an RTSP stream URL.

Everything else has sensible defaults — including DuckDB behavioral analytics, which is built into the image and enabled by default.

## 1. Clone and create your `.env`

```bash
git clone https://github.com/tomtom215/BirdNet-Behavior.git
cd BirdNet-Behavior
cp .env.example .env
```

## 2. Edit the REQUIRED section

Pick **one** audio variable and leave the others blank.

```dotenv
# Station location
BIRDNET_LATITUDE=42.3601
BIRDNET_LONGITUDE=-71.0589

# Audio source — set exactly ONE
BIRDNET_ALSA_DEVICE=plughw:1,0          # USB/ALSA mic  (use `arecord -l` to find it)
# BIRDNET_RTSP_URL=rtsp://cam.lan:554/stream
# BIRDNET_PIPEWIRE_DEVICE=default

# Image tag — pin a release like 0.6.0, or leave as latest (analytics is built in)
BIRDNET_IMAGE_TAG=latest
```

## 3. Start the stack

`docker compose up -d` pulls the image and starts everything — analytics is built in, so there is no variant to choose.

```bash
# A) RTSP camera / multi-stream / file-watch mode — no microphone hardware
docker compose up -d

# B) USB / ALSA microphone (most Raspberry Pi setups)
docker compose -f docker-compose.yml -f docker-compose.alsa.yml up -d

# C) PulseAudio / PipeWire (desktop Linux)
docker compose -f docker-compose.yml -f docker-compose.pulse.yml up -d
```

Your recordings, database, and cached model live in the `birdnet-data` named volume, so they survive restarts and image upgrades.

## 4. Watch the first-run model download

On a fresh install the container downloads the BirdNET+ model (~541 MB) — from the same GitHub release line as the image, sha256-verified, falling back to Zenodo — before starting the web server. This happens **exactly once** per named volume.

```bash
docker compose logs -f birdnet
```

Interrupted downloads **resume** on the next start (`curl --continue-at -`), so a dropped connection is not fatal — just `docker compose up -d` again. The health check has a 15-minute start period to accommodate slow first-run downloads.

Once you see `Starting birdnet-behavior`, open **<http://localhost:8502>**.

> If the download fails, the entrypoint fails loud (not silent) with the exact cause and remediation. See [Troubleshooting → Model not found](../guides/troubleshooting.md#model-not-found).

## Single `docker run` (no compose)

For the absolute minimum — no clone, no compose files — edit the four marked values and paste:

```bash
docker run -d \
  --name birdnet-behavior \
  --restart unless-stopped \
  -p 8502:8502 \
  -v birdnet-data:/data \
  --device /dev/snd \
  --group-add audio \
  -e BIRDNET_LATITUDE=42.3601 \
  -e BIRDNET_LONGITUDE=-71.0589 \
  -e BIRDNET_ALSA_DEVICE=plughw:1,0 \
  -e BIRDNET_LISTEN=0.0.0.0:8502 \
  ghcr.io/tomtom215/birdnet-behavior:latest
docker logs -f birdnet-behavior         # watch the model download
```

For RTSP, drop `--device /dev/snd --group-add audio` and set `BIRDNET_RTSP_URL=` instead of `BIRDNET_ALSA_DEVICE`.

## Pre-built images

| Tag | Contents |
|---|---|
| `ghcr.io/tomtom215/birdnet-behavior:latest` | Latest release — includes DuckDB behavioral analytics |
| `ghcr.io/tomtom215/birdnet-behavior:0.6.0` | A specific version (same contents) |

Published for `linux/amd64` and `linux/arm64`, and signed with [cosign](https://docs.sigstore.dev/) (keyless). There is no separate `-analytics` image — every image has analytics built in.

## Data layout & compose reference

All persistent data lives in one Docker volume at `/data`:

```text
/data/
  model/        BirdNET+ ONNX model + labels (auto-downloaded)
  recordings/   Audio segments from the capture pipeline
  cache/        Wikipedia species image cache
  birdnet.db    SQLite detections database
  analytics.db  DuckDB behavioral analytics (on by default)
```

| File | Purpose |
|---|---|
| `quickstart.sh` | One-command bootstrap — auto-detects audio, writes `.env`, starts the stack |
| `docker-compose.yml` | Base compose — works for RTSP and file-watch mode |
| `docker-compose.alsa.yml` | Overlay for USB/ALSA microphone (passes `/dev/snd`) |
| `docker-compose.pulse.yml` | Overlay for PulseAudio/PipeWire (mounts the PA socket) |
| `.env.example` | Documented template for every supported environment variable |
