# Handoff

What only the maintainer can do, what the last working session did and did not
verify, and where the next one should start. This file was **rewritten**, not
appended to, on 2026-10-21 and its sections 3 to 6 were brought up to date on 2026-10-22 for
the tree at the commit named in section 4. Facts about GitHub were read on 2026-10-21 with
read-only `gh api` calls, each named in section 1; read them again before relying on them.
Anything not attributed to a command is marked as reported, read or unknown.

The working branch is `claude/amazing-babbage-nxiuuc`, pushed to `origin`. `main` is `ea2d439`, the
merge of #264 (`gh api repos/tomtom215/BirdNet-Behavior/commits/main`); the branch is
`git rev-list --count origin/main..HEAD` commits ahead of it and `git diff --shortstat
origin/main HEAD` is the size of the difference (both move; the commands are the source).
**No pull request has been opened**, so no `pull_request`-triggered workflow has run on it
(section 6, step 1).

## 1. Repository settings only the maintainer can change

Read on 2026-10-21 (`gh api repos/tomtom215/BirdNet-Behavior/…`). These are settings,
not files, so no commit can make them true.

| Setting | State read | Why it matters | Where |
|---|---|---|---|
| Private vulnerability reporting | **off** (`…/private-vulnerability-reporting` answers `{"enabled":false}`) | with it off, `SECURITY.md` tells a reporter to open an issue that holds no technical detail and says how to be reached; the issue chooser and `CODE_OF_CONDUCT.md` point at that route, and the chooser carries a comment saying when to point back at the advisory form (`.github/ISSUE_TEMPLATE/config.yml`) | Settings → Code security → Private vulnerability reporting |
| Dependabot alerts | **off** (`…/dependabot/alerts` answers 403 "disabled for this repository") | the weekly `cargo audit` / `cargo deny` run and the OSV scan are the only advisory monitors | Settings → Code security |
| Protection of `main` | **none** that could be read: the one ruleset, `protect-main` (id 14230539), has `enforcement: disabled`; the classic branch-protection endpoint answered 403 to this token, so it is **unknown**, not absent | nothing required means nothing stops a push that fails CI | Settings → Rules → Rulesets: enable `protect-main` and require the seventeen checks listed in [`.github/workflows/README.md`](../.github/workflows/README.md) (read its note on path-filtered workflows) |
| Annotated release tags | **3 of 24** tag refs are annotated, 21 are lightweight (`…/git/refs/tags`, object type per ref) | `RELEASING.md` prescribes `git tag -a`, and `release.yml` now refuses a lightweight tag, so the *next* tag must be annotated | `git tag -a vX.Y.Z` (or `-s`) |
| Issue labels | the repository has `bug`, `documentation`, `duplicate`, `enhancement`, `good first issue`, `help wanted`, `invalid`, `question`, `wontfix` (`…/labels`). **Missing, and named by files in the tree:** `needs-triage` (both issue forms), `dependencies`, `rust`, `ci`, `docker` (`.github/dependabot.yml`) | Dependabot's pull requests ask for labels that do not exist; GitHub drops a label it cannot find | Issues → Labels |
| Topics | **none** (`topics: []`) | discoverability only | the gear beside About |
| Repository licence detection | `license.spdx_id` is `NOASSERTION` (name "Other") | the sidebar shows no licence | see section 2: replacing `LICENSE` will **not** fix it |
| Discussions | **off** (`has_discussions: false`) | `SUPPORT.md` and the issue chooser no longer point at them | Settings → Features, if wanted |

Not readable from the session, so **unknown**: Actions permissions (the proxy answers
"not permitted"), code-scanning alerts (403), secret scanning, who has write access.

**Open pull requests:** none (`…/pulls?state=open`, 0). The four Dependabot pull requests
the previous hand-off listed are gone.

**CI on `main` (`ea2d439`):** 17 workflow runs, none with a conclusion other than
`success` (`…/actions/runs?branch=main`, filtered to that SHA): A11y & Visual QA, CI,
CodeQL, Coverage, Docker, Docs, Fuzz, Install smoke test, Mutation testing, OSV-Scanner
(push and schedule), Publish model release (twice), Scorecard, Supply chain, Upgrade test,
and one Dependabot run. That is the merged state of the *previous* branch; nothing says
what this branch will do.

## 2. Decisions that are the maintainer's

- **The default classifier is BirdNET V2.4, not the V3.0 preview** (already made, by the
  last session, and reversible). A new install, a new Docker volume and `quickstart.sh`
  get V2.4 with its v2 range filter, `CONFIDENCE` 0.70, `SENSITIVITY` as BirdNET-Pi maps it
  and the human-sound rule at rank 10; `--classifier birdnet-v3` chooses the preview; an
  existing station never changes model. The places that name the default are
  `installer/lib/52-classifier.sh`, `select_classifier` in `docker/entrypoint.sh`, the
  compose files, the CI model fetch, the README and the book.
- **The V3.0 privacy default is not protective: decide whether to document it or build a
  rule.** Over the same 91 clips the human-sound rule masks 246 of 665 windows (37.0 %) on
  V2.4 and 18 of 459 (3.9 %) on V3.0, and 0 of 31 windows of five LibriVox speech clips on
  V3.0 (`docs/book/guides/parity.md`, "What the human-sound rule does on V3.0"); on one of
  those clips V3.0 gives its human classes 0.002 or less, so a confidence threshold would
  not catch it either. The page, the README and the changelog now say so. The options are
  to leave it documented, or to build a V3.0 rule (voice-activity detection is G-13 in
  [`FEATURE_GAP_ANALYSIS.md`](FEATURE_GAP_ANALYSIS.md)); nothing in the project measured a
  setting that works.
- **The licence, and what GitHub will do with it.** `LICENSE` is a summary that points at
  `LICENSE-UPSTREAM`; GitHub reports `NOASSERTION`. The previous hand-off suggested
  replacing `LICENSE` with the full CC BY-NC-SA 4.0 text. The release agent ran GitHub's
  detector (`licensee` 10.1.0) over candidate files and reports to the lead that **replacing
  it with the verbatim text does not change `NOASSERTION`**; I did not repeat that, so treat
  the suggestion as withdrawn and the cause as unknown. `LICENSE-UPSTREAM` carries the
  third-party texts and goes into every release archive (`cp … LICENSE LICENSE-UPSTREAM
  NOTICE THIRD_PARTY_LICENSES.txt …`, `release.yml`) and every crate tarball
  (`scripts/check-crate-licences.py`).
- **The licence fit of the bird-name tables.** `crates/birdnet-core/data/l18n/` (35
  languages) and the English names in `crates/birdnet-core/data/labels/` come from
  BirdNET-Pi's tables, whose README credits translations and Wikipedia without saying which
  name came from where; Wikipedia-derived names are CC BY-SA 4.0 and this project is
  CC BY-NC-SA 4.0. Whether they may be combined and redistributed is unsettled
  (`NOTICE`, both READMEs). `birdnet-core`'s crate tarball carries the table.
- **Publishing to crates.io, and in which order.** Nothing is published; the nine names
  were unclaimed on 2026-10-15 (`RELEASING.md`). crates.io admits five new crates in a
  burst and then one per ten minutes, so `RELEASING.md` gives two batches: `birdnet-core`,
  `birdnet-db`, `birdnet-behavioral`, `birdnet-timeseries`, `birdnet-scheduler` first;
  `birdnet-integrations`, `birdnet-migrate`, `birdnet-web`, `birdnet-behavior` about forty
  minutes later. A non-commercial licence on a registry most consumers read as open source
  is a separate question. **`cargo install` caveats** (say them wherever it is
  documented): use `--locked` (otherwise `ort`'s `^2.0.0-rc.14` resolves to the newest
  candidate of the day); it builds the bundled DuckDB and downloads ONNX Runtime; a package
  build renders no in-app `/help` manual (`BNB_HELP_DIR` can point at a release archive's
  `help/`); and it embeds neither DuckDB extension, which are fetched into `~/.duckdb` on
  first use, so the analytics need the network once and a writable home directory.
- **A workspace lint policy for the five numeric cast lints.** Proposal, not implemented:
  allow `cast_precision_loss`, `cast_possible_truncation`, `cast_sign_loss`,
  `cast_possible_wrap` and `cast_lossless` once in `[workspace.lints.clippy]`, the way
  `module_name_repetitions` already is, and delete the per-site allows; keep
  `reason = "…"` for every other lint. The debt pass's `syn` parse counted 429 `#[allow]`
  attributes, 419 without a reason, 732 lint entries, of which the five cast lints are 573
  (78 %). A regex (the script at the end of this file) over the tracked `.rs` files of `crates`,
  `src`, `tests` and `tools` at the tip when this was written (`042973f`) finds 459 attributes, 440 without a reason, 796
  entries and 599 cast-lint entries (75 %); the two methods differ and the tree moved, so
  the proportion is the stable number. Not measured: how many casts in code that has no allow
  today the workspace allow would silence.
- **Mirroring the model files.** `publish-model.yml` was run (run 39469063446, from the
  previous branch): the release `models-v2.4` holds `birdnet-v24.onnx` (54,068,648 bytes),
  `birdnet-v24-range-v2.onnx` (29,524,170) and `SHA256SUMS` (`gh api …/releases`, sizes read
  2026-10-21). **Nothing fetches from it**: `crates/birdnet-integrations/src/model_catalog.rs`
  has one URL per entry and no mention of the mirror, so a default install still depends on
  Hugging Face and `raw.githubusercontent.com`. To do: name the mirror first with the
  upstream as the fallback (the V3.0 model already works that way in the installer's
  shell), and re-run the workflow from `main` after the merge so its attestation names
  `main`. The previous hand-off recorded that the attestation step succeeded; nobody ran
  `gh attestation verify`.
- **Three things BirdNET-Pi has and this project does not, with no recorded decision**
  (A-16, A-17, A-18 in the gap analysis): Reboot and Shutdown from the browser (the unit's
  `SystemCallFilter` forbids `@reboot`, which is a decision nobody wrote where a user would
  look), a scheduled self-update (the migration guide tells a migrator it is not done, which
  is the guide's statement, not yours), and a LAN switch that skips the sign-in (the
  installer generates a password, the opposite choice). Decide each: build, or decline and
  say so in one place a migrator will find.
- **BirdNET-Pi's default range filter (MData v1)** is hosted nowhere; `tools/model-convert`
  converts it and v2 (max difference 3.7e-6 from TFLite over 1,344 calls) and the catalogue
  carries v2 only. Hosting v1, and under what terms, is yours.
- **Accept, amend or drop [`STABILITY.md`](../STABILITY.md)**, still marked *proposed*. Its
  counts were re-measured on 2026-10-21 (56 migrations, 155 config keys, 118 `--help`
  option lines, 106 paths and 108 operations in `openapi.json`). Under its section 3 several
  changes of this release are breaking (defaults, `SENSITIVITY`'s meaning, `APPRISE_TRIGGER=off`,
  webhook requests no longer using `HTTP(S)_PROXY`, the Docker build arguments) and section 4
  asks for entries that begin with the word *Breaking*; the changelog groups them under
  *Upgrade notes* instead and uses the word once, for the Docker build arguments.
- **Release signing.** Tags and `SHA256SUMS` are unsigned; the in-app updater trusts the
  GitHub release.
- **Where the Docker quickstart points.** The bare-metal one-liner names the release's
  `install.sh`; `quickstart.sh`, the three compose files it downloads, `scripts/hardware-test.sh`
  and `packaging/macos/verify-macos.sh` are still fetched from `main`. Moving them means
  making them release assets, a `release.yml` change that cannot be tested before a release
  carries them.
- **Several stations writing to one store** ([`design/multi-station.md`](design/multi-station.md)):
  nothing is built; five decisions block Phase 0 (who the stations are, where the hub runs,
  the default clip policy, the scale target, one hub or a tree).
- **Whether the first-run wizard should demand a setup code.** A station whose admin
  account has no password logs one warning at start-up, but the first browser to finish the
  wizard still chooses the password.
- **Product calls agents made that you may want reversed.** From the earlier work:
  confidence colours call anything under the cut-off plus ten points *borderline*; the
  alert-rule cooldown is entered in minutes, 5 by default; `BIRDNET_CORS_ALLOWED_ORIGINS=*`
  is dropped with an error; a feed's `?base=` is honoured only for the request's own host or
  `BNB_PUBLIC_URL`/`BNB_BASE_URL`; the occurrence page rounds the position to 0.1°; the
  daemon treats an unreadable species list as empty while the pages refuse to; the doctor's
  data-volume check does not flag an anonymous Docker volume; frequency shift needs
  ffmpeg 4.3 or newer and the sox fallback is gone. **From this session:** several
  notification rules matching one detection send one message with every reason, where
  BirdNET-Pi sends one per rule; the importer sets `notify_confidence` to 0 for a migrated
  station so the same detections notify; the species lists are added to, never replaced;
  the weather API has no `sun`, `moon` or `daily` (a public sunrise time gives away the
  longitude); AviCommons uses only CC0, CC BY and CC BY-SA photographs; `EMBED_ORIGINS` is
  a list and never `*`; a learned-threshold reset is a `POST`; `FULL_DISK=keep` stops
  recording only in the recordings directory; Opus and AAC clip export is declined (the
  decoder cannot read them); the 3D terrain loads a 134 kB script only when asked for.

## 3. What the last session changed

The changelog's *Unreleased* section has an entry for each; this is the map, by theme, with
the commits to read first (hashes valid on this branch).

- *Same audio, same detections.* V2.4 installable and pinned; the parity harness
  (`tools/parity`, goldens from BirdNET-Pi's own code); `CONFIDENCE` 0.70; the human-sound
  rule by rank; BirdNET-Pi's English names for V2.4; `SENSITIVITY` as BirdNET-Pi maps it; the
  resampler delay (`280f7fc`: non-48 kHz audio reached the model up to a sample early);
  the 120-recording measurement (`77aab06`, `9a01a73`, `70ef51b`).
- *A station set up from the browser* (`25843ca`, `d47aee2`, `4c9b572`): model install,
  microphone choice, restart, "when does this apply", a place for every setting.
- *Moving from BirdNET-Pi* (`4cfa600`, `2e6c8aa`, `62478ad`): settings, species lists, history
  and clips, a generated mapping table, `FULL_DISK=keep`, independent notification triggers.
- *Parity with birdnet-go* (`d098923`, `d398fc6`, `8782d93`, `a8929d4`, `3a6f6f5`, `acd3920`,
  `ca0da8b`, `8287eb4`, `ce7163d`, `8f516fb`): rows P-2 and P-8 to P-21 of the gap analysis.
- *Interface:* a design-token system and gates (`3089bb8`, `53d3d59`, `1582410`), the shell
  (`0c6a1c3`, `c7d768c`, `6c3a71e`), a chart kit and the charts on it (`da2aec9`, `2b6d184`,
  `8508703`, `fc90a5a` vendors three.js 0.186.1, `aa9c50c`).
- *Engineering debt* (`a82fbbb`, `64fc64b`, `51b9188`, `3abfa0e`, `18967b3`, `c971f64`):
  failed reads no longer printed as facts (a sweep that is a test), no blocking filesystem call on
  a runtime worker, test passwords built at run time, no TODO ships, dead lint allows deleted.
- *Release engineering and public-repository hygiene* (`cbc2c21`, `b213bde`, `bcc28ee`,
  `65094fc`, `14b34f1`, `fa3015f`, `e783ef6`, `71129a0`, `e88ee2c`, `ac18953`, `e88cd67`).
- *Lint, format and typos clean-up* by the last agent (`b89e8e0` and following; read `git log`).
- *The final hardening pass (2026-10-22).* Credentials are masked in admin forms and the
  support bundle (`masking.rs`); the cross-site check holds over HTTP/2 and refuses
  `Sec-Fetch-Site: cross-site`; every form handler takes `HtmxForm` so a blank or garbled field
  is a toast and not a 422 htmx hides (`184b3db`); the service worker is dormant, honestly
  (`sw.js`, gate `sw-state.mjs`); BirdWeather's privacy settings fail closed; every page fits
  a 320 px phone and the setup wizard a 360 px one (`2e63d18`, `ba1f5a1`); the code-scanning
  alerts were reproduced locally and fixed in code or listed for dismissal
  (`tools/code-scanning/dismiss.sh`).

## 4. What was verified, and by what command

Run on 2026-10-22 on commit `59762e1` (later commits changed only `CHANGELOG.md` and this
file), in a container with the real BirdNET V2.4 and V3.0 models and the range model loaded
through the `BIRDNET_TEST_*` variables, with `~/.duckdb` moved aside. Exit codes come from the
runner's own results files, not from a pipe.

| Command | Printed |
|---|---|
| `cargo test --all-features --no-fail-fast`, run in 65 batches (each crate's `--lib`, the binary, the root `tests/` six at a time, each crate's `tests/` six at a time, `--doc` per crate) | 293 `test result:` lines, every batch exit 0: **4597 passed, 0 failed, 9 ignored** (the ones I read are subprocess workers started by their parent tests) |
| `cargo fmt --check --all` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings`, then with `--no-default-features`, then `--all-features` | exit 0, 0, 0 |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features` | exit 0 |
| `typos --config ./.typos.toml`; `scripts/gen-third-party-licenses.py --check`; `scripts/check-crate-licences.py --self-test`; `scripts/gen-cli-help.sh` then `git diff --exit-code` on the reference; `scripts/check-ci-config.py` (needs `cargo-mutants` 27.1.0) | exit 0 each |
| 30 browser-gate steps in `tools/visual-qa` against a fixture built from `59762e1` (Chromium 141): `layout-selftest`, `qa-selftest`, `layout`, `tokens`, `chart-ramps`, `surface-layers`, `token-doc --check`, `motion`, `focus`, `figures --fail`, `dom-sinks`, `interactions`, `charts`, `relational`, `terrain`, `listen-fx`, `display-format`, `legibility`, `cls`, `axe`, `onboarding-wizard`, `live-logs-connect`, `sw-state`, `sonar`, `base-path` and `sw-state` under `/bn`, `charts-edge` on the `empty`, `one` and `stress` profiles, `STRICT=1 qa` | exit 0 each. `layout` graded 336 page states (1440, 390, 360 and 320 px, both themes); `qa` reported 0 pages with issues; `interactions` 79 passed; `tokens` 91 checks; `axe` no blocking violations; `charts-edge` graded 48 / 42 / 2 charts |
| `live-logs-connect`, `sw-state`, `sonar` with `PLAYWRIGHT_BROWSERS_PATH` set | pass in Chromium 141, Firefox 157 and WebKit 27.2 (Playwright's Linux builds) |
| a one-off script (not committed) loading all 42 routes in Firefox and WebKit at 320, 360 and 390 px | 252 route/width states, 0 with sideways overflow |
| the resampler gate `tests/resampler_matches_resampy_parity.rs` against the file before `280f7fc` | fails ("44100 Hz: output length left: 96001 right: 96000"), passes after |

Other gates were seen failing before their fix, as reported in the commit messages that add
them (`184b3db`, `2e63d18`, `ba1f5a1`, `59762e1`, `7282666`); the commit message is the record,
and I re-ran only the resampler one myself. The documentation pass's counts (56 migrations, 155 config keys, 106 paths and 108
operations in `openapi.json`) date from 2026-10-21 and were not repeated.

**Not verified:** a CI run of any workflow on this branch (no pull request); OpenSSF Scorecard
(it cannot run here); an upgrade or fresh install on a real systemd host
(`installer/test/upgrade-e2e.sh`); a real Raspberry Pi, soundcard, GPU or screen reader; iOS
or macOS Safari (only Playwright's Linux WebKit); birdnet-go (its source was read, never run).

CodeQL: the pinned bundle (v2.27.2, `security-extended`, `--build-mode=none`, the version
codeql-action v4.39.0 uses) was run locally on a `git archive` export of `033c6c9` for `actions`,
`python`, `javascript` and `rust`. It reported **13 alerts** where the same procedure
reproduced 117 CodeQL alerts on the original list: 11 `rust/path-injection`, 1
`rust/command-line-injection` and 1 `js/http-to-file-access`, i.e. exactly the 13 that
`tools/code-scanning/dismiss.sh` lists, with their reasons re-read against the current code
for the four whose line numbers moved (`state.rs`, `cache.rs`, `backup_recovery.rs`). Every
other rule on the list (`hard-coded-cryptographic-value`, `cleartext-logging`,
`log-injection`, `non-https-url`, the `js/` rules) is gone. This is a local reproduction, not
GitHub's analysis: GitHub's alert states change only after an analysis of the default branch,
and a dismissal is tied to a code fingerprint, so a changed line can reopen one. The Scorecard items that only a
maintainer can settle are listed in `dismiss.sh`'s closing text.

## 5. Open items, with severity

Severity is a judgement. Everything here was found or confirmed in the last session unless
it says otherwise.

1. **The pull-request-triggered gates have not seen this branch.** Open a draft pull
   request early (CLAUDE.md: work has sat broken on a pushed branch for hours). A full CI
   run is about 45 minutes and each push cancels the run in progress.
2. **The imported `apprise.txt` is outside the full backup (A-24).** Medium. A restore onto
   another machine stops notifications until the file is copied in again.
3. **The interface text is English only (A-5).** Medium, large. Species names come in 35
   languages; birdnet-go has 15 interface locales.
4. **The classifier runs on 2 threads and no setting changes that (A-7).** Medium, small.
5. **The V3.0 privacy default (section 2) and voice-activity detection (G-13).**
6. **92 of 108 settings need a restart (P-7).** Low. They say so; hot reload is not built.
7. **Smaller gaps:** weather on a detection (A-6), per-species actions (A-22), the public
   address is an environment variable only (A-25; `IMAGE_PROVIDER` and the Flickr keys *are*
   browser settings now), no certificate upload or generate page (P-22), the data-volume check
   judges one of upstream's four paths (P-4), no OIDC (G-20), the rest of the gap analysis'
   9.7 table.
8. **Known debt, each with evidence:** [`TECH_DEBT.md`](TECH_DEBT.md) TD-1 (a failed read
    inside the station-health evaluator ends an alert), TD-2 (12 of 24 help topics have no
    call site), TD-3 (the boot journal records a detection count it could not read as 0),
    TD-4 (the Capture tab's duplicate guard; the wizard logs and says nothing), TD-5 (RTSP
    credentials on ffmpeg's command line; a way around it is proven with `concat` and
    `tools/rtsp-probe`, not built), TD-6 (ten page partials routed and never linked); TD-7 (protected clips) is closed. Counts in that file were rechecked on 2026-10-21 where a command could.
9. **Public pages that scan the whole history are still slow one at a time**, and an
    unescaped reflection can still inject non-script markup (the CSP nonce goes only onto
    sealed tags and any other `<script` is rewritten to text; an `hx-post` attribute is a
    different matter): accounts in `docs/book/field/hardening.md` and
    `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`. The heavy-read gate and the species-and-date index
    helped concurrency and the feeds' worst case; a single `rare.rss` is not faster.
10. **`--doctor` reports a config value outside its range as a warning, not a refusal**
    (`START_CRITICAL` holds four names), so a bad number leaves a station running on a value
    the daemon cannot use. Whether some should stop the start is a decision.
11. **Workflows that have not run as a whole:** `release.yml` (read against a real toolchain
    and fixed; not executed), the new `docs.yml` tag and browser gates, the `fuzz.yml` pull-request
    path. The release SemVer check accepts build metadata (`+…`), which a Docker tag cannot
    carry (read from the regular expression, not run).
12. **Floating pins:** every `uses:` line is commit-pinned (133 of 133, counted); the
    `ubuntu:24.04` container `install-smoke.yml` starts with `docker run` is a tag, and
    `taiki-e/install-action` and `EmbarkStudios/cargo-deny-action` choose their own tool builds.
13. **Dependencies.** `ort` 2.0.0-rc.15, `tokio` 1.54.0, `rustls` 0.23.46, `zip` 9.0.5
    (build-time only), htmx 2.0.11 (npm also publishes a 4.0.0, not evaluated). Four packages
    have newer releases their parents hold back (`comfy-table`, `generic-array`, `matchit`,
    `psm`: from the previous `cargo update --dry-run`, not re-run).
14. **Suggestions for `CLAUDE.md`, not added** (agents proposed them; I did not edit that file
    on their say-so): a full-page escaping test must probe with a payload `seal::redeem` does
    not rewrite (`<i>`, `<img onerror>`) and assert on the rendered region; a BLOB in a TEXT
    column (`X'ff'`) makes `row.get::<String>` refuse it, which separates two reads of the
    same table in a failure test; an equality test between a form's bounds and a validator's
    holds for any grammar, so pin a table of answers too; a test helper that takes a literal
    and returns a string made from it hands the literal to every caller, so a CodeQL taint
    query alerts at each call site; the mutation job runs unit tests only and has no model
    files; and a real clock never equals a `Duration`, so put a time comparison in a
    function of its two durations.

- **Refusals that htmx hides.** `HtmxForm` turns unreadable forms into toasts, but three
  middleware layers still answer a plain 4xx to an htmx write, which htmx discards: the
  cross-site check (403; behind a reverse proxy whose `Origin` differs from `Host`, every
  write would do nothing visible), the viewer-role refusal (403) and the rate limiter (429).
  One outermost layer turning a 4xx/5xx on a non-GET `HX-Request` into 200, `HX-Reswap: none`
  and a status toast (excluding `HX-Redirect`, `HX-Location`, `HX-Refresh`) would close it;
  not built because it changes the status every htmx caller sees. Check the proxy case first.
- **WebKit reports a `style-src-attr` violation for every plain `<select>`** under this
  station's CSP. Reproduced in Playwright's Linux WebKit 27.2 with a bare page and none of our
  code; Firefox and Chromium do not. Its effect on a select's look in Safari is **unknown**
  (not run on macOS or iOS).
- **Smaller items found at the end and left:** the species hourly chart and the weekly chart
  still scale down on a phone rather than being redrawn; a hint under the Data tab's source
  time zone select overlaps slightly at 320 px and no gate flags it (read from a screenshot by
  the agent that found it, not by me); `cargo install` caveats and the crates.io publish order
  are section 2 decisions.

## 6. Where to start

1. Open a **draft** pull request from `claude/amazing-babbage-nxiuuc` and read the first
   CI run. Expect to fix what only CI shows: the previous branch's first run on #264 found
   a mutation survivor, 17 CodeQL alerts and more mutation survivors that no local run had.
   The default layout gate is now about four times longer (336 page states, roughly 12 minutes
   locally); `a11y.yml` has not run it.
2. Run `APPLY=1 tools/code-scanning/dismiss.sh` with a maintainer token for the dismissals it
   lists, and read what it prints for the Scorecard items only a maintainer can settle.
3. Make the settings in section 1 (private vulnerability reporting, the five labels,
   topics, `protect-main`) and tag the next release with `git tag -a`.
4. Decide, in this order: the V3.0 privacy default; whether to wire the model mirror; the
   three missing controls (A-16 to A-18); the cast-lint policy; crates.io.
5. Close A-24 (section 5, item 2): a migrated station's notifications stop on restore until
   `apprise.txt` is copied in again.

The lint-allow count in section 2 came from this script, run from the repository root:

```python
import re, subprocess, collections
files = [f for f in subprocess.run(["git","ls-files","*.rs"],capture_output=True,text=True).stdout.split("\n") if f and f.split("/")[0] in ("crates","src","tests","tools")]
attrs = noreason = 0; entries = collections.Counter()
for f in files:
    s = open(f, errors="replace").read()
    for m in re.finditer(r"#!?\[allow\(", s):
        i = m.end(); depth = 1; j = i
        while j < len(s) and depth:
            c = s[j]
            if c == "(": depth += 1
            elif c == ")": depth -= 1
            elif c == '"':
                j += 1
                while j < len(s) and s[j] != '"': j += 2 if s[j] == "\\" else 1
            j += 1
        body = s[i:j-1]; attrs += 1
        if not re.search(r"\breason\s*=", body): noreason += 1
        for e in body.replace("\n", " ").split(","):
            e = re.sub(r'reason\s*=\s*"(?:[^"\\]|\\.)*"', "", e).strip()
            if e: entries[e.split("::")[-1]] += 1
cast = sum(entries[k] for k in ("cast_precision_loss","cast_possible_truncation","cast_sign_loss","cast_possible_wrap","cast_lossless"))
print(attrs, noreason, sum(entries.values()), cast)
```
