# Audit 04: out-of-the-box experience and defaults

Audited 2026-11-22 against `9d8bb1c` (`main`, committed 2026-11-12). Read-only: nothing was
built, run, installed or edited. `bash -n` passes on `install.sh`, `quickstart.sh` and
`uninstall.sh`. `install.sh` is generated from `installer/lib/*.sh` (`installer/build.sh`), so
citations point at `installer/lib/`.

The reader I have in mind is a birder who bought a Raspberry Pi 4 or 5, a USB microphone and an
SD card, and has never opened a terminal. Unless a line says "CONJECTURED", every claim below
comes from a file and line I opened. "Tracked" refs point at `docs/HANDOFF.md` §5,
`docs/TECH_DEBT.md` (TD-*), `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` (ON-*, LC-*, PS-*, O-*, NT-*,
RC-*) and `docs/FEATURE_GAP_ANALYSIS.md` (A-*).

---

## Journey today (bare metal, the path README calls "nothing else needs setting up")

Marks: **[T]** = something the person must type, **[D]** = a decision, **[J]** = jargon on the
path, **[S]** = a place where they can get stuck with nothing saying so.

1. **Get a 64-bit Raspberry Pi OS *Trixie* onto the card.** **[D][J][S]** The product has no
   image to flash. The manual's install page opens with a table: "64-bit Raspberry Pi OS
   **Trixie**", and "glibc ≥ 2.39" (`docs/book/getting-started/installation.md:5-15`). It never
   says how to flash an OS, enable SSH or Wi-Fi, set a hostname or set a time zone: no file under
   `docs/book` mentions Raspberry Pi Imager, pi-gen or an image (grep `flashable|Imager|pi-gen`:
   nothing). The person has to discover Raspberry Pi Imager on their own, work through its
   customisation screen (user, password, Wi-Fi, SSH, locale), flash, and boot. Anyone who
   flashes Bookworm, the OS most online guides still describe, is turned away at step 4
   (`installer/lib/30-platform.sh:227`, `check_glibc`).
2. **Get a terminal on the Pi.** **[T][J]** That means `ssh user@raspberrypi.local` from another
   computer, or a keyboard and monitor. Undocumented.
3. **Paste the one-liner.** **[T][J]**
   `curl --proto '=https' --tlsv1.2 -fsSL …/install.sh | sudo bash` (`README.md:112`). The README
   adds a warning about `sudo bash <(curl …)` (`README.md:115`) that this reader cannot parse.
   The installer refuses to run as root directly (`installer/lib/30-platform.sh:137-140`).
4. **Preflight and downloads.** These run on their own: glibc, tools, `alsa-utils` and
   `qrencode` installed (`installer/lib/45-preflight.sh:48-58`), a disk check, the binary, then
   about 85 MB of model from `huggingface.co` and `raw.githubusercontent.com`
   (`installer/lib/55-model.sh:373-411`). With no internet this step is **fatal**
   (`:403`). **[S]** if the network is down.
5. **Prompt: "Use detected ALSA device 'plughw:CARD=…,DEV=0'?"** **[D][J]**
   (`installer/lib/70-station.sh:209-212`). Only the first card is offered. Choosing another
   means typing an ALSA string, prompted as "ALSA device (e.g. plughw:1,0) or rtsp:// URL"
   (`:219`).
6. **Prompt: latitude, or both coordinates.** **[T][D]** The prompt sends the person to
   OpenStreetMap to right-click and copy a pair (`:253-256`). There is no lookup in the
   installer. Pressing Enter skips it, with a warning that scrolls away (`:260-262`).
7. **Prompt: admin password, or Enter to auto-generate one.** **[D]** (`:297`). The default
   (Enter) produces a random 22-character password (`:150-165`).
8. **Copy that password out of the terminal.** **[S]** The summary prints it once with "Save
   it now — it is not shown again" (`installer/lib/80-summary.sh:40-46`). Getting it back later
   needs `sudo grep '^CADDY_PWD' /etc/birdnet/birdnet.conf`
   (`crates/birdnet-web/templates/login.html:136-141`).
9. **Open the dashboard.** **[T]** The summary prints `http://<ip>:8502`, then
   `http://<hostname>.local:8502` "works on most devices, but not all", and a QR code when
   `qrencode` installed (`installer/lib/80-summary.sh:135-137`, `:10-17`). The person has to
   type a port number. The `.local` name is whatever the Pi happens to be called, usually
   `raspberrypi`.
10. **First screen.** If coordinates were given, there is no wizard redirect
    (`crates/birdnet-web/src/routes/pages/today.rs:425-433`). Today draws the "Getting ready"
    checklist: admin password, microphone, model, disk space, "Listening for the first call…"
    (`today.rs:290-410`). **[S]** if the microphone is muted or its gain is at zero (OOB-05).
    **[S]** if the Pi's time zone is wrong (OOB-06).
11. **First detection.** Arrives within minutes on a correctly levelled, unmuted microphone
    near birds.

**Count.** After the OS is on the card, the shortest run takes **6 required human actions**:
open a terminal, paste the command, answer three prompts (the mic prompt is Enter, coordinates
must be fetched from a map and typed, the password prompt is Enter), and save the password.
Opening the dashboard (type an IP and port, or scan the QR) makes **7**. Getting the OS onto
the card takes another **~6–8** that the product does not document: install Imager, pick the
board, pick the Trixie 64-bit image, fill in the customisation screen (user and password,
Wi-Fi, SSH, time zone), flash, boot. That is about **13–15 actions**, at least four of them
needing a terminal or jargon. There is **no flashable image**, only `curl | sudo bash`.

**Docker path** (`quickstart.sh`, required on Bookworm): install Docker
(`curl -fsSL https://get.docker.com | sudo sh`, `README.md:117`). Fix docker-group membership,
which means logging out and back in (`quickstart.sh:238-245`). Run `bash <(curl … main/quickstart.sh)`;
this is the development branch's script, not a release (`README.md:117`). Then answer: working
directory (`:294`), mic (`:344`), IP auto-detect yes/no (`:398`), coordinates (`:421-428`), and
a time zone if the host's cannot be detected (`:453`). Three to five more steps than bare metal.

**Headless variant** (`BIRDNET_NONINTERACTIVE=1`, or no TTY): no prompts. The mic is
auto-detected and location is left unset (`installer/lib/70-station.sh:198-205`). The first
browser visit goes to `/onboarding` only while there are still zero detections
(`today.rs:425-433`), and a sign-in with the generated password comes first
(`crates/birdnet-web/src/routes/pages/onboarding.rs:105-110`).

---

## Key defaults

| Setting | Default | Source | Verdict |
|---|---|---|---|
| Classifier | BirdNET V2.4 + range filter v2 (~85 MB) | `installer/lib/90-args.sh:30-36`; `docs/book/getting-started/installation.md:31` | **Good.** Matches BirdNET-Pi and birdnet-go, and is small. No licence notice (OOB-19). |
| Confidence threshold | 0.70 | `crates/birdnet-core/src/config.rs:43` | **Good.** BirdNET-Pi's default, kept the same in daemon, form and wizard (`:28-42`). |
| Sensitivity | 1.25 | `config.rs:68` | **Good.** BirdNET-Pi parity. |
| Overlap | 0.0 s | `src/daemon/mod.rs:365-369`; `src/cli.rs:891` | **Judgement.** BirdNET-Pi parity and cheap on a Pi. Still, every 15 s segment has a seam (S-1, tracked), and calls that cross a window are analysed in halves. |
| Species occurrence filter | On only when coordinates *and* the range filter are present; `SF_THRESH` 0.03 | `installer/lib/62-config-file.sh:128-135`; `src/cli.rs:721` | **Right default, weak guard.** A missing location is not shown on the first screen (OOB-02). |
| Human-sound privacy filter | `PRIVACY_RANK=10` (on) | `config.rs:54`; `src/daemon/mod.rs:317-327` | **Good.** BirdNET-Pi's rule. The *live stream* is not covered by it (OOB-13). |
| Recording schedule | `all-day` | `src/cli.rs:614` | **Good.** Owls and night migrants count. The fixed-hours help text is wrong (OOB-04). |
| Disk-full action | purge at 95 %, species floor 5 | `src/helpers/system.rs:24`; `src/cli.rs:950-973` | **Good.** |
| Clip retention by age / per species | 0 (forever) / 0 (unlimited) | `src/cli.rs:932,948` | **Acceptable** given the 95 % purge. |
| Clip format | WAV | `src/cli.rs:925` | **Judgement.** About 576 kB per 6 s clip, and SD-card wear (PS-4, tracked). BirdNET-Pi uses mp3. Archival-quality WAV is defensible. |
| Extraction length | 6 s | `src/daemon/config.rs:478` | Fine. |
| Notification trigger | `each` (CLI, settings form); `new-species` (wizard) | `src/cli.rs:658`; `crates/birdnet-web/src/routes/admin/settings/render/mod.rs:159`; `onboarding.rs:169` | **Inconsistent** (OOB-12). |
| Notify confidence | 0.8 | `src/cli.rs:508` | Fine. |
| Detection deadman | 24 h, delivered only through a configured channel | `src/cli.rs:511-521` | **Long for day one.** Nothing alarms a never-detecting station on the first screen (OOB-05). |
| Listen address | `0.0.0.0:8502` | `src/cli.rs:100`; `installer/lib/10-config.sh:36` | **Good for reach.** Port 8502 must be typed (OOB-09). |
| Viewing / live audio / metrics | open to the LAN; private mode off | `crates/birdnet-web/src/private_mode.rs:1-26` | **Questionable for live audio** (OOB-13). Metrics: O-15, tracked. |
| Admin password | random 22 characters, printed once | `installer/lib/70-station.sh:150-183` | Secure, but hostile to this reader (OOB-07). |
| HTTPS | off | `src/cli.rs:117` | Reasonable on a LAN. A self-signed CA is no help to this reader. |
| Capture sample rate | 48 kHz, or the device's nearest | `src/capture/sources.rs:19`, `:551-567` | **Good.** Probed, not assumed. |
| Mic device id | `plughw:CARD=<id>` (bare metal); `plughw:<index>` (Docker) | `installer/lib/70-station.sh:56-67`; `quickstart.sh:333-340` | **Bare metal good; Docker wrong** (OOB-10). |
| Auto-update | none | `README.md:162`; `crates/birdnet-web/src/routes/admin/update.rs:17-21` | **Gap** (OOB-08). |
| Watchdog / restart | `WatchdogSec=120`, `Restart=always`, backoff 10 s→5 min, no start limit | `installer/lib/65-service.sh:87,145-164` | **Good.** |
| Journal | persistent, 200 MB cap | `65-service.sh:280-290` | Good for diagnosis. Some extra SD writes. |
| Local DB backups | weekly, on the same card; offsite opt-in | `src/maintenance.rs:65`; `README.md:202` | Effective retention is 5 (PS-15, tracked). Config is not backed up (LC-13, tracked). |
| Stream tmpfs | 256 MB mount vs `STREAM_MAX_MB` 512 | `installer/lib/60-dirs.sh:58`; `src/cli.rs:991` | PS-18, tracked. |
| Memory ceiling | `MemoryMax=1G` | `65-service.sh:176-177` | Fine for V2.4. |

---

## Findings

### OOB-01: No flashable image, and no documented path from a blank card to a terminal
- Severity: **high**. Kind: gap. **NEW**. The flashable-image ask is not in A-*, LC-* or §5.
- Evidence: `docs/book/getting-started/installation.md:5-15,68-71` starts at "64-bit Raspberry
  Pi OS Trixie" and a `curl | sudo bash`. `README.md:109-115`. A grep of `docs/` for
  `Imager|pi-gen|flashable|sd image` finds nothing. `docs/book/getting-started/first-steps.md:9`:
  "Run `hostname -I` on the Pi". Status: VERIFIED-BY-READING.
- Impact: the target reader stalls before the product runs at all. Choosing Trixie over
  Bookworm is a hard requirement the reader cannot evaluate. Picking wrong costs a reflash, or
  the Docker path, which is harder.
- Proposed change: (a) now: a "Before you start" page covering Raspberry Pi Imager with the
  exact choices (Pi OS Lite 64-bit Trixie, hostname `birdnet`, Wi-Fi, SSH on, time zone) and
  screenshots, linked first from README Quick start. (b) Then: a pi-gen stage (or an Imager
  `os_list` JSON) that builds a release image. That image would carry the binary, the V2.4
  model and range filter, and a first-boot unit that auto-detects the mic and starts the
  service, so the only step left is opening `http://birdnet.local`. (c) Also consider an
  Imager custom-repository entry, so the image appears inside Imager.
- Acceptance gate: a CI job builds the image and boots it under QEMU or `systemd-nspawn`.
  `/api/v2/health` must answer 200, and `/onboarding` must render with no terminal input.
  Write the counterpart: boot the image with no network and assert that the wizard says the
  model is present (it is baked in).
- Effort: L (image). S (doc page). Depends on the release workflow and on OOB-09 (hostname).

### OOB-02: A station without coordinates says so only in the installer's scrollback and on `/station`
- Severity: **medium**. Kind: footgun. **ALREADY-TRACKED(ON-16)**. This report adds a new
  mechanism: the redirect to the wizard stops at the first detection.
- Evidence: `crates/birdnet-web/src/routes/pages/today.rs:425-433` redirects to `/onboarding`
  only when `!onboarded && !has_detections && !has_location`. A headless install auto-detects
  the mic and starts capture with no location (`installer/lib/70-station.sh:198-205`,
  `installer/lib/75-start.sh:68-76`). The first detection, possibly of a bird from the wrong
  continent, turns the redirect off for good. The first-run checklist has no location row
  (`today.rs:290-410`: admin, mic, model, disk, listening). The only HTML surface is the
  Pipeline cell on `/station`, "off · every species the model knows is a candidate"
  (`station_health.rs:740`). Status: VERIFIED-BY-READING.
- Impact: Grey Plovers in a back garden in Ohio. The reader concludes the model is bad.
- Proposed change: add a "Location" row to `firstrun_checklist`, plus a persistent banner on
  Today whenever `latitude`/`longitude` are unset. The banner should say "Your station does not
  know where it is, so it may name birds that don't live near you. Set your location →" and
  carry a one-click IP look-up. Gate the wizard redirect on `!onboarded && !has_location`
  alone, not on detections.
- Acceptance gate: a test that seeds one detection and no location must find the banner on
  `/`. Seen failing on current code. Counterpart: with a location set, no banner.
- Effort: S.

### OOB-03: Installing over a BirdNET-Pi host adopts BirdNET-Pi's config file and changes its ownership
- Severity: **high** (if the precondition holds). Kind: bug. **NEW**.
- Evidence: the repository's own migration guide says that on BirdNET-Pi
  "`/etc/birdnet/birdnet.conf` is a link to it" (`docs/book/guides/migration.md:21`), which is
  this installer's `CONFIG_FILE` (`installer/lib/10-config.sh:13`). Given that file:
  `resolve_classifier` takes the "kept" branch and, finding no `MODEL_PATH` that matches V2.4,
  **chooses V3.0** (`installer/lib/52-classifier.sh:105-113`).
  `prompt_station_settings` returns before asking anything (`70-station.sh:193`).
  `ensure_admin_password` generates nothing (`:175`). `write_config` keeps the file and runs
  `chown root:${SERVICE_USER}` and `chmod 0640` on it (`62-config-file.sh:88-96`). Both follow
  the symlink, so BirdNET-Pi's own `~/BirdNET-Pi/birdnet.conf` changes owner. The installer
  never checks for `birdnet_recording`/`birdnet_analysis` units (grep of `installer/lib`:
  none), so if both run, both open the same microphone. Status: the code path is
  VERIFIED-BY-READING. The precondition (a BirdNET-Pi install on a Trixie host that passes
  `check_glibc`) is **CONJECTURED**. Probe: Trixie VM, Nachtzuster BirdNET-Pi installed, run
  `install.sh`, then check `stat ~/BirdNET-Pi/birdnet.conf`, the chosen classifier, whether
  `CADDY_PWD` was used, and whether `RECS_DIR=$HOME/BirdSongs` was adopted.
- Impact: the upgrader the README invites ("Migrating from BirdNET-Pi") gets a station on the
  wrong network (541 MB V3.0), with BirdNET-Pi's paths and possibly an open admin panel. Their
  old station's config is also altered.
- Proposed change: `detect_existing_install` should recognise a BirdNET-Pi install (the
  symlink target under `~/BirdNET-Pi`, or `birdnet_*.service`). Then it should refuse with a
  plain message: "BirdNET-Pi is installed here. Stop it (…) and choose: move its settings in
  (we'll import them), or start fresh." Never `chown` or `chmod` through a symlink
  (`chown -h`, or test `-L` first).
- Acceptance gate: `installer/test/` case with a symlinked `/etc/birdnet/birdnet.conf` pointing
  at a BirdNET-Pi-shaped file. Assert the installer stops before `write_config` and that the
  target's mode and owner are unchanged. Run it against the current lib first and watch it
  fail.
- Effort: M.

### OOB-04: The settings page and the generated config say fixed recording hours are UTC; the code uses local time
- Severity: **medium**. Kind: bug. **NEW**. The CHANGELOG records fixing the *doc comments*
  (`CHANGELOG.md:7462-7465`), not these two.
- Evidence: `crates/birdnet-web/src/routes/admin/settings/render/location.rs:56`
  `<option value="fixed:06:00-20:00">Fixed hours, UTC</option>` and `:58` "Fixed hours are
  evaluated in <strong>UTC</strong>, not local time". `installer/lib/62-config-file.sh:218-219`
  "Fixed hours are evaluated in UTC, not local time". Against that:
  `crates/birdnet-scheduler/src/schedule.rs:142-146` returns `ScheduleGate::Window(_) =>
  ScheduleClock::Local`, and `src/capture/runloop.rs:129-131` measures a window in local
  minutes. Status: VERIFIED-BY-READING.
- Impact: a birder who follows the instruction converts "06:00-20:00 local" into UTC by hand
  and records the wrong part of the day, shifted by their offset. They may miss the dawn chorus
  every day.
- Proposed change: change both strings to "in this station's local time (see Settings → Time
  zone)", and show the resolved window in local time next to the field.
- Acceptance gate: a unit test that renders the location tab and asserts no "UTC" near
  `fixed:`, plus an `installer/test/config-template.sh` assertion. Both fail on current text.
- Effort: S.

### OOB-05: A muted, zero-gain or clipping microphone looks healthy on the first screen
- Severity: **medium**. Kind: gap. **ALREADY-TRACKED(ON-15, ON-18 in part; HANDOFF §5 item 42)**.
  The "no low-gain advice" and "peak ignored" points are NEW.
- Evidence: the checklist's microphone row ticks on the supervisor's process gauge (`today.rs:330-337`),
  and `listening_row` then promises "Listening for the first call…" (`:394-397`). The outage
  banner and pill need a *previous* detection: `capture_outage` returns `Ok(None)` when there
  has never been one (`today.rs:454-455`). The wizard's two-second test is the only level
  check in the UI (`onboarding.rs:1278-1308`). It grades RMS alone, calls −60 to −80 dBFS
  "Very quiet. Normal in a quiet room" (`:1329-1340`), and computes `peak_dbfs` but never judges
  it (`:1299`). So clipping is never mentioned. An interactive bare-metal install with
  coordinates never shows the wizard at all (OOB-02's redirect rule). The doctor's gain grade
  has Muted, AboveCap and AtMaximum and **no "too low"**
  (`crates/birdnet-core/src/audio/input_level.rs:82-91,121-139`), so a capture control at 0 %
  with the switch on grades Ok. Its output is on `/admin/doctor`, behind sign-in.
  `docs/book/getting-started/hardware.md:27` points to "the Audio settings level meter", which
  does not exist (grep `level meter|measure_level` in `birdnet-web`: only the wizard).
  Status: VERIFIED-BY-READING.
- Impact: the commonest first-week failure (wrong gain, mic muted by a USB re-plug, foam
  touching the capsule) produces a calm "Listening…" for 24 h, until the deadman fires, and
  only to a notification channel the reader has not set up.
- Proposed change: run `acoustic_health`'s measurement, or a 10 s `measure_level`, at first
  start and every few minutes until the first detection. Put the verdict on the checklist:
  "Silent — check the mic isn't muted", "Very quiet", "Clipping — turn the gain down",
  "Mains hum at 50/60 Hz". Add `Advice::TooLow` (for example under 10 % or below −10 dB of
  hardware gain). Grade peaks at or above −1 dBFS as clipping. Fix the hardware.md link, or
  build the meter on `/station/capture`.
- Acceptance gate: a checklist test with a gauge "up" and a level of −90 dBFS must render
  "Silent". The counterpart at −40 dBFS must not. A `level_message` test asserting that
  peak = 0 dBFS says "clipping". Both observed failing first.
- Effort: M.

### OOB-06: A Pi left on the wrong time zone files every detection at the wrong hour, and the doctor passes it
- Severity: **medium**. Kind: gap. **NEW**. ON-7 and ON-8 cover Docker and the wizard's
  look-up, not this.
- Evidence: the installer never reads or sets a time zone (grep `timedatectl|timezone|TZ=` in
  `installer/lib`: no hits). The doctor's check returns **Pass** when the zone is valid and
  there is no wizard-detected zone to compare against (`src/doctor/clock.rs:256-259`). That is
  always the case after an installer-prompt location. Detections are filed in local time from
  the OS (`src/doctor/clock.rs:6-8`). Status: VERIFIED-BY-READING. CONJECTURED: how often Pi OS
  is left on its default zone (probe: count Imager users who skip "Set locale settings").
- Impact: dawn chorus at 11:00, "night" birds at noon. Every hour-of-day chart is wrong,
  silently.
- Proposed change: in the installer, after the coordinates, compare
  `round(lon/15)` with the host's UTC offset. If they differ by more than 2 h, say "This Pi's
  clock is set to Europe/London, but your location is in North America. Detections will be
  filed at the wrong hours. Fix: sudo timedatectl set-timezone America/New_York". Use a
  zone from a small coordinate→zone table, or from the same IP look-up the wizard uses. Run
  the same comparison in the doctor and as a Today banner.
- Acceptance gate: a doctor unit test with `system=Europe/London`, lat/lon in Ohio and no
  detected zone must warn (it passes today). Counterpart: `America/New_York` passes.
- Effort: S–M.

### OOB-07: The admin password is a 22-character string the reader must copy from a terminal, and recovery needs `sudo grep`
- Severity: **medium**. Kind: footgun. **NEW** as a usability finding. Security behaviour is
  as designed.
- Evidence: `installer/lib/70-station.sh:150-165,174-183`. `installer/lib/80-summary.sh:40-46`
  "Save it now — it is not shown again". `crates/birdnet-web/templates/login.html:136-141`
  (recovery: `sudo grep '^CADDY_PWD' /etc/birdnet/birdnet.conf`). A headless install with no
  location then *requires* that password before the wizard will draw (`onboarding.rs:105-110`).
  The wizard's Welcome text promises "nothing to type in a terminal" (`onboarding.rs:1677`).
  Status: VERIFIED-BY-READING.
- Impact: lost passwords lead to terminal work or a reinstall. A phone user over SSH may not be
  able to copy it at all.
- Proposed change: have the installer ask for **no** password by default, and let the wizard's
  existing "first browser to finish owns the station" step (`onboarding.rs:334-372`) set it.
  Bind that window to a one-time **setup code** of six digits, printed in the summary and
  encoded in the QR URL (`/onboarding?code=…`), so a LAN neighbour cannot claim the station.
  Add `birdnet-behavior --reset-admin-password` and say so on the login page.
- Acceptance gate: an integration test that fresh-installs with no password. `/onboarding`
  without the code must refuse to set a password, and with the code must set it. Counterpart:
  after setup the code no longer works.
- Effort: M.

### OOB-08: Updating needs a terminal; the station is never updated by this reader
- Severity: **medium** (high over a year). Kind: gap. **ALREADY-TRACKED(LC-3, NP-11, RC-35, A-17)**.
- Evidence: `README.md:162` says "no scheduled self-update". `POST /admin/update/apply` is
  routed (`crates/birdnet-web/src/routes/admin/update.rs:17-21`) with no UI caller (LC-3).
  New detail: the unit is `ProtectSystem=strict` with `ReadWritePaths=${DATA_DIR} ${CONFIG_DIR}
  /run /var/log` (`installer/lib/65-service.sh:189-191`), and the binary is in `/usr/local/bin`
  (`installer/lib/10-config.sh:8`). So the in-process updater cannot write its target under the
  shipped unit even if a button existed. Status: VERIFIED-BY-READING (the sandbox effect is
  inferred from the unit, not run).
- Impact: security and model fixes never reach the stations that most need them.
- Proposed change: a root-owned `birdnet-behavior-update.service` plus `.timer` (weekly, off
  or on as a wizard choice) that runs the release `install.sh update` with its checksum and
  rollback (`.prev`). Add a "Check for updates / Update now" button that starts that unit
  through a polkit rule scoped to it.
- Acceptance gate: `installer/test/upgrade-e2e.sh` on real systemd: trigger the timer and
  assert the version changes and the service is healthy. Counterpart: a broken release
  (checksum mismatch) leaves the old binary running.
- Effort: M–L.

### OOB-09: The address the reader must type is an IP and a port; no `birdnet.local`
- Severity: low. Kind: polish/opportunity. **ALREADY-TRACKED(A-19)** for the alias. The port
  point is NEW.
- Evidence: `installer/lib/80-summary.sh:93-137` prints IP:8502 first, and `.local` as "works
  on most devices, but not all". `src/helpers/system.rs:416-428`: "a station is reachable as
  `birdnet.local` only if the machine is called `birdnet`". The unit drops every capability
  (`65-service.sh:216`), so port 80 is impossible. Status: VERIFIED-BY-READING.
- Impact: "http://raspberrypi.local:8502" is hard to remember, and collides with a second Pi.
- Proposed change: offer (default yes) to publish an Avahi alias `birdnet.local`. Add
  `AmbientCapabilities=CAP_NET_BIND_SERVICE`, or a second listener on :80 that redirects to
  8502. Print `http://birdnet.local` as the headline address.
- Acceptance gate: `installer/test/service-unit.sh` asserts the alias file and the :80 listener.
  Manual: a phone on the same Wi-Fi opens `http://birdnet.local`.
- Effort: S–M.

### OOB-10: The Docker quick-start stores the unstable ALSA card index
- Severity: **medium**. Kind: footgun. **ALREADY-TRACKED(LC-9, RC-32)**, still true at this commit.
- Evidence: `quickstart.sh:333-340` builds `candidate="plughw:${first_card},${first_dev:-0}"`.
  The bare-metal installer documents that the index moved from 1 to 3 across a cold reboot on
  a Pi 4 and addresses the card by id instead (`installer/lib/70-station.sh:33-67`).
  Status: VERIFIED-BY-READING.
- Impact: after a reboot that re-enumerates USB, a Docker station "serves a healthy dashboard
  and records nothing". These are the Bookworm users who were sent to Docker.
- Proposed change: port `detect_first_audio_device` into `quickstart.sh`, or source one shared
  function.
- Acceptance gate: `installer/test/alsa-device-detect.sh` run against quickstart's detector with
  the fixture listing. Expect `plughw:CARD=PRO,DEV=0`.
- Effort: S.

### OOB-11: The setup documentation describes controls that do not exist
- Severity: low. Kind: bug (docs). **NEW**, except the last item.
- Evidence (each doc line against the code):
  - `docs/book/getting-started/installation.md:57` says the quick-start's auto-detect is "via
    ipwho.is over TLS". `quickstart.sh:398-401` asks **ipapi.co**.
  - `installation.md:78` says "If you answer 'restrict to this device' in the interactive
    prompts". There is no such prompt, deliberately (`installer/lib/70-station.sh:308-314`).
  - `docs/book/getting-started/first-steps.md:32` says Location has "a lookup from a place
    name". The wizard has only an IP look-up and two number boxes (`onboarding.rs:1719-1724`).
  - `docs/book/getting-started/hardware.md:27` mentions the "Audio settings level meter". It
    does not exist (OOB-05).
  - `hardware.md:39` says ffmpeg is installed "when your config already has an `RTSP_URL`". It
    is installed for every station (`installer/lib/45-preflight.sh:160-171`).
  - `installer/lib/70-station.sh:189` says the config file "is the source of truth afterwards".
    The settings table wins (**ON-17**, tracked).
  Status: VERIFIED-BY-READING.
- Impact: a reader who follows the manual looks for things that are not there, and stops
  trusting the rest.
- Proposed change: correct each line. For the place-name search, either build it (OOB-20b) or
  delete the claim.
- Acceptance gate: a docs test (in the style of `tests/*docs*`) grepping for "restrict to this
  device", "place name", "level meter" and "ipwho.is" in the quick-start section. Fails now.
- Effort: S.

### OOB-12: Three different notification-trigger defaults
- Severity: low. Kind: footgun. **NEW**.
- Evidence: the wizard pre-selects `new-species` (`onboarding.rs:169`, rendered "recommended",
  `:1777`). The daemon's flag defaults to `each` (`src/cli.rs:658`). The settings form's
  fallback is `each` (`crates/birdnet-web/src/routes/admin/settings/render/mod.rs:159`). An
  installer-configured station skips the wizard (OOB-02's rule), so when its owner later adds
  Telegram under Alerts, the trigger is `each`, which the wizard itself calls "Chatty —
  hundreds a day at a busy feeder" (`onboarding.rs:1779`). Status: VERIFIED-BY-READING.
- Impact: the first channel added floods the phone, and the reader turns notifications off for
  good.
- Proposed change: one constant `DEFAULT_NOTIFY_TRIGGER` in `birdnet-core::config`, read by all
  three, the way `DEFAULT_CONFIDENCE_THRESHOLD` already is. Choose `new-species-daily` or
  `new-species`.
- Acceptance gate: a test that the three agree, in the style of the confidence-default one.
  Fails now.
- Effort: S.

### OOB-13: The live microphone feed is open to anyone on the network by default
- Severity: **medium**. Kind: footgun (privacy). **NEW** as a default. The mechanism to close
  it exists (O-4, private mode).
- Evidence: `crates/birdnet-web/src/private_mode.rs:1-26`: "the detection history, the live
  microphone feed of somebody's garden and both WebSockets are all served to whoever has the
  URL". Private mode is opt-in (`PRIVATE_MODE`, `src/helpers/private_mode.rs:15-36`), and the
  wizard never mentions it. The human-sound filter governs stored windows
  (`config.rs:45-54`), not `/stream`. Status: VERIFIED-BY-READING.
- Impact: a guest-Wi-Fi user, a housemate, or the internet (after a router's automatic port forwarding or a reader's
  port-forward) can listen live to the garden and the conversations in it.
- Proposed change: require sign-in for `/stream` and the spectrogram socket by default (the
  `live_audio` carve-out becomes opt-in). Add a wizard question: "Who can see your station?
  Everyone on my Wi-Fi / Only me", which sets private mode.
- Acceptance gate: an unauthenticated `GET /stream` on a default station answers 401/303.
  Counterpart: with `live_audio` opted in, it streams.
- Effort: S–M. Judgement: this changes what an existing station's listeners see. Decide with
  the maintainer.

### OOB-14: SD-card wear for an unattended Pi
- Severity: **medium** (over a year). Kind: gap. **ALREADY-TRACKED(PS-4)**. Defaults add to it.
- Evidence: PS-4 measured 82.6 KB of block writes per detection
  (`docs/UNATTENDED_DEPLOYMENT_AUDIT.md` §3.1). Defaults that add writes: WAV clips
  (`src/cli.rs:925`) and a persistent 200 MB journal (`installer/lib/65-service.sh:282-289`).
  Status: VERIFIED-BY-READING for the defaults; the wear figure is the register's.
- Impact: a cheap card fails in months, and backups live on the same card (LC-13).
- Proposed change: beyond PS-4's remedy, have the installer detect when root is an SD card
  (`/dev/mmcblk0`) and recommend, in plain words, a USB SSD or "high-endurance" card. Offer
  `AUDIOFMT=flac` (lossless, about half the size) as the default for new installs.
- Acceptance gate: per PS-4. For FLAC: `daemon::config` default test plus a doctor encoder
  check.
- Effort: S (advice, FLAC default). L (PS-4).

### OOB-15: First-run wording on the installer and wizard path is jargon
- Severity: low. Kind: polish. **NEW**.

| Where | Today | Plain replacement |
|---|---|---|
| `installer/lib/70-station.sh:212` | "Use detected ALSA device 'plughw:CARD=PRO,DEV=0'?" | "Use the microphone 'Comica Traxshot PRO' (USB)?" (the bracketed name from `arecord -l`) |
| `70-station.sh:219` | "Audio source — ALSA device (e.g. plughw:1,0) or rtsp:// URL (Enter to skip)" | "No microphone found. Plug one in and press Enter to look again, or paste a camera's stream address (starts with rtsp://)" |
| `70-station.sh:253` | "Station location (solar schedule, species filter, BirdWeather)" | "Where is the microphone? (so it only names birds that live near you)" |
| `70-station.sh:297` | "Set an admin password now (Enter to auto-generate one)" | "Choose a password for changing settings (or press Enter and set it in the browser)" |
| `installer/lib/80-summary.sh:129-130` | "A new install takes --classifier birdnet-v24 or birdnet-v3; this station keeps what /etc/birdnet/birdnet.conf names." | delete on a fresh install; keep on update only |
| `80-summary.sh:45` | "(auto-generated, saved as CADDY_PWD in /etc/birdnet/birdnet.conf …)" | "Write it down. Forgot it later? See 'Forgotten the password?' on the sign-in page." |
| `80-summary.sh:177` | "Logs:  sudo journalctl -u birdnet-behavior -f" | move under "For troubleshooting" |
| `80-summary.sh:148-149` | "set ALSA_CARD / RTSP_URL in /etc/birdnet/birdnet.conf" | "Open the dashboard → Settings → Capture and choose your microphone" (drop the file instruction) |
| `onboarding.rs:738` | "A sound card is written like plughw:CARD=Device,DEV=0 or plughw:2,0." | "Pick the microphone from the list above. Typing a name is for experts." |
| `onboarding.rs:661` (mic body) | "written the way ALSA names it, like plughw:CARD=Device,DEV=0" | Collapse behind "Advanced". |
| `today.rs:298` | "no model is loaded — this process runs without the detector, or the model files are missing" | "The bird-identifying model isn't running. Open Diagnostics to see why." |
| `onboarding.rs:1717` | "compute sunrise / sunset for the dawn-chorus window" | fine. Keep. |
| `docs/book/getting-started/first-steps.md:9` | "Run `hostname -I` on the Pi" | "Look for 'birdnet' in your router's list of devices, or scan the QR code the installer printed." |

- Status: VERIFIED-BY-READING (strings quoted from the files).
- Acceptance gate: a string-inventory test over the installer prompts that forbids the tokens
  `plughw`, `ALSA`, `CADDY_PWD` and `--classifier` on the fresh-install path. Fails now.
- Effort: S.

### OOB-16: The wizard's microphone test probably fails with a raw ALSA error on the card the station is already using
- Severity: low. Kind: bug. **NEW**, CONJECTURED.
- Evidence: `measure_level` runs `arecord -D <card> -d 2 …` and passes on the first stderr line
  (`crates/birdnet-core/src/audio/capture/probe.rs:285-305`). The Test button appears on
  listed cards (`onboarding.rs:690-703`). On bare metal the service is already capturing from
  the auto-detected card (`installer/lib/75-start.sh:68`), and a USB `hw` device is usually
  exclusive. Status: **CONJECTURED**. Probe: on a Pi with the service running, open
  `/onboarding`, press Test on the configured card, and read the message. Expected text:
  "audio open error: Device or resource busy".
- Impact: the reader sees an error and assumes the microphone is broken.
- Proposed change: when the card is the configured, running source, read the level from the
  running capture instead (the live spectrogram or acoustic-health path). Otherwise map
  `busy` to "This microphone is already in use by the station. Its level is …".
- Acceptance gate: a unit test on the error mapping, plus the probe above.
- Effort: S–M.

### OOB-17: After a power cut with no network, an RTC-less Pi records under a stale clock with nothing saying so
- Severity: **medium** for field stations. Kind: gap. **NEW** (CONJECTURED). Related to TD-8
  and NT-*.
- Evidence: the "clock is untrustworthy" floor is 2024-01-01
  (`crates/birdnet-core/src/civil.rs:476`), so only an epoch-like clock fails open. Pi OS
  restores the last-saved time at boot (fake-hwclock or `systemd-timesyncd`'s clock file;
  outside knowledge, not read here). That passes the floor, so capture runs and files
  detections at times that are hours or days late. The unit orders on `time-sync.target` but
  does not `Wants=` it (`installer/lib/65-service.sh:62-63`). The clock alert is a
  station-health episode (TD-8) and is not on Today. Status: **CONJECTURED**. Probe: Pi
  offline, cut power for 2 h, boot, compare `detections.Time` with a phone's clock and read
  `/` and `/station`.
- Impact: a field season's timestamps shift silently, and phenology and dawn data are wrong.
- Proposed change: when `timedatectl show -p NTPSynchronized` is `no`, mark new detections as
  "clock unverified" (a column, or an `analysis_runs` flag). Show a Today banner: "The station
  can't check the time (no internet). Times may be off." Recommend an RTC battery (Pi 5 has an
  RTC connector) in the hardware guide.
- Acceptance gate: a unit test that feeds NTP state "no" and expects the banner. Counterpart
  "yes": no banner.
- Effort: M.

### OOB-18: The settings and password are in no backup; nothing guides a dead-card recovery
- Severity: low. Kind: gap. **ALREADY-TRACKED(LC-13)**.
- Evidence: `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` LC-13. The README promises "survives … a
  dead SD card" via opt-in `OFFSITE_BACKUP` (`README.md:202`). Nothing in the wizard offers
  backup (`onboarding.rs:1672-1832`, seven steps, none about backup). Status: VERIFIED-BY-READING.
- Proposed change: an eighth wizard card, "Keep a copy somewhere safe", that downloads a
  passphrase-protected backup to the browser monthly (a reminder) or sets up offsite.
- Effort: M.

### OOB-19: No notice of the model's non-commercial licence at install or in the wizard
- Severity: low. Kind: opportunity/compliance. **NEW**.
- Evidence: grep `licen` in `crates/birdnet-web/src/classifier_setup.rs`,
  `crates/birdnet-integrations/src/model_catalog.rs` and `installer/lib/55-model.sh`: no hits.
  The README has the licence banner (`README.md:27-29`). Status: VERIFIED-BY-READING.
- Proposed change: one line on the Model step: "BirdNET is free for personal and research
  use (CC BY-NC-SA 4.0)." No click-through, which would be friction without a legal need (the
  maintainer's call).
- Effort: S.

### OOB-20: Location entry has no map and no place search; an offline install cannot complete
- Severity: low. Kind: opportunity. **NEW**.
- Evidence: (a) `installer/lib/55-model.sh:396-403`: a failed model download is `fatal`, and
  the installer needs `huggingface.co` and `raw.githubusercontent.com`
  (`installation.md:40`). There is no "install now, fetch the model later from the wizard"
  path, although the wizard can install the model (`onboarding.rs:1703-1710`). (b) Location
  is an IP look-up (third party, city-level) or typed decimals (`onboarding.rs:1719-1724`).
  `navigator.geolocation` is unusable on `http://` LAN origins (browsers require a secure
  context; outside knowledge), so the phone's GPS cannot help either. Status:
  VERIFIED-BY-READING (a). (b) as read; the secure-context limit is external.
- Proposed change: (a) on a failed download, warn and continue. The service starts and the
  wizard's Model step offers the retry. (b) Embed a small offline place list (GeoNames
  cities ≥ 15 k, about 1 MB) for "type your town" search, or a tap-on-map with bundled
  low-resolution tiles.
- Effort: (a) S, (b) M.

### OOB-21: Hardware guidance is generic; no recommended kit, height or orientation, and the target level disagrees across pages
- Severity: low. Kind: gap. **NEW**.
- Evidence: `docs/book/getting-started/hardware.md:16-27` lists microphone classes only, with
  no tested model, no mounting height, and no advice on power supply or case.
  `hardware.md:27` says "aim for peaks near −6 dB". `docs/book/field/input-level.md` says
  "−12 to −6 dB" and that "Nothing here was measured on a sound card". HANDOFF §5 item 42:
  "The capture-gain reader has not met a sound card". Status: VERIFIED-BY-READING.
- Proposed change: a "Shopping list" page with two or three tested USB microphones and a
  waterproofing method, *after* measuring them on a Pi (an acceptance run, not a claim). Unify
  the level target. Put a "Sound check" card on `/station/capture` (OOB-05) with mains-hum and
  wind detection (the acoustic-health spectrum is already computed).
- Effort: M (needs hardware).

### OOB-22: The Docker quick-start runs the development branch's script, and the install needs Docker group surgery
- Severity: low. Kind: footgun. **ALREADY-TRACKED(LC-9, LC-16 in part)**.
- Evidence: `README.md:117-120` (`bash <(curl … /main/quickstart.sh)`, compose files from
  `main`). `quickstart.sh:238-245` tells the reader to `usermod -aG docker` and log out. Status:
  VERIFIED-BY-READING.
- Proposed change: publish `quickstart.sh` and the compose files as release assets, the way
  `install.sh` already is (`installation.md:74`). Let the script add the group and re-exec
  itself under `sg docker`.
- Effort: S.

### OOB-23: What works well (recorded so it is kept)
- Kind: note. Status: VERIFIED-BY-READING.
  - Mic identity by ALSA id, which survives re-enumeration (`installer/lib/70-station.sh:33-67`).
  - Sample rate probed per device (`src/capture/sources.rs:551-567`).
  - The `audio` group and `char-alsa` device access are in the unit (`65-service.sh:253-255`).
  - The service starts with no microphone and the dashboard says so
    (`75-start.sh:56-76`; `today.rs:310-317`).
  - Watchdog plus infinite back-off restart (`65-service.sh:87,145-164`).
  - Disk purge with a species floor (`src/cli.rs:950-973`).
  - Persistent bounded journal.
  - The coordinate parser accepts an OSM pair and a decimal comma (`70-station.sh:77-133`).
  - A QR code of the dashboard URL (`80-summary.sh:10-17`).
  - The wizard refuses to draw from defaults on a failed read (`onboarding.rs:33-37`).
  - BirdNET-Pi-parity detection defaults (`config.rs:28-68`).

---

## Not examined

- No probe was run (read-only audit). Every CONJECTURED item above names its probe.
- `Dockerfile`, `docker/entrypoint.sh` and `docker-compose.pulse.yml` were not read beyond the
  compose excerpts cited. Neither was the macOS path (`installer/lib/85-macos.sh`,
  `packaging/macos/*`).
- `uninstall.sh` was syntax-checked only. `install.sh uninstall` vs `uninstall.sh` is LC-14.
- `routes/admin/settings/*` beyond the location hint and the trigger default.
  `routes/admin/audio.rs` beyond route names. `open_admin_host.rs`, `tls.rs` and the
  accounts/first-run seed in `birdnet-db` were not read. `--channel-report` (stereo-mic
  downmix loss) is not surfaced on the first-run path, and whether it should be was not
  assessed.
- The tmpfs stream mount under `PrivateTmp=yes` (`60-dirs.sh` vs `65-service.sh:192`):
  whether the service ever sees the host's tmpfs mount was not settled. PS-18 is adjacent.
- `crates/birdnet-integrations/src/model_catalog.rs` origins and resume behaviour. TROUBLESHOOTING.md,
  SUPPORT.md and `docs/book/guides/troubleshooting.md` were not read.
- `birdnet-migrate`'s handling of a BirdNET-Pi `birdnet.conf` adopted *as the station's own
  config* (OOB-03's second half).
- Accessibility and phone layout of the wizard (covered by the repository's own gates).
