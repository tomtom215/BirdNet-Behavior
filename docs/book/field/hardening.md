# Security Hardening Guide

BirdNet-Behavior is designed to be safe by default on a trusted home network,
but if you are deploying it somewhere more exposed — a shared flat, a research
site with multiple tenants, a campus network, or anywhere reachable from the
internet — this guide collects the knobs that matter.

For the project's threat model and trust boundaries, see
[`docs/architecture/12-risks.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/docs/architecture/12-risks.md).
To report a vulnerability, see
[`SECURITY.md`](https://github.com/tomtom215/BirdNet-Behavior/blob/main/SECURITY.md).

> **TL;DR for an exposed deployment:** keep `CADDY_PWD` set (a fresh install
> generates one automatically), restrict the bind to loopback (or a VPN) if you
> don't need LAN access, put a reverse proxy with TLS in front for anything
> off-LAN, leave CORS at its same-origin default, and verify release artifacts
> before installing.

---

## 1. Network exposure

The single most important decision is *what can reach the web UI*. **Viewing the
dashboard requires no login; the `/admin` panel** — which can change settings,
trigger database backups, and update the software — **is gated by a
session-cookie sign-in enforced by the binary itself.** Treat reachability as
the primary control — and when the station is reachable from beyond the LAN,
turn on [private mode](#private-mode-everything-behind-the-sign-in), which
puts the dashboard itself behind that sign-in.

- **Default: all interfaces.** A bare-metal binary defaults to
  `--listen 0.0.0.0:8502`, so the dashboard is reachable from other devices on
  the LAN out of the box. The installer auto-generates an admin password
  (`CADDY_PWD`) on a fresh install, so `/admin` is protected by default; the
  open dashboard exposes only read-only views.
- **Restrict to this host:** set `BIRDNET_LISTEN=127.0.0.1:8502` (env, the
  config file, or answer "restrict to this device" in the interactive
  installer) — then reach it remotely with an SSH tunnel
  (`ssh -L 8502:localhost:8502 pi@host`) or a VPN.
- **Startup guard.** When the server binds to a non-loopback address *and* no
  `CADDY_PWD` is configured (e.g. you cleared it), it logs a prominent warning
  at startup. If you see

  ```
  WARN admin web UI is bound to a non-loopback address with NO authentication …
  ```

  in the journal, either set `CADDY_PWD` (below) or bind to `127.0.0.1`.
- **Encrypt the LAN traffic.** `--tls-mode self-signed` brings HTTPS up on 8503
  and generates a local CA to import once; `--tls-mode manual` serves your own
  certificate and reloads it when your ACME client renews it. Off by default.
  `--doctor` verifies the whole setup before startup does.
- **Private mode for anything reachable from outside.** A tunnel, a mesh VPN
  shared with other people, or a port forward makes "anyone who can reach the
  port" a much larger set than your LAN, and `--listen 127.0.0.1` does not
  help: the tunnel is what connects to it. Set `BIRDNET_PRIVATE_MODE=true`
  (config file: `PRIVATE_MODE=true`) and the whole station — dashboard, read
  API, live audio, both WebSockets — needs the sign-in. See
  [Private mode](#private-mode-everything-behind-the-sign-in) below.
- **Never port-forward `8502`/`8503` to the internet.** Built-in HTTPS encrypts
  the traffic; it does not add a login lockout or a WAF, and a self-signed
  certificate carries no publicly-trusted name. Put it behind a reverse proxy
  (Caddy/nginx) that terminates HTTPS with a real certificate and adds
  authentication, or use a mesh VPN (Tailscale/WireGuard). See
  [Remote Access & Security](../admin/remote-access.md) for proxy configs.

---

## 2. Authentication

Authentication gates **the `/admin*` panel, the Station management tabs
(`/station/capture|alerts|data|settings|access`) and every page action that
changes something** — viewing the dashboard, the read-only `/api/v2/*`
endpoints, the WebSockets, and the health check are open to anyone who can
reach the port, unless [private mode](#private-mode-everything-behind-the-sign-in)
is on. A fresh install auto-generates a strong admin
password, so `/admin` is protected by default; for anything LAN- or
internet-reachable, keep it set (and add TLS off-LAN).

- **Built-in admin sign-in.** A fresh install sets `CADDY_PWD` automatically
  and prints it once in the post-install summary, storing it in
  `/etc/birdnet/birdnet.conf`. Sign in as **`admin`** — the account the
  dashboard seeds. `/admin*` without a session redirects to a `/login` form
  that issues a session cookie; this is not HTTP Basic Auth, so `curl -u` does
  not apply. Change the password any time via `CADDY_PWD` in the config or the
  environment:

  ```dotenv
  CADDY_PWD=a-long-random-password
  ```

  `CADDY_USER` is consulted from the **process environment only** (Docker), not
  from `birdnet.conf` — the systemd unit sets no `EnvironmentFile`, so on a
  bare-metal install the sign-in name stays `admin`.

  A signed-in session survives a restart of the station: the signing secret
  is generated once and kept beside the database as `session.secret` (mode
  0600), unless `BNB_SESSION_SECRET` is set, which takes precedence. Rotating
  `CADDY_PWD` in the config signs every session out at the next start;
  rotating on the accounts page signs every *other* session out.

  The form is throttled per client address: five failed attempts inside
  fifteen minutes and that address is answered `429 Too Many Requests` (with a
  `Retry-After`) until its oldest failure is a quarter-hour old — the password
  is not even checked, so a guessing script costs the Pi a map lookup rather
  than an Argon2 hash per guess. Each refused attempt is in the audit log as
  `auth.login.throttled`; a successful sign-in clears the address, and a
  restart forgives everything. Behind a reverse proxy the address is the
  visitor's only if the proxy is trusted (`--trusted-proxies`); otherwise every
  visitor shares the proxy's bucket, which is the conservative failure.

  This is compatible with the BirdNET-Pi `CADDY_PWD` convention. The password
  crosses the wire in clear text unless TLS is on — turn on `--tls-mode`, put a
  proxy in front, or keep the station on a trusted LAN. **Clearing `CADDY_PWD`
  leaves `/admin` open** — and with it every state-changing action on the
  dashboard (delete a detection, relabel it, set a review verdict, approve or
  delete a quarantined record, save the onboarding wizard) — to anyone who can
  reach it. Reading stays open either way.
- **Reverse-proxy auth** (recommended for internet exposure): terminate TLS and
  require a password at the proxy (Caddy `basic_auth`, nginx `auth_basic`), so
  credentials never cross the wire in clear text.
- **WebSocket caveat.** The live-detection WebSocket (`/api/v2/ws/detections`)
  and the health endpoint (`/api/v2/health`) are intentionally exempt from the
  built-in sign-in layer, because a browser cannot attach credentials to a
  `WebSocket` handshake. (They are read-only and outside the gated router in any
  case.) The live detection stream is therefore readable by anyone who can reach
  the port. If that matters, gate access at the network layer (VPN / proxy
  allow-list) rather than relying on app-level auth.

### Private mode: everything behind the sign-in

```dotenv
BIRDNET_PRIVATE_MODE=true
BIRDNET_PUBLIC_ACCESS=share,metrics   # optional carve-outs
```

(`PRIVATE_MODE=true` and `PUBLIC_ACCESS=…` in `birdnet.conf`; `--private-mode`
and `--public-access` on the command line.)

With private mode on, a visitor with no session gets nothing but the sign-in
form: pages answer a `303` to `/login`, the API and the WebSockets a `401`.
What stays open without a session is exactly:

| Always open | Why |
|---|---|
| `/login`, `/logout`, `/static/*`, `/favicon.ico` | a browser has to be able to render the sign-in form |
| `/api/v2/health` | the systemd watchdog and the container healthcheck read it |

plus whatever `BIRDNET_PUBLIC_ACCESS` names, comma-separated:

| Carve-out | Opens |
|---|---|
| `live_audio` | `/stream` and the live spectrogram WebSocket — for a station whose feed is meant to be listened to |
| `share` | the signed `/r/<token>` links the **Share clip** button mints, with their audio and spectrogram; *not* the recordings route behind them |
| `metrics` | `/api/v2/metrics`, for a Prometheus scraper that has no cookie |

Anything else — the detection history, the recordings, the feeds, the
detection WebSocket — is behind the sign-in with no way to open it. Viewer
accounts (the `/station/access` tab) can see everything a signed-in admin
can, and change nothing, so a private station can still be shared with the
household.

**Private mode needs a password.** A private station with no `CADDY_PWD`
answers `503` to everything but the sign-in and the health probe — it does
*not* fall back to the open station, because that is the one thing it was
asked not to be. The startup log says so at `ERROR`, the page says so, and
`--doctor` reports it under **Private mode**. An unknown name in
`BIRDNET_PUBLIC_ACCESS` is reported and skipped (the station starts, with
that surface closed); `--doctor` reports it too.

---

## 3. Cross-origin requests (CORS)

By default the API allows **no cross-origin reads** — it emits no
`Access-Control-Allow-Origin` header. The station's own UI is served from the
same origin it calls, so this is all most deployments need, and it prevents a
malicious website you visit from reading your station's API over the LAN.

If you front the API from a *different* origin (a separate dashboard host),
allow it explicitly:

```dotenv
# comma-separated list of allowed origins
BIRDNET_CORS_ALLOWED_ORIGINS=https://dashboard.example.com,https://lab.example.com
```

State-changing requests are additionally protected by a stateless CSRF guard
regardless of this setting.

---

## 4. Privacy

The station listens to a live microphone, so audio handling is privacy-relevant.

- **Human-voice filter.** Set `BIRDNET_PRIVACY_THRESHOLD` (0.0–1.0; `0.02` is a
  usual start, and lower suppresses more) to suppress analysis windows in which
  the model's confidence for a human class reaches the value, and their
  neighbours. `0.0` disables it. It binds independently of the detection
  threshold.
- **Recording retention.** Extracted detection clips accumulate on disk; cap
  them with `BIRDNET_MAX_FILES_PER_SPECIES` and rely on the disk manager's
  purge threshold. Audio you never want persisted should be filtered at the
  source.
- **What is stored:** detection rows (species, time, confidence, location if
  configured), short extracted audio clips, and spectrogram images. No raw
  continuous audio is retained beyond the rolling capture buffer.

---

### Credentials as mounted files

Every outbound credential — the notification URLs (whose bot token is *inside*
the URL), the BirdWeather token, the MQTT password, the heartbeat URL whose path
is the secret — can be supplied as the contents of a file rather than as an
environment variable, using the `BIRDNET_<KEY>_FILE` convention Docker and
Kubernetes both use. That keeps it out of `docker inspect`, out of
`/proc/<pid>/environ`, and out of the settings table (and so out of every backup
taken from the database). See
[Notifications](../admin/notifications.md#keeping-the-credential-out-of-the-environment)
for the list and the compose snippet.

## 5. A bad configuration edit cannot take the station down

The daemon validates its configuration at startup. An invalid setting (a
latitude outside ±90, a malformed `RECORDING_SCHEDULE`, an unsupported
`AUDIO_FORMAT`) used to make it refuse to start, and since validation ran in
the new process after systemd had stopped the old one, a typo made over SSH
became a restart loop with no web UI and no way back. Now every successful
start keeps a copy of the file it ran on as `birdnet.conf.last-good`; a start
whose file has errors runs on that copy and reports `config_reverted` on
`/api/v2/health` and the station page; a start with errors and no copy runs
web-only on the file as it is and reports `config_rejected`, so the diagnostics
are reachable and show the errors.

Change the file the safe way, which validates before anything is installed:

```bash
sudo cp /etc/birdnet/birdnet.conf /tmp/birdnet.conf && sudo nano /tmp/birdnet.conf
sudo birdnet-behavior --apply-config /tmp/birdnet.conf   # refuses a file with errors
birdnet-behavior --doctor                                # the same checks, on demand
birdnet-behavior --doctor-json                           # exit code: 0 ok, 1 warn, 2 error
```

The systemd unit still runs `--doctor` as an `ExecStartPre` gate for the
journal's sake; a configuration error is reported there as a warning naming
what the start will do, so the gate lets it happen.

---

## 6. Data & backups

- **Database backups.** Take a hot backup from **Station → Data** (`/station/data#backups`) or
  `birdnet-behavior --backup-db`; the periodic maintenance task also rotates
  backups beside the database. Those are on the same card as the database, so
  they cover a corrupt page and not a dead card.
- **Get a copy off the device automatically.** Set `OFFSITE_BACKUP=s3` or
  `OFFSITE_BACKUP=sftp` and each weekly snapshot is encrypted on the station —
  argon2id + ChaCha20-Poly1305, not optional — and uploaded. The passphrase is
  the only thing that opens it, is stored nowhere, and is not recoverable:
  **write it down somewhere that is not this station.**

  Give the station its own credentials, scoped to the one prefix it writes to.
  For S3 that is `PutObject`, `ListBucket` and `DeleteObject`, nothing more; the
  station never reads a backup back. For SFTP it is a dedicated key on a
  dedicated account, and host key checking that cannot be turned off — `yes`, or
  `accept-new` for the first connection on a network you control.

  Secrets have no command-line flags on purpose: an argument is visible in `ps`
  to every user on the box and is copied into the journal by `ExecStart=`. Put
  them in the config file (mode `0600`, owned by the service user) or the unit's
  environment. `--doctor` reports what is missing, and checks the SSH key's mode
  — OpenSSH refuses a key others can read, and says so only on its own stderr.

  See [Backups & Recovery](../admin/backups.md#offsite-backups).
- **Integrity & recovery.** On startup the database is integrity-checked; a
  corrupt database that cannot be recovered from a backup is quarantined aside
  and a fresh one is started so the station keeps recording (the quarantined
  file is preserved for offline recovery). Run `--check-db` to verify on demand.
- **Restore** from the Backups page or by stopping the service and replacing the
  database file with a known-good copy.

---

## 7. Verify what you install

Every release ships signed provenance and a bill of materials:

- **SLSA build provenance** — verify an archive came from this repository's
  release workflow:

  ```bash
  gh attestation verify --repo tomtom215/BirdNet-Behavior \
    birdnet-behavior-<version>-aarch64-unknown-linux-gnu.tar.gz
  ```

- **Checksums** — `sha256sum -c SHA256SUMS --ignore-missing`.
- **SBOM** — a CycloneDX 1.5 SBOM (JSON + XML) is attached to each release for
  dependency/vulnerability auditing.

Prefer the one-line installer or Docker image from the official repository, and
pin a specific version tag in production rather than `latest`.

---

## 8. Host hardening (what the unit already ships)

The installer also writes a journald drop-in,
`/etc/systemd/journald.conf.d/birdnet-behavior.conf`, with
`Storage=persistent` and `SystemMaxUse=200M`. On a default Raspberry Pi OS
the journal is volatile, so a power cut or a watchdog restart erased the
evidence of what caused it; persistent and bounded, it keeps more than a
year of the station's logging in 200 MB (the two per-file INFO lines that
were most of the volume are DEBUG now) and cannot fill the card. The setting
is host-wide, as journald's configuration is; `install.sh uninstall` removes
the drop-in and leaves the journal itself.

The installed systemd unit runs as a non-root user, gates startup on the
doctor, and already carries the hardening a drop-in would usually add. From
`install.sh`'s unit template:

```ini
[Service]
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=<data dir> <config dir> /run /var/log
PrivateTmp=yes
ProtectKernelLogs=yes
ProtectKernelModules=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
RestrictRealtime=yes
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
NoNewPrivileges=yes
CapabilityBoundingSet=
UMask=0027
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 AF_NETLINK
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @mount @debug @cpu-emulation @obsolete @reboot @swap @raw-io @clock @module
DevicePolicy=closed
SupplementaryGroups=audio
```

Do not paste an older "recommended drop-in" over this. The data directory lives
under the service user's home (`~/BirdNet-Behavior`), which is why the unit
relies on `ProtectHome=read-only` plus its own `ReadWritePaths=` line to reach
it — a drop-in setting `ProtectHome=true` or `ReadWritePaths=/var/lib/birdnet-behavior`
names a stricter mode and a directory the installer never creates. If you
tighten further with `systemctl edit birdnet-behavior`, verify the service
still starts and can reach its audio device and data directory afterwards. Pair with a host firewall (`ufw`/`nftables`) that
only opens the ports you actually use.

---

## Checklist for an exposed deployment

- [ ] Restrict the bind to `127.0.0.1` (+ SSH/VPN) if you don't need LAN access; for off-LAN access, put a TLS reverse proxy in front.
- [ ] Turn on `--tls-mode self-signed` if the dashboard is reachable by anyone else on the network.
- [ ] Keep `CADDY_PWD` set (a fresh install generates one) — or use proxy/VPN auth. Don't clear it on a non-loopback bind.
- [ ] Turn on `BIRDNET_PRIVATE_MODE` if the station is reachable from beyond your own LAN (a tunnel, a port forward, a VPN other people are on), and carve out only what you mean to publish with `BIRDNET_PUBLIC_ACCESS`.
- [ ] Set `OFFSITE_BACKUP` (with `OFFSITE_PASSPHRASE`) so a dead SD card is not the end of the records — and store the passphrase somewhere other than the station.
- [ ] Leave CORS at its same-origin default unless you genuinely need a second origin.
- [ ] Set `BIRDNET_PRIVACY_THRESHOLD` if voices may be captured.
- [ ] Back up the database **off the device** and test a restore.
- [ ] Verify release provenance/checksums before installing; pin a version.
- [ ] Consider the systemd sandboxing drop-in and a host firewall.
