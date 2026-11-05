# Input-level fixtures

What `crates/birdnet-core/tests/input_level_readings.rs` parses, and where each file came from.
**None of these is output from a sound card**: the machine they were gathered on has none.

| File | Origin |
|---|---|
| `pactl_list_sources_pulseaudio_16.1.json` | **Captured.** `pactl --format=json list sources` (pactl 16.1) against a PulseAudio 16.1 server with null sinks and sources, 2026-11-05. Virtual sources: no hardware volume control. |
| `pactl_list_sources_pipewire_pulse_1.0.5.json` | **Captured.** The same command against `pipewire-pulse` 1.0.5, same day. Virtual sources. |
| `pactl_info_pulseaudio_16.1.txt` | **Captured.** `pactl info` against the same PulseAudio 16.1 server. |
| `amixer_scontents_emulated_pulse_plugin.txt` | **Captured.** `amixer -D pulse scontents` (alsa-utils 1.2.9) through the ALSA `pulse` plugin: real `amixer` formatting over an *emulated* mixer (range 0 to 65536, no dB), not a hardware driver. |
| `amixer_scontents_published_usb_codec.txt` | **Published, copied verbatim.** A USB audio codec on Ubuntu, in a Digirig forum post by a user, 2023-04-12 (`forum.digirig.net/t/unable-to-adjust-volume-on-linux/1467`). The post also prints `dBminmax-min=0.00dB,max=23.81dB` for `Mic Capture Volume`. Community source. |
| `amixer_scontents_published_intel_hda.txt` | **Published, copied verbatim.** An Intel HDA codec in Ubuntu bug 495134, comment 25, 2010-03-29. Community source. |

A reading of a *hardware-controlled* PulseAudio source (the `HW_VOLUME_CTRL` flag, a base volume below 0 dB) has no
captured sample. The tests build one by changing the captured JSON in the one place that matters, and say so.
