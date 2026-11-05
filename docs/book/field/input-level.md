# Input Level

How loud the operating system delivers a microphone to the station, how to read that, and why the answer is never "100%".

This page separates what was run, what was read from a source and what is a judgement, and says which is which. **Nothing here was measured on a sound card**: the machine it was researched on has none. The mixer readings below come from real PulseAudio and PipeWire servers with virtual sources, from `amixer` against an emulated mixer, and from output that other people published.

## Why it matters

The station records 16-bit integers (`arecord -f S16_LE`). A level set too high clips the loudest calls, and a clipped sample cannot be recovered later by any setting. A level set too low costs the quiet calls their margin above the noise floor. Neither is an error the station can see by itself, except the extreme: a source that delivers only zeros, or only full-scale samples, is reported by the source-fault alert.

What to aim for is stated differently by the two sources that were found:

- Cornell Lab's eBird help: set the record level so that "the loudest part of the target vocalization peaks at a safe level (such as -12 dB)", and do not change the level during a recording.
- The Audacity manual, for general recording: "minus 6 dB is a good target".

They differ by 6 dB, and no source found tests either against a bird classifier. **No controlled study of recording level against BirdNET's accuracy was found**, and whether BirdNET normalises amplitude internally is unknown. Treat -12 to -6 dB as the range for the loudest call's peak, and clipping as the thing to rule out.

## Why not 100%

A mixer control's top is not a neutral setting. It is whatever the codec's designer made the maximum, and it varies a lot:

| Device | Control and range | At the top | A published or vendor setting |
|---|---|---|---|
| A USB audio codec in a Linux forum post (published, not measured here) | `Mic` capture, 0 to 16 | +23.81 dB | 8 of 16 is 50%, +11.90 dB |
| WM8960 codec of the Raspberry Pi HATs (Linux kernel table) | `Capture Volume`, 0 to 63 steps of 0.75 dB from -17.25 dB | +30.00 dB, and `Mic Boost` adds up to +29 dB | Seeed's saved state is 39 of 63, 62%, +12 dB |

So a USB microphone at 100% can sit 24 dB above its own 0 dB, and a HAT at 100% with the boost on 59 dB above it. 75% of the range works out to about +18 dB on both (+17.9 dB on the USB codec, +18.0 dB on the WM8960's gain control alone); the guide's cap, below, is built from that coincidence and is a guard rail, not a published recommendation.

## Reading it

**ALSA** (`ALSA_CARD`, e.g. `plughw:1,0` or `plughw:CARD=PRO,DEV=0`):

```bash
amixer -c 1 scontents
```

Look for the controls whose `Capabilities:` include `cvolume`: `Mic`, `Capture` or `Internal Mic`. `Capture 8 [50%] [11.90dB] [on]` is the raw step, its percentage of the control's range, its gain and its capture switch. Three things the readings showed:

- `amixer`'s percentage is of the **raw range** (its manual: `-R` "is the default mode"); `alsamixer` shows a mapped percentage (`-M`, "more natural for human ear"). The same setting can show two numbers. The dB figure is the one to trust.
- Ask for the card (`-c 1`, or `-D hw:CARD=PRO`), never `default`. Through the sound server the mixer is emulated: `amixer -D pulse scontents` printed `Capture 0 - 65536`, 100%, no dB.
- A negative dB value needs `--` before it, or `amixer` reports `invalid option -- '6'`.

**PulseAudio, and PipeWire through `pipewire-pulse`** (`PIPEWIRE_DEVICE`):

```bash
pactl --format=json list sources
```

The percentage is **cubic**: on PulseAudio 16.1 and `pipewire-pulse` 1.0.5, 50% was -18.06 dB and 150% was +10.57 dB. 100% is 0 dB of the source's own scale, which is unity for a software or virtual source, and for one with a hardware volume control the hardware's maximum (this is read from the PulseAudio and PipeWire source files and was not observed on hardware). `Base Volume` is the point on that scale where the hardware gain is its own 0 dB, so `volume dB - base volume dB` is the gain on top of the hardware's 0.

Do not take `wpctl get-volume`'s exit status as an answer: for a source that does not exist it exited 0 with the message on stderr (WirePlumber 0.4.17 and 0.5.8). `wpctl` ids also change between runs. `pactl` exited 1 for a missing source and for no server.

## What the doctor says

```bash
birdnet-behavior --doctor
```

reads each configured microphone's capture gain this way, with a deadline on every tool, and grades it:

| Finding | When | Status |
|---|---|---|
| within the cap | at or below **75%** and at or below **+18 dB** | pass |
| above the cap | above either | warning, with a command that reaches it |
| at the maximum | 100% on a control with a hardware range | warning |
| capture switched off | the control's capture switch is off | warning |
| not read | no `amixer` or `pactl`, no sound server, a fixed-gain device | skipped, with the reason |

A finding here is at most a warning. A warning makes `--doctor` exit 1 (a failure exits 2); the service's start gate (`--doctor-gate`) is stopped only by failures of the checks it names, and this is not one, so a mixer setting can never stop the station starting. The cap is the lesser of 75% and +18 dB of hardware gain; the evidence for it is **low** (three devices with public tables, one design judgement), so it flags the extremes and cannot tell you that a level in range is right. For a PulseAudio source with no hardware control, 100% is unity and the doctor warns only above it.

## Setting it, and why the station does not

The station never changes a mixer. The command the doctor prints is for you to read and run. Three reasons, in order of weight:

1. **It would not last.** `alsa-restore` stores the mixer when the system shuts down cleanly (`alsactl store` on stop) and restores it every time the card appears (a udev rule, `alsactl restore`). A change made at run time is replaced by the stored one when the microphone is replugged and lost if the power is cut. Storing needs root (`sudo alsactl store`), which the service, running with `ProtectSystem=strict`, cannot and should not do, and it would store the whole machine's mixer state. For PipeWire, WirePlumber keeps its own per-user state (a `restore-stream` file was seen under a user's state directory on WirePlumber 0.4.17), and a service user's PipeWire instance is not the desktop user's (inferred, not tested).
2. **The right level is a measurement.** It is the one that leaves the loudest call below full scale, and only the recorded signal says so.
3. **Do not change it during a recording** (Cornell Lab), and a station records all day.

To set it by hand:

```bash
# ALSA: an exact raw value, read back afterwards
amixer -c 1 -- sset 'Mic' capture 12
amixer -c 1 sget 'Mic'
sudo alsactl store            # keep it across a power cut and a replug

# PulseAudio / PipeWire: an absolute percentage, never "+10%" or a linear factor
pactl set-source-volume <source-name> 75%
pactl --format=json list sources     # read it back; the percentage is cubic
```

Read back after setting: a set can succeed and change nothing (`wpctl set-volume` exited 0 on a PipeWire virtual node and did not change it, cause unknown). Leave `Auto Gain Control` and the microphone-boost switches alone unless you know what they do; they change the level during recording.

## Check what actually arrives

The level to rule out is clipping, and the station's own recording shows it. Record ten seconds of the loudest realistic sound and read the peak:

```bash
arecord -D plughw:1,0 -f S16_LE -r 48000 -c 1 -d 10 /tmp/level-test.wav
ffmpeg -hide_banner -nostats -i /tmp/level-test.wav -af volumedetect -f null - 2>&1 | grep max_volume
```

`max_volume: -32.6 dB` is what `volumedetect` reports for the repository's reference magpie recording (`tests/testdata/Pica_pica_30s.wav`), and an independent calculation of the same file's peak gives -32.60 dBFS. That recording is quiet and is identified correctly; the thing to avoid is a peak at or near 0 dB, not a low one.

## In Docker

The image has `amixer` but not `pactl`, and the container shares the host's `/dev/snd`, so a mixer change made inside it changes the host's. The doctor skips the PulseAudio check there and says why.

## What is not established

- **Hardware.** No reading here comes from a sound card. The USB and HAT figures are published or taken from kernel tables; the HiFiBerry post is a vendor forum.
- **The cap.** 75% and +18 dB are a judgement. No source gives a universal limit.
- **BirdNET's sensitivity to level** in either direction, and to automatic gain control, are unmeasured.
- **macOS.** `input volume of (get volume settings)` is documented by Apple for the default input device; nothing else about macOS input levels was checked, and the doctor does not read it.
- **Network streams** have no operating-system level to read.
