# First Steps

After starting (via the installer or Docker), open the web dashboard:

```text
http://<your-ip>:8502
```

> Not sure of your Pi's IP? Run `hostname -I` on the Pi, or check your router's device list.

The dashboard binds to all interfaces by default, so it's reachable from any device on your LAN — no extra step. Viewing needs no login; only the **Admin** panel does. The bare-metal installer prints an auto-generated admin password once in its post-install summary (username `birdnet`) — log in with that when you open `/admin`. Lost it? See [Remote Access & Security](../admin/remote-access.md#built-in-http-basic-auth) to read or reset `CADDY_PWD`. To restrict the dashboard to the Pi itself, set `BIRDNET_LISTEN=127.0.0.1:8502`.

If you set your latitude/longitude and an audio source, detections start appearing within a minute or two of the first bird call — no further configuration required.

## A two-minute tour

The UI has six homes — the tabs along the top (and the phone bottom bar):

1. **Today** (`/`) — the home and right-now view: live signal, the day strip, and the full searchable detection log. See [Today](../guide/today.md).
2. **Species** (`/species`) — browse every species you've recorded as a list, a photo grid or your growing life list. See [Species & the Life List](../guide/species.md).
3. **Patterns** (`/patterns`) — the analytics: when birds are active, the dawn chorus, migration, who sings together, trends. See [Patterns](../guide/patterns.md).
4. **Recordings** (`/recordings`) — play back captured clips and listen to live audio.
5. **Reports** (`/reports`) — the weekly recap, year in review and history calendar. See [Reports](../guide/reports.md).
6. **Station** (`/station`) — health at a glance, plus all settings and admin. Start with **Settings** (`/admin/settings`) for optional fine-tuning: confidence threshold, per-species overrides, email, quarantine rules.

## First-run wizard

If you'd like a guided setup, open **`/onboarding`** for a five-step wizard — location, microphone, and alert preferences — that gets a new station listening in about ninety seconds.

![The first-run onboarding wizard](../images/onboarding.png)

## Health check

At any time you can run the built-in diagnostic, which prints a one-screen report covering CPU, configuration, audio reachability, the model file, database integrity, disk, and network — each problem with a concrete suggested fix:

```bash
# Bare metal
sudo -u birdnet birdnet-behavior --doctor

# Docker
docker compose exec birdnet birdnet-behavior --doctor
```

See [Troubleshooting](../guides/troubleshooting.md) if anything looks off.
