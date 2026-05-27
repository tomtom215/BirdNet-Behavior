# Audio & Microphones

Microphone setup is the single most common support topic, so it gets its own page (`/admin/audio`) designed to be bulletproof.

![The audio settings page](../images/admin-audio.png)

## Your sources at a glance

Every input the station listens on appears in the source list — USB sound cards and RTSP cameras alike. Each row shows a live level meter with SNR, uptime, the last detection, and a 24-hour count, so you can tell at a glance whether a mic is healthy.

## Tuning a source

Expand any source with **▸ tune** to open its control panel:

- **Input gain** (−12 → +24 dB) with a zero mark,
- **Sample rate** (8 / 16 / 22.05 / 44.1 / 48 kHz),
- **Channels** (mono / left / right / stereo),
- **Bit depth** (16 / 24-bit PCM),
- **Pipeline toggles** — high-pass filter, DC-offset removal, auto-gain control, RTSP keepalive.

> Aim for peaks near −6 dB. Gain set too high clips the loudest calls and hurts identification more than a quiet signal does.

## Adding an RTSP camera

The **Add a source** wizard walks through three steps — URL (with a locked `rtsp://` prefix), optional auth, and a label — and shows a live reachability pill with the sniffed audio properties before you commit. A preview row lets you listen for ten seconds first.

> RTSP capture needs **`ffmpeg`** on the host. The bare-metal installer installs it automatically when your config has an `RTSP_URL` (re-run `sudo bash install.sh repair` if you add RTSP later), and the Docker image already bundles it. Without `ffmpeg`, RTSP sources record zero detections.

## Finding a USB mic

```bash
arecord -l
# card 1: Device [USB Audio], device 0: USB Audio [USB Audio]
arecord -D plughw:1,0 -d 3 /tmp/test.wav && aplay /tmp/test.wav   # test it
```

Set the device in `.env` (`BIRDNET_ALSA_DEVICE=plughw:1,0`) or via the CLI flag — see [Configuration](../getting-started/configuration.md).

## Common pitfalls

- USB hubs can drop audio under load — prefer a direct port on the Pi.
- RTSP over UDP drops packets on busy Wi-Fi — use `tcp` transport for stability (set under the researcher options).
