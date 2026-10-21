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
author saw fail against the mutant applied by hand. On PR #264 the shard passes,
but not by catching that mutant: the bot's comment on `c1fbea97` read one mutant,
none caught, none missed, one unviable, so the frequency-shift rewrite changed the
function until that mutant no longer compiles. The scheduled OSV scan failed on 2026-10-17
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
  `raw.githubusercontent.com` (`tphakala/birdnet-go`), each at a pinned revision and
  checked against a SHA-256 compiled into it, so a default install depends on one
  third party's hosting. **You approved mirroring them, and `publish-model.yml` was
  run on 2026-10-20** (run 39469063446, from this branch): the release `models-v2.4`
  holds `birdnet-v24.onnx` (54,068,648 bytes), `birdnet-v24-range-v2.onnx`
  (29,524,170) and `SHA256SUMS`. GitHub reports their digests as equal to the
  catalogue's pins and to the files the tests ran on (compared in full); the
  workflow's own sha256 and size check and its SLSA attestation step succeeded, but
  I did not run `gh attestation verify`. **Nothing fetches from it yet.** The
  catalogue has one URL per entry, so the dependency is unchanged until
  `model_catalog.rs` names the mirror first with the upstream as the fallback (the
  V3.0 model already works that way, in the installer's shell). That change touches
  the install path of every new station and was not made here. Two follow-ups:
  re-run the workflow from `main` after this merges, so the attestation names `main`
  and not a feature branch; then wire the fallback in the catalogue.
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

**In GitHub Actions, on draft PR #264:** at `47558b97` all 71 check runs had
finished, 69 `success` and 2 `skipped` (`deploy` and `Merge multi-arch manifest`,
which do not run on a pull request), none failed. That was tallied by a script from
the check-runs API, not read off the list. It includes the four new `a11y.yml`
steps, `cargo-deny`, `cargo-audit`, every `cargo-mutants` shard, the three upgrade
jobs (from v0.15.0, v0.16.1 and v0.17.0, each running
`installer/test/upgrade-e2e.sh` under real systemd on a GitHub runner), the
real-model inference job and CodeQL.
The first run, on `c1fbea97`, was not green. Three things were red, each in this
PR's own code, each fixed with the evidence in its commit message:

- Mutation testing, `validate.rs` shard 2/2: `h < 24` to `h <= 24` in `parse_hhmm`
  survived, because no test refused an hour of 24 on its own (`e641bb13`).
- CodeQL: 17 alerts, 15 at failure level (14 hard-coded cryptographic value, 1
  cleartext logging) and 2 warnings (section 5). Nine were on lines the PR added,
  among them the `pw(tag)` test helper, which carried its literal through `format!`
  so that every call site was an alert; six were on lines identical to main's
  (`26ff8f74`). The check passes at `47558b97`; its annotations went from 17 to 2.
- Mutation testing, `src/daemon/*.rs` shards 4/6, 5/6 and 6/6: five survivors in
  `alert_cooldown.rs` (the prune threshold) and `occurrence.rs` (cache expiry,
  `Debug`), which the model-gated tests cannot reach in a job that has no models
  (`47558b97`).

A full run takes about 45 minutes (Tests (x86_64) 45, the accessibility sweep 36),
and every push cancels the run in progress. A PR's CodeQL alerts can be listed
without a token from the check run's annotations,
`GET /repos/{owner}/{repo}/check-runs/{id}/annotations`; the code-scanning API
answers 403 without one, so what `main`'s Security tab lists is **not known**.

**Not run, anywhere:** Firefox and Safari; a screen reader; a Raspberry Pi (the
parity numbers are an x86 ONNX build against BirdNET-Pi's FP16 TFLite; the memory a
second copy of the range model takes on a Pi 4B was not measured); Docker (no
daemon here, so the mount-table fixtures are modelled on Docker's output, not
captured); the release workflow as a whole; `redocly lint` (not installed here and not in CI);
a verifying `cargo package` of any crate (it needs a full second build, including
the bundled DuckDB); `cargo publish`; the V3.0 geomodel variant of the new
range-filter tests.

## 4. Where to start

1. **Draft pull request #264 is open from `claude/practical-dijkstra-flz0c8`, and
   CI is green at `47558b97`** (section 3). It is a draft on purpose; marking it
   ready is the maintainer's call. `cargo-mutants` runs in pull-request mode
   (`--in-diff`), which mutates the lines the whole PR changes against `main`
   (`git diff origin/main...HEAD`), so code a fix adds is mutated too: the
   `is_fresh` helper that fixed one survivor brought mutants of its own.
2. Do the settings in section 1; close #228, #200, #199 and #263 when it merges.
3. Decide the defaults in section 2, in this order: the V2.4 default (the only
   one that changes what a new user sees), wiring the model mirror, the
   multi-station decisions.
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
- **Settings that take effect without a restart** (P-7): settled. Every entry in
  the settings page now carries an "Applies immediately" or "Applies after restart"
  badge from one table (`routes/admin/settings/applies.rs`), each entry naming the
  consumer that was read to decide it; a test fails for a field without a badge or
  an entry without evidence. A banner lists changes still waiting for a restart.
- **`#[allow]` attributes without a reason:** 419 of 429 attributes (732 lint
  entries; a `syn` parser over `crates`, `src`, `tests` and `tools`, counted on
  2026-10-21 after 24 were deleted and 19 trimmed because clippy, run with every
  named lint forced to `warn`, showed them silencing nothing; the same clippy
  diagnostics came out before and after for the seven non-web crates). The five
  numeric cast lints are 573 of the 732 entries (`cast_precision_loss` 200,
  `cast_possible_truncation` 184, `cast_sign_loss` 124, `cast_possible_wrap` 42,
  `cast_lossless` 23), 261 of them in `birdnet-web` and 182 in `birdnet-core`; 109
  entries are module-wide (`#![allow]`). **Proposal, not implemented:** allow the
  cast lints once in `[workspace.lints.clippy]`, the way `module_name_repetitions`
  already is, and delete the 573 entries; keep `reason = "…"` for every other lint.
  Not implemented because it silences casts in code that has no allow today, and
  the number of such sites was not measured. Not done: the dead-allow analysis for
  `birdnet-web` (376 of the 732 entries) and the root crate (43), because the
  `birdnet-web` lib-test target did not compile while another change was in flight;
  `suboptimal_flops` allows were removed where the workspace already allows it.
  The six `TODO(O-nn-followup)` markers are resolved (reworded to what the code
  does or recorded in `docs/TECH_DEBT.md`) and `tests/no_todo_ships_in_the_source.rs`
  refuses a new one.
- **The Unreleased CHANGELOG is 121,915 bytes.** `release.yml` warns above
  120,000; GitHub caps a release body at 125,000 characters. Entries added after
  this must be short, or older ones condensed.
- **Public pages that scan the whole history are still slow one at a time**, and
  **unescaped reflection can still inject non-script markup** (the CSP nonce goes
  only onto sealed tags, and any other `<script` is rewritten to text; an `hx-post`
  attribute is a different matter): the long accounts of both are in
  `docs/book/field/hardening.md` and `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`;
  neither was worked on this session.
- **Failed reads shown as facts:** the sweep is now a test
  (`tests/a_failed_read_is_not_defaulted_into_a_fact.rs`, a `syn` parse of every
  crate; its table is `tests/data/read_default_sweep.tsv`, one row per site, with a
  class and a gate that fails on an unclassified site and on a stale row). Fixed
  with a failing-first test each: the species page hero, per-species thresholds,
  the last scheduled backup, the image blacklist (now fails closed: a withdrawn
  picture is not served because the database did not answer), the live source
  picker, `/stream` (503 where the table is unreadable, 404 only for a missing
  row), the Today pill and outage banner, the BirdWeather and rule tests, share
  links, the Migration cards, sign-in, and the invite label. Still open, as
  `tracked:TD-n` rows with their evidence in `docs/TECH_DEBT.md`: TD-1 (a failed
  read inside the station-health evaluator ends an alert), TD-3 (the boot
  journal's detection count), TD-4 (the Capture tab's duplicate guard and the
  wizard's source list) and TD-6 (partials routed and never linked).
- **Blocking filesystem calls inside `async fn` bodies:** the gate
  `tests/database_work_stays_off_the_async_runtime.rs` now follows `std::fs`
  and `Path` stat calls (a `syn` parse with a fixed point over call names, in
  every crate); 42 calls in 28 functions moved to `tokio::fs` or the blocking pool.
  The gate's limits: it resolves calls by name, so a method that shares a name with
  a `std` one is on a deny list, `/proc` reads are exempt, and the one-shot
  command-line actions in `main` and the start-up calls in `serve` are exempt by an
  explicit list.
- **Numeric settings with no bound in `validate()`:** done for the fifteen
  (`validate::FIELD_BOUNDS`, read from what the daemon does with each). They are
  warnings in the file, not errors, because an error reverts the whole station to
  its last-good file; the settings page and `PUT /api/v2/settings` refuse them.
  `RECORDING_LENGTH` is read as `SEGMENT_DURATION` when that is absent or blank.
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
  pin a table of answers too. From the CI run on #264: a test helper that takes a
  literal and returns a string made from it (`pw("tag")` into `format!("{tag}-…")`)
  hands the literal to every caller, so a CodeQL taint query alerts at each call
  site; CodeQL's cleartext-logging query alerts on a variable *named* `secret` that
  an assertion message prints, whatever it holds; the mutation job runs unit tests
  only and has no model files, so a mutant in code whose tests need `BIRDNET_TEST_*`
  models survives unless a model-free test reaches it; and a real clock never
  equals a `Duration`, so put a time comparison in a function of its two durations
  to test the boundary.
- **Two CodeQL warnings:** `sw.js` now compares `event.origin` with its own
  origin before acting on `SKIP_WAITING` (`sw_harness.mjs` was seen failing for a
  foreign origin first). `qa.mjs` writes fixture data to the report file; it is a
  dismissal, with the reason written above the line. Whether CodeQL stops reporting
  either is not known until it runs.
- **The literal-password pattern:** `tests/no_literal_reaches_a_credential_parameter.rs`
  parses the tree and fails on a literal that reaches a parameter named `password`,
  `nonce`, `iv`, `salt` or `secret`; 62 sites were fixed with `format!` over the
  process id. That the CodeQL query agrees was not run here.
- **Smaller findings:** `/pages/quarantine-pending-count` is deleted (nothing
  linked to it); the blank invite label is stored as no label; the range-filter list
  shows 25 species and appends 25 more in place (about 1,800 px on a phone,
  measured, from about 4,900); RTSP credentials are still on the `ffmpeg` command
  line: ffmpeg 6.1.1's `concat` demuxer, fed a list file, accepted credentials
  without them appearing in `ps`, against a test RTSP server that required Basic
  authentication (`tools/rtsp-probe/`), but the station does not use it yet
  (TD-5);
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
