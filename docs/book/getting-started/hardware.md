# Hardware & Audio Sources

You don't need much — a Raspberry Pi (or any Linux box) and something to listen with. This page helps you choose.

## Which computer?

| Device | Notes |
|---|---|
| **Raspberry Pi 5** | Recommended. Comfortable headroom for behavioral analytics and the live spectrogram. |
| **Raspberry Pi 4B / 400** | Fully supported and very capable for everyday use. |
| **Raspberry Pi 3B+** | Works on the 64-bit Pi OS. Tight on RAM — consider disabling analytics (remove `--analytics-db`) to keep memory low. |
| **Any x86_64 Linux** | Fully supported — an old laptop or mini-PC is a great always-on host. |

BirdNet-Behavior is undemanding — there's no Python interpreter or virtualenv resident around the model — so the limiting factor is usually storage for recordings, not compute. Budget **~1.5 GB free** to start (541 MB of that is the model).

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

RTSP capture needs **`ffmpeg`** on the host. When your config already has an `RTSP_URL`, the bare-metal installer (`install`/`repair`) installs it for you via apt, or prints the exact command if it can't; the Docker image bundles it. Use the [Add-RTSP wizard](../admin/audio.md#adding-an-rtsp-camera) in the UI to test reachability and sniff the audio properties before committing, and prefer **TCP** transport on busy Wi-Fi.

## "Watch a folder" mode

If you already have a pile of recordings, BirdNet-Behavior can classify a folder of audio files instead of listening live — handy for batch-processing field recordings.

Next: [Configuration](./configuration.md) to wire your choice in, then [First Steps](./first-steps.md).
