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
  baseline of nothing. `BootRecord::detections` is a `u64`, so there is no way to
  write "unknown".
* **Evidence:** PROVEN by reading `record_boot` and `birdnet_web::boot_journal`.
* **Why it is open:** it is a start-up edge (the database has to open and then
  fail one `COUNT(*)`), and the fix changes the persisted journal's schema
  (`Option<u64>`) and `record`'s comparison, which want their own compatibility
  tests against journals written by earlier versions.

## TD-4 The Capture tab's duplicate guard reads an unreadable source list as empty; the wizard logs it and says nothing

* **Where:** `crates/birdnet-web/src/routes/admin/audio.rs` (`create`: the check
  that refuses to add the same source twice, `.with_read_db(AudioSourceStore::list)
  .unwrap_or_default()`) and `crates/birdnet-web/src/routes/pages/onboarding.rs`
  (`add_source`).
* **What it does:** `create` reads the list with `.unwrap_or_default()`, so a list
  that could not be read passes the "already configured" check and the insert is
  attempted; on a database that cannot be read the insert fails too, so the cost is
  a duplicate only if the read fails and the write succeeds. The wizard no longer
  treats the list as empty: `add_source` matches on the read, logs "could not read
  the audio sources; the microphone was not added" at `error` and returns, but it
  returns `()`, so the page the person sees does not say that nothing was added.
  The other sites this entry first listed (`sources_body`, `listen_default`,
  `count_oobs`, `listen_default_id`) were rewritten by the setup-wizard change and no
  longer default their errors.
* **Evidence:** read at the cited places on 2026-10-21; the rows of class
  `tracked:TD-4` in `tests/data/read_default_sweep.tsv` (lines 33 and 134 to 137)
  are what the sweep still reports, and I did not re-classify the four wizard rows
  after the rewrite.
* **Why it is open:** the guard's cost is a duplicate row in a corner case, and the
  wizard's fix is a message on a page that has just been rewritten; neither was part
  of the change that found them.

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

## TD-6 Ten more page partials are routed and nothing links to them

* **Where:** `/pages/disk-status` (`routes/pages/health.rs`), `/pages/history-dates`
  (`history.rs`), `/pages/today-lock` and `/pages/today-unlock` (`today.rs`), and
  `/pages/stats`, `/pages/hero-status`, `/pages/species-list`, `/pages/hourly-chart`,
  `/pages/daily-chart` and `/pages/most-recent` (`routes/pages/dashboard/mod.rs`).
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

## TD-9 A detection's comments are addressed by date, time and species

* **Where:** `crates/birdnet-db/src/detection_comments.rs` (`list` selects `WHERE date = ?1 AND time
  = ?2 AND sci_name = ?3`, line 201; `detection_comments` has no clip column, migration 38) and
  `crates/birdnet-web/src/routes/pages/detection_comments.rs` (`thread_placeholder(date, time,
  sci_name)`).
* **What it does:** the thread on a detection's page is the thread of every detection that shares
  its date, time and species. Two microphones that heard one bird in the same second have two
  detections, and a comment on the one is shown under the other. A relabel leaves a thread under
  the old name. This is the shape reviews had until migration 59, with a smaller consequence: a
  comment withdraws nothing from any count.
* **Evidence:** read at the two places above; not reproduced with a test.
* **Why it is open:** the reviews change (migration 59) was already a table rebuild, a data
  decision about legacy rows and a change to every form that posts a verdict; comments are
  append-only records whose deletion is audited one by one, so re-keying them wants its own
  decision about what a comment on an ambiguous legacy triple becomes. The same key
  (`DetectionKey`: date, time, species, clip) is the one to use.

