# 08 — The existing backlogs, re-checked against the tree

Re-checked on 2026-11-22 against `9d8bb1c` (branch `claude/quirky-johnson-rst020`, identical to `main`). Read-only:
nothing was built or run with cargo. Every status cites the command or file:line it rests on. Status words:
**STILL-OPEN** (the gap is in the code today), **FIXED**, **STALE** (superseded, decided, or no longer true as
written), **MAINTAINER-ONLY**, **UNVERIFIABLE-HERE**. Themes are given for STILL-OPEN rows only. "Sev" is the
severity the source recorded ("—" where it recorded none).

## State of the repo today

- `git rev-parse --short HEAD` → `9d8bb1c`; `git log -1 main` → the same commit (merge of PR #284, 2026-11-12).
- `grep '^version' Cargo.toml` → `0.19.0`; `CHANGELOG.md` has `## [Unreleased]` (line 8) above `## [0.19.0] - 2026-10-31` (line 202).
- `git ls-remote --tags origin` → `v0.19.0` is annotated and peels to `ec6f71e` (merge of #277, the `sel` fix): **the tag
  was moved as HANDOFF §6 asked**. `list_releases` (GitHub API) → v0.19.0 **published 2026-11-01 14:33 UTC**, 8 assets
  (`gh api …/releases/tags/v0.19.0`); latest release is v0.19.0, previous v0.18.1 (2026-10-26).
- `git log --oneline -40` → **#281 was merged** (`c7f69d9`, 2026-11-05) and #284 after it (`9d8bb1c`); 32 commits since the 0.19.0 commit `06a066e`.
  `git tag --contains HEAD` → nothing (no local tags; HEAD is unreleased).
- `list_pull_requests state=open` → three Dependabot PRs, #285, #286, #287 (2026-11-16). HANDOFF's #267/#268 are gone.
- **`main` is red.** CI run 43311161060 on `9d8bb1c`: job "Clippy (pedantic + nursery)" failed in 39 s, 6 errors in
  `birdnet-core`, all `clippy::map_unwrap_or` on `.map(..).unwrap_or_default()` (clippy 1.100.0, newer than the
  1.99 floor): `audio/spectrogram/live.rs:128`, `detection/daemon/mod.rs:477` and `:493`, `i18n.rs:503` and `:670`,
  and one more whose header was cut from the log tail. Every other CI job passed (tests x86_64 and aarch64, MSRV, rustdoc, real-model inference).
  Mutation testing failed (push run 43311161138 and schedule run 43837159692; the scheduled one failed only in "CI config (paths, shards, timeouts)").
  Scorecard failed at signing its results (Fulcio OIDC error), not on a check. No backlog document records any of this.

## docs/HANDOFF.md

Header (lines 10–49) describes PR #281 as an unmerged draft and v0.19.0 as "tagged and not released": both are false now (see above).

### Section 1 — repository settings (re-read with `gh api`, 2026-11-22)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| 1.a | Private vulnerability reporting off | MAINTAINER-ONLY | `…/private-vulnerability-reporting` → `{"enabled":false}` | | — |
| 1.b | Dependabot alerts off | MAINTAINER-ONLY | `…/dependabot/alerts` → 403 "disabled for this repository" | | — |
| 1.c | `protect-main` ruleset disabled | MAINTAINER-ONLY | `…/rulesets` → `enforcement: disabled`; Scorecard Branch-Protection 0 on `9d8bb1c` | | — |
| 1.d | Old tags lightweight | MAINTAINER-ONLY | now **6 of 27** tag refs annotated (`ls-remote`, `^{}` lines), not 3 of 24; v0.19.0 is annotated | | — |
| 1.e | Labels missing (`needs-triage`, `dependencies`, `rust`, `ci`, `docker`) | MAINTAINER-ONLY | `…/labels` → the nine defaults only | | — |
| 1.f | Topics none | MAINTAINER-ONLY | `topics: []` | | — |
| 1.g | Licence `NOASSERTION` | MAINTAINER-ONLY | `license.spdx_id` = `NOASSERTION` | | — |
| 1.h | Discussions off | MAINTAINER-ONLY | `has_discussions: false` | | — |
| 1.i | Open PRs #267, #268 | STALE | open now: #285, #286, #287 | | — |
| 1.j | Code-scanning alert state (175 etc.) | UNVERIFIABLE-HERE | `…/code-scanning/alerts` → 403 to this token; needs a maintainer token | | — |

### Section 2 — maintainer decisions

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| 2.1 | Default classifier V2.4 | STALE (decided) | `installer/lib/52-classifier.sh:20`. But see N-1 below | | — |
| 2.2 | V3.0 privacy default: document or build a rule | MAINTAINER-ONLY | no VAD in tree (`git grep -il silero` → only a doc comment, `inference/classifier.rs:19`) | | — |
| 2.3 | Licence detection | MAINTAINER-ONLY | as 1.g | | — |
| 2.4 | Licence fit of the l18n name tables | MAINTAINER-ONLY | `crates/birdnet-core/src/i18n.rs:20-24` | | — |
| 2.5 | Publish to crates.io | MAINTAINER-ONLY | crates.io API → 404 for `birdnet-core`, `birdnet-behavior` (unpublished) | | — |
| 2.6 | Workspace policy for the five cast lints | MAINTAINER-ONLY | HANDOFF's own script at `9d8bb1c`: **484** `#[allow]`, 463 without reason, 830 entries, 624 cast (75%) | | — |
| 2.7 | Fetch V2.4 from the `models-v2.4` mirror | MAINTAINER-ONLY | `model_catalog.rs:221-230` still points at the upstream repository; `models-v2.4` named nowhere in the catalogue | | — |
| 2.8 | Reboot/Shutdown, scheduled update, LAN no-login (A-16..18) | MAINTAINER-ONLY | `installer/lib/65-service.sh:226` filters `@reboot`; no route; `docs/book/guides/migration.md` records none of the three | | — |
| 2.9 | Host MData v1 range filter | MAINTAINER-ONLY | — | | — |
| 2.10 | Accept/amend `STABILITY.md` | MAINTAINER-ONLY | `STABILITY.md:3` "Status: proposed" | | — |
| 2.11 | Release signing | MAINTAINER-ONLY | Scorecard Signed-Releases 0 ("v0.19.0 not signed", "does not have provenance") | | — |
| 2.12 | Docker quickstart fetched from `main` | MAINTAINER-ONLY | `README.md:117` | | — |
| 2.13 | Multi-station | MAINTAINER-ONLY | see multi-station table | | — |
| 2.14 | Setup code for the first-run wizard | MAINTAINER-ONLY | — | | — |
| 2.15 | Product calls agents made | MAINTAINER-ONLY | review list only | | — |
| 2.16 | Analytic organisation A/B/C | STALE (decided A) | remainder is 5.31 | | — |

### Section 5 — open items (rows 23, 29, 30, 33, 34, 35 are already marked done in the file and are omitted)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| 5.1 | No macOS build on pull requests | STILL-OPEN | `grep -l macos .github/workflows/*.yml` → `release.yml` only | RELEASE/CI | — |
| 5.1b | Why Docs `deploy` failed on a tag's first attempt | UNVERIFIABLE-HERE | needs Actions history and the `github-pages` environment settings | | — |
| 5.2 | Imported `apprise.txt` not in the full backup (A-24) | STILL-OPEN | `routes/admin/system_controls/backup.rs:216-246` archives the DB snapshot, `birdnet.conf`, recordings only; file is `apprise-from-birdnet-pi.txt` (`migration/conf_import.rs:280`) | RELIABILITY | Medium |
| 5.3 | Interface English only (A-5) | STILL-OPEN | no message catalogue in `git ls-files`; `birdnet-core/src/i18n.rs` translates species names only | UI | Medium, large |
| 5.4 | Classifier fixed at 2 threads (A-7) | STILL-OPEN | `inference/model.rs:80` `num_threads: 2`; `src/daemon/config.rs:243` "comes from the model's defaults" | RELIABILITY | Medium, small |
| 5.5 | V3.0 privacy / VAD (G-13) | STILL-OPEN (decision first) | as 2.2 | SECURITY | — |
| 5.6 | Settings needing restart (P-7) | STILL-OPEN | *Corrected by WP-00:* the count is not stale. A scratch unit test calling `applies::every()` on 2026-11-22 printed 108 entries (74 `APPLIES` + 34 extras): 92 Restart, 12 Now, 3 Soon, 1 RecordOnly, so HANDOFF's "92 of 108" holds; the 117/97 first written here was wrong | RELIABILITY | Low |
| 5.7a | Weather per detection (A-6) | STILL-OPEN | `routes/pages/detection_detail.rs` has no "weather" | UI | Low |
| 5.7b | Per-species actions (A-22) | STILL-OPEN | `git grep -il species_action` → nothing | NON-TECHNICAL/SETUP | Low |
| 5.7c | Public address env-only (A-25) | STILL-OPEN | `config/known_keys.rs:315` "read from the environment as BNB_PUBLIC_URL" | NON-TECHNICAL/SETUP | Low |
| 5.7d | No certificate upload/generate page (P-22) | STALE (declined) | `routes/admin/https.rs:17-24` records why both are not built | | Low |
| 5.7e | Data-volume check judges one path (P-4) | UNVERIFIABLE-HERE | not traced | | Low |
| 5.7f | No OIDC (G-20) | STILL-OPEN | `git grep -il` for oidc and openid → only `static/terrain.js` (unrelated); no `X-Forwarded-User` | SECURITY | size L |
| 5.8 | Known debt | STALE (duplicate) | see TECH_DEBT table; this item omits TD-9 (added `b5f8b8d`) | | — |
| 5.9 | Whole-history public pages slow; non-script markup injection | UNVERIFIABLE-HERE | needs a load test and an escaping sweep | | — |
| 5.10 | `--doctor` range errors warn, not refuse | STILL-OPEN (decision) | `src/doctor/render.rs:101-106` `START_CRITICAL` holds 4 names | RELIABILITY | — |
| 5.11a | Release/docs/fuzz workflows never run whole | STALE | release published 2026-11-01 (8 assets); `fuzz.yml` has 41 `pull_request` runs | | — |
| 5.11b | SemVer check accepts `+build` | STILL-OPEN | `.github/workflows/release.yml:105` `(\+[A-Za-z0-9.]+)?` | RELEASE/CI | — |
| 5.12 | Floating container tag | STILL-OPEN | `install-smoke.yml:120` `ubuntu:24.04`; `uses:` pinned 133 of 133 (recounted) | RELEASE/CI | — |
| 5.13 | Dependency versions | STALE | `Cargo.lock`: ort 2.0.0-rc.15, tokio 1.54.1, rustls 0.23.46; htmx 2.0.11; the live list is Dependabot #285–#287 | | — |
| 5.14 | Suggestions for `CLAUDE.md` | MAINTAINER-ONLY | none of the six is in `CLAUDE.md` | | — |
| 5.15 | Plate index covers 7.03% | MAINTAINER-ONLY | `tools/plate-index/report.md:8`; accepting "No restrictions" files is a licence call | | Low–Medium |
| 5.16 | Works' years typed constants; plate captions unchecked | UNVERIFIABLE-HERE | needs bibliographic check | | Low |
| 5.17 | Plates drawn smaller than photographs | UNVERIFIABLE-HERE | needs a render | | Low |
| 5.18 | Uncredited legacy pictures not swept | UNVERIFIABLE-HERE | a grep for a sweep found none (`species_images/cache.rs:540` replaces on read); not conclusive | | Low |
| 5.19 | Empty/loading/failed vocabulary per page | UNVERIFIABLE-HERE | design judgement; needs a render | | Low |
| 5.20 | Terrain flat under Plate style | UNVERIFIABLE-HERE | needs a render | | Low |
| 5.21 | Plate request per species | STILL-OPEN (by design) | `pages/dashboard/plate/mod.rs:263` answers 204 | UI | Low |
| 5.22 | Plate unreadable on small screens | STILL-OPEN | `static/plate.js:40` `MIN_SHEET_PX = 1180` redirects to Numbers | UI | Medium (small display) |
| 5.24 | View-module unit tests in no workflow | STILL-OPEN | a grep of `.github/workflows/*.yml` for `unit/` and `.test.mjs` → nothing; 4 `*.test.mjs` in `tools/visual-qa/unit/` | RELEASE/CI | Low–Medium |
| 5.25 | axe 4.14 label-content-name-mismatch | FIXED | `924de5b`; `templates/layout.html:65` `aria-label="BirdNet Behavior home"`; lockfile now axe-core 4.14.0 | | Low |
| 5.26 | CLS over 0.1 on three routes | UNVERIFIABLE-HERE | needs a browser; `cls.mjs` in no workflow | | Low |
| 5.27 | Daylight rail stroke 2.85:1 | STILL-OPEN | `static/css/app.css:4797` `.ck-rail-day { stroke: var(--dawn) }` | UI | Low |
| 5.28 | Plan owes cohort retention, gap sensitivity, funnel probes | STILL-OPEN | no `cohort` in `routes/` | CHARTS | — |
| 5.31 | Four layers on 6 cards only | STILL-OPEN | `frame::method` called only from `pages/behavioral.rs:91` and `behavioral_assembly.rs:220` | CHARTS | Medium |
| 5.32 | Other shares printed without denominator | UNVERIFIABLE-HERE | the sweep was never done | | — |
| 5.36 | Mutation budget gate red | STILL-OPEN | schedule run 43837159692 (2026-11-16, `9d8bb1c`): one failed job, "CI config (paths, shards, timeouts)" | RELEASE/CI | Medium |
| 5.37 | macOS runner; binary never run on a Mac | FIXED (runner) / UNVERIFIABLE-HERE (runs) | release published with `macos-15`; nothing runs it | | High (release) |
| 5.38 | `sel` markup injection blocked 0.19.0 | FIXED | `fc9cfde` (`is_valid_date`, `simple_url_encode`); release shipped from `ec6f71e` | | High |
| 5.38b | Other clock-seeded tests not held to month/year/leap boundaries; `species.rs`/`api_write.rs` length checks unread | STILL-OPEN | 17 files under `tests/`/`crates/*/tests/` call `now()`/`today()` (regex differs from the 30/31 cited); `routes/species.rs:115` `is_iso_date` checks `len()==10` first | RELEASE/CI | part of High item |
| 5.39 | Non-primary classifier NaN drops chunk | MAINTAINER-ONLY | decision recorded as open | | Medium |
| 5.40 | Merge by exact scientific name | MAINTAINER-ONLY | decision about the stored name | | Medium |
| 5.41 | Perch bar measured, not validated | STILL-OPEN | `tools/perch-bar` only; no station-recording study in tree | RESEARCH | Medium |
| 5.42 | Capture-gain reader never met a sound card | UNVERIFIABLE-HERE | needs hardware | | Medium |
| 5.43 | MSRV choice; Action release notes unread | MAINTAINER-ONLY | — | | Low |
| 5.44 | Research method record | STALE (record) | — | | Low |
| 5.45 | `cpu_temperature_alone_never_waits` slow once | UNVERIFIABLE-HERE | Coverage passed on `c7f69d9` and `9d8bb1c`. *WP-00:* failed twice more on `0019295` (1.5 s); the bound was replaced by a count of `sample()` entries (HANDOFF item 45) | | Low |
| 5.A | htmx hides middleware 4xx (CSRF 403, viewer 403, 429) | STILL-OPEN | `HX-Reswap` set only in route handlers (`routes/form.rs:25`, `pages/toast.rs:284`), none in `rate_limit.rs` | UI | — |
| 5.B | WebKit `style-src-attr` on `<select>` | UNVERIFIABLE-HERE | needs Safari | | — |
| 5.C | Phone charts scale; Data tab hint at 320 px | UNVERIFIABLE-HERE | needs a render | | — |

### Section 6 — where to start

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| 6.1 | Merge the `sel` PR | FIXED | `ec6f71e` (#277) | | — |
| 6.2 | Move `v0.19.0` | FIXED | `ls-remote` peels to `ec6f71e` | | — |
| 6.3 | Read the release run | STALE | release published; attestation not checked here | | — |
| 6.4 | CHANGELOG/CITATION date | STALE (accepted) | `CHANGELOG.md:202` 2026-10-31 | | — |
| 6.5a | README names v0.17.1 as latest; *(unreleased)* marks | STILL-OPEN | `README.md:32`, `:42`, `:44`, `:56`, `:117` | DOCS | — |
| 6.5b | `cli-help.txt` current | FIXED | CI step "Check generated CLI help is current" passed on `9d8bb1c` | | — |

## docs/TECH_DEBT.md

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| TD-2 | 12 of 24 help topics have no call site; `help_drawer` uncalled | STILL-OPEN | recounted: Sharing, Feeds, DisplayPrefs, Kiosk, AdminSettings, AdminAudio, AdminRecording, AdminRemoteAccess, AdminBackups, Glossary, Faq, MigrationImport named only in `pages/help.rs`; `help_drawer` (`help.rs:264`) called only from its test (`:363`) | UI | — |
| TD-3 | Boot journal writes an unreadable count as 0 | STILL-OPEN | `src/helpers/boot_journal.rs:27` `detection_count(conn).unwrap_or(0)`; field is `i64` (`birdnet-web/src/boot_journal.rs:32`), not the `u64` the entry says | RELIABILITY | — |
| TD-4 | Capture duplicate guard reads an unreadable list as empty; wizard silent | STILL-OPEN | `routes/admin/audio.rs:522-526` `.unwrap_or_default()`; `pages/onboarding.rs:1236-1242` `add_source` returns `()` | RELIABILITY | — |
| TD-5 | RTSP credentials on ffmpeg's command line | STILL-OPEN | `audio/capture/process.rs:779-780` `.arg("-i").arg(url)`; no `concat` in `crates`/`src` | SECURITY | — |
| TD-6 | Ten routed partials nothing links to | STILL-OPEN | all ten `.route(` lines present (`pages/health.rs:14`, `history.rs:50`, `today.rs:57-58`, `dashboard/mod.rs:29,30,37,38,39,45`); no reference outside the route and comments | UI | — |
| TD-7 | Protected clips (closed) | STALE (closed) | only "field not rendered in a browser" remains — UNVERIFIABLE-HERE | | — |
| TD-8 | Clock "cannot tell" read as recovered | STILL-OPEN (decision) | `src/integrations/station_health.rs:527` `if ntp != NtpState::Unsynced { return None }`; `3fdafa1` left it on purpose | RELIABILITY | — |
| TD-9 | Comments keyed by date, time, species | STILL-OPEN | `crates/birdnet-db/src/detection_comments.rs:201`; `pages/detection_comments.rs:289` | RELIABILITY | — |

## docs/FEATURE_GAP_ANALYSIS.md, Part 10.3 (Part 9.8 is marked superseded by it)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| A-5 | English-only UI | STILL-OPEN | as 5.3 | UI | Medium, L |
| A-23 | Field not rendered | UNVERIFIABLE-HERE | needs a browser | | closed |
| A-24 | Apprise file outside backup | STILL-OPEN | as 5.2 | RELIABILITY | Medium, S |
| A-7 | 2 threads | STILL-OPEN | as 5.4 | RELIABILITY | Medium, S |
| A-16/17/18 | Decisions | MAINTAINER-ONLY | as 2.8 | | Medium–Low |
| 5a / P-16 | `the_clock_and_temperature_are_printed_in_one_place` fails | FIXED | CI "Run integration tests" (`cargo test --workspace --tests`) passed on `9d8bb1c`; file last touched `c26c72b` | | Low |
| G-13 | VAD / V3.0 privacy | STILL-OPEN (decision) | as 2.2 | SECURITY | Medium |
| P-7 | Restart-only settings | STILL-OPEN | as 5.6 | RELIABILITY | Low |
| A-6, A-22, A-25 | | STILL-OPEN | as 5.7a–c | UI / SETUP | Low |
| P-22 | | STALE (declined) | as 5.7d | | Low |
| P-4 | | UNVERIFIABLE-HERE | not traced | | Low |
| G-20 | OIDC | STILL-OPEN | as 5.7f | SECURITY | L |
| A-8 | OBS text log | STILL-OPEN | no `realtime.log` writer (grep) | MULTI-SOURCE-CAPTURE | Low |
| A-9 | Benchmark | STILL-OPEN | no `--benchmark` flag | RELIABILITY | Low |
| A-10 | Import from birdnet-go | STILL-OPEN | `birdnet-migrate` reads BirdNET-Pi only | NON-TECHNICAL/SETUP | Medium |
| A-11, A-14 | Video embed/map; range what-if | UNVERIFIABLE-HERE | not traced | | Low |
| A-12 | ffmpeg path/args | STILL-OPEN | no `FFMPEG_PATH` in `known_keys.rs` | MULTI-SOURCE-CAPTURE | Low |
| A-13 | Folder picker | STILL-OPEN | no `filesystem/browse` route | UI | Low |
| A-21 | Clip bitrate/gain | STILL-OPEN | no such key in `known_keys.rs` | UI | Low |
| G-6 | Extended capture | STILL-OPEN | no extended-capture code (grep) | MULTI-SOURCE-CAPTURE | as Part 2 |
| G-10 S6 | Bat classifier | STILL-OPEN (blocked) | `migration.rs:368` `CHECK (sample_rate IN (…, 48000))`; only `embed` groundwork (`model.rs:720`) | MULTI-MODEL | as Part 2 |
| G-11 | Backends beyond CPU | STILL-OPEN | no `ExecutionProvider` in `crates` | RELIABILITY | as Part 2 |
| G-12 rem. | Species preview shows configured, not effective, threshold | UNVERIFIABLE-HERE | `admin/species/learned.rs:98` reads `effective_threshold`; the `/test` preview was not traced | | — |
| G-14 | Ultrasonic filter | STILL-OPEN (blocked on G-10 S6) | no ultrasonic path | MULTI-MODEL | as Part 2 |
| 10.5.3 | Re-run the path check | STALE | file last changed `c26c72b`; not re-run here | | — |

## docs/RELEASE_PUNCHLIST.md (header: superseded; no checkboxes; only P3-4 and P3-5 remainders)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| P3-4 | Migration "missing species" stub | FIXED | `routes/pages/migration.rs:538-540` (the card is now computed; "Forecast model pending" gone) | | P3 |
| P3-5a | Cached image URL not persisted across restart | STILL-OPEN | `species_images/cache.rs:361` `scan` rebuilds `url: String::new()` | SECURITY | P3 |
| P3-5b | Doctor reports image cache only when configured | STILL-OPEN | `src/doctor/paths.rs:43-46` | DOCS | P3 |
| P3-5c | `species_image_info` not blacklist-aware | STILL-OPEN | `routes/images.rs:34` has no blacklist check; `/file` path does (`:268-281`) | SECURITY | P3 |

## docs/RELEASE_PLAN.md (header: complete; 0 open boxes, 9 ticked; "still open" list at lines 718–812)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| RP-a | Upgrade path never executed | FIXED | `upgrade.yml` passed on `9d8bb1c`; `installer/test/upgrade-e2e.sh` exists | | — |
| RP-b | Inference latency, injection failure, thermal headroom, multi-distro on real boxes, unplug transition, Accuracy step on a board, season soak | UNVERIFIABLE-HERE | needs a Pi / real hosts | | — |
| RP-c | 0.75 default confidence unmeasured | STALE | default is now 0.70 (HANDOFF §2) | | — |
| RP-d | MQTT has no settings section | FIXED | `admin/settings/extra.rs:67-68,86` "Home Assistant & MQTT" | | — |
| RP-e | `audio::quality` does not gate inference | STILL-OPEN | only `src/integrations/acoustic_health.rs:167` calls `assess_quality` (observe only) | PRECISION/ACCURACY | — |
| RP-f | Docker image never built end to end | FIXED | Docker workflow passed on `9d8bb1c` | | — |
| RP-g | `--verify-extension` under systemd undocumented | UNVERIFIABLE-HERE | needs systemd | | — |

## docs/UNATTENDED_DEPLOYMENT_AUDIT.md (last changed `f895361`, 2026-10-13)

Register rows matched by the header `| ID | P<n> |` with a script: **285** rows. Rows carrying no `[FIXED…]`/`DONE`/`disproven`
marker: P1 9, P2 99, P3 29; P1 rows marked fixed in part: 8. Only the P0/P1 rows not wholly fixed were re-checked:

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| PS-4 | 82.6 KB written per detection | UNVERIFIABLE-HERE | needs a block-layer measurement | | P1 |
| PS-6 | Quarantined stores never pruned | UNVERIFIABLE-HERE | not traced | | P1 |
| PS-7 | Segments written under final names; orphan pass (S-14) | UNVERIFIABLE-HERE | not traced | | P1 |
| PS-9 | No Prometheus metric for the write probe | UNVERIFIABLE-HERE | not traced | | P1 |
| PR-3 | Stream tmpfs and `MemoryMax=1G` share a budget | STILL-OPEN | `installer/lib/65-service.sh:177` `MemoryMax=1G`, `:192` `PrivateTmp=yes`; `62-config-file.sh:246` `STREAM_MAX_MB=512` | RELIABILITY | P1 |
| PR-4 | "FP32 model is mmap'd" justification | FIXED | `65-service.sh:168-174` now says it is NOT mmap'd and gives measured RSS | | P1 |
| LC-3 | No working remote upgrade on bare metal | STILL-OPEN | `routes/admin/update.rs:20` route, no UI caller found; `65-service.sh:189-191` `ProtectSystem=strict`, `ReadWritePaths` excludes the binary's directory | RELIABILITY | P1 |
| LC-5 | install.sh paths untested | STALE (largely) | `upgrade.yml`, `install-smoke.yml` passed on `9d8bb1c`; idempotency/uninstall coverage not traced | | P1 |
| OB-14, ON-10, DD-4, DD-9 | | FIXED | each row carries its fix commit; remainder text not re-traced | | P1 |
| S-1 | Windows never cross a segment boundary; tail zero-padded | STILL-OPEN | `detection/pipeline.rs:198-207` (tail padded to BirdNET-Pi's 1.5 s rule, a parity choice); no cross-segment carry | PRECISION/ACCURACY | P1 |
| R-5 | No re-analysis path | STILL-OPEN | raw segments are kept (`disk/manager.rs:40-49`, R-4) but no re-analysis command or route (grep) | RESEARCH | P1 |
| R-17 | Exports read `detections`, not the analytic view | FIXED | `routes/export/csv.rs:49`, `export/birddb.rs:48` call `analytic_detections` | | P1 |
| FR-2 | Soundscape destroyed | FIXED | `RawKeep` duty-cycle retention, `disk/manager.rs:40-49` | | P1 |
| FR-3 | No deployment record (mic, height, habitat) | STILL-OPEN | no such columns in `birdnet-db` (grep) | RESEARCH | P1 |

The 128 P2/P3 rows without a fix marker were **not** re-checked (see "Not checked").

## docs/design/ux-audit-2026-10.md ("What is still open")

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| 10 | Plates smaller than photos | STALE (duplicate of HANDOFF 5.17) | | | low |
| 11 | Empty/loading vocabulary | STALE (duplicate of 5.19) | | | low |
| 12 | Today hero | STALE (by design; duplicate of 5.19) | | | low |
| 13 | Terrain flat | STALE (duplicate of 5.20) | | | low |
| 14 | Legacy uncredited pictures | STALE (duplicate of 5.18) | | | low |

## docs/design/tier-c-proposals.md (last changed 2026-09-07)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| TC-1 | Multi-model and gallery | STALE | G-10 Stages 1–5 shipped (`inference/registry.rs`, `detection/merge.rs`, `model_catalog.rs`, `detection_model_scores`); the header's "Nothing in this file is implemented" is false for it | | — |
| TC-2 | MySQL | STALE (declined) | G-31 DECLINED | | — |
| TC-3 | OIDC / trusted-header SSO | STILL-OPEN (decision) | no `X-Forwarded-User` in `crates/birdnet-web/src` | SECURITY | — |
| TC-4 | UI localisation | STILL-OPEN | as 5.3 | UI | — |
| TC-5 | Settings hot-reload | STILL-OPEN | as 5.6 | RELIABILITY | — |

## docs/design/multi-station.md

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| MS-1..5 | Who the stations are; hub location; clip policy; scale; one hub or tree | MAINTAINER-ONLY | nothing built (no hub ingest route found by grep) | | — |

## docs/architecture/12-risks.md and 13-implementation-status.md

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| RK-1 | FP16 accuracy on a real Pi | UNVERIFIABLE-HERE | needs a Pi | | — |
| RK-2 | Large-dataset DuckDB sync benchmark | UNVERIFIABLE-HERE | no benchmark suite traced | | — |
| RK-3 | WAL contention load test | UNVERIFIABLE-HERE | — | | — |
| IS-1 | Module map omits the multi-classifier work | STILL-OPEN | `grep -ci` in 13 for perch, classifier, `registry.rs`, `merge.rs`, `model_catalog` → 0 each; file last changed `fa32b29` (2026-10-20) | DOCS | — |

## New findings made while checking (not in any backlog)

| Ref | Item | Status | Evidence | Theme | Sev |
|---|---|---|---|---|---|
| N-1 | Catalogue calls V3.0 "the station's default classifier" | STILL-OPEN | `crates/birdnet-integrations/src/model_catalog.rs:189`; default is V2.4 (`52-classifier.sh:20`) | DOCS | — |
| N-2 | Clippy 1.100 `map_unwrap_or` turns `main` red | STILL-OPEN | CI run 43311161060, 6 errors in `birdnet-core` (sites above) | RELEASE/CI | — |
| N-3 | `12-risks.md` "Target <50 MB RSS" against a unit comment measuring ~575 MB with V3.0 | STILL-OPEN | `12-risks.md` Operational Risks; `65-service.sh:168-171` | DOCS | — |

## Counts (rows in the tables above)

Counted by a script over this file's table rows (status = the first status word of the Status cell): **158 rows —
STILL-OPEN 64, MAINTAINER-ONLY 29, UNVERIFIABLE-HERE 28, STALE 22, FIXED 15.** Rows that cross-reference another
("as 5.3") are counted in each table, so the distinct open items are fewer than 64. STILL-OPEN by theme: RELIABILITY 16,
UI 12 (+1 "UI / SETUP"), SECURITY 8, RELEASE/CI 7, DOCS 5, NON-TECHNICAL/SETUP 3, RESEARCH 3, MULTI-SOURCE-CAPTURE 3,
CHARTS 2, MULTI-MODEL 2, PRECISION/ACCURACY 2. STILL-OPEN rows recorded as High or P1: 5.38b, PR-3, LC-3, S-1, R-5, FR-3.

## Documents whose headers or claims are stale (not corrected here)

- **`docs/HANDOFF.md`**: header lines 10–49 (#281 "not merged"; v0.19.0 "tagged and not released"; `main` red since
  `f54e8ab`); §1 open PRs (#267/#268 → #285–#287) and "3 of 24 annotated" (→ 6 of 27); §2 cast-lint counts (→ 484/463/830/624);
  §5 item 6 count "92 of 108"; item 8 omits TD-9; item 25 is fixed (`924de5b`); item 11's first half is stale; §6 steps 1–3
  are done. No entry records #284 or the red clippy job on `main`.
- **`docs/FEATURE_GAP_ANALYSIS.md`**: 10.3 rank 5a (P-16 gate) is green in CI; P-22 should read declined; P-7 counts; header
  "written at `3eb0055`".
- **`docs/TECH_DEBT.md`**: TD-3 says `BootRecord::detections` is a `u64`; it is `i64`.
- **`docs/RELEASE_PUNCHLIST.md`**: header says P3-4's stub is still open; it is gone (`migration.rs:538`).
- **`docs/RELEASE_PLAN.md`**: "still open" list (718–812): MQTT settings, upgrade path, Docker build and the 0.75 default are no longer true.
- **`docs/UNATTENDED_DEPLOYMENT_AUDIT.md`**: R-17's "still open for the rest" (CSV/BirdDB now read the analytic view); FR-2
  (raw segments are kept); PR-4 (the unit comment was corrected).
- **`docs/design/tier-c-proposals.md`**: "Nothing in this file is implemented" is false for Proposal 1; MySQL is declined.
- **`docs/architecture/13-implementation-status.md`**: no classifier registry, merge, model catalogue or model scores;
  migrations print 59 now (54 cited), `CHECK_FAMILIES` 30 (26 cited), both by the file's own commands.
- **`docs/architecture/12-risks.md`**: "<50 MB RSS" target (N-3).
- **`README.md`**: lines 32, 42, 44, 56, 117 name v0.17.1 as the latest release.
- **`crates/birdnet-integrations/src/model_catalog.rs:189`**: N-1.

## Not checked

- Nothing was compiled or run with cargo (task rule), so every "FIXED by test" claim rests on the cited commit and on CI run
  43311161060, not on a local run.
- `UNATTENDED_DEPLOYMENT_AUDIT.md`: the 128 P2/P3 rows without a fix marker, its §4 plan stages and §6 "What to do first";
  the fixed-row remainders (OB-14, ON-10, DD-4, DD-9).
- `FEATURE_GAP_ANALYSIS.md` Part 4 tiers (Part 10.3 and the 9.8 note supersede them), and the Part 2 rows not named in 10.3.
- Anything needing a browser, a Raspberry Pi, a sound card, systemd, macOS/Safari, or a maintainer token
  (code-scanning alerts, branch-protection classic endpoint, the attestation of the v0.19.0 assets).
- The sixth clippy error's file (cut from the log tail read); the reason the push-triggered mutation run 43311161138 failed.
- Whether the "Where BirdNET-Pi is installed" field (A-23/TD-7) renders.
