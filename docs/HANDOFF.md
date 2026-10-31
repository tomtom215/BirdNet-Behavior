# Handoff

What only the maintainer can do, what the last working session did and did not
verify, and where the next one should start. This file was **rewritten**, not
appended to, on 2026-10-21 and its sections 3 to 6 were brought up to date on 2026-10-22 for
the tree at the commit named in section 4; the header, section 5 item 1 and section 6 were
corrected on 2026-10-26 after the first release attempt failed. Facts about GitHub were read on 2026-10-21 with
read-only `gh api` calls, each named in section 1; read them again before relying on them.
Anything not attributed to a command is marked as reported, read or unknown.

The working branch is `claude/amazing-babbage-nxiuuc`, pushed to `origin`.
[#265](https://github.com/tomtom215/BirdNet-Behavior/pull/265) (opened as a draft on 2026-10-22) was
**merged on 2026-10-25 as `71539e5`**; `git log -1 origin/main` is the source for where `main` is
now. The verification in section 4 predates #265's CI. The CI on #265's last head was read while it
ran: the CodeQL results check passed after the maintainer's dismissals, and `Tests (x86_64)`, the
accessibility sweep and five mutation shards were still running when last read, so their end state
is not recorded here. **0.18.0 has not been released**; section 5, item 1 says why.

**2026-10-28 additions.** A later session worked on `claude/serene-hamilton-6wlll0`, branched from
`d0fcfdb` (the merge of #270, which is `origin/main` in this checkout; GitHub was not re-read),
and pushed it; it is pull request #271 (section 6 says what its CI found). It moved the engine to DuckDB 2.0.0, gave the
Patterns, Species and Weekly pages one frame, added the kiosk plate and the public-domain plate tier behind
it, put a credit on every picture, added the Plate chart style and the terrain's sunrise finding, and wrote
`docs/design/ux-audit-2026-10.md`. Section 3 (last bullets), section 4 (its last subsection), section 5 (items 15
to 22) and the DuckDB paragraph of section 6 were updated for it; **everything else in this file is as it was on
2026-10-26 and was not re-read.**

**2026-10-31 additions.** A later session worked on `claude/amazing-bardeen-317rgu`, branched from `ec59953`
(the merge of #271; GitHub was not re-read) and pushed it. **No pull request was opened** (the session was told not to open one
unasked), so nothing on GitHub has looked at it; `CLAUDE.md` advises a draft pull request early for exactly that reason. It
put every 24-hour chart on the day's real sky, notched the columns, turned the heat grids into discs and gave the busiest bar a
margin note; added three views each built for one question (Today's *Every song*, Year in review's *Who led, and when it
changed*, the life list's *When each bird joined*); added one new use of the extension, *Who answers whom*
(`sequence_count`); and wrote `docs/design/insights-and-organisation.md`, which is a plan and not a build. Section 2 (last
bullet), section 3 (last bullet), section 4 (last subsection), section 5 (items 23 to 29) and section 6 (last paragraph but
one) were updated for it; **everything else is as it was on 2026-10-28 and was not re-read.**

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
  the proportion is the stable number. The same script at `91812fc` (2026-10-31) gives 475 attributes,
  454 without a reason, 818 entries and 615 cast-lint entries (75 %). Not measured: how many casts in code that has no allow
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

- **How the analytic pages are organised: A, B or C** (`docs/design/insights-and-organisation.md`, section 3). **A**: keep the
  six homes and give every analytic page the same four layers (At a glance, The picture, Look closer, Method and export), with
  the Patterns tabs grouped as When / Who / How; no URL or tab key changes; the plan's recommendation. **B**: a Depth switch
  (Glance, Explore, Research) per page; not recommended, because a reader never finds what a switch hides. **C**: a seventh
  home, "Insights"; better as a strip at the top of Today and of each Patterns tab. Nothing is built for any of them; the
  Method-and-export panel and the Patterns regrouping wait on this.

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

- *2026-10-28, branch `claude/serene-hamilton-6wlll0` (`git log d0fcfdb..`):*
  - *Engine and dependencies* (`105c620`, `8c59d2c`, `c7fe862`): DuckDB 2.0.0 with `behavioral` v0.10.1 and `icu` v2.0.0,
    `rusqlite` 0.41 (SQLite 3.54.1), a plain `cargo update`. What the engine bump needed beyond a version change (its
    self-reported version, the stable C API table, and a `build.rs` link argument for GNU ld) is in the changelog.
  - *Page frames* (`013dad2`): `pages/frame.rs` and `pages/insight.rs`, one range on the When tab, the weekly report
    compared like with like. Recordings and the three Reports tabs moved onto the frame afterwards.
  - *The kiosk plate* (`de7045e`): `/kiosk` as a page from a field guide, after fugleramme (credited in `README.md`,
    `NOTICE` and the changelog; no code or artwork shared).
  - *Pictures and credits:* every picture kept with title, author, source and licence (`species_images::ImageCredit`),
    shown under the picture, in the image JSON and on `/system/credits`; Flickr CC BY-ND no longer requested; a public-domain
    plate tier (`species_images/plates.rs`, `/api/v2/species/plate/{name}/file`, `static/plate.js`) from an index built by
    `tools/plate-index/`.
  - *Display → Charts → Plate* (`bnb-illustration`, `app.css` "Plate charts"): colour first, strokes second, sequential
    scales flat; the terrain says whether song gathers at sunrise and marks sunrise and sunset.

- *2026-10-31, branch `claude/amazing-bardeen-317rgu` (`git log ec59953..`):*
  - *Engine* (`afe58a8`): `AnalyticsDb::answers` with `queries::answers_sql` and `types::Answers` (a `sequence_count` per ordered
    pair, pattern `(?1).*(?t>0)(?t<=N)(?2)`), and `monthly_species_counts` in `birdnet-db`.
  - *Chart vocabulary and four views* (`47baf36`): `viz/kit.rs` (sky, notches, notes, halos), `viz/bars.rs`, `heat.rs`,
    `timeline.rs`; `pages/{today_songs,life_timeline,behavioral_answers}.rs` and the seasons card in `year_in_review.rs`;
    `static/{view-switch,skysong,seasons,discovery}.js` (the last three written by builder agents against one brief, then
    re-read and re-run here; their own mutation reports are theirs and were not repeated); the `--sky-*` tokens in `app.css`.
  - *Fixes the full run found* (`79c1010`, `91812fc`): four gates red in the web crate and one in the root crate's sweep, all on this branch's own work, 23 clippy errors, and three
    tests too weak to fail against their own mutant (section 4).

## 4. What was verified, and by what command

The test suite and the lint gates (the first five rows) were run again on 2026-10-23 on the tree
of commit `14ae57d` (the dependency refresh and the 0.18.0 bump; later commits changed only
`docs/HANDOFF.md`). The browser-gate rows were run on 2026-10-22 on `59762e1`; nothing a browser
renders has changed since (the later commits touched Rust logic behind tests, workflows,
dependencies and documents). All in a container with the real BirdNET V2.4 and V3.0 models and
the range model loaded through the `BIRDNET_TEST_*` variables, with `~/.duckdb` moved aside.
Exit codes come from the runner's own results files, not from a pipe.

| Command | Printed |
|---|---|
| `cargo test --all-features --no-fail-fast`, run in 65 batches (each crate's `--lib`, the binary, the root `tests/` six at a time, each crate's `tests/` six at a time, `--doc` per crate) | 293 `test result:` lines, every batch exit 0: **4599 passed, 0 failed, 9 ignored** (4597 on 2026-10-22) (the ones I read are subprocess workers started by their parent tests) |
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

**Not verified:** OpenSSF Scorecard
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

### 2026-10-28 (branch `claude/serene-hamilton-6wlll0`, container with `~/.duckdb` moved aside)

| Command | Printed |
|---|---|
| `cargo test -p birdnet-web --features analytics --no-fail-fast`, the library and 25 batches of up to six test files | 149 `test result:` lines, 2078 passed, 0 failed |
| `cargo test -p birdnet-integrations`; `cargo test -p birdnet-db` | 560 passed, 0 failed; 575 passed, 0 failed |
| `cargo test -p birdnet-behavior --no-fail-fast` (root tests), 12 batches of up to six, then `web_api_pages` | 74 `test result:` lines, 414 passed, 0 failed; `--bins` (the binary's own tests): 1150 passed, 0 failed in 4 results |
| `cargo clippy -p birdnet-web -p birdnet-integrations -p birdnet-db -p birdnet-behavior --all-targets --features birdnet-web/analytics` (the analytics feature only, not CI's `--no-default-features` and `--all-features` variants); `cargo fmt --check --all` | exit 0 with 0 warnings or errors; exit 0 |
| `ILLUSTRATION=plate node tools/visual-qa/axe.mjs`; `... layout.mjs` | 168 route/theme/viewport states with 0 violations; 336 page states with no finding |
| `node tools/visual-qa/base-path.mjs` | PASS: every same-origin request stayed under `/bn` |
| `python3 -I tools/plate-index/build.py --verify`; the same with `--offline --generated 2026-10-28` from the cache | `verify: 0 problem(s) in crates/birdnet-integrations/data/plates/index.json`; byte-identical (`index.json` sha256 `c43a4d15c7e553ff984888613a6979e9428c3a426fa8d9a6af897927ce2c50cf`, `report.md` `de996c9804aad45ddfba270ed7d318633d33507149b48db50af0457d505ac416`) |

The first full root run found six failures, each a gate catching this branch's own change: a
sunrise default, now classified `omitted` with its reason in `tests/data/read_default_sweep.tsv`; a blank `ar` on the kiosk that answered 400
(fixed, with a test); a credit write that blocked a runtime worker (now `spawn_blocking`); three public-address
types that `no_secret_in_derived_debug_guard` took for credentials (named in its table with reasons); a test provider
that returned an uncredited picture (now credited); and the species page, which grew past a 64 KiB read bound in a
test. Batches stop at their first failing file, so the whole sweep was run again with `--no-fail-fast`.

Gates seen failing, each against a mutant or the old code (details in the commit messages):
`a_plate_stroke_can_be_seen_on_every_colour_it_crosses` (two mutants, 2.09:1 and 2.93:1),
`the_plate_chart_style_keeps_its_promises` and `every_page_carries_the_hatch_patterns` (three mutants),
`every_numbered_figure_is_a_sheet` (14 cards named), `the_kiosk_draws_public_domain_plates` (three mutants),
`today_has_one_h1_and_the_help_link_in_its_eyebrow`, the plate credit tests (an `@` appended to the author),
`a_blank_aspect_ratio_draws_the_default_sheet_but_a_garbled_one_is_refused` (400 on the old handler).

**Not verified (2026-10-28):** anything on a Raspberry Pi; the sunrise and sunset lines across a change of clocks (the
container's zone is UTC, and setting another needs `unsafe`); Firefox, WebKit and macOS on this branch; arm64 beyond
CI (the aarch64 Docker image build passed on `2b2a4c1`, which is what ran the `build.rs` GNU ld argument there; nothing
else was run on arm64 by hand);
`scripts/check-ci-config.py` (needs `cargo-mutants`); whether the backup includes the new `plates/` folder in the picture cache; the plate tier on more than twelve
real plates and the engraving on more than four real photographs.

### 2026-10-31 (branch `claude/amazing-bardeen-317rgu`, `~/.duckdb` absent)

| Command | Printed |
|---|---|
| `cargo test -p birdnet-web --features analytics --no-fail-fast`, the 148 files of `tests/` six at a time, on the tree before `91812fc` | 148 `test result:` lines; **4 files red, all on this branch's own work**: `no_page_handler_answers_a_5xx_with_a_body_htmx_will_discard` (`today_songs.rs` answers 500), `amber_is_never_used_as_a_text_colour` (`.ck-sun` was `--dawn`, 2.85:1), `every_variable_the_stylesheet_uses_is_defined_or_declared_runtime` (`--p`), `print_css_overrides_every_token_that_differs_between_themes` (four `--sky-*`) |
| the four files again, after the fixes | 5, 2, 4 and 1 passed |
| the same 148 files, second run, on `91812fc` | 25 of 25 batches exit 0; 148 `test result:` lines, 660 passed, 0 failed, 0 ignored (the first run's 659 and the one test added since). `print.css`'s axis colour was changed after that run (64% to 61%, to follow `app.css`), and the 13 files that read a stylesheet were run again: 72 passed, 0 failed |
| the root crate's `tests/` (`birdnet-behavior`), six at a time | 13 batches (the 70 files of `tests/` six at a time, then `--bins`): 78 `test result:` lines, 1563 passed, 1 failed, 0 ignored. The one failure, `every_read_that_is_defaulted_has_been_looked_at`, was this branch's: the twilight default in `today_songs.rs` had no row in `tests/data/read_default_sweep.tsv`; it is classified `omitted`, as the same call in `dawn_chorus.rs` is, and the file passed on a rerun (3 passed). The binary's own tests were 1140 of the passed |
| `qa-selftest.mjs`; `layout-selftest.mjs`; `STRICT=1 node qa.mjs` against a fixture rebuilt from `a7797c9` | "all pass"; "all pass"; **exit 1**: 180 states, 4 with issues, all `species-lifelist` (light and dark, desktop and mobile): `allSpeciesPhotosFailed(6/6)`. Item 29 |
| `cargo test --lib` for `-p birdnet-web --features analytics`; `-p birdnet-behavioral --features analytics`; `-p birdnet-db` | 1434, 128 (including `live_answers` against the embedded extension) and 501 passed; 0 failed in each |
| `cargo clippy -p birdnet-web -p birdnet-behavioral -p birdnet-db --all-targets --features birdnet-web/analytics -- -D warnings` | exit 0 (it had reported 2 errors in this branch's code, then, once those were fixed, 20 in `birdnet-web` and 1 in a `birdnet-behavioral` test) |
| `node tools/visual-qa/unit/{skysong,seasons,discovery}.test.mjs` | "47 tests passed", "38 checks passed", "38 tests passed" |
| browser gates against a fixture built from the tree before `91812fc`'s CSS change (Playwright's Chromium build 1243, `axe-core` 4.11.4): `tokens`, `charts`, `relational`, `interactions`, `legibility`, `focus`, `motion`, `layout`, `axe`, `figures`, `dom-sinks`, `cls`, `charts-edge` on `empty`, `one` and `stress`, `base-path` | exit 0 each. tokens 91 checks; charts "all checks passed"; relational "all passed"; interactions 80 passed, 0 failed; legibility 913 chart labels on 45 routes; focus 2613 stops, none with a missing or weak ring; layout 352 page states; axe "no blocking accessibility violations"; charts-edge 2, 42 and 48 charts graded; base-path every same-origin request under `/bn` |
| after the one rendered change (`.ck-sun`, `print.css`): `chart-ramps`, `tokens`, `surface-layers`, `token-doc --check`, and a render of the daylight rail in both themes | 146 checks, 0 failures; 91 checks; 30 checks, 0 failures; "generated regions are current"; read by eye: a deep amber dot on the light theme, a bright one on the dark |
| a probe injecting `axe.min.js` 4.11.4, 4.13.0 and 4.14.0 and running `label-content-name-mismatch` alone on `/` | 0, 0 and 1 nodes (`.topnav-brand`): item 25 |

Gates seen failing, each against a mutant or the old code. The four red files above, and the root crate's defaulted-read sweep, at the previous head. Then, for the Rust
tests this branch added, one mutant per claim (the full list is in the message of `91812fc`): 15 in `birdnet-web` and 9 in the
engine crates. **Three survived on the first pass and the tests were too weak, not the code**: a balanced pair given the
"other way round" sentence passed because the test looked only for the word "one-sided" (it asserts the whole headline now); a
heat-grid halo on a different index than its disc went unnoticed (`every_disc_has_a_halo_on_the_same_index`); and the pair
conditions' name escaping was hidden by the escaped `IN` list (the test now checks the condition and that no bare quote
survives). One engine mutant was invalid (it removed a bound parameter, so it failed on the parameter count and not on the
claim) and was replaced.

**Not verified (2026-10-31):** that CI as locked passes (`package-lock.json` pins `axe-core` 4.13.0 and Playwright 1.64.1; I ran
4.11.4 and 1.63.0); `terrain` and the other `a11y.yml` steps not listed above; the `ILLUSTRATION=plate` variants; Firefox and WebKit; a Raspberry Pi; clippy under `--no-default-features` and
`--all-features` and over the rest of the workspace; the run time of `sequence_count` over 56 ordered pairs on a Pi; and
whether the CLS result below is a regression (an earlier comparison against a baseline checkout said it was not, and its
numbers were not kept).

## 5. Open items, with severity

Severity is a judgement. Everything here was found or confirmed in the last session unless
it says otherwise.

1. **0.18.0 is not released: the macOS build did not link.** High, until it is. After #265
   merged, the maintainer pushed `v0.18.0`. `release.yml` run 40237669090 built both Linux targets,
   but `Build aarch64-apple-darwin` failed at the link step with `Undefined symbols for
   architecture arm64: "___isPlatformVersionAtLeast"`, from the CoreML objects in the ONNX Runtime
   library `ort-sys` 2.0.0-rc.15 downloads. Every job after the builds was skipped, so no GitHub
   Release exists and nothing reached crates.io; the maintainer deleted the tag. v0.17.1 had linked
   the same target (`ort` rc.13). Probe run 40257984945 (macos-14, rustc 1.99.1): the unchanged tree
   failed the same way; the same tree with Xcode's `libclang_rt.osx.a` added to the link line linked;
   `ort` rc.13 does not compile with this code (it uses the rc.14 `Session::builder(&Environment)`
   API), and rc.14 also failed at Build, for a reason nobody read. Rust 1.99.1's `std` defines the
   symbol too, weakly, and the link did not take it from there; why was not established.
   The fix is in `build.rs` (`link_clang_runtime`, `cargo:rustc-link-arg-bins`, on macOS targets
   only). Probe run 40259852343 built the tree with that and no `RUSTFLAGS` in 18m14s with 0
   undefined symbols, and the binary ran `--version` and `--help`. **Not covered:** nothing builds
   the macOS target on a pull request, so the next break of this kind shows only at tag time (a
   path-filtered macOS job in `ci.yml` would catch it; none exists); `cargo test` for
   `birdnet-core` on macOS is untested and probably hits the same symbol; the release job's
   `--features analytics` build on macOS is not what the probe built.
   Two more workflows failed on both tag pushes (`71539e5` on 2026-10-25, `f7ff337` on 2026-10-26)
   and were not read after the first: **Docker** failed its tag gate in 9 s with "cannot find
   origin/main" because `fetch-depth: ${{ startsWith(github.ref, 'refs/tags/') && 0 || 1 }}` is
   always 1 (0 is falsy); the checkout log shows `--depth=1`. It is `fetch-depth: 0` now, and
   `scripts/check-ci-config.py` refuses the `cond && <falsy> || x` shape. A tag's workflows run
   from the tagged commit, so those two tags' image runs cannot be repaired by re-running them.
   **Docs**' `build` job passed but its `deploy` job failed in 1 s with no steps and no log
   (HTTP 404). The cause was not read; PLAUSIBLE: the `github-pages` environment allows only
   `main`, not tags (Settings → Environments → github-pages → Deployment branches and tags; the
   failed job's page names the rule if so). Nothing in the repo documents that setting.
   Also found while reading `main` after #265, and fixed in #270: the full mutation run missed
   `DuplicateGate::is_enabled -> bool with false` (every test of it read `false`; a pull request's
   `--in-diff` run cannot see a line it did not change), and `CI config` found four mutation
   shards over its 75% line on `main`'s latest runs (`civil.rs [2/3]` 71m of 85, `model.rs [2/3]` and
   `[3/3]` 36m and 38m of 45, `validate.rs [2/2]` 35m of 45), now 100, 60 and 50.
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
13. **Dependencies.** `ort` 2.0.0-rc.15, `tokio` 1.54.1 (1.54.0 carries RUSTSEC-2026-0384), `rustls` 0.23.46, `zip` 9.0.5
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

15. **The plate index covers 7.0% of species, mostly European and Asian.** Low to medium. 458 of BirdNET V2.4's 6,515
    (`tools/plate-index/report.md`); 17 of a fixed sample of 40 common European and North American species; 1 of the 15 in
    the North American fixture. The licence rule (Commons' own metadata must say public domain, CC0 or Public Domain
    Mark) costs at most 46 species; **10 of them are lost only to "No restrictions" files** (`Copyrighted=True`, mostly
    Flickr Commons scans), which are probably free in fact and which the rule refuses. Accepting them is the
    maintainer's decision, not a script's. Plates exist only in the work categories the crawl read; a plate filed only
    under a species category is not found.
16. **The works' years in the index are typed constants.** Low. Checked against bibliographies: Europe 1832–1837, Australia
    1840–1848, Great Britain 1862–1873, New Guinea 1875–1888, Asia 1850–1883 (other sources say 1880 or 1886), the
    Century 1830–1832 (one paper says 1830–1833). Naumann's and von Wright's are empty because they were not verified.
    Whether each chosen plate shows the bird the index names was not checked against its own caption.
17. **Plates are drawn smaller than photographs.** Low. A folio plate is portrait and the figure boxes are landscape.
    A box shape that follows the picture needs the layout to know it before the picture arrives.
18. **Pictures cached before credits were kept are replaced only when asked for.** Low. A station that never opens a
    species page keeps them uncredited; `/system/credits` counts them. There is no sweep that refreshes them.
19. **The empty, loading and failed vocabulary is still page by page; Today keeps its own hero.** Low.
    `tests/every_home_opens_with_the_shared_page_head.rs` holds Today to the two structural rules it shares.
20. **The soundscape terrain stays a flat canvas under the Plate chart style.** Low.
21. **The plate tier asks for a plate that most species do not have**, which is one extra request per species. Low. It
    answers `204`, so a browser logs nothing for it (it answered `404` first, and the strict visual sweep failed on the
    console line per species); the alternative was a second index in the page.
22. **The plate is only readable on a wide screen.** Medium for a small Pi display. The sheet is 1600 units wide and its
    smallest caption 11.5, so a caption is 8.5 px at about 1180 px of window and 2.8 px at 390 px (`legibility.mjs`
    measured 84 captions under 8.5 px on a phone). `static/plate.js` now sends a window under 1180 px to
    `?mode=numbers` unless the address names a mode; `kioskOpensReadable` in `interactions.mjs` holds it. A 7-inch
    800 × 480 display therefore opens on Numbers; its owner can ask for `?mode=plate` and gets small captions.
    Making the plate itself legible there needs a layout drawn for the screen, not scaled to it.

23. **"Who answers whom" has no REST route.** Low. The page reads `AnalyticsDb::answers` directly. A route needs an entry in
    `openapi.json` (`every_route_the_api_serves_is_in_the_openapi_document` fails without one), a handler and a test; until
    then the numbers leave only through the table on the page.
24. **The view modules' unit tests run in no workflow.** Low to medium. `tools/visual-qa/unit/*.test.mjs` (`node
    unit/<name>.test.mjs`, `tools/visual-qa/README.md`) are run by hand; `a11y.yml` does not call them, and its time budget was
    not measured with them.
25. **axe-core 4.14.0 will flag the top bar's home link when the lockfile moves.** Low until then. `.topnav-brand` has
    `aria-label="BirdNet-Behavior home"` and visible text "BirdNet Behavior": 4.14.0 reports `label-content-name-mismatch`
    (84 nodes across `axe.mjs`); 4.11.4 and 4.13.0, which the lockfile pins, report none (the probe in section 4). The markup
    is not this branch's and was not changed; making the accessible name begin with the visible words would clear it.
26. **`cls.mjs` lists three routes over 0.1** (it fails only with `--fail`, which no workflow passes): 90 page loads, total 0.752;
    dashboard 0.197 (`div.bnb-skel-bars` 0.155), species-detail-unheard 0.195, species-detail 0.135. Low. Whether this branch
    moved them is section 4's last unverified item.
27. **The daylight rail's day line is `--dawn` as a stroke** (`.ck-rail-day`), 2.85:1 on the page in the light theme, and
    `amber_is_never_a_text_colour` reads `fill:` and `color:` only. Low: it is decoration, and the same fact is printed beside
    it ("9 h 47 min of daylight").
28. **What the plan owes** (`docs/design/insights-and-organisation.md`, sections 4 to 6): *the chorus assembling*
    (`window_funnel_events` is in the REST API and no page draws it), cohort retention of new arrivals, gap sensitivity, the
    Method-and-export panel and the Patterns regrouping (both wait on the decision in section 2), the bursts timeline on a
    per-day sky, a margin note on the streamgraph, and a probe per funnel mode before any page names one (the extension's two
    documentation pages disagree about `allow_reentry`, `strict_order` and `strict`). Not known: whether a station with under
    about 200 detections a day gives the answers grid enough pairs to be more than noise.

29. **The strict visual sweep fails on the life list in this container.** Medium until it is explained on a runner with a
    network. `qa.mjs` counts the `<img>` elements that point at `/api/v2/species/image/…` and fails a page on which the number of
    distinct broken URLs equals the number of such elements. On `/species?view=lifelist` there are exactly six, the avatars of "New to the list", and the
    fixture answers `404 image not available` for all six: it holds a picture for some species only (six other species
    answered `200` on that page), none of the six newest firsts is among them, and the Wikipedia-backed fallback answered
    `429` to a probe here. What I checked: the six are the
    avatars (a live count, none from the new timeline), and `git diff ec59953..HEAD` is empty for the fixture
    (`examples/`), `routes/images.rs`, `birdnet-integrations`, `atoms.rs` and `qa.mjs`. What I did not do: run the sweep on the
    base commit, so "this fails there too" is not shown, only that nothing this branch changed can reach it. The floor also has
    a flaw: `speciesImgTotal` counts elements and `speciesImgBroken` counts distinct URLs, so a page that repeats species
    (the dashboard: 31 elements, 5 URLs) can never reach equality, and a page with one avatar per species and no pictures
    always does.

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

The 0.18.0 preparation (a plain `cargo update`, `4e6bdd1`; the bump and changelog roll, `14ae57d`)
merged with #265, and the macOS link fix with #266. The maintainer then pushed `v0.18.0` a second
time (2026-10-26, on `f7ff337`, the merge of #266); **that tag exists**, its Release run 40306761292
was still running when last read (the Docker image and the Docs deploy had already failed, section 5
item 1), and the maintainer chose to release **0.18.1** next. Pull request #270 carries 0.18.1: the
Docker fix, a mutation test, four mutation-shard budgets, and the bump to 0.18.1 with its changelog
section dated 2026-10-26.

1. Merge #270 when its CI is green.
2. Run the **dry run** (`release.yml`, `workflow_dispatch`, version blank) on `main` after the
   merge, with `Build aarch64-apple-darwin` and every `Upgrade from vA.B.C` row green; it is the
   only workflow that builds the macOS target. Then
   `git tag -a v0.18.1 -m "Release v0.18.1" && git push origin v0.18.1`. The release date
   (`CHANGELOG.md` heading, `CITATION.cff` `date-released`) is 2026-10-26; change both if the
   tag lands on another day, because `validate` checks the heading's form and the CITATION
   version, not that the date is today's. Read the Docker and Docs runs as well as Release's: all
   three start on a tag push, and both of the others failed on v0.18.0 without anyone reading them.
   Before the tag, let `github-pages` accept tags (section 5, item 1) or Docs' `deploy` will fail again.
3. **After the tag**: the README says "unreleased" 11 times (and the installation chapter once)
   about things 0.18.x ships, and names `v0.17.1` as what installs the V3.0 preview
   (line 131). Make those true in a small follow-up pull request; also run
   `scripts/gen-cli-help.sh` and confirm `docs/book/_generated/cli-help.txt` is unchanged.
4. Done 2026-10-25: the maintainer ran `APPLY=1 tools/code-scanning/dismiss.sh` (16 dismissals)
   and the CodeQL results check on #265 went from 12 alerts to "No new alerts". What the script
   prints for the Scorecard items only a maintainer can settle (#1, #8, #9, #165) is still open.
5. Make the settings in section 1 (private vulnerability reporting, the five labels,
   topics, `protect-main`), then publish to crates.io by hand in two batches
   (`RELEASING.md`, "Publishing to crates.io").
6. Decide, in this order: the V3.0 privacy default; whether to wire the model mirror; the
   three missing controls (A-16 to A-18); the cast-lint policy.
7. Close A-24 (section 5, item 2): a migrated station's notifications stop on restore until
   `apprise.txt` is copied in again.

Done 2026-10-28, on `claude/serene-hamilton-6wlll0`: the bundled DuckDB is 2.0.0 (`duckdb` 1.20000.0), with `behavioral`
v0.10.1 and `icu` v2.0.0 embedded and loaded; `the_engine_names_itself_as_a_release` and
`a_correctly_built_binary_reports_no_mismatch` are the gates that failed without the first two fixes the bump needed; the
third, `keep_duckdb_extension_registrar` in `build.rs`, is guarded by the aarch64 Docker image build (changelog, *Changed*).
**First, next session:** read pull request #271's current checks; this is a snapshot taken while it was open.
These findings were this branch's and were fixed: the typos check on the two generated plate-index files
(`.typos.toml`), a docs link to `/system/credits`, two rustdoc lints, the GNU ld link of the DuckDB archive (which only
the aarch64 Docker image build showed, and which passed there on `2b2a4c1`), CodeQL's `rust/path-injection` on the
picture cache's new credit files (the traversal was real and is closed and tested; three forms of check on the key left
the alert open, and the last alert's argument is `&self.cache_dir`, the configured directory that
`tools/code-scanning/dismiss.sh` says CodeQL takes as a remote source through axum's `State`, as it did for the cache's
dismissed alert 23. That is a false positive for a maintainer's token to dismiss, not one more check; the five in-place
`..` checks in `cache.rs` repeat `is_plain_name` and the tests pass without them), and axe's `link-in-text-block` on the species page's credit lines. The axe job stops at its
first failing step, so I ran the thirteen gates behind it locally against a **fresh** fixture started the way the
workflow starts it (a long-lived one is not the same: it hid the axe finding); they found, all in the kiosk work, that
the plate is 2.8 px on a phone (it now opens on Numbers under 1180 px; item 22), two defects of the Numbers face's
mode links, a console 404 per species without a plate (the endpoint answers 204 now), the operator's reduced motion
not reaching the plate's drift, and a gate left behind for the polar clock I had removed. Those passed locally, and CI then
found one more that a local run after dusk could not: the kiosk's exit link had the app's green focus ring, 1.47:1 on
the day sheet, because the kiosk picks its palette from the sun and the gates graded whichever the clock gave (fixed;
the route list now holds `/kiosk?night=0` and `/kiosk?night=1`). Two checks were red on `main` too and are not this
branch's: OSV-Scanner, now waived by #272 (merged), and the CI-config check, whose `a11y.yml` annotation #272 fixed and
whose nine mutation rows read `main`'s last five runs, so they clear as runs without an apt stall replace those with
one. #273 (merged) now holds a pull request to the budgets it edits, so they should no longer redden this one; the
first `CI config` run after the merge of `main` (3bc8865) is where to confirm it. Then the two decisions in section 5,
items 15 and 16, and CodeQL alert 175 on the picture cache, which is a maintainer's dismissal.

Done 2026-10-31, on `claude/amazing-bardeen-317rgu` (pushed; no pull request; section 4 says what ran). **First, next session:**
decide with the maintainer whether to open a draft pull request, because nothing on GitHub has seen this branch and the jobs most
likely to find what a container could not are `a11y.yml` (locked axe-core and Playwright, not the versions I ran) and the aarch64
build. Then the organisation decision in section 2, which gates the Patterns regrouping and the Method-and-export panel, and then
*the chorus assembling*, which is one query and one picture.

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
