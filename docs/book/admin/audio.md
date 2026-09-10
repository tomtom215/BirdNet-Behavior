# Audio & Microphones

Microphone setup is the single most common support topic, so it gets its own tab (**Station → Capture**, `/station/capture`; the old `/admin/audio` redirects there) designed to be bulletproof.

![The audio settings page](../images/admin-audio.png)

## Your sources at a glance

Every input the station listens on appears in the source list — USB sound cards and RTSP cameras alike. Each row shows a live level meter with SNR, uptime, the last detection, and a 24-hour count, so you can tell at a glance whether a mic is healthy.

You can run **several sources at once** — one or more USB/ALSA mics, a PipeWire source, and any number of RTSP streams, in any combination (this is the headline gap in the original BirdNET-Pi). Each source captures into its own subprocess, tags its recordings with its own stream label, and gets its own health gauge, so detections stay attributable to the source that heard them.

## Independent supervision (built for the field)

Every source is supervised on its own, which is what makes a multi-camera station survivable when nobody is on site:

- **One source failing never disturbs the others.** A dead capture process — a camera that rebooted, a mic that was unplugged, a network blip — is restarted on its own with capped exponential backoff (2 s → 4 s → … → 60 s, then every 60 s **forever**; a source down for an hour is still recording when it comes back on hour two). The other sources keep recording and detection never pauses.
- **Silent stalls are caught too.** A source whose process is still *alive* but has stopped delivering audio — a wedged RTSP session, a mic hung after a USB re-enumeration — is detected by watching its segment output: no fresh recording for several segment-durations and it is restarted exactly like a crash. A plain "is the process running?" check can't see this; it's the failure mode that quietly loses a whole night otherwise. (It fails open while the system clock is unsynced, so a wrong boot-time clock never triggers a false restart.)
- **A source that keeps dying and coming back is caught as well.** A marginal USB connection or an under-powered hub gives a source that dies every few minutes and restarts in seconds: it is live at every glance, its backoff never grows, and its uptime strip stays green, but every restart loses the audio around it. Restarts are counted over the last hour, and five or more make the source *flapping*: a line on its Station Health card, an issue in the banner, a warning in the log, and the `flapping` station-health condition for the notifier.
- **You can see it.** The per-source `birdnet_audio_source_up{source="…"}` Prometheus gauge reflects real liveness, and a source that has been down a couple of minutes logs a loud, rate-limited warning to the journal.
- **You can say which source Listen plays.** Each row carries **Make listen default**; the source you pick is what `/stream` serves — and so what the Listen button plays — when the listener has not chosen one of their own. A two-microphone station (feeder and nest box) has one that people actually want to hear, and this is where it says so. If that source is later disabled or removed, Listen falls back to the first working source rather than going silent.
- **You can restart one source by hand.** Each row carries a **Restart** button that stops and starts that source alone, within a couple of seconds; every other source keeps recording, and nothing in flight elsewhere is lost. It is the right remedy for one camera that has wedged in a way the supervisor has not yet called stalled. Restarts you ask for are deliberately not counted towards the *flapping* verdict — that number is there to spot a source failing on its own. The same action is `POST /api/v2/control/restart-source` for automation, and `GET /api/v2/system/capture` reads the state back; both are in the [API reference](../reference/api.md). Note that a per-source restart **re-launches the source with the settings the service started with** — it does not pick up an edit, which still needs a service restart.

## Tuning a source

Open a source's **edit** panel to change:

- **Label** and **device**,
- **Quiet window** — an optional per-source pause in the **station's local time**, each end a clock time (`22:00`) or a solar anchor (`sunset`, `sunset+30`, `sunrise-15`), e.g. to silence a noisy road-facing mic during rush hour without touching the others. (Earlier releases evaluated it in UTC; if you set the hours to compensate for that, set them back to the local hours you actually want. The source row shows the window with a `local` suffix so you can tell which convention a station is on.)
- **Pipeline toggles** — high-pass filter, DC-offset removal, auto-gain control, RTSP keepalive.
- **Equaliser** — a filter chain of your own, when the toggles are not the right shape. See below.

**Sample rate** (48 or 44.1 kHz) and, for camera sources, **RTSP transport** (auto / TCP / UDP — auto resolves to the NAT-robust TCP default; force UDP only for a camera that needs it) are chosen when the source is **added**. Per-source **gain**, the **channel pick** (mono / left / right / stereo — see [Stereo microphones](#stereo-microphones-and-the-channels-setting) below) and **bit depth** are stored with the source and shown in its detail line, but are **not yet editable from the UI**; capture honours whatever the `audio_sources` row holds.

> Per-source settings (device, gain, sample rate, transport, quiet window) are read when the capture subsystem starts, so **restart the service after changing them** for the change to take effect.

> Aim for peaks near −6 dB. Gain set too high clips the loudest calls and hurts identification more than a quiet signal does.

## The equaliser

The **high-pass** and **DC offset** toggles are two fixed filters: a corner at
120 Hz and one at 5 Hz. That is a reasonable compromise for a garden and the
wrong answer at plenty of sites. A station beside a motorway wants a steeper
cut than one filter section gives. A station with mains hum wants a *notch* at
50 or 60 Hz — a high-pass cannot remove hum without also removing everything
below it, including the low end of a grouse, a bittern or an owl.

The **Equaliser** field takes a chain of filter stages. Type one and the
response curve under the box redraws as you type, computed from the same
coefficients that will filter your audio — so the picture cannot disagree with
what you will hear.

### Writing a chain

One stage per `;`. Each stage is:

```
kind : frequency [ : q [ : gain [ : passes ] ] ]
```

| Kind | What it does | Uses gain |
|------|--------------|-----------|
| `highpass` | Passes above the corner, cuts below | no |
| `lowpass` | Passes below the corner, cuts above | no |
| `bandpass` | Keeps a band, cuts either side | no |
| `notch` | Cuts a narrow band, leaves the rest | no |
| `peaking` | A bell: boost or cut around a centre | yes |
| `lowshelf` | Boost or cut everything below a corner | yes |
| `highshelf` | Boost or cut everything above a corner | yes |

- **frequency** is in hertz, and must be below half your source's sample rate.
  A 48 kHz source can filter up to 24 kHz; a 16 kHz source only to 8 kHz. The
  form refuses a chain the source cannot carry rather than accepting it and
  silently ignoring it later.
- **q** is the width. `0.707` (the default) is the gentlest useful shape. Higher
  is narrower: `20` is a hum notch, `1` is a broad tone control.
- **gain** is in decibels, positive to boost, negative to cut. Ignored by the
  kinds that do not use it.
- **passes** repeats the stage, doubling its slope each time. `highpass:120:0.707:0:2`
  is twice as steep as `highpass:120`.

### Worked examples

| Chain | For |
|-------|-----|
| `highpass:120` | The default high-pass, written out |
| `highpass:120:0.707:0:2` | Twice as steep — a windy or roadside site |
| `notch:50:20` | Mains hum, Europe/Asia (use `notch:60:20` in North America) |
| `notch:50:20; notch:100:20` | Hum and its first harmonic |
| `highpass:200; lowshelf:400:0.7:-6` | Heavy traffic rumble |
| `peaking:4000:1:4` | Lift the band where most songbirds sit |

### What a chain replaces

A non-empty chain **replaces** the high-pass and DC-offset toggles. It does not
stack on top of them, so a chain with your own 120 Hz corner gives you one
filter, not two. Automatic gain control is unaffected either way: it is a
dynamic-range process, not a filter.

Clear the field to go back to the toggles.

### One thing worth knowing about the toggles

The two backends do not implement the **high-pass** toggle identically. A local
microphone gets a one-pole filter (6 dB/octave); an RTSP camera gets ffmpeg's
two-pole one (12 dB/octave). From the same tick-box, a microphone therefore
keeps considerably more low-frequency energy:

| | 20 Hz | 30 Hz | 50 Hz | 60 Hz | 80 Hz | 120 Hz |
|---|---|---|---|---|---|---|
| Microphone | −15.7 dB | −12.3 dB | −8.3 dB | −7.0 dB | −5.1 dB | −3.0 dB |
| RTSP camera | −31.1 dB | −24.1 dB | −15.3 dB | −12.3 dB | −7.8 dB | −3.0 dB |

This is left as it is because changing either filter would change what every
existing station of that kind records. Writing an explicit chain is how you get
the two to agree — a chain is rendered for both backends from the one
specification, so a microphone and a camera given the same chain are filtered
the same way.

> Like the other per-source settings, the chain is read when capture starts —
> **restart the service** after changing it.

## Stereo microphones and the Channels setting

The BirdNET model takes **one** channel, so a stereo microphone's two channels
have to become one before inference. On `mono` — the default — that reduction is
a plain average of the two.

For two capsules in the same spot that is harmless. For two capsules a few
centimetres apart it is a **comb filter**: the same wavefront reaches them at
slightly different times, and averaging then cancels every frequency where the
two arrive out of phase. Measured through this project's own decode path, a half
period of delay costs about **66 dB** — the signal essentially disappears — while
a quarter period costs 3 dB and a full period costs nothing. Which case a given
station sits in depends on the capsule spacing *and* on where the bird is, so it
cannot be answered from a datasheet.

There is a command that answers it on your hardware, in your acoustics:

```bash
sudo systemctl stop birdnet-behavior     # an ALSA capture device is exclusive
birdnet-behavior --channel-report        # add --channel-report-secs 15 for a steadier read
sudo systemctl start birdnet-behavior
```

It records a few seconds, then prints each channel's level, the inter-channel
delay and the capsule spacing that implies, and what **Mono / Left / Right**
would each hand the model. Run it while birds are actually singing: the
cancellation is direction-dependent, so ambient noise alone can look benign on a
microphone that loses badly on real song.

Act on the answer by setting the source's channel pick — `left` or `right`
takes one capsule intact instead of averaging two. There is not yet a UI
control for it: set the `channels` column of the source's `audio_sources` row
(`mono` / `left` / `right` / `stereo`) directly and restart the service.

> This matters most for exactly the deployments that can least afford it: a
> sealed enclosure nobody opens for a season. A station losing 66 dB to its own
> downmix looks completely healthy from every gauge, dashboard and alert this
> project has.

## Adding an RTSP camera

The **Add a source** wizard walks through three steps — URL (with a locked `rtsp://` prefix), optional auth, and a label — and shows a live reachability pill with the sniffed audio properties before you commit. A preview row lets you listen for ten seconds first.

> RTSP capture needs **`ffmpeg`** on the host. The bare-metal installer installs it automatically when your config has an `RTSP_URL` (re-run `sudo bash install.sh repair` if you add RTSP later), and the Docker image already bundles it. Without `ffmpeg`, RTSP sources record zero detections.

## Finding a USB mic

```bash
arecord -l
# card 1: PRO [Comica_Traxshot PRO], device 0: USB Audio [USB Audio]
#      ^ index                ^ id
arecord -D plughw:CARD=PRO,DEV=0 -d 3 /tmp/test.wav && aplay /tmp/test.wav
```

Set the device in `.env` (`BIRDNET_ALSA_DEVICE=…`) or via the CLI flag — see [Configuration](../getting-started/configuration.md).

## Name the card, don't number it

A card **index** (the `1` in `plughw:1,0`) is assigned in detection order and is
not stable. It changes when USB devices are re-enumerated, and a reboot is free
to do that.

This is not hypothetical. During an on-device acceptance run, the same
microphone on the same Raspberry Pi 4 was `card 1: PRO` before a cold reboot and
`card 3: PRO` afterwards. The station came back up, served a perfectly healthy
dashboard, and recorded nothing — the capture supervisor retried a device that
no longer existed, indefinitely. Nothing was broken except a number.

The **id** (`PRO` above) does not move. Address the card by it:

```dotenv
ALSA_CARD=plughw:CARD=PRO,DEV=0
```

`CARD` is a first-class ALSA argument, not a trick: alsa-lib's own `alsa.conf`
declares `pcm.plughw { @args [ CARD DEV SUBDEV ] }` with `@args.CARD { type
string }`. A fresh install now writes this form automatically when it can, and
falls back to the index only when the id would be ambiguous — two identical
microphones report the same id, and then only the index can tell them apart.

After editing the config, apply it with `sudo bash install.sh repair`.

A station still addressing its card by index is told so twice. `--doctor`
grades a resolving index as an advisory naming the exact `CARD=` form for the
card it resolved to (an absent index was already a warning). And at every
start the station writes the id it finds behind each index-form device into its
boot journal and compares it with the last start: if `plughw:1,0` was `PRO`
then and is something else now, that is the `audio_card_moved` boot anomaly,
which reaches `/api/v2/health`, the station-health notification, and the log
as an error, because the station is recording from a different device than
it was set up with.

### Pinning your own names (recommended for multi-mic stations)

The id comes from the device's own firmware, so two identical microphones give
you two cards called `PRO`. [`usb-audio-mapper`](https://github.com/tomtom215/usb-audio-mapper)
solves this properly: it writes a udev rule keyed on vendor/product **and USB
port path**, setting `ATTR{id}` to a name you choose.

```bash
sudo ./usb-audio-mapper.sh          # interactive; reboot when it asks
arecord -l                          # now shows your chosen name as the id
```

Then set `ALSA_CARD=plughw:CARD=<your-name>,DEV=0`. The name survives reboots,
re-plugging, and swapping identical devices between ports — which is the only
way to keep several microphones straight on one station.

`--doctor` validates either form and, if the configured card is missing, names
the card that *is* present and prints the exact line to set.

## Listening live

The **Recordings → Live** view (`/recordings?view=live`; the old `/listen` redirects there) plays whatever the station is recording *right now*.
It does not open the microphone itself — an ALSA capture device only allows one
opener, so anything that tried would be refused with `Device or resource busy`
for as long as recording was in progress, which on a working station is always.
Instead, capture publishes the audio it is already writing to disk, and the live
stream is a second reader of that.

Two things follow from this that are worth knowing:

- **Live audio is exactly what the detector hears**, including any per-source
  gain you have set. If the live stream sounds clipped, so does the audio being
  classified.
- **Live audio follows capture.** If a source is paused — outside the recording
  schedule, or inside its quiet window — or is down, the Live tab reports that
  the source is not recording rather than playing silence. Station Health will
  say why.

The source picker on the Live view chooses what *you* hear. What everyone else
hears when they arrive without choosing — and what `/api/v2/stream` serves with
no `?source_id=` — is the station's **listen default**, set with the **Make
listen default** button on a source's row on this page. Unset, the first
working source serves, which is the whole story on a one-microphone station.
If the chosen source is later disabled or removed, the stream falls back to the
first working source rather than going silent; the journal says so when it does.

Live audio still needs `ffmpeg` installed: the station uses it to encode the
stream as MP3 for the browser. `--doctor` warns if it is missing.

## If you cannot hear the high notes

Age-related hearing loss takes the top of the range first, and a great deal of
warbler, kinglet and treecreeper song lives above 8 kHz — where a station can
hear it perfectly well and its owner cannot.

The **pitch** control beside the Listen button on `/recordings` shifts the live
audio **down**, into a band that still works. Choose one of the downward
presets, and your choice is remembered in that browser: hearing is a property
of a person, so two people listening to the same station at the same time each
get their own setting. (There is also one upward option, which is what you want
for bat calls rather than for hearing loss.)

Changing the pitch reconnects the stream — the shift is applied by the encoder
on the station, not in your browser — so expect about a second of silence.

For **saved clips** the equivalent is `--freq-shift-hz` (config key
`FREQ_SHIFT`), applied when the clip is written. The same rule applies:
**negative** shifts down and is the direction that helps.

> Releases before this one documented that setting backwards, saying a positive
> value helped high-frequency hearing loss. It does the opposite. If you set a
> positive value on that advice, negate it.

## Common pitfalls

- USB hubs can drop audio under load — prefer a direct port on the Pi.
- RTSP over UDP drops packets on busy Wi-Fi — leave the per-source **transport** on `auto` (TCP) for stability; only force `udp` for a camera that requires it.
