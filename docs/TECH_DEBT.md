# Known debt

Defects and unfinished work that are known, have evidence, and were not fixed in
the change that found them. A row here is a debt, not a verdict: it says what is
wrong, how that was established, and why it is still open. Anything fixed is
deleted from this file in the commit that fixes it; the history is in git and the
CHANGELOG.

`tests/a_failed_read_is_not_defaulted_into_a_fact.rs` reads this file: every
sweep row of class `tracked:TD-n` in `tests/data/read_default_sweep.tsv` must
name a `## TD-n` heading below.

Rows are labelled the way the rest of the repository labels claims: **PROVEN**
(a test or a command showed it), **VALIDATED** (checked another way, said how),
**PLAUSIBLE**, **CONJECTURED**.

## TD-2 Help links: half the topics have no call site

* **Where:** `crates/birdnet-web/src/routes/pages/help.rs`.
* **What:** 12 of the 24 `Topic` variants are never named outside `help.rs`
  (`Sharing`, `Feeds`, `DisplayPrefs`, `Kiosk`, `AdminSettings`, `AdminAudio`,
  `AdminRecording`, `AdminRemoteAccess`, `AdminBackups`, `Glossary`, `Faq`,
  `MigrationImport`), and `help_drawer` has no caller. Counted by a script over
  `Topic::<Variant>` in `crates/birdnet-web/src`.
* **Why it is open:** the admin-settings, audio and backup pages are being
  reworked by other changes; adding a one-line `help_link(Topic::…)` to a page
  that is being rewritten is churn. These are the `TODO(O-20-followup)` markers
  the code used to carry.

## TD-3 The boot journal records a detection count it could not read as 0

* **Where:** `src/helpers/boot_journal.rs`, `record_boot`:
  `detection_count(conn).unwrap_or(0)`, and the same for the list of USB sources.
* **What it does:** a start whose database answers every other question but the
  count writes `detections: 0` into the journal. The next start compares itself
  to that, so a database that comes back with its three years of records is not
  seen to have come back, and one that loses them later is compared with a
  baseline of nothing. `BootRecord::detections` is a plain `i64` (`birdnet-web/src/boot_journal.rs`), so there
  is no way to write "unknown".
* **Evidence:** PROVEN by reading `record_boot` and `birdnet_web::boot_journal`.
* **Why it is open:** it is a start-up edge (the database has to open and then
  fail one `COUNT(*)`), and the fix changes the persisted journal's schema
  (`Option<i64>`) and `record`'s comparison, which want their own compatibility
  tests against journals written by earlier versions.

## TD-4 The setup wizard logs a failed source read and says nothing

* **Where:** `crates/birdnet-web/src/routes/pages/onboarding.rs` (`add_source`).
* **What it does:** `add_source` matches on the read of the source list, logs
  "could not read the audio sources; the microphone was not added" at `error` and
  returns, but it returns `()`, so the page the person sees does not say that
  nothing was added. The Capture tab's own duplicate guard, which this entry also
  covered, read the list with `.unwrap_or_default()` until WP-04 of the 2026-11
  audit (MSC-10); it now refuses when the read fails. The other sites this entry
  first listed (`sources_body`, `listen_default`, `count_oobs`,
  `listen_default_id`) were rewritten by the setup-wizard change and no longer
  default their errors.
* **Evidence:** read at the cited places on 2026-10-21 and the Capture tab re-read
  on 2026-11-24; the rows of class `tracked:TD-4` in
  `tests/data/read_default_sweep.tsv` are what the sweep still reports, and I did
  not re-classify the four wizard rows after the rewrite.
* **Why it is open:** the wizard's fix is a message on a page that has just been
  rewritten, and was not part of the change that found it.

## TD-5 RTSP credentials are on ffmpeg's command line

* **Where:** every place the station gives ffmpeg a stream address: capture
  (`crates/birdnet-core/src/audio/capture/process.rs`, `rtsp_command`), the live view
  and listening streams (`crates/birdnet-web/src/routes/livestream.rs`,
  `start_encoder`), HLS (`routes/hls.rs`, through `start_encoder`) and the source probe
  (`routes/admin/audio.rs`).
* **What it does:** `ps` and `/proc/<pid>/cmdline` show `rtsp://user:pass@camera/…`
  to any user of the host while the stream runs. Logs and the support bundle are
  redacted; the process table is not. The book says what to do about it
  (`docs/book/admin/audio.md`: a read-only camera account, `hidepid=2`).
* **What was found, and corrected:** the book and the previous hand-off said ffmpeg
  has no way to read its input address from a file. That is wrong. The `concat`
  demuxer reads a list from a file and opens each `file` in it as an input, RTSP
  included, with `option` lines after it for the demuxer's own options.
  **PROVEN on ffmpeg 6.1.1**, with `tools/rtsp-probe/rtsp_auth_server.py` as the
  camera (it answers `401` until it gets the right Basic credentials): with
  `ffconcat version 1.0 / file 'rtsp://viewer:…@127.0.0.1:18554/live' / option
  rtsp_transport tcp` in a mode-0600 file, run as `ffmpeg -f concat -safe 0
  -protocol_whitelist file,rtsp,tcp,udp,rtp,crypto -i list.ffconcat …`, the server
  logged `DESCRIBE auth=ok`, the captured audio was identical to the direct
  command's (32,320 bytes, `cmp` equal), and the process's command line held the
  list file's path and no password. A wrong password was `401` on both routes.
* **Why it is open:** the technique is not the work. Capture would have to write the
  list file (mode 0600, in a directory only the service user can enter, escaping a
  quote in the address as `'\''`) before each start, remove it after the process
  has opened it, and carry the stall timeout and the transport as `option` lines
  because options before `-i` now apply to `concat`, not to RTSP; the live view,
  HLS and the probe open the address themselves and would need the same. It has
  not run against a real camera, only against this server (TCP transport, one
  audio stream); UDP transport, a camera that sends video as well, and restart on a
  dropped stream are untested through `concat`. That is a change to the capture
  path with its own gates, not an edit to find in a debt sweep.

## TD-6 Fifteen page partials and one page are routed and nothing links to them

* **Where:** `/pages/disk-status` (`routes/pages/health.rs`), `/pages/history-dates`
  (`history.rs`), `/pages/today-lock` and `/pages/today-unlock` (`today.rs`), and
  `/pages/stats`, `/pages/hero-status`, `/pages/species-list`, `/pages/hourly-chart`,
  `/pages/daily-chart` and `/pages/most-recent` (`routes/pages/dashboard/mod.rs`).
  Added 2026-11-24 from the audit's UIC-05: `/pages/today-relabel` (`today.rs`, a
  write behind the sign-in with no control that posts to it), `/pages/confidence-chart`
  and `/pages/activity-heatmap` (`dashboard/mod.rs`), `/pages/life-accumulation`
  (`life_list.rs`), `/pages/ts-heatmap` (`timeseries_dash.rs`, which says it keeps
  the route though the page no longer loads it), and the full page
  `/player/{filename}` (`audio_player.rs`), which no page links to but the manual's
  web API reference documents. (`/pages/ts-sessions`, which the Trends card loaded,
  was deleted with that card under UIC-07 and is not on this list.)
* **How it was found:** the one the previous hand-off named,
  `/pages/quarantine-pending-count`, was deleted (no nav entry ever called it, and
  the Quarantine page and the Today strip show the same number). A scan for every
  `/pages/…` path registered with `.route(` that appears nowhere else in
  `crates/birdnet-web/{src,templates,static}` outside a comment or a test found these
  ten (the heading said nine; the list names ten, recounted 2026-10-21). It counts a path in a Rust string, a template attribute or a script, so a
  URL built from pieces (`"/pages/" + name`) is invisible to it.
* **Why they are open:** unlike the badge, each is covered by tests (the dashboard
  partials by a dozen) and several are named as parity features in
  `FEATURE_GAP_ANALYSIS.md` (`hero-status`, `hourly-chart`, `most-recent`). Whether
  they are an old dashboard that Today replaced, or an HTMX surface for something
  outside this repository (a kiosk, an embed), is a decision about what the
  interface promises, not something to settle by deleting what a grep did not find.

## TD-7 BirdNET-Pi's protected clips are parsed and nothing uses the result (closed 2026-10-21)

* **Closed by:** commit `486e775`. `protected_names` is now called through `clips::read_protected_list` and
  `protect_imported` (`crates/birdnet-migrate/src/clips.rs`), from the browser's clip import
  (`routes/admin/migration/clip_import.rs`) and the CLI (`src/helpers/migrate_cli.rs`): an imported clip on
  BirdNET-Pi's `disk_check_exclude.txt` gets `is_locked = 1`, which the disk manager honours. Evidence:
  `git grep protected_names` finds those callers; `protected_clips_are_locked` (4 tests) and
  `a_protected_clip_survives_the_purge` (2, with an unlocked control) were run by their author.
* **Still open about it:** the new "Where BirdNET-Pi is installed" field was not rendered in a browser, and
  the CLI has no flag for that folder (it discovers the list beside the recordings).

## TD-8 The clock check reads "cannot tell" as "recovered"

* **Where:** `src/integrations/station_health.rs`: `check_clock` and `clock_condition`.
  `clock_condition` returns no condition when the system answers neither synchronised nor
  unsynchronised (`NtpState::Unknown`), and `transitions` treats a key missing from a poll as
  recovered.
* **What it does:** when `timedatectl` does not answer in time (`TIMEDATECTL_TIMEOUT`, ten
  seconds), an open "Station clock is not synchronised" episode closes with "Station health
  recovered". The next poll that does answer re-opens it, after the debounce. A station whose
  clock is really unsynchronised and whose `timedatectl` hangs now and then therefore alerts and
  recovers in turn. This is the TD-1 shape (a check that could not look is read as a check that
  found nothing); the database and disk reads that had it are fixed (`Findings`, `Unchecked`).
* **Evidence:** read at `clock_condition` and `transitions`; not reproduced with a test.
* **Why it is open:** `Unknown` is also the permanent answer in a container with no systemd,
  where `NtpState`'s documentation says the clock is the host's problem. Reporting it as "could
  not look" would list the clock as unchecked on every such station for ever, so the fix needs a
  decision first: whether a probe that can never answer is "unchecked" or simply "not
  applicable here", and how the API says which.

## TD-9 A relabelled detection's comments follow it; they do not say what they were written under

* **Where:** `crates/birdnet-db/src/migration.rs`, migration 65 (`detections_relabel_carries_comments`),
  and `crates/birdnet-db/src/detection_comments.rs`.
* **What it does now:** since `0f4bf36` (MSC-22 of the 2026-11 audit) a comment moves with its detection
  when the detection's `Sci_Name` changes, so the relabelled detection's page shows its thread. Before, the
  thread stayed under the old name and was on no detection's page. Gate:
  `a_comment_follows_its_detection_through_a_relabel`, seen failing first.
* **The decision:** this entry used to say that whether a comment follows the new name, stays as history,
  or is shown on both "is a decision about meaning, not a key change". The batch of 2026-11-24 took the
  first option, and the maintainer, asked afterwards, left the choice to it: a comment is a note on one
  recording, keyed by its date, time and clip, so it stays beside that recording. Reviews do the opposite
  on a relabel (`detections_relabel_detaches_review`, migration 59) because a verdict is a claim about the
  species. To reverse it, drop the migration-65 trigger in a new migration.
* **What remains:** a comment with no clip (`file_name` NULL, written before migration 62) is not moved,
  and a moved comment does not record the species it was written under.
