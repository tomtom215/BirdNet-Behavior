# FAQ

## Is it accurate? Which model does it use?

It uses the **BirdNET+ V3.0** model — the same neural network as upstream BirdNET, so identification accuracy matches BirdNET-Pi. The model (~541 MB) downloads automatically from Zenodo on first run.

## Do I need a Raspberry Pi?

No. It runs on any x86_64 Linux machine too. The Pi 5 is recommended; the Pi 4B/400 are fully supported, and the 3B+ works on the **64-bit** Pi OS (tight on RAM — see [Hardware](../getting-started/hardware.md)). The binary is tiny (~20–50 MB of RAM at runtime), so it's happy on modest hardware.

## Does it work offline / without internet?

Yes, once set up. The only things that need the network are the **one-time model download**, optional **Wikipedia photo** caching, and any **notification/upload** channels you enable. The UI itself is fully self-contained — all fonts are embedded in the binary and it never calls out to a CDN.

## Does it need a camera, or just a microphone?

Either. Point it at a USB microphone, or at an RTSP stream from an IP camera (many bird-box and feeder cams expose one). You can run several RTSP sources at once. See [Audio & Microphones](../admin/audio.md).

## Where are my recordings and database stored?

Under `/data` in Docker (a named volume) or under the configured data directory on bare metal: the SQLite `birdnet.db`, the `recordings/` clips, the cached `model/`, and the Wikipedia image `cache/`.

## How do I keep a recording I love from being deleted?

Retention is disk-based, not time-based — the oldest clips are purged once the disk fills past the threshold. **Lock** any detection (on the Today page) to pin its recording permanently. See [Backups & Recovery](../admin/backups.md).

## Do I need anything special for behavioral analytics?

No. The DuckDB engine behind the deeper behavioral views — activity sessions, species retention, next-species prediction, year-on-year trends — is **built into every release and on by default**. The installer runs the service with `--analytics-db` and Docker compose sets `BIRDNET_ANALYTICS_DB`, so there is no separate build, flag, or image to pick. (From source, build with `--features analytics`. On a very low-RAM board you can turn it off — see [Troubleshooting](./troubleshooting.md).)

## Can I use it commercially?

No. BirdNet-Behavior is licensed **CC BY-NC-SA 4.0**, matching upstream BirdNET and BirdNET-Pi — non-commercial use only. See [Credits & License](../about.md).

## How do I update?

- **Docker:** `docker compose pull && docker compose up -d`.
- **Bare metal:** re-run the installer, or use the in-app updater at `/admin/update/check`.
- Your database, recordings and cached model are preserved across updates.

## It's not detecting anything — what now?

Almost always an audio-source problem. Run `birdnet-behavior --doctor`, confirm a source is set and reachable, and check the level meter on [Audio & Microphones](../admin/audio.md). The [Troubleshooting](./troubleshooting.md) guide has step-by-step recipes.
