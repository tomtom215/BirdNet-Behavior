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
- **Where the Docker quickstart and the maintainers' harness point.** The bare-metal
  one-liner (README, the book, the installer's usage text and error messages) names
  `releases/latest/download/install.sh`, the file each release checksums and
  attests, and the admin "Check for updates" text names the new version's own
  `releases/download/vX.Y.Z/install.sh`;
  `installer/test/install-one-liner-is-a-release-asset.sh` fails on a branch-pinned
  form. Still fetched from `main`: the Docker one-liner (`quickstart.sh`) and the
  three compose files that script downloads, `scripts/hardware-test.sh` (a
  maintainer's harness that fetches `main`'s installer and test recording on
  purpose) and `packaging/macos/verify-macos.sh`. Moving the quickstart means
  making `quickstart.sh` and the compose files release assets and having the script
  fetch the compose files from the same release: a `release.yml` change this
  session could not run, and one whose one-liner must not change until a release
  carries the assets (v0.17.1, the latest, has none, so it would 404).

## 3. What the last session changed, and how far it was checked

Everything is on branch `claude/gallant-gates-txf2k5`, directly on `main`
(`e34fd55`), as 35 commits before the one that carries this file; no pull request was
opened from it, so **no GitHub Actions workflow has run on any of it** (the
`pull_request`-triggered gates never see an un-PR'd branch). Open a draft pull request
first and read what CI says.

What the commits are, each with its red/green evidence in its message:

- *Same audio, same detections* (V2.4 against BirdNET-Pi): `SENSITIVITY` maps as
  BirdNET-Pi's does; the chunk tail, resampler delay and clip rounding follow it;
  V2.4 and its range filter are installable and pinned; a parity harness
  (`tools/parity`) and three Rust tests diff against BirdNET-Pi's own code on 14
  clips; the Docker entrypoint no longer overwrites a user's own model files.
- *Security*: a hostile BirdNET-Pi database, CSV or backup header cannot exhaust the
  station; the CSP nonce goes only onto tags the server sealed and every other
  `<script` becomes text (`seal.rs`); the local TLS CA is constrained to the station's
  names (verified with rustls's WebPKI verifier; devices import the new CA once);
  uploaded migration databases are staged on the data disk and their SQLite
  side files removed; the installer's downloads are https-only and the install
  one-liner names the release's `install.sh`, not `main`; two public pages no longer
  reflect a URL value as markup; `--apply-config` is not steerable through a planted
  symlink; a webhook's custom header stays on its origin; the Docker quickstart has an
  admin password and a private `.env`; argon2 0.6 and rubato 5.
- *Concurrency*: 117 `with_db` call sites that only read now use the read-only pool
  (the 55 that write are listed with their reasons in `writer_lock_sites.txt` and
  enforced by a test), a session's `last_seen` is rewritten at most once a minute,
  the analytics store has two read connections beside its writer, and
  `(Com_Name, Date)` has an index (migration 54) with a windowed rare-bird query.
- *Correctness*: the third-octave meter's edges, the retention query's cost (74 s →
  6.65 s on 300 000 rows, identical output), clip export bit for bit, the species
  whitelist with BirdNET-Pi's meaning, htmx 2.0.11, `THIRD_PARTY_LICENSES.txt`
  regenerated (it was stale against the dependency tree).
- *Interface and manual*: the design-token set; a layout gate (now also grading chart
  labels); every book image regenerated from a fixture built from this tree, the
  section called Settings in the manual as in the navigation (a test holds the two
  together), and the visual defects those pictures showed (chart labels colliding,
  a total written across its curve, a phone pushed sideways by a database path).
- *CI*: `release.yml`, `docker.yml` and `mutation.yml` were read against the tree and
  every part of them that does not publish was run; four defects fixed (the release
  gate's MSRV leg ran on stable, a dry run attested provenance for archives never
  released, the image workflow tagged `latest` on pre-releases and ran without the tag
  gate, and its manual run pushed by default). `scripts/check-ci-config.py` checks the
  toolchain rule that caused the first.

How far each was checked is in [`CHANGELOG.md`](../CHANGELOG.md) under
*Unreleased*, entry by entry. What follows is the last session's own run, on `df086a9`
and, for the whole of the gate below, again on `b81c3b3` (same counts) after the
lockfile update in the CHANGELOG's *Security* section. The commits after those change
documentation, workflow YAML, the shellcheck and spelling configuration and
`tools/visual-qa` only.

- **Static:** `cargo fmt --check --all`; `cargo clippy --workspace --all-targets -- -D warnings`
  with default features and with `--all-features`; `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps --document-private-items --all-features`: all exit 0. (The
  `--no-default-features` clippy build, which an earlier handoff listed, was not run.)
  The first run of the script on the commit before `df086a9` failed both clippy
  steps on a test helper's type complexity; fixed in `df086a9`, which is why the
  script was run again.
- **Tests**, from one scripted run (`final-gate.sh`, kept outside the repository) that
  deletes regenerable executables between batches, because the disk allowance does
  not hold a workspace's worth at once. All 28 steps exited 0. Summed from its log:
  236 test executables, 5,084 tests passed, 0 failed, 5 ignored, plus 12 doctests
  passed (6 ignored). Not run: `cargo test --workspace --all-features` as one build
  (the feature sets were exercised per crate: the root and web crates and
  `birdnet-behavioral` with `analytics`).
- **Real models**, with `BIRDNET_REQUIRE_MODEL` and `BIRDNET_REQUIRE_V24_MODEL` set so a
  missing model is a failure, not a skip: BirdNET+ V3.0 preview with its geomodel, and
  V2.4 with the MData v2 range filter, all from files on the working machine (not
  re-downloaded by the gate). They are inside the 28 steps above and passed there; the
  per-test counts were not re-extracted from that run. From an earlier run on a tree
  that differs from this one only in web, installer and documentation files:
  `inference_e2e` (2), `pipeline_e2e` (3), `v24_matches_birdnet_pi` (1),
  `v24_range_filter_matches_birdnet_pi` (1) and the geomodel test in `birdnet-core`
  (2); on the Eurasian Magpie recording V3.0 stored 93.3 %, 92.3 % and 93.4 % in
  chunks 1, 4 and 7. `tests/parity_dump.rs` skips without its `PARITY_*` variables and
  was run against the BirdNET-Pi goldens earlier in the session (see
  `docs/book/guides/parity.md`), not in this run.
- **Browser**, on a fixture built from this tree's web crate (a harness kept outside the
  repository runs the CI steps in order, with the base-path gate against a second
  instance under `/bn`): axe over 152 page states (38 routes × 2 themes × 2 widths)
  with the WCAG 2.2 and best-practice rules blocking, 0 violations; interactions 74
  passed, 0 failed (the fixture's `hls/default/start` answers 503 and the run still
  passes; not investigated); legibility, 455 chart labels over 39 routes, none under
  8.5 px; layout, 152 page states, 0 findings across its five checks; base-path
  pass; the strict sweep, 156 screenshots, 0 pages with issues. Earlier runs of the
  same gates on earlier trees of this branch failed once for each defect named in the
  CHANGELOG entry *The manual's images…* and *Names on the co-occurrence chart…*;
  both are fixed and the run above is the one after. I looked at the regenerated
  images: at full size, 25 of the 26 desktop ones (all but `today.png`, which is the
  dashboard's page) on an earlier generation of the same fixture, and after the last
  generation the contact sheets of all 26 desktop and all 24 phone images plus
  full-size views of `correlation.png` and crops of the life-list chart, the wizard
  and the phone gallery. Of the phone images only the dashboard, correlation,
  system-health and species-detail ones were looked at full size. Not run: Firefox,
  Safari, a screen reader, the compact and comfy density settings.
- **Elsewhere:** the hermetic installer test suite passes, including the new
  one-liner gate, and `installer/build.sh --check` agrees with `install.sh`;
  `gen-third-party-licenses.py --check` passes (389 packages); every internal link
  in the rendered in-app manual resolves (`scripts/check-book-links.py`);
  `scripts/check-ci-config.py` passes with `cargo-mutants` 27.1.0 on the path (it has
  no GitHub token here, so it did not read run durations). The versioned manual
  (`scripts/build-versioned-docs.sh`) was **not** rebuilt: there is no `mdbook`
  command-line tool on this machine.
- **cargo-mutants**, partly: 28 of the 281 mutants the workflow's six shards divide
  (everything in `crates/birdnet-core/src/audio/extraction/convert.rs`), run on a copy
  of the tree with `--lib` as the workflow does, in 11 minutes: 24 caught, 4
  unviable, 0 missed. The workflow itself has not run.

**Not run, anywhere:** the release workflow (read against a real toolchain and fixed;
not executed); building or pushing the Docker image (no daemon on this machine);
`cargo-mutants` over the other 253 mutants; the BirdNET-Pi FP16 model on a Raspberry Pi
(the parity numbers are against its FP16 TFLite model on an x86 machine); a verifying
`cargo package` of any crate (it was packaged with `--no-verify`, so the bundled
DuckDB build of a packaged crate was not run); anything on Raspberry Pi hardware;
the OIDC attestation, `ubuntu-24.04-arm` and macOS jobs; the install one-liner end to
end; `cargo publish`.

**Observed while rendering the manual's pictures, not changed:**

- The header badge reads *Healthy* over a Health page whose banner says *Needs
  attention* (two of the three fixture sources stalled). `grade()` in
  `routes/pages/health.rs` looks at the integrity check, whether *any* source is up,
  and one disk percentage; the Health banner (`station_health.rs::status_banner`) also
  lists scratch space, queued uploads, a flapping source, a species filter that admits
  nothing and a failing classifier. The badge is cheap on purpose (it polls every 30
  s on every page); making the two agree means sharing the issue list and deciding
  what that costs.
- The Today page's layout editor (*Today Page Layout*) exists only on the full
  `/admin/settings` page. The Settings tabs link there from one conditional hint on the
  Alerts tab and the admin menu does, so it is hard to find; the manual now says where
  it is. Putting a link on the General tab is the obvious change.
- The fixture seeds detections across the whole day regardless of the clock, so the
  pictures show a 21:55 detection on a page whose "now" marker is at 8 am. The
  dashboard's *recording* pill and *Healthy* badge are real states; the times are not.
- The layout gate cannot see a line drawn through a label (the life-list defect was
  found by eye), only labels over labels.

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
- **Public pages that scan the whole history are still slow one at a time.** They no
  longer starve everything else (the feeds, exports, insights and migration cards
  read through the pool behind a heavy-read gate; every other read in the web crate
  uses a read connection, and what is left on the single writer is listed with its
  reason in `crates/birdnet-web/tests/writer_lock_sites.txt` and
  `analytics_writer_sites.txt`, enforced by `the_writer_lock_is_for_writing`). The
  scans themselves are not faster except the rare-bird feeds (migration 54 and a
  windowed query: 4.6 to 1.5 s in the real binary). Timing every public GET on a
  300 000-row station before the index (debug build, one at a time): analytics
  retention 6.0, the detection export 4.5 and its raven / birddb / ebird variants
  4.2 / 4.1 / 2.5, `/station/data` 3.9, analytics species summary 2.4, dashboard
  KPIs 2.4, insights migration 1.9 seconds. The exports and KPIs are whole-history
  aggregates; whether an index or a maintained summary table helps was not
  examined. With four readers the heavy-read gate has two slots: a visitor holding
  both makes other heavy requests wait (the feeds are refused with 503), and the
  per-address rate limit and private mode are the only other limits on that.
  Throughput of concurrent heavy DuckDB queries was not measured.
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
- **Smaller audit findings, not fixed** (read or run by the audit, not re-checked since):
  RTSP credentials are on the `ffmpeg` command line (`ps` shows them; logs are
  redacted): not fixable with the ffmpeg 6.1.1 on the working machine (`-/i` is an
  unrecognised option there), and the audio guide says so; the unit runs as the login
  user with `ProtectHome=read-only`, so that user's `~/.ssh` is readable by the
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
- **Workflows that have never run on GitHub:** `codeql.yml`, `osv.yml`,
  `scorecard.yml` and `fuzz.yml` were adapted or written without a first run;
  `release.yml`, `docker.yml` and `mutation.yml` were read and their non-publishing
  parts run (section 3), not executed as workflows. Left unchanged from that read:
  the release SemVer check accepts build metadata (`+…`), which a Docker tag cannot
  carry, so a tag like `v1.2.3+x` would pass validation and then fail to publish the
  image (read from the regular expression in `release.yml`, not run).
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
