# Hardware & Audio Sources

You don't need much — a Raspberry Pi (or any Linux box) and something to listen with. This page helps you choose.

## Which computer?

| Device | Notes |
|---|---|
| **Raspberry Pi 5** | Recommended. Comfortable headroom for behavioral analytics and the live spectrogram. |
| **Raspberry Pi 4B / 400** | Fully supported and very capable for everyday use. |
| **Older 64-bit-capable Pis (3B+, Zero 2 W)** | **Not a supported target.** [STABILITY](https://github.com/tomtom215/BirdNet-Behavior/blob/main/STABILITY.md) lists the Pi 4, 5 and 400, and this repository records no test on a 3B+ or a Zero 2 W. The aarch64 binary needs the **64-bit** Pi OS (a 32-bit OS cannot run it at all), and these boards have 1 GB and 512 MB of RAM, which is tight. If you try one anyway, turn analytics off to save memory: pass an *empty* value, `--analytics-db ""` (removing `--analytics-db` only falls back to `<database>.duckdb`, which leaves it on), as [Troubleshooting](../guides/troubleshooting.md#memory-and-cpu-pressure-on-small-hardware) shows. |
| **Any x86_64 Linux** | Fully supported — an old laptop or mini-PC is a great always-on host. |

BirdNet-Behavior is undemanding — there's no Python interpreter or virtualenv resident around the model — so the limiting factor is usually storage for recordings, not compute. Budget **about 1.5 GB free** to start, most of it for recordings and the database (the default BirdNET V2.4 files are about 85 MB; the V3.0 preview's are 541 MB).

## Which microphone?

Any of these work — the goal is a clean signal, not an expensive one.

- **USB conference/lavalier mics** and **USB sound cards** with an electret capsule are the easy default. They show up as an ALSA card (`arecord -l`) and need no drivers.
- **Dedicated USB measurement mics** (e.g. an omnidirectional electret) give the best results outdoors.
- **An old USB webcam's mic** will even work for a first test.

What matters more than the mic:

- **Placement** — outside, off a hard wall (which echoes), with a simple foam windscreen. Wind is the number-one source of false positives.
- **Levels** — aim for peaks near −6 dB on the [Audio settings](../admin/audio.md) level meter. Too hot clips the loudest calls; too quiet buries the faint ones.

## RTSP cameras (no microphone hardware)

Many bird-box and feeder cameras expose an RTSP stream that carries audio. Point BirdNet-Behavior at the URL and it treats the camera as a microphone — no sound card required. You can run **several streams at once**.

```dotenv
BIRDNET_RTSP_URL=rtsp://cam.lan:554/stream
# or multiple, comma-separated:
BIRDNET_RTSP_URLS=rtsp://cam1.lan/stream,rtsp://cam2.lan/stream
```

RTSP capture needs **`ffmpeg`** on the host. When your config already has an `RTSP_URL`, the bare-metal installer (`install`/`repair`) installs it for you via apt, or prints the exact command if it can't; the Docker image bundles it. Add the camera under [Cameras and streams](../admin/audio.md#adding-a-camera-or-stream) on the Capture tab — which also takes HTTP/Icecast, HLS, RTMP, UDP and SRT streams — and prefer **TCP** transport on busy Wi-Fi. `--doctor` checks that the address resolves and its port answers.

## "Watch a folder" mode

If you already have a pile of recordings, BirdNet-Behavior can classify a folder of audio files instead of listening live — handy for batch-processing field recordings.

Next: [Configuration](./configuration.md) to wire your choice in, then [First Steps](./first-steps.md).
