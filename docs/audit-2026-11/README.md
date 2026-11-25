# Audit of 2026-11-22: inventory, plan and checklist

An adversarial review of the tree at `9d8bb1c` (the same commit as `main` on that date), written so
that later sessions can pick up one work package at a time. It was asked for from six angles: the UI
for a non-technical birder, the UI for people who love charts, research-grade rigour, the multi-model
pipeline, precision and accuracy, zero-setup defaults, and the handling of several microphones and
RTSP streams.

## What this is, and what it is not

- Eight audits were run in parallel by delegated sessions, one per file `01` to `08` below. Seven read
  code and wrote findings with `file:line` evidence; one (`05`) built the fixture server and looked at
  rendered pages.
- **Every finding is a reading of the code unless its entry says otherwise.** "VERIFIED-BY-READING"
  means a person-equivalent read the cited lines; it does not mean a test was seen failing. Findings
  marked CONJECTURED name the probe that would settle them. No finding's *size* (how often, how many
  detections) has been measured.
- The session that assembled this plan spot-checked six of the high-severity claims against the
  source, by opening the cited lines: the sample-rate refusal (`inference/registry.rs:329`), the
  privacy filter's `contains("human")` (`detection/privacy.rs:214`), the bare `continue` on the
  range filter (`detection/daemon/process.rs:534`), `MAX(seconds)` in the effort-corrected rate
  (`phenology/abundance.rs:322`), the existence of `shed_split` (`detection/daemon/run.rs:492`), and the colour slot chosen by
  position in a chart (`routes/pages/viz/kit.rs:587`).
  Those held. The other findings were **not** re-checked; the implementing session re-reads the
  cited lines before acting, as `CLAUDE.md` requires ("distrust confident prose in this repo's
  history, including your own").
- Statements about outside standards (Raven selection tables, Darwin Core, eBird's import format,
  occupancy models, BirdNET-Analyzer's own arithmetic) are labelled FOLKLORE in the audit files. They
  come from general knowledge and must be checked against the primary source before code or copy
  relies on them.

## The files

| File | Area | Finding ids |
|------|------|-------------|
| `01-multi-source-capture.md` | Several microphones and RTSP streams | MSC |
| `02-multi-model-pipeline.md` | Running more than one classifier | MMP |
| `03-precision-accuracy.md` | Signal path, thresholds, filters | ACC |
| `04-zero-setup-defaults.md` | Install, first run, defaults | OOB |
| `05-ui-rendered-walkthrough.md` | The pages as rendered | UIR |
| `06-ui-code-charts-ia.md` | Routes, charts, copy, states | UIC |
| `07-research-grade.md` | Provenance, effort, verification, exports | RES |
| `08-existing-backlog-reconciled.md` | The older backlogs, re-checked | (table rows) |
| `09-chart-review.md` | Every chart: keep, refine, rebuild, replace or remove; missing figures; rendering technology; methods from ornithology (added 2026-11-24, from reading code, nothing rendered) | CHT |
| `10-model-delivery.md` | How each model file reaches a station on every install path, and 18 ways a station ends up not detecting, or without its filter, with nothing on the dashboard (added 2026-11-25, a delegated read of the code, three claims re-checked, nothing run) | rows to be written by the batch that verifies them |
| `work-packages.md` | Every finding, grouped into work packages | generated |
| `checklist.json` | The same, machine-readable, with a `status` field | generated |

`work-packages.md` and `checklist.json` were generated from the headings of the audit files by a
script, so their counts are counts of headings, not of anything remembered. A session that finishes
an item sets its `status` in `checklist.json` to `done` with the commit hash in `evidence`, or to
`rejected` with the reason. It does not delete the row.

## The answers to the questions that were asked

**Are several microphones and streams handled precisely? No, in four specific ways.**

1. Nothing groups one bird heard by two sources. Each source writes its own row, by design
   (`migration.rs:455-462`), and every count, chart, analytic and BirdWeather post is a row count
   (MSC-02, MSC-06, MSC-07, RES-18). Two microphones in one garden roughly double the numbers.
2. The one place that corrects for listening time divides all sources' detections by the hours of
   the single longest-running source (MSC-03, RES-03).
3. Under load or heat the daemon analyses one segment in two, and the split is over a list sorted by
   name. When sources rotate together the same source may lose every segment (MSC-01; the code is
   read, the field behaviour is a conjecture a two-source unit test settles).
4. Comments and the quarantine are keyed by date, time and species without the source, so one
   source's record can swallow or share another's (MSC-04, MSC-05, TD-9).

Calls are not wrongly *filtered out* across sources, because no cross-source filter exists. They are
lost at segment edges instead: windows never cross a segment boundary, a tail shorter than 1.5 s is
dropped, and the corroboration and human-sound rules cannot see past the boundary (MSC-08, ACC-10).

**What is wrong with the multi-model pipeline?** The default primary (BirdNET V2.4, 48 kHz) cannot run
beside Perch (32 kHz), and the refusal stops all detection while the web page stays up; the install
command and the manual both recommend that pairing (MMP-01). Species only the second model knows
skip the location filter (MMP-03); Perch's sound classes such as "Speech" and "Engine" can be stored
and posted as species (MMP-04); the human-sound rule misses most of Perch's speech classes (MMP-05);
one threshold is applied to scores on three scales (MMP-06); exports credit every row to the primary
model's checksum (MMP-08).

**Precision and accuracy.** There is no measurement of either for the default configuration (ACC-03).
The parity study measures agreement with BirdNET-Pi. What the filters discard is neither counted nor
kept (ACC-01, ACC-02), so nobody can audit what was thrown away, and an out-of-range rarity vanishes
at any confidence. The review tools draw conclusions from as few as three to five verdicts (ACC-07,
ACC-08).

**Research grade.** Per-row provenance is mostly present. The blockers are effort (it is daily,
counts time that was not analysed, and most analytics ignore it: RES-01, RES-02, RES-14), the
absence of a validation protocol and of reviewer identity (RES-06, RES-07), no stored non-detections
or sub-threshold scores (RES-04), and no way to re-analyse audio or ingest recorder cards (RES-05).

**Zero setup.** Six or seven manual actions after the operating system is on the card, all in a
terminal; no flashable image (OOB-01). A silent or clipping microphone, a missing location and a
wrong time zone each look healthy on the first screen (OOB-02, OOB-05, OOB-06).

**UI and charts.** `06` holds the route and chart inventories (382 routes and 47 charts drawn by 7
renderers, as that audit counted them). `05` is what the pages look like: 548 screenshots were taken
and the three browser gates (`qa.mjs`, `axe.mjs`, `layout.mjs`) were green on the same fixture that
showed a species changing colour from chart to chart (UIR-01), a sentence naming Thursday beside
bars where Saturday is longest (UIR-02), a second settings application under `/admin` whose numbers
disagree with the first (UIR-15), and three signals disagreeing about whether the station is
recording (UIR-11). The screenshots were in a session scratch directory and are not kept; `05` gives
the build command and fixture settings to take them again.

## Item zero: `main` is red

Read on 2026-11-22 by the reconciliation audit (`08`), from the workflow run's job log: CI run
43311161060 fails in Clippy on six `clippy::map_unwrap_or` errors in `birdnet-core`, reported by
clippy 1.100.0. Nothing in the older backlogs records it. Fix this before any work package, on its
own pull request, and correct the header of `docs/HANDOFF.md`, which still describes #281 as an
unmerged draft and `v0.19.0` as unreleased (`08` lists every stale claim, document by document).

**WP-00 done, 2026-11-22:** `d726025` (Clippy; the six errors were only `birdnet-core`'s, and the workspace held
101 `map_unwrap_or` sites, two `redundant_clone` and four `needless_range_loop` behind them), `760e425` (the
mutation budget gate's five checks), `cc4aec3` (`HANDOFF.md` and the other stale documents; `08`'s row 5.6 count
was wrong and is corrected there), `acb3d9a` (the read-default sweep learns `map_or_default`, which
`d726025` had hidden three catalogued reads behind; found by the integration tests). Not settled by this session: whether the next scheduled mutation run is green
(its headroom and annotation checks need run history), and the README's `v0.17.1` marks.

## Order of work

The packages are in `work-packages.md`. The order below puts first what loses or misstates data,
because every chart and every research claim downstream inherits it.

| Phase | Packages | Why here |
|-------|----------|----------|
| 0 | WP-00 | A red `main` blocks every merge |
| 1. Stop losing and misreporting | WP-01 to WP-08 | Detection stops, silent discards, doubled counts, wrong denominators |
| 2. Research grade | WP-09 to WP-13 | Needs the effort and event data that phase 1 creates |
| 3. Non-technical owner | WP-14 to WP-16 | Independent of phases 1 and 2; can run in parallel with them |
| 4. Charts and polish | WP-17 | Last, so the charts are redrawn once, on correct denominators |

Dependencies that are not obvious from the table: WP-05 (effort) must land before any chart is
rescaled (UIC-01 to UIC-03, RES-14), and WP-06 (events across sources) before analytics are told to
count events instead of rows. WP-03 (the discard ledger) is the base for WP-09 and WP-10.

## Decisions that are the maintainer's

These change what gets built. Each has a recommendation, and none should be decided by a coding
session on its own.

**On 2026-11-25 the maintainer left the open ones to the session, to be made from analysis.** What was
decided, on what evidence, and what would reverse each is in `decisions-2026-11-25.md`, which supersedes
the recommendations below where they differ: events are computed when read and not stored on the row
(item 1), each source is its own site until told otherwise (2), a three-tier store replaces the single
summary table (4), no image for now (5), and analysis stays per file by default because the premise of
item 6 no longer held at the shipped settings.

1. **Cross-source grouping (WP-06).** Recommended: keep every row, add an event id that groups rows
   of one species from different sources whose windows overlap in time, count events by default and
   offer rows as the other view. Never delete or merge rows. The time tolerance is the open number:
   sources' clocks agree only to the second, and RTSP adds unmeasured delay (MSC-14), so it has to be
   chosen from a measurement on a real two-source station, not picked.
2. **Whether co-located sources are one station or several (MSC-18, `docs/design/multi-station.md`).**
   Grouping only makes sense for sources that can hear the same bird. A per-source "site" label
   settles which sources are compared.
3. **V2.4 beside Perch (MMP-01).** Recommended first step: an extra classifier that cannot run is
   dropped with a loud warning and detection continues. Resampling a second rate for the second
   model is a separate, larger change with a memory and time cost nobody has measured on a Pi
   (MMP-13).
   **Decided 2026-11-23 by the maintainer: resample.** An extra classifier at another rate runs on
   its own resample of each recording (WP-01, `af8bb36`); a second model file that will not load
   still stops the daemon at startup, and `--doctor` now fails on it beforehand. The Pi cost is
   still unmeasured (MMP-13).
4. **Storing non-detections and sub-threshold scores (RES-04).** It is what calibration and occupancy
   work need, and it multiplies the database's size. Recommended: a per-window summary table that is
   off by default and on in a "research" profile.
5. **A flashable image (OOB-01).** It is the single largest reduction in setup steps and a new
   release artefact to build, sign and support.
6. **Segment seams (MSC-08, S-1).** Carrying a tail of audio across the boundary changes results
   against BirdNET-Pi; the zero padding is a deliberate parity choice today.

## Rules for the sessions that pick this up

- One work package, or one coherent part of one, per pull request. Read the finding in its audit
  file, then re-read the cited lines: line numbers were true at `9d8bb1c`.
- Each finding carries an acceptance gate. Per `CLAUDE.md`, the gate is written first and watched
  failing on the unfixed code, and the commit message says how. For a CONJECTURED finding the gate
  is also the probe: if it passes on today's code, the finding is wrong, and the checklist row
  becomes `rejected` with that evidence.
- Run `scripts/preflight.sh` before every push.
- When a fix makes a sentence in `docs/HANDOFF.md`, `docs/TECH_DEBT.md`, the manual or a doc comment
  untrue, correct it in the same change. `08` and the "stale documentation" findings (MMP-17,
  ACC-16, OOB-11, UIC-34) list the ones already known.

## Not examined

Each audit file ends with its own list. Across all of them: nothing ran on a Raspberry Pi or under a
real systemd; no audio was analysed; no page was timed; code-scanning alerts could not be read (the
session token is refused); and 128 lower-severity rows of `docs/UNATTENDED_DEPLOYMENT_AUDIT.md` were
not re-checked.
