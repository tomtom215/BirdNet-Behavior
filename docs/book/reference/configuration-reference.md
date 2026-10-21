# Full Configuration Reference

This page mirrors the project's two canonical sources of truth **verbatim**:

- the documented [`.env.example`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/.env.example) template — every environment variable, and
- the binary's `--help` output — every CLI flag.

The `.env.example` block below is embedded straight from the repository at build time, so it can't drift from the code. For *how* settings layer together (a CLI flag or its `BIRDNET_*` variable > the Settings page > `birdnet.conf` > the built-in default) and the web-UI-only settings, see [Configuration](../getting-started/configuration.md).

`.env.example` names every setting by its **environment variable** (`BIRDNET_…`). The same settings have a different name in `birdnet.conf`, and a station configured by file wants that name, so the next section lists the file keys that neither block below spells out.

## `birdnet.conf` keys by their file names

The file keys are the ones `birdnet-behavior --doctor` knows (`crates/birdnet-core/src/config/known_keys.rs`); a key outside that list is reported at startup with the nearest real name. For most of them the environment variable is `BIRDNET_` plus the key, and the flag is the lower-case, hyphenated key; where it is not, the table says so. Defaults and meanings are the flag's own, from the CLI reference below. The settings that are only in `.env.example` or the [Configuration](../getting-started/configuration.md) table are not repeated here.

| File key | Flag · environment variable | Default | What it does |
|---|---|---|---|
| `PIPEWIRE_DEVICE` | `--pipewire-device` · `BIRDNET_PIPEWIRE_DEVICE` | — | PipeWire or PulseAudio source (`default` for the system default). Takes precedence over `ALSA_CARD` when both are set |
| `SEGMENT_DURATION` | `--segment-duration` · `BIRDNET_SEGMENT_DURATION` | `15` | Seconds per recording segment, from 3 to 600 (`--doctor` checks the config file against that; the Settings page and `PUT /api/v2/settings` refuse a value outside it, and a fraction, a unit or a negative number). BirdNET-Pi's `RECORDING_LENGTH` is read as this value when `SEGMENT_DURATION` is absent or blank (an explicit `SEGMENT_DURATION` wins), so a `birdnet.conf` carried over from BirdNET-Pi keeps its chunk length |
| `NIGHT_INHIBIT` | `--night-inhibit` · `BIRDNET_NIGHT_INHIBIT` | off | Do not record at night; needs `LATITUDE` and `LONGITUDE` |
| `PRE_SUNRISE_OFFSET` | `--pre-sunrise-offset` · `BIRDNET_PRE_SUNRISE_OFFSET` | unset: `TWILIGHT_OFFSET` | Minutes before sunrise at which a `solar` schedule starts recording |
| `POST_SUNSET_OFFSET` | `--post-sunset-offset` · `BIRDNET_POST_SUNSET_OFFSET` | unset: `TWILIGHT_OFFSET` | Minutes after sunset at which a `solar` schedule stops (`TWILIGHT_OFFSET` itself defaults to `30`) |
| `AUDIOFMT` | `--audio-format` · `BIRDNET_AUDIO_FORMAT` | `wav` | BirdNET-Pi's name for the clip format (`wav`, `mp3`, `flac`, `ogg`); `AUDIO_FORMAT` is the other accepted spelling, and `AUDIOFMT` wins if both are set. Non-WAV needs ffmpeg or sox |
| `CLIP_TARGET_LUFS` | `BIRDNET_CLIP_TARGET_LUFS` (no flag) | off | Normalise saved clips to this integrated loudness, e.g. `-18` (−40 to −6; a value outside that is ignored, not clamped); see [Audio & Microphones](../admin/audio.md#evening-out-the-level-of-saved-clips) |
| `CLIP_PEAK_CEILING_DBFS` | `BIRDNET_CLIP_PEAK_CEILING_DBFS` (no flag) | `-1` | The sample peak a normalised clip may not pass, in dBFS (−12 to 0) |
| `CUSTOM_IMAGE_DIR` | `--custom-image-dir` · `BIRDNET_CUSTOM_IMAGE_DIR` | — | A folder of your own species pictures, one file per species, named `{lowercase_scientific_name_with_underscores}` plus `.jpg`, `.jpeg`, `.png` or `.webp` (the code tries them in that order), checked before the image cache. It replaces a species' picture; it is not a single dashboard image |
| `INFO_SITE` | `--info-site` · `BIRDNET_INFO_SITE` | `ebird` | Species info links: `ebird`, `allaboutbirds` or `none` |
| `SITENAME` | `--site-name` · `BIRDNET_SITENAME` | `BirdNet-Behavior` | The name in page titles and the header |
| `STATION_NAME` | — (the Station Name field on the Settings page) | — | The station's own name: the Home Assistant discovery name when `--site-name` is not given, and the default location name in the eBird export |
| `LANG` | — | — | Range-checked as a language code but **not** read as the language: the language key is `DATABASE_LANG` (`--lang`, `BIRDNET_LANG`; default `en`) |
| `MODEL` | — | — | BirdNET-Pi's model name; it is only echoed in the start-up log. The model that runs is `MODEL_PATH` |
| `LOG_LEVEL` | `--log-level` · `BIRDNET_LOG_LEVEL` | the built-in filter | `trace`, `debug`, `info`, `warn`, `error` or `off`; `RUST_LOG` overrides it |
| `LOG_MODULES` | `--log-modules` · `BIRDNET_LOG_MODULES` | — | Per-subsystem levels, e.g. `audio=debug,web=warn` (subsystems: `audio`, `detection`, `web`, `db`, `integrations`, `analytics`) |
| `WEEKLY_REPORT_SCHEDULE` | `--weekly-report-schedule` · `BIRDNET_WEEKLY_REPORT_SCHEDULE` | `monday` | The weekday the weekly report is sent through Apprise, or `disabled` |
| `APPRISE_TRIGGER` | `--notify-trigger` · `BIRDNET_NOTIFY_TRIGGER` | `each` | `each`, `new-species` (fewer than 5 detections this week), `new-species-daily` (first of the day) or `off`; a comma-separated list sends one message when any of them matches (BirdNET-Pi's three `APPRISE_NOTIFY_*` switches) |
| `FULL_DISK` | — (the *When the disk is full* field on the Settings page) | `purge` | `purge` or `keep`: at `DISK_PURGE_THRESHOLD`, delete the oldest clips, or stop recording and delete nothing. A held recording resumes by itself once usage is 2 points below the threshold (below 93 % for 95 %), or after a restart with a higher threshold. Only the recordings directory takes `keep`: if `--watch-dir` is the recordings directory it is ignored, and the start-up log says so |
| `APPRISE_WATCHLIST` | `--notify-species-only` · `BIRDNET_NOTIFY_SPECIES_ONLY` | — | Notify only for these species (comma-separated scientific names) |
| `APPRISE_WATCHLIST_EXCLUDE` | `--notify-species-exclude` · `BIRDNET_NOTIFY_SPECIES_EXCLUDE` | — | Never notify for these species |
| `APPRISE_COOLDOWN` | — | `300` | Seconds before the same species can notify again |
| `MQTT_PORT` | `--mqtt-port` · `BIRDNET_MQTT_PORT` | `1883`, or `8883` with `MQTT_TLS` | Broker port |
| `MQTT_TLS` | `--mqtt-tls` · `BIRDNET_MQTT_TLS` | off | Connect over TLS; the certificate is always verified, with no way to skip it |
| `MQTT_CA_FILE` | `--mqtt-ca-file` · `BIRDNET_MQTT_CA_FILE` | — | PEM file of extra certificates to trust, for a broker behind a private or self-signed CA |
| `MQTT_CLIENT_CERT` | `--mqtt-client-cert` · `BIRDNET_MQTT_CLIENT_CERT` | — | PEM file with this station's client certificate, for a broker that requires mutual TLS (`require_certificate true`). Needs `MQTT_CLIENT_KEY`; implies `MQTT_TLS`. `--doctor` checks the files without connecting |
| `MQTT_CLIENT_KEY` | `--mqtt-client-key` · `BIRDNET_MQTT_CLIENT_KEY` | — | PEM file with the private key for `MQTT_CLIENT_CERT` (PKCS#8, PKCS#1 or SEC1). Never quoted in an error |
| `MQTT_TLS_SERVER_NAME` | `--mqtt-tls-server-name` · `BIRDNET_MQTT_TLS_SERVER_NAME` | the broker host | The name to check the broker's certificate against, for a broker reached by IP |
| `BIRDWEATHER_THRESHOLD` | — (the *BirdWeather upload threshold* field on the Settings page) | — | A confidence (0 to 1) a detection must reach to be **uploaded to BirdWeather**, on top of the station's own threshold; the station's log keeps everything it recorded. Unset or `0` uploads every detection the station records. A value above 1 (75 typed for 75 %) is refused, not clamped: the bar is then ignored (every detection is uploaded) and `--doctor` and the start-up log say so. It never affects the blur setting |
| `BIRDWEATHER_LOCATION_ACCURACY` | — (the *BirdWeather location blur* field) | `0` (exact) | Metres, 0 to 50 000: every post carries the station's position moved by one **fixed** offset (a distance up to this, in a direction, drawn once from a secret seed kept in `birdweather-blur-seed` beside the database), so the public map does not show the station's address. The offset is the same on every post and after every restart, which is what stops someone averaging many posts back to the true spot; it is not hidden from anyone who learns it (the seed file, or another source that places the station), and changing the radius or losing the seed file draws a new one. A value that is set and cannot be used (`5km`, `60000`, a negative number) **turns BirdWeather uploads off** rather than posting the exact position; `--doctor` and the start-up log say so |
| `EMBED_ORIGINS` | — (the *Sites that may embed this station* field) | — (nobody) | Comma-separated origins (`http://homeassistant.local:8123`, `https://abcd1234.ui.nabu.casa`, `https://*.example.com`) allowed to show the station in an `<iframe>`. Anything else, `*` included, and a wildcard over a suffix strangers register under (`*.ui.nabu.casa`, `*.github.io`, `*.co.uk`) is refused and then **no** site may embed. Only `/`, `/kiosk` and `/recordings` can be framed. See [Embedding](../admin/remote-access.md#embedding-the-station-in-another-page) |
| `MQTT_USERNAME` | `--mqtt-username` · `BIRDNET_MQTT_USERNAME` | — | Broker user name |
| `MQTT_PASSWORD` | `--mqtt-password` · `BIRDNET_MQTT_PASSWORD`, or `BIRDNET_MQTT_PASSWORD_FILE` | — | Broker password. Prefer the file form: a flag is visible in `ps` |
| `MQTT_TOPIC_PREFIX` | `--mqtt-topic-prefix` · `BIRDNET_MQTT_TOPIC_PREFIX` | `birdnet` | Detections go to `{prefix}/detection/{species}`, the heartbeat to `{prefix}/status` |
| `TLS_CERT` | `--tls-cert` · `BIRDNET_TLS_CERT` | — | Certificate chain (PEM) for `TLS_MODE=manual` |
| `TLS_KEY` | `--tls-key` · `BIRDNET_TLS_KEY` | — | Private key (PEM) for `TLS_MODE=manual` |
| `TLS_DIR` | `--tls-dir` · `BIRDNET_TLS_DIR` | a `tls` folder beside the database | Where the generated CA and server certificate are kept |
| `PRIVACY_RANK` | `--privacy-rank` · `BIRDNET_PRIVACY_RANK` | `10` | BirdNET-Pi's human-sound rule; `0` = off, up to 12000. On by default. A field on the Settings page (*Privacy Rank*) |
| `WATCHDOG_CHECK_SECS` | `BIRDNET_WATCHDOG_CHECK_SECS` | `2` (1–300) | How often every capture source is checked |
| `WATCHDOG_STALL_SEGMENTS` | `BIRDNET_WATCHDOG_STALL_SEGMENTS` | `4` (2–60) | Segments of missing output before a running source is called stalled |
| `WATCHDOG_STALL_FLOOR_SECS` | `BIRDNET_WATCHDOG_STALL_FLOOR_SECS` | `120` (10–3600) | Floor under that stall threshold |
| `WATCHDOG_BACKOFF_BASE_SECS` | `BIRDNET_WATCHDOG_BACKOFF_BASE_SECS` | `2` (1–60) | First retry delay after a source dies |
| `WATCHDOG_BACKOFF_CAP_SECS` | `BIRDNET_WATCHDOG_BACKOFF_CAP_SECS` | `60` (2–3600) | Ceiling on the retry delay; there is no retry limit |
| `WATCHDOG_DOWN_WARN_AFTER_SECS` | `BIRDNET_WATCHDOG_DOWN_WARN_AFTER_SECS` | `120` (10–86400) | How long a source may be down before the loud journal warning |
| `WATCHDOG_DOWN_WARN_EVERY_SECS` | `BIRDNET_WATCHDOG_DOWN_WARN_EVERY_SECS` | `300` (30–86400) | How often that warning repeats |

A watchdog value outside its range is clamped and the adjustment is logged; one that is not a number falls back to the default. The ranges and defaults are in `src/capture/watchdog.rs`.

`--offsite-backup` accepts `off`, `s3`, `sftp` **and `rsync`**: the CLI help below says "`off`, `s3` or `sftp`", which predates the fourth. See [Backups & Recovery](../admin/backups.md#over-rsync-when-the-link-is-bad).

## Environment variables — `.env.example`

```dotenv
{{#include ../../../.env.example}}
```

## CLI flags — `birdnet-behavior --help`

```text
{{#include ../_generated/cli-help.txt}}
```

> **Keeping this page current:** the `.env.example` block updates itself on every docs build. Regenerate the CLI block after changing any flag with:
>
> ```bash
> ./scripts/gen-cli-help.sh
> ```
>
> CI fails if the committed `cli-help.txt` is stale, so this can't silently drift.
