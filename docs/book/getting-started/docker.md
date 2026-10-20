# Running with Docker

The [quick-start script](./installation.md#option-1--docker-quick-start) is the easiest path. If you prefer to do it by hand, you only need to decide **two** things:

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

# Timezone — detections are filed under the container's local hours, which
# come from this. Unset, the container runs on UTC and says so at start.
TZ=America/New_York

# Audio source — set exactly ONE
BIRDNET_ALSA_DEVICE=plughw:CARD=PRO,DEV=0   # USB/ALSA mic — see the note below
# BIRDNET_RTSP_URL=rtsp://cam.lan:554/stream
# BIRDNET_PIPEWIRE_DEVICE=default

# Image tag — pin a release such as 0.17.1 (the releases are listed at
# https://github.com/tomtom215/BirdNet-Behavior/releases), or leave it as latest
# (analytics is built in)
BIRDNET_IMAGE_TAG=latest

# Classifier — leave unset. A new volume then runs BirdNET V2.4 (with its range
# filter); a volume that already holds the BirdNET+ V3.0 model keeps V3.0.
# BIRDNET_CLASSIFIER=birdnet-v24     # or birdnet-v3 for the V3.0 preview
```

> **Name the card, do not number it.** Run `arecord -l` on the *host* and read
> the word after `card N:` — that is the card's **id**, and
> `plughw:CARD=<id>,DEV=0` addresses it by that id. A card **index** (the `1` in
> `plughw:1,0`) is assigned in detection order, so a reboot, a re-plug, or a
> second sound device can renumber it, and nothing in this station re-resolves a
> device string once capture has started: the supervisor retries the old name
> forever and the microphone stays down until someone intervenes. See
> [Audio & Microphones](../admin/audio.md#name-the-card-dont-number-it) for the full
> explanation and for how to pin a name with a udev rule.

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

## 4. Watch the first-run model install

On a new volume the container installs the classifier before it starts the web server, and says which one in a single `Classifier:` log line. By default that is **BirdNET V2.4** and its **range filter v2**, about 85 MB: the container asks the binary (`birdnet-behavior --install-model birdnet-v24`, then `birdnet-v24-range-v2`), which downloads each file, checks it against a SHA-256 compiled into the binary, and renames it into place only when it matches. The model comes from Hugging Face and the range filter from `raw.githubusercontent.com` (see [Installation](./installation.md#which-classifier-a-new-install-runs)); the label file is written by the binary itself. A file that is already on the volume is not fetched again, so a restart installs nothing, and the install happens **once** per named volume.

```bash
docker compose logs -f birdnet
```

Which classifier a start runs, first match:

| Set | Runs |
|---|---|
| `BIRDNET_CLASSIFIER=birdnet-v24` or `birdnet-v3` | that one (any other value stops the container and names the two) |
| `BIRDNET_MODEL` to a model of your own | your model, with the BirdNET+ V3.0 labels and occurrence filter as defaults, unless it names the V2.4 file |
| nothing, and the V3.0 model file is already on the volume | **V3.0**, with one log line saying the volume is kept on it and how to change that |
| nothing, on a new volume | **BirdNET V2.4** |

So an upgrade never switches a running station's model: every volume created by a release that defaulted to V3.0 holds the V3.0 file. Setting `BIRDNET_CLASSIFIER=birdnet-v24` on such a volume does move it to V2.4, and its history then contains rows from two networks (see [the classifiers page](../admin/classifiers.md#choosing-and-changing-the-primary-classifier)).

With `BIRDNET_CLASSIFIER=birdnet-v3` the container instead downloads the BirdNET+ V3.0 model (~541 MB) and its occurrence model from this project's `models-v3.0-preview3` release, with Zenodo as the fallback. **That download resumes** if it is interrupted (`curl --continue-at -`); the V2.4 install does not resume, it starts the file again on the next start. The health check has a 15-minute start period to accommodate a slow first run either way.

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
  -e BIRDNET_ALSA_DEVICE=plughw:CARD=PRO,DEV=0 \
  -e BIRDNET_LISTEN=0.0.0.0:8502 \
  ghcr.io/tomtom215/birdnet-behavior:latest
docker logs -f birdnet-behavior         # watch the model install
```

For RTSP, drop `--device /dev/snd --group-add audio` and set `BIRDNET_RTSP_URL=` instead of `BIRDNET_ALSA_DEVICE`.

## Pre-built images

| Tag | Contents |
|---|---|
| `ghcr.io/tomtom215/birdnet-behavior:latest` | Latest release — includes DuckDB behavioral analytics |
| `ghcr.io/tomtom215/birdnet-behavior:0.17.1` | A specific release (`X.Y.Z`; `X.Y` and `X` tags follow the newest patch). Replace `0.17.1` with a release that exists |

Published for `linux/amd64` and `linux/arm64`, and signed with [cosign](https://docs.sigstore.dev/) (keyless). There is no separate `-analytics` image — every image has analytics built in.

## Data layout & compose reference

All persistent data lives in one Docker volume at `/data`. Name it (`-v birdnet-data:/data`, as the compose file does): the image declares `/data` a volume, so a `docker run` with no `-v` still gets one there, but an anonymous volume with a random name that nothing reattaches to the next container, and that `docker rm -v` or `docker run --rm` deletes.

The data is lost with the container only when the data directory is *not* on a mount at all: `DB_PATH` in a mounted `birdnet.conf` pointing outside `/data`, or an image built without the `VOLUME` line. The station says so in three places: the Station Health banner, the badge in the page header, and **Data volume** in `--doctor` (a warning, not an error, because the station works) with the fix. The check reads the container's mount table, not the volume's name, so a volume of any kind (an anonymous one too) or a bind mount counts as somewhere the data outlives the container.

The layout inside it:

```text
/data/
  model/        the classifier's ONNX model + labels (and the occurrence filter), installed on first start
  recordings/   Audio segments from the capture pipeline
  cache/        Wikipedia species image cache
  analytics.db  DuckDB behavioral analytics (compose sets BIRDNET_ANALYTICS_DB)
  BirdNet-Behavior/
    birds.db    SQLite detections database (see note below)
```

> The image sets no `DB_PATH`, so the database file is named by the binary's
> default, `$HOME/BirdNet-Behavior/birds.db` — and the container user's home
> directory is `/data`. Set `DB_PATH` in a mounted `birdnet.conf` to put it
> somewhere else.

| File | Purpose |
|---|---|
| `quickstart.sh` | One-command bootstrap — auto-detects audio, writes `.env`, starts the stack |
| `docker-compose.yml` | Base compose — works for RTSP and file-watch mode |
| `docker-compose.alsa.yml` | Overlay for USB/ALSA microphone (passes `/dev/snd`) |
| `docker-compose.pulse.yml` | Overlay for PulseAudio/PipeWire (mounts the PA socket) |
| `.env.example` | Documented template for every supported environment variable |
