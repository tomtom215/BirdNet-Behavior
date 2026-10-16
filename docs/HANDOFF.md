# Handoff

What only the maintainer can do, what the last working session did and did not
verify, and where the next one should start. Facts about GitHub are as read on
**2026-10-14** (settings) and **2026-10-15** (open pull requests) with read-only
calls; re-read them before relying on them.

## 1. Repository settings only the maintainer can change

These are settings, not files, so no commit can make them true. They were read on
2026-10-14 and **not re-read** by the last session.

| Setting | State on 2026-10-14 | Why it matters | Where |
|---|---|---|---|
| Private vulnerability reporting | **off** (`{"enabled":false}`) | `SECURITY.md` and the issue-template chooser both send reporters to *Report a vulnerability*, which does not exist while this is off | Settings → Code security → Private vulnerability reporting |
| Dependabot alerts | **off** (the API answers 403) | the weekly `cargo audit` / `cargo deny` run is the only advisory monitor; alerts would add a daily one | Settings → Code security → Dependabot alerts |
| Protection of `main` | **none**: `protected: false`, no required checks; the one ruleset, `protect-main`, is `disabled` | nothing stops a push that fails CI; no check is required | Settings → Rules → Rulesets: enable `protect-main`, add the checks listed in [`.github/workflows/README.md`](../.github/workflows/README.md) (mind its note on path-filtered workflows) |
| Annotated release tags | 20 of the 23 tag refs are lightweight commits; `v0.16.1`, `v0.17.0`, `v0.17.1` are tag objects | `RELEASING.md` prescribes `git tag -a`; nothing checks it | use `git tag -a` (or `-s`); `release.yml` does not yet refuse a lightweight tag |
| Repository licence detection | `license.spdx_id` is `NOASSERTION` | `LICENSE` is a summary that points at `LICENSE-UPSTREAM`; GitHub cannot identify it | replace `LICENSE` with the full CC BY-NC-SA 4.0 text and move the lineage paragraph to `NOTICE`, if that is acceptable to you |

Not readable from here, so unknown: Actions permissions, environments, CodeQL
default setup, secret scanning, who has write access.

Open pull requests on 2026-10-15, all from Dependabot: #261 (`taiki-e/install-action`
patch bump), #260 (rubato 4 → 5.0.1) and #228 (argon2 0.5 → 0.6), which this branch
supersedes (it takes both, with the API changes they need: close them once it is
merged), and #200 and #199 (`actions/checkout` 7.0.1, `actions/cache` 6.1.0), open
since 2026-08-09 and not looked at.

## 2. Decisions that are the maintainer's

- **The default classifier.** It is BirdNET+ V3.0 preview: a different network from
  the one BirdNET-Pi and birdnet-go run (V2.4), with different windows, calibrated
  probabilities and 11 560 species. The same audio does **not** give BirdNET-Pi's
  detections on it, by design. V2.4 is one command (`--install-model birdnet-v24`)
  and was measured against BirdNET-Pi's own code (see
  [`docs/book/guides/parity.md`](book/guides/parity.md)). If "install it and see what
  BirdNET-Pi saw" is the promise to people migrating, the default should be V2.4;
  the recommendation was to leave it and document it (done), because V3.0 is the
  newer model and the README now says which is which. Changing it touches every place that names the default model: the
  installer, the Docker entrypoint and compose file, the CI model fetch, the README
  and the book.
- **Publishing BirdNET-Pi's default range filter (MData v1) as ONNX.** It is not
  hosted anywhere. `tools/model-convert` converts it (and v2) and checks the result
  against the TFLite originals; the catalogue carries v2 only (a pinned birdnet-go
  file). Hosting the v1 conversion, and under what licence terms, is yours.
- **Accept, amend or drop [`STABILITY.md`](../STABILITY.md).** It is marked
  *proposed*. Every number in it was re-measured on 2026-10-14 and has **not** been
  re-measured since.
- **Publishing to crates.io.** The licence is CC BY-NC-SA 4.0 because the work
  derives from BirdNET-Pi. Whether to publish, under which crate names, and who holds
  the account is not something the repository can decide. See section 5.
- **Whether the bundled species names may be redistributed.** The 35 language
  tables behind *Common-name language* are BirdNET-Pi's `model/l18n/`, which its
  README credits to @patlevin and to Wikipedia without saying which name came
  from where. Wikipedia text is CC BY-SA 4.0; this project is CC BY-NC-SA 4.0.
  `NOTICE` and `crates/birdnet-core/data/l18n/README.md` give the credit and say
  the question is not settled here. It matters most before a crates.io release,
  because the 3.7 MB table ships inside `birdnet-core`.
- **Release signing.** Tags and `SHA256SUMS` are unsigned; the in-app updater
  trusts the GitHub release. A signing key and a verifier in the updater would
  close that, and the key is yours to create.
- **Where the installer one-liner points.** README, the book and the installer's
  own usage text all say `curl … raw.githubusercontent.com/…/main/install.sh | sudo bash`,
  and the admin "Check for updates" text does too. `main` can be ahead of the last
  release, and the release's own `install.sh` is the one the release workflow attests
  (`releases/latest/download/install.sh`). Switching is twelve places and a decision
  about what an install should track; nothing here consumes the attestation yet.
- **The local TLS certificate authority has no Name Constraints.** The CA that
  `--tls-mode self-signed` makes (`crates/birdnet-web/src/tls.rs`, `ca_params`) is
  valid for ten years and may sign for any name; its key is `0600` under the service
  user. Anything running as that user can mint a certificate for any site that every
  device which imported the CA will trust. A name constraint limited to the station's
  own names would close it, and would mean regenerating the CA when the names change.
  Not done: it changes what operators must re-import.

## 3. What the last session changed, and how far it was checked

Everything is on branch `claude/gallant-gates-txf2k5`, directly on `main`
(`e34fd55`), as 22 commits; no pull request was opened from it, so **no GitHub
Actions workflow has run on any of it** (the `pull_request`-triggered gates never see
an un-PR'd branch). Open a draft pull request first and read what CI says.

What the commits are, each with its red/green evidence in its message:

- *Same audio, same detections* (V2.4 against BirdNET-Pi): `SENSITIVITY` maps as
  BirdNET-Pi's does; the chunk tail, resampler delay and clip rounding follow it;
  V2.4 and its range filter are installable and pinned; a parity harness
  (`tools/parity`) and three Rust tests diff against BirdNET-Pi's own code on 14
  clips; the Docker entrypoint no longer overwrites a user's own model files.
- *Security*: a hostile BirdNET-Pi database, CSV or backup header cannot exhaust the
  station; two public pages no longer reflect a URL value as markup (and a sweep gate
  covers the class); `--apply-config` is not steerable through a planted symlink; a
  webhook's custom header stays on its origin; the Docker quickstart has an admin
  password and a private `.env`; argon2 0.6 and rubato 5.
- *Correctness*: the third-octave meter's edges, the retention query's cost (74 s →
  6.65 s on 300 000 rows, identical output), clip export bit for bit, the species
  whitelist with BirdNET-Pi's meaning, htmx 2.0.11, `THIRD_PARTY_LICENSES.txt`
  regenerated (it was stale against the dependency tree).
- *Interface*: the design-token set, a layout gate, and the visual fixes the gate and
  a screenshot review found.

How far each was checked is in [`CHANGELOG.md`](../CHANGELOG.md) under
*Unreleased*, entry by entry. What follows is the last session's own run, on the tree
at the head of the branch.

- **Static:** `cargo fmt --check --all`; `cargo clippy --workspace --all-targets -- -D warnings`
  with default features and with `--all-features`; `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps --document-private-items --all-features`: clean. (The
  `--no-default-features` clippy build, which an earlier handoff listed, was not run.)
- **Tests**, from one scripted run (`final-gate.sh`, kept outside the repository) that
  deletes regenerable executables between batches, because the disk allowance does
  not hold a workspace's worth at once. All 27 steps of the last run exited 0. Summed
  from its log: 229 test executables, 5,044 tests passed, 0 failed, 5 ignored, plus
  12 doctests passed (6 ignored). The run was made on the code of `af1967f` (the
  heavy-read gate); `ccb06b9` after it changed one documentation file. An earlier run
  of the same script, before that change, had one failure,
  `every_action_written_is_one_this_list_documents` (two new whitelist audit actions
  undocumented), fixed in `4fe4e54`. Not run: `cargo test --workspace --all-features`
  as one build (the feature sets were exercised per crate: the root and web crates and
  `birdnet-behavioral` with `analytics`).
- **Real models**, with `BIRDNET_REQUIRE_MODEL` and `BIRDNET_REQUIRE_V24_MODEL` set so a
  missing model is a failure, not a skip: BirdNET+ V3.0 preview with its geomodel, and
  V2.4 with the MData v2 range filter, all from files on the working machine (not
  re-downloaded by the gate). `inference_e2e` (2), `pipeline_e2e` (3),
  `v24_matches_birdnet_pi` (1), `v24_range_filter_matches_birdnet_pi` (1) and the
  geomodel test in `birdnet-core` (2) passed; on the Eurasian Magpie recording V3.0
  stored 93.3 %, 92.3 % and 93.4 % in chunks 1, 4 and 7. `tests/parity_dump.rs` skips
  without its `PARITY_*` variables and was run against the BirdNET-Pi goldens earlier
  in the session (see `docs/book/guides/parity.md`), not in this run.
- **Browser**, on a fixture built from this branch: the full gates were run on the
  tree at `3e7ccd8` (everything but the clip-export, audit-list and handoff commits):
  axe over 38 routes × 2 themes × 2 widths with the WCAG 2.2 and best-practice rules
  blocking, 0 findings; interactions 74 passed (the fixture's `hls/default/start`
  answers 503 and the run still passes; not investigated); legibility, 0
  labels under 8.5 px; base-path; the strict sweep, 0 pages with issues, on a second
  run (the first reported all six photos on the life-list page failed, on a cold
  image cache; not investigated). The layout gate was run in full on the interface
  commit and, after the whitelist card was added, on the three pages it changed
  (axe and layout both clean). I looked at renders of the dashboard (desktop and
  phone), the species page and the Capture tab's species section. The browser
  gates were not re-run after `af1967f` (the heavy-read gate); it changed no
  template, stylesheet or script (checked with `git diff --name-only`), but it did
  change how the migration-stats cards and the feeds read the database, and the
  measurements for it are over HTTP, not in a browser.
- **Elsewhere:** the hermetic installer test suite passes; the versioned manual builds
  and every tree passes the link check; `gen-third-party-licenses.py --check` passes;
  `scripts/check-ci-config.py` fails only for the things this machine does not have
  (`cargo-mutants`, a GitHub token).

**Not run, anywhere:** the release workflow; pushing the Docker image; the
`cargo-mutants` jobs on any of this; the BirdNET-Pi FP16 model on a Raspberry Pi
(the parity numbers are against its FP16 TFLite model on an x86 machine); a verifying
`cargo package` of any crate (it was packaged with `--no-verify`, so the bundled
DuckDB build of a packaged crate was not run); Firefox, Safari, a screen reader,
and the compact and comfy density settings; anything on Raspberry Pi hardware;
`cargo publish`.

**Not verified, and why it matters less than it might:** the documentation images
(`docs/book/images/`) are from an older interface (the navigation says *Station*,
the status pills say *Healthy*). A regeneration by the visual-QA tooling was made and
**not kept**: the screenshot fixture publishes no audio source as up, so every
dashboard shot would show a *Not recording* pill. A regeneration needs the fixture to
publish a healthy capture gauge (`metrics().set_source_up`, and the daemon-running
flag) and a clock whose last detection is recent enough that no outage banner shows.

## 4. Where to start

1. Open a draft pull request from `claude/gallant-gates-txf2k5` and read what CI says.
   Expect the `third-party licence notices`, `CI config`, `a11y` (it has a new layout
   step) and the V2.4 model-fetch step in `ci.yml` to be the first to run something new.
2. Do the settings in section 1; close #260 and #228 when this merges.
3. Read the open items below; each is real and none was started unless it says so.

## 5. Open items

Not done, in the order they seem to matter:

- **crates.io.** Ready to publish, not published; the decision and the account
  are yours (section 2). What was done and what was not is in
  [`RELEASING.md`](../RELEASING.md#publishing-to-cratesio-the-maintainer-not-a-workflow):
  the bundled-DuckDB build of the packaged crates, docs.rs, and `cargo publish`
  itself were not run.
- **Public pages that scan the whole history are still slow one at a time, and the
  analytics store is one connection behind a mutex.** What changed in this branch is
  that they no longer starve everything else: the feeds, exports, insights and
  migration cards read through the pool with a heavy-read gate (CHANGELOG, *Fixed*),
  measured with sixteen in flight on a 300 000-row station: cheap reads 0.01–0.10 s.
  The scans themselves were not made faster. Timing every public GET on that station
  (debug build, one request at a time) found, in seconds: analytics retention 6.0,
  `rare.ics` 4.9, `rare.rss` 4.8, the detection export 4.5, its raven / birddb / ebird
  variants 4.2 / 4.1 / 2.5, `/station/data` 3.9, analytics species summary 2.4,
  dashboard KPIs 2.4, insights migration 1.9. `(Com_Name, Date)` has no index; that
  and the analytics mutex are the next two things to look at. 223 `with_db(` lines in
  56 files of `birdnet-web/src` still take the single writer (62 `with_read_db(`
  lines do not); `a_public_read_does_not_hold_the_writer_lock` covers ten routes,
  not those. With four readers the gate has two slots: a visitor holding both makes
  other heavy requests wait (the feeds are refused with 503), and the per-address
  rate limit and private mode are the only other limits on that.
- **Unescaped reflection still injects markup.** The CSP nonce now goes only onto the
  tags the server sealed, and every other `<script` is rewritten to text
  (`src/seal.rs`; CHANGELOG, *Security*), checked in Chromium for a full page and for
  an htmx-swapped fragment. What an unescaped reflection can still do is inject other
  markup, notably an `hx-post` attribute that htmx fires on load, same-origin, past
  the CSRF origin check. `a_query_value_is_never_echoed_as_markup.rs` sweeps GET
  parameters across the page layer; it does not cover POST bodies, JSON, or a value
  stored by one page and rendered by another. Client scripts were swept for DOM sinks
  (`innerHTML`, `eval`, …): the only one is the log viewer's event stream, whose
  producer escapes (`logs.rs`, with a test). Not tested in Firefox or Safari.
- **Smaller audit findings, not fixed** (read or run by the audit, not re-checked since): the staged upload of a migration file lives
  in the system temp directory (up to 4 GiB; with a tmpfs `/tmp` that is memory) and a
  WAL-mode source leaves `-shm`/`-wal` files beside it; RTSP credentials are on the
  `ffmpeg` command line (`ps` shows them; logs are redacted); the unit runs as the
  login user with `ProtectHome=read-only`, so that user's `~/.ssh` is readable by the
  service; the compose file has no `cap_drop` or `no-new-privileges`; the macOS
  formula carries an all-zero `sha256` (it fails closed); `/api/v2/soundlevel?source=`
  echoes its raw query value into its JSON (not HTML, and no client script reads it
  into the DOM).
- **Feature parity.** The gap analysis against birdnet-go and BirdNET-Pi is in
  [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md); Part 7 triages birdnet-go's
  last 37 commits and says which were read. Open there: the interface text is still
  English, pages carry `lang="en"` around names in other scripts, voice-activity
  detection, a page to edit trusted proxies, native non-WAV clip export, and
  birdnet-go's `range/heatmap`.
- **The browser sweeps grade a populated station.** axe, `qa.mjs` and `layout.mjs`
  run against a fixture that has detections, so a page's empty state is only reached
  where a fixture happens to be empty.
- **Workflows that have never run:** `codeql.yml`, `osv.yml`, `scorecard.yml` and
  `fuzz.yml` were adapted or written without a first run on GitHub.
- **Pins that are still floating:** the `ubuntu:24.04` container that
  `install-smoke.yml` starts with `docker run` is a tag (Dependabot does not read
  `docker run` arguments); `taiki-e/install-action` and
  `EmbarkStudios/cargo-deny-action` choose their own tool builds from the version
  named.
- **Dependencies.** `cargo update --dry-run` on 2026-10-15 locked nothing and
  listed four packages with newer releases that their parents hold back
  (`comfy-table`, `generic-array`, `matchit`, `psm`). htmx is 2.0.11, the latest 2.x;
  npm also publishes a 4.0.0, which was not evaluated.
- **The OpenAPI document is not complete** (`api.md` says so): eight REST paths
  and two WebSockets are routed but undocumented, and nothing checks that direction. `redocly.yaml` is
  committed but no workflow runs `redocly lint`.
