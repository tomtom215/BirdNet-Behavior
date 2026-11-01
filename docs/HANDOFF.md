# Handoff

What only the maintainer can do, what the last working sessions did and did not verify, and where the
next one should start. This file was **rewritten**, not appended to, on 2026-10-21. Sections 3 and 4 are
dated logs: each entry names the tree it describes and what ran on it. Section 5 holds the open items and
section 6 says where to start. The header and sections 5 and 6 were corrected for the state below on
2026-11-01. Facts about GitHub were read on the dates given, with the commands named; read them again before
relying on them. Anything not attributed to a command is marked as reported, read or unknown.

**State on 2026-11-01.** The latest *published* release is **v0.18.1** (2026-10-26). **`v0.19.0` is tagged and not
released**, and `main` has been red since #276 merged (`f54e8ab`, 2026-10-31 23:54 UTC):

- CI on `main` at `f54e8ab` (run 41254446443): "Tests (x86_64)" failed in "Run integration tests", which began at
  00:04 UTC on 2026-11-01, on `a_value_in_the_url_is_never_put_back_as_markup`. The steps behind it were skipped.
- `v0.19.0` was pushed at 09:24 UTC on 2026-11-01, at `f54e8ab`. Its Release run (41292972060) failed at "CI gate: test"
  on the same test, so there is no GitHub Release, no binary and no attestation. Its Docker run (41292972083) succeeded and
  pushed and signed the multi-arch manifest, which by `docker.yml`'s tag rules carries `:0.19.0` and `:latest` (the job's
  steps were read; GHCR was not queried). Its Docs run (41292972099) succeeded.
- The cause is item 38; the pull request that carries this file fixes it. **After it merges, `v0.19.0` is moved to its
  merge commit** (section 6): the maintainer chose that over cutting `v0.19.1`.

0.19.0 is the first release since v0.18.1, so its changelog section holds the work of four merged pull requests:

- #271 (merged 2026-10-29): the engine moved to DuckDB 2.0.0; the Patterns, Species and Weekly pages got one
  frame; the kiosk is a plate, with a public-domain plate tier behind it; every picture carries a credit; the
  Plate chart style and the terrain's sunrise finding; `docs/design/ux-audit-2026-10.md`.
- #274 (merged 2026-10-31): the charts' vocabulary (the day's sky, notches, discs, margin notes, stacked hour
  columns), three views each built for one question (*Every song*, *Who led, and when it changed*, *When each bird
  joined*), *Who answers whom* and *the chorus assembling* (the extension's `sequence_count` and
  `window_funnel`), the Patterns bar grouped When / Who / How with Behavior split into Visits and Sequences, the
  Method and export panel and the four layers on those six cards, `scripts/preflight.sh`, and
  `docs/design/insights-and-organisation.md`.
- #275 (merged 2026-10-31): the version bump and changelog roll, the four items #274 left open (the fixture's
  pictures no longer depend on the network, the species page's landmark, the running order's labels, the stale OSV
  waiver), a flaky webhook test that turned `main`'s CI red after #274's merge, and this file's corrections.
- #276 (merged 2026-10-31): the release workflow's macOS leg moved from `macos-14` (retired 2026-11-02) to `macos-15`,
  with a step that proves the runner is `arm64` and one that prints the binary's `minos` (item 37).

This pull request adds the `sel` fix, a seed for its sweep that spans two months on every day, and this file's corrections.
Section 4's last subsection says what ran.

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

**Open pull requests, read on 2026-10-31** (`list_pull_requests`, state open, before #275 was opened; #275 has merged
since and the pull request that carries this file was not yet open): two, both Dependabot's from 2026-10-26: #268 (the
GitHub Actions group, six updates) and #267 (the `debian` base image digest). Neither is reviewed here; the pins they
touch are the ones `.github/workflows/` and `docker/` name.

**CI on `main` at `49a8173`, read on 2026-10-31** (`actions_list`, the runs for that SHA): eleven workflow runs, ten
`success` (A11y & Visual QA, CodeQL, Coverage, Docker, Docs, Install smoke test, OSV-Scanner, Scorecard, Supply chain,
Upgrade test) and **one `failure`: CI, job "Tests (x86_64)"**, step "Run integration tests". The test was
`webhook::tests::a_redirect_within_one_origin_still_carries_the_credential` ("error sending request"), a race in the
test's own stub server and not in the product; it failed 6 of 600 runs here and was fixed in #275 (item 35).
The steps behind it in that job (doc tests, the full suite with analytics, the embedded-extension proof) were skipped
on that run. **At `b731204`, the merge of #275, all eleven push-triggered runs ended `success`** (read the same way,
2026-10-31): CI (with "Tests (x86_64)"), A11y & Visual QA, CodeQL, Coverage, Docker, Docs, Install smoke test,
OSV-Scanner, Scorecard, Supply chain and Upgrade test. (The dry run dispatched on `b731204`, run 41239225909, lost its macOS leg
to a brownout: item 37.) **At `f54e8ab`, the merge of #276, CI failed**: the header and item 38. The other workflows' push runs at that
SHA were not read.

**Mutation testing on `ec59953`** (the merge of #271, 2026-10-29, the last run that exists: the workflow's push filter
does not match the files of #274 or #275): `failure`, in the job "CI config
(paths, shards, timeouts)" only. Every one of the 29 `cargo-mutants` shard jobs succeeded. The config job's check 4
reported six shards past 75 % of their `timeout-minutes` (`inference/model.rs [3/3]` 60 of 60 minutes, 101 %;
`sqlite/queries/detections/*.rs` shards 1, 3, 4 and 5 at 97 to 101 % of 45; `validate.rs [1/2]` 43 of 50, 86 %).
That is the budget gate doing its job, and it will be red again on the Monday 09:00 UTC schedule until the budgets are
raised or the rows split; editing `mutation.yml` would start the full matrix (hours) on the pull request that makes
the edit, so it is left for its own change (item 36). #276 edited `release.yml`, which the filter matches
(`.github/workflows/**`), so it ran (run 41249887562, 37 jobs): "CI config" was green, because that pull request edited no
`timeout-minutes`, and two shards timed out at "Install ffmpeg" (15 minutes, before any mutation ran) and passed on one
re-run. On a pull request each shard mutates only the changed lines, so the matrix took 18 minutes and not hours.

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
  home, "Insights"; better as a strip at the top of Today and of each Patterns tab. **Decided 2026-10-31: A** (the maintainer's
  words were "Go with option A and build the chorus assembling"). Built for the Visits and Sequences tabs only; the other
  Patterns tabs have the grouping and not the layers (item 31).

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
  - *CI parity* (the commit after those): `scripts/preflight.sh` and `scripts/osv-check.py` (every locally-runnable CI gate in one
    command, a missing tool a failure), the rule in `CLAUDE.md` and `CONTRIBUTING.md`, and what the first runs found: a rustdoc
    `private-intra-doc-links` error in `queries.rs`, five `typos` hits (a three-letter variable in `skysong.js`, two fixture
    codes in `tools/visual-qa/unit/discovery.*`), and the root crate's `no_query_field_with_a_rendered_control_reads_a_blank_strictly` failing on
    `AssemblyQuery.hour_start`/`hour_end` (a blank hour was a 400).
  - *Option A and the chorus* (`1807fdf`, `c4d048b`, and the commit after them): `AnalyticsDb::assembly`, `queries::assembly_sql`,
    `types::{Assembly, AssemblyParams, ASSEMBLY_WINDOWS}`; `GET /api/v2/analytics/{assembly,answers}`; `viz/assembly.rs` (the disc field,
    drawn for a card and for a phone) and `pages/behavioral_assembly.rs` (the card); `frame::Method` and `static/figure-export.js`
    (Method and export, with a CSV built in the browser); `homes::{Group, grouped_subtabs}` and `static/tab-strip.js`; the
    `sequences` tab and `templates/analytics_sequences.html`; `pages/behavioral_glance.rs` (the sentences for bursts, the return-rate
    ledger and the neighbours). The neighbour flow's tooltip, its doc comment, the guide and the card's Method note were corrected
    (item 32).

- *2026-10-31, the same branch name restarted from `49a8173` for 0.19.0 (`git log 49a8173..`):*
  - *The release* : the version in `Cargo.toml` (the workspace and the eight `birdnet-*` dependency lines), `Cargo.lock`, `CITATION.cff` (with
    `date-released`), `openapi.json` (`info.version` and its example) and the book's `/health` sample; the changelog's *Unreleased* rolled
    into `## [0.19.0] - 2026-10-31`, with a new empty *Unreleased* and the two compare links.
  - *The four items #274 left open:* the fixture's pictures (`screenshot_server.rs`: `FixturePictures`, `FIXTURE_PICTURE_PATH`, served through
    `ImageCache::new`), the species page's `<aside>` (now a `<div>`) and the default of `AXE_FAIL_ON` in `axe.mjs` (every impact level), the
    running order (`viz/funnel.rs`: `outside_bands`, `time_label_spot`, nine new tests, 17 in the file, that read its markup back), and `osv-scanner.toml` (no
    waivers). Items 29, 30, 33 and 34 below say what each did and did not settle.
  - *One more found on the way:* `main`'s CI at `49a8173` was red on a flaky webhook test (item 35), fixed in its stub; and the mutation
    workflow's budget gate (item 36), left open on purpose.
  - *This file:* the header, section 1's two stale paragraphs (now read on 2026-10-31), items 29 to 36, and section 6.

- *2026-10-31, later, the same branch name restarted from `b731204` (`git log b731204..`):* the macOS leg of `release.yml` on
  `macos-15`, with `Prove this runner is Apple Silicon` and `Report the minimum macOS the binary asks for`; and the five other
  places that named `macos-14` (`RELEASING.md`, `docs/architecture/03-coding-standards.md`, `docs/book/field/macos.md`,
  `packaging/macos/verify-macos.sh`, the 0.19.0 changelog). Item 37.

- *2026-11-01, the same branch name restarted from `f54e8ab` (`git log f54e8ab..`):* the `sel` fix in `pages/history.rs` (the
  boundary, the sink, four tests), the sweep's `seed_sql`, the changelog entry, the lesson in `CLAUDE.md`. Item 38.

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

### 2026-10-31, later the same day (option A, the chorus, and a CI-parity pass)

The tree was `c4d048b` plus 37 uncommitted files, the day's work before the commit that follows it; `~/.duckdb` absent. The gate list was
built from `.github/workflows/*.yml` with the CI-parity preflight skill, then turned into `scripts/preflight.sh`.

| Command | Printed |
|---|---|
| `scripts/preflight.sh --msrv` (tools: typos 1.51.1, cargo-machete, cargo-deny 0.20.2, cargo-audit 0.22.2) | exit 0; 18 gates `ok`: fmt, typos, machete, installer drift, deny, audit, OSV, clippy at default, all and no default features over the **workspace**, rustdoc `-D warnings`, build, CLI help current, build all features, manual links, licence notices, doc tests, **`cargo check` on rustc 1.95.0** (`--workspace --all-features --all-targets --locked`) |
| the same gates, first run, before fixes | **rustdoc failed** (`answers_sql`'s public doc linked the private `ordered_pattern`); `typos` 1.51.1, the version CI pins, found **5 hits** (`skysong.js`, two fixture codes in `tools/visual-qa/unit/discovery.*`); both fixed. The three clippy runs passed |
| `cargo test -p birdnet-behavior` (root crate), 13 batches | first run: 1564 passed, **1 failed**: `no_query_field_with_a_rendered_control_reads_a_blank_strictly` (`AssemblyQuery.hour_start`/`hour_end`). Fixed with `routes::form::blank_is_none`; `a_blank_hour_is_not_given_rather_than_refused` returned 400 against the old handler and passes now. Final run on the finished tree: 13 of 13 batches exit 0, 78 `test result:` lines, **1565 passed, 0 failed** |
| `cargo test -p birdnet-web --features analytics` over every file of `crates/birdnet-web/tests/` (six at a time) and `--lib`; `-p birdnet-behavioral --features analytics --lib`; `-p birdnet-db --lib` | 26 of 26 batches exit 0, 151 `test result:` lines, **670 passed, 0 failed**; lib 1481, 132 and 501 passed |
| `python3 scripts/osv-check.py` (the database `osv.yml`'s scanner reads, the same six lockfiles, `osv-scanner.toml` applied) | 649 packages, 0 advisories match; the waiver matches nothing (item 34). Counterparts on a planted `smallvec 0.6.9`: 9 advisories, exit 1; waived to 2099, exit 0; waived to 2020, exit 1 |
| `cargo deny --all-features check`; `cargo audit --deny warnings` | "advisories ok, bans ok, licenses ok, sources ok"; 1395 advisories loaded, 544 crates scanned, exit 0 |
| browser gates against a fixture rebuilt from the tree each time (Chromium 1243, axe-core 4.11.4) | every gate exit 0 on its last run. On the **final tree**: `relational`, `interactions`, `dom-sinks`, `figures --fail`, `legibility`. After the full-width layout change but **before** the `skysong.js` rename and the route fix: `layout`, `axe`, `cls`, `focus`. On earlier builds only: `motion`, `charts`, `base-path`, `tokens`, `chart-ramps`, `surface-layers`, `token-doc`, the two self-tests (no CSS changed since, but they were not re-run). `axe` prints 12 moderate findings, one rule on the species page (item 30). `legibility` graded 940, 925, 925 and 941 labels in four runs of this branch; the fixture is seeded from the clock and nothing that draws SVG text changed between them, so I did not chase it |
| `STRICT=1 node qa.mjs`, three runs on this branch | 4 states with `allSpeciesPhotosFailed(6/6)` (before the session's work), **0 pages with issues** (184 screenshots, 10:30 UTC), 4 states again (12:42 UTC) on the same code. Item 29: the six avatars' pictures come from Wikipedia through this container's shared egress, which answered 429; CI's a11y job passed this step on every run I could read (15 successes in the last 30, the other 15 being 12 cancelled and 3 failures; I read the failing step of one of them, the Focus gate, a real finding, and not the other two) |
| mutants, one site each, applied and restored with `touch`: the three sentences and the kind counting (8, then 6, with the two that shared a test run singly), the phone drawing of the chorus (8, singly), the figure's tooltip wording; `relational.mjs MUTATE=notabreveal` and `widephone` | each mutant failed at least one test it was meant to; the first round's 8 each had a failing test no other mutant in it could cause; `notabreveal` fails "the tab you are on is in view", `widephone` the two phone-drawing checks. The new `relational.mjs` check "no return rate is broken over two lines" failed on the half-width card (3 cells at 1100 to 1440 px, 33 at 1000) and passes on the full-width one |
| `node tools/visual-qa/unit/{skysong,seasons,discovery,figure-export}.test.mjs` | 47, 38, 38 and 7 passed |
| after the commit, on the clean tree: `cargo package --workspace --locked --no-verify`; `scripts/check-crate-licences.py --self-test` and on `target/package`; the 8 MiB size loop from `supply-chain.yml` | packaged; "self-test: 0 failure(s)"; "all crate tarballs carry the licence files"; no tarball over 8 MiB |

**Not run (2026-10-31, later):** CodeQL (a pull request with new JavaScript, `figure-export.js` and `tab-strip.js`, is graded by it; I read the new scripts for
the sinks its queries look at and found none that a gate had not already covered, which is not a CodeQL result); the container image, the install and upgrade
smoke tests and the aarch64 cross-check and native tests (Docker, systemd and a cross toolchain are not here); `docs.yml`'s versioned build and its browser
check of the built site (mdBook is not installed; the in-app render of the same sources was link-checked); the coverage and inference-model jobs;
`mutation.yml` (the diff touches none of its paths, so it will not run); the core, integrations, migrate, scheduler and timeseries crates' own tests (nothing in them changed).

**Not verified (2026-10-31):** that CI as locked passes (`package-lock.json` pins `axe-core` 4.13.0 and Playwright 1.64.1; I ran
4.11.4 and 1.63.0); `terrain` and the other `a11y.yml` steps not listed above; the `ILLUSTRATION=plate` variants; Firefox and WebKit; a Raspberry Pi; clippy under `--no-default-features` and
`--all-features` and over the rest of the workspace; the run time of `sequence_count` over 56 ordered pairs on a Pi; and
whether the CLS result below is a regression (an earlier comparison against a baseline checkout said it was not, and its
numbers were not kept).

### 2026-10-31, the 0.19.0 branch (`claude/amazing-bardeen-317rgu` restarted from `49a8173`, `~/.duckdb` absent)

The tree was `49a8173` plus 17 uncommitted files, the work of the commit that follows it. Each row names what ran on which state of the tree.

| Command | Printed |
|---|---|
| `scripts/preflight.sh --msrv`, first run | 16 `ok`, **2 FAILED**: clippy at default and at all features, `this function has too many lines (201/200)` on `morning_order` (the dot-avoidance had added to it). Fixed by moving the sentence under the picture into `note()` and the dot position into one closure, `centre`. |
| `scripts/preflight.sh --msrv`, on the final tree | exit 0; 18 gates `ok`: fmt, typos, machete, installer drift, deny, audit, OSV, clippy at default, all and no default features over the workspace, rustdoc, build at default and all features, CLI help, manual links, third-party notices, doc tests, MSRV (Rust 1.95). "Not run": workspace tests through the script (run by hand, below), CodeQL, the image, install and upgrade smoke, the aarch64 cross-check. |
| `cargo test -p birdnet-web --features analytics` over every file of `crates/birdnet-web/tests/` (six at a time) | 26 of 26 batches exit 0, 151 `test result:` lines, 670 passed, 0 failed (before the three refactors named below) |
| `--lib`: `birdnet-web` (analytics), `birdnet-behavioral` (analytics), `birdnet-db`, `birdnet-integrations` | 1,490 (1,491 on the final tree), 132, 501 and 445 passed, 0 failed |
| `cargo test -p birdnet-behavior` (root crate), 13 batches | 1,564 passed, **1 failed**: `no_async_function_blocks_on_the_database_or_the_filesystem` named `morning_order` as blocking. The sweep resolves a bare call by name, and a closure in `meets_box` called `inside` was read as `birdnet-migrate`'s `inside`, which reads the filesystem. Found by printing the sweep's blocking keys and intersecting them with the identifiers I had added; green on `HEAD`'s `funnel.rs`, red on mine, green after renaming the closure (`holds`, with a comment saying why). That target alone was re-run after each later edit (6 passed); the 13 batches were not run again. |
| `funnel.rs` unit tests | 18 pass (9 new). Single-site mutants, each applied and restored with `touch`: F1 to F8 (the words' band, no cut, pieces that never join, first spot always, the edge costing nothing, words never in the way, towards the outside, no candidates above) and D1 to D4 (dots not passed, a line weighing one dot, a dot not counted, a dot's radius 1 instead of 3): **12 of 12 red** (run before the three refactors below). D2 and D4 first survived; the crowd in `a_time_gives_way_to_a_dot_but_gives_a_dot_up_before_a_line` and the radius assertion in `a_time_is_not_written_over_another_mornings_dot` killed them. F6's text no longer matched after the weighting changed and was rewritten. |
| digest of `morning_order`'s output over four data sets at two clock ranges, before and after the closure rename, `centre` and `note` | `243f9659cba732e2` both times: those three refactors change nothing it draws (VALIDATED for those inputs, not proven) |
| `a_redirect_within_one_origin_still_carries_the_credential`, one process per run | before: 3 failures in 300 idle, 3 in 300 with four busy loops. A scratch probe of the same server shape with the full error chain: 2 of 1,500 `Connection reset by peer`, and 0 of 1,500 with `Connection: close` on the `302`. After the fix: 0 of 1,500 (item 35). |
| `python3 scripts/osv-check.py` | 649 packages, 0 advisories match, no waivers (item 34) |
| browser gates against a fixture rebuilt from the tree (Chromium 1243, axe-core 4.11.4), tree before the three refactors | every gate exit 0: `axe.mjs` **0 total violation(s)** with `AXE_FAIL_ON` at every impact level (12 on the species page before the `<aside>` became a `<div>`), `interactions` 80 passed 0 failed, `legibility` 973 labels on 46 routes at two widths (the count varies run to run: 913 to 973 over six runs here), `layout` 360 page states, `tokens` 91, `chart-ramps` 146, `surface-layers` 30, `focus` 2,684 stops with no missing or weak ring, `figures` 2,231 figures, `motion`, `dom-sinks` 50, `relational`, `charts`, `cls`, `qa-selftest`, `layout-selftest`, `token-doc`. |
| `STRICT=1 node qa.mjs`, same tree | **0 pages with issues**, 184 screenshots |
| `STRICT=1 node qa.mjs` against a fixture started with `HTTPS_PROXY`, `ALL_PROXY` pointed at a closed port | **0 pages with issues**, 184 screenshots; all 15 fixture species answer `200` for their picture, and 15 of 15 under `BIRDNET_BASE_PATH=/bn` (`base-path.mjs` PASS). The old fixture failed this (item 29). |
| the regenerated `docs/book/images/sequences.png`, looked at | the first render, at full size and magnified: "08:22" over a dot (what the new tests assert against). After the dot-avoidance, a 1140 by 620 crop of the figure at full size: no time struck by a line or over a dot, the lines broken at each lane's header row (item 33). One reading of one figure; the tests are the mechanical check. |
| after the commit `06a066e`, on the clean tree: `cargo package --workspace --locked --no-verify`; `scripts/check-crate-licences.py --self-test` and on `target/package`; the 8 MiB loop from `supply-chain.yml` | exit 0 ("Packaged 105 files, 3.2MiB (964.0KiB compressed)" for the root crate); nine tarballs at 0.19.0 (the nine left from 0.18.1 were deleted first); "self-test: 0 failure(s)"; "all crate tarballs carry the licence files"; the largest, `birdnet-web-0.19.0.crate`, is 3,081,956 bytes against the 8,388,608 limit |

**Not run:** CodeQL (the only JavaScript touched is one default in `tools/visual-qa/axe.mjs`; the rest is Rust, HTML and CSS), the container image, the install and upgrade smoke tests, the aarch64 cross-check, the full root-crate batches after the rename, the workspace tests through `preflight.sh --tests`, and `release.yml`'s dry run (an outward action; item 2 in section 6). The browser gates ran before the three refactors above, not after; the digest row is the evidence that they did not matter.

### 2026-10-31, the macOS runner (`claude/amazing-bardeen-317rgu` restarted from `b731204`)

The tree was `b731204` plus the commit `ed6473e`. A macOS runner is not available in the container, so the first row is stubs and the second is the real thing.

| Command | Printed |
|---|---|
| the two new `run:` blocks extracted from `release.yml` and run under `bash -eo pipefail` with stub `uname`, `sw_vers` and `otool` | arch step: exit 1 with `::error::the macOS runner is x86_64, not arm64` for `x86_64`, exit 0 for `arm64`. minos step: prints and writes the summary when `LC_BUILD_VERSION` is present; warns and exits 0 when it is absent and when `otool` fails. (Stubs: the real `otool -l` format was not seen until the next row.) |
| `scripts/preflight.sh --msrv`; `python3 scripts/check-ci-config.py` with `cargo-mutants` 27.1.0; `installer/test/release-tag-gate.sh` | 18 gates `ok`, exit 0; "all CI-config checks passed" (its run-duration checks 4 and 5 need `GITHUB_TOKEN` and were not read); the gate's cases all `PASS` |
| `release.yml` dispatched on the branch, version blank (run 41246486494, head `ed6473e`) | run `success`, 17 jobs. `Validate release tag`, the five CI gates, `Choose the releases to upgrade from`, `Build x86_64-unknown-linux-gnu`, `Build aarch64-unknown-linux-gnu`, `Build aarch64-apple-darwin`, `Package and attest` (attestation skipped, as in a dry run), the four `Upgrade from vA.B.C` rows: `success`. `Publish GitHub Release` and `Verify the published Release`: skipped, as in a dry run. The macOS job: label `macos-15`, `uname -m: arm64; macOS 15.8.2`, `Finished release profile [optimized] target(s) in 19m 30s`, `minos 11.0` and `sdk 15.5` from `otool -l`, job time 20 m 38 s |

**Not run:** the macOS binary on any Mac (CI builds it and never executes it); a tag push (the real `github-release` and `verify-release` jobs); the dry run on `main` after this branch merges (not needed unless `main` differs from the branch by more than documentation).

### 2026-11-01, the `sel` fix (`claude/amazing-bardeen-317rgu` restarted from `f54e8ab`)

The tree was `f54e8ab` plus 5 uncommitted files (`history.rs`, the sweep's test file, the changelog, `CLAUDE.md`, this file). The container's clock read
2026-11-01 UTC throughout, which is the point: these ran on the day the gate was red.

| Command | Printed |
|---|---|
| `cargo test -p birdnet-web --test a_query_value_is_never_echoed_as_markup` on `f54e8ab` | **failed**: "1 response(s) put a URL value back as markup: `/pages/history-calendar` (all parameters = `"><zq-p>12`)": the same text as the Release run's log |
| a scratch unit test in `history.rs` (deleted): `render_calendar` with a ten-character probe as `sel` | days in two months (10-31, 11-01): raw `<zq-p>` present; days in one month (10-30, 10-31): absent |
| `cargo test -p birdnet-web --lib routes::pages::history` on the fix | 14 passed. Mutants, one site each, restored with `touch`: the sink writes `sel` raw (**red**: `a_selected_day_is_never_put_into_the_month_buttons_as_markup`), the boundary asks only for ten characters (**red**: `the_calendar_handler_drops_a_selected_day_that_is_not_a_date`), both (**red**, two tests: the shipped bug) |
| the sweep file on the fix | 4 passed, including `the_seed_spans_two_months_whatever_day_the_sweep_runs` |
| the sweep with today pinned to 2026-11-10 (scratch edit of `station()`, restored) and the shipped bug put back | old two-row seed: **passed** (the blind spot); new three-row seed: **failed** on `/pages/history-calendar` |
| the whole suite on the fixed tree: `birdnet-web` integration tests (26 batches), `--lib` for `birdnet-web` (analytics), `birdnet-behavioral` (analytics), `birdnet-db`, `birdnet-integrations`; the root crate (13 batches); `--tests` for `birdnet-db`, `-core`, `-timeseries`, `-scheduler`, `-migrate`, `-integrations`, `-behavioral` (analytics) | every batch exit 0, 0 failed: web integration 671 passed; libs 1,495, 132, 501 and 445; root crate 1,565; `--tests`: 575, 1,043, 31, 31, 168, 560 and 157 |
| `scripts/preflight.sh --msrv` | exit 0; 18 gates `ok` (fmt, typos, machete, installer drift, deny, audit, OSV, clippy at default, all and no default features, rustdoc, build at default and all features, CLI help, manual links, third-party notices, doc tests, MSRV 1.95) |

**Not run:** the browser gates (for a real `sel` the calendar's output is what it was), the `--all-features` workspace test command as one command (the same tests ran by crate), CodeQL, the image, the install and upgrade smoke tests and the aarch64 cross-check.

## 5. Open items, with severity

Severity is a judgement. Everything here was found or confirmed in the last session unless
it says otherwise.

1. **0.18.0 was never released; 0.18.1 was (2026-10-26). Resolved, with three things left.** After #265 merged the
   maintainer pushed `v0.18.0`; its macOS build did not link (`___isPlatformVersionAtLeast`, from the CoreML objects in
   the ONNX Runtime library `ort-sys` 2.0.0-rc.15 downloads), so no Release was published and the tag was deleted. The
   fix is in `build.rs` (`link_clang_runtime`, #266). `v0.18.1` (#270) then published, and its Release, Docker and Docs
   runs on the tag all ended `success`, Docs and Release on a second attempt (read with `actions_list`, 2026-10-31; why
   the first attempts failed was not read). The Docker gate had refused both earlier tags in 9 s because
   `fetch-depth: ${{ startsWith(github.ref, 'refs/tags/') && 0 || 1 }}` is always 1 (0 is falsy); it is `fetch-depth: 0`
   and `scripts/check-ci-config.py` refuses that shape. #270 also closed a mutation-test gap
   (`DuplicateGate::is_enabled`) and raised three mutation-shard budgets. **Still open:** nothing builds the macOS target
   on a pull request, so the next break of that kind shows only at tag time or in the release dry run (a path-filtered
   macOS job in `ci.yml` would catch it); `cargo test` for `birdnet-core` on macOS is untested and probably hits the same
   symbol; and why Docs' `deploy` failed on a tag's first attempt is not established (PLAUSIBLE: the `github-pages`
   environment allows only `main`, not tags; Settings → Environments → github-pages → Deployment branches and tags).
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

23. ~~"Who answers whom" has no REST route.~~ **Done 2026-10-31**: `GET /api/v2/analytics/answers` (and `/assembly`), in
    `openapi.json`, with `the_assembly_and_answers_endpoints_answer`.
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
28. **What the plan owes** (`docs/design/insights-and-organisation.md`, sections 4 to 6): cohort retention of new arrivals, gap
    sensitivity, the four layers on the other Patterns tabs (item 31), the bursts timeline on a per-day sky, a margin note on the
    streamgraph, and a probe per funnel mode before any page names one (the extension's two documentation pages disagree about
    `allow_reentry`, `strict_order` and `strict`; the chorus uses `strict_increase`, whose behaviour was probed). `window_funnel_events`
    is still drawn by no page as a lag, and should not be (section 6). Not known: whether a station with under
    about 200 detections a day gives the answers grid enough pairs to be more than noise.

29. ~~The strict visual sweep failed on the life list in some environments.~~ **Fixed in 0.19.0.** The fixture's species pictures
    came from Wikipedia, so a run behind a proxy or a rate limit drew initials where CI drew photographs: two of three
    strict runs of #274's branch failed `allSpeciesPhotosFailed(6/6)` and one had 0 issues, on the same code. The fixture
    (`examples/screenshot_server.rs`, `FixturePictures`) now serves a credited picture and a line of prose for every
    species it knows from its own address, through the provider, download, disk and credit path the binary uses, so the
    gates grade the same pages everywhere. With outbound HTTPS pointed at a dead proxy all 15 fixture species answer `200`
    (and under a base path); a strict sweep (`STRICT=1 node qa.mjs`) against that fixture: **0 pages with issues, 184 screenshots**. **Not covered:** the sweep no longer grades what
    a real Wikipedia answer looks like (a long description, an odd thumbnail ratio); that is now the product's tests'
    job, not the fixture's.
30. ~~axe reports `landmark-complementary-is-top-level` on the species page.~~ **Fixed in 0.19.0.** The `<aside>` that held the
    best-recording card (`templates/species_detail.html`) is a `<div>`; `axe.mjs` printed 12 findings before (three states, two
    themes, two sizes, one node each) and **0 total** after, over every route. Its default for `AXE_FAIL_ON` is every impact
    level now (it was `serious,critical`, so these moderate findings were printed and read by no one). Why axe objected to this
    aside and not to the ones on Today, which sit in the same place in the same layout, was not established: its own data
    said the aside's role was null.
31. **The four layers are on six cards, not on every analytic page.** Medium. The Visits and Sequences tabs have them; When
    active, Dawn chorus, Trends, Who sings together and Migration are regrouped under When / Who and are otherwise as they were
    (some already have a sentence from `pages/insight.rs`; none has a Method and export panel). The Method panel's JSON link
    exists only where a REST route does (`sessions`, `retention`, `next-species`, `previous-species`, `assembly`, `answers`;
    the running order has none), and its CSV button saves the table on the page, not detection-level rows.
32. **A number on the neighbour card was described as something it is not.** Fixed 2026-10-31; kept here because the same
    sentence may exist elsewhere. The engine's probability is the share of the sessions that *had a bird on that side*
    (`adjacent_species_sql` says so; a probe confirmed it: a Blackbird followed by a Jay on 2 of its 7 sessions reported the Jay at
    100%). The flow's tooltip, the guide, the card's Method note and a doc comment said "share of sessions". Corrected in the
    tooltip (`the_reading_says_what_the_share_is_of`), the guide, `docs/design/charts-relational.md`, the Method note and
    `NextSpeciesPrediction::probability`; the card's new sentence states the basis and a test runs a real engine over a station
    whose answer is worked out by hand. Not searched for: other pages that print a share without its denominator.
33. ~~The morning running order's labels cross its own lines and dots.~~ **Fixed in 0.19.0** (`viz/funnel.rs`). The lines are drawn in
    pieces that leave out the band each lane's words are written in, and the time beside each large dot is put on a side its own
    line, the lane's words, the picture's edge and the other mornings' dots leave free (a line or a word weighs ten dots, so a
    crowded lane gives up a dot's face before a name). Tests read the markup back as geometry and were red on the old drawing (the
    heavy line through the right-hand count of the lanes below it; a steep morning's time struck by its own line; "08:22" over a
    dot); twelve single-site mutants of the new code are each caught. **Two limits:** each line is now visibly broken at every
    lane's header row, so "a line that slants steadily to the right" is read from pieces, not one stroke (the band is 34 of a
    lane's 84 units; narrowing it to the name and count rows would keep more line and let a typical-time label be crossed, which
    the test forbids), and a lane so crowded that no spot is free still puts a time over a dot (the least-bad spot, not none).
34. ~~`osv-scanner.toml`'s only waiver matched nothing.~~ **Removed in 0.19.0.** `GHSA-qwgh-2vcv-g2f7` was waived for block-buffer 0.10.4
    until 2026-12-01 (#272). Read from the database on 2026-10-31 (`scripts/osv-check.py`), the record was last modified 2026-10-30
    and lists `block_buffer` 0.11.0 up to 0.12.1 as affected; the lockfile holds 0.10.4 and 0.12.1, so nothing matched, and all 649
    locked packages across the six lockfiles match no advisory. If the record changes back and the job goes red, add a waiver
    then, with that day's evidence. The general point stands: an OSV finding depends on the database of the day and not on the
    branch, so a red `OSV-Scanner` job on a branch that does not touch a lockfile is `main`'s state.
35. ~~A webhook test failed `main`'s CI.~~ **Fixed in 0.19.0.** `a_redirect_within_one_origin_still_carries_the_credential`
    (`crates/birdnet-integrations/src/webhook.rs`) has a stub that answers a `302` and closes the connection without
    `Connection: close`, so the client sometimes reused the closed connection for the redirected request and met
    "Connection reset by peer". Measured on this machine, one process per run: 6 failures in 600 (3 of 300 idle, 3 of 300 with
    four busy loops). A scratch probe with the same server shape printed the error chain (`client error (SendRequest) <-
    connection error <- Connection reset by peer (os error 104)`, 2 of 1,500) and, with the header on the `302`, 0 of
    1,500; the real test with the header ran 1,500 times in separate processes with 0 failures (at 1 % the chance of
    that by luck is under one in a million). The test was added in `3301e4b` and passed on CI many times before, which is
    what an intermittent failure looks like. The other stubs that answer a redirect send the client to another origin, so
    they never reuse a connection. **Not changed:** the product's reaction to a reset (it logs it), because reusing a
    connection the peer has closed is an ordinary HTTP/1.1 race and a retry would be a feature, not a fix.
36. **The mutation workflow's budget gate is red on its schedule.** Medium (it makes `Mutation testing` red on `main`
    without any mutant surviving). See the paragraph under section 1: six shards use 86 to 101 % of their `timeout-minutes`.
    Either raise the budgets in `.github/workflows/mutation.yml` (and run `python3 scripts/check-ci-config.py
    --update-observed`) or split `inference/model.rs`, the detections glob and `validate.rs` into more shards. The
    change touches `.github/workflows/**`, so its pull request runs the whole matrix: budget a few hours, and keep it
    out of any pull request that is meant to be small. Check 4 needs `GITHUB_TOKEN` to read run history, so it cannot be
    reproduced from this container.

37. **The release workflow's macOS leg was on an image GitHub retires on 2026-11-02.** High for the release; handled in #276
    (merged 2026-10-31 23:54 UTC). The `aarch64-apple-darwin` row of `build` ran on `macos-14`. GitHub fails every job on that
    image in scheduled brownouts (ten windows from 2026-10-05, each 14:00 to 00:00 UTC; the last two are 10-31 and 11-01) and
    retires it on 2026-11-02 ([actions/runner-images#13518](https://github.com/actions/runner-images/issues/13518), read
    2026-10-31). The dry run on `main` (run 41239225909, attempt 2) lost that leg to one, and `package` and `upgrade` need the
    whole matrix. **Now `macos-15`.** From the README of `actions/runner-images`: `macos-15` is macOS 15 Arm64 and
    `macos-15-intel` the x64 image; no retirement date is announced for macOS 15 or 26; the policy is at most two GA images at
    once, the oldest beginning deprecation when a newer one is GA (macOS 14: announced 2026-01-11, brownouts from 2026-10-05,
    retired 2026-11-02). Not `macos-latest`: `check-ci-config.py` refuses a `-latest` runner, and it would move on GitHub's
    schedule. Two steps were added: one fails the job unless `uname -m` is `arm64`, one prints the binary's `minos`.
    **Seen on the branch's dry run (run 41246486494: 15 of its 17 jobs `success`, the two publish jobs skipped as a dry run skips them):** the runner printed `uname -m: arm64; macOS
    15.8.2`; the release build took 19 m 30 s; `otool -l` gave `minos 11.0`, `sdk 15.5`; the archive, checksum and upload
    steps passed; `Package and attest` and the four `Upgrade from v0.17.0 / v0.17.1 / v0.18.0 / v0.18.1` rows passed (the
    upgrade rows run the x86_64 build, not the macOS one). **Not verified:** that the macOS binary *runs*: CI builds it and
    never executes it, and nothing here is a Mac. `minos 11.0` is the header's claim (the Rust default for the target; nothing
    pins `MACOSX_DEPLOYMENT_TARGET`), not a test on macOS 11 to 14, and `README.md` and `docs/book/field/macos.md` still call
    macOS a preview that is "not yet confirmed on real hardware". Whether to promise a floor, and to pin one, is the
    maintainer's; the number is printed in every release run's summary from now on, so a change of it is seen.
38. **A fixture seeded from the clock hid a markup injection, then failed `main` and the release.** High: it blocked 0.19.0; the
    bug itself is low severity. `/pages/history-calendar` wrote its `sel` query value into the previous- and next-month
    buttons' `hx-get` after checking only that it was ten characters long (`"><zq-p>12` is ten); the buttons are drawn when the
    data spans two months, which is every station past its first month. The code is at least as old as `3301e4b5` (2026-10-22),
    which v0.18.0 and v0.18.1 contain (`git log -S`; those tags were not rebuilt). The strict CSP stops a script, so this is
    reflected markup injection of at most ten characters; whether it warrants an advisory is the maintainer's.
    `a_query_value_is_never_echoed_as_markup`, the sweep that grades every page for exactly this, seeded today and yesterday,
    which are in two months on the 1st only, so it was green on every other day. It failed `main`'s CI at 00:04 UTC on
    2026-11-01 and the `v0.19.0` Release run (it ended at 09:49 UTC); the dry runs of 10-31 could not see it. **Fixed (the pull request that
    carries this file):** `sel` must be a `YYYY-MM-DD` (`crate::routes::is_valid_date`) and is percent-encoded where it is
    written (`simple_url_encode`). Four tests on fixed dates, each seen failing against the shipped code: the sink alone
    reverted, the boundary alone reverted, and both reverted (the shipped bug). The sweep's seed is `seed_sql(today)`, with a
    third row on the last day of the month before, and `the_seed_spans_two_months_whatever_day_the_sweep_runs` holds it to
    that on eight fixed days. **Checked that the seed discriminates:** with the shipped bug and "today" pinned to
    2026-11-10, the old two-row seed's sweep passed (the blind spot) and the new one failed. **Swept:** the
    `len() == N` validity filters in `routes/`. Read: the four in `history.rs` (`history_chart` and the day page escape or
    encode the date at every sink; `month` is clamped to the data's own months), `is_ymd`, `is_valid_date` and the command
    palette's two uses of it. **Not read:** `species.rs`'s `is_iso_date` and the two in `api_write.rs`, a JSON write API.
    **Not done:** 30 other Rust test files seed from the clock and pass today (section 4), but nothing holds them to a month,
    year or leap-day boundary.
    A failing test binary stops `cargo test`, so on 2026-11-01 neither CI on `main` nor the release gate ran the binaries
    after the sweep; that is why the whole suite was run by hand on the day.
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

**Releasing 0.19.0** (`RELEASING.md` is the procedure and its pre-release checklist is the list to tick). The bump is
merged (#275, #276). `v0.19.0` exists on GitHub at `f54e8ab` and is to be **moved**; no GitHub Release exists for it.

1. **Merge the pull request that carries this file** when every check on its last head is green.
2. **Move the tag** (the maintainer's choice over `v0.19.1`; nothing was published under it except the container image):
   `git push origin --delete v0.19.0`, then `git tag -d v0.19.0`, then `git fetch origin`, then `git tag -a v0.19.0 -m
   "Release v0.19.0" <the merge commit of that pull request>` and `git push origin v0.19.0`. Name the commit: `git tag -a`
   with none tags whatever is checked out. Check the result with `git ls-remote --tags origin 'v0.19.0*'`: the peeled line
   (`^{}`) must be that commit. Consequences: Release, Docker and Docs run again; Docker pushes `:0.19.0`, `:0.19`, `:0` and `:latest`
   over the first push and signs them again, and whoever pulled `:0.19.0` or `:latest` since 2026-11-01 09:50 UTC has the build
   with the `sel` bug; there is no first GitHub Release to delete.
3. Read the Release run: this time `Package and attest` attests, `Publish GitHub Release` and `Verify the published Release`
   run, and the macOS job's summary has the `minos` line (item 37). Read the Docker and Docs runs as well (section 5, item 1
   for what to look for in Docs).
4. The date in `CHANGELOG.md` and `CITATION.cff` is 2026-10-31 and the tag lands later; `validate` checks the heading's form and
   the CITATION version, not that the date is today's, so leaving it costs nothing but a day's accuracy.
5. **After the tag**, in a small follow-up pull request: `README.md` line 32 names `v0.17.1` as the latest release
   and several features carry an *(unreleased)* mark that 0.18.0 or 0.18.1 already shipped (line 131 says `v0.17.1`
   installs the V3.0 preview). They are true or false only once the tag exists, which is why they are not in the
   release pull request. Also confirm `scripts/gen-cli-help.sh` leaves `docs/book/_generated/cli-help.txt`
   unchanged (CI's `build` job checks it).

**Maintainer decisions and settings still open** (this release changes none): the settings in section 1
(private vulnerability reporting, the five labels, topics, `protect-main`); publishing to crates.io by hand in two
batches (`RELEASING.md`, "Publishing to crates.io"); the V3.0 privacy default, whether to wire the model mirror, the
three missing controls (A-16 to A-18) and the cast-lint policy; closing A-24 (section 5, item 2); what the script
`tools/code-scanning/dismiss.sh` prints for the Scorecard items only a maintainer can settle (#1, #8, #9, #165);
CodeQL alert 175 on the picture cache (a maintainer's dismissal; its state was not read since #271); and section 5,
items 15 and 16.

**Next work on the line #274 opened:** the four layers on the five other Patterns tabs (section 5, item 31), then
cohort retention of new arrivals and gap sensitivity (item 28).

**Before every push: `scripts/preflight.sh`** (add `--msrv`; `--tests` when the disk allows). It is the workflows'
own commands at the workflows' scope, and it fails on a tool it cannot find. Install the tools it names first
(`cargo install --locked typos-cli --version 1.51.1`, `cargo-machete`, `cargo-deny`, `cargo-audit --version
0.22.2`; crates.io and PyPI are reachable from the container, GitHub release downloads are not). The pull requests
before #274 lost CI cycles to checks that were not run locally; `CLAUDE.md`, *Testing Conventions*, has the list and
the reasons. What earlier sessions learned from CI (CodeQL's path-injection class, the kiosk's focus ring, the GNU ld
link of the DuckDB archive) is there too, and section 3 is the log of what each session changed.

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
