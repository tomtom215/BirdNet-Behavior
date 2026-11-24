# Work packages

Generated from the headings of the audit files; see `README.md` for the order and the rules. A finding's line gives its id, severity, whether an older backlog already tracks it, and its title. "c" marks a finding whose entry mentions a conjecture: read its Status line before relying on it.

**187 findings**: 25 high, 87 medium, 75 low. 41 are already tracked in an older backlog.

| File | Findings |
|------|----------|
| `01-multi-source-capture.md` | 21 |
| `02-multi-model-pipeline.md` | 21 |
| `03-precision-accuracy.md` | 20 |
| `04-zero-setup-defaults.md` | 22 |
| `05-ui-rendered-walkthrough.md` | 37 |
| `06-ui-code-charts-ia.md` | 34 |
| `07-research-grade.md` | 32 |

## WP-00 (phase 0): Make main green and the handoff true

Six clippy::map_unwrap_or errors in birdnet-core (CI run 43311161060), the mutation workflow's config check, and the stale header and section 6 of docs/HANDOFF.md. Source: 08-existing-backlog-reconciled.md.


## WP-01 (phase 1): A second classifier that cannot run must not stop detection

Drop the unusable extra classifier, keep the primary, say so on the page and in --doctor.

- [ ] **MMP-01** (high) The documented "install Perch as a second classifier" recipe stops all detection on a default (V2.4) station
- [ ] **MMP-02** (medium) `--doctor` does not check extra classifiers or routes, although code and manual say it does
- [ ] **MMP-21** (medium) The multi-model invariants that are not tested
- [ ] **MMP-12** (low) A loaded extra classifier that no route names costs its memory and never runs, silently
- [ ] **MMP-16** (low) Startup diagnostics describe the primary, or the configured overlap, not what runs
- [ ] **MMP-18** (low, c) FP16/INT8-I/O and rank-3 models are accepted by the shape code but would fail at run time
- [ ] **MMP-19** (low) Public `process_and_infer` skips every filter and has no caller
- [ ] **MMP-22** (high) A route to a classifier the memory plan declined stops all detection (added 2026-11-23, after this file was generated; not in the counts above)

## WP-02 (phase 1): Non-bird classes, the human-sound rule and the range filter across classifiers

One list of non-bird and human classes per vocabulary; the location filter applied to every classifier's species.

- [x] **MMP-03** (high) Species only a secondary classifier knows are never location-filtered, and the include list can silently stop binding for them
- [x] **MMP-04** (high, c) Perch's 200 sound classes ("Speech", "Engine", "Crow", "Chirp_and_tweet", …) are recorded, notified and posted as species
- [x] **MMP-05** (high, c) The privacy filter does not recognise Perch's speech classes
- [-] **MMP-14** (medium, tracked) One classifier's non-finite human score suppresses chunks whichever classifier heard the bird
- [x] **ACC-18** (low, c) Non-bird classes can be stored as "species"
- [x] **MMP-23** (low) The human-class rule reads a cicada as a person (added 2026-11-23, after this file was generated; not in the counts above)
- [x] **MMP-24** (medium) An include list naming only a second classifier's species leaves the primary unfiltered (added 2026-11-23, likewise)
- [ ] **MMP-25** (low) The range-filter explainer answers for the primary's vocabulary only (added 2026-11-23, likewise)

`[x]` done, `[-]` closed by the maintainer's decision of 2026-11-23 without a code change (MMP-14); `checklist.json` has the evidence.

## WP-03 (phase 1): A ledger of everything the filters discard

Count every discard by reason, keep out-of-range detections in the quarantine with the reason the UI already has, persist the counters.

- [x] **ACC-01** (high) The occurrence filter discards out-of-range birds without a trace; the "Unlikely here" quarantine is never written
- [x] **ACC-02** (high) The biggest discard paths are invisible: privacy, noise, corroboration and occurrence are not counted
- [x] **RES-28** (low, tracked, c) Kept raw audio bypasses the privacy filter, and suppressions are invisible
- [x] **ACC-21** (low) The manual's review-queue table said quarantine holds only per-species threshold misses (added 2026-11-23, after this file was generated; not in the counts above)

`[x]` done; `checklist.json` has the evidence.

## WP-04 (phase 1): Source identity and fairness between sources

Shed fairly across sources; key comments and quarantine by source; duplicate-device guard; channel selection; removed sources stay removed.

- [x] **MSC-01** (high, c) Shedding drops the same source's segment on every sweep
- [x] **MSC-04** (medium) A second source's quarantined detection in the same second is discarded; an approved one loses its source and its audio
- [x] **MSC-05** (medium) Comments attach to every source's detection of the same species in the same second
- [x] **MSC-10** (medium, tracked, c) The same physical microphone or camera can be configured twice
- [x] **MSC-11** (medium) Channel selection is unreachable from the UI and ignored for PipeWire and RTSP; `bit_depth` is ignored everywhere
- [x] **MSC-12** (medium) Removing every source does not stop capture; a restart resurrects it from config under a different label
- [x] **MSC-13** (low) Source identity is the row id, not the device — re-adding splits history, editing a URL merges two places
- [x] **MSC-17** (low) Filename-derived source matching is prefix-unsafe for ids containing `-`
- [x] **MSC-20** (low) "Also heard by" hides a failed read and uses a fixed 30 s window

`[x]` done; what each leaves open is in `checklist.json` and `status/WP-04.md`.

## WP-05 (phase 1): Effort that is true, hourly and per source, and used as the denominator

Credit analysed audio, not uptime; hourly resolution; per-source rates; every rate, average and anomaly reads it.

- [ ] **MSC-03** (high) The effort-corrected abundance rate doubles on a two-source station
- [ ] **RES-01** (high) The effort ledger counts listening the station did not analyse
- [ ] **RES-02** (high, tracked) Effort has a resolution of one day, so detection/non-detection histories cannot be built
- [ ] **RES-03** (high) Effort-corrected abundance divides detections from every source by one source's hours
- [ ] **RES-14** (high) The time-series analytics ignore effort entirely
- [ ] **UIC-01** (high) Year in review says "days of listening" and "≈ N a day" for days on which something was *detected*
- [ ] **UIC-02** (high) An effort-corrected abundance API exists and no chart uses it; every Patterns chart draws raw counts
- [ ] **UIR-02** (high) "No weekday stands out: the busiest, Thursday" beside bars where Saturday is longest
- [ ] **RES-15** (medium) "Abundance" is a vocal-activity rate, its doc claims an effort correction it does not make, and absences are dropped
- [ ] **RES-19** (medium) Phenology dates and first-of-season flags are effort-blind and sensitive to single false positives
- [ ] **UIC-03** (medium) Trend average and anomaly detection count an outage as a quiet day
- [ ] **UIR-08** (medium) "A loud week, +315 %" against a week the station was partly off
- [ ] **MSC-16** (low) Effort counts audio the pipeline never analysed
- [ ] **RES-26** (low) A "listened" day needs only 60 s of recording

## WP-06 (phase 1): One bird heard by several sources is one event

Non-destructive event grouping across sources; analytics, DuckDB copy, BirdWeather and notifications count events and carry the source. Maintainer decision 1 and 2 first.

- [ ] **MSC-02** (high) Every count is a row count, and N sources hearing one bird are N rows; no deduplicated view exists
- [ ] **MSC-06** (medium) The DuckDB analytics copy has no `Source`, so every behavioural analytic pools sources
- [ ] **MSC-07** (medium, c) BirdWeather receives one post per source for one bird
- [ ] **RES-18** (medium) Pseudo-replication and double counting across sources
- [ ] **UIC-11** (medium, tracked) No source, model or confidence filter on any Patterns chart
- [ ] **MSC-18** (low) No per-source thresholds, species lists, location or dynamic threshold
- [ ] **MSC-19** (low, tracked) Live and notification payloads do not say which source heard the bird

## WP-07 (phase 1): Segment seams, timing and capacity

Calls across segment boundaries, dropped tails, timestamp precision, RTSP delay, limits that scale with the number of sources. Maintainer decision 6 first.

- [ ] **ACC-10** (medium, c) Corroboration cannot see across a segment boundary, so edge windows are held to a one-sided test
- [ ] **MSC-08** (medium, c) Segment boundaries are hard edges — calls are split, edge chunks filtered harder, and a non-multiple segment length discards audio
- [ ] **MSC-14** (medium, c) Timestamps are wall clock at segment open with one-second resolution; RTSP adds unmeasured latency, burst-time stamps and possible same-second overwrites
- [ ] **MSC-15** (medium, c) The stream directory's limits do not scale with the number of sources
- [ ] **RES-13** (medium, tracked) Time resolution, and local-time buckets in the levels and effort tables
- [ ] **ACC-13** (low, c) Decode errors silently drop packets, shifting every later sample earlier
- [ ] **ACC-17** (low, c) Detection time is truncated to the whole second
- [ ] **ACC-19** (low, tracked) Overlapping windows are separate rows, and counts scale with `OVERLAP`
- [ ] **MSC-09** (low) With OVERLAP > 0, one call is several rows unless the operator also enables the duplicate interval
- [ ] **MSC-21** (low) One analysis thread for every source

## WP-08 (phase 1): Thresholds, score scales and the signal path

A bar per classifier scale; dynamic thresholds that cannot feed themselves; the shared model cut; the night list; the capture high-pass in the parity study.

- [ ] **ACC-04** (medium, c) One low per-species threshold, or dynamic thresholds, changes the evidence every other species is judged on
- [ ] **ACC-05** (medium, tracked) Dynamic thresholds can reinforce a systematic confuser down to 0.25 within half an hour
- [ ] **ACC-06** (medium, c) The default capture high-pass (120 Hz) and DC block change the model's input and are outside every parity measurement
- [ ] **ACC-11** (medium) The night filter's genus list omits well-known nocturnal singers
- [ ] **MMP-06** (medium) One confidence bar is applied to scores on different scales, and adding a classifier can quarantine a detection the primary alone accepts
- [ ] **MMP-07** (medium) With dynamic thresholds on, a classifier's "own bar" in the merge is the dynamic floor, not the station's threshold
- [ ] **MMP-09** (medium, tracked) Exact-scientific-name merge splits renamed species into two rows with no agreement
- [ ] **MMP-11** (medium, c) Output scale and sample rate are inferred from input length, with no override for the scale
- [ ] **MMP-15** (medium, tracked) Perch's bar is measured, not validated on station audio
- [ ] **ACC-14** (low) No warning when a source cannot carry the band the model listens to
- [ ] **ACC-15** (low) Sigmoid without logit clipping; an unverified claim about BirdNET-Analyzer
- [ ] **MMP-10** (low, c) A merged row's common name follows whichever classifier won

## WP-09 (phase 2): A validation protocol and honest confidence wording

Reviewer identity kept, verdict history, blind random-sample review with intervals, exports of verified sets, no certainty claims from an uncalibrated score.

- [ ] **RES-06** (high) The reviewer's identity is lost after 180 days, and one verdict overwrites the last
- [ ] **RES-07** (high) No validation protocol: reviewers see confidence, the queue is not random, and there are no precision estimates or intervals
- [ ] **ACC-07** (medium) The "Model vs. human" panel presents a statistically invalid comparison as an accuracy signal
- [ ] **ACC-08** (medium) The per-species threshold suggester is too thin to trust and biased by what was reviewed
- [ ] **ACC-09** (medium) An uncalibrated score is presented as certainty
- [ ] **RES-27** (medium) The eBird export claims presence from unreviewed detections
- [ ] **RES-30** (low, tracked, c) Confidence is presented as a probability

## WP-10 (phase 2): Measured accuracy: a ground-truth harness on both architectures

Precision and recall on annotated recordings with the station's filters on; model output checked on aarch64; timings on a Pi.

- [ ] **ACC-03** (high, tracked) No ground-truth precision or recall exists for the default configuration; "parity" is agreement with BirdNET-Pi only
- [ ] **RES-04** (high) No record of windows that were analysed and found nothing, and no scores below the threshold
- [ ] **ACC-12** (medium, tracked) Nothing checks the model's output on aarch64, the target platform
- [ ] **ACC-20** (low, tracked) The V3.0 window length and its privacy default rest on thin evidence
- [ ] **MMP-13** (low) Inference is sequential with a fixed 2 threads; no Pi measurement exists for V3 or Perch
- [ ] **RES-31** (low, c) Reproducibility of outputs is unmeasured

## WP-11 (phase 2): Provenance, exports and the deployment record

Per-row model attribution, run records that hold the whole configuration, clock trust, stable ids, methods-ready exports, calibrated or relabelled sound levels.

- [ ] **MMP-08** (medium) Exports and run records attribute every row to the primary model's file and checksum
- [ ] **RES-08** (medium, tracked) The CSV export leaves out the columns a methods section needs, and carries no schema version or licence
- [ ] **RES-09** (medium, c) The Raven export points at clips that have been deleted
- [ ] **RES-10** (medium) Retention deletes the audio behind reviewed and commented records
- [ ] **RES-11** (medium) `analysis_runs` records less than the configuration that decided what was recorded
- [ ] **RES-12** (medium) Clock trust is never written down, and capture records under an untrusted clock
- [ ] **RES-20** (medium) Sound levels are labelled dB SPL from one unverified offset and a ~5 % sample
- [ ] **RES-21** (medium) Weather comes from a forecast model, from mixed providers, with no provenance
- [ ] **RES-22** (medium, tracked) There is no deployment record or station identity
- [ ] **RES-25** (medium, tracked) No stable detection identifier
- [ ] **UIC-10** (medium, tracked) Data export is a date range behind a login; the detection log cannot export what it found
- [ ] **MMP-20** (low) The catalogue carries no licence, and installs models under different licences without saying so
- [ ] **RES-23** (low) Clips carry thin metadata and no checksum
- [ ] **RES-24** (low) No researcher chapter and no citable methods block
- [ ] **RES-29** (low, c) Offsite backup covers the database, not the audio evidence
- [ ] **RES-32** (low) The taxonomy edition is not recorded

## WP-12 (phase 2): Re-analysis and batch ingest of recorder cards

Re-run kept audio under another model or threshold as a new versioned run; accept recorder filenames.

- [ ] **RES-05** (high, tracked) No re-analysis or batch path, and SD-card recordings cannot be ingested

## WP-13 (phase 2): Analytics that are statistics, not metaphors

Null models and multiple-comparison control for the behavioural views; sun-relative dawn analyses; Jaccard for companions.

- [ ] **RES-16** (medium) The behavioural "patterns" are web-analytics measures with no null model
- [ ] **RES-17** (medium) Dawn analyses use clock hours and today's sun across a 60-day window
- [ ] **UIC-04** (medium) "Companion species" ranks by raw shared days — the defect the pair view was fixed for
- [ ] **UIR-18** (medium) Dawn and dusk are defined by the clock on some charts and by the sun on others
- [ ] **UIR-21** (medium) "Nothing stands out" beside a disc that dominates the figure
- [ ] **UIR-22** (medium) Co-occurrence: the strongest tie is 3 %, drawn as if it were strong; three definitions of "together"
- [ ] **UIR-34** (medium) Trends asks a question and does not answer it
- [ ] **UIC-26** (low) Dawn and dusk colouring on "Detections by hour" uses fixed clock hours

## WP-14 (phase 3): A first run that tells the truth in plain words

Silent or clipping microphone, missing location, wrong time zone and stale clock each shown on the first screen; one notification default; plain wording.

- [ ] **OOB-02** (medium, tracked) A station without coordinates says so only in the installer's scrollback and on `/station`
- [ ] **OOB-04** (medium) The settings page and the generated config say fixed recording hours are UTC; the code uses local time
- [ ] **OOB-05** (medium, tracked) A muted, zero-gain or clipping microphone looks healthy on the first screen
- [ ] **OOB-06** (medium, c) A Pi left on the wrong time zone files every detection at the wrong hour, and the doctor passes it
- [ ] **OOB-07** (medium) The admin password is a 22-character string the reader must copy from a terminal, and recovery needs `sudo grep`
- [ ] **OOB-13** (medium) The live microphone feed is open to anyone on the network by default
- [ ] **OOB-17** (medium, c) After a power cut with no network, an RTC-less Pi records under a stale clock with nothing saying so
- [ ] **UIR-11** (medium) Three signals disagree about whether the station is recording
- [ ] **OOB-11** (low) The setup documentation describes controls that do not exist
- [ ] **OOB-12** (low) Three different notification-trigger defaults
- [ ] **OOB-15** (low) First-run wording on the installer and wizard path is jargon
- [ ] **OOB-16** (low, c) The wizard's microphone test probably fails with a raw ALSA error on the card the station is already using
- [ ] **OOB-19** (low) No notice of the model's non-commercial licence at install or in the wizard
- [ ] **OOB-20** (low) Location entry has no map and no place search; an offline install cannot complete
- [ ] **OOB-21** (low) Hardware guidance is generic; no recommended kit, height or orientation, and the target level disagrees across pages
- [ ] **UIR-12** (low) Capture tab says the classifier status is unavailable while Health lists two running models

## WP-15 (phase 3): Install, update and recovery without a terminal

Flashable image (maintainer decision 5), in-app update that the unit's sandbox allows, safe behaviour on a BirdNET-Pi host, stable device names in Docker.

- [ ] **OOB-01** (high) No flashable image, and no documented path from a blank card to a terminal
- [ ] **OOB-03** (high) Installing over a BirdNET-Pi host adopts BirdNET-Pi's config file and changes its ownership
- [ ] **OOB-08** (medium, tracked) Updating needs a terminal; the station is never updated by this reader
- [ ] **OOB-10** (medium, tracked) The Docker quick-start stores the unstable ALSA card index
- [ ] **OOB-14** (medium, tracked) SD-card wear for an unattended Pi
- [ ] **OOB-09** (low, tracked) The address the reader must type is an IP and a port; no `birdnet.local`
- [ ] **OOB-18** (low, tracked) The settings and password are in no backup; nothing guides a dead-card recovery
- [ ] **OOB-22** (low, tracked) The Docker quick-start runs the development branch's script, and the install needs Docker group surgery

## WP-16 (phase 3): Navigation, naming and stale words

One name per thing, orphaned routes removed or linked, a Birder and a Researcher depth, documentation that matches the code.

- [ ] **UIR-15** (high, tracked) Two settings applications that disagree with each other
- [ ] **UIC-05** (medium, tracked) Six more routes are orphaned than TD-6 lists, including a full page
- [ ] **UIC-06** (medium) The "is it working?" home is labelled "Settings"
- [ ] **UIC-07** (medium, tracked) Two "Activity sessions", two review queues, three searches, four hour-of-day pictures
- [ ] **UIC-15** (medium) Jargon a birder will not understand (with proposed wording)
- [ ] **UIC-30** (medium) Proposed simplified IA — one switch, two depths, same URLs
- [ ] **UIR-07** (medium) The changelog prints raw Markdown and cuts every entry to its first line
- [ ] **UIR-16** (medium) "Is it working?" lives under a nav item called Settings
- [ ] **UIR-17** (medium, tracked) Engine internals and command lines on pages written for birders
- [ ] **UIR-32** (medium) Alerts is a rules engine where a birder expects "tell me when…"
- [ ] **ACC-16** (low) Stale documentation misdescribes the signal path
- [ ] **MMP-17** (low) Stale and contradictory documentation of the input contract
- [ ] **UIC-09** (low) Sharing exists only for one detection
- [ ] **UIC-14** (low) A birder tab shows the database's file path and build instructions
- [ ] **UIC-16** (low) The retention lede describes a table that is no longer there
- [ ] **UIC-20** (low) A dead template with inline styles and a retired link
- [ ] **UIC-24** (low) Delete and lock controls are drawn for visitors who cannot use them
- [ ] **UIC-34** (low) Stale module docs
- [ ] **UIR-13** (low) "The station's watch folder The station takes…"
- [ ] **UIR-14** (low) The Data tab prints a setting's name where its value belongs, and refers to itself
- [ ] **UIR-26** (low, tracked) A bird never heard gets a full, empty species page
- [ ] **UIR-35** (low) Queues with no home in the navigation

## WP-17 (phase 4): One chart system, accessible and linkable, then the new graphics

Shared helpers and scales, table alternatives, ranges in the URL, localised numbers and times, then the charts the data supports.

- [ ] **UIR-01** (high) A species changes colour from chart to chart
- [ ] **UIC-08** (medium) The Patterns range is JavaScript-only, not in the URL, not remembered, and not shareable
- [ ] **UIC-12** (medium) Every play button on Today is named "Play clip"
- [ ] **UIC-17** (medium, tracked, c) Localisation stops at species names; chart times ignore the 12-hour setting; numbers always use commas
- [ ] **UIC-21** (medium, tracked) History scans the whole table on every month click; a day page lists every detection
- [ ] **UIC-25** (medium) Graphics the stored data supports and no page draws (for chart lovers and researchers)
- [ ] **UIC-27** (medium) The most-seen charts have no table alternative
- [ ] **UIR-03** (medium) Retention heat table never shows its colour
- [ ] **UIR-04** (medium) "Darker means more" is written over dark-theme charts where more is lighter
- [ ] **UIR-05** (medium) The per-row "waveform" is invented
- [ ] **UIR-06** (medium) Species list rank "10"–"15" break over two lines on a desktop
- [ ] **UIR-09** (medium) History calendar numbers collide, and an off day looks like a future one
- [ ] **UIR-10** (medium) On Recordings, Play is invisible until you hover
- [ ] **UIR-29** (medium, tracked) Phone: hidden tabs and clipped charts
- [ ] **UIR-30** (medium, tracked) Numbers without provenance
- [ ] **UIC-13** (low, c) Today's headline lives in a region swapped every 5 minutes, named by `aria-label` on a `div`
- [ ] **UIC-18** (low) Duplicated chart helpers, one with its arguments swapped; three heat scales
- [ ] **UIC-19** (low) Hashed species colours on lists that show many species together
- [ ] **UIC-22** (low, tracked) Loading vocabulary is uneven; most Patterns and Reports cards say "Loading…" in text
- [ ] **UIC-23** (low) Cached analytic figures do not say how old they are
- [ ] **UIC-28** (low) History's month cells are `href='#'` links driven only by htmx
- [ ] **UIC-29** (low, c) Focus after HTMX swaps is left to chance outside a few scripts
- [ ] **UIC-31** (low) Escaping discipline holds where sampled; it is per-call-site, not structural
- [ ] **UIC-32** (low) Polling and the live socket overlap on Today
- [ ] **UIC-33** (low, tracked) The failed-read-as-zero sweep: gate exists and holds; blind spots named
- [ ] **UIR-19** (low) Two hour-axis vocabularies
- [ ] **UIR-20** (low) Five date formats
- [ ] **UIR-23** (low) Small encodings that say the wrong thing
- [ ] **UIR-24** (low) Species page shows its four numbers twice and opens with "Circadian"
- [ ] **UIR-25** (low) Life list draws one curve twice and names 4 of 15 birds
- [ ] **UIR-27** (low, tracked) Page heads, H1 faces and content columns differ by page
- [ ] **UIR-28** (low) Six designs for "a detection"
- [ ] **UIR-31** (low, tracked) Kiosk details
- [ ] **UIR-33** (low) Resizing replays chart entrances (and empties full-page screenshots)
- [ ] **UIR-36** (low) Migration and Dawn: pills that look like filters, colour that names a season
- [ ] **UIR-37** (low) The occurrence answer stops at "Held back"
