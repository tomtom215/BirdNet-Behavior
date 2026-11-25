# Decisions of 2026-11-25

The audit's README lists decisions "that are the maintainer's". On 2026-11-25 the maintainer left them to the
session, to be made from analysis and not from a guess. Five read-only analyses were run against `main` at
`89a64fb`, each told to re-verify the audit's claims at today's lines and to argue against the audit's own
recommendation where it was weak. This file records what was decided, on what evidence, what is still
unmeasured, and what would reverse each decision. The maintainer can overrule any of them; nothing here is
built yet.

How the evidence is labelled: **read** means the cited lines were opened; **run** means a script or command
produced the number; **modelled** means arithmetic on the code's constants with stated assumptions;
**unmeasured** means nobody has the number. The session spot-checked a sample of each analysis's claims
(named under each decision) and reran the two arithmetic scripts; the rest of each analysis is the
analysing session's reading, not this session's.

Two rules apply to every decision below. An existing station keeps its behaviour on upgrade unless its
owner changes a setting (`STABILITY.md` counts a changed default as breaking). And nothing that is stored is
merged, moved or deleted to make a count come out differently.

## 1. Events across sources (MSC-02, MSC-07, RES-18)

**Decided: an event is computed when it is read, never stored on the row.** One definition in SQLite and one
in the DuckDB copy, over `detected_at_utc`, site and species. An event holds at most one row per source, rows
are paired to the nearest and never chained, so a chorus cannot grow into one hour-long event. Counts are
counts of events. Every response and export that counts events states the tolerance and the site map it used.
BirdWeather and notifications are sent once per event; the row is always written.

The audit recommended an event id written with the row. That is the least reversible form: sources are
analysed one segment at a time and a second source's row can land minutes after the first (read:
`file_settle.rs:29`, `daemon/mod.rs:265-300`), so the writer would have to look back and sometimes join two
existing events, and changing the tolerance would mean rewriting history.

**The tolerance is a setting whose default is the model's window plus one second** (4 s for V2.4, 5.5 s for
V3.0). That is the lower bound the code fixes: a row's time is its segment's whole-second stamp plus the
window offset, truncated (read: `civil.rs:369`, `model.rs:544`), so two sources on one host clock can differ
by up to a window and a second before any latency. What the repository cannot give is the difference in
latency between sources, which for RTSP is unmeasured (MSC-14). The measurement: a click every 10 s into a
microphone and a camera side by side for 30 minutes, at overlap 0 and 1.5, then again after a forced stream
stall; the tolerance is the window plus one second plus the 99th percentile of the difference. The same
histogram can be drawn from a station's own rows and should ship as a diagnostic.

Spot-checked: MSC-02, MSC-07 and RES-18 said `in_progress` with evidence "Not started" (now `open`); the only
cross-source matcher is the read-only "Also heard by" card with a fixed 30 s on local time
(`read.rs:271-297`, `detection_detail.rs:101`); the duplicate gate is keyed by source and drops rows
(`duplicate.rs:38-56`).

**Against it:** a count now depends on a setting, so last month's export can disagree with today's after the
tolerance changes. Stamping the parameters on every export is the answer; a researcher who needs a frozen
answer keeps the export. **Reverse it if** all-time event queries prove too slow in SQLite on a Pi
(unmeasured): add a derived table that is rebuilt when the tolerance or the site map changes, still never a
column on the row.

## 2. One site or several (MSC-18, N06-02)

**Decided: every source has a site, and on upgrade each existing source is its own site.** Nothing is grouped
until an owner says two sources are in the same place; adding a second source asks. The site is also where a
per-site location and species list will hang.

The two mistakes cost different things. Counting one bird twice is today's visible state and one label fixes
it. Merging two birds heard 200 m apart is a silent undercount that also withholds posts from BirdWeather,
which cannot be taken back.

## 3. Segment seams (MSC-08, ACC-10)

**Decided: analysis stays per file by default.** The audit's premise was out of date. At the shipped settings
(15 s segments, 3 s windows, no overlap; read: `cli.rs:609,895`) no window is zero-padded and a segment
boundary is exactly as costly as each of the four window boundaries inside a segment (run:
`scratchpad/seams/seams.py`, rerun by this session: a 1 s call fails to fit whole in 33.0 % of positions,
identical for per-file and continuous analysis). Parity with BirdNET-Pi is "each file analysed alone", and a
test pins the window starts for 155 lengths.

The seam does cost recall off the defaults: with overlap, the 4.5 s model, a segment length that is not a
multiple of the step, or corroboration on. So, in order:

1. Ship now, changing no count: a `--doctor` and settings warning when the segment length is not a multiple
   of the step or the last window is padded, and the correction to the CLI help ACC-10 names.
2. Measure, in this repository's own container: slide the real magpie clip across a seam in 0.1 s steps with
   the real V2.4 model, per file and continuous, at overlap 0, 1.5 and 2.0. The prediction to falsify is no
   difference at overlap 0 and a dip at the seam otherwise.
3. Only if the dip is real, ship continuous analysis as an opt-in setting: the next segment reads a tail the
   length of the overlap from the previous file on disk, guarded by the clip extractor's existing "these two
   files join" check, falling back to per file.

**Against it:** a code path whose benefit exists only off the defaults, resting on whole-second file stamps.
**Unmeasured:** how often the join check refuses a real join, and the Pi cost.

## 4. Non-detections and scores below the threshold (RES-04)

**Decided: three tiers, and the audit's single "per-window summary, off by default" is not built.**

1. **On for every station:** one row per hour, source, analysis run and species holding the highest score
   and counts at fixed floors, for species that cross a low floor, and the number of windows analysed added
   to the effort ledger. Human and sound classes are never written. Its size is bounded by species times
   hours and is of the order of the level tables the station already keeps (modelled: about 22 MB a year for
   150 species on synthetic scores).
2. **Opt-in:** every window's scores at or above a floor, one packed row per window, keyed to the analysis
   run, with rolling retention, in a separate database file so it stays out of the 14-copy backup ring.
3. **Opt-in:** the audio of a random sample of windows below the threshold, stratified by score, fed to
   blind review. This, not a table of scores, is what calibration and a measured miss rate need.

Why not the audit's proposal: the top five scores by rank cannot re-threshold old data exactly, a score
table alone cannot calibrate anything because nobody has verified those windows, and binary occupancy is
already exportable as detected, listened-and-nothing, or not listening (read: `routes/export/mod.rs:41`).
Sizes (run, rerun by this session, one source at defaults, 28,800 windows a day): top five packed 0.46 GB a
year, a 150-species list 2.0 GB, the full vector 92 GB. Each full backup multiplies the live file by up to
fourteen (read: `maintenance.rs:105`).

**Against it:** tier 1 adds a write path most stations never read, and its floors freeze what can be re-cut
later. **Unmeasured, before tier 2's defaults are chosen:** how many species cross each floor per hour on
real audio, and bytes written per day on an SD card. **Privacy rule for all tiers:** a masked window is
counted per hour, never flagged per window, because a per-window flag is a record of speech to the second.

## 5. Live audio (OOB-13)

**Decided: the audio routes need a sign-in on new installs; the live spectrogram stays open; an existing
station keeps what it has.** A new setting, `LIVE_AUDIO=signed-in|anyone`, with a control in Settings. A
station that already holds detections is seeded `anyone` at its first start on the new version, and the
changelog entry is marked breaking for new installs. Players that cannot send a cookie get a signed listen
link.

The reason is consistency, not a new threat: stored clips pass the human-sound filter and live audio passes
none, so the unfiltered surface is the one open to everyone who can reach port 8502 (read: `routes/mod.rs:
95-98`, `server.rs:186-188`; bind `0.0.0.0`, `cli.rs:100`). The spectrogram is left open because the Today
page draws it for visitors and each frame is a 128-band picture of about 0.7 s.

**Must be tested before shipping, because it is inferred and not run:** that speech cannot be recovered from
the spectrogram frames. If it can, the spectrogram goes behind the same setting. **Against it:** the home
network is already the stated trust boundary, and this adds a sign-in for every household listener.

## 6. First sign-in (OOB-07)

**Decided: the audit's six-digit setup code is rejected as written. A new install gets a single-use setup
token of at least 80 bits, and the admin routes are closed while it is pending.**

The proposal left the station with no password until the wizard ran. In that state the admin gate opens for
every admin route, not only the password step (read: `auth_middleware.rs:316-336`): updates, backups, the
support bundle, audio sources. That is a regression from today's install, where a visitor can read and
change nothing. Six digits is about 20 bits, and the login throttle is five failures per address per fifteen
minutes with no global cap (read: `login_throttle.rs:23-38`), which a host with many addresses walks
through.

What is built instead: the token rides in the QR code the installer already prints, so a phone user types
nothing; the printed fallback is short groups of letters with its own global attempt limit; setting the
password consumes the token; a command prints a new one. An interactive install can still type a password.
A station that already has a password, or already runs without one, is untouched. The sign-in page's
"forgotten the password" hint is corrected in the same change: after a password change in the browser, the
command it gives prints a stale one (read: `helpers/auth.rs:122-135`).

**Against it:** a third credential state in an auth layer that has failed closed before. A 22-character
password is simpler and already safe; the token is chosen because a non-technical owner cannot be expected to
copy it from a terminal.

## 7. A model download that fails during install (OOB-20)

**Decided: retry a bounded number of times, then finish the install and say loudly that the station is not
detecting.** The installer exits with a distinct status so a script can tell, the summary says what is
missing, and the Today page already says "Detector not running" with the wizard's one-button install
(read: `today.rs:406`, `onboarding.rs:8-11`). A model file on local disk is accepted when it matches the
pinned SHA-256 and size, which is the same check every download passes, so no path weakens integrity (read:
`model_catalog.rs:228-262`). The project's own mirror is tried first. Docker's entrypoint follows the same
rule.

Today the failure stops the install before the configuration and the service exist, leaving a binary and
nothing else. The test that pins "stops the install" (`installer/test/classifier-default.sh:244`) is changed
on purpose and watched failing first.

**Against it:** a finished-looking install that hears nothing is the defect this repository warns about
most. The distinct exit status and the first screen are the mitigation; a station that reports "installed"
with no model and no warning would be a failed implementation of this decision.

## 8. A flashable image (OOB-01)

**Decided: no image now.** First a "before you start" page for Raspberry Pi Imager's own settings (host name,
Wi-Fi, user, time zone), then a first-boot snippet that runs the release installer, once decision 6 exists to
carry the credential. An image is a per-release build to rebase on every Raspberry Pi OS update, sign, boot
test and support. Whether to take that on is a commitment of the maintainer's time and stays theirs; nothing
in the repository can settle it.

## 9. The low-severity rows

All 65 low rows that were open or in progress were read against today's code. The table of verdicts is in
`checklist.json`, row by row. In sum:

- **Will be done: 39**, 27 of them small. Six are sentences that are untrue today.
- **Wait for something named: 12** (decisions 1 and 2, CHT-24, RES-07, ACC-03, UIC-08, or data that does not
  exist yet).
- **Will not be done: 4.** UIR-28 (one design for every detection row: six surfaces doing different jobs,
  a large regression risk for consistency alone), UIC-22 (skeletons instead of the word "Loading…": the text
  is honest and accessible), UIC-31 (a structural escaping type across about 640 call sites, with no defect
  found and four gates already holding the class), and the single-chart half of UIR-25 (the second curve is
  what a reader without scripts gets). Each row says what would change the verdict.
- **Need a Pi or a microphone: 4** (MSC-21, MMP-13, OOB-21, UIC-32).
- **Already fixed: 2** in substance (ACC-17, UIC-19), each with a sentence left to correct.
- **Decided here:** OOB-09 will not get a network alias or port 80 (the host name belongs to the Imager page
  of decision 8, and the QR code removes the typing); OOB-20 is decision 7.
- **Still the maintainer's:** where the review queues live in the navigation (UIR-35), which species count as
  triggers for the event-aligned figure (CHT-19), and the licence of exported data (RES-24).

Three rows were rated too low and are now medium: RES-26 (two definitions of a listened day, 60 s in
`listening.rs:28` and a share of the typical day in `reporting.rs`, so one station's figures disagree),
N24-08 (raw weekly counts decide which species are drawn as migrants), and the kiosk's controls at 35 %
opacity (UIR-31, an accessibility failure at rest on a touch display).

## Found on the way, untrue today

`docs/book/field/hardening.md:36-38` describes an installer prompt that does not exist, and `:144-150` gives
a wrong reason for the open sockets; `SECURITY.md:94-98` points to a threat model that is not in the file it
names; `README.md:117` says the Docker quick-start runs v0.17.1; `docs/UNATTENDED_DEPLOYMENT_AUDIT.md:603`
says every segment is padded. They are rows N25-12 to N25-15.

## Order of work this sets

Decisions 3 (the warning and the measurement), 7 and the 27 small low rows need nothing else and come first.
Decision 6 comes before 5's listen link and 8's first-boot snippet. Decisions 1 and 2 are built together,
with the diagnostic histogram first so a real station can supply the tolerance. Decision 4's first tier
follows the effort ledger it extends.
