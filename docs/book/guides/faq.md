# FAQ

## Is it accurate? Which model does it use?

It uses the **BirdNET+ V3.0** model — the same neural network as upstream BirdNET, so identification accuracy matches BirdNET-Pi. The model (~541 MB) downloads automatically on first run — sha256-verified, from the same GitHub release line as the binary, falling back to Zenodo (the upstream source) if that asset is unavailable.

## Do I need a Raspberry Pi?

No. It runs on any x86_64 Linux machine too. The Pi 5 is recommended; the Pi 4B/400 are fully supported, and the 3B+ works on the **64-bit** Pi OS (tight on RAM — see [Hardware](../getting-started/hardware.md)). There's no Python interpreter or virtualenv overhead around the model, so it's happy on modest hardware.

## Does it work offline / without internet?

Yes, once set up. The only things that need the network are the **one-time model download**, optional **Wikipedia photo** caching, and any **notification/upload** channels you enable. The UI itself is fully self-contained — all fonts are embedded in the binary and it never calls out to a CDN.

## Does it need a camera, or just a microphone?

Either. Point it at a USB microphone, or at an RTSP stream from an IP camera (many bird-box and feeder cams expose one). You can run several RTSP sources at once. See [Audio & Microphones](../admin/audio.md).

## Where are my recordings and database stored?

Under `/data` in Docker (a named volume) or under the configured data directory on bare metal: the SQLite `birdnet.db`, the `recordings/` clips, the cached `model/`, and the Wikipedia image `cache/`.

## How do I keep a recording I love from being deleted?

Retention is disk-based, not time-based — the oldest clips are purged once the disk fills past the threshold. **Lock** any detection (on the Today page) to pin its recording permanently. See [Backups & Recovery](../admin/backups.md).

## Is the dashboard exposed on my network? Do I need a login?

By default the dashboard binds to all interfaces (`0.0.0.0:8502`), so it's reachable from any device on your LAN. **Viewing needs no login; only the `/admin` panel is password-protected.** A fresh bare-metal install auto-generates that password for you. To restrict the dashboard to the local machine, set `BIRDNET_LISTEN=127.0.0.1:8502` and reach it via SSH tunnel or VPN. Don't port-forward it to the internet — put a reverse proxy with TLS or a VPN in front. See [Remote Access & Security](../admin/remote-access.md).

## How do I find or reset the admin password?

The bare-metal installer auto-generates a strong admin password (username `birdnet`) and prints it **once** in the post-install summary, storing it as `CADDY_PWD` in `/etc/birdnet/birdnet.conf`. To set your own, edit that file (or the `CADDY_PWD` environment variable / `.env` under Docker) and restart:

```bash
sudo nano /etc/birdnet/birdnet.conf   # set CADDY_PWD=your-new-password
sudo systemctl restart birdnet-behavior
```

Sign in as `admin` — that is the account the dashboard seeds. `CADDY_USER` is read from the process environment only, so setting it in `birdnet.conf` does not rename it (it does work under Docker). Clearing `CADDY_PWD` leaves `/admin` open to anyone who can reach the dashboard.

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
