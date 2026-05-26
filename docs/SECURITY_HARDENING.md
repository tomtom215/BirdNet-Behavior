# Security Hardening Guide

BirdNet-Behavior is designed to be safe by default on a trusted home network,
but if you are deploying it somewhere more exposed — a shared flat, a research
site with multiple tenants, a campus network, or anywhere reachable from the
internet — this guide collects the knobs that matter.

For the project's threat model and trust boundaries, see
[`architecture/12-risks.md`](architecture/12-risks.md). To report a
vulnerability, see [`../SECURITY.md`](../SECURITY.md).

> **TL;DR for an exposed deployment:** bind to loopback (or a VPN), put a
> reverse proxy with TLS + auth in front, set `CADDY_PWD`, leave CORS at its
> same-origin default, and verify release artifacts before installing.

---

## 1. Network exposure

The single most important decision is *what can reach the web UI*. The admin
panel can change settings, trigger database backups, and update the software,
so treat reachability as the primary control.

- **Default (safest): loopback only.** A bare-metal binary defaults to
  `--listen 127.0.0.1:8502` — reachable only from the machine itself. Reach it
  remotely with an SSH tunnel (`ssh -L 8502:localhost:8502 pi@host`) or a VPN.
- **LAN access:** set `BIRDNET_LISTEN=0.0.0.0:8502` (the installer and Docker
  default to this for convenience). This exposes the UI to **everyone on the
  network**. Fine on a trusted home LAN; pair it with authentication anywhere
  else.
- **Startup guard.** When the server binds to a non-loopback address *and* no
  password is configured, it logs a prominent warning at startup. If you see

  ```
  WARN admin web UI is bound to a non-loopback address with NO authentication …
  ```

  in the journal, either set `CADDY_PWD` (below) or bind to `127.0.0.1`.
- **Never port-forward `8502` to the internet.** The built-in server speaks
  plain HTTP with no TLS. Put it behind a reverse proxy (Caddy/nginx) that
  terminates HTTPS and adds authentication, or use a mesh VPN
  (Tailscale/WireGuard). See
  [Remote Access & Security](book/admin/remote-access.md) for proxy configs.

---

## 2. Authentication

The UI ships with **no authentication** unless you enable it — appropriate for a
loopback-only or VPN-only deployment, but not for anything LAN- or
internet-reachable.

- **Built-in HTTP Basic Auth.** Set `CADDY_PWD` (and optionally `CADDY_USER`,
  which defaults to `birdnet`) in the config or environment:

  ```dotenv
  CADDY_USER=birder
  CADDY_PWD=a-long-random-password
  ```

  This is compatible with the BirdNET-Pi `CADDY_PWD` convention. It is **still
  plain HTTP** — only rely on it behind TLS or on a trusted LAN.
- **Reverse-proxy auth** (recommended for internet exposure): terminate TLS and
  require a password at the proxy (Caddy `basic_auth`, nginx `auth_basic`), so
  credentials never cross the wire in clear text.
- **WebSocket caveat.** The live-detection WebSocket (`/api/v2/ws/detections`)
  and the health endpoint (`/api/v2/health`) are intentionally exempt from the
  built-in Basic Auth layer, because browsers cannot attach Basic-auth headers
  to a `WebSocket` handshake. The live detection stream is therefore readable by
  anyone who can reach the port. If that matters, gate access at the network
  layer (VPN / proxy allow-list) rather than relying on app-level auth.

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
  good start) to suppress analysis windows that contain human speech. `0.0`
  disables it.
- **Recording retention.** Extracted detection clips accumulate on disk; cap
  them with `BIRDNET_MAX_FILES_PER_SPECIES` and rely on the disk manager's
  purge threshold. Audio you never want persisted should be filtered at the
  source.
- **What is stored:** detection rows (species, time, confidence, location if
  configured), short extracted audio clips, and spectrogram images. No raw
  continuous audio is retained beyond the rolling capture buffer.

---

## 5. Fail-fast configuration

The daemon validates its configuration at startup and **refuses to start** on an
invalid setting (e.g. a latitude outside ±90, a malformed `RECORDING_SCHEDULE`,
or an unsupported `AUDIO_FORMAT`) rather than running in a silently-degraded
state. Run the bundled diagnostic before deploying a config change:

```bash
birdnet-behavior --doctor          # human-readable preflight
birdnet-behavior --doctor-json     # machine-readable (exit code: 0 ok, 1 warn, 2 error)
```

The systemd unit runs `--doctor` as an `ExecStartPre` gate, so a broken config
fails fast with an actionable journal entry instead of a restart loop.

---

## 6. Data & backups

- **Database backups.** Take a hot backup from **Admin → Backups** or
  `birdnet-behavior --backup-db`; the periodic maintenance task also rotates
  backups beside the database. Copy them **off the device** — an SD card that
  fails takes its local backups with it.
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

## 8. Host hardening (recommended drop-ins)

The installed systemd unit runs as a non-root user and gates startup on the
doctor. For an exposed or multi-tenant host, consider tightening it further with
a drop-in (`systemctl edit birdnet-behavior`):

```ini
[Service]
# Filesystem & kernel isolation (adjust paths the service must write to).
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/birdnet-behavior
PrivateTmp=true
NoNewPrivileges=true
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
```

Verify the service still starts and can reach its audio device and data
directory after adding these. Pair with a host firewall (`ufw`/`nftables`) that
only opens the ports you actually use.

---

## Checklist for an exposed deployment

- [ ] Bind to `127.0.0.1` (+ SSH/VPN) or put a TLS reverse proxy in front.
- [ ] Set `CADDY_PWD` (and `CADDY_USER`) — or proxy/VPN auth.
- [ ] Leave CORS at its same-origin default unless you genuinely need a second origin.
- [ ] Set `BIRDNET_PRIVACY_THRESHOLD` if voices may be captured.
- [ ] Back up the database **off the device** and test a restore.
- [ ] Verify release provenance/checksums before installing; pin a version.
- [ ] Consider the systemd sandboxing drop-in and a host firewall.
