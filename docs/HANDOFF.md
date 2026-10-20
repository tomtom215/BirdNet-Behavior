# Handoff

What only the maintainer can do, what the last working session did and did not
verify, and where the next one should start. Facts about GitHub were read on
**2026-10-20** with read-only calls (open pull requests, workflow runs on `main`)
and on **2026-10-14** (repository settings, not re-read); read them again before
relying on them.

The working branch is `claude/practical-dijkstra-flz0c8`, 89 commits on top of
`main` (`a48b3803`, the merge of #262), 334 files changed (counted at `19641a9d`, before
this file's own commit). **No pull request has
been opened from it**, so no `pull_request`-triggered workflow has seen any of it
(see section 4).

## 1. Repository settings only the maintainer can change

These are settings, not files, so no commit can make them true. They were read on
2026-10-14 and **not re-read** by the last session.

| Setting | State on 2026-10-14 | Why it matters | Where |
|---|---|---|---|
| Private vulnerability reporting | **off** | `SECURITY.md` and the issue-template chooser both send reporters to *Report a vulnerability*, which does not exist while this is off | Settings → Code security → Private vulnerability reporting |
| Dependabot alerts | **off** (the API answers 403) | the weekly `cargo audit` / `cargo deny` run and the scheduled OSV scan are the only advisory monitors | Settings → Code security → Dependabot alerts |
| Protection of `main` | **none**: no required checks; the one ruleset, `protect-main`, is `disabled` | nothing stops a push that fails CI | Settings → Rules → Rulesets: enable `protect-main`, add the checks listed in [`.github/workflows/README.md`](../.github/workflows/README.md) (mind its note on path-filtered workflows) |
| Annotated release tags | 20 of the 23 tag refs were lightweight | `RELEASING.md` prescribes `git tag -a`; nothing checks it | use `git tag -a` (or `-s`) |
| Repository licence detection | `license.spdx_id` is `NOASSERTION` | `LICENSE` is a summary pointing at `LICENSE-UPSTREAM`; GitHub cannot identify it | replace `LICENSE` with the full CC BY-NC-SA 4.0 text and move the lineage paragraph to `NOTICE`, if that is acceptable to you |

Not readable from the session, so unknown: Actions permissions, environments,
CodeQL default setup, secret scanning, who has write access.

**Open pull requests on 2026-10-20**, all from Dependabot: #228 (argon2 0.6) is
superseded, since `main` already carries argon2 0.6 from #262; #200 and #199
(`actions/checkout` 7.0.1, `actions/cache` 6.1.0) are superseded by this branch,
which pins both; #263 (`github/codeql-action` 4.39.0 and `taiki-e/install-action`)
is also already pinned here. Close all four once the branch is merged.

**CI on `main` (`a48b3803`), read from the Actions run list, not from logs:** every
workflow that ran on the merge of #262 finished `success` except **Mutation
testing**, whose shard `cargo-mutants on audio/extraction/extractor.rs` failed at
its step *Fail if too many mutants survived*. The scheduled run on the commit
before it failed too. The surviving mutant is `extractor.rs:223`, `!=` to `==` in
`Extractor::extract_detection_clip`. This branch carries a test written for it
(`3b5185cb`: a WAV clip with a frequency shift must come out shifted), which its
author saw fail against the mutant applied by hand; whether the `cargo-mutants`
shard now passes is **not known** until the workflow runs on the branch (the
frequency-shift rewrite touched the same function, so the line may have moved). The scheduled OSV scan failed on 2026-10-17
to 19 and passed on 2026-10-20, after the `duckdb` bump cleared the `zip`
advisories it named.

## 2. Decisions that are the maintainer's

- **The default classifier is now BirdNET V2.4, not the V3.0 preview.** This was
  the session's own call, made so that "install it and see what BirdNET-Pi saw"
  holds without customisation, and it is reversible. A new install, a new Docker
  volume and `quickstart.sh` get V2.4 with its v2 range filter, `CONFIDENCE` 0.70,
  `SENSITIVITY` 1.25 and the human-sound rule on at rank 10. The model, the 0.70,
  the 1.25 and the rank are BirdNET-Pi's own defaults; its range filter is v1,
  which is hosted nowhere, so v2 is what is installed. `--classifier birdnet-v3` or
  `BIRDNET_CLASSIFIER=birdnet-v3` chooses the preview. An existing station never
  changes model: its `birdnet.conf` decides, and a Docker volume that holds the
  V3.0 file keeps it. The switch of what a new install gets is isolated in the merge
  `1ce00bff` (installer, Docker entrypoint, compose files, `quickstart.sh`) and the
  docs commit `1cc46c51`; the 0.70 and rank-10 defaults are separate code commits,
  so reverting the classifier choice does not undo them. The places that name the
  default are `installer/lib/52-classifier.sh`, `select_classifier` in
  `docker/entrypoint.sh`, the compose files, the CI model fetch, the README and the book.
  `docs/book/guides/parity.md` has the measured comparison.
- **Where the default model files come from.** The binary fetches V2.4 from
  `huggingface.co/tphakala/BirdNET-v2.4` and the range filter from
  `raw.githubusercontent.com` (`tphakala/birdnet-go`), each at a pinned revision
  and checked against a SHA-256 compiled into it. A default install therefore
  depends on one third party's hosting. `publish-model.yml` has a `classifier`
  input that mirrors them into a `models-v2.4` release of this repository; **it
  has not been run and nothing fetches from it**. Running it and making the
  catalogue name the mirror first is yours to decide.
- **Publishing BirdNET-Pi's default range filter (MData v1) as ONNX.** It is
  hosted nowhere. `tools/model-convert` converts it and v2 (max difference 3.7e-6
  from TFLite over 1,344 calls); the catalogue carries v2 only. Hosting v1, and
  under what licence terms, is yours.
- **Accept, amend or drop [`STABILITY.md`](../STABILITY.md).** Still marked
  *proposed*. Its route counts were updated on 2026-10-20 (the OpenAPI document now
  has 96 paths and 98 operations); its other numbers were last re-measured on
  2026-10-14.
- **Publishing to crates.io.** The licence is CC BY-NC-SA 4.0 because the work
  derives from BirdNET-Pi. Whether to publish, under which names, and who holds the
  account is not something the repository can decide. See
  [`RELEASING.md`](../RELEASING.md#publishing-to-cratesio-the-maintainer-not-a-workflow):
  crates.io rate-limits new crates to a burst of five and then one per ten minutes
  (so nine crates take two batches), `cargo install` needs `--locked`, and
  `birdnet-core`'s tarball carries the bird-name table (`data/l18n/`) whose
  Wikipedia-derived names are CC BY-SA 4.0 and whose provenance per name is not
  settled (`NOTICE`, `crates/birdnet-core/data/l18n/README.md`).
- **Release signing.** Tags and `SHA256SUMS` are unsigned; the in-app updater
  trusts the GitHub release.
- **Where the Docker quickstart points.** The bare-metal one-liner names the
  release's `install.sh`; `quickstart.sh` and the three compose files it
  downloads are still fetched from `main`, as are `scripts/hardware-test.sh` and
  `packaging/macos/verify-macos.sh`. Moving them means making them release assets,
  a `release.yml` change that cannot be tested before a release carries them.
- **Several stations writing to one store.** You asked for it; the design is
  [`docs/design/multi-station.md`](design/multi-station.md) and **nothing in it is
  built**. Five decisions block Phase 0: who the stations are (one owner, or
  contributors), where the hub runs, the default clip policy, the scale target, and
  one hub or a tree of hubs.
- **Whether the first-run wizard should demand a setup code.** A station whose
  admin account has no password now logs one warning at start-up, but the first
  browser to finish the wizard still chooses the password. Requiring a one-time
  code printed in the log trades first-run friction for closing that window.
- **Product calls made by agents that you may want reversed:**
  confidence colours use *borderline* for anything under the station's cut-off plus
  ten points (a judgement, not a calibration: the score is not a hit rate);
  the alert-rule cooldown is entered in minutes, 5 by default; the Privacy Rank
  field accepts 75 as a valid rank (BirdNET-Pi's 1.25 × 60) and refuses fractions;
  `BIRDNET_CORS_ALLOWED_ORIGINS=*` is dropped with an error rather than honoured (a
  wildcard would let any website read the API; honouring it is a one-line change);
  a feed address's `?base=` is honoured only for the request's own host or
  `BNB_PUBLIC_URL`/`BNB_BASE_URL`, so a proxy that rewrites `Host` needs
  `BNB_BASE_URL`; the occurrence page shows the station's position rounded to
  0.1°; the daemon treats an unreadable species list as empty (logged), while the
  pages refuse to; the doctor's data-volume check does not flag an anonymous
  Docker volume (a bare `docker run` of the shipped image gets one, and
  `docker rm -v` deletes it); accessibility frequency shift needs ffmpeg 4.3 or
  newer (`afreqshift`) and the sox fallback is gone.

## 3. What the last session changed, and how far it was checked

The CHANGELOG's *Unreleased* section has an entry for each theme below, with what
its author observed. The themes, in the order the work landed:

- *Same audio, same detections.* V2.4 is installable and pinned; the default and
  its parity harness (`tools/parity`, goldens from BirdNET-Pi's own code on 14
  clips); `CONFIDENCE` 0.70; the human-sound rule by rank (`PRIVACY_RANK`); the
  V2.4 label names BirdNET-Pi stores; `SENSITIVITY` as BirdNET-Pi maps it.
- *Honest defaults and failures.* BirdWeather is never sent a position the station
  does not have; the support bundle coarsens coordinates; a password changed on the
  Accounts page survives a restart; a station with an open admin says so in the log;
  API routes and admin pages that published a failed read as "nothing" now say it
  failed (each is a CHANGELOG entry).
- *Hardening.* The session cookie's `Secure` follows the request; private pages are
  not kept by the service worker; feed links cannot be steered; camera passwords
  stay out of the live-stream log; a CORS wildcard no longer aborts the service.
- *Features.* A Privacy Rank field, an alert-rule cooldown field, "Why isn't a bird
  heard here?" (page and API), a doctor check and a banner for data on a
  container's own layer, accessibility frequency shift as a true shift in hertz.
- *Interface.* One confidence rule and key; one token set held by gates; the header
  badge and the Health page read one list; chart palette and legends; phone feed
  names; focus shape; tabular figures; layout shift.
- *Release engineering.* Versioned docs from `main` and tags, per-crate licence
  files and a checker, action re-pins, the installer and Docker V2.4 selection with
  upgrade safety, a mutation test for the extractor, and a CI step that lints the
  build without default features.

**Checked on the final tree (`19641a9d`, the dependency update; the commit after it
only adds this file), by commands the session ran:**

- `cargo fmt --check --all`; `cargo clippy --workspace --all-targets -- -D warnings`
  with default features, with `--no-default-features` and with `--all-features`;
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
  --document-private-items --all-features`: all exit 0. The rustdoc run found four
  broken intra-doc links the earlier gates had not; they are fixed.
- **Tests:** every test target of every crate, run in disk-sized batches
  (257 result lines: 5,237 passed, 0 failed, 9 ignored, no cargo invocation exiting
  nonzero, on the tree with the updated dependencies). `cargo test -p
  birdnet-behavioral` and `-p birdnet-timeseries` with `--features analytics`, re-run on the final tree: 220
  passed, 0 failed, 1 ignored. The runs found,
  over the session, six failures that neither agent branch showed alone and two test
  targets that did not compile without their feature (all fixed, each commit says
  how it was seen failing; the two targets carried the defect on `main`).
- **Real models, with `BIRDNET_REQUIRE_MODEL` and `BIRDNET_REQUIRE_V24_MODEL` set
  so a missing file is a failure:** the root crate's end-to-end set (18 tests),
  `birdnet-core`'s geomodel file (4), the V2.4 label-order test (3) and the new
  range-filter tests (7), all passing. The model files' SHA-256 matched the
  catalogue's pins for the V2.4 model, its range filter and the V3.0 model; the V3.0
  geomodel's digest is not in `model_catalog.rs` and was not compared.
- **Parity, re-run on 2026-10-20:** `tests/parity_dump.rs` over the 14 clips
  against BirdNET-Pi's goldens (`compare_rust.py`, V2.4 fp32 ONNX against BirdNET-Pi's
  FP16 TFLite): chunking equal on 13 of 13 clips BirdNET-Pi chunks; the same
  species in the same chunks at `CONFIDENCE` 0.7 on 12 of 13; the privacy mask
  equal on 13 of 14 (the exception is the synthetic digital-silence clip); the
  largest logit difference 1.75. The script exits 1 because a few top-1 labels on
  silence and speech differ; `docs/book/guides/parity.md` lists them and these
  numbers are the ones it states.
- **Browser, on a fixture built from the final tree:** axe over both viewports and
  both themes, 0 violations; interactions 79 passed, 0 failed (the fixture's
  `hls/default/start` answers 503 and the run passes; as at baseline); legibility,
  530 chart labels on 43 routes, none under 8.5 px; layout, 168 page states, 0
  findings; the four measured gates now in `a11y.yml` (tokens 61 checks, reduced
  motion 43 routes, focus 2,502 stops, figures 1,225), all clean; the strict sweep,
  172 screenshots, 0 pages with issues; the base-path gate, pass. I read the new
  page's screenshots at 1440 and 390 px; the phone capture shows the fixed tab bar
  mid-page, which is a full-page-capture artifact (checked: it is `position: fixed`
  on the new page and on an existing one).
- `typos`, `check-book-links.py` over a fresh `mdbook` render, the generated CLI
  help (no diff), `gen-third-party-licenses.py --check` (389 packages) and
  `check-crate-licences.py`: clean.
- **Wave 3.** Verified by my full re-run (the tests, lints and browser gates above,
  on the final tree), and I read the production diffs of `today.rs`, `toast.rs`,
  `quarantine.rs`, `accounts.rs`, `validate.rs` and the Segment Duration markup; I
  did not read the other wave-3 production files (`recordings.rs`, `handler.rs`,
  `station_tabs.rs`, `provenance.rs`, `onboarding.rs`) line by line, and the mutation
  results in the agent's commit messages were not re-run by me. What it changed:
  the failed-read fixes (24 tests in `a_defaulted_read_is_not_published_as_fact.rs`,
  each with a counterpart for a working and an empty station), the settings bounds
  (`segment_duration`, the recording window, clip format, info site, and the API
  running the page's whole check list), and the escaping audit. Two things I found
  by reading its report and fixed with a test seen failing first: the Segment
  Duration input allowed 5 to 60 where the validator allows 3 to 600 (htmx would
  have refused to submit the form on a valid value of 120), and a fixed recording
  window that ends before it starts, which the scheduler ignores, is now a
  `--doctor` warning for a station that has one in `birdnet.conf`.

**Not run, anywhere:** Firefox and Safari; a screen reader; a Raspberry Pi (the
parity numbers are an x86 ONNX build against BirdNET-Pi's FP16 TFLite; the memory a
second copy of the range model takes on a Pi 4B was not measured); Docker (no
daemon here, so the mount-table fixtures are modelled on Docker's output, not
captured); systemd (`installer/test/upgrade-e2e.sh` needs a real systemd host);
the new steps in `a11y.yml` in GitHub Actions (the YAML was parsed, not run); the
release workflow as a whole; `redocly lint` (not installed here and not in CI); a
verifying `cargo package` of any crate (it needs a full second build, including the
bundled DuckDB); `cargo-mutants` (not installed here); `cargo publish`; the V3.0
geomodel variant of the new range-filter tests.

## 4. Where to start

1. **Open a draft pull request** from `claude/practical-dijkstra-flz0c8` and read
   what CI says. Expect the jobs that only run on a pull request, and the four
   new `a11y.yml` steps, to be the first to run something new. `cargo-mutants`
   runs in pull-request mode (`--in-diff`), which mutates only changed lines.
2. Do the settings in section 1; close #228, #200, #199 and #263 when it merges.
3. Decide the defaults in section 2, in this order: the V2.4 default (the only
   one that changes what a new user sees), the model hosting, the multi-station
   decisions.
4. Read the open items below; each is real and none was started unless it says so.

## 5. Open items

Not done, in the order they seem to matter:

- **The parity audit's remaining rows** ([`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md)
  Part 8): 4 shipped, 7 partial, 11 gaps. Of the gaps, the larger ones are browser
  playback gain and filters (P-8), opt-in iframe embedding (P-9), MQTT client
  certificates (P-10), the dynamic-threshold list and reset routes (P-11), a
  per-species minimum interval (P-12), a dashboard custom image (P-13), a weather
  JSON API (P-21), temperature and clock-format settings (P-16), spectrogram
  options (P-17), Opus and AAC export (P-18) and an AviCommons image provider (P-19).
  Also open from earlier parts: the interface text is English only, voice-activity
  detection, a page to edit trusted proxies, native non-WAV clip export, and
  birdnet-go's `range/heatmap`.
- **Settings that take effect without a restart** (P-7): 47 of the 67 entries in
  `SETTING_SPECS` are overlaid onto the config and take effect on the next start
  (counted by parsing the table). Whether each of the other 20 is read per use or
  once at start was not checked.
- **`#[allow]` attributes without a reason:** 430 of 438 (counted by a parser over
  `crates`, `src` and `tests`), mostly numeric cast lints (`cast_precision_loss`
  201, `cast_possible_truncation` 189, `cast_sign_loss` 129). Adding a truthful
  reason to each needs reading each site; a mass edit would invent reasons. Six
  `TODO(O-nn-followup)` markers remain, each tied to an audit row.
- **The Unreleased CHANGELOG is 121,915 bytes.** `release.yml` warns above
  120,000; GitHub caps a release body at 125,000 characters. Entries added after
  this must be short, or older ones condensed.
- **Public pages that scan the whole history are still slow one at a time**, and
  **unescaped reflection can still inject non-script markup** (the CSP nonce goes
  only onto sealed tags, and any other `<script` is rewritten to text; an `hx-post`
  attribute is a different matter): the long accounts of both are in
  `docs/book/field/hardening.md` and `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`;
  neither was worked on this session.
- **Failed reads still shown as empty, found by the last sweep and not fixed**
  (each is outside code that session was changing; severity is the agent's, from
  reading): the species page hero says "No recording captured for this species
  yet" when its query failed (`species_pages.rs` ~1131/1143, medium); the Capture
  tab's source list and chips read empty and 0/0 (`admin/audio.rs` ~198, medium);
  "No per-species thresholds configured" on a failed read
  (`admin/species/handler.rs` ~301, medium); the last scheduled backup reads as
  never run (`admin/backup_recovery.rs` ~122, low-medium); listen-default and count
  chips show 0 (`admin/audio.rs` ~220/708/1019, low); the image blacklist check
  fails open and serves a blacklisted image (`images.rs` ~232, low); the live source
  picker and the wizard's source list read empty (`recordings.rs` ~575,
  `onboarding.rs` ~170, low); `livestream.rs` ~810 answers 404 where 503 is true
  (low); `capture_outage` reads green "recording" if its freshness read fails during
  the first seconds after a start (low). The sweep (a script kept in the session's
  scratch, not in the repository) found 188 hit-lines and classified 61 as handled,
  34 decoration, 29 request parameters, 16 filesystem or clock, 13 fail-closed, 9
  deliberate omissions from an exposition, 7 task panics (6 fixed), 1 legitimate
  zero and the 15 above.
- **Blocking filesystem calls inside `async fn` bodies** (an approximate scan, not a
  `syn` parse): 11 call sites in 6 functions, and 5 more through synchronous helpers.
  On a page path: `species_image_file` (`std::fs::read` up to four times per
  avatar), `serve_recording` and `serve_spectrogram` (`canonicalize` twice each),
  and `fetch_clips`, which reads the whole recordings directory
  (`recording_basenames`, the largest single one). Rare: `download_backup`,
  `database_overview`, the HLS start. Extending
  `tests/database_work_stays_off_the_async_runtime.rs` to filesystem calls looks
  feasible (about 150 to 200 lines in the test and about ten production edits to
  `tokio::fs` or into the existing blocking closures); the risks are std method
  names colliding in the by-name fixed point, and callees in other crates.
- **Numeric settings with no bound in `validate()`** (the daemon parses them and
  silently falls back to the default on failure; its behaviour for each was not
  examined): `freq_shift_hz`, `pre_sunrise_offset`, `post_sunset_offset`,
  `notify_cooldown`, `deadman_hours`, `clip_retention_days`, `max_files_per_species`,
  `purge_threshold`, `extraction_length`, `rare_species_days`, `stream_retention_secs`,
  `stream_max_mb`, `email_smtp_port`, `email_cooldown_secs`, `clip_target_lufs`.
  `RECORDING_LENGTH` is a known key that nothing reads.
- **`--doctor` reports a config value outside its range as a warning, not a
  refusal** (`START_CRITICAL` holds four names: configuration file, web listen
  address, database directory, HTTPS), so a bad number leaves a station running on a
  value the daemon cannot use. Two comments that said the opposite were corrected.
  Whether some value errors should stop the start is a decision, not a defect.
- **Suggestions for `CLAUDE.md`, not added (the agents proposed them, and I did not
  edit that file on their say-so):** a full-page escaping test must probe with a
  payload `seal::redeem` does not rewrite (`<i>`, `<img onerror>`) and assert on the
  rendered region, because a typed `<script` is neutralised by the response pass
  whatever the page did (an audit of the existing tests found none relying on it; the
  new range-filter page's first test did); a BLOB in a TEXT column (`X'ff'`) makes
  `row.get::<String>` refuse it, which separates two reads of the same table in a
  failure test (`DROP COLUMN` is blocked by an index: drop the index first); an
  equality test between a form's bounds and a validator's holds for any grammar, so
  pin a table of answers too.
- **Smaller findings, not fixed:** `/pages/quarantine-pending-count` is routed but
  nothing links to it; the invite form posts a blank label as `""`; the occurrence
  page lists 100 species before "Show the next 100", about 4,900 px on a phone;
  RTSP credentials are on the `ffmpeg` command line (`ps` shows them; logs are
  redacted) and cannot be avoided with ffmpeg 6.1.1;
  the unit runs as the login user with `ProtectHome=read-only`, so that user's
  `~/.ssh` is readable by the service; the compose file has no `cap_drop` or
  `no-new-privileges`; the macOS formula carries an all-zero `sha256` (it fails
  closed).
- **Workflows that have not run as a whole:** `release.yml` (read against a real
  toolchain and fixed; not executed). The others have run on GitHub, most recently
  on the merge of #262. The release SemVer check accepts build metadata (`+…`),
  which a Docker tag cannot carry (read from the regular expression, not run).
- **Pins that are still floating:** the `ubuntu:24.04` container
  `install-smoke.yml` starts with `docker run` is a tag; `taiki-e/install-action`
  and `EmbarkStudios/cargo-deny-action` choose their own tool builds.
- **Dependencies.** `cargo update` on 2026-10-20 moved 20 packages (`ort` rc.14 to
  rc.15, `tokio` 1.53.3 to 1.54.0, `rustls-webpki` 0.103.16, the webpki roots, and fifteen
  smaller ones); it needed no source change, and the parity dump's logits are
  byte-identical before and after. `cargo update --dry-run --verbose` then lists four
  packages with newer releases that their parents hold back (`comfy-table`,
  `generic-array`, `matchit`, `psm`). `cargo-deny` and `cargo-audit` are not
  installed where this was done, so advisories were not checked locally; the
  `supply-chain` and `osv` workflows do. htmx is 2.0.11, the latest 2.x; npm also publishes a 4.0.0,
  which was not evaluated.
