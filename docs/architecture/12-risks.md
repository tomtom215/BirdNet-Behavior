# Risk Assessment

> Known risks, severity ratings, and mitigation strategies.

## Table of Contents

- [Risk Matrix](#risk-matrix)
- [Trust Boundaries](#trust-boundaries)
- [Dependency Risks](#dependency-risks)
- [Operational Risks](#operational-risks)
- [Outstanding Issues](#outstanding-issues)

---

## Risk Matrix

| Risk | Severity | Likelihood | Mitigation |
|------|----------|-----------|------------|
| Mel spectrogram divergence from librosa | Medium | Low | Pure-Rust implementation validated against librosa reference spectrograms within 1e-4 tolerance; covered by unit tests |
| ONNX model accuracy loss | Medium | Low | End-to-end tests run a real WAV fixture through the full pipeline and assert expected species at expected confidence |
| DuckDB ARM64 performance under load | Medium | Medium | Analytics feature is optional; operational queries stay on SQLite; DuckDB queries are synced from SQLite and tuned for columnar access |
| Cross-compilation drift between release and Docker images | Low | Low | Release binaries are cross-compiled with Ubuntu 24.04's GCC 13 aarch64 toolchain; Docker images build natively per architecture on matching runners |
| RTSP / audio capture subprocess instability | Low | Low | Supervised `ffmpeg` / `arecord` subprocesses with restart logic, gap detection, and disk monitoring |

## Trust Boundaries

What follows is the code as it stands on 2026-11-25, each statement with the
lines it was read from. It describes who can reach what without signing in;
it is not a promise about later releases.

### Who can reach the station

- **Every interface, by default.** `--listen` defaults to `0.0.0.0:8502`
  (`src/cli.rs:100`); the installer's default is the same
  (`installer/lib/10-config.sh:36`) and it writes it into the unit's
  `ExecStart` (`installer/lib/65-service.sh:138`). The interactive installer
  deliberately does not offer to bind to loopback
  (`installer/lib/70-station.sh:395-401`). The working boundary is therefore
  "anyone who can reach port 8502": the LAN, and whatever a port forward,
  tunnel or shared VPN adds to it.
- **TLS is off by default**; `--tls-mode` turns it on
  (`docs/book/field/hardening.md`, "Encrypt the LAN traffic"). Without it a
  password typed into `/login` crosses the network in clear.

### What is open without signing in

The router is three parts (`crates/birdnet-web/src/server.rs:173-191`): the
public router, the admin router behind the cookie gate, and the bearer-token
write API.

- **The public router is open on a station that is not in private mode.**
  `apply_public` wraps it in a layer that passes every request through unless
  private mode is on (`crates/birdnet-web/src/auth_middleware.rs:164-167`).
  It holds (`crates/birdnet-web/src/routes/mod.rs:76-109`):
  - every dashboard page and the read API, including the detection history
    and its exports (`routes/export/mod.rs:38-50`: CSV/JSON, eBird,
    `BirdDB.txt`, Raven);
  - **recorded clips**, `GET /api/v2/recordings/{filename}`
    (`routes/recordings.rs:27-28`), and their spectrograms,
    `GET /api/v2/spectrogram/{filename}` (`routes/spectrogram/mod.rs:52`);
  - **live audio**: `GET /stream` (`routes/livestream.rs:137`) and the HLS
    variant, whose `start` and `stop` are `POST`s on the same router
    (`routes/hls.rs:267-271`);
  - **the live spectrogram** and **live detections** WebSockets,
    `/api/v2/ws/spectrogram` (`routes/spectrogram_ws.rs:91`) and
    `/api/v2/ws/detections` (`routes/websocket.rs:94`);
  - the health probe, `/api/v2/health` (`routes/system.rs:15`).

  A stored clip is subject to the human-sound rule when it is on
  (`docs/book/admin/recording.md`); live audio and the live spectrogram are
  not: nothing in `routes/livestream.rs`, `routes/hls.rs` or
  `routes/spectrogram_ws.rs` filters for speech.
- **The admin gate's exclusion list is not what opens the sockets.**
  `is_excluded` names `/api/v2/health` and every `/api/v2/ws/*`
  (`auth_middleware.rs:500-502`), but those paths are mounted on the public
  router, so the admin gate never sees them.
- **The admin router** (`/admin/*` and every state-changing page route,
  `server.rs:173-174`) needs a `bnb-session` cookie
  (`auth_middleware.rs:269-334`). A viewer account may read but not write, and
  may not read the routes that export credentials or the whole database
  (`auth_middleware.rs:336-428`).
- **The write API** (`/api/v2` mutations) needs a bearer token and answers
  `404` on a station with no `BNB_API_TOKEN` (`server.rs:175-181`,
  `crates/birdnet-web/src/api_token.rs:173`). It never inherits the admin
  gate's open bypass.

### The open-admin bypass

- **A station with no admin password opens the admin router to everyone.**
  `admin_password_configured` is false when `CADDY_PWD` is not in the
  process environment and the seed admin row still carries its placeholder
  hash (`auth_middleware.rs:518-528`); the gate then attaches a synthetic
  admin identity to the request (`auth_middleware.rs:308-327`). Updates,
  backups, restores, the support bundle and the audio sources are all behind
  that one door. The bare-metal installer generates a password on a fresh
  install (`installer/lib/70-station.sh:251-258`); a Docker run without
  `CADDY_PWD` has none until the first-run wizard sets one.
- **The bypass has a host-name rule** against DNS rebinding: it is granted only
  to a request whose `Host` is an IP literal, `localhost`, a single-label
  name, a name under `.local`, `.lan`, `.home`, `.home.arpa`, `.internal`,
  `.localdomain` or `.localhost`, or a name listed in `BIRDNET_ALLOWED_HOSTS`
  (`crates/birdnet-web/src/open_admin_host.rs:32-42`, `:86-100`). Any other
  name gets a `403` page asking for a password first (`:122-140`). A request
  with no `Host` at all is allowed (`:87-89`).
- **Cross-site writes** are refused by the CSRF guard, which compares `Origin`
  with `Host` (`crates/birdnet-web/src/security.rs:362`); the host-name rule
  exists because a rebound name makes those two agree
  (`open_admin_host.rs:8-17`).

### Private mode

- With `PRIVATE_MODE` on, every public path outside a short list needs the
  same session the admin router needs: pages are redirected to `/login`, the
  API and the WebSockets answer `401` (`auth_middleware.rs:164-201`,
  `:232-250`). Always open: `/api/v2/health`, `/login`, `/logout`,
  `/favicon.ico` and `/static/*`; opened only on request: live audio
  (`/stream`, HLS and the spectrogram socket), signed share links, and the
  metrics (`crates/birdnet-web/src/private_mode.rs:121-137`).
- A private station with no password fails closed: it answers `503` rather
  than falling back to the open bypass (`auth_middleware.rs:188-194`,
  `:300-306`).

### Signing in

- **The login throttle** counts a failure per client address: five inside
  fifteen minutes and that address is refused
  (`crates/birdnet-web/src/login_throttle.rs:21-38`). A trusted proxy that
  vouches for many clients is held to thirty failures, summed
  (`:25-35`). **Limits:** there is no cap across addresses, so a host with
  many addresses gets five guesses for each; the counts live in memory
  (`crates/birdnet-web/src/state.rs:245`) and start again at every restart.
- **Rate limiting** is per IP, 30 requests a second with a burst of 60
  (`server.rs:88-98`).
- **Passwords** are stored as Argon2id hashes in the `users` table; the
  session-signing secret is a file beside the database, mode 0600
  (`crates/birdnet-web/src/session.rs:26`, `:130-142`).

### What is stored in plain text

- **`CADDY_PWD`**, the admin password the installer generates, is written into
  `/etc/birdnet/birdnet.conf`, owned `root:<service user>` with mode 0640
  (`installer/lib/62-config-file.sh:94-95`, `:284-285`), and read from there
  at every start (`src/helpers/auth.rs:56-61`). The installer also prints it
  once in its summary.
- **The offsite secrets**, `OFFSITE_PASSPHRASE` and `OFFSITE_S3_SECRET_KEY`,
  are read from the environment or the same file, never from a flag
  (`src/helpers/offsite.rs:62`, `:92-102`).
- **Credentials saved in the browser**, such as the BirdWeather token and the
  SMTP password, are rows of the `settings` table in the database
  (`src/helpers/settings_overlay.rs:256`, `:450`), so they are in every
  snapshot and in the full backup, which is an unencrypted `.tar.gz`
  (`crates/birdnet-web/src/routes/admin/system_controls/backup.rs:215-246`).
  The offsite copy of a snapshot is encrypted before it leaves.

### Planned, not built

Decisions 5 and 6 of `docs/audit-2026-11/decisions-2026-11-25.md` change two
of the statements above: a new install will need a sign-in for live audio and
the recorded-clip routes (an existing station keeps what it has), and a new
install will get a single-use setup token with the admin routes closed while
it is pending, instead of a station that is open until a password is set.
Neither is built at the time of writing; until they are, the sections above
are what runs.

## Dependency Risks

| Dependency | Risk | Mitigation |
|-----------|------|------------|
| `ort` | ONNX Runtime version churn and large native dependency | Pinned version; release images statically link ONNX Runtime so deployed binaries have no runtime ONNX Runtime dependency |
| `symphonia`, `rubato`, `realfft` | Pure Rust, actively maintained | Low risk |
| `rusqlite` (bundled) | Wraps SQLite C; very stable | Low risk |
| `duckdb` | C++ build cost and version coupling | Optional `analytics` feature; native per-architecture Docker builds avoid emulation; `Cargo.lock` pinned |
| `axum` / `tokio` | Active, well-maintained ecosystem | Low risk |
| `lettre` | Pure Rust SMTP with rustls | Low risk |
| `sysinfo` | API churn between minor versions | Pinned to 0.40; workspace manifest enables the `system`, `component` and `network` features explicitly |

## Operational Risks

| Risk | Mitigation |
|------|------------|
| Disk exhaustion on long-running stations | Disk manager with automatic rotation, per-species file caps, and disk-usage thresholds |
| Database corruption from power loss | WAL mode, integrity checks at startup, auto-recovery from the most recent backup |
| Memory exhaustion on 1 GB Pi 4 | Target <50 MB RSS (a design target — nothing in-tree gates it — and not met with a model loaded: the comment above `MemoryHigh` in `installer/lib/65-service.sh` records ~575 MB idle and ~650 MB analysing with the 541 MB V3.0 model; the default V2.4 model is unmeasured); bounded broadcast channels (detections 256, spectrogram 16, log lines 512); no unbounded accumulation |
| Network outage blocking BirdWeather uploads | Exponential backoff with offline buffering |
| Model file corruption or missing weights | Loader refuses to start with an invalid model; the Docker entrypoint re-downloads the model on first run |
| Migration corrupts target database | Each import batch runs inside its own transaction (`insert_batch_tagged`), so a failure leaves earlier batches committed and the re-run is idempotent via `ON CONFLICT … DO NOTHING`; source is opened read-only |
| Directory traversal on backup download | Canonical path check + filename allow-list |
| Unauthorised web access | Session-cookie sign-in on `/admin*` with argon2id password hashing; a bearer token gates the write API; per-IP token-bucket rate limiter on API and admin routes |

## Outstanding Issues

These are known open items tracked for future releases:

1. **FP16 model handling on ARM** — BirdNET V2.4 ships FP16 weights. ONNX
   Runtime's ARM backend converts to FP32 at load, but end-to-end
   accuracy on FP16 models should be spot-checked on a real Pi.
2. **Large dataset performance** — Stations running for years may
   accumulate millions of detections. Operational queries are bounded
   (paginated, time-windowed); DuckDB sync throughput at scale still
   warrants a dedicated benchmark suite.
3. **Concurrent database contention under load** — WAL mode allows
   concurrent readers plus one writer. High-frequency detection events
   combined with web requests should be load-tested to confirm no lock
   contention emerges at the upper end of realistic station throughput.

---

[← Migration](11-migration.md) | [Back to Index](../RUST_ARCHITECTURE_PLAN.md) | [Next: Implementation Status →](13-implementation-status.md)
