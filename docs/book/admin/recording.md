# Recording & Retention

How audio is captured, segmented, and eventually purged.

## Capture & segmentation

Incoming audio is split into fixed-length segments before inference. Two settings shape this:

| Setting | Env var / flag | Default | What it does |
|---|---|---|---|
| Segment duration | `BIRDNET_SEGMENT_DURATION` / `--segment-duration` | `15` | Length (seconds) of each recorded segment. |
| Overlap | `BIRDNET_OVERLAP` / `--overlap` | `0.0` | Seconds of overlap between consecutive analysis chunks. A small overlap (e.g. `1.5`) helps catch calls that straddle a chunk boundary, at some CPU cost. |

## Recording schedule

By default the station listens **all day**. To record only during chosen windows, set a schedule:

```dotenv
BIRDNET_RECORDING_SCHEDULE=all-day      # default
```

The [scheduler](../reference/architecture.md) computes sunrise and sunset from your latitude/longitude, so schedules can be anchored to solar events (e.g. the dawn-chorus window) rather than fixed clock times. This is also what powers the day/night cues in the dashboard and the kiosk night mode.

> Set your **location** accurately — sunrise/sunset, the species-frequency filter, and any solar-anchored schedule all depend on it.

## Privacy threshold

`BIRDNET_PRIVACY_THRESHOLD` (default `0.0`, disabled) discards every analysis window in which the model's confidence for a **human** class (speech, whistling, other human sounds) reaches the value, and the windows either side of it, so casual conversation near the mic isn't written to disk. The confidence is read from the model's output before the detection threshold applies, so the setting binds on its own: changing the detection threshold does not change what the privacy filter does. Lower values suppress more. `0.02` is a usual starting point; *lower* it (towards `0.01`) if the microphone is near a patio or path, and raise it if birdsong is being suppressed.

## Retention — how clips are purged

Retention is **disk-based by default**, with an optional age limit on top:

- **Keep Clip Audio (days)** (`BIRDNET_CLIP_RETENTION_DAYS`, `--clip-retention-days`, default `0` = keep for ever) reclaims the audio of detections older than N days.
- The disk manager purges the **oldest** recordings once the disk crosses `DISK_PURGE_THRESHOLD` (default **95%**, in `birdnet.conf`).
- At most `BIRDNET_MAX_FILES_PER_SPECIES` (`--max-files-per-species`, default `0` = unlimited) clips are kept per species.
- **Locked** clips are *never* purged — lock anything you want to keep permanently from the [Today](../guide/today.md) page or a species' recordings.

The detection rows in the database are kept regardless; purging only removes the audio files, not the history. The [storage breakdown](./backups.md) on the Backups page shows how much space recordings are using.
