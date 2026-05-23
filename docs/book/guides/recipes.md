# Common Tasks

Short, copy-pasteable answers to "how do I…?" Each links to the page with the full story.

## Get a phone alert when a rare bird shows up

1. Create an [Apprise](https://github.com/caronc/apprise) URL for your service (Telegram, Pushover, ntfy, Discord…).
2. Set it: `BIRDNET_APPRISE_URL=tgram://bottoken/chatid` (or via `/admin/settings → Notifications`).
3. Set `BIRDNET_NOTIFY_CONFIDENCE=0.85` so only solid detections ping you.
4. Use the rare-bird rules so you're alerted on *new* or *unusual* species, not every robin.

See [Notifications & Integrations](../admin/notifications.md).

## Set up a daily digest instead of per-detection pings

Choose the **Daily digest** notification mode (in the [onboarding wizard](../getting-started/first-steps.md#first-run-wizard) or `/admin/settings → Notifications`) to receive one evening summary rather than a stream of alerts.

## Keep a recording forever

Retention is disk-based — old clips are purged once the disk fills. Open the [Today](../guide/today.md) page (or a species' recordings), and use the **lock** action on the detection. Locked clips are never purged. See [Recording & Retention](../admin/recording.md).

## Fix a misidentified detection

On the [Today](../guide/today.md) page, use **re-label** on the row to correct the species, or **delete** it if it's pure noise.

## Cut down on false positives

Enable the quality pre-filter and nudge the confidence threshold up — see the full [Tuning guide](./tuning.md). For a single noisy species, add a per-species threshold instead of raising the global bar.

## Listen to last night's owl

Go to [Recordings](../guide/recordings.md), browse **by species** (or by date), pick the owl, and play the clip inline. The [History](../guide/reports.md#history-calendar) calendar is the fastest way to jump to a specific night.

## Add a second microphone or an RTSP camera

Use the [Add-a-source wizard](../admin/audio.md#adding-an-rtsp-camera) on the Audio page, or set `BIRDNET_RTSP_URLS=` with comma-separated URLs. You can run several sources at once.

## Expose the dashboard to the internet (safely)

Don't bind it to `0.0.0.0` on a public IP directly. Put it behind a reverse proxy with HTTPS and a password — see [Remote Access & Security](../admin/remote-access.md).

## Send detections to Home Assistant

Set `BIRDNET_MQTT_HOST` and `BIRDNET_MQTT_HA_DISCOVERY=1`. The station registers itself automatically and the latest detection appears as Home Assistant entities. See [Integrations Reference](../reference/integrations.md#home-assistant).

## Back up everything before an upgrade

Take a manual snapshot on the [Backups](../admin/backups.md) page (auto-snapshots also run nightly), then update. Pre-upgrade snapshots are tagged so you can roll back.

## Move my data to a new Pi

Export a **full bundle** from [Backups](../admin/backups.md) on the old Pi, install on the new one, and restore the bundle (its signature is verified first). Your detections, settings, recordings and cached model come across.

## Import my old BirdNET-Pi history

See [Migrating from BirdNET-Pi](./migration.md) — it's a safe, read-only import at `/admin/migrate`.
